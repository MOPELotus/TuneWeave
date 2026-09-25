use super::session::{Selection, validate_account};
use super::*;
use crate::{credential::error, passport::cookies::PassportCookies};
use rand::{TryRng, rngs::SysRng};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use tuneweave_core::{
    AuthChallengeAction, AuthChallengeBackend, AuthChallengeProgress, AuthChallengeRequest,
    AuthChallengeStatus, AuthImageAnswerKind, AuthImageChallenge, ErrorCode, ProviderAuthChallenge,
};

pub(super) const CAPACITY: usize = 128;
pub(super) const TTL: Duration = Duration::from_secs(300);
pub(super) const COOLDOWN: Duration = Duration::from_secs(60);
const ATTEMPTS: u8 = 5;
const IMAGE_INTERVAL: Duration = Duration::from_secs(2);

#[derive(Clone, Copy)]
enum ImagePurpose {
    Send,
    Verify,
}
enum Stage {
    Code {
        captcha: String,
        chinese: bool,
    },
    Image {
        image: crate::client::passport::image::PassportImage,
        purpose: ImagePurpose,
    },
}

struct Context {
    stage: Stage,
    image_attempts: u8,
    refreshes: u8,
    next_image_at: Instant,
    cookies: PassportCookies,
    previous: Option<Selection>,
}
struct Entry {
    receipt: ProviderAuthChallenge,
    created_at: Instant,
    expires_at: Instant,
    attempts: u8,
    context: Option<Context>,
}
#[derive(Default)]
pub(super) struct PassportTransactions {
    entries: BTreeMap<String, Entry>,
    cooldowns: BTreeMap<String, Instant>,
    pub(super) passwords: BTreeMap<String, super::password::Entry>,
    pub(super) secondary_cooldowns: BTreeMap<String, Instant>,
}
impl PassportTransactions {
    pub(super) fn cancel_account(&mut self, account: &str) {
        self.passwords.retain(|_, v| {
            !v.receipt.credential_mode().persists_on_server()
                || v.receipt.identity().account != account
        });
        self.entries.retain(|_, v| {
            !v.receipt.credential_mode().persists_on_server()
                || v.receipt.request().account != account
        });
    }
    pub(super) fn pending_count(&self) -> usize {
        self.entries.len() + self.passwords.len()
    }
    pub(super) fn prune(&mut self) {
        let now = Instant::now();
        self.entries.retain(|_, v| v.expires_at > now);
        self.cooldowns.retain(|_, until| *until > now);
        self.passwords.retain(|_, v| v.expires_at > now);
        self.secondary_cooldowns.retain(|_, until| *until > now);
    }
}

struct Lease {
    store: Arc<Mutex<PassportTransactions>>,
    id: String,
    created_at: Instant,
    armed: bool,
}
fn missing() -> TuneWeaveError {
    error(
        ErrorCode::ResourceNotFound,
        "Migu SMS transaction is missing, expired, or consumed",
    )
}
fn locked() -> TuneWeaveError {
    error(
        ErrorCode::InternalError,
        "Migu SMS transaction state is unavailable",
    )
}
fn changed() -> TuneWeaveError {
    error(ErrorCode::Conflict, "Migu SMS login ownership changed")
}

impl Lease {
    fn check(&self) -> Result<()> {
        let mut store = self.store.lock().map_err(|_| locked())?;
        store.prune();
        if store
            .entries
            .get(&self.id)
            .is_some_and(|v| v.created_at == self.created_at)
        {
            Ok(())
        } else {
            Err(missing())
        }
    }
    fn publish(mut self, context: Context) -> Result<()> {
        let mut store = self.store.lock().map_err(|_| locked())?;
        store.prune();
        let entry = store
            .entries
            .get_mut(&self.id)
            .filter(|v| v.created_at == self.created_at)
            .ok_or_else(missing)?;
        if entry.context.is_some() {
            return Err(changed());
        }
        entry.context = Some(context);
        self.armed = false;
        Ok(())
    }
    fn finish(self) -> Result<()> {
        self.check()
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if self.armed {
            if let Ok(mut store) = self.store.lock() {
                if store
                    .entries
                    .get(&self.id)
                    .is_some_and(|v| v.created_at == self.created_at)
                {
                    store.entries.remove(&self.id);
                }
            }
        }
    }
}

