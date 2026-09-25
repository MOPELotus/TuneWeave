use std::{
    collections::BTreeMap,
    future::Future,
    sync::{Arc, Mutex},
    time::Instant,
};

use tuneweave_core::{
    AccountCredentialStore, CredentialMode, ErrorCode, Platform, Result, StoredAccountCredential,
    TuneWeaveError,
};

const MAX_ATTEMPTS: usize = 128;

#[derive(Clone, Default)]
pub(crate) struct AuthTransactions(Arc<Mutex<Attempts>>);

#[derive(Default)]
struct Attempts {
    sequence: u64,
    active: BTreeMap<u64, Attempt>,
}

struct Attempt {
    account: Option<String>,
    mode: CredentialMode,
    deadline: Instant,
}

// The registry contains only metadata. Dropping a lease never drops another lease
// while holding its mutex. No mutex is held across network I/O.
pub(crate) struct AuthLease {
    transactions: AuthTransactions,
    id: u64,
    pub(crate) account: Option<String>,
    pub(crate) mode: CredentialMode,
    pub(crate) deadline: Instant,
    store: Option<Arc<dyn AccountCredentialStore>>,
    expected: Option<StoredAccountCredential>,
    legacy_snapshot: Option<[u8; 32]>,
}

impl AuthTransactions {
    pub(crate) fn reserve(
        &self,
        account: Option<&str>,
        mode: CredentialMode,
        store: Option<Arc<dyn AccountCredentialStore>>,
        deadline: Instant,
    ) -> Result<AuthLease> {
        let mut attempts = self.0.lock().map_err(|_| internal())?;
        attempts
            .active
            .retain(|_, entry| Instant::now() < entry.deadline);
        if Instant::now() >= deadline {
            return Err(expired());
        }
        if attempts.active.len() >= MAX_ATTEMPTS {
            return Err(TuneWeaveError::new(
                ErrorCode::RateLimited,
                "Soda authentication transaction capacity has been reached",
            )
            .with_platform(Platform::Soda)
            .retryable(true));
        }
        let store = if mode.persists_on_server() {
            Some(store.ok_or_else(internal)?)
        } else {
            None
        };
        let rows = store
            .as_ref()
            .map(|store| store.load_platform(Platform::Soda))
            .transpose()?
            .unwrap_or_default();
        let expected = account
            .and_then(|account| rows.iter().find(|row| row.account == account))
            .cloned();
        let legacy_snapshot =
            (mode.persists_on_server() && account.is_none()).then(|| fingerprint(&rows));
        attempts.sequence = attempts.sequence.checked_add(1).ok_or_else(internal)?;
        let id = attempts.sequence;
        attempts.active.insert(
            id,
            Attempt {
                account: account.map(str::to_owned),
                mode,
                deadline,
            },
        );
        Ok(AuthLease {
            transactions: self.clone(),
            id,
            account: account.map(str::to_owned),
            mode,
            deadline,
            store,
            expected,
            legacy_snapshot,
        })
    }

    // Serialize local cancellation with publication, including the initially empty
    // alias. Store CAS still protects independently constructed providers/processes.
    pub(crate) fn cancel_server<T>(
        &self,
        account: &str,
        action: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let mut attempts = self.0.lock().map_err(|_| internal())?;
        let result = action()?;
        attempts.active.retain(|_, entry| !affects(entry, account));
        Ok(result)
    }
}

fn affects(entry: &Attempt, account: &str) -> bool {
    entry.mode.persists_on_server()
        && entry
            .account
            .as_deref()
            .is_none_or(|owner| owner == account)
}

impl AuthLease {
    fn check(&self, attempts: &Attempts) -> Result<Vec<StoredAccountCredential>> {
        if !attempts.active.contains_key(&self.id) && Instant::now() < self.deadline {
            return Err(changed());
        }
        let rows = self
            .store
            .as_ref()
            .map(|store| store.load_platform(Platform::Soda))
            .transpose()?
            .unwrap_or_default();
        if let Some(snapshot) = self.legacy_snapshot {
            if fingerprint(&rows) != snapshot {
                return Err(changed());
            }
        } else if self.store.is_some()
            && rows
                .iter()
                .find(|row| Some(row.account.as_str()) == self.account.as_deref())
                != self.expected.as_ref()
        {
            return Err(changed());
        }
        // Capacity pruning removes expired metadata, but must not turn an expired
        // QR into a cancellation. Still check persisted sources before expiry.
        if Instant::now() >= self.deadline {
            return Err(expired());
        }
        Ok(rows)
    }

