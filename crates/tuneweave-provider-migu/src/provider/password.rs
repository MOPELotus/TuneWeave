use super::*;
use super::{
    session::Selection,
    sms::{CAPACITY, COOLDOWN, PassportTransactions, TTL},
};
use crate::{
    client::passport::{
        image::{ImageScene, PassportImage},
        password::{PasswordOutcome, SecondaryIdentity},
    },
    credential::error,
    passport::cookies::PassportCookies,
};
use rand::{TryRng, rngs::SysRng};
use std::time::{Duration, Instant};
use tuneweave_core::{
    AuthImageAnswerKind, AuthImageChallenge, ErrorCode, PasswordChallengeAction,
    PasswordLoginIdentity, PasswordLoginProgress, PasswordLoginRequest, PasswordVerification,
    ProviderPasswordChallenge,
};

const ATTEMPTS: u8 = 5;
const SECONDARY_SENDS: u8 = 3;
const IMAGE_INTERVAL: Duration = Duration::from_secs(2);

enum Stage {
    Starting,
    Image(PassportImage),
    Sms(SecondaryIdentity),
    Voice(SecondaryIdentity),
}
struct Context {
    cookies: PassportCookies,
    previous: Option<Selection>,
    stage: Stage,
    image_attempts: u8,
    refreshes: u8,
    secondary_attempts: u8,
    secondary_sends: u8,
    next_image_at: Instant,
    next_secondary_at: Instant,
}
pub(super) struct Entry {
    pub(super) receipt: ProviderPasswordChallenge,
    created_at: Instant,
    pub(super) expires_at: Instant,
    context: Option<Context>,
}
struct Lease {
    store: Arc<Mutex<PassportTransactions>>,
    id: String,
    created_at: Instant,
    armed: bool,
}
fn locked() -> TuneWeaveError {
    error(
        ErrorCode::InternalError,
        "Migu password transaction state is unavailable",
    )
}
fn missing() -> TuneWeaveError {
    error(
        ErrorCode::ResourceNotFound,
        "Migu password transaction is missing, expired, or consumed",
    )
}
fn changed() -> TuneWeaveError {
    error(ErrorCode::Conflict, "Migu password login ownership changed")
}
impl Lease {
    fn check(&self) -> Result<()> {
        let mut store = self.store.lock().map_err(|_| locked())?;
        store.prune();
        if store
            .passwords
            .get(&self.id)
            .is_some_and(|e| e.created_at == self.created_at)
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
            .passwords
            .get_mut(&self.id)
            .filter(|e| e.created_at == self.created_at)
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
                    .passwords
                    .get(&self.id)
                    .is_some_and(|e| e.created_at == self.created_at)
                {
                    store.passwords.remove(&self.id);
                }
            }
        }
    }
}
fn verification(context: &Context) -> Result<PasswordVerification> {
    match &context.stage {
        Stage::Image(image) => Ok(PasswordVerification::Image {
            image: AuthImageChallenge {
                image_data_url: image.data_url.clone(),
                answer_kind: image.kind,
                remaining_attempts: ATTEMPTS.saturating_sub(context.image_attempts),
                refresh_after_secs: context
                    .next_image_at
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .div_ceil(1000) as u64,
            },
        }),
        Stage::Sms(identity) => Ok(PasswordVerification::Sms {
            masked_destination: identity.masked.clone(),
            remaining_attempts: ATTEMPTS.saturating_sub(context.secondary_attempts),
            resend_after_secs: context
                .next_secondary_at
                .saturating_duration_since(Instant::now())
                .as_millis()
                .div_ceil(1000) as u64,
        }),
        Stage::Voice(identity) => Ok(PasswordVerification::Voice {
            masked_destination: identity.masked.clone(),
            remaining_attempts: ATTEMPTS.saturating_sub(context.secondary_attempts),
            resend_after_secs: context
                .next_secondary_at
                .saturating_duration_since(Instant::now())
                .as_millis()
                .div_ceil(1000) as u64,
        }),
        Stage::Starting => Err(locked()),
    }
}
impl MiguProvider {
    fn check_password_owner(
        &self,
        receipt: &ProviderPasswordChallenge,
        previous: &Option<Selection>,
    ) -> Result<()> {
        if receipt.credential_mode().persists_on_server()
            && &self.selected(&receipt.identity().account)? != previous
        {
            return Err(changed());
        }
        Ok(())
    }
    pub(super) async fn begin_password(
        &self,
        request: &PasswordLoginRequest,
        mode: CredentialMode,
    ) -> Result<PasswordLoginProgress> {
        self.validate_password_input(request, mode)?;
        let _guard = self.auth_mutation.lock().await;
        let previous = if mode.persists_on_server() {
            self.selected(&request.account)?
        } else {
            None
        };
        let (receipt, lease) = {
            let mut store = self.passport_transactions.lock().map_err(|_| locked())?;
            store.prune();
            if store.pending_count() >= CAPACITY {
                return Err(error(
                    ErrorCode::RateLimited,
                    "Migu authentication capacity is exhausted",
                ));
            }
            let mut id = None;
            for _ in 0..8 {
                let mut bytes = [0; 32];
                SysRng.try_fill_bytes(&mut bytes).map_err(|_| locked())?;
                let candidate = hex::encode(bytes);
                if !store.passwords.contains_key(&candidate) {
                    id = Some(candidate);
                    break;
                }
            }
            let id = id.ok_or_else(locked)?;
            let now = Instant::now();
            let receipt = ProviderPasswordChallenge::new(
                Platform::Migu,
                PasswordLoginIdentity::from(request),
                mode,
                id.clone(),
            )?;
            store.passwords.insert(
                id.clone(),
                Entry {
                    receipt: receipt.clone(),
                    created_at: now,
                    expires_at: now + TTL,
                    context: None,
                },
            );
            (
                receipt,
                Lease {
                    store: self.passport_transactions.clone(),
                    id,
                    created_at: now,
                    armed: true,
                },
            )
        };
        let mut context = Context {
            cookies: PassportCookies::default(),
            previous,
            stage: Stage::Starting,
            image_attempts: 0,
            refreshes: 0,
            secondary_attempts: 0,
            secondary_sends: 0,
            next_image_at: Instant::now(),
            next_secondary_at: Instant::now(),
        };
        let check = || {
            lease.check()?;
            self.check_password_owner(&receipt, &context.previous)
        };
        let outcome = self
            .client
            .password_outcome(
                &request.principal,
                &request.password,
                "",
                &mut context.cookies,
                &check,
            )
            .await?;
        self.password_outcome(&receipt, context, lease, outcome)
            .await
    }

