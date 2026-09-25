//! Native image, browser, and SMS verification bound to one password login.
use super::*;
use crate::{
    KugouNativePasswordChallenge, KugouNativePasswordChallengeKind,
    device::{KugouDevice, KugouDeviceIdentity},
    login::native_password::{CLIENT_VERSION, NativePasswordAnswer, NativePasswordOutcome},
};
use tuneweave_core::{
    AuthImageAnswerKind, AuthImageChallenge, PasswordBrowserProtocol, PasswordChallengeAction,
    PasswordLoginIdentity, PasswordLoginProgress, PasswordVerification, ProviderPasswordChallenge,
};

const ATTEMPTS: u8 = 5;
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

pub(super) struct Entry {
    receipt: ProviderPasswordChallenge,
    context: Option<Context>,
}

struct Context {
    device: KugouDeviceIdentity,
    previous: Option<Selection>,
    stage: Option<Stage>,
    attempts: u8,
    refreshes: u8,
    next_image_at: Instant,
}

// No password, browser answer, or SMS code is retained in any stage.
enum Stage {
    Image(KugouNativePasswordChallenge),
    Browser {
        challenge: KugouNativePasswordChallenge,
        verification_id: String,
    },
    Sms(sms::Sms),
}

mod browser;
mod sms;

fn missing() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::ResourceNotFound,
        "KuGou password challenge is missing, expired, or consumed",
    )
    .with_platform(Platform::Kugou)
}

fn invalid() -> TuneWeaveError {
    TuneWeaveError::invalid_request("Invalid KuGou password verification action or binding")
        .with_platform(Platform::Kugou)
}

fn limited() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::RateLimited,
        "KuGou password verification limit reached",
    )
    .with_platform(Platform::Kugou)
}

fn request_for(receipt: &ProviderPasswordChallenge, password: &str) -> PasswordLoginRequest {
    let identity = receipt.identity();
    PasswordLoginRequest {
        backend: identity.backend,
        account: identity.account.clone(),
        principal_type: identity.principal_type,
        principal: identity.principal.clone(),
        password: password.to_owned(),
        password_format: identity.password_format,
        country_code: identity.country_code.clone(),
        secure_captcha: None,
    }
}

impl KugouProvider {
    pub(in crate::provider) async fn begin_native_password(
        &self,
        request: &PasswordLoginRequest,
        mode: CredentialMode,
    ) -> Result<PasswordLoginProgress> {
        let (lease, previous) = self.reserve_password(request, mode)?;
        let receipt = ProviderPasswordChallenge::new(
            Platform::Kugou,
            PasswordLoginIdentity::from(request),
            mode,
            lease.id.clone(),
        )?;
        {
            let mut registry = self.qr_transactions.lock().map_err(|_| state_error())?;
            registry
                .passwords
                .get_mut(&lease.id)
                .ok_or_else(missing)?
                .native = Some(Entry {
                receipt: receipt.clone(),
                context: None,
            });
        }
        let context = Context {
            device: KugouDevice::default().identity(),
            previous,
            stage: None,
            attempts: 0,
            refreshes: 0,
            next_image_at: Instant::now(),
        };
        self.check_native_password(&lease, &receipt, &context)?;
        let outcome = self
            .client
            .native_password_attempt(request, &context.device, None)
            .await;
        self.check_native_password(&lease, &receipt, &context)?;
        self.finish_native_password(&receipt, context, lease, outcome?)
            .await
    }

    fn check_native_password(
        &self,
        lease: &Lease,
        receipt: &ProviderPasswordChallenge,
        context: &Context,
    ) -> Result<()> {
        self.check_password(
            lease,
            &receipt.identity().account,
            receipt.credential_mode(),
            &context.previous,
        )
    }

