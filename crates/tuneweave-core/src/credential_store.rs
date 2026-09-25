use std::{
    fmt, fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    process,
    sync::{
        Mutex, MutexGuard,
        atomic::{AtomicU64, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{ErrorCode, Platform, Result, TuneWeaveError};

const CREDENTIAL_FILE_VERSION: u32 = 1;
static CREDENTIAL_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);
// Coordinate independently opened stores as well as clones within this server process.
// Network requests must complete before entering a store operation.
static CREDENTIAL_STORE_LOCK: Mutex<()> = Mutex::new(());

/// A provider-owned secret associated with one stable platform/account alias pair.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct StoredAccountCredential {
    pub platform: Platform,
    pub account: String,
    pub kind: String,
    secret: String,
}

impl StoredAccountCredential {
    pub fn new(
        platform: Platform,
        account: impl Into<String>,
        kind: impl Into<String>,
        secret: impl Into<String>,
    ) -> Result<Self> {
        let credential = Self {
            platform,
            account: account.into(),
            kind: kind.into(),
            secret: secret.into(),
        };
        credential.validate()?;
        Ok(credential)
    }

    #[must_use]
    pub fn secret(&self) -> &str {
        &self.secret
    }

    #[must_use]
    pub fn into_secret(self) -> String {
        self.secret
    }

    fn validate(&self) -> Result<()> {
        let account = self.account.trim();
        if account.is_empty() {
            return Err(TuneWeaveError::invalid_request(
                "account credential alias cannot be empty",
            ));
        }
        if account.len() > 64 {
            return Err(TuneWeaveError::invalid_request(
                "account credential alias cannot exceed 64 bytes",
            ));
        }
        if account != self.account {
            return Err(TuneWeaveError::invalid_request(
                "account credential alias cannot contain surrounding whitespace",
            ));
        }
        if self.kind.trim().is_empty() {
            return Err(TuneWeaveError::invalid_request(
                "account credential kind cannot be empty",
            ));
        }
        if self.secret.is_empty() {
            return Err(TuneWeaveError::invalid_request(
                "account credential secret cannot be empty",
            ));
        }
        Ok(())
    }
}

impl fmt::Debug for StoredAccountCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("StoredAccountCredential")
            .field("platform", &self.platform)
            .field("account", &self.account)
            .field("kind", &self.kind)
            .field("has_secret", &true)
            .finish()
    }
}

/// Persistent secret storage shared by every platform provider.
pub trait AccountCredentialStore: Send + Sync {
    fn load_platform(&self, platform: Platform) -> Result<Vec<StoredAccountCredential>>;
    fn put(&self, credential: &StoredAccountCredential) -> Result<()>;
    fn remove(&self, platform: Platform, account: &str) -> Result<bool>;

    /// Publish a first login only while its platform/account alias is still absent.
    /// The absence check and publication must be atomic with every other store mutation.
    fn insert_if_absent(&self, _credential: &StoredAccountCredential) -> Result<bool> {
        Err(TuneWeaveError::new(
            ErrorCode::CapabilityNotSupported,
            "account storage does not support conditional first credential writes",
        ))
    }

    /// Replace or remove an unchanged credential. Returns false after logout, replacement,
    /// or another refresh. Implementations must perform comparison and mutation atomically.
    /// Providers should include a fresh session generation in each new login's secret.
    fn compare_exchange(
        &self,
        _expected: &StoredAccountCredential,
        _replacement: Option<&StoredAccountCredential>,
    ) -> Result<bool> {
        Err(TuneWeaveError::new(
            ErrorCode::CapabilityNotSupported,
            "account storage does not support conditional credential updates",
        ))
    }
}

/// A compact, generation-based file store rooted below TuneWeave's private data directory.
/// Reads and mutations share an advisory OS lock across cooperating processes on the
/// same filesystem. All writers must use this store's locking protocol; the reserved
/// `.credential-store.lock` file must not be removed while a store is in use.
///
/// Secrets are intentionally excluded from Debug and errors. Files are published by an atomic
/// same-directory rename; Unix files/directories are created with `0600`/`0700` permissions.
/// Windows inherits the ACL of the selected private data directory.
#[derive(Clone, Debug)]
pub struct FileAccountCredentialStore {
    root: PathBuf,
}