    fn reserve_secondary_send(
        &self,
        receipt: &ProviderPasswordChallenge,
        context: &mut Context,
    ) -> Result<()> {
        if context.secondary_sends >= SECONDARY_SENDS || context.next_secondary_at > Instant::now()
        {
            return Err(error(
                ErrorCode::RateLimited,
                "Migu secondary verification resend limit reached",
            ));
        }
        let mut store = self.passport_transactions.lock().map_err(|_| locked())?;
        store.prune();
        if store
            .secondary_cooldowns
            .contains_key(&receipt.identity().principal)
            || store.secondary_cooldowns.len() >= CAPACITY
        {
            return Err(error(
                ErrorCode::RateLimited,
                "Migu secondary verification delivery is cooling down",
            ));
        }
        context.secondary_sends += 1;
        context.next_secondary_at = Instant::now() + COOLDOWN;
        store.secondary_cooldowns.insert(
            receipt.identity().principal.clone(),
            context.next_secondary_at,
        );
        Ok(())
    }

    async fn password_outcome(
        &self,
        receipt: &ProviderPasswordChallenge,
        mut context: Context,
        lease: Lease,
        outcome: PasswordOutcome,
    ) -> Result<PasswordLoginProgress> {
        match outcome {
            PasswordOutcome::Token(token) => {
                let check = || {
                    lease.check()?;
                    self.check_password_owner(receipt, &context.previous)
                };
                check()?;
                let exchanged = self.client.exchange_passport_token(token).await;
                check()?;
                let (uid, token) = exchanged?;
                let read = self
                    .client
                    .account_profile(&receipt.identity().account, &token, Some(&uid))
                    .await;
                check()?;
                let read = read?;
                lease.finish()?;
                return self
                    .commit_new_login(
                        &receipt.identity().account,
                        receipt.credential_mode(),
                        context.previous,
                        read,
                    )
                    .map(PasswordLoginProgress::Confirmed);
            }
            PasswordOutcome::Image { chinese } => {
                if context.image_attempts >= ATTEMPTS {
                    return Err(error(
                        ErrorCode::RateLimited,
                        "Migu password image attempts exhausted",
                    ));
                }
                let chinese=chinese.unwrap_or(matches!(&context.stage,Stage::Image(image) if image.kind==AuthImageAnswerKind::Chinese));
                let check = || {
                    lease.check()?;
                    self.check_password_owner(receipt, &context.previous)
                };
                let image = self
                    .client
                    .passport_image(ImageScene::Password, chinese, &mut context.cookies, &check)
                    .await?;
                context.stage = Stage::Image(image);
                context.next_image_at = Instant::now() + IMAGE_INTERVAL;
            }
            PasswordOutcome::Secondary(identity) => {
                self.reserve_secondary_send(receipt, &mut context)?;
                let check = || {
                    lease.check()?;
                    self.check_password_owner(receipt, &context.previous)
                };
                self.client
                    .send_secondary_sms(&identity, &mut context.cookies, &check)
                    .await?;
                context.stage = Stage::Sms(identity);
            }
            PasswordOutcome::Voice(identity) => {
                self.reserve_secondary_send(receipt, &mut context)?;
                let check = || {
                    lease.check()?;
                    self.check_password_owner(receipt, &context.previous)
                };
                self.client
                    .send_secondary_voice(&identity, &mut context.cookies, &check)
                    .await?;
                context.stage = Stage::Voice(identity);
            }
        }
        let verification = verification(&context)?;
        lease.publish(context)?;
        Ok(PasswordLoginProgress::Pending {
            challenge: receipt.clone(),
            verification,
        })
    }

