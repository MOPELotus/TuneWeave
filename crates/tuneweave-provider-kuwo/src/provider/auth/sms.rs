use super::*;
use crate::client::KuwoWebSmsChallenge;
use crate::client::native::sms::{
    self as native_sms, KuwoNativeSmsChallenge, KuwoNativeSmsRequest,
};
use rand::{TryRng, rngs::SysRng};
use std::fmt::Write as _;
use tuneweave_core::{
    AuthChallengeBackend, AuthChallengeRequest, AuthChallengeStatus, ProviderAuthChallenge,
};

#[cfg(test)]
mod tests;

const COOLDOWN: Duration = Duration::from_secs(60);
const COOLDOWN_CAPACITY: usize = 1024;

pub(super) struct Entry {
    challenge: ProviderAuthChallenge,
    previous: Option<Selection>,
    state: Stage,
}
enum Stage {
    Preparing,
    Sending,
    WaitingNative(KuwoNativeSmsChallenge),
    WaitingWeb(KuwoWebSmsChallenge),
    Verifying,
}

impl KuwoProvider {
    fn validate_sms(
        &self,
        request: &AuthChallengeRequest,
        mode: CredentialMode,
    ) -> Result<KuwoNativeSmsRequest> {
        self.require_mode(&request.account, mode)?;
        request.reject_platform_policies_option(Platform::Kuwo)?;
        if request.method != tuneweave_core::ChallengeMethod::Sms
            || request.backend != AuthChallengeBackend::Standard
            || request
                .country_code
                .as_deref()
                .is_some_and(|value| !matches!(value, "86" | "+86"))
        {
            return Err(kuwo_invalid_request(
                "Kuwo SMS requires the standard backend and mainland China country code",
            ));
        }
        let request = KuwoNativeSmsRequest {
            phone: request.principal.clone(),
            allow_account_creation: request.allow_account_creation,
        };
        native_sms::validate_request(&request)?;
        Ok(request)
    }

    pub(in crate::provider) async fn begin_sms(
        &self,
        request: &AuthChallengeRequest,
        mode: CredentialMode,
    ) -> Result<ProviderAuthChallenge> {
        if request.backend == AuthChallengeBackend::Middle {
            return self.begin_web_sms(request, mode).await;
        }
        let input = self.validate_sms(request, mode)?;
        let challenge = ProviderAuthChallenge::stateful(
            Platform::Kuwo,
            request.clone(),
            mode,
            random_handle()?,
        )?;
        let (mut lease, previous) = self.reserve(&request.account, mode)?;
        {
            let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
            self.check_attempt(&mut registry, &lease, previous.as_ref())?;
            check_cooldown(&registry, &request.principal)?;
            if registry
                .attempts
                .values()
                .filter_map(|attempt| attempt.sms.as_ref())
                .any(|sms| {
                    sms.challenge.request().principal == request.principal
                        && !matches!(sms.state, Stage::WaitingNative(_) | Stage::WaitingWeb(_))
                })
            {
                return Err(busy());
            }
            // An explicit new delivery retires the earlier code for this phone,
            // across aliases and ownership modes in the shared provider.
            registry.attempts.retain(|_, attempt| {
                attempt
                    .sms
                    .as_ref()
                    .is_none_or(|sms| sms.challenge.request().principal != request.principal)
            });
            registry
                .attempts
                .get_mut(&lease.id)
                .ok_or_else(changed)?
                .sms = Some(Entry {
                challenge: challenge.clone(),
                previous: previous.clone(),
                state: Stage::Preparing,
            });
        }
        let device = self
            .boundary(
                &lease,
                previous.as_ref(),
                self.device_store.initialize(&self.client),
            )
            .await?;
        {
            let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
            self.check_attempt(&mut registry, &lease, previous.as_ref())?;
            check_cooldown(&registry, &request.principal)?;
            let entry = registry
                .attempts
                .get_mut(&lease.id)
                .and_then(|a| a.sms.as_mut())
                .ok_or_else(changed)?;
            if !matches!(entry.state, Stage::Preparing) {
                return Err(changed());
            }
            entry.state = Stage::Sending;
            // Keep the cooldown even if delivery later fails or gets cancelled:
            // a network error cannot prove that the SMS was not sent.
            registry
                .sms_cooldowns
                .insert(request.principal.clone(), Instant::now() + COOLDOWN);
        }
        let receipt = self
            .boundary(
                &lease,
                previous.as_ref(),
                self.client.send_native_login_sms(&input, &device),
            )
            .await?;
        {
            let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
            self.check_attempt(&mut registry, &lease, previous.as_ref())?;
            let entry = registry
                .attempts
                .get_mut(&lease.id)
                .and_then(|a| a.sms.as_mut())
                .ok_or_else(changed)?;
            if entry.challenge != challenge || !matches!(entry.state, Stage::Sending) {
                return Err(changed());
            }
            entry.state = Stage::WaitingNative(receipt);
            registry
                .sms_cooldowns
                .insert(request.principal.clone(), Instant::now() + COOLDOWN);
            lease.armed = false;
        }
        Ok(challenge)
    }