#[derive(Clone, Copy)]
enum CredentialStoreOperation {
    LoadPlatform,
    Put,
    Remove,
}

impl CredentialStoreOperation {
    const fn name(self) -> &'static str {
        match self {
            Self::LoadPlatform => "load_platform",
            Self::Put => "put",
            Self::Remove => "remove",
        }
    }
}

fn log_credential_store_failure(
    operation: CredentialStoreOperation,
    platform: Platform,
    error: &TuneWeaveError,
) {
    tracing::error!(
        event = "credential_store_failure",
        operation = operation.name(),
        platform = %platform,
        error_code = error.code.as_str(),
        retryable = error.retryable,
        "TuneWeave account credential persistence failed"
    );
}

impl FileAccountCredentialStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let store = Self::new(root);
        create_private_dir_all(&store.root)?;
        for platform in Platform::ALL {
            store.load_platform(platform)?;
        }
        Ok(store)
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn platform_dir(&self, platform: Platform) -> PathBuf {
        self.root.join(platform.as_str())
    }

    fn account_dir(&self, platform: Platform, account: &str) -> PathBuf {
        self.platform_dir(platform)
            .join(hex::encode(account.as_bytes()))
    }
}

impl FileAccountCredentialStore {
    fn load_platform_unlocked(&self, platform: Platform) -> Result<Vec<StoredAccountCredential>> {
        let result = (|| {
            let platform_dir = self.platform_dir(platform);
            let entries = match fs::read_dir(&platform_dir) {
                Ok(entries) => entries,
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
                Err(error) => return Err(store_io_error("read platform credentials", error)),
            };
            let mut credentials = Vec::new();
            for entry in entries {
                let entry =
                    entry.map_err(|error| store_io_error("read credential entry", error))?;
                let file_type = entry
                    .file_type()
                    .map_err(|error| store_io_error("inspect credential entry", error))?;
                if !file_type.is_dir() || file_type.is_symlink() {
                    continue;
                }
                if let Some(credential) = load_latest_credential(&entry.path(), platform)? {
                    credentials.push(credential);
                }
            }
            credentials.sort_by(|left, right| left.account.cmp(&right.account));
            Ok(credentials)
        })();
        if let Err(error) = &result {
            log_credential_store_failure(CredentialStoreOperation::LoadPlatform, platform, error);
        }
        result
    }

    fn put_unlocked(&self, credential: &StoredAccountCredential) -> Result<()> {
        let result = (|| {
            credential.validate()?;
            let account_dir = self.account_dir(credential.platform, &credential.account);
            create_private_dir_all(&account_dir)?;
            let generation = credential_generation(&account_dir)?;
            let temporary_path = account_dir.join(format!("{generation}.tmp"));
            let final_path = account_dir.join(format!("{generation}.json"));
            let file = CredentialFile {
                version: CREDENTIAL_FILE_VERSION,
                credential: credential.clone(),
            };
            let encoded = serde_json::to_vec(&file).map_err(|error| {
                TuneWeaveError::new(
                    ErrorCode::InternalError,
                    format!("failed to serialize account credential: {error}"),
                )
            })?;
            if let Err(error) = write_private_file(&temporary_path, &encoded) {
                let _ = fs::remove_file(&temporary_path);
                return Err(error);
            }
            if let Err(error) = fs::rename(&temporary_path, &final_path) {
                let _ = fs::remove_file(&temporary_path);
                return Err(store_io_error("publish account credential", error));
            }
            remove_old_generations(&account_dir, &final_path)?;
            Ok(())
        })();
        if let Err(error) = &result {
            log_credential_store_failure(CredentialStoreOperation::Put, credential.platform, error);
        }
        result
    }

