use super::*;
use crate::client::KuwoWebFormChallenge;
use tuneweave_core::{
    PasswordChallengeAction, PasswordFormat, PasswordLoginBackend, PasswordLoginIdentity,
    PasswordLoginProgress, PasswordVerification, PrincipalType, ProviderPasswordChallenge,
};

pub(super) struct Entry {
    receipt: ProviderPasswordChallenge,
    challenge: Option<KuwoWebFormChallenge>,
    previous: Option<Selection>,
    busy: bool,
}

impl KuwoProvider {
    pub(in crate::provider) async fn begin_web_password(
        &self,
        request: &PasswordLoginRequest,
        mode: CredentialMode,
    ) -> Result<PasswordLoginProgress> {
        validate_request(request)?;
        let (mut lease, previous) = self.reserve(&request.account, mode)?;
        let transaction_id = lease.id.to_string();
        let receipt = ProviderPasswordChallenge::new(
            Platform::Kuwo,
            PasswordLoginIdentity::from(request),
            mode,
            transaction_id,
        )?;
        let challenge = self
            .boundary(
                &lease,
                previous.as_ref(),
                self.client.create_web_form_password_challenge(),
            )
            .await?;
        let verification = PasswordVerification::Image {
            image: challenge.image()?,
        };
        {
            let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
            self.check_attempt(&mut registry, &lease, previous.as_ref())?;
            let attempt = registry.attempts.get_mut(&lease.id).ok_or_else(missing)?;
            if attempt.web_password.is_some() {
                return Err(changed());
            }
            attempt.web_password = Some(Entry {
                receipt: receipt.clone(),
                challenge: Some(challenge),
                previous,
                busy: false,
            });
        }
        lease.armed = false;
        Ok(PasswordLoginProgress::Pending {
            challenge: receipt,
            verification,
        })
    }

    pub(in crate::provider) async fn advance_web_password(
        &self,
        receipt: &ProviderPasswordChallenge,
        action: &PasswordChallengeAction,
    ) -> Result<PasswordLoginProgress> {
        self.require_base()?;
        if receipt.platform() != Platform::Kuwo
            || receipt.identity().backend != PasswordLoginBackend::Web
        {
            return Err(kuwo_invalid_request(
                "Kuwo Web password challenge receipt is invalid",
            ));
        }
        match action {
            PasswordChallengeAction::SubmitImage { answer, password } => {
                validate_password(password)?;
                if !(1..=6).contains(&answer.len())
                    || !answer.bytes().all(|byte| byte.is_ascii_alphanumeric())
                {
                    return Err(kuwo_invalid_request(
                        "Kuwo Web image answers must contain 1–6 ASCII letters or digits",
                    ));
                }
            }
            PasswordChallengeAction::RefreshImage => {}
            _ => {
                return Err(kuwo_invalid_request(
                    "Kuwo Web password challenge accepts only image refresh or image submission",
                ));
            }
        }

        let (id, deadline, previous, stored_receipt, mut challenge) = {
            let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
            registry.prune();
            let id = registry
                .attempts
                .iter()
                .find_map(|(id, attempt)| {
                    attempt
                        .web_password
                        .as_ref()
                        .filter(|entry| &entry.receipt == receipt)
                        .map(|_| *id)
                })
                .ok_or_else(missing)?;
            let previous = registry
                .attempts
                .get(&id)
                .and_then(|attempt| attempt.web_password.as_ref())
                .ok_or_else(missing)?
                .previous
                .clone();
            self.check_attempt_id(&mut registry, id, previous.as_ref())?;
            let attempt = registry.attempts.get_mut(&id).ok_or_else(missing)?;
            let entry = attempt.web_password.as_mut().ok_or_else(missing)?;
            if entry.busy {
                return Err(busy());
            }
            let challenge = entry.challenge.as_ref().ok_or_else(missing)?;
            if let PasswordChallengeAction::SubmitImage { answer, .. } = action {
                challenge.validate_answer(answer)?;
            }
            let challenge = entry.challenge.take().ok_or_else(missing)?;
            entry.busy = true;
            (
                id,
                attempt.deadline,
                previous,
                entry.receipt.clone(),
                challenge,
            )
        };
        let mut lease = Lease {
            registry: self.auth_registry.clone(),
            id,
            deadline,
            armed: true,
        };
        if stored_receipt != *receipt {
            return Err(missing());
        }

        match action {
            PasswordChallengeAction::RefreshImage => {
                let refresh = self
                    .boundary(
                        &lease,
                        previous.as_ref(),
                        self.client
                            .refresh_web_form_password_challenge(&mut challenge),
                    )
                    .await;
                let image = match refresh {
                    Ok(()) => challenge.image()?,
                    Err(error)
                        if error.code == ErrorCode::RateLimited && challenge.image().is_ok() =>
                    {
                        self.restore_challenge(&lease, &previous, challenge)?;
                        lease.armed = false;
                        return Err(error);
                    }
                    Err(error) => {
                        return Err(error.with_consumed_auth_challenge());
                    }
                };
                self.restore_challenge(&lease, &previous, challenge)?;
                lease.armed = false;
                Ok(PasswordLoginProgress::Pending {
                    challenge: stored_receipt,
                    verification: PasswordVerification::Image { image },
                })
            }
            PasswordChallengeAction::SubmitImage { answer, password } => {
                let identity = receipt.identity();
                let session = self
                    .boundary(
                        &lease,
                        previous.as_ref(),
                        self.client.submit_web_form_password(
                            &challenge,
                            &identity.principal,
                            password,
                            answer,
                        ),
                    )
                    .await
                    .map_err(TuneWeaveError::with_consumed_auth_challenge)?;
                let device = self
                    .boundary(
                        &lease,
                        previous.as_ref(),
                        self.device_store.initialize(&self.client),
                    )
                    .await
                    .map_err(TuneWeaveError::with_consumed_auth_challenge)?;
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
                .map(PasswordLoginProgress::Confirmed)
                .map_err(TuneWeaveError::with_consumed_auth_challenge)
            }
            _ => unreachable!("action validated before consuming challenge"),
        }
    }

