use super::session::{Selection, changed, state_error, validate_account};
use super::*;
use crate::web::sms::{self, KugouWebSmsChallenge, KugouWebSmsRequest};
use rand::{TryRng, rngs::SysRng};
use std::time::Duration;
use tokio::time::Instant;
use tuneweave_core::{
    AuthChallengeAction, AuthChallengeBackend, AuthChallengeProgress, AuthChallengeRequest,
    AuthChallengeStatus, ProviderAuthChallenge,
};

pub(super) struct Entry {
    pub(super) challenge: ProviderAuthChallenge,
    pub(super) deadline: Instant,
    previous: Option<Selection>,
    context: Option<KugouWebSmsChallenge>,
}
impl Entry {
    pub(super) fn is_busy(&self) -> bool {
        self.context.is_none()
    }
}
struct Lease {
    registry: Arc<Mutex<qr::Transactions>>,
    id: String,
    armed: bool,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if self.armed {
            if let Ok(mut r) = self.registry.lock() {
                r.sms.remove(&self.id);
            }
        }
    }
}
fn busy() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::Conflict,
        "KuGou SMS verification is already in progress",
    )
    .with_platform(Platform::Kugou)
}
fn unavailable() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "KuGou SMS transaction is expired or consumed",
    )
    .with_platform(Platform::Kugou)
    .with_consumed_auth_challenge()
}
fn cooldown() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::RateLimited,
        "KuGou SMS delivery cooldown or capacity limit reached",
    )
    .with_platform(Platform::Kugou)
}

