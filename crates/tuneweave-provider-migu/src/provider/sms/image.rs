use super::*;

pub(super) fn image_required(e: &TuneWeaveError, chinese: bool) -> Option<bool> {
    if e.code != ErrorCode::AuthenticationRequired {
        return None;
    }
    match e
        .details
        .get("platform_code")
        .and_then(serde_json::Value::as_str)
    {
        Some("4002") => Some(chinese),
        Some("4044") => Some(false),
        Some("4045") => Some(true),
        _ => None,
    }
}

impl Context {
    pub(super) fn status(&self) -> AuthChallengeStatus {
        match &self.stage {
            Stage::Code { .. } => AuthChallengeStatus::Waiting,
            Stage::Image { image, .. } => AuthChallengeStatus::VerificationRequired {
                verification: AuthImageChallenge {
                    image_data_url: image.data_url.clone(),
                    answer_kind: image.kind,
                    remaining_attempts: ATTEMPTS.saturating_sub(self.image_attempts),
                    refresh_after_secs: self
                        .next_image_at
                        .saturating_duration_since(Instant::now())
                        .as_millis()
                        .div_ceil(1000) as u64,
                },
            },
        }
    }
}

impl MiguProvider {
    pub(in crate::provider) fn sms_status(
        &self,
        receipt: &ProviderAuthChallenge,
    ) -> Result<AuthChallengeStatus> {
        self.validate_sms(receipt.request(), receipt.credential_mode())?;
        let id = receipt
            .provider_transaction_id()
            .ok_or_else(|| migu_invalid_request("Migu SMS requires a stateful receipt"))?;
        let mut store = self.passport_transactions.lock().map_err(|_| locked())?;
        store.prune();
        let entry = store.entries.get(id).ok_or_else(missing)?;
        if &entry.receipt != receipt {
            return Err(migu_invalid_request(
                "Migu SMS receipt binding does not match",
            ));
        }
        let context = entry.context.as_ref().ok_or_else(|| {
            error(
                ErrorCode::Conflict,
                "Migu SMS transaction is already in use",
            )
        })?;
        self.check_sms_owner(receipt, &context.previous)?;
        Ok(context.status())
    }

    pub(in crate::provider) async fn advance_sms_image(
        &self,
        receipt: &ProviderAuthChallenge,
        action: &AuthChallengeAction,
    ) -> Result<AuthChallengeProgress> {
        self.validate_sms(receipt.request(), receipt.credential_mode())?;
        let id = receipt
            .provider_transaction_id()
            .ok_or_else(|| migu_invalid_request("Migu SMS requires a stateful receipt"))?;
        let (mut context, purpose, chinese, lease) = {
            let mut store = self.passport_transactions.lock().map_err(|_| locked())?;
            store.prune();
            let entry = store.entries.get_mut(id).ok_or_else(missing)?;
            if &entry.receipt != receipt {
                return Err(migu_invalid_request(
                    "Migu SMS receipt binding does not match",
                ));
            }
            let context = entry.context.as_ref().ok_or_else(|| {
                error(
                    ErrorCode::Conflict,
                    "Migu SMS transaction is already in use",
                )
            })?;
            let Stage::Image { image, purpose } = &context.stage else {
                return Err(migu_invalid_request(
                    "Migu SMS is not waiting for an image answer",
                ));
            };
            match action {
                AuthChallengeAction::SubmitImage { answer } => image.validate_answer(answer)?,
                AuthChallengeAction::RefreshImage => {
                    if context.refreshes >= ATTEMPTS || context.next_image_at > Instant::now() {
                        return Err(error(
                            ErrorCode::RateLimited,
                            "Migu image refresh limit reached",
                        ));
                    }
                }
                _ => return Err(migu_invalid_request("Invalid Migu image action")),
            }
            let (purpose, chinese) = (*purpose, image.kind == AuthImageAnswerKind::Chinese);
            let context = entry.context.take().ok_or_else(locked)?;
            (
                context,
                purpose,
                chinese,
                Lease {
                    store: self.passport_transactions.clone(),
                    id: id.into(),
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
        let outcome = async {
            check()?;
            match action {
                AuthChallengeAction::RefreshImage => {
                    context.refreshes += 1;
                    self.client
                        .sms_image(chinese, &mut context.cookies, &check)
                        .await
                        .map(Some)
                }
                AuthChallengeAction::SubmitImage { answer } => {
                    context.image_attempts += 1;
                    if context.image_attempts > ATTEMPTS {
                        return Err(error(
                            ErrorCode::RateLimited,
                            "Migu image answer limit reached",
                        ));
                    }
                    let checked = self
                        .client
                        .check_sms_image(answer, &mut context.cookies, &check)
                        .await;
                    let result = match checked {
                        Ok(()) if matches!(purpose, ImagePurpose::Send) => {
                            // An answer can arrive long after the initial request. Reserve a
                            // fresh delivery cooldown before this potentially accepted send.
                            self.passport_transactions
                                .lock()
                                .map_err(|_| locked())?
                                .cooldowns
                                .insert(
                                    receipt.request().principal.clone(),
                                    Instant::now() + COOLDOWN,
                                );
                            self.client
                                .send_sms(
                                    &receipt.request().principal,
                                    answer,
                                    &mut context.cookies,
                                    &check,
                                )
                                .await
                        }
                        other => other,
                    };
                    match result {
                        Ok(()) => Ok(None),
                        Err(e) => {
                            if let Some(chinese) = image_required(&e, chinese) {
                                if context.image_attempts < ATTEMPTS {
                                    return self
                                        .client
                                        .sms_image(chinese, &mut context.cookies, &check)
                                        .await
                                        .map(Some);
                                }
                            }
                            Err(e)
                        }
                    }
                }
                _ => unreachable!(),
            }
        }
        .await
        .map_err(TuneWeaveError::with_consumed_auth_challenge)?;
        context.stage = match outcome {
            Some(image) => {
                context.next_image_at = Instant::now() + IMAGE_INTERVAL;
                Stage::Image { image, purpose }
            }
            None => {
                let AuthChallengeAction::SubmitImage { answer } = action else {
                    unreachable!()
                };
                Stage::Code {
                    captcha: answer.clone(),
                    chinese,
                }
            }
        };
        let status = context.status();
        lease
            .publish(context)
            .map_err(TuneWeaveError::with_consumed_auth_challenge)?;
        Ok(AuthChallengeProgress::Pending(status))
    }
}
