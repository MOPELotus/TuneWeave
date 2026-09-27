use super::session::{Selection, changed, state_error, validate_account};
use super::*;
use crate::{KugouLoginClient, KugouQrPoll, KugouQrSession};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use qrcode::{QrCode, render::svg};
use rand::{TryRng, rngs::SysRng};
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicBool, Ordering},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const CAPACITY: usize = 128;

#[derive(Default)]
pub(super) struct Transactions {
    entries: BTreeMap<String, Transaction>,
    creating: usize,
    pub(super) passwords: BTreeMap<String, super::password::Attempt>,
    pub(super) sms: BTreeMap<String, super::sms::Entry>,
    pub(super) sms_cooldowns: BTreeMap<String, tokio::time::Instant>,
}
#[derive(Clone)]
struct Transaction {
    qr: KugouQrSession,
    mode: CredentialMode,
    binding: Option<Binding>,
    busy: Arc<AtomicBool>,
    deadline: Instant,
}
#[derive(Clone)]
struct Binding {
    account: String,
    previous: Option<Selection>,
}

impl Transactions {
    pub(super) fn cancel_account(&mut self, account: &str) {
        self.sms.retain(|_, entry| {
            !(entry.challenge.credential_mode().persists_on_server()
                && entry.challenge.request().account == account)
        });
        self.passwords
            .retain(|_, entry| !(entry.mode.persists_on_server() && entry.account == account));
        self.entries.retain(|_, entry| {
            let cancel = entry.mode.persists_on_server()
                && entry.binding.as_ref().is_some_and(|v| v.account == account);
            if cancel {
                entry.qr.cancel();
            }
            !cancel
        });
    }
    pub(super) fn prune(&mut self) {
        self.sms
            .retain(|_, entry| tokio::time::Instant::now() < entry.deadline);
        self.sms_cooldowns
            .retain(|_, until| tokio::time::Instant::now() < *until);
        self.passwords
            .retain(|_, entry| Instant::now() < entry.deadline);
        self.entries.retain(|_, entry| {
            let keep = Instant::now() < entry.deadline;
            if !keep {
                entry.qr.cancel();
            }
            keep
        });
    }

    pub(super) fn require_capacity(&self) -> Result<()> {
        if self.entries.len() + self.creating + self.passwords.len() + self.sms.len() >= CAPACITY {
            return Err(rate_limited());
        }
        Ok(())
    }
}

struct Reservation(Arc<Mutex<Transactions>>);
impl Drop for Reservation {
    fn drop(&mut self) {
        if let Ok(mut state) = self.0.lock() {
            state.creating = state.creating.saturating_sub(1);
        }
    }
}
struct PollGuard {
    transactions: Arc<Mutex<Transactions>>,
    id: String,
    busy: Arc<AtomicBool>,
    consuming: bool,
}
impl Drop for PollGuard {
    fn drop(&mut self) {
        if self.consuming {
            if let Ok(mut transactions) = self.transactions.lock() {
                if let Some(entry) = transactions.entries.remove(&self.id) {
                    entry.qr.cancel();
                }
            }
        }
        self.busy.store(false, Ordering::SeqCst);
    }
}

impl KugouProvider {
    /// Cancels a provider QR transaction, including discarding late login completion.
    /// Only a base provider may operate on provider transaction identifiers.
    pub fn cancel_qr_login(&self, id: &str) -> Result<bool> {
        if self.caller_credential.is_some() {
            return Err(kugou_invalid_request(
                "KuGou QR cancellation requires the base provider",
            ));
        }
        valid_id(id)?;
        let mut state = self.qr_transactions.lock().map_err(|_| state_error())?;
        Ok(state
            .entries
            .remove(id)
            .map(|entry| entry.qr.cancel())
            .is_some())
    }

