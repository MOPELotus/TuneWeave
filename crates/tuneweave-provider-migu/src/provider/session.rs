use super::*;
use crate::credential::{KIND, MiguCredential, authentication_required, error, import_cookie};
use tuneweave_core::{
    AccountProfile, CredentialImportRequest, CredentialMode, ErrorCode, ImportedCredential,
    ProviderAuthResult, ProviderCredential, ProviderLogoutResult, StoredAccountCredential,
};

pub(super) type Selection = (MiguCredential, Option<StoredAccountCredential>);

impl MiguProvider {
    pub(super) fn caller_scope(&self, value: &ProviderCredential) -> Result<Self> {
        Ok(Self {
            client: self.client.clone(),
            credential_store: None,
            caller_credential: Some(Arc::new(std::sync::Mutex::new(
                MiguCredential::parse_caller(value)?,
            ))),
            response_credential: Arc::default(),
            auth_mutation: self.auth_mutation.clone(),
            passport_transactions: self.passport_transactions.clone(),
        })
    }

    pub(super) fn require_public_source(&self) -> Result<()> {
        if self.caller_credential.is_some() {
            return Err(migu_invalid_request(
                "This Migu catalogue/media operation does not yet support account credentials",
            ));
        }
        Ok(())
    }

    fn validate_ownership(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<()> {
        validate_account(account, mode)?;
        if self.caller_credential.is_some()
            || (source.is_some() && mode == CredentialMode::Server)
            || (source.is_none() && mode == CredentialMode::Client)
        {
            return Err(migu_invalid_request(
                "Migu session ownership requires an explicit source on the base provider",
            ));
        }
        Ok(())
    }

    pub(super) fn selected(&self, account: &str) -> Result<Option<Selection>> {
        validate_account(account, CredentialMode::Server)?;
        if let Some(caller) = &self.caller_credential {
            if account != "default" {
                return Err(migu_invalid_request(
                    "Migu caller credentials cannot select a server account alias",
                ));
            }
            return Ok(Some((
                caller.lock().map_err(|_| lock_error())?.clone(),
                None,
            )));
        }
        let Some(store) = &self.credential_store else {
            return Ok(None);
        };
        let Some(stored) = store
            .load_platform(Platform::Migu)?
            .into_iter()
            .find(|v| v.account == account)
        else {
            return Ok(None);
        };
        if stored.kind != KIND {
            return Err(lock_error());
        }
        let source = MiguCredential::parse(stored.secret()).map_err(|_| lock_error())?;
        Ok(Some((source, Some(stored))))
    }

    fn ownership_source(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<Option<Selection>> {
        self.validate_ownership(account, source, mode)?;
        let caller = source.map(MiguCredential::parse_caller).transpose()?;
        if mode == CredentialMode::Client {
            return Ok(caller.map(|v| (v, None)));
        }
        let selected = self.selected(account)?;
        if let Some(caller) = caller {
            let Some((server, _)) = &selected else {
                return Err(authentication_required());
            };
            if !server.same_login(&caller) {
                return Err(session_changed());
            }
            // Both explicitly synchronizes with the latest server token of this login.
        }
        Ok(selected)
    }

    pub(super) fn accept_read(
        &self,
        source: &MiguCredential,
        stored: Option<&StoredAccountCredential>,
        refreshed: &MiguCredential,
    ) -> Result<()> {
        if !source.same_login(refreshed) {
            return Err(session_changed());
        }
        if let Some(stored) = stored {
            let replacement = StoredAccountCredential::new(
                Platform::Migu,
                &stored.account,
                KIND,
                refreshed.serialize()?,
            )?;
            if !self
                .credential_store
                .as_ref()
                .ok_or_else(lock_error)?
                .compare_exchange(stored, Some(&replacement))?
            {
                return Err(session_changed());
            }
        } else {
            let mut caller = self
                .caller_credential
                .as_ref()
                .ok_or_else(lock_error)?
                .lock()
                .map_err(|_| lock_error())?;
            if &*caller != source {
                return Err(session_changed());
            }
            if source != refreshed {
                let issued = refreshed.caller()?;
                let mut response = self.response_credential.lock().map_err(|_| lock_error())?;
                *caller = refreshed.clone();
                *response = Some(issued);
            }
        }
        Ok(())
    }

    pub(super) async fn import_session(
        &self,
        request: &CredentialImportRequest,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        validate_account(&request.account, mode)?;
        if self.caller_credential.is_some() {
            return Err(migu_invalid_request(
                "Migu imports require the base provider",
            ));
        }
        if mode.persists_on_server() && self.credential_store.is_none() {
            return Err(lock_error());
        }
        let ImportedCredential::Cookie { value } = &request.credential;
        let token = import_cookie(value)?;
        // Serialize fresh logins and official logout on this provider, including its clones.
        let _guard = self.auth_mutation.lock().await;
        let previous = if mode.persists_on_server() {
            self.selected(&request.account)?
        } else {
            None
        };
        let token = self.client.check_pacm(&token).await?;
        let read = self
            .client
            .account_profile(&request.account, &token, None)
            .await?;
        self.commit_new_login(&request.account, mode, previous, read)
    }

    pub(super) fn validate_password_input(
        &self,
        request: &tuneweave_core::PasswordLoginRequest,
        mode: CredentialMode,
    ) -> Result<()> {
        request.require_backend(
            Platform::Migu,
            tuneweave_core::PasswordLoginBackend::Default,
        )?;
        validate_account(&request.account, mode)?;
        if self.caller_credential.is_some()
            || (mode.persists_on_server() && self.credential_store.is_none())
        {
            return Err(migu_invalid_request(
                "Migu password login requires the base provider and selected ownership storage",
            ));
        }
        use tuneweave_core::{PasswordFormat, PrincipalType};
        if request.password_format != PasswordFormat::Plain
            || request.secure_captcha.is_some()
            || request.principal.is_empty()
            || request.principal.trim() != request.principal
            || request.principal.encode_utf16().count() > 128
            || request.principal.chars().any(char::is_control)
            || request.password.is_empty()
            || request.password.len() > 501
            || request.password.chars().any(char::is_control)
            || request
                .country_code
                .as_deref()
                .is_some_and(|v| !matches!(v, "86" | "+86"))
            || (request.principal_type == PrincipalType::Phone
                && (request.principal.len() != 11
                    || !request.principal.starts_with('1')
                    || !request.principal.bytes().all(|b| b.is_ascii_digit())))
        {
            return Err(migu_invalid_request(
                "Migu password login requires a valid principal and plain password; use challenge actions for additional verification",
            ));
        }
        Ok(())
    }

    pub(super) async fn password_session(
        &self,
        request: &tuneweave_core::PasswordLoginRequest,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        self.validate_password_input(request, mode)?;
        let _guard = self.auth_mutation.lock().await;
        let previous = if mode.persists_on_server() {
            self.selected(&request.account)?
        } else {
            None
        };
        let (uid, token) = self.client.password_music_session(request).await?;
        let read = self
            .client
            .account_profile(&request.account, &token, Some(&uid))
            .await?;
        self.commit_new_login(&request.account, mode, previous, read)
    }

    pub(super) fn commit_new_login(
        &self,
        account: &str,
        mode: CredentialMode,
        previous: Option<Selection>,
        read: crate::client::account::AccountRead,
    ) -> Result<ProviderAuthResult> {
        let profile = read.profile?;
        let credential = MiguCredential::verified(read.user_id, read.token)?;
        let issued = mode
            .returns_to_caller()
            .then(|| credential.caller())
            .transpose()?;
        if mode.persists_on_server() {
            let store = self.credential_store.as_ref().ok_or_else(lock_error)?;
            let mut transactions = self
                .passport_transactions
                .lock()
                .map_err(|_| lock_error())?;
            let value = StoredAccountCredential::new(
                Platform::Migu,
                account,
                KIND,
                credential.serialize()?,
            )?;
            if let Some((_, Some(previous))) = previous {
                if !store.compare_exchange(&previous, Some(&value))? {
                    return Err(session_changed());
                }
            } else if !store.insert_if_absent(&value)? {
                return Err(session_changed());
            }
            transactions.cancel_account(account);
        }
        Ok(ProviderAuthResult {
            profile,
            credential: issued,
        })
    }

    pub(super) async fn read_session_profile(&self, account: &str) -> Result<AccountProfile> {
        let empty = || AccountProfile {
            platform: Platform::Migu,
            account: account.to_owned(),
            user_id: None,
            nickname: None,
            avatar_url: None,
            authenticated: false,
            extensions: Default::default(),
        };
        let Some((source, stored)) = self.selected(account)? else {
            return Ok(empty());
        };
        match self
            .client
            .account_profile(account, source.token(), Some(source.user_id()))
            .await
        {
            Ok(read) => {
                let refreshed = source.rotate(read.token)?;
                self.accept_read(&source, stored.as_ref(), &refreshed)
                    .inspect_err(|e| {
                        let _ = self.discard_response_credential_after_error(e.code);
                    })?;
                read.profile
            }
            Err(e) => {
                // A delayed failure must not describe a newly replaced login as logged out.
                self.accept_read(&source, stored.as_ref(), &source)
                    .inspect_err(|e| {
                        let _ = self.discard_response_credential_after_error(e.code);
                    })?;
                self.discard_response_credential_after_error(e.code)?;
                if e.code == ErrorCode::AuthenticationRequired {
                    Ok(empty())
                } else {
                    Err(e)
                }
            }
        }
    }

    fn advance_refresh(
        &self,
        current: &mut MiguCredential,
        stored: &mut Option<StoredAccountCredential>,
        next: MiguCredential,
    ) -> Result<()> {
        if let Some(previous) = stored.as_ref() {
            let replacement = StoredAccountCredential::new(
                Platform::Migu,
                &previous.account,
                KIND,
                next.serialize()?,
            )?;
            self.accept_read(current, Some(previous), &next)?;
            *stored = Some(replacement);
        }
        *current = next;
        Ok(())
    }

    pub(super) async fn refresh_owned_session(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        let (original, mut stored) = self
            .ownership_source(account, source, mode)?
            .ok_or_else(authentication_required)?;
        let mut current = original.clone();
        let mut verified_step = false;
        let result: Result<ProviderAuthResult> = async {
            let token = self.client.check_pacm(current.token()).await?;
            let next = current.rotate(token)?;
            self.advance_refresh(&mut current, &mut stored, next)?;
            verified_step = true;
            let read = self
                .client
                .account_profile(account, current.token(), Some(current.user_id()))
                .await?;
            let next = current.rotate(read.token)?;
            self.advance_refresh(&mut current, &mut stored, next)?;
            let mut profile = read.profile?;
            profile
                .extensions
                .insert("refreshed".to_owned(), json!(current != original));
            profile.extensions.insert(
                "refresh_method".to_owned(),
                json!("pacm_check_and_identity_verification"),
            );
            Ok(ProviderAuthResult {
                profile,
                credential: mode
                    .returns_to_caller()
                    .then(|| current.caller())
                    .transpose()?,
            })
        }
        .await;
        match result {
            Ok(result) => Ok(result),
            Err(mut error) => {
                if let Some(snapshot) = stored.as_ref() {
                    if error.code == ErrorCode::AuthenticationRequired {
                        // Do not retain a token proven unauthenticated or bound to another UID.
                        if !self
                            .credential_store
                            .as_ref()
                            .ok_or_else(lock_error)?
                            .compare_exchange(snapshot, None)?
                        {
                            return Err(session_changed());
                        }
                    } else {
                        // Also check late failures, including unchanged-token responses.
                        self.accept_read(&current, Some(snapshot), &current)?;
                    }
                }
                if verified_step && mode.returns_to_caller() {
                    error = error.with_caller_credential_update(current.caller()?);
                }
                Err(error)
            }
        }
    }

    pub(super) async fn logout_owned_session(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<ProviderLogoutResult> {
        self.validate_ownership(account, source, mode)?;
        if source.is_none() && mode != CredentialMode::Server {
            return Err(migu_invalid_request(
                "Migu caller logout requires a caller credential",
            ));
        }
        let _guard = self.auth_mutation.lock().await;
        let selected = self.ownership_source(account, source, mode)?;
        if mode.persists_on_server() {
            self.passport_transactions
                .lock()
                .map_err(|_| lock_error())?
                .cancel_account(account);
        }
        let Some((credential, stored)) = selected else {
            return Ok(ProviderLogoutResult {
                removed: false,
                caller_credential_discard_required: false,
            });
        };
        match self.client.clear_pacm(&credential).await {
            Ok(()) => {}
            Err(e) if e.code == ErrorCode::AuthenticationRequired => {}
            Err(e) => return Err(e.retryable(false)),
        }
        let removed = if let Some(stored) = stored {
            if !self
                .credential_store
                .as_ref()
                .ok_or_else(lock_error)?
                .compare_exchange(&stored, None)?
            {
                return Err(session_changed());
            }
            true
        } else {
            false
        };
        Ok(ProviderLogoutResult {
            removed,
            caller_credential_discard_required: source.is_some(),
        })
    }
}

pub(super) fn validate_account(account: &str, mode: CredentialMode) -> Result<()> {
    if account.is_empty()
        || account.len() > 64
        || account.trim() != account
        || account.chars().any(char::is_control)
        || (mode == CredentialMode::Client && account != "default")
    {
        return Err(migu_invalid_request(
            "Migu account alias is invalid for the requested ownership",
        ));
    }
    Ok(())
}
fn lock_error() -> TuneWeaveError {
    error(
        ErrorCode::InternalError,
        "Migu account storage or session state is unavailable",
    )
}
pub(super) fn session_changed() -> TuneWeaveError {
    error(
        ErrorCode::Conflict,
        "Migu session changed while the request was in flight",
    )
}

#[cfg(test)]
pub(super) mod tests;