impl MiguProvider {
    fn validate_sms(&self, request: &AuthChallengeRequest, mode: CredentialMode) -> Result<()> {
        request.reject_account_creation_option(Platform::Migu)?;
        request.reject_platform_policies_option(Platform::Migu)?;
        validate_account(&request.account, mode)?;
        if self.caller_credential.is_some()
            || (mode.persists_on_server() && self.credential_store.is_none())
            || request.backend != AuthChallengeBackend::Standard
            || request.principal.len() != 11
            || !request.principal.starts_with('1')
            || !request.principal.bytes().all(|v| v.is_ascii_digit())
            || request
                .country_code
                .as_deref()
                .is_some_and(|v| !matches!(v, "86" | "+86"))
        {
            return Err(migu_invalid_request(
                "Migu SMS requires a mainland phone, standard backend, and explicit base-provider ownership",
            ));
        }
        Ok(())
    }
    fn check_sms_owner(
        &self,
        receipt: &ProviderAuthChallenge,
        previous: &Option<Selection>,
    ) -> Result<()> {
        if receipt.credential_mode().persists_on_server()
            && &self.selected(&receipt.request().account)? != previous
        {
            return Err(changed());
        }
        Ok(())
    }
    pub(super) async fn begin_sms(
        &self,
        request: &AuthChallengeRequest,
        mode: CredentialMode,
    ) -> Result<ProviderAuthChallenge> {
        self.validate_sms(request, mode)?;
        let (receipt, mut context, lease) = {
            let _guard = self.auth_mutation.lock().await;
            let previous = if mode.persists_on_server() {
                self.selected(&request.account)?
            } else {
                None
            };
            let mut store = self.passport_transactions.lock().map_err(|_| locked())?;
            store.prune();
            if let Some(until) = store.cooldowns.get(&request.principal) {
                return Err(error(ErrorCode::RateLimited, "Migu SMS delivery is cooling down")
                    .with_details(json!({"resend_after_secs": until.saturating_duration_since(Instant::now()).as_secs().saturating_add(1)})));
            }
            if store.pending_count() >= CAPACITY || store.cooldowns.len() >= CAPACITY {
                return Err(error(
                    ErrorCode::RateLimited,
                    "Migu SMS transaction capacity is exhausted",
                ));
            }
            let mut id = None;
            for _ in 0..8 {
                let mut bytes = [0; 32];
                SysRng.try_fill_bytes(&mut bytes).map_err(|_| locked())?;
                let candidate = hex::encode(bytes);
                if !store.entries.contains_key(&candidate) {
                    id = Some(candidate);
                    break;
                }
            }
            let id = id.ok_or_else(locked)?;
            let receipt =
                ProviderAuthChallenge::stateful(Platform::Migu, request.clone(), mode, id.clone())?;
            let created_at = Instant::now();
            store.entries.insert(
                id.clone(),
                Entry {
                    receipt: receipt.clone(),
                    created_at,
                    expires_at: created_at + TTL,
                    attempts: 0,
                    context: None,
                },
            );
            store
                .cooldowns
                .insert(request.principal.clone(), created_at + COOLDOWN);
            (
                receipt,
                Context {
                    stage: Stage::Code {
                        captcha: String::new(),
                        chinese: false,
                    },
                    image_attempts: 0,
                    refreshes: 0,
                    next_image_at: created_at,
                    cookies: PassportCookies::default(),
                    previous,
                },
                Lease {
                    store: self.passport_transactions.clone(),
                    id,
                    created_at,
                    armed: true,
                },
            )
        };
        let check = || {
            lease.check()?;
            self.check_sms_owner(&receipt, &context.previous)
        };
        if let Err(e) = self
            .client
            .send_sms(&request.principal, "", &mut context.cookies, &check)
            .await
        {
            if let Some(chinese) = image_required(&e, false) {
                let image = self
                    .client
                    .sms_image(chinese, &mut context.cookies, &check)
                    .await?;
                context.stage = Stage::Image {
                    image,
                    purpose: ImagePurpose::Send,
                };
                context.next_image_at = Instant::now() + IMAGE_INTERVAL;
            } else {
                return Err(e);
            }
        }
        lease.publish(context)?;
        Ok(receipt)
    }

    pub(super) async fn complete_sms(
        &self,
        receipt: &ProviderAuthChallenge,
        code: &str,
    ) -> Result<ProviderAuthResult> {
        match self.submit_sms(receipt, code).await? {
            AuthChallengeProgress::Confirmed(result) => Ok(result),
            AuthChallengeProgress::Pending(_) => Err(error(
                ErrorCode::AuthenticationRequired,
                "Migu SMS requires an image challenge action",
            )
            .with_details(json!({"verification_required":true}))),
        }
    }