    fn remove_unlocked(&self, platform: Platform, account: &str) -> Result<bool> {
        let result = (|| {
            let account = account.trim();
            if account.is_empty() || account.len() > 64 {
                return Err(TuneWeaveError::invalid_request(
                    "stored account alias must contain at most 64 bytes",
                ));
            }
            let account_dir = self.account_dir(platform, account);
            let entries = match fs::read_dir(&account_dir) {
                Ok(entries) => entries,
                Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(store_io_error("read account credentials", error)),
            };
            for entry in entries {
                let entry =
                    entry.map_err(|error| store_io_error("read credential entry", error))?;
                let file_type = entry
                    .file_type()
                    .map_err(|error| store_io_error("inspect credential entry", error))?;
                if file_type.is_file() && is_credential_generation(&entry.path()) {
                    fs::remove_file(entry.path())
                        .map_err(|error| store_io_error("remove account credential", error))?;
                }
            }
            fs::remove_dir(&account_dir)
                .map_err(|error| store_io_error("remove account credential directory", error))?;
            Ok(true)
        })();
        if let Err(error) = &result {
            log_credential_store_failure(CredentialStoreOperation::Remove, platform, error);
        }
        result
    }
}

impl AccountCredentialStore for FileAccountCredentialStore {
    fn load_platform(&self, platform: Platform) -> Result<Vec<StoredAccountCredential>> {
        let _guard = self.lock()?;
        self.load_platform_unlocked(platform)
    }

    fn put(&self, credential: &StoredAccountCredential) -> Result<()> {
        let _guard = self.lock()?;
        self.put_unlocked(credential)
    }

    fn remove(&self, platform: Platform, account: &str) -> Result<bool> {
        let _guard = self.lock()?;
        self.remove_unlocked(platform, account)
    }

    fn insert_if_absent(&self, credential: &StoredAccountCredential) -> Result<bool> {
        credential.validate()?;
        let _guard = self.lock()?;
        let current = self.load_platform_unlocked(credential.platform)?;
        if current
            .iter()
            .any(|value| value.account == credential.account)
        {
            return Ok(false);
        }
        self.put_unlocked(credential)?;
        Ok(true)
    }

    fn compare_exchange(
        &self,
        expected: &StoredAccountCredential,
        replacement: Option<&StoredAccountCredential>,
    ) -> Result<bool> {
        expected.validate()?;
        if let Some(replacement) = replacement {
            replacement.validate()?;
            if replacement.platform != expected.platform || replacement.account != expected.account
            {
                return Err(TuneWeaveError::invalid_request(
                    "conditional credential updates must preserve the platform and account",
                ));
            }
        }
        let _guard = self.lock()?;
        let current = self.load_platform_unlocked(expected.platform)?;
        if current
            .iter()
            .find(|entry| entry.account == expected.account)
            != Some(expected)
        {
            return Ok(false);
        }
        match replacement {
            Some(replacement) => self.put_unlocked(replacement)?,
            None => {
                self.remove_unlocked(expected.platform, &expected.account)?;
            }
        }
        Ok(true)
    }
}

// Keep the file first: closing it releases the OS lock before another local thread
// can enter. The fixed lock file must never be removed or replaced during normal operation.
struct CredentialStoreGuard {
    _file: fs::File,
    _local: MutexGuard<'static, ()>,
}
const LOCK_FILE: &str = ".credential-store.lock";
impl FileAccountCredentialStore {
    fn lock(&self) -> Result<CredentialStoreGuard> {
        let local = CREDENTIAL_STORE_LOCK.lock().map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "account credential storage lock is unavailable",
            )
        })?;
        create_private_dir_all(&self.root)?;
        let path = self.root.join(LOCK_FILE);
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = match options.open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == ErrorKind::AlreadyExists => {
                validate_lock_file(&path)?;
                fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&path)
                    .map_err(|e| store_io_error("open account credential lock", e))?
            }
            Err(e) => return Err(store_io_error("create account credential lock", e)),
        };
        let path_metadata = validate_lock_file(&path)?;
        let file_metadata = file
            .metadata()
            .map_err(|e| store_io_error("inspect account credential lock", e))?;
        if !file_metadata.is_file() || file_metadata.len() != 0 {
            return Err(invalid_lock_file());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            if path_metadata.dev() != file_metadata.dev()
                || path_metadata.ino() != file_metadata.ino()
                || file_metadata.nlink() != 1
            {
                return Err(invalid_lock_file());
            }
            file.set_permissions(fs::Permissions::from_mode(0o600))
                .map_err(|e| store_io_error("protect account credential lock", e))?;
        }
        #[cfg(not(unix))]
        let _ = path_metadata;
        // Fully qualify the extension method to honor the crate's Rust 1.85 MSRV.
        fs4::FileExt::lock(&file)
            .map_err(|e| store_io_error("lock account credential storage", e))?;
        Ok(CredentialStoreGuard {
            _file: file,
            _local: local,
        })
    }
}
fn invalid_lock_file() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        "account credential lock file is invalid",
    )
}
fn validate_lock_file(path: &Path) -> Result<fs::Metadata> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| store_io_error("inspect account credential lock", e))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != 0 {
        return Err(invalid_lock_file());
    }
    Ok(metadata)
}

