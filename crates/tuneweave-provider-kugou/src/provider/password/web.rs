//! Official Web password secondary-phone continuation, using the Web SMS protocol.
use super::*;
use crate::{
    device::KugouDevice,
    web::{password::WebPasswordOutcome, sms::KugouWebSmsChallenge},
};
use tuneweave_core::{
    AuthChallengeAction, AuthChallengeProgress, AuthChallengeStatus, PasswordChallengeAction,
    PasswordLoginIdentity, PasswordLoginProgress, PasswordVerification, ProviderPasswordChallenge,
};

const DELIVERY_INTERVAL: Duration = Duration::from_secs(60);
const MAX_SENDS: u8 = 5;

pub(super) struct Entry {
    receipt: ProviderPasswordChallenge,
    pub(super) phone: Option<String>,
    pub(super) context: Option<Context>,
}
pub(super) struct Context {
    previous: Option<Selection>,
    sms: KugouWebSmsChallenge,
    phone: String,
    sends: u8,
    next_send: tokio::time::Instant,
}

fn invalid() -> TuneWeaveError {
    kugou_invalid_request("Invalid KuGou Web password verification action or binding")
}
fn missing() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::ResourceNotFound,
        "KuGou password challenge is missing, expired, or consumed",
    )
    .with_platform(Platform::Kugou)
    .with_consumed_auth_challenge()
}
fn busy() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::Conflict,
        "KuGou password verification is already in progress",
    )
    .with_platform(Platform::Kugou)
}
fn limited() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::RateLimited,
        "KuGou Web SMS delivery cooldown or limit reached",
    )
    .with_platform(Platform::Kugou)
}

impl KugouProvider {
    pub(in crate::provider) async fn begin_web_password(
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
        let deadline = {
            let mut registry = self.qr_transactions.lock().map_err(|_| state_error())?;
            let attempt = registry.passwords.get_mut(&lease.id).ok_or_else(missing)?;
            attempt.web = Some(Entry {
                receipt: receipt.clone(),
                phone: None,
                context: None,
            });
            attempt.deadline
        };
        let device = KugouDevice::default().identity().into_web();
        self.check_password(&lease, &request.account, mode, &previous)?;
        let outcome = self.client.web_password_attempt(request, &device).await;
        self.check_password(&lease, &request.account, mode, &previous)?;
        match outcome? {
            WebPasswordOutcome::Session(session) => self
                .commit_password(
                    &lease,
                    &request.account,
                    mode,
                    &previous,
                    session.profile().and_then(|profile| {
                        KugouCredential::verified_web(session).map(|c| (profile, c))
                    }),
                )
                .map(PasswordLoginProgress::Confirmed),
            WebPasswordOutcome::Phone(phone) => {
                let mut context = Context {
                    previous,
                    sms: KugouWebSmsChallenge::for_password(
                        phone.clone(),
                        device,
                        deadline,
                        request.principal.clone(),
                    )?,
                    phone,
                    sends: 0,
                    next_send: tokio::time::Instant::now(),
                };
                self.deliver_web_password_sms(&lease, &receipt, &mut context)
                    .await?;
                self.publish_web_password(&receipt, context, lease)
            }
        }
    }