    async fn begin_web_sms(
        &self,
        request: &AuthChallengeRequest,
        mode: CredentialMode,
    ) -> Result<ProviderAuthChallenge> {
        validate_web_sms_request(request)?;
        self.require_mode(&request.account, mode)?;
        let challenge = ProviderAuthChallenge::stateful(
            Platform::Kuwo,
            request.clone(),
            mode,
            random_handle()?,
        )?;
        let (mut lease, previous) = self.reserve(&request.account, mode)?;
        {
            let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
            self.check_attempt(&mut registry, &lease, previous.as_ref())?;
            check_cooldown(&registry, &request.principal)?;
            if registry
                .attempts
                .values()
                .filter_map(|attempt| attempt.sms.as_ref())
                .any(|sms| {
                    sms.challenge.request().principal == request.principal
                        && !matches!(sms.state, Stage::WaitingNative(_) | Stage::WaitingWeb(_))
                })
            {
                return Err(busy());
            }
            registry.attempts.retain(|_, attempt| {
                attempt
                    .sms
                    .as_ref()
                    .is_none_or(|sms| sms.challenge.request().principal != request.principal)
            });
            registry
                .attempts
                .get_mut(&lease.id)
                .ok_or_else(changed)?
                .sms = Some(Entry {
                challenge: challenge.clone(),
                previous: previous.clone(),
                state: Stage::Preparing,
            });
        }
        {
            let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
            self.check_attempt(&mut registry, &lease, previous.as_ref())?;
            check_cooldown(&registry, &request.principal)?;
            let entry = registry
                .attempts
                .get_mut(&lease.id)
                .and_then(|attempt| attempt.sms.as_mut())
                .ok_or_else(changed)?;
            if !matches!(entry.state, Stage::Preparing) {
                return Err(changed());
            }
            entry.state = Stage::Sending;
            registry
                .sms_cooldowns
                .insert(request.principal.clone(), Instant::now() + COOLDOWN);
        }
        let receipt = self
            .boundary(
                &lease,
                previous.as_ref(),
                self.client.send_web_login_sms(&request.principal),
            )
            .await?;
        {
            let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
            self.check_attempt(&mut registry, &lease, previous.as_ref())?;
            let entry = registry
                .attempts
                .get_mut(&lease.id)
                .and_then(|attempt| attempt.sms.as_mut())
                .ok_or_else(changed)?;
            if entry.challenge != challenge || !matches!(entry.state, Stage::Sending) {
                return Err(changed());
            }
            entry.state = Stage::WaitingWeb(receipt);
            registry
                .sms_cooldowns
                .insert(request.principal.clone(), Instant::now() + COOLDOWN);
            lease.armed = false;
        }
        Ok(challenge)
    }

    pub(in crate::provider) fn sms_status(
        &self,
        challenge: &ProviderAuthChallenge,
    ) -> Result<AuthChallengeStatus> {
        self.require_base()?;
        let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
        registry.prune();
        let id = lookup(&registry, challenge)?;
        let previous = registry.attempts[&id]
            .sms
            .as_ref()
            .ok_or_else(changed)?
            .previous
            .clone();
        if let Err(error) = self.check_attempt_id(&mut registry, id, previous.as_ref()) {
            registry.attempts.remove(&id);
            return Err(error);
        }
        match registry.attempts[&id]
            .sms
            .as_ref()
            .map(|entry| &entry.state)
        {
            Some(Stage::WaitingNative(_) | Stage::WaitingWeb(_)) => {
                Ok(AuthChallengeStatus::Waiting)
            }
            _ => Err(busy()),
        }
    }