#[derive(Serialize, Deserialize)]
struct CredentialFile {
    version: u32,
    credential: StoredAccountCredential,
}

fn load_latest_credential(
    account_dir: &Path,
    platform: Platform,
) -> Result<Option<StoredAccountCredential>> {
    let mut generations = Vec::new();
    for entry in fs::read_dir(account_dir)
        .map_err(|error| store_io_error("read account credential generations", error))?
    {
        let entry = entry.map_err(|error| store_io_error("read credential entry", error))?;
        let file_type = entry
            .file_type()
            .map_err(|error| store_io_error("inspect credential generation", error))?;
        if file_type.is_file() && is_published_credential(&entry.path()) {
            generations.push(entry);
        }
    }
    generations.sort_by_key(fs::DirEntry::file_name);
    let Some(latest) = generations.last() else {
        return Ok(None);
    };
    let encoded = fs::read(latest.path())
        .map_err(|error| store_io_error("read account credential", error))?;
    let file: CredentialFile = serde_json::from_slice(&encoded).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::InternalError,
            format!("failed to parse stored account credential: {error}"),
        )
    })?;
    if file.version != CREDENTIAL_FILE_VERSION {
        return Err(TuneWeaveError::new(
            ErrorCode::InternalError,
            format!("unsupported account credential version: {}", file.version),
        ));
    }
    file.credential.validate()?;
    if file.credential.platform != platform
        || hex::encode(file.credential.account.as_bytes())
            != account_dir
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
    {
        return Err(TuneWeaveError::new(
            ErrorCode::InternalError,
            "stored account credential identity does not match its directory",
        ));
    }
    Ok(Some(file.credential))
}

fn create_private_dir_all(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .map_err(|error| store_io_error("create account credential directory", error))?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| store_io_error("protect account credential directory", error))?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(path)
        .map_err(|error| store_io_error("create account credential directory", error))?;
    Ok(())
}

fn write_private_file(path: &Path, data: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| store_io_error("create account credential", error))?;
    file.write_all(data)
        .map_err(|error| store_io_error("write account credential", error))?;
    file.sync_all()
        .map_err(|error| store_io_error("sync account credential", error))?;
    Ok(())
}

fn credential_generation(account_dir: &Path) -> Result<String> {
    let mut nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "system clock is before the Unix epoch",
            )
        })?
        .as_nanos();
    // Publication must remain newer than every previous generation even when the
    // wall clock moves backwards. This also makes a crash before cleanup harmless.
    // The caller holds the store's process and OS locks throughout this operation.
    for entry in fs::read_dir(account_dir)
        .map_err(|error| store_io_error("read account credential generations", error))?
    {
        let entry = entry.map_err(|error| store_io_error("read credential entry", error))?;
        if !entry
            .file_type()
            .map_err(|error| store_io_error("inspect credential generation", error))?
            .is_file()
            || !is_credential_generation(&entry.path())
        {
            continue;
        }
        let invalid = || {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "stored credential generation name is invalid",
            )
        };
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(invalid)?;
        let stem = name.rsplit_once('.').ok_or_else(invalid)?.0;
        let bytes = stem.as_bytes();
        if bytes.len() != 67
            || bytes[39] != b'-'
            || bytes[50] != b'-'
            || !bytes[..39].iter().all(u8::is_ascii_digit)
            || !bytes[40..50].iter().all(u8::is_ascii_digit)
            || !bytes[51..].iter().all(u8::is_ascii_hexdigit)
        {
            return Err(invalid());
        }
        let previous: u128 = stem[..39].parse().map_err(|_| invalid())?;
        let next = previous.checked_add(1).ok_or_else(invalid)?;
        nanos = nanos.max(next);
    }
    let sequence = CREDENTIAL_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(format!("{nanos:039}-{:010}-{sequence:016x}", process::id()))
}