    fn check_web_password(
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

    async fn deliver_web_password_sms(
        &self,
        lease: &Lease,
        receipt: &ProviderPasswordChallenge,
        context: &mut Context,
    ) -> Result<()> {
        self.check_web_password(lease, receipt, context)?;
        {
            let mut registry = self.qr_transactions.lock().map_err(|_| state_error())?;
            registry.prune();
            let now = tokio::time::Instant::now();
            if context.sends >= MAX_SENDS
                || now < context.next_send
                || registry.sms_cooldowns.len() >= 1024
                || registry
                    .sms_cooldowns
                    .get(&context.phone)
                    .is_some_and(|until| now < *until)
            {
                return Err(limited());
            }
            if registry
                .sms
                .values()
                .any(|e| e.challenge.request().principal == context.phone && e.is_busy())
                || registry.passwords.iter().any(|(id, e)| {
                    id != &lease.id
                        && e.web_sms_target()
                            .is_some_and(|(phone, busy)| phone == context.phone && busy)
                })
            {
                return Err(busy());
            }
            let entry = registry
                .passwords
                .get_mut(&lease.id)
                .and_then(|a| a.web.as_mut())
                .ok_or_else(missing)?;
            entry.phone = Some(context.phone.clone());
            // A new code retires older receipts for this phone across both Web entry points.
            registry
                .sms
                .retain(|_, e| e.challenge.request().principal != context.phone);
            registry.passwords.retain(|id, e| {
                id == &lease.id
                    || !e
                        .web_sms_target()
                        .is_some_and(|(phone, _)| phone == context.phone)
            });
            context.next_send = now + DELIVERY_INTERVAL;
            context.sends += 1;
            registry
                .sms_cooldowns
                .insert(context.phone.clone(), context.next_send);
        }
        let result = self.client.resend_web_sms(&mut context.sms).await;
        if let Err(error) = &result {
            if error.code == ErrorCode::RateLimited {
                let delay = error.details["retry_after_secs"]
                    .as_u64()
                    .unwrap_or(2)
                    .clamp(2, 300);
                let mut registry = self.qr_transactions.lock().map_err(|_| state_error())?;
                if let Some(until) = registry.sms_cooldowns.get_mut(&context.phone) {
                    *until = (*until).max(tokio::time::Instant::now() + Duration::from_secs(delay));
                }
            }
        }
        self.check_web_password(lease, receipt, context)?;
        result
    }

    fn publish_web_password(
        &self,
        receipt: &ProviderPasswordChallenge,
        context: Context,
        mut lease: Lease,
    ) -> Result<PasswordLoginProgress> {
        self.check_web_password(&lease, receipt, &context)?;
        let remaining_attempts = context.sms.remaining_attempts();
        let verification = match context.sms.status()? {
            AuthChallengeStatus::Waiting => PasswordVerification::Sms {
                masked_destination: format!("{}*****{}", &context.phone[..3], &context.phone[8..]),
                remaining_attempts,
                resend_after_secs: context
                    .next_send
                    .saturating_duration_since(tokio::time::Instant::now())
                    .as_millis()
                    .div_ceil(1000) as u64,
            },
            AuthChallengeStatus::AccountSelectionRequired { accounts } => {
                PasswordVerification::AccountSelection {
                    accounts,
                    remaining_attempts,
                }
            }
            AuthChallengeStatus::BrowserVerificationRequired { verification } => {
                PasswordVerification::SmsBrowser { verification }
            }
            _ => return Err(state_error()),
        };
        let mut registry = self.qr_transactions.lock().map_err(|_| state_error())?;
        registry.prune();
        let entry = registry
            .passwords
            .get_mut(&lease.id)
            .and_then(|a| a.web.as_mut())
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

    pub(in crate::provider) async fn advance_web_password(
        &self,
        receipt: &ProviderPasswordChallenge,
        action: &PasswordChallengeAction,
    ) -> Result<PasswordLoginProgress> {
        self.require_login_mode(receipt.credential_mode())?;
        if receipt.platform() != Platform::Kugou
            || !matches!(
                receipt.identity().backend,
                PasswordLoginBackend::Default | PasswordLoginBackend::Web
            )
        {
            return Err(invalid());
        }
        let sms_action = match action {
            PasswordChallengeAction::SubmitSms { code } => {
                Some(AuthChallengeAction::SubmitCode { code: code.clone() })
            }
            PasswordChallengeAction::SelectAccount { user_id, code } => {
                Some(AuthChallengeAction::SelectAccount {
                    user_id: user_id.clone(),
                    code: code.clone(),
                })
            }
            PasswordChallengeAction::SubmitSmsBrowser {
                verification_id,
                code,
                response,
            } => Some(AuthChallengeAction::SubmitBrowser {
                verification_id: verification_id.clone(),
                code: code.clone(),
                response: response.clone(),
            }),
            PasswordChallengeAction::ResendSms => None,
            _ => return Err(invalid()),
        };
        let (mut context, lease) = {
            let mut registry = self.qr_transactions.lock().map_err(|_| state_error())?;
            registry.prune();
            let entry = registry
                .passwords
                .get_mut(receipt.provider_transaction_id())
                .and_then(|a| a.web.as_mut())
                .ok_or_else(missing)?;
            if &entry.receipt != receipt {
                return Err(invalid());
            }
            let context = entry.context.as_ref().ok_or_else(busy)?;
            if sms_action.is_none()
                && (context.sends >= MAX_SENDS || tokio::time::Instant::now() < context.next_send)
            {
                return Err(limited());
            }
            (
                entry.context.take().ok_or_else(busy)?,
                Lease {
                    registry: self.qr_transactions.clone(),
                    id: receipt.provider_transaction_id().into(),
                    armed: true,
                },
            )
        };
        self.check_web_password(&lease, receipt, &context)
            .map_err(TuneWeaveError::with_consumed_auth_challenge)?;
        let result = match sms_action {
            Some(action) => {
                let check = || {
                    self.check_password(
                        &lease,
                        &receipt.identity().account,
                        receipt.credential_mode(),
                        &context.previous,
                    )
                };
                self.client
                    .advance_web_sms_guarded(&mut context.sms, &action, &check)
                    .await
            }
            None => self
                .deliver_web_password_sms(&lease, receipt, &mut context)
                .await
                .map(|()| AuthChallengeProgress::Pending(AuthChallengeStatus::Waiting)),
        };
        self.check_web_password(&lease, receipt, &context)
            .map_err(TuneWeaveError::with_consumed_auth_challenge)?;
        match result {
            Ok(AuthChallengeProgress::Confirmed(result)) => {
                let credential = KugouCredential::parse_caller(
                    result.credential.as_ref().ok_or_else(state_error)?,
                );
                self.commit_password(
                    &lease,
                    &receipt.identity().account,
                    receipt.credential_mode(),
                    &context.previous,
                    credential.map(|c| (result.profile, c)),
                )
                .map(PasswordLoginProgress::Confirmed)
                .map_err(TuneWeaveError::with_consumed_auth_challenge)
            }
            Ok(AuthChallengeProgress::Pending(_)) => {
                self.publish_web_password(receipt, context, lease)
            }
            Err(error) => {
                if context.sms.status().is_ok() {
                    self.publish_web_password(receipt, context, lease)?;
                    Err(error)
                } else {
                    Err(error.with_consumed_auth_challenge())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
