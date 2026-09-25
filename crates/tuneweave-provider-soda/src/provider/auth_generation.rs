use super::*;
use crate::authentication::AuthLease;
use std::time::{Duration, Instant};

struct QrOperation {
    transactions: SodaQrTransactions,
    id: String,
    keep: bool,
}

impl Drop for QrOperation {
    fn drop(&mut self) {
        if !self.keep {
            let _ = self.transactions.remove(&self.id);
        }
    }
}

impl SodaProvider {
    pub(super) fn authentication_lease(
        &self,
        account: Option<&str>,
        mode: CredentialMode,
        deadline: Instant,
    ) -> Result<AuthLease> {
        self.require_credential_mode(mode)?;
        if self.caller_credential.is_some() {
            return Err(soda_invalid_request(
                "start Soda authentication on the base provider",
            ));
        }
        let account = account.or((mode == CredentialMode::Client).then_some("default"));
        if let Some(account) = account {
            validate_soda_login_account(account, mode)?;
        }
        self.auth_transactions.reserve(
            account.map(str::trim),
            mode,
            self.credential_store.clone(),
            deadline,
        )
    }

    pub(super) async fn start_bound_qr(
        &self,
        login_type: Option<&str>,
        account: Option<&str>,
        mode: CredentialMode,
    ) -> Result<ProviderQrStart> {
        if let Some(login_type) = login_type.map(str::trim).filter(|value| !value.is_empty())
            && !matches!(
                login_type.to_ascii_lowercase().as_str(),
                "default" | "web" | "soda"
            )
        {
            return Err(soda_invalid_request(format!(
                "unsupported Soda QR login type: {login_type}"
            )));
        }
        let mut lease =
            self.authentication_lease(account, mode, Instant::now() + Duration::from_secs(300))?;
        let result = tokio::time::timeout_at(
            lease.deadline.into(),
            self.qr_transactions.start(&self.client, mode),
        )
        .await;
        let start = match result {
            Ok(Ok(start)) => start,
            other => {
                lease.ensure_current()?;
                return Err(match other {
                    Ok(Err(error)) => error,
                    Err(_) => soda_invalid_request("Soda QR creation expired"),
                    Ok(Ok(_)) => unreachable!(),
                });
            }
        };
        let mut operation = QrOperation {
            transactions: self.qr_transactions.clone(),
            id: start.provider_transaction_id.clone(),
            keep: false,
        };
        lease.shorten_deadline(
            self.qr_transactions
                .expires_at(&start.provider_transaction_id)?,
        )?;
        self.qr_transactions
            .attach_authentication(&start.provider_transaction_id, lease)?;
        operation.keep = true;
        Ok(ProviderQrStart {
            provider_transaction_id: start.provider_transaction_id,
            url: start.image_data_url.clone(),
            image_data_url: Some(start.image_data_url),
            expires_at: start.expires_at,
        })
    }

    pub(super) async fn continue_bound_qr(
        &self,
        id: &str,
        account: &str,
        mode: CredentialMode,
        action: Option<&tuneweave_core::QrVerificationAction>,
    ) -> Result<ProviderQrPoll> {
        validate_soda_login_account(account, mode)?;
        if self.caller_credential.is_some() {
            return Err(soda_invalid_request(
                "continue Soda authentication on the base provider",
            ));
        }
        let deadline = self.qr_transactions.expires_at(id)?;
        let mut access = tokio::time::timeout_at(
            deadline.into(),
            self.qr_transactions.access(id, account, mode),
        )
        .await
        .map_err(|_| soda_invalid_request("Soda QR access expired"))??;
        let mut operation = QrOperation {
            transactions: self.qr_transactions.clone(),
            id: id.to_owned(),
            keep: false,
        };
        let result = async {
            let lease = access
                .authentication
                .as_mut()
                .ok_or_else(soda_credential_lock_error)?;
            lease.bind(account)?;
            let outcome = if let Some(action) = action {
                lease
                    .wait(self.qr_transactions.verify(&self.client, id, action))
                    .await
            } else {
                lease
                    .wait(self.qr_transactions.poll(&self.client, id))
                    .await
            };
            // Retryable protocol errors keep their existing cooldown/MFA state. A
            // cancelled future, expired lease or changed account consumes the entry.
            lease.ensure_current()?;
            let outcome = match outcome {
                Ok(outcome) => outcome,
                Err(error) => {
                    operation.keep = true;
                    return Err(error);
                }
            };
            let result = self.qr_poll_result(id, outcome, &mut access).await?;
            if !matches!(
                result.state,
                AuthState::Confirmed | AuthState::Expired | AuthState::Failed
            ) {
                access
                    .authentication
                    .as_ref()
                    .ok_or_else(soda_credential_lock_error)?
                    .ensure_current()?;
                operation.keep = true;
            }
            Ok(result)
        }
        .await;
        match result {
            Err(error)
                if error.code == ErrorCode::InvalidRequest
                    && access
                        .authentication
                        .as_ref()
                        .is_some_and(|lease| Instant::now() >= lease.deadline) =>
            {
                Ok(ProviderQrPoll {
                    state: AuthState::Expired,
                    verification: None,
                    message: Some("Soda QR login expired".into()),
                    profile: None,
                    credential: None,
                })
            }
            Err(error) if !operation.keep => Err(error.with_consumed_auth_challenge()),
            other => other,
        }
    }
}
