use super::*;
use tuneweave_core::{ProviderSessionRevocationResult, SessionRevocationState};

struct Cleanup<'a> {
    provider: &'a KuwoProvider,
    account: &'a str,
    selected: Selection,
    armed: bool,
    started: bool,
}
impl Cleanup<'_> {
    fn current(&self) -> Result<()> {
        if self.selected.stored.is_some() {
            self.provider
                .check_selection(self.account, &self.selected)?;
        }
        Ok(())
    }
    fn clean(&self) -> Result<&'static str> {
        if !self.armed {
            return Ok("not_attempted");
        }
        if self.selected.stored.is_none() {
            return Ok("not_server_owned");
        }
        let _registry = self
            .provider
            .auth_registry
            .lock()
            .map_err(|_| state_error())?;
        for _ in 0..3 {
            let Some(current) = self.provider.selected(self.account)? else {
                return Ok("already_absent");
            };
            // Refresh rotates the SID within the same login generation. Revoking
            // the selected old SID says nothing about that replacement SID, so only
            // remove the exact selected credential snapshot.
            if current.credential != self.selected.credential {
                return Ok("preserved_replacement");
            }
            if self
                .provider
                .credential_store
                .as_ref()
                .ok_or_else(state_error)?
                .compare_exchange(current.stored.as_ref().ok_or_else(state_error)?, None)?
            {
                return Ok("removed");
            }
        }
        Err(changed())
    }
    fn finish(&mut self) -> Result<&'static str> {
        let result = self.clean();
        self.armed = false;
        result
    }
}
impl Drop for Cleanup<'_> {
    fn drop(&mut self) {
        let _ = self.clean();
    }
}

impl KuwoProvider {
    pub(in crate::provider) async fn revoke_owned(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
        budget: Duration,
    ) -> Result<ProviderSessionRevocationResult> {
        let selected = {
            let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
            let selected = self.owned_source(account, source, mode)?;
            if mode.persists_on_server() {
                registry.cancel_account(account);
            }
            selected
        };
        let Some(selected) = selected else {
            return Ok(ProviderSessionRevocationResult {
                state: SessionRevocationState::NoStoredSession,
                removed: false,
                caller_credential_discard_required: false,
                revocation_request_started: false,
            });
        };
        let input = selected.credential.input()?;
        crate::client::native::validate_session_metadata(&input)?;
        let mut cleanup = Cleanup {
            provider: self,
            account,
            selected,
            armed: false,
            started: false,
        };
        let outcome = tokio::time::timeout(budget, async {
            cleanup.current()?;
            let authenticated = self.client.native_session_authenticated(&input).await;
            cleanup.current()?;
            if !authenticated? {
                cleanup.armed = true;
                return Ok(SessionRevocationState::AlreadyInvalid);
            }
            let sent = self
                .client
                .send_native_session_revocation(&input, || {
                    cleanup.current()?;
                    cleanup.armed = true;
                    cleanup.started = true;
                    Ok(())
                })
                .await;
            cleanup.current()?;
            if !cleanup.started {
                sent?;
                return Err(kuwo_upstream_error("Kuwo revocation did not start"));
            }
            // A receipt alone is not invalidation evidence. A lost response also
            // cannot prove rejection: independently validate the exact original SID.
            let authenticated = self.client.native_session_authenticated(&input).await;
            cleanup.current()?;
            if authenticated? {
                return Err(kuwo_upstream_error(
                    "Kuwo session remains valid after revocation",
                ));
            }
            Ok(SessionRevocationState::Invalidated)
        })
        .await
        .unwrap_or_else(|_| {
            Err(TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Kuwo revocation exceeded its total time budget",
            )
            .with_platform(Platform::Kuwo))
        });
        let armed = cleanup.armed;
        let started = cleanup.started;
        let cleaned = cleanup.finish();
        let evidence = match &outcome {
            Ok(SessionRevocationState::Invalidated) => "invalidated",
            Ok(SessionRevocationState::AlreadyInvalid) => "already_invalid",
            _ if started => "unconfirmed",
            _ => "not_attempted",
        };
        let failure = |error: TuneWeaveError, status: &str| {
            error.retryable(false).with_details(json!({
                "upstream_outcome": evidence, "revocation_request_started": started,
                "local_cleanup": status, "removed": status == "removed",
                "caller_credential_discard_required": armed && mode.returns_to_caller(),
            }))
        };
        let cleaned = cleaned.map_err(|e| failure(e, "failed"))?;
        let state = outcome.map_err(|e| failure(e, cleaned))?;
        if matches!(cleaned, "preserved_replacement" | "already_absent") {
            return Err(failure(changed(), cleaned));
        }
        Ok(ProviderSessionRevocationResult {
            state,
            removed: cleaned == "removed",
            caller_credential_discard_required: mode.returns_to_caller(),
            revocation_request_started: started,
        })
    }
}

#[cfg(test)]
mod tests;