    pub(super) async fn begin_qr(
        &self,
        login_type: Option<&str>,
        mode: CredentialMode,
    ) -> Result<ProviderQrStart> {
        self.require_login_mode(mode)?;
        let kind = match login_type {
            None | Some("standard") => KugouLoginClient::Standard,
            Some("concept") => KugouLoginClient::Concept,
            Some("web") => KugouLoginClient::Web,
            _ => {
                return Err(kugou_invalid_request(
                    "KuGou login_type must be standard, concept or web",
                ));
            }
        };
        let reservation = {
            let mut state = self.qr_transactions.lock().map_err(|_| state_error())?;
            state.prune();
            state.require_capacity()?;
            state.creating += 1;
            Reservation(self.qr_transactions.clone())
        };
        let qr = self.client.create_login_qr(kind).await?;
        let lifetime = qr.expires_in();
        if lifetime.is_zero() {
            return Err(changed());
        }
        let mut id = [0; 32];
        SysRng.try_fill_bytes(&mut id).map_err(|_| state_error())?;
        let id = hex::encode(id);
        let expires = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| state_error())?
            .as_secs()
            .saturating_add(lifetime.as_secs());
        let start = ProviderQrStart {
            provider_transaction_id: id.clone(),
            url: qr.url().to_owned(),
            image_data_url: Some(qr_image_data_url(qr.url())?),
            expires_at: Some(expires.to_string()),
        };
        {
            let mut state = self.qr_transactions.lock().map_err(|_| state_error())?;
            if state.entries.contains_key(&id) {
                return Err(state_error());
            }
            state.entries.insert(
                id,
                Transaction {
                    qr,
                    mode,
                    binding: None,
                    busy: Arc::new(AtomicBool::new(false)),
                    deadline: Instant::now() + lifetime,
                },
            );
        }
        drop(reservation);
        Ok(start)
    }

    pub(super) async fn poll_qr(
        &self,
        id: &str,
        account: &str,
        mode: CredentialMode,
    ) -> Result<ProviderQrPoll> {
        self.require_login_mode(mode)?;
        validate_account(account, mode)?;
        valid_id(id)?;
        let entry = {
            let mut transactions = self.qr_transactions.lock().map_err(|_| state_error())?;
            let entry = transactions.entries.get_mut(id).ok_or_else(not_found)?;
            if entry.mode != mode || entry.binding.as_ref().is_some_and(|v| v.account != account) {
                return Err(kugou_invalid_request(
                    "KuGou QR ownership and account are fixed for this transaction",
                ));
            }
            if Instant::now() >= entry.deadline {
                entry.qr.cancel();
                transactions.entries.remove(id);
                return Ok(progress(AuthState::Expired));
            }
            if entry.busy.load(Ordering::SeqCst) {
                return Err(rate_limited());
            }
            if entry.binding.is_none() {
                let previous = if mode.persists_on_server() {
                    self.selected(account)?
                } else {
                    None
                };
                entry.binding = Some(Binding {
                    account: account.to_owned(),
                    previous,
                });
            }
            entry.busy.store(true, Ordering::SeqCst);
            entry.clone()
        };
        let mut guard = PollGuard {
            transactions: self.qr_transactions.clone(),
            id: id.to_owned(),
            busy: entry.busy.clone(),
            consuming: false,
        };
        let poll = entry.qr.poll().await;
        // A cancellation/expiry during upstream polling always wins over its response.
        if !self.qr_is_current(id, &entry)? {
            guard.consuming = true;
            return Ok(progress(AuthState::Expired));
        }
        match poll? {
            KugouQrPoll::WaitingForScan => Ok(progress(AuthState::Waiting)),
            KugouQrPoll::WaitingForConfirmation => Ok(progress(AuthState::Scanned)),
            KugouQrPoll::Expired => {
                guard.consuming = true;
                Ok(progress(AuthState::Expired))
            }
            KugouQrPoll::AuthorizationReceived(authorization) => {
                // Dropping this future after token consumption also retires the transaction.
                guard.consuming = true;
                let (credential, mut profile) =
                    if authorization.client_kind() == KugouLoginClient::Web {
                        let exchanged = self.client.exchange_web_qr(authorization).await;
                        if !self.qr_is_current(id, &entry)? {
                            return Ok(progress(AuthState::Expired));
                        }
                        let session = exchanged?;
                        let profile = session.profile()?;
                        (KugouCredential::verified_web(session)?, profile)
                    } else {
                        let exchanged = self.client.exchange_qr_authorization(authorization).await;
                        if !self.qr_is_current(id, &entry)? {
                            return Ok(progress(AuthState::Expired));
                        }
                        let session = exchanged?;
                        let result = self.client.native_profile(&session).await;
                        if !self.qr_is_current(id, &entry)? {
                            return Ok(progress(AuthState::Expired));
                        }
                        (KugouCredential::verified(session)?, result?)
                    };
                let caller = credential.caller()?;
                if !profile.authenticated
                    || profile.user_id.as_deref() != Some(credential.user_id())
                {
                    return Err(changed());
                }
                profile.account = account.to_owned();
                let mut state = self.qr_transactions.lock().map_err(|_| state_error())?;
                let current = state.entries.get(id).ok_or_else(changed)?;
                if !Arc::ptr_eq(&current.busy, &entry.busy) {
                    return Err(changed());
                }
                if Instant::now() >= current.deadline {
                    return Ok(progress(AuthState::Expired));
                }
                if mode.persists_on_server() {
                    let store = self.credential_store.as_ref().ok_or_else(state_error)?;
                    let replacement = credential.stored(account)?;
                    let previous = entry
                        .binding
                        .as_ref()
                        .ok_or_else(state_error)?
                        .previous
                        .as_ref();
                    let saved = if let Some((_, Some(stored))) = previous {
                        store.compare_exchange(stored, Some(&replacement))?
                    } else {
                        store.insert_if_absent(&replacement)?
                    };
                    if !saved {
                        return Err(changed());
                    }
                    state.cancel_account(account);
                } else {
                    state.entries.remove(id);
                }
                Ok(ProviderQrPoll {
                    verification: None,
                    state: AuthState::Confirmed,
                    message: None,
                    profile: Some(profile),
                    credential: mode.returns_to_caller().then_some(caller),
                })
            }
        }
    }

    fn qr_is_current(&self, id: &str, entry: &Transaction) -> Result<bool> {
        let state = self.qr_transactions.lock().map_err(|_| state_error())?;
        let current = state.entries.get(id).ok_or_else(changed)?;
        if !Arc::ptr_eq(&current.busy, &entry.busy) {
            return Err(changed());
        }
        Ok(Instant::now() < current.deadline)
    }
}

fn qr_image_data_url(url: &str) -> Result<String> {
    let image = QrCode::new(url.as_bytes())
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "KuGou QR login URL could not be encoded",
            )
            .with_platform(Platform::Kugou)
        })?
        .render::<svg::Color>()
        .min_dimensions(320, 320)
        .build();
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        BASE64.encode(image.as_bytes())
    ))
}

fn progress(state: AuthState) -> ProviderQrPoll {
    ProviderQrPoll {
        verification: None,
        state,
        message: None,
        profile: None,
        credential: None,
    }
}
fn valid_id(id: &str) -> Result<()> {
    if id.len() != 64
        || !id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(not_found());
    }
    Ok(())
}
fn not_found() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::ResourceNotFound,
        "KuGou QR transaction is unavailable",
    )
    .with_platform(Platform::Kugou)
}
fn rate_limited() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::RateLimited,
        "KuGou QR transaction capacity or polling limit reached",
    )
    .with_platform(Platform::Kugou)
    .with_details(json!({"retry_after_secs":2}))
}

#[cfg(test)]
mod tests;