    pub(crate) fn ensure_current(&self) -> Result<()> {
        let attempts = self.transactions.0.lock().map_err(|_| internal())?;
        self.check(&attempts).map(|_| ())
    }

    pub(crate) fn bind(&mut self, account: &str) -> Result<()> {
        let mut attempts = self.transactions.0.lock().map_err(|_| internal())?;
        let rows = self.check(&attempts)?;
        if let Some(owner) = &self.account {
            if owner != account {
                return Err(TuneWeaveError::invalid_request(
                    "Soda QR account is fixed at creation",
                )
                .with_platform(Platform::Soda));
            }
        } else {
            self.expected = rows.into_iter().find(|row| row.account == account);
            self.legacy_snapshot = None;
            self.account = Some(account.to_owned());
            attempts
                .active
                .get_mut(&self.id)
                .ok_or_else(changed)?
                .account = self.account.clone();
        }
        Ok(())
    }

    pub(crate) fn shorten_deadline(&mut self, deadline: Instant) -> Result<()> {
        let mut attempts = self.transactions.0.lock().map_err(|_| internal())?;
        self.check(&attempts)?;
        self.deadline = self.deadline.min(deadline);
        attempts
            .active
            .get_mut(&self.id)
            .ok_or_else(changed)?
            .deadline = self.deadline;
        if Instant::now() >= self.deadline {
            return Err(expired());
        }
        Ok(())
    }

    pub(crate) async fn wait<T>(&self, operation: impl Future<Output = Result<T>>) -> Result<T> {
        self.ensure_current()?;
        let result = tokio::time::timeout_at(self.deadline.into(), operation).await;
        // Conflict takes precedence over both successful and failed late responses.
        self.ensure_current()?;
        result.map_err(|_| expired())?
    }

    pub(crate) fn publish(&self, replacement: &StoredAccountCredential) -> Result<()> {
        let mut attempts = self.transactions.0.lock().map_err(|_| internal())?;
        self.check(&attempts)?;
        if self.account.as_deref() != Some(replacement.account.as_str()) {
            return Err(internal());
        }
        if let Some(store) = &self.store {
            let saved = match &self.expected {
                Some(expected) => store.compare_exchange(expected, Some(replacement))?,
                None => store.insert_if_absent(replacement)?,
            };
            if !saved {
                return Err(changed());
            }
            attempts
                .active
                .retain(|_, entry| !affects(entry, &replacement.account));
        } else {
            attempts.active.remove(&self.id);
        }
        Ok(())
    }
}

impl Drop for AuthLease {
    fn drop(&mut self) {
        if let Ok(mut attempts) = self.transactions.0.lock() {
            attempts.active.remove(&self.id);
        }
    }
}

// Legacy SDK QR creation has no account argument. Until the first poll binds an
// alias, any server credential change conservatively invalidates that transaction.
// Store only a digest, never another retained copy of every account's secrets.
fn fingerprint(rows: &[StoredAccountCredential]) -> [u8; 32] {
    let mut sorted: Vec<_> = rows.iter().collect();
    sorted
        .sort_by(|a, b| (&a.account, &a.kind, a.secret()).cmp(&(&b.account, &b.kind, b.secret())));
    let mut hash = ring::digest::Context::new(&ring::digest::SHA256);
    for row in sorted {
        for value in [&row.account, &row.kind, row.secret()] {
            hash.update(&(value.len() as u64).to_be_bytes());
            hash.update(value.as_bytes());
        }
    }
    let mut result = [0; 32];
    result.copy_from_slice(hash.finish().as_ref());
    result
}

fn internal() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        "Soda authentication storage is unavailable",
    )
    .with_platform(Platform::Soda)
}
fn changed() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::Conflict,
        "Soda authentication source changed or the transaction was cancelled",
    )
    .with_platform(Platform::Soda)
}
fn expired() -> TuneWeaveError {
    TuneWeaveError::invalid_request("Soda authentication transaction expired")
        .with_platform(Platform::Soda)
}
