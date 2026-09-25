use super::*;
use tuneweave_core::{ProviderSessionRevocationResult, SessionRevocationState};

#[derive(Clone, Copy, PartialEq)]
enum Cleanup {
    Removed,
    Absent,
    Replaced,
    NotOwned,
    NotAttempted,
}
impl Cleanup {
    fn name(self) -> &'static str {
        match self {
            Self::Removed => "removed",
            Self::Absent => "already_absent",
            Self::Replaced => "preserved_new_login",
            Self::NotOwned => "not_server_owned",
            Self::NotAttempted => "not_attempted",
        }
    }
}

// Synchronous conditional cleanup also runs if the request future is cancelled.
// It can remove rotations of this login, but never a newly established login.
struct RevocationCleanup {
    store: Option<Arc<dyn AccountCredentialStore>>,
    stored: Option<StoredAccountCredential>,
    source: SodaCredential,
    armed: bool,
    request_started: bool,
}
impl RevocationCleanup {
    fn current(&self) -> Result<Option<StoredAccountCredential>> {
        let Some(expected) = &self.stored else {
            return Ok(None);
        };
        Ok(self
            .store
            .as_ref()
            .ok_or_else(soda_credential_lock_error)?
            .load_platform(Platform::Soda)?
            .into_iter()
            .find(|v| v.account == expected.account))
    }
    fn ensure_current(&self) -> Result<()> {
        if self.stored.is_some() && self.current()? != self.stored {
            return Err(soda_session_changed());
        }
        Ok(())
    }
    fn clean(&self) -> Result<Cleanup> {
        if !self.armed {
            return Ok(Cleanup::NotAttempted);
        }
        if self.stored.is_none() {
            return Ok(Cleanup::NotOwned);
        }
        for _ in 0..3 {
            let Some(current) = self.current()? else {
                return Ok(Cleanup::Absent);
            };
            if current.kind != SODA_CREDENTIAL_KIND
                || !SodaCredential::parse(current.secret())?.same_login(&self.source)
            {
                return Ok(Cleanup::Replaced);
            }
            if self
                .store
                .as_ref()
                .ok_or_else(soda_credential_lock_error)?
                .compare_exchange(&current, None)?
            {
                return Ok(Cleanup::Removed);
            }
        }
        Err(soda_session_changed())
    }
    fn finish(&mut self) -> Result<Cleanup> {
        let result = self.clean();
        self.armed = false;
        result
    }
}
impl Drop for RevocationCleanup {
    fn drop(&mut self) {
        let _ = self.clean();
    }
}

impl SodaProvider {
    pub(super) async fn revoke_owned_session(
        &self,
        account: &str,
        caller: Option<&ProviderCredential>,
        mode: CredentialMode,
        budget: std::time::Duration,
    ) -> Result<ProviderSessionRevocationResult> {
        let _pending = PendingCredentialUpdate::new(self.response_credential.clone());
        self.validate_session_operation(account, caller, mode)?;
        if caller.is_none() && mode != CredentialMode::Server {
            return Err(soda_invalid_request(
                "caller-managed Soda revocation requires a caller credential",
            ));
        }
        let caller_source = caller.map(parse_soda_caller_credential).transpose()?;
        let stored = if mode.persists_on_server() {
            self.stored_credential(account)?
        } else {
            None
        };
        let source = if mode.persists_on_server() {
            let Some(stored) = &stored else {
                if caller_source.is_some() {
                    return Err(soda_session_changed());
                }
                let cancelled = self.auth_transactions.cancel_server(account, || {
                    if self.stored_credential(account)?.is_some() {
                        return Err(soda_session_changed());
                    }
                    self.qr_transactions.cancel_server(account)
                })?;
                drop(cancelled);
                return Ok(ProviderSessionRevocationResult {
                    state: SessionRevocationState::NoStoredSession,
                    removed: false,
                    caller_credential_discard_required: false,
                    revocation_request_started: false,
                });
            };
            if stored.kind != SODA_CREDENTIAL_KIND {
                return Err(soda_credential_lock_error());
            }
            let current = SodaCredential::parse(stored.secret())?;
            if caller_source
                .as_ref()
                .is_some_and(|v| !v.same_login(&current))
            {
                return Err(soda_session_changed());
            }
            current
        } else {
            caller_source.ok_or_else(soda_authentication_required)?
        };
        if source.user_id().is_none() {
            return Err(soda_invalid_request(
                "Soda session revocation requires a bound account identity",
            ));
        }
        if mode.persists_on_server() {
            let cancelled = self.auth_transactions.cancel_server(account, || {
                if self.stored_credential(account)? != stored {
                    return Err(soda_session_changed());
                }
                self.qr_transactions.cancel_server(account)
            })?;
            drop(cancelled);
        }
        let mut cleanup = RevocationCleanup {
            store: self.credential_store.clone(),
            stored,
            source: source.clone(),
            armed: false,
            request_started: false,
        };
        let outcome = tokio::time::timeout(budget, async {
            cleanup.ensure_current()?;
            let authenticated = self.client.revocation_session_authenticated(&source).await;
            cleanup.ensure_current()?;
            if !authenticated? {
                cleanup.armed = true;
                return Ok(SessionRevocationState::AlreadyInvalid);
            }
            let sent = self
                .client
                .send_session_revocation(&source, || {
                    cleanup.ensure_current()?;
                    cleanup.armed = true;
                    cleanup.request_started = true;
                    Ok(())
                })
                .await;
            cleanup.ensure_current()?;
            if !cleanup.request_started {
                sent?;
                return Err(soda_upstream_error("Soda revocation did not start"));
            }
            // A transport/status error does not prove that the write was rejected.
            // Conversely, a 200 or a cleared Cookie does not prove invalidation.
            let authenticated = self.client.revocation_session_authenticated(&source).await;
            cleanup.ensure_current()?;
            if authenticated? {
                return Err(soda_upstream_error(
                    "Soda session remains authenticated after the revocation request",
                ));
            }
            Ok(SessionRevocationState::Invalidated)
        })
        .await
        .unwrap_or_else(|_| {
            Err(TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Soda session revocation exceeded its total time budget",
            )
            .with_platform(Platform::Soda))
        });
        let armed = cleanup.armed;
        let request_started = cleanup.request_started;
        let cleaned = cleanup.finish();
        let evidence = match &outcome {
            Ok(SessionRevocationState::Invalidated) => "invalidated",
            Ok(SessionRevocationState::AlreadyInvalid) => "already_invalid",
            _ if request_started => "unconfirmed",
            _ => "not_attempted",
        };
        let failure = |error: TuneWeaveError, status: &str, removed: bool| {
            error.retryable(false).with_details(json!({
                "upstream_outcome": evidence,
                "revocation_request_started": request_started,
                "local_cleanup": status,
                "removed": removed,
                "caller_credential_discard_required": armed && caller.is_some(),
            }))
        };
        let cleaned = cleaned.map_err(|e| failure(e, "failed", false))?;
        let removed = cleaned == Cleanup::Removed;
        let state = outcome.map_err(|e| failure(e, cleaned.name(), removed))?;
        if matches!(cleaned, Cleanup::Replaced | Cleanup::Absent) {
            return Err(failure(soda_session_changed(), cleaned.name(), removed));
        }
        Ok(ProviderSessionRevocationResult {
            state,
            removed,
            caller_credential_discard_required: caller.is_some(),
            revocation_request_started: request_started,
        })
    }
}