    pub(in crate::provider) async fn complete_sms(
        &self,
        challenge: &ProviderAuthChallenge,
        code: &str,
    ) -> Result<ProviderAuthResult> {
        self.require_base()?;
        native_sms::validate_code(code)?;
        let (id, deadline, previous, receipt) = {
            let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
            registry.prune();
            let id = lookup(&registry, challenge)?;
            let previous = registry.attempts[&id]
                .sms
                .as_ref()
                .ok_or_else(changed)?
                .previous
                .clone();
            if let Err(error) = self.check_attempt_id(&mut registry, id, previous.as_ref()) {
                registry.attempts.remove(&id);
                return Err(error);
            }
            let attempt = registry.attempts.get_mut(&id).ok_or_else(changed)?;
            let entry = attempt.sms.as_mut().ok_or_else(changed)?;
            if !matches!(entry.state, Stage::WaitingNative(_) | Stage::WaitingWeb(_)) {
                return Err(busy());
            }
            let receipt = match std::mem::replace(&mut entry.state, Stage::Verifying) {
                Stage::WaitingNative(receipt) => SmsReceipt::Native(receipt),
                Stage::WaitingWeb(receipt) => SmsReceipt::Web(receipt),
                _ => return Err(state_error()),
            };
            (id, attempt.deadline, previous, receipt)
        };
        // Construct the armed lease only after releasing the registry lock.
        let lease = Lease {
            registry: self.auth_registry.clone(),
            id,
            deadline,
            armed: true,
        };
        match receipt {
            SmsReceipt::Native(receipt) => {
                let authorization = self
                    .boundary(
                        &lease,
                        previous.as_ref(),
                        self.client.authenticate_native_sms(&receipt, code),
                    )
                    .await?;
                self.boundary(
                    &lease,
                    previous.as_ref(),
                    self.client.validate_native_session(authorization.session()),
                )
                .await?;
                self.commit_auth(
                    &lease,
                    previous.as_ref(),
                    NativeCredential::verified(authorization.session())?,
                    authorization.nickname().map(str::to_owned),
                )
            }
            SmsReceipt::Web(receipt) => {
                let session = self
                    .boundary(
                        &lease,
                        previous.as_ref(),
                        self.client.complete_web_login_sms(&receipt, code),
                    )
                    .await?;
                let device = self
                    .boundary(
                        &lease,
                        previous.as_ref(),
                        self.device_store.initialize(&self.client),
                    )
                    .await?;
                let native_session = device
                    .session_input(&session.user_id, &session.session_id)
                    .map_err(TuneWeaveError::with_consumed_auth_challenge)?;
                self.boundary(
                    &lease,
                    previous.as_ref(),
                    self.client.validate_native_session(&native_session),
                )
                .await
                .map_err(TuneWeaveError::with_consumed_auth_challenge)?;
                self.commit_auth(
                    &lease,
                    previous.as_ref(),
                    NativeCredential::verified(&native_session)
                        .map_err(TuneWeaveError::with_consumed_auth_challenge)?,
                    None,
                )
                .map_err(TuneWeaveError::with_consumed_auth_challenge)
            }
        }
    }
}

enum SmsReceipt {
    Native(KuwoNativeSmsChallenge),
    Web(KuwoWebSmsChallenge),
}

fn validate_web_sms_request(request: &AuthChallengeRequest) -> Result<()> {
    if request.method != tuneweave_core::ChallengeMethod::Sms
        || request.backend != AuthChallengeBackend::Middle
        || !request.allow_account_creation
        || !request.accept_platform_policies
        || request
            .country_code
            .as_deref()
            .is_some_and(|value| !matches!(value, "86" | "+86"))
    {
        return Err(kuwo_invalid_request(
            "Kuwo Web SMS requires mainland China, account-creation consent, and acceptance of the platform policies",
        ));
    }
    let native = KuwoNativeSmsRequest {
        phone: request.principal.clone(),
        allow_account_creation: request.allow_account_creation,
    };
    native_sms::validate_request(&native)
}

fn lookup(registry: &Registry, challenge: &ProviderAuthChallenge) -> Result<u64> {
    if challenge.platform() != Platform::Kuwo || challenge.provider_transaction_id().is_none() {
        return Err(kuwo_invalid_request(
            "Kuwo requires its original stateful SMS receipt",
        ));
    }
    registry
        .attempts
        .iter()
        .find_map(|(id, attempt)| {
            attempt
                .sms
                .as_ref()
                .filter(|sms| &sms.challenge == challenge)
                .map(|_| *id)
        })
        .ok_or_else(missing)
}
fn check_cooldown(registry: &Registry, phone: &str) -> Result<()> {
    if let Some(until) = registry
        .sms_cooldowns
        .get(phone)
        .filter(|until| **until > Instant::now())
    {
        let remaining = until.saturating_duration_since(Instant::now());
        let seconds = remaining
            .as_secs()
            .saturating_add(u64::from(remaining.subsec_nanos() > 0));
        return Err(TuneWeaveError::new(
            ErrorCode::RateLimited,
            "Kuwo SMS delivery is in cooldown",
        )
        .with_platform(Platform::Kuwo)
        .with_details(json!({"retry_after_secs": seconds.max(1)})));
    }
    if registry.sms_cooldowns.len() >= COOLDOWN_CAPACITY
        && !registry.sms_cooldowns.contains_key(phone)
    {
        return Err(TuneWeaveError::new(
            ErrorCode::RateLimited,
            "Kuwo has too many recent SMS deliveries",
        )
        .with_platform(Platform::Kuwo)
        .with_details(json!({"retry_after_secs":60})));
    }
    Ok(())
}
fn random_handle() -> Result<String> {
    let mut bytes = [0; 32];
    SysRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| state_error())?;
    let mut value = String::with_capacity(64);
    for byte in bytes {
        write!(value, "{byte:02x}").map_err(|_| state_error())?;
    }
    Ok(value)
}
fn missing() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::ResourceNotFound,
        "Kuwo SMS transaction is missing, expired, or consumed",
    )
    .with_platform(Platform::Kuwo)
}
fn busy() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::Conflict,
        "Kuwo SMS operation is already in progress",
    )
    .with_platform(Platform::Kuwo)
}