    fn restore_challenge(
        &self,
        lease: &Lease,
        previous: &Option<Selection>,
        challenge: KuwoWebFormChallenge,
    ) -> Result<()> {
        let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
        self.check_attempt(&mut registry, lease, previous.as_ref())?;
        let attempt = registry.attempts.get_mut(&lease.id).ok_or_else(missing)?;
        let entry = attempt.web_password.as_mut().ok_or_else(missing)?;
        if !entry.busy || entry.challenge.is_some() {
            return Err(changed());
        }
        entry.challenge = Some(challenge);
        entry.busy = false;
        Ok(())
    }
}

fn validate_request(request: &PasswordLoginRequest) -> Result<()> {
    request.require_backend(Platform::Kuwo, PasswordLoginBackend::Web)?;
    if request.principal_type != PrincipalType::Username
        || request.country_code.is_some()
        || request.password_format != PasswordFormat::Plain
        || request.secure_captcha.is_some()
        || request.principal.is_empty()
        || request.principal.len() > 256
        || request.principal.trim() != request.principal
        || request.principal.chars().any(char::is_control)
    {
        return Err(kuwo_invalid_request(
            "Kuwo Web password login supports only a plain username and image challenge",
        ));
    }
    // The initial password is not retained or sent; the generic challenge action
    // requires the caller to resubmit it together with the image answer.
    validate_password(&request.password)
}

fn validate_password(password: &str) -> Result<()> {
    if password.is_empty() || password.len() > 1024 || password.chars().any(char::is_control) {
        Err(kuwo_invalid_request("Kuwo Web password is invalid"))
    } else {
        Ok(())
    }
}

fn missing() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::ResourceNotFound,
        "Kuwo Web password challenge is missing, expired, or consumed",
    )
    .with_platform(Platform::Kuwo)
}

fn busy() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::Conflict,
        "Kuwo Web password verification is already in progress",
    )
    .with_platform(Platform::Kuwo)
}

#[cfg(test)]
mod tests;
