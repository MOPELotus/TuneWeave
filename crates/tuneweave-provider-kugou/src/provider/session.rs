use super::*;
use tuneweave_core::StoredAccountCredential;

pub(super) type Selection = (KugouCredential, Option<StoredAccountCredential>);

pub(super) struct AccountRead {
    current: KugouCredential,
    stored: Option<StoredAccountCredential>,
    pub(super) vip_type: Option<u32>,
}
impl AccountRead {
    pub(super) fn web_session(&self) -> Option<&crate::web::WebSession> {
        match &self.current {
            KugouCredential::Web(v) => Some(&v.session),
            KugouCredential::Native(_) => None,
        }
    }
    pub(super) fn session(&self) -> Result<&crate::credential::NativeSession> {
        match &self.current {
            KugouCredential::Native(native) => Ok(&native.session),
            KugouCredential::Web(_) => Err(state_error()),
        }
    }
}

impl KugouProvider {
    pub(super) async fn begin_native_read(
        &self,
        account: &str,
        expected_uid: Option<&str>,
    ) -> Result<AccountRead> {
        self.begin_account_read(account, expected_uid, true).await
    }

    pub(super) async fn begin_media_read(&self, account: &str) -> Result<AccountRead> {
        self.begin_account_read(account, None, false).await
    }

