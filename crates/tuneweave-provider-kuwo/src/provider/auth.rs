use super::*;
use crate::client::native::{
    credential::{self, NativeCredential},
    password,
};
use std::{
    collections::BTreeMap,
    future::Future,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tuneweave_core::{
    AccountProfile, CredentialMode, ErrorCode, PasswordLoginRequest, ProviderAuthResult,
    ProviderCredential, ProviderLogoutResult, StoredAccountCredential,
};

mod following_artists;
mod library;
mod management;
mod media;
mod membership;
mod password_web;
mod playlist;
#[cfg(test)]
mod profile_tests;
mod revocation;
mod sms;
mod submissions;
#[cfg(test)]
mod tests;
const CAPACITY: usize = 128;
const LOGIN_BUDGET: Duration = Duration::from_secs(300);

#[derive(Default)]
pub(super) struct Registry {
    next: u64,
    attempts: BTreeMap<u64, Attempt>,
    sms_cooldowns: BTreeMap<String, Instant>,
}
struct Attempt {
    account: String,
    mode: CredentialMode,
    deadline: Instant,
    sms: Option<sms::Entry>,
    web_password: Option<password_web::Entry>,
}
impl Registry {
    fn prune(&mut self) {
        self.attempts.retain(|_, v| Instant::now() < v.deadline);
        self.sms_cooldowns
            .retain(|_, deadline| Instant::now() < *deadline);
    }
    fn cancel_account(&mut self, account: &str) {
        self.attempts
            .retain(|_, v| !(v.mode.persists_on_server() && v.account == account));
    }
}
struct Lease {
    registry: Arc<Mutex<Registry>>,
    id: u64,
    deadline: Instant,
    armed: bool,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if self.armed {
            if let Ok(mut registry) = self.registry.lock() {
                registry.attempts.remove(&self.id);
            }
        }
    }
}
#[derive(Clone)]
struct Selection {
    credential: NativeCredential,
    stored: Option<StoredAccountCredential>,
}