    pub(super) async fn advance_password(
        &self,
        receipt: &ProviderPasswordChallenge,
        action: &PasswordChallengeAction,
    ) -> Result<PasswordLoginProgress> {
        if self.caller_credential.is_some() || receipt.platform() != Platform::Migu {
            return Err(migu_invalid_request(
                "Invalid Migu password challenge ownership",
            ));
        }
        let (mut context, lease) = {
            let mut store = self.passport_transactions.lock().map_err(|_| locked())?;
            store.prune();
            let entry = store
                .passwords
                .get_mut(receipt.provider_transaction_id())
                .ok_or_else(missing)?;
            if &entry.receipt != receipt {
                return Err(migu_invalid_request(
                    "Migu password receipt binding does not match",
                ));
            }
            let context = entry.context.as_ref().ok_or_else(|| {
                error(
                    ErrorCode::Conflict,
                    "Migu password transaction is already in use",
                )
            })?;
            match (&context.stage, action) {
                (
                    Stage::Image(image),
                    PasswordChallengeAction::SubmitImage { answer, password },
                ) => {
                    image.validate_answer(answer)?;
                    let identity = receipt.identity();
                    self.validate_password_input(
                        &PasswordLoginRequest {
                            backend: identity.backend,
                            account: identity.account.clone(),
                            principal_type: identity.principal_type,
                            principal: identity.principal.clone(),
                            password: password.clone(),
                            password_format: identity.password_format,
                            country_code: identity.country_code.clone(),
                            secure_captcha: None,
                        },
                        receipt.credential_mode(),
                    )?;
                }
                (Stage::Image(_), PasswordChallengeAction::RefreshImage) => {
                    if context.refreshes >= ATTEMPTS || context.next_image_at > Instant::now() {
                        return Err(error(
                            ErrorCode::RateLimited,
                            "Migu password image refresh limit reached",
                        ));
                    }
                }
                (Stage::Sms(_), PasswordChallengeAction::SubmitSms { code }) => {
                    if !matches!(code.len(), 4 | 6) || !code.bytes().all(|b| b.is_ascii_digit()) {
                        return Err(migu_invalid_request(
                            "Migu secondary SMS requires four or six digits",
                        ));
                    }
                }
                (Stage::Voice(_), PasswordChallengeAction::SubmitVoice { code }) => {
                    if !matches!(code.len(), 4 | 6) || !code.bytes().all(|b| b.is_ascii_digit()) {
                        return Err(migu_invalid_request(
                            "Migu voice verification requires four or six digits",
                        ));
                    }
                }
                (Stage::Sms(_), PasswordChallengeAction::ResendSms)
                | (Stage::Voice(_), PasswordChallengeAction::ResendVoice) => {
                    if context.secondary_sends >= SECONDARY_SENDS
                        || context.next_secondary_at > Instant::now()
                    {
                        return Err(error(
                            ErrorCode::RateLimited,
                            "Migu secondary verification resend limit reached",
                        ));
                    }
                }
                _ => {
                    return Err(migu_invalid_request(
                        "Action does not match the Migu password verification stage",
                    ));
                }
            }
            (
                entry.context.take().ok_or_else(locked)?,
                Lease {
                    store: self.passport_transactions.clone(),
                    id: receipt.provider_transaction_id().into(),
                    created_at: entry.created_at,
                    armed: true,
                },
            )
        };
        let _guard = self.auth_mutation.lock().await;
        // Once an action owns the context, uncertain outcomes consume it. An explicitly
        // rejected SMS code is the only error path that republishes its rotated cookies.
        let result = async {
            lease.check()?;
            self.check_password_owner(receipt, &context.previous)?;
            let outcome = match action {
                PasswordChallengeAction::SubmitImage { answer, password } => {
                    context.image_attempts += 1;
                    let check = || {
                        lease.check()?;
                        self.check_password_owner(receipt, &context.previous)
                    };
                    match self
                        .client
                        .check_passport_image(
                            ImageScene::Password,
                            answer,
                            &mut context.cookies,
                            &check,
                        )
                        .await
                    {
                        Ok(()) => {
                            self.client
                                .password_outcome(
                                    &receipt.identity().principal,
                                    password,
                                    answer,
                                    &mut context.cookies,
                                    &check,
                                )
                                .await?
                        }
                        Err(e) => match e
                            .details
                            .get("platform_code")
                            .and_then(serde_json::Value::as_str)
                        {
                            Some("4002") => PasswordOutcome::Image { chinese: None },
                            Some("4044") => PasswordOutcome::Image {
                                chinese: Some(false),
                            },
                            Some("4045") => PasswordOutcome::Image {
                                chinese: Some(true),
                            },
                            _ => return Err(e),
                        },
                    }
                }
                PasswordChallengeAction::RefreshImage => {
                    context.refreshes += 1;
                    PasswordOutcome::Image { chinese: None }
                }
                PasswordChallengeAction::SubmitSms { code } => {
                    context.secondary_attempts += 1;
                    let Stage::Sms(identity) = &context.stage else {
                        return Err(locked());
                    };
                    let check = || {
                        lease.check()?;
                        self.check_password_owner(receipt, &context.previous)
                    };
                    match self
                        .client
                        .verify_secondary_sms(identity, code, &mut context.cookies, &check)
                        .await
                    {
                        Ok(token) => PasswordOutcome::Token(token),
                        Err(mut e) => {
                            if e.code == ErrorCode::AuthenticationRequired
                                && e.details
                                    .get("platform_code")
                                    .and_then(serde_json::Value::as_str)
                                    == Some("4005")
                            {
                                e.details["remaining_attempts"] =
                                    json!(ATTEMPTS.saturating_sub(context.secondary_attempts));
                                if context.secondary_attempts < ATTEMPTS {
                                    lease.publish(context)?;
                                    return Ok(Err(e));
                                }
                            }
                            return Err(e);
                        }
                    }
                }
                PasswordChallengeAction::SubmitVoice { code } => {
                    context.secondary_attempts += 1;
                    let Stage::Voice(identity) = &context.stage else {
                        return Err(locked());
                    };
                    let check = || {
                        lease.check()?;
                        self.check_password_owner(receipt, &context.previous)
                    };
                    // The voice callback does not identify a retryable rejection
                    // code. In particular, do not import SMS's 4005 semantics.
                    let token = self
                        .client
                        .verify_secondary_voice(identity, code, &mut context.cookies, &check)
                        .await?;
                    PasswordOutcome::Token(token)
                }
                PasswordChallengeAction::SubmitBrowser { .. }
                | PasswordChallengeAction::SubmitSmsBrowser { .. }
                | PasswordChallengeAction::PrepareSlider { .. }
                | PasswordChallengeAction::RefreshSlider { .. }
                | PasswordChallengeAction::SubmitSlider { .. }
                | PasswordChallengeAction::SelectAccount { .. } => {
                    return Err(migu_invalid_request(
                        "Unsupported password verification action",
                    ));
                }
                PasswordChallengeAction::ResendSms | PasswordChallengeAction::ResendVoice => {
                    self.reserve_secondary_send(receipt, &mut context)?;
                    let check = || {
                        lease.check()?;
                        self.check_password_owner(receipt, &context.previous)
                    };
                    match &context.stage {
                        Stage::Sms(identity) => {
                            self.client
                                .send_secondary_sms(identity, &mut context.cookies, &check)
                                .await?;
                        }
                        Stage::Voice(identity) => {
                            self.client
                                .send_secondary_voice(identity, &mut context.cookies, &check)
                                .await?;
                        }
                        _ => return Err(locked()),
                    }
                    let verification = verification(&context)?;
                    lease.publish(context)?;
                    return Ok(Ok(PasswordLoginProgress::Pending {
                        challenge: receipt.clone(),
                        verification,
                    }));
                }
            };
            self.password_outcome(receipt, context, lease, outcome)
                .await
                .map(Ok)
        }
        .await;
        result.map_err(TuneWeaveError::with_consumed_auth_challenge)?
    }
}

#[cfg(test)]
mod tests;
