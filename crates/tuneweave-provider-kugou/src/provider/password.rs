use super::session::{Selection, changed, state_error, validate_account};
use super::*;
use rand::{TryRng, rngs::SysRng};
use std::time::{Duration, Instant};
use tuneweave_core::{PasswordLoginBackend, PasswordLoginRequest};

pub(super) struct Attempt {
    pub(super) account: String,
    pub(super) mode: CredentialMode,
    pub(super) deadline: Instant,
    native: Option<native::Entry>,
    web: Option<web::Entry>,
}

impl Attempt {
    pub(in crate::provider) fn web_sms_target(&self) -> Option<(&str, bool)> {
        let entry = self.web.as_ref()?;
        Some((entry.phone.as_deref()?, entry.context.is_none()))
    }
}

struct Lease {
    registry: Arc<Mutex<qr::Transactions>>,
    id: String,
    armed: bool,
}
impl Drop for Lease {
    fn drop(&mut self) {
        if self.armed {
            if let Ok(mut registry) = self.registry.lock() {
                registry.passwords.remove(&self.id);
            }
        }
    }
}

impl KugouProvider {
    fn reserve_password(
        &self,
        request: &PasswordLoginRequest,
        mode: CredentialMode,
    ) -> Result<(Lease, Option<Selection>)> {
        self.require_login_mode(mode)?;
        validate_account(&request.account, mode)?;
        match request.backend {
            PasswordLoginBackend::Default | PasswordLoginBackend::Web => {
                crate::web::password::validate(request)?
            }
            PasswordLoginBackend::Native => crate::login::native_password::validate(request)?,
        }
        let mut random = [0; 32];
        SysRng
            .try_fill_bytes(&mut random)
            .map_err(|_| state_error())?;
        let id = hex::encode(random);
        let previous = {
            let mut registry = self.qr_transactions.lock().map_err(|_| state_error())?;
            registry.prune();
            registry.require_capacity()?;
            let previous = if mode.persists_on_server() {
                self.selected(&request.account)?
            } else {
                None
            };
            if registry.passwords.contains_key(&id) {
                return Err(state_error());
            }
            registry.passwords.insert(
                id.clone(),
                Attempt {
                    account: request.account.clone(),
                    mode,
                    deadline: Instant::now() + Duration::from_secs(300),
                    native: None,
                    web: None,
                },
            );
            previous
        };
        // Passwords are never retained; native verification may retain a bound identity.
        let lease = Lease {
            registry: self.qr_transactions.clone(),
            id,
            armed: true,
        };
        Ok((lease, previous))
    }

    pub(super) async fn login_password(
        &self,
        request: &PasswordLoginRequest,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        let (lease, previous) = self.reserve_password(request, mode)?;
        let result = async {
            match request.backend {
                PasswordLoginBackend::Default | PasswordLoginBackend::Web => {
                    let session = self.client.authenticate_web_password(request).await?;
                    Ok((session.profile()?, KugouCredential::verified_web(session)?))
                }
                PasswordLoginBackend::Native => {
                    let session = self.client.authenticate_native_password(request).await?;
                    // Do not start another authenticated request after this attempt loses ownership.
                    self.check_password(&lease, &request.account, mode, &previous)?;
                    let profile = self.client.native_profile(&session).await?;
                    Ok((profile, KugouCredential::verified(session)?))
                }
            }
        }
        .await;
        self.commit_password(&lease, &request.account, mode, &previous, result)
    }

    fn check_password(
        &self,
        lease: &Lease,
        account: &str,
        mode: CredentialMode,
        previous: &Option<Selection>,
    ) -> Result<()> {
        let mut registry = self.qr_transactions.lock().map_err(|_| state_error())?;
        registry.prune();
        if !registry.passwords.contains_key(&lease.id) {
            return Err(changed());
        }
        if mode.persists_on_server() && &self.selected(account)? != previous {
            return Err(changed());
        }
        Ok(())
    }

    fn commit_password(
        &self,
        lease: &Lease,
        account: &str,
        mode: CredentialMode,
        previous: &Option<Selection>,
        result: Result<(AccountProfile, KugouCredential)>,
    ) -> Result<ProviderAuthResult> {
        let mut registry = self.qr_transactions.lock().map_err(|_| state_error())?;
        registry.prune();
        if !registry.passwords.contains_key(&lease.id) {
            return Err(changed());
        }
        if mode.persists_on_server() && &self.selected(account)? != previous {
            return Err(changed());
        }
        let (mut profile, credential) = result?;
        let caller = credential.caller()?;
        if mode.persists_on_server() {
            let store = self.credential_store.as_ref().ok_or_else(state_error)?;
            let replacement = credential.stored(account)?;
            let committed = match previous.as_ref() {
                Some((_, Some(previous))) => {
                    store.compare_exchange(previous, Some(&replacement))?
                }
                None => store.insert_if_absent(&replacement)?,
                _ => return Err(state_error()),
            };
            if !committed {
                return Err(changed());
            }
            registry.cancel_account(account);
        } else {
            registry.passwords.remove(&lease.id);
        }
        profile.account = account.to_owned();
        Ok(ProviderAuthResult {
            profile,
            credential: mode.returns_to_caller().then_some(caller),
        })
    }
}

mod native;
mod web;

#[cfg(test)]
mod native_tests;
#[cfg(test)]
mod tests;