impl KuwoProvider {
    fn require_base(&self) -> Result<()> {
        if self.caller_credential.is_some() {
            return Err(kuwo_invalid_request(
                "Kuwo authentication requires the base provider",
            ));
        }
        Ok(())
    }
    fn require_mode(&self, account: &str, mode: CredentialMode) -> Result<()> {
        self.require_base()?;
        validate_account(account, mode)?;
        if mode.persists_on_server() && self.credential_store.is_none() {
            return Err(state_error());
        }
        Ok(())
    }
    fn selected(&self, account: &str) -> Result<Option<Selection>> {
        validate_account(account, CredentialMode::Server)?;
        if let Some(caller) = &self.caller_credential {
            if account != "default" {
                return Err(kuwo_invalid_request(
                    "Kuwo caller credentials cannot select a server account",
                ));
            }
            return Ok(caller
                .lock()
                .map_err(|_| state_error())?
                .clone()
                .map(|credential| Selection {
                    credential,
                    stored: None,
                }));
        }
        let Some(store) = &self.credential_store else {
            return Ok(None);
        };
        let values = store.load_platform(Platform::Kuwo)?;
        let mut matches = values.into_iter().filter(|v| v.account == account);
        let Some(stored) = matches.next() else {
            return Ok(None);
        };
        if stored.platform != Platform::Kuwo || matches.next().is_some() {
            return Err(state_error());
        }
        let credential = NativeCredential::parse_stored(&stored).map_err(|_| state_error())?;
        Ok(Some(Selection {
            credential,
            stored: Some(stored),
        }))
    }
    fn owned_source(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<Option<Selection>> {
        self.require_mode(account, mode)?;
        if (mode == CredentialMode::Server && source.is_some())
            || (mode == CredentialMode::Client && source.is_none())
        {
            return Err(kuwo_invalid_request(
                "Kuwo credential source conflicts with its ownership mode",
            ));
        }
        let caller = source.map(NativeCredential::parse).transpose()?;
        if mode == CredentialMode::Client {
            return Ok(caller.map(|credential| Selection {
                credential,
                stored: None,
            }));
        }
        let stored = self.selected(account)?;
        if let Some(caller) = caller {
            let selected = stored.as_ref().ok_or_else(authentication_required)?;
            if !caller.same_login(&selected.credential) {
                return Err(changed());
            }
        }
        Ok(stored)
    }
    fn reserve(&self, account: &str, mode: CredentialMode) -> Result<(Lease, Option<Selection>)> {
        self.require_mode(account, mode)?;
        let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
        registry.prune();
        if registry.attempts.len() >= CAPACITY {
            return Err(TuneWeaveError::new(
                ErrorCode::RateLimited,
                "Kuwo has too many active login requests",
            )
            .with_platform(Platform::Kuwo)
            .with_details(json!({"retry_after_secs":2})));
        }
        let previous = if mode.persists_on_server() {
            self.selected(account)?
        } else {
            None
        };
        let id = registry.next;
        registry.next = id.checked_add(1).ok_or_else(state_error)?;
        let deadline = Instant::now() + LOGIN_BUDGET;
        registry.attempts.insert(
            id,
            Attempt {
                account: account.into(),
                mode,
                deadline,
                sms: None,
                web_password: None,
            },
        );
        Ok((
            Lease {
                registry: self.auth_registry.clone(),
                id,
                deadline,
                armed: true,
            },
            previous,
        ))
    }
    fn check_attempt(
        &self,
        registry: &mut Registry,
        lease: &Lease,
        previous: Option<&Selection>,
    ) -> Result<()> {
        self.check_attempt_id(registry, lease.id, previous)
    }
    fn check_attempt_id(
        &self,
        registry: &mut Registry,
        id: u64,
        previous: Option<&Selection>,
    ) -> Result<()> {
        registry.prune();
        let entry = registry.attempts.get(&id).ok_or_else(changed)?;
        if entry.mode.persists_on_server() {
            let current = self.selected(&entry.account)?;
            if current.as_ref().and_then(|v| v.stored.as_ref())
                != previous.and_then(|v| v.stored.as_ref())
            {
                return Err(changed());
            }
        }
        Ok(())
    }
    async fn boundary<T>(
        &self,
        lease: &Lease,
        previous: Option<&Selection>,
        future: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        {
            let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
            self.check_attempt(&mut registry, lease, previous)?;
        }
        // The same deadline covers the whole transaction, including device creation.
        let result =
            tokio::time::timeout_at(tokio::time::Instant::from_std(lease.deadline), future)
                .await
                .unwrap_or_else(|_| {
                    Err(TuneWeaveError::new(
                        ErrorCode::UpstreamError,
                        "Kuwo login request timed out",
                    )
                    .with_platform(Platform::Kuwo)
                    .retryable(false))
                });
        let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
        self.check_attempt(&mut registry, lease, previous)?;
        result
    }
    fn commit_auth(
        &self,
        lease: &Lease,
        previous: Option<&Selection>,
        credential: NativeCredential,
        nickname: Option<String>,
    ) -> Result<ProviderAuthResult> {
        let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
        self.check_attempt(&mut registry, lease, previous)?;
        let attempt = registry.attempts.get(&lease.id).ok_or_else(changed)?;
        let (account, mode) = (attempt.account.clone(), attempt.mode);
        let caller = credential.caller()?;
        let mut profile = credential::profile(&credential.input()?, nickname);
        profile.account = account.clone();
        if mode.persists_on_server() {
            let store = self.credential_store.as_ref().ok_or_else(state_error)?;
            let replacement = credential.stored(&account)?;
            let saved = match previous.and_then(|v| v.stored.as_ref()) {
                Some(previous) => store.compare_exchange(previous, Some(&replacement))?,
                None => store.insert_if_absent(&replacement)?,
            };
            if !saved {
                return Err(changed());
            }
            registry.cancel_account(&account);
        } else {
            registry.attempts.remove(&lease.id);
        }
        Ok(ProviderAuthResult {
            profile,
            credential: mode.returns_to_caller().then_some(caller),
        })
    }

    pub(super) async fn login_password(
        &self,
        request: &PasswordLoginRequest,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        password::validate_request(request)?;
        let (lease, previous) = self.reserve(&request.account, mode)?;
        let device = self
            .boundary(
                &lease,
                previous.as_ref(),
                self.device_store.initialize(&self.client),
            )
            .await?;
        let authorization = self
            .boundary(
                &lease,
                previous.as_ref(),
                self.client.authenticate_native_password(request, &device),
            )
            .await?;
        self.boundary(
            &lease,
            previous.as_ref(),
            self.client.validate_native_session(authorization.session()),
        )
        .await?;
        let credential = NativeCredential::verified(authorization.session())?;
        self.commit_auth(
            &lease,
            previous.as_ref(),
            credential,
            authorization.nickname().map(str::to_owned),
        )
    }
    pub(super) async fn refresh_owned(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        let selected = self
            .owned_source(account, source, mode)?
            .ok_or_else(authentication_required)?;
        let (lease, previous) = self.reserve(account, mode)?;
        if mode.persists_on_server()
            && previous.as_ref().and_then(|v| v.stored.as_ref()) != selected.stored.as_ref()
        {
            return Err(changed());
        }
        let input = selected.credential.input()?;
        let exchange = self
            .boundary(
                &lease,
                previous.as_ref(),
                self.client.exchange_native_session(&input),
            )
            .await?;
        self.boundary(
            &lease,
            previous.as_ref(),
            self.client.validate_native_session(exchange.session()),
        )
        .await?;
        // Preserve generation only after the exact replacement SID is independently valid.
        let credential = selected.credential.rotate(exchange.session())?;
        self.commit_auth(
            &lease,
            previous.as_ref(),
            credential,
            exchange.nickname().map(str::to_owned),
        )
    }
    pub(super) fn logout_owned(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<ProviderLogoutResult> {
        let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
        let selected = self.owned_source(account, source, mode)?;
        let removed = if mode.persists_on_server() {
            match selected.as_ref().and_then(|v| v.stored.as_ref()) {
                Some(stored) => {
                    if !self
                        .credential_store
                        .as_ref()
                        .ok_or_else(state_error)?
                        .compare_exchange(stored, None)?
                    {
                        return Err(changed());
                    }
                    true
                }
                None => false,
            }
        } else {
            selected.is_some()
        };
        if mode.persists_on_server() {
            registry.cancel_account(account);
        }
        Ok(ProviderLogoutResult {
            removed,
            caller_credential_discard_required: mode.returns_to_caller(),
        })
    }
    fn check_selection(&self, account: &str, expected: &Selection) -> Result<()> {
        let current = self.selected(account)?.ok_or_else(changed)?;
        if current.credential != expected.credential || current.stored != expected.stored {
            return Err(changed());
        }
        Ok(())
    }
    async fn read_selected(&self, account: &str, selected: Selection) -> Result<AccountProfile> {
        let input = selected.credential.input()?;
        let result = self.client.validate_native_session(&input).await;
        self.finish_selected(account, &selected, result)?;
        let mut profile = credential::profile(&input, None);
        profile.account = account.into();
        Ok(profile)
    }
    fn finish_selected<T>(
        &self,
        account: &str,
        selected: &Selection,
        result: Result<T>,
    ) -> Result<T> {
        let mut registry = self.auth_registry.lock().map_err(|_| state_error())?;
        self.check_selection(account, selected)?;
        if let Err(error) = result {
            if error.code == ErrorCode::AuthenticationRequired {
                if let Some(stored) = &selected.stored {
                    if !self
                        .credential_store
                        .as_ref()
                        .ok_or_else(state_error)?
                        .compare_exchange(stored, None)?
                    {
                        return Err(changed());
                    }
                    registry.cancel_account(account);
                } else if let Some(caller) = &self.caller_credential {
                    let mut current = caller.lock().map_err(|_| state_error())?;
                    if current.as_ref() != Some(&selected.credential) {
                        return Err(changed());
                    }
                    *current = None;
                }
            }
            return Err(error);
        }
        result
    }
    pub(super) async fn read_session(&self, account: &str) -> Result<AccountProfile> {
        let selected = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        self.read_selected(account, selected).await
    }
    pub(super) fn caller_scope(&self, credential: &ProviderCredential) -> Result<Self> {
        self.require_base()?;
        let credential = NativeCredential::parse(credential)?;
        let mut scoped = self.clone();
        scoped.caller_credential = Some(Arc::new(Mutex::new(Some(credential))));
        Ok(scoped)
    }
    pub(super) fn require_public_scope(&self) -> Result<()> {
        if self.caller_credential.is_some() {
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "Kuwo account music operations are not implemented for this credential",
            )
            .with_platform(Platform::Kuwo));
        }
        Ok(())
    }
    pub(super) async fn read_user_profile(
        &self,
        id: &str,
        backend: tuneweave_core::UserProfileBackend,
        account: Option<&str>,
    ) -> Result<tuneweave_core::UserProfile> {
        use tuneweave_core::UserProfileBackend;
        if backend != UserProfileBackend::Modern {
            return Err(TuneWeaveError::unsupported(
                Platform::Kuwo,
                Capability::UserProfileLegacy,
            ));
        }
        let account = account
            .or_else(|| self.caller_credential.as_ref().map(|_| "default"))
            .ok_or_else(authentication_required)?;
        let selected = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        if selected.credential.input()?.user_id() != id {
            return Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "Kuwo self profile cannot resolve another user",
            )
            .with_platform(Platform::Kuwo));
        }
        let input = selected.credential.input()?;
        crate::client::native::validate_session_metadata(&input)?;
        let validation = self.client.validate_native_session(&input).await;
        self.finish_selected(account, &selected, validation)?;
        let result = self.client.fetch_native_self_profile(&input).await;
        self.finish_selected(account, &selected, result)
    }
}
fn validate_account(account: &str, mode: CredentialMode) -> Result<()> {
    if account.is_empty()
        || account.len() > 64
        || account.trim() != account
        || account.chars().any(char::is_control)
        || (mode == CredentialMode::Client && account != "default")
    {
        return Err(kuwo_invalid_request(
            "Kuwo account alias is invalid for this ownership mode",
        ));
    }
    Ok(())
}
fn state_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        "Kuwo account state or store is unavailable",
    )
    .with_platform(Platform::Kuwo)
}
fn changed() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::Conflict,
        "Kuwo account or authentication transaction changed",
    )
    .with_platform(Platform::Kuwo)
}
fn authentication_required() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "Kuwo account authentication is required",
    )
    .with_platform(Platform::Kuwo)
}
