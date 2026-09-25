//! Capacity reservations and exclusive verification for authentication challenges.
use super::*;
use tuneweave_core::Result;

/// Dropping an unfinished operation removes its transaction: cancellation may have happened
/// after the upstream accepted it. Only an explicit provider error permits another attempt.
pub(super) struct AuthChallengeLease {
    store: AuthTransactions,
    id: String,
    created_at: Instant,
    armed: bool,
}

impl AuthTransactions {
    pub(super) fn reserve_challenge(
        &self,
        platform: Platform,
        credential_mode: CredentialMode,
        request: AuthChallengeRequest,
    ) -> Result<AuthChallengeLease> {
        let id = self.insert(StoredAuthKind::Challenge {
            platform,
            credential_mode,
            request,
            provider_challenge: None,
            verifying: false,
        })?;
        let created_at = self.get(&id)?.created_at;
        Ok(AuthChallengeLease {
            store: self.clone(),
            id,
            created_at,
            armed: true,
        })
    }

    pub(super) fn reserve_password(
        &self,
        platform: Platform,
        credential_mode: CredentialMode,
        identity: PasswordLoginIdentity,
    ) -> Result<AuthChallengeLease> {
        let id = self.insert(StoredAuthKind::Password {
            platform,
            credential_mode,
            identity,
            provider_challenge: None,
            verifying: false,
        })?;
        let created_at = self.get(&id)?.created_at;
        Ok(AuthChallengeLease {
            store: self.clone(),
            id,
            created_at,
            armed: true,
        })
    }

    pub(super) fn claim_challenge(
        &self,
        id: &str,
    ) -> Result<(StoredAuthTransaction, AuthChallengeLease)> {
        self.claim_verification(id, false)
    }
    pub(super) fn claim_password(
        &self,
        id: &str,
    ) -> Result<(StoredAuthTransaction, AuthChallengeLease)> {
        self.claim_verification(id, true)
    }
    fn claim_verification(
        &self,
        id: &str,
        password: bool,
    ) -> Result<(StoredAuthTransaction, AuthChallengeLease)> {
        let mut entries = self.entries.write().map_err(|_| auth_store_error())?;
        let expired = Self::take_expired(&mut entries, Instant::now());
        let result = (|| {
            let transaction = entries.get_mut(id).ok_or_else(auth_transaction_not_found)?;
            let verifying = match (&mut transaction.kind, password) {
                (
                    StoredAuthKind::Challenge {
                        provider_challenge: Some(_),
                        verifying,
                        ..
                    },
                    false,
                )
                | (
                    StoredAuthKind::Password {
                        provider_challenge: Some(_),
                        verifying,
                        ..
                    },
                    true,
                ) => verifying,
                _ => return Err(auth_transaction_not_found()),
            };
            if *verifying {
                return Err(TuneWeaveError::new(
                    ErrorCode::Conflict,
                    "Authentication challenge verification is already in progress",
                ));
            }
            *verifying = true;
            Ok((
                transaction.clone(),
                AuthChallengeLease {
                    store: self.clone(),
                    id: id.to_owned(),
                    created_at: transaction.created_at,
                    armed: true,
                },
            ))
        })();
        drop(entries);
        Self::log_expired(expired);
        result
    }
}

impl AuthChallengeLease {
    pub(super) fn publish_password(
        mut self,
        challenge: ProviderPasswordChallenge,
    ) -> Result<String> {
        let mut entries = self.store.entries.write().map_err(|_| auth_store_error())?;
        let transaction = entries
            .get_mut(&self.id)
            .filter(|t| t.created_at == self.created_at && t.expires_at > Instant::now())
            .ok_or_else(auth_transaction_not_found)?;
        let StoredAuthKind::Password {
            platform,
            credential_mode,
            identity,
            provider_challenge,
            verifying,
        } = &mut transaction.kind
        else {
            return Err(auth_store_error());
        };
        if challenge.platform() != *platform
            || challenge.credential_mode() != *credential_mode
            || challenge.identity() != identity
            || provider_challenge
                .as_ref()
                .is_some_and(|old| old != &challenge)
        {
            return Err(auth_provider_contract_error(
                "Provider returned a password challenge with different ownership",
            ));
        }
        *provider_challenge = Some(challenge);
        *verifying = false;
        self.armed = false;
        Ok(self.id.clone())
    }

    pub(super) fn publish(mut self, challenge: ProviderAuthChallenge) -> Result<String> {
        let mut entries = self.store.entries.write().map_err(|_| auth_store_error())?;
        let transaction = entries
            .get_mut(&self.id)
            .filter(|t| t.created_at == self.created_at && t.expires_at > Instant::now())
            .ok_or_else(auth_transaction_not_found)?;
        let StoredAuthKind::Challenge {
            platform,
            request,
            credential_mode,
            provider_challenge: slot @ None,
            verifying: false,
        } = &mut transaction.kind
        else {
            return Err(auth_store_error());
        };
        if challenge.platform() != *platform
            || challenge.request() != request
            || challenge.credential_mode() != *credential_mode
        {
            return Err(TuneWeaveError::new(
                ErrorCode::InternalError,
                "Provider returned a challenge with different ownership",
            ));
        }
        *slot = Some(challenge);
        self.armed = false;
        Ok(self.id.clone())
    }

    pub(super) fn retry(mut self) -> Result<()> {
        let mut entries = self.store.entries.write().map_err(|_| auth_store_error())?;
        let transaction = entries
            .get_mut(&self.id)
            .filter(|t| t.created_at == self.created_at && t.expires_at > Instant::now())
            .ok_or_else(auth_transaction_not_found)?;
        let verifying = match &mut transaction.kind {
            StoredAuthKind::Challenge { verifying, .. }
            | StoredAuthKind::Password { verifying, .. } => verifying,
            _ => return Err(auth_store_error()),
        };
        *verifying = false;
        self.armed = false;
        Ok(())
    }

    pub(super) fn finish(mut self) -> Result<()> {
        let transaction = self.take()?.ok_or_else(auth_transaction_not_found)?;
        self.armed = false;
        if transaction.expires_at <= Instant::now() {
            log_auth_transaction_completed(&self.id, &transaction, AuthState::Expired);
            return Err(auth_transaction_not_found());
        }
        log_auth_transaction_completed(&self.id, &transaction, AuthState::Confirmed);
        Ok(())
    }

    fn take(&self) -> Result<Option<StoredAuthTransaction>> {
        let mut entries = self.store.entries.write().map_err(|_| auth_store_error())?;
        if entries
            .get(&self.id)
            .is_some_and(|t| t.created_at == self.created_at)
        {
            Ok(entries.remove(&self.id))
        } else {
            Ok(None)
        }
    }
}

impl Drop for AuthChallengeLease {
    fn drop(&mut self) {
        if self.armed {
            if let Ok(Some(transaction)) = self.take() {
                let state = if transaction.expires_at <= Instant::now() {
                    AuthState::Expired
                } else {
                    AuthState::Failed
                };
                log_auth_transaction_completed(&self.id, &transaction, state);
            }
        }
    }
}
