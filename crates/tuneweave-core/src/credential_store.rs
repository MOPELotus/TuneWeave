use std::{
    fmt, fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    process,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{ErrorCode, Platform, Result, TuneWeaveError};

const CREDENTIAL_FILE_VERSION: u32 = 1;
static CREDENTIAL_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

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
}

/// A compact, generation-based file store rooted below TuneWeave's private data directory.
///
/// Secrets are intentionally excluded from Debug and errors. Files are published by an atomic
/// same-directory rename; Unix files/directories are created with `0600`/`0700` permissions.
/// Windows inherits the ACL of the selected private data directory.
#[derive(Clone, Debug)]
pub struct FileAccountCredentialStore {
    root: PathBuf,
}

impl FileAccountCredentialStore {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
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

impl AccountCredentialStore for FileAccountCredentialStore {
    fn load_platform(&self, platform: Platform) -> Result<Vec<StoredAccountCredential>> {
        let platform_dir = self.platform_dir(platform);
        let entries = match fs::read_dir(&platform_dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(store_io_error("read platform credentials", error)),
        };
        let mut credentials = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| store_io_error("read credential entry", error))?;
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
    }

    fn put(&self, credential: &StoredAccountCredential) -> Result<()> {
        credential.validate()?;
        let account_dir = self.account_dir(credential.platform, &credential.account);
        create_private_dir_all(&account_dir)?;
        let generation = credential_generation()?;
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
    }

    fn remove(&self, platform: Platform, account: &str) -> Result<bool> {
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
            let entry = entry.map_err(|error| store_io_error("read credential entry", error))?;
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
    }
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

fn credential_generation() -> Result<String> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "system clock is before the Unix epoch",
            )
        })?
        .as_nanos();
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
            && is_published_credential(&path)
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
mod tests {
    use super::*;

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
}