    pub(super) async fn submit_sms(
        &self,
        receipt: &ProviderAuthChallenge,
        code: &str,
    ) -> Result<AuthChallengeProgress> {
        self.validate_sms(receipt.request(), receipt.credential_mode())?;
        if receipt.platform() != Platform::Migu
            || !matches!(code.len(), 4 | 6)
            || !code.bytes().all(|v| v.is_ascii_digit())
        {
            return Err(migu_invalid_request(
                "Migu SMS verification requires the original receipt and four or six digits",
            ));
        }
        let id = receipt.provider_transaction_id().ok_or_else(|| {
            migu_invalid_request("Migu SMS verification requires a stateful receipt")
        })?;
        let (mut context, remaining, lease) = {
            let mut store = self.passport_transactions.lock().map_err(|_| locked())?;
            store.prune();
            let entry = store.entries.get_mut(id).ok_or_else(missing)?;
            if &entry.receipt != receipt {
                return Err(migu_invalid_request(
                    "Migu SMS receipt binding does not match",
                ));
            }
            if entry.attempts >= ATTEMPTS {
                return Err(missing());
            }
            if matches!(
                entry.context.as_ref().map(|v| &v.stage),
                Some(Stage::Image { .. })
            ) {
                return Err(migu_invalid_request(
                    "Complete the Migu image challenge before submitting an SMS code",
                ));
            }
            let context = entry.context.take().ok_or_else(|| {
                error(
                    ErrorCode::Conflict,
                    "Migu SMS transaction is already in use",
                )
            })?;
            entry.attempts += 1;
            (
                context,
                ATTEMPTS - entry.attempts,
                Lease {
                    store: self.passport_transactions.clone(),
                    id: id.to_owned(),
                    created_at: entry.created_at,
                    armed: true,
                },
            )
        };
        let _guard = self.auth_mutation.lock().await;
        let check = || {
            lease.check()?;
            self.check_sms_owner(receipt, &context.previous)
        };
        let authentication = self
            .client
            .authenticate_sms(
                &receipt.request().principal,
                code,
                match &context.stage {
                    Stage::Code { captcha, .. } => captcha,
                    Stage::Image { .. } => unreachable!(),
                },
                &mut context.cookies,
                &check,
            )
            .await;
        let token = match authentication {
            Ok(token) => token,
            Err(mut e) => {
                if let Some(chinese) = image_required(
                    &e,
                    matches!(context.stage, Stage::Code { chinese: true, .. }),
                ) {
                    if remaining == 0 || context.image_attempts >= ATTEMPTS {
                        return Err(e.with_consumed_auth_challenge());
                    }
                    let image = self
                        .client
                        .sms_image(chinese, &mut context.cookies, &check)
                        .await
                        .map_err(TuneWeaveError::with_consumed_auth_challenge)?;
                    context.stage = Stage::Image {
                        image,
                        purpose: ImagePurpose::Verify,
                    };
                    context.next_image_at = Instant::now() + IMAGE_INTERVAL;
                    let status = context.status();
                    lease
                        .publish(context)
                        .map_err(TuneWeaveError::with_consumed_auth_challenge)?;
                    return Ok(AuthChallengeProgress::Pending(status));
                }
                if e.code == ErrorCode::AuthenticationRequired
                    && e.details
                        .get("platform_code")
                        .and_then(serde_json::Value::as_str)
                        == Some("4005")
                {
                    e.details["remaining_attempts"] = json!(remaining);
                    if remaining > 0 {
                        lease.publish(context)?;
                        return Err(e);
                    }
                }
                return Err(e.with_consumed_auth_challenge());
            }
        };
        let result = async {
            check()?;
            let exchange = self.client.exchange_passport_token(token).await;
            check()?;
            let (uid, token) = exchange?;
            let read = self
                .client
                .account_profile(&receipt.request().account, &token, Some(&uid))
                .await;
            check()?;
            read
        }
        .await;
        let read = result.map_err(|e: TuneWeaveError| e.with_consumed_auth_challenge())?;
        lease
            .finish()
            .map_err(TuneWeaveError::with_consumed_auth_challenge)?;
        self.commit_new_login(
            &receipt.request().account,
            receipt.credential_mode(),
            context.previous,
            read,
        )
        .map(AuthChallengeProgress::Confirmed)
        .map_err(TuneWeaveError::with_consumed_auth_challenge)
    }
}

#[cfg(test)]
mod tests;

mod image;
use image::image_required;