    async fn fetch_native_password_verification(
        &self,
        lease: &Lease,
        receipt: &ProviderPasswordChallenge,
        context: &Context,
        kind: KugouNativePasswordChallengeKind,
    ) -> Result<Stage> {
        self.check_native_password(lease, receipt, context)?;
        let challenge = self
            .client
            .native_password_challenge_for_version(kind, CLIENT_VERSION)
            .await;
        self.check_native_password(lease, receipt, context)?;
        let challenge = challenge?;
        // The official consumer gives browser verification priority over an image.
        if let Some(target) = challenge.browser_target() {
            browser::page_url(target)?;
            let mut random = [0; 32];
            SysRng
                .try_fill_bytes(&mut random)
                .map_err(|_| state_error())?;
            return Ok(Stage::Browser {
                challenge,
                verification_id: hex::encode(random),
            });
        }
        if challenge.verify_key().is_none() || challenge.image_data_url().is_none() {
            return Err(TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "KuGou password image omitted its image or verification key",
            )
            .with_platform(Platform::Kugou));
        }
        Ok(Stage::Image(challenge))
    }

    fn publish_native_password(
        &self,
        receipt: &ProviderPasswordChallenge,
        context: Context,
        mut lease: Lease,
    ) -> Result<PasswordLoginProgress> {
        self.check_native_password(&lease, receipt, &context)?;
        let remaining_attempts = ATTEMPTS.saturating_sub(context.attempts);
        let verification = match context.stage.as_ref().ok_or_else(state_error)? {
            Stage::Image(image) => PasswordVerification::Image {
                image: AuthImageChallenge {
                    image_data_url: image.image_data_url().ok_or_else(state_error)?.to_owned(),
                    answer_kind: AuthImageAnswerKind::Alphanumeric,
                    remaining_attempts,
                    refresh_after_secs: context
                        .next_image_at
                        .saturating_duration_since(Instant::now())
                        .as_millis()
                        .div_ceil(1000) as u64,
                },
            },
            Stage::Browser {
                challenge,
                verification_id,
            } => PasswordVerification::Browser {
                protocol: PasswordBrowserProtocol::KugouNativeBridge,
                verification_id: verification_id.clone(),
                url: browser::page_url(challenge.browser_target().ok_or_else(state_error)?)?,
                device_id: context.device.mid.clone(),
                client_version: CLIENT_VERSION,
                remaining_attempts,
            },
            Stage::Sms(sms) => sms.verification(remaining_attempts),
        };
        let mut registry = self.qr_transactions.lock().map_err(|_| state_error())?;
        registry.prune();
        let entry = registry
            .passwords
            .get_mut(&lease.id)
            .and_then(|entry| entry.native.as_mut())
            .ok_or_else(missing)?;
        if &entry.receipt != receipt || entry.context.is_some() {
            return Err(changed());
        }
        entry.context = Some(context);
        lease.armed = false;
        Ok(PasswordLoginProgress::Pending {
            challenge: receipt.clone(),
            verification,
        })
    }

    async fn finish_native_password(
        &self,
        receipt: &ProviderPasswordChallenge,
        mut context: Context,
        lease: Lease,
        outcome: NativePasswordOutcome,
    ) -> Result<PasswordLoginProgress> {
        match outcome {
            NativePasswordOutcome::Phone { target, .. } => {
                return self
                    .start_native_secondary(receipt, context, lease, target)
                    .await;
            }
            NativePasswordOutcome::Session(session) => {
                self.check_native_password(&lease, receipt, &context)?;
                let profile = self.client.native_profile(&session).await;
                let result = profile.and_then(|profile| {
                    KugouCredential::verified(*session).map(|credential| (profile, credential))
                });
                return self
                    .commit_password(
                        &lease,
                        &receipt.identity().account,
                        receipt.credential_mode(),
                        &context.previous,
                        result,
                    )
                    .map(PasswordLoginProgress::Confirmed);
            }
            NativePasswordOutcome::Verification { kind, .. } => {
                if context.attempts >= ATTEMPTS {
                    return Err(limited());
                }
                context.stage = Some(
                    self.fetch_native_password_verification(&lease, receipt, &context, kind)
                        .await?,
                );
                context.next_image_at = Instant::now() + REFRESH_INTERVAL;
            }
        }
        self.publish_native_password(receipt, context, lease)
    }

    pub(in crate::provider) async fn advance_native_password(
        &self,
        receipt: &ProviderPasswordChallenge,
        action: &PasswordChallengeAction,
    ) -> Result<PasswordLoginProgress> {
        self.require_login_mode(receipt.credential_mode())?;
        if receipt.platform() != Platform::Kugou
            || receipt.identity().backend != PasswordLoginBackend::Native
        {
            return Err(invalid());
        }
        let (mut context, lease, browser_ticket) = {
            let mut registry = self.qr_transactions.lock().map_err(|_| state_error())?;
            registry.prune();
            let entry = registry
                .passwords
                .get_mut(receipt.provider_transaction_id())
                .and_then(|entry| entry.native.as_mut())
                .ok_or_else(missing)?;
            if &entry.receipt != receipt {
                return Err(invalid());
            }
            let context = entry.context.as_ref().ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::Conflict,
                    "KuGou password verification is already in progress",
                )
                .with_platform(Platform::Kugou)
            })?;
            let stage = context.stage.as_ref().ok_or_else(state_error)?;
            let mut browser_ticket = None;
            match (stage, action) {
                (
                    _,
                    PasswordChallengeAction::SubmitVoice { .. }
                    | PasswordChallengeAction::ResendVoice,
                ) => return Err(invalid()),
                (Stage::Image(_), PasswordChallengeAction::SubmitImage { answer, password }) => {
                    if answer.is_empty()
                        || answer.len() > 64
                        || !answer.bytes().all(|b| b.is_ascii_alphanumeric())
                    {
                        return Err(invalid());
                    }
                    if context.attempts >= ATTEMPTS {
                        return Err(limited());
                    }
                    crate::login::native_password::validate(&request_for(receipt, password))?;
                }
                (
                    Stage::Browser {
                        verification_id: expected,
                        ..
                    },
                    PasswordChallengeAction::SubmitBrowser {
                        verification_id,
                        response,
                        password,
                    },
                ) => {
                    if verification_id != expected {
                        return Err(invalid());
                    }
                    if context.attempts >= ATTEMPTS {
                        return Err(limited());
                    }
                    crate::login::native_password::validate(&request_for(receipt, password))?;
                    browser_ticket = Some(browser::ticket(response)?);
                }
                (Stage::Image(_), PasswordChallengeAction::RefreshImage) => {
                    if context.refreshes >= ATTEMPTS || Instant::now() < context.next_image_at {
                        return Err(limited());
                    }
                }
                (Stage::Sms(sms), action) => sms.validate(action, context.attempts)?,
                _ => return Err(invalid()),
            }
            (
                entry.context.take().ok_or_else(state_error)?,
                Lease {
                    registry: self.qr_transactions.clone(),
                    id: receipt.provider_transaction_id().into(),
                    armed: true,
                },
                browser_ticket,
            )
        };
        // Once network work can start, errors or cancellation consume the provider receipt.
        let result = async {
            self.check_native_password(&lease, receipt, &context)?;
            match action {
                PasswordChallengeAction::SubmitSmsBrowser { .. }
                | PasswordChallengeAction::SubmitVoice { .. }
                | PasswordChallengeAction::ResendVoice
                | PasswordChallengeAction::PrepareSlider { .. }
                | PasswordChallengeAction::RefreshSlider { .. }
                | PasswordChallengeAction::SubmitSlider { .. } => Err(invalid()),
                PasswordChallengeAction::SubmitImage { answer, password }
                | PasswordChallengeAction::SubmitBrowser {
                    response: answer,
                    password,
                    ..
                } => {
                    context.attempts += 1;
                    let challenge = match context.stage.as_ref().ok_or_else(state_error)? {
                        Stage::Image(image) => image,
                        Stage::Browser { challenge, .. } => challenge,
                        Stage::Sms(_) => return Err(invalid()),
                    };
                    let answer = browser_ticket.as_deref().unwrap_or(answer);
                    let outcome = self
                        .client
                        .native_password_attempt(
                            &request_for(receipt, password),
                            &context.device,
                            Some(NativePasswordAnswer {
                                key: challenge.verify_key().unwrap_or_default(),
                                answer,
                            }),
                        )
                        .await;
                    self.check_native_password(&lease, receipt, &context)?;
                    self.finish_native_password(receipt, context, lease, outcome?)
                        .await
                }
                PasswordChallengeAction::RefreshImage => {
                    context.refreshes += 1;
                    // Image dialog refresh uses codetype=0, as in the native consumer.
                    context.stage = Some(
                        self.fetch_native_password_verification(
                            &lease,
                            receipt,
                            &context,
                            KugouNativePasswordChallengeKind::Image,
                        )
                        .await?,
                    );
                    context.next_image_at = Instant::now() + REFRESH_INTERVAL;
                    self.publish_native_password(receipt, context, lease)
                }
                PasswordChallengeAction::SubmitSms { .. }
                | PasswordChallengeAction::ResendSms
                | PasswordChallengeAction::SelectAccount { .. } => {
                    self.advance_native_secondary(receipt, context, lease, action)
                        .await
                }
            }
        }
        .await;
        result.map_err(TuneWeaveError::with_consumed_auth_challenge)
    }
}

#[cfg(test)]
mod tests;