    async fn begin_account_read(
        &self,
        account: &str,
        expected_uid: Option<&str>,
        native_only: bool,
    ) -> Result<AccountRead> {
        if expected_uid.is_some_and(|uid| !crate::credential::valid_uid(uid)) {
            return Err(kugou_invalid_request("KuGou account user ID is invalid"));
        }
        let (original, mut stored) = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        if native_only && !matches!(original, KugouCredential::Native(_)) {
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "This KuGou account operation requires a native app credential",
            )
            .with_platform(Platform::Kugou));
        }
        let user_id = match &original {
            KugouCredential::Native(v) => &v.session.user_id,
            KugouCredential::Web(v) => &v.session.user_id,
        };
        if expected_uid.is_some_and(|uid| uid != user_id) {
            return Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "KuGou library belongs to a different account",
            )
            .with_platform(Platform::Kugou));
        }
        let next = match &original {
            KugouCredential::Native(native) => self
                .client
                .exchange_native_token(&native.session)
                .await
                .and_then(|session| native.rotate(session))
                .map(KugouCredential::Native),
            KugouCredential::Web(web) => self
                .client
                .refresh_web_session(&web.session)
                .await
                .and_then(|session| original.rotate_web(session)),
        };
        let next = match next {
            Ok(next) => next,
            Err(failure) => {
                return Err(self.finish_read_error(
                    &original,
                    &mut stored,
                    failure,
                    false,
                    self.caller_credential.is_some(),
                ));
            }
        };
        if let Err(failure) = self.apply_read(&original, &mut stored, &next) {
            self.discard_response_credential_after_error(failure.code)?;
            return Err(failure);
        }
        let mut read = AccountRead {
            current: next,
            stored,
            vip_type: None,
        };
        let profile = match &read.current {
            KugouCredential::Native(v) => self.client.native_profile(&v.session).await,
            KugouCredential::Web(v) => v.session.profile(),
        };
        match profile {
            Ok(profile) => {
                read.vip_type = profile
                    .extensions
                    .get("vip_type")
                    .and_then(serde_json::Value::as_u64)
                    .and_then(|v| u32::try_from(v).ok())
            }
            Err(failure) => return self.finish_account_read(read, Err(failure)),
        }
        self.check_account_read(&mut read)?;
        Ok(read)
    }

    pub(super) fn check_account_read(&self, read: &mut AccountRead) -> Result<()> {
        let result = self.apply_read(&read.current, &mut read.stored, &read.current);
        if let Err(failure) = &result {
            self.discard_response_credential_after_error(failure.code)?;
        }
        result
    }

    pub(super) fn finish_account_read<T>(
        &self,
        mut read: AccountRead,
        result: Result<T>,
    ) -> Result<T> {
        match result {
            Ok(value) => {
                self.check_account_read(&mut read)?;
                Ok(value)
            }
            Err(failure) => Err(self.finish_read_error(
                &read.current,
                &mut read.stored,
                failure,
                true,
                self.caller_credential.is_some(),
            )),
        }
    }

    pub(super) fn caller_scope(&self, credential: &ProviderCredential) -> Result<Self> {
        Ok(Self {
            client: self.client.clone(),
            credential_store: None,
            caller_credential: Some(Arc::new(Mutex::new(Some(KugouCredential::parse_caller(
                credential,
            )?)))),
            response_credential: Arc::default(),
            qr_transactions: self.qr_transactions.clone(),
        })
    }

    pub(super) fn require_public_source(&self) -> Result<()> {
        if self.caller_credential.is_some() {
            return Err(kugou_invalid_request(
                "This KuGou catalogue/media operation does not yet accept account credentials",
            ));
        }
        Ok(())
    }

    pub(super) fn require_login_mode(&self, mode: CredentialMode) -> Result<()> {
        if self.caller_credential.is_some() {
            return Err(kugou_invalid_request(
                "KuGou authentication requires the base provider",
            ));
        }
        if mode.persists_on_server() && self.credential_store.is_none() {
            return Err(state_error());
        }
        Ok(())
    }

    pub(super) fn selected(&self, account: &str) -> Result<Option<Selection>> {
        validate_account(account, CredentialMode::Server)?;
        if let Some(caller) = &self.caller_credential {
            if account != "default" {
                return Err(kugou_invalid_request(
                    "KuGou caller credentials cannot select a server account alias",
                ));
            }
            return Ok(caller
                .lock()
                .map_err(|_| state_error())?
                .clone()
                .map(|v| (v, None)));
        }
        let Some(store) = &self.credential_store else {
            return Ok(None);
        };
        let values = store.load_platform(Platform::Kugou)?;
        let mut matches = values.into_iter().filter(|v| v.account == account);
        let Some(stored) = matches.next() else {
            return Ok(None);
        };
        if matches.next().is_some() {
            return Err(state_error());
        }
        let credential = KugouCredential::parse_stored(&stored).map_err(|_| state_error())?;
        Ok(Some((credential, Some(stored))))
    }

    fn ownership_source(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<Option<Selection>> {
        validate_account(account, mode)?;
        self.require_login_mode(mode)?;
        if (source.is_some() && mode == CredentialMode::Server)
            || (source.is_none() && mode == CredentialMode::Client)
        {
            return Err(kugou_invalid_request(
                "KuGou credential source conflicts with the selected ownership",
            ));
        }
        let caller = source.map(KugouCredential::parse_caller).transpose()?;
        if mode == CredentialMode::Client {
            return Ok(caller.map(|v| (v, None)));
        }
        let stored = self.selected(account)?;
        if let (Some(caller), Some((server, _))) = (&caller, &stored) {
            if !caller.same_login(server) {
                return Err(changed());
            }
        }
        // Both explicitly selects the latest server token of the same login generation.
        Ok(stored)
    }

    pub(super) fn apply_read(
        &self,
        source: &KugouCredential,
        stored: &mut Option<StoredAccountCredential>,
        next: &KugouCredential,
    ) -> Result<()> {
        if !source.same_login(next) {
            return Err(changed());
        }
        if let Some(previous) = stored.as_ref() {
            let replacement = next.stored(&previous.account)?;
            if !self
                .credential_store
                .as_ref()
                .ok_or_else(state_error)?
                .compare_exchange(previous, Some(&replacement))?
            {
                return Err(changed());
            }
            *stored = Some(replacement);
        } else if let Some(state) = &self.caller_credential {
            let mut caller = state.lock().map_err(|_| state_error())?;
            if caller.as_ref() != Some(source) {
                return Err(changed());
            }
            if source != next {
                let issued = next.caller()?;
                let mut response = self.response_credential.lock().map_err(|_| state_error())?;
                *caller = Some(next.clone());
                *response = Some(issued);
            }
        }
        Ok(())
    }

    pub(super) fn finish_read_error(
        &self,
        current: &KugouCredential,
        stored: &mut Option<StoredAccountCredential>,
        mut failure: TuneWeaveError,
        accepted: bool,
        returns_to_caller: bool,
    ) -> TuneWeaveError {
        let check = if failure.code == ErrorCode::AuthenticationRequired {
            if let Some(previous) = stored.as_ref() {
                self.credential_store
                    .as_ref()
                    .ok_or_else(state_error)
                    .and_then(|store| store.compare_exchange(previous, None))
                    .and_then(|removed| if removed { Ok(()) } else { Err(changed()) })
            } else if let Some(state) = &self.caller_credential {
                (|| {
                    let mut caller = state.lock().map_err(|_| state_error())?;
                    if caller.as_ref() != Some(current) {
                        return Err(changed());
                    }
                    *caller = None;
                    Ok(())
                })()
            } else {
                Ok(())
            }
        } else {
            self.apply_read(current, stored, current)
        };
        if let Err(conflict) = check {
            failure = conflict;
        }
        if let Err(error) = self.discard_response_credential_after_error(failure.code) {
            return error;
        }
        if accepted
            && returns_to_caller
            && !matches!(
                failure.code,
                ErrorCode::AuthenticationRequired | ErrorCode::Conflict
            )
        {
            match current.caller() {
                Ok(caller) => failure = failure.with_caller_credential_update(caller),
                Err(error) => return error,
            }
        }
        failure
    }

    async fn verified_account_read(
        &self,
        account: &str,
        selection: Selection,
        returns_to_caller: bool,
    ) -> Result<ProviderAuthResult> {
        let (original, mut stored) = selection;
        let mut current = original.clone();
        let exchanged = match &current {
            KugouCredential::Native(native) => self
                .client
                .exchange_native_token(&native.session)
                .await
                .and_then(|session| native.rotate(session))
                .map(KugouCredential::Native),
            KugouCredential::Web(web) => self
                .client
                .refresh_web_session(&web.session)
                .await
                .and_then(|session| current.rotate_web(session)),
        };
        let next = match exchanged {
            Ok(next) => next,
            Err(error) => {
                return Err(self.finish_read_error(
                    &current,
                    &mut stored,
                    error,
                    false,
                    returns_to_caller,
                ));
            }
        };
        if let Err(error) = self.apply_read(&current, &mut stored, &next) {
            self.discard_response_credential_after_error(error.code)?;
            return Err(error);
        }
        current = next;
        let result = match &current {
            KugouCredential::Native(native) => self.client.native_profile(&native.session).await,
            KugouCredential::Web(web) => web.session.profile(),
        };
        let mut profile = match result {
            Ok(profile) => profile,
            Err(error) => {
                return Err(self.finish_read_error(
                    &current,
                    &mut stored,
                    error,
                    true,
                    returns_to_caller,
                ));
            }
        };
        // Even a response without rotation must not outlive logout or another login.
        if let Err(error) = self.apply_read(&current, &mut stored, &current) {
            self.discard_response_credential_after_error(error.code)?;
            return Err(error);
        }
        profile.account = account.to_owned();
        Ok(ProviderAuthResult {
            profile,
            credential: returns_to_caller.then(|| current.caller()).transpose()?,
        })
    }

    pub(super) async fn read_session(&self, account: &str) -> Result<AccountProfile> {
        let empty = || AccountProfile {
            platform: Platform::Kugou,
            account: account.to_owned(),
            user_id: None,
            nickname: None,
            avatar_url: None,
            authenticated: false,
            extensions: Default::default(),
        };
        let Some(selected) = self.selected(account)? else {
            return Ok(empty());
        };
        // The self-profile may omit UID, so caller input must first be verified by token exchange.
        match self
            .verified_account_read(account, selected, self.caller_credential.is_some())
            .await
        {
            Ok(result) => Ok(result.profile),
            Err(error) if error.code == ErrorCode::AuthenticationRequired => Ok(empty()),
            Err(error) => Err(error),
        }
    }

    pub(super) async fn refresh_owned(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        let selected = self
            .ownership_source(account, source, mode)?
            .ok_or_else(authentication_required)?;
        self.verified_account_read(account, selected, mode.returns_to_caller())
            .await
    }

    pub(super) async fn read_user_profile(
        &self,
        id: &str,
        backend: tuneweave_core::UserProfileBackend,
        account: Option<&str>,
    ) -> Result<tuneweave_core::UserProfile> {
        use tuneweave_core::{ResourceRef, User, UserProfile, UserProfileBackend};
        if backend != UserProfileBackend::Modern {
            return Err(TuneWeaveError::unsupported(
                Platform::Kugou,
                Capability::UserProfileLegacy,
            ));
        }
        let account = account
            .or_else(|| self.caller_credential.as_ref().map(|_| "default"))
            .ok_or_else(authentication_required)?;
        let selected = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        if selected.0.user_id() != id {
            return Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "KuGou self profile cannot resolve another user",
            )
            .with_platform(Platform::Kugou));
        }
        let profile = self
            .verified_account_read(account, selected, self.caller_credential.is_some())
            .await?
            .profile;
        Ok(UserProfile {
            user: User {
                resource_ref: ResourceRef::new(Platform::Kugou, id).map_err(|_| state_error())?,
                platform: Platform::Kugou,
                id: id.to_owned(),
                name: profile.nickname.unwrap_or_default(),
                avatar_url: profile.avatar_url,
                signature: None,
                followed: None,
                mutual: None,
                extensions: Default::default(),
            },
            level: None,
            listened_track_count: None,
            playlist_count: None,
            playlist_subscriber_count: None,
            following_count: None,
            follower_count: None,
            event_count: None,
            birthday: None,
            created_at: None,
            background_url: None,
            description: None,
            public_listening_history: None,
            extensions: Default::default(),
        })
    }

    pub(super) fn logout_owned(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<ProviderLogoutResult> {
        if source.is_none() && mode != CredentialMode::Server {
            return Err(kugou_invalid_request(
                "KuGou caller logout requires the caller credential",
            ));
        }
        let selected = self.ownership_source(account, source, mode)?;
        let mut transactions = self.qr_transactions.lock().map_err(|_| state_error())?;
        let removed = if let Some((_, Some(stored))) = selected {
            if !self
                .credential_store
                .as_ref()
                .ok_or_else(state_error)?
                .compare_exchange(&stored, None)?
            {
                return Err(changed());
            }
            transactions.cancel_account(account);
            true
        } else {
            if mode == CredentialMode::Server {
                transactions.cancel_account(account);
            }
            false
        };
        // This only removes local ownership; it does not claim upstream token revocation.
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
        return Err(kugou_invalid_request(
            "KuGou account alias is invalid for the selected ownership",
        ));
    }
    Ok(())
}
pub(super) fn state_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        "KuGou account state or store is unavailable",
    )
    .with_platform(Platform::Kugou)
}
pub(super) fn changed() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::Conflict,
        "KuGou account or login transaction changed while the request was in flight",
    )
    .with_platform(Platform::Kugou)
}
pub(super) fn authentication_required() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "KuGou account authentication is required",
    )
    .with_platform(Platform::Kugou)
}

#[cfg(test)]
pub(super) mod tests;