fn remove_old_generations(account_dir: &Path, keep: &Path) -> Result<()> {
    let keep_name = keep.file_name().ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::InternalError,
            "published account credential path has no filename",
        )
    })?;
    for entry in fs::read_dir(account_dir)
        .map_err(|error| store_io_error("read account credential generations", error))?
    {
        let entry = entry.map_err(|error| store_io_error("read credential entry", error))?;
        let path = entry.path();
        if path == keep {
            continue;
        }
        let file_type = entry
            .file_type()
            .map_err(|error| store_io_error("inspect credential generation", error))?;
        if file_type.is_file()
            && is_credential_generation(&path)
            && entry.file_name().as_os_str() < keep_name
        {
            fs::remove_file(path)
                .map_err(|error| store_io_error("remove stale account credential", error))?;
        }
    }
    Ok(())
}

fn is_credential_generation(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "json" || extension == "tmp")
}

fn is_published_credential(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == "json")
}

fn store_io_error(operation: &str, error: std::io::Error) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        format!("failed to {operation}: {error}"),
    )
    .with_details(json!({ "operation": operation }))
}

#[cfg(test)]
mod process_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conditional_updates_preserve_other_accounts_and_reject_stale_sources() {
        let directory = TestDirectory::new();
        let store = FileAccountCredentialStore::new(&directory.0);
        let original =
            StoredAccountCredential::new(Platform::Soda, "a", "session", "generation-1").unwrap();
        let rotated =
            StoredAccountCredential::new(Platform::Soda, "a", "session", "generation-2").unwrap();
        let other =
            StoredAccountCredential::new(Platform::Soda, "b", "session", "other-session").unwrap();
        store.put(&original).unwrap();
        store.put(&other).unwrap();
        assert!(store.compare_exchange(&original, Some(&other)).is_err());
        assert!(store.compare_exchange(&original, Some(&rotated)).unwrap());
        assert!(!store.compare_exchange(&original, None).unwrap());
        assert_eq!(
            store.load_platform(Platform::Soda).unwrap(),
            vec![rotated.clone(), other.clone()]
        );
        assert!(store.compare_exchange(&rotated, None).unwrap());
        assert!(!store.compare_exchange(&rotated, Some(&original)).unwrap());
        assert_eq!(store.load_platform(Platform::Soda).unwrap(), vec![other]);
    }

    #[test]
    fn simultaneous_refreshes_have_one_winner_across_store_instances() {
        let directory = TestDirectory::new();
        let original =
            StoredAccountCredential::new(Platform::Migu, "a", "session", "initial").unwrap();
        let store = FileAccountCredentialStore::new(&directory.0);
        store.put(&original).unwrap();
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|index| {
                let store = FileAccountCredentialStore::new(&directory.0);
                let original = original.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let next = StoredAccountCredential::new(
                        Platform::Migu,
                        "a",
                        "session",
                        format!("rotated-{index}"),
                    )
                    .unwrap();
                    barrier.wait();
                    store.compare_exchange(&original, Some(&next)).unwrap()
                })
            })
            .collect();
        let winners = workers
            .into_iter()
            .map(|worker| usize::from(worker.join().unwrap()))
            .sum::<usize>();
        assert_eq!(winners, 1);
    }

    #[test]
    fn credential_store_operation_names_are_stable() {
        assert_eq!(
            CredentialStoreOperation::LoadPlatform.name(),
            "load_platform"
        );
        assert_eq!(CredentialStoreOperation::Put.name(), "put");
        assert_eq!(CredentialStoreOperation::Remove.name(), "remove");
    }

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "tuneweave-credential-store-{}-{}",
                process::id(),
                CREDENTIAL_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).expect("create test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let temp = std::env::temp_dir();
            if self.0.starts_with(&temp) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }

    #[test]
    fn file_store_round_trips_updates_and_removes_multiple_platform_accounts() {
        let directory = TestDirectory::new();
        let store = FileAccountCredentialStore::new(&directory.0);
        let personal = StoredAccountCredential::new(
            Platform::Netease,
            "personal",
            "cookie",
            "MUSIC_U=first-secret",
        )
        .expect("personal credential");
        let premium = StoredAccountCredential::new(
            Platform::Netease,
            "premium/账号",
            "cookie",
            "MUSIC_U=second-secret",
        )
        .expect("premium credential");
        let qq = StoredAccountCredential::new(Platform::Qq, "personal", "cookie", "uin=qq-secret")
            .expect("QQ credential");
        store.put(&personal).expect("save personal credential");
        store.put(&premium).expect("save premium credential");
        store.put(&qq).expect("save QQ credential");

        let netease = store
            .load_platform(Platform::Netease)
            .expect("load NetEase credentials");
        assert_eq!(
            netease
                .iter()
                .map(|credential| credential.account.as_str())
                .collect::<Vec<_>>(),
            vec!["personal", "premium/账号"]
        );
        assert_eq!(netease[0].secret(), "MUSIC_U=first-secret");
        assert_eq!(
            store
                .load_platform(Platform::Qq)
                .expect("load QQ credentials")[0]
                .secret(),
            "uin=qq-secret"
        );

        let updated = StoredAccountCredential::new(
            Platform::Netease,
            "personal",
            "cookie",
            "MUSIC_U=refreshed-secret",
        )
        .expect("updated credential");
        store.put(&updated).expect("update credential");
        assert_eq!(
            fs::read_dir(store.account_dir(Platform::Netease, "personal"))
                .expect("read stored generations")
                .filter_map(|entry| entry.ok())
                .filter(|entry| is_published_credential(&entry.path()))
                .count(),
            1
        );
        assert_eq!(
            store
                .load_platform(Platform::Netease)
                .expect("reload credentials")[0]
                .secret(),
            "MUSIC_U=refreshed-secret"
        );
        assert!(
            store
                .remove(Platform::Netease, "personal")
                .expect("remove credential")
        );
        assert!(
            !store
                .remove(Platform::Netease, "personal")
                .expect("remove missing credential")
        );
        assert_eq!(
            store
                .load_platform(Platform::Netease)
                .expect("load remaining credentials")
                .len(),
            1
        );
    }

    #[test]
    fn credential_debug_and_errors_never_echo_the_secret() {
        let credential = StoredAccountCredential::new(
            Platform::Netease,
            "default",
            "cookie",
            "MUSIC_U=must-not-appear",
        )
        .expect("credential");
        let debug = format!("{credential:?}");
        assert!(debug.contains("has_secret: true"));
        assert!(!debug.contains("must-not-appear"));

        for invalid in [
            StoredAccountCredential::new(Platform::Netease, "", "cookie", "secret"),
            StoredAccountCredential::new(Platform::Netease, " personal ", "cookie", "secret"),
            StoredAccountCredential::new(Platform::Netease, "default", "", "secret"),
            StoredAccountCredential::new(Platform::Netease, "default", "cookie", ""),
        ] {
            assert_eq!(
                invalid.expect_err("invalid credential").code,
                ErrorCode::InvalidRequest
            );
        }
    }

    #[test]
    fn open_creates_the_private_root_and_rejects_corrupt_generations() {
        let directory = TestDirectory::new();
        let root = directory.0.join("accounts");
        let store = FileAccountCredentialStore::open(&root).expect("open empty store");
        assert_eq!(store.root(), root);
        assert!(root.is_dir());

        let account_dir = store.account_dir(Platform::Netease, "default");
        fs::create_dir_all(&account_dir).expect("create corrupt account directory");
        fs::write(account_dir.join("generation.json"), b"{not-json")
            .expect("write corrupt generation");
        let error = FileAccountCredentialStore::open(&root).expect_err("reject corrupt store");
        assert_eq!(error.code, ErrorCode::InternalError);
        assert!(!error.message.contains(&root.to_string_lossy().to_string()));
        assert!(!error.message.contains("{not-json"));
    }
}
