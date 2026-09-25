//! The password receipt transitions to native SMS login, without keeping its password.
use super::*;
use crate::login::native_secondary::{self, Outcome, Target};
use tuneweave_core::AuthAccountChoice;

const RESEND_INTERVAL: Duration = Duration::from_secs(31);
const MAX_SENDS: u8 = 5;

pub(super) struct Sms {
    target: Target,
    accounts: Option<Vec<AuthAccountChoice>>,
    next_send: Instant,
    sends: u8,
}
impl Sms {
    pub(super) fn verification(&self, remaining_attempts: u8) -> PasswordVerification {
        match &self.accounts {
            Some(accounts) => PasswordVerification::AccountSelection {
                accounts: accounts.clone(),
                remaining_attempts,
            },
            None => PasswordVerification::Sms {
                masked_destination: self.target.masked_destination(),
                remaining_attempts,
                resend_after_secs: self
                    .next_send
                    .saturating_duration_since(Instant::now())
                    .as_millis()
                    .div_ceil(1000) as u64,
            },
        }
    }
    pub(super) fn validate(&self, action: &PasswordChallengeAction, attempts: u8) -> Result<()> {
        if attempts >= ATTEMPTS {
            return Err(limited());
        }
        match action {
            PasswordChallengeAction::SubmitSms { code } if self.accounts.is_none() => {
                native_secondary::validate_code(code)
            }
            PasswordChallengeAction::SelectAccount { user_id, code }
                if self
                    .accounts
                    .as_ref()
                    .is_some_and(|choices| choices.iter().any(|a| a.user_id == *user_id)) =>
            {
                native_secondary::validate_code(code)
            }
            PasswordChallengeAction::ResendSms => {
                if self.sends >= MAX_SENDS || Instant::now() < self.next_send {
                    Err(limited())
                } else {
                    Ok(())
                }
            }
            _ => Err(invalid()),
        }
    }
}

impl KugouProvider {
    pub(super) async fn start_native_secondary(
        &self,
        receipt: &ProviderPasswordChallenge,
        mut context: Context,
        lease: Lease,
        target: Target,
    ) -> Result<PasswordLoginProgress> {
        if context.attempts >= ATTEMPTS {
            return Err(limited());
        }
        self.check_native_password(&lease, receipt, &context)?;
        let result = self
            .client
            .native_secondary_send(&context.device, &target)
            .await;
        self.check_native_password(&lease, receipt, &context)?;
        result?;
        context.stage = Some(Stage::Sms(Sms {
            target,
            accounts: None,
            next_send: Instant::now() + RESEND_INTERVAL,
            sends: 1,
        }));
        self.publish_native_password(receipt, context, lease)
    }
    pub(super) async fn advance_native_secondary(
        &self,
        receipt: &ProviderPasswordChallenge,
        mut context: Context,
        lease: Lease,
        action: &PasswordChallengeAction,
    ) -> Result<PasswordLoginProgress> {
        let Some(Stage::Sms(sms)) = context.stage.as_ref() else {
            return Err(invalid());
        };
        let target = sms.target.clone();
        match action {
            PasswordChallengeAction::ResendSms => {
                self.check_native_password(&lease, receipt, &context)?;
                let result = self
                    .client
                    .native_secondary_send(&context.device, &target)
                    .await;
                self.check_native_password(&lease, receipt, &context)?;
                result?;
                let Some(Stage::Sms(sms)) = context.stage.as_mut() else {
                    return Err(invalid());
                };
                sms.sends += 1;
                sms.accounts = None;
                sms.next_send = Instant::now() + RESEND_INTERVAL;
            }
            PasswordChallengeAction::SubmitSms { code }
            | PasswordChallengeAction::SelectAccount { code, .. } => {
                let selected = match action {
                    PasswordChallengeAction::SelectAccount { user_id, .. } => {
                        Some(user_id.as_str())
                    }
                    _ => None,
                };
                context.attempts += 1;
                self.check_native_password(&lease, receipt, &context)?;
                let result = self
                    .client
                    .native_secondary_login(&context.device, &target, code, selected)
                    .await;
                self.check_native_password(&lease, receipt, &context)?;
                match result? {
                    Outcome::Session(session) => {
                        return self
                            .finish_native_password(
                                receipt,
                                context,
                                lease,
                                NativePasswordOutcome::Session(session),
                            )
                            .await;
                    }
                    Outcome::InvalidCode => {
                        if context.attempts >= ATTEMPTS {
                            return Err(limited());
                        }
                        let Some(Stage::Sms(sms)) = context.stage.as_mut() else {
                            return Err(invalid());
                        };
                        sms.accounts = None;
                    }
                    Outcome::ChooseAccount => {
                        if context.attempts >= ATTEMPTS {
                            return Err(limited());
                        }
                        self.check_native_password(&lease, receipt, &context)?;
                        let accounts = self
                            .client
                            .native_secondary_accounts(&context.device, &target, code)
                            .await;
                        self.check_native_password(&lease, receipt, &context)?;
                        let Some(Stage::Sms(sms)) = context.stage.as_mut() else {
                            return Err(invalid());
                        };
                        sms.accounts = Some(accounts?);
                    }
                }
            }
            _ => return Err(invalid()),
        }
        self.publish_native_password(receipt, context, lease)
    }
}

#[cfg(test)]
mod tests;