impl KugouProvider {
    fn check_sms_entry(&self, registry: &qr::Transactions, id: &str) -> Result<()> {
        let entry = registry.sms.get(id).ok_or_else(unavailable)?;
        if Instant::now() >= entry.deadline {
            return Err(unavailable());
        }
        if entry.challenge.credential_mode().persists_on_server()
            && self.selected(&entry.challenge.request().account)? != entry.previous
        {
            return Err(changed().with_consumed_auth_challenge());
        }
        Ok(())
    }
    fn check_sms(&self, id: &str) -> Result<()> {
        let mut registry = self.qr_transactions.lock().map_err(|_| state_error())?;
        registry.prune();
        self.check_sms_entry(&registry, id)
    }
    fn sms_id(
        &self,
        registry: &qr::Transactions,
        challenge: &ProviderAuthChallenge,
    ) -> Result<String> {
        self.require_login_mode(challenge.credential_mode())?;
        if challenge.platform() != Platform::Kugou {
            return Err(kugou_invalid_request("Invalid KuGou SMS receipt"));
        }
        let id = challenge
            .provider_transaction_id()
            .ok_or_else(unavailable)?;
        let entry = registry.sms.get(id).ok_or_else(unavailable)?;
        if entry.challenge != *challenge {
            return Err(kugou_invalid_request(
                "KuGou SMS receipt ownership does not match",
            ));
        }
        Ok(id.to_owned())
    }
    pub(super) async fn begin_sms(
        &self,
        request: &AuthChallengeRequest,
        mode: CredentialMode,
    ) -> Result<ProviderAuthChallenge> {
        request.reject_platform_policies_option(Platform::Kugou)?;
        self.require_login_mode(mode)?;
        validate_account(&request.account, mode)?;
        sms::validate_phone(&request.principal)?;
        if request.backend != AuthChallengeBackend::Standard
            || request
                .country_code
                .as_deref()
                .is_some_and(|s| !matches!(s, "86" | "+86"))
        {
            return Err(kugou_invalid_request(
                "KuGou SMS requires the standard Web backend and mainland China number",
            ));
        }
        let mut random = [0; 32];
        SysRng
            .try_fill_bytes(&mut random)
            .map_err(|_| state_error())?;
        let id = hex::encode(random);
        let challenge =
            ProviderAuthChallenge::stateful(Platform::Kugou, request.clone(), mode, id.clone())?;
        {
            let mut registry = self.qr_transactions.lock().map_err(|_| state_error())?;
            registry.prune();
            registry.require_capacity()?;
            if registry
                .sms_cooldowns
                .get(&request.principal)
                .is_some_and(|until| *until > Instant::now())
                || registry.sms_cooldowns.len() >= 1024
            {
                return Err(cooldown());
            }
            if registry.sms.values().any(|e| {
                e.challenge.request().principal == request.principal && e.context.is_none()
            }) || registry.passwords.values().any(|e| {
                e.web_sms_target()
                    .is_some_and(|(phone, busy)| phone == request.principal && busy)
            }) {
                return Err(busy());
            }
            let previous = if mode.persists_on_server() {
                self.selected(&request.account)?
            } else {
                None
            };
            if registry.sms.contains_key(&id) {
                return Err(state_error());
            }
            // A new explicit delivery retires older codes for the same phone. Keep
            // the cooldown after failures because a timeout cannot prove no SMS was sent.
            registry
                .sms
                .retain(|_, e| e.challenge.request().principal != request.principal);
            registry.passwords.retain(|_, e| {
                !e.web_sms_target()
                    .is_some_and(|(phone, _)| phone == request.principal)
            });
            registry.sms_cooldowns.insert(
                request.principal.clone(),
                Instant::now() + Duration::from_secs(60),
            );
            registry.sms.insert(
                id.clone(),
                Entry {
                    challenge: challenge.clone(),
                    deadline: Instant::now() + Duration::from_secs(300),
                    previous,
                    context: None,
                },
            );
        }
        let mut lease = Lease {
            registry: self.qr_transactions.clone(),
            id,
            armed: true,
        };
        let result = self
            .client
            .send_web_login_sms(KugouWebSmsRequest {
                phone: request.principal.clone(),
                allow_account_creation: request.allow_account_creation,
            })
            .await;
        let mut registry = self.qr_transactions.lock().map_err(|_| state_error())?;
        if let Err(error) = &result {
            if error.code == ErrorCode::RateLimited {
                // The send may be rejected with a longer server cooldown. Never
                // shorten the reservation, including when its account was changed.
                let delay = error.details["retry_after_secs"]
                    .as_u64()
                    .unwrap_or(2)
                    .clamp(2, 300);
                if let Some(until) = registry.sms_cooldowns.get_mut(&request.principal) {
                    *until = (*until).max(Instant::now() + Duration::from_secs(delay));
                }
            }
        }
        self.check_sms_entry(&registry, &lease.id)?;
        registry
            .sms
            .get_mut(&lease.id)
            .ok_or_else(unavailable)?
            .context = Some(result?);
        lease.armed = false;
        Ok(challenge)
    }
    pub(super) fn sms_status(
        &self,
        challenge: &ProviderAuthChallenge,
    ) -> Result<AuthChallengeStatus> {
        let mut registry = self.qr_transactions.lock().map_err(|_| state_error())?;
        registry.prune();
        let id = self.sms_id(&registry, challenge)?;
        if let Err(e) = self.check_sms_entry(&registry, &id) {
            registry.sms.remove(&id);
            return Err(e.with_consumed_auth_challenge());
        }
        registry.sms[&id]
            .context
            .as_ref()
            .ok_or_else(busy)?
            .status()
    }
    /// Cancels exactly this SDK SMS receipt, without affecting other accounts or codes.
    pub fn cancel_sms_login(&self, challenge: &ProviderAuthChallenge) -> Result<bool> {
        let mut registry = self.qr_transactions.lock().map_err(|_| state_error())?;
        registry.prune();
        let id = self.sms_id(&registry, challenge)?;
        Ok(registry.sms.remove(&id).is_some())
    }
    pub(super) async fn advance_sms(
        &self,
        challenge: &ProviderAuthChallenge,
        action: &AuthChallengeAction,
    ) -> Result<AuthChallengeProgress> {
        let code = match action {
            AuthChallengeAction::SubmitCode { code }
            | AuthChallengeAction::SelectAccount { code, .. }
            | AuthChallengeAction::SubmitBrowser { code, .. } => code,
            _ => return Err(kugou_invalid_request("Unsupported KuGou SMS action")),
        };
        sms::validate_code(code)?;
        let (id, mut context) = {
            let mut registry = self.qr_transactions.lock().map_err(|_| state_error())?;
            registry.prune();
            let id = self.sms_id(&registry, challenge)?;
            if let Err(e) = self.check_sms_entry(&registry, &id) {
                registry.sms.remove(&id);
                return Err(e.with_consumed_auth_challenge());
            }
            let context = registry
                .sms
                .get_mut(&id)
                .ok_or_else(unavailable)?
                .context
                .take()
                .ok_or_else(busy)?;
            (id, context)
        };
        let mut lease = Lease {
            registry: self.qr_transactions.clone(),
            id,
            armed: true,
        };
        let result = self
            .client
            .advance_web_sms_guarded(&mut context, action, &|| self.check_sms(&lease.id))
            .await;
        let mut registry = self
            .qr_transactions
            .lock()
            .map_err(|_| state_error().with_consumed_auth_challenge())?;
        self.check_sms_entry(&registry, &lease.id)
            .map_err(TuneWeaveError::with_consumed_auth_challenge)?;
        let confirmed = match result {
            Ok(AuthChallengeProgress::Confirmed(result)) => result,
            other => {
                if context.status().is_ok() {
                    registry
                        .sms
                        .get_mut(&lease.id)
                        .ok_or_else(unavailable)?
                        .context = Some(context);
                    lease.armed = false;
                }
                return other;
            }
        };
        let credential = KugouCredential::parse_caller(
            confirmed
                .credential
                .as_ref()
                .ok_or_else(|| state_error().with_consumed_auth_challenge())?,
        )
        .map_err(TuneWeaveError::with_consumed_auth_challenge)?;
        let mode = challenge.credential_mode();
        let account = &challenge.request().account;
        if mode.persists_on_server() {
            let store = self
                .credential_store
                .as_ref()
                .ok_or_else(|| state_error().with_consumed_auth_challenge())?;
            let next = credential
                .stored(account)
                .map_err(TuneWeaveError::with_consumed_auth_challenge)?;
            let previous = &registry.sms[&lease.id].previous;
            let committed = match previous {
                Some((_, Some(old))) => store
                    .compare_exchange(old, Some(&next))
                    .map_err(TuneWeaveError::with_consumed_auth_challenge)?,
                None => store
                    .insert_if_absent(&next)
                    .map_err(TuneWeaveError::with_consumed_auth_challenge)?,
                _ => return Err(state_error().with_consumed_auth_challenge()),
            };
            if !committed {
                return Err(changed().with_consumed_auth_challenge());
            }
            registry.cancel_account(account);
        } else {
            registry.sms.remove(&lease.id);
        }
        let mut profile = confirmed.profile;
        profile.account = account.clone();
        lease.armed = false;
        Ok(AuthChallengeProgress::Confirmed(ProviderAuthResult {
            profile,
            credential: mode
                .returns_to_caller()
                .then_some(confirmed.credential)
                .flatten(),
        }))
    }
}

#[cfg(test)]
mod tests;
