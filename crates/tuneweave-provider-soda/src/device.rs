use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};

const DEVICE_SCHEMA_VERSION: u32 = 1;
const MAX_DEVICE_STATE_BYTES: u64 = 64 * 1024;
const MIN_PLATFORM_ID: u64 = 1_000_000_000_000_000_000;
const MAX_PLATFORM_ID: u64 = 9_999_999_999_999_999_999;

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SodaDeviceState {
    pub(crate) schema_version: u32,
    pub(crate) device_id: String,
    pub(crate) install_id: String,
    pub(crate) created_at_ms: u64,
}

impl SodaDeviceState {
    fn generate() -> Result<Self> {
        let device_id = random_platform_id();
        let mut install_id = random_platform_id();
        while install_id == device_id {
            install_id = random_platform_id();
        }
        Ok(Self {
            schema_version: DEVICE_SCHEMA_VERSION,
            device_id,
            install_id,
            created_at_ms: unix_millis_now()?,
        })
    }

    fn validate(self) -> Result<Self> {
        if self.schema_version != DEVICE_SCHEMA_VERSION {
            return Err(device_state_error(
                "stored state uses an unsupported schema version",
            ));
        }
        validate_platform_id("device_id", &self.device_id)?;
        validate_platform_id("install_id", &self.install_id)?;
        if self.device_id == self.install_id {
            return Err(device_state_error(
                "stored device and install identities must be distinct",
            ));
        }
        if self.created_at_ms == 0 {
            return Err(device_state_error(
                "stored state does not contain a creation time",
            ));
        }
        Ok(self)
    }
}

pub(crate) struct SodaDeviceStore {
    path: Option<PathBuf>,
    state: Mutex<Option<SodaDeviceState>>,
}

impl std::fmt::Debug for SodaDeviceStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SodaDeviceStore")
            .field("persistent", &self.path.is_some())
            .finish_non_exhaustive()
    }
}

impl SodaDeviceStore {
    pub(crate) const fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            state: Mutex::new(None),
        }
    }

    pub(crate) fn initialize(&self) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| device_state_error("in-memory device state lock is poisoned"))?;
        if state.is_some() {
            return Ok(());
        }

        let loaded = match self.path.as_deref() {
            Some(path) => {
                recover_interrupted_publish(path)?;
                if path.exists() {
                    read_device(path)?
                } else {
                    let generated = SodaDeviceState::generate()?;
                    save_device(path, &generated)?;
                    generated
                }
            }
            None => SodaDeviceState::generate()?,
        };
        *state = Some(loaded);
        Ok(())
    }

    pub(crate) fn snapshot(&self) -> Result<SodaDeviceState> {
        self.initialize()?;
        self.state
            .lock()
            .map_err(|_| device_state_error("in-memory device state lock is poisoned"))?
            .clone()
            .ok_or_else(|| device_state_error("in-memory device state was not initialized"))
    }
}

fn read_device(path: &Path) -> Result<SodaDeviceState> {
    let metadata = fs::metadata(path).map_err(|error| device_io_error("inspect", error))?;
    if !metadata.is_file() || metadata.len() > MAX_DEVICE_STATE_BYTES {
        return Err(device_state_error(
            "stored state is not a regular bounded file",
        ));
    }
    let bytes = fs::read(path).map_err(|error| device_io_error("read", error))?;
    serde_json::from_slice::<SodaDeviceState>(&bytes)
        .map_err(|_| device_state_error("stored state is not valid JSON"))?
        .validate()
}

fn save_device(path: &Path, state: &SodaDeviceState) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        create_private_dir(parent).map_err(|error| device_io_error("create directory", error))?;
    }
    let encoded = serde_json::to_vec(state)
        .map_err(|_| device_state_error("generated state could not be encoded"))?;
    let temporary = path.with_extension(format!(
        "tmp-{}-{}",
        std::process::id(),
        rand::random::<u64>()
    ));
    let mut file =
        create_private_file(&temporary).map_err(|error| device_io_error("write", error))?;
    if let Err(error) = file.write_all(&encoded).and_then(|()| file.sync_all()) {
        let _ = fs::remove_file(&temporary);
        return Err(device_io_error("write", error));
    }
    if let Err(error) = publish_file(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

fn validate_platform_id(field: &str, value: &str) -> Result<()> {
    if value.len() != 19
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || value.starts_with('0')
    {
        return Err(device_state_error(&format!(
            "stored {field} is not a canonical 19-digit identity"
        )));
    }
    Ok(())
}

fn random_platform_id() -> String {
    rand::random_range(MIN_PLATFORM_ID..=MAX_PLATFORM_ID).to_string()
}

fn unix_millis_now() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .map_err(|_| device_state_error("system clock is before the Unix epoch"))
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    fs::create_dir_all(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)
}

#[cfg(unix)]
fn create_private_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;

    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn create_private_file(path: &Path) -> std::io::Result<std::fs::File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(not(windows))]
fn publish_file(temporary: &Path, path: &Path) -> Result<()> {
    fs::rename(temporary, path).map_err(|error| device_io_error("publish", error))
}

#[cfg(not(windows))]
fn recover_interrupted_publish(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(windows)]
fn recover_interrupted_publish(path: &Path) -> Result<()> {
    let backup = path.with_extension("backup");
    match (path.exists(), backup.exists()) {
        (false, true) => fs::rename(&backup, path)
            .map_err(|error| device_io_error("recover interrupted publish", error)),
        (true, true) => {
            fs::remove_file(&backup).map_err(|error| device_io_error("remove stale backup", error))
        }
        _ => Ok(()),
    }
}

#[cfg(windows)]
fn publish_file(temporary: &Path, path: &Path) -> Result<()> {
    let backup = path.with_extension("backup");
    if backup.exists() {
        fs::remove_file(&backup).map_err(|error| device_io_error("remove stale backup", error))?;
    }
    if path.exists() {
        fs::rename(path, &backup)
            .map_err(|error| device_io_error("prepare atomic publish", error))?;
    }
    match fs::rename(temporary, path) {
        Ok(()) => {
            if backup.exists() {
                fs::remove_file(&backup)
                    .map_err(|error| device_io_error("remove publish backup", error))?;
            }
            Ok(())
        }
        Err(error) => {
            if backup.exists() {
                let _ = fs::rename(&backup, path);
            }
            Err(device_io_error("publish", error))
        }
    }
}

fn device_io_error(action: &str, error: std::io::Error) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        format!("failed to {action} Soda device state: {error}"),
    )
    .with_platform(Platform::Soda)
}

fn device_state_error(reason: &str) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        format!("Soda device state {reason}"),
    )
    .with_platform(Platform::Soda)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tuneweave-soda-{label}-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ))
    }

    #[test]
    fn generated_identity_uses_distinct_canonical_ids_and_redacted_debug() {
        let store = SodaDeviceStore::new(None);
        let identity = store.snapshot().expect("generate device identity");
        validate_platform_id("device_id", &identity.device_id).expect("valid device id");
        validate_platform_id("install_id", &identity.install_id).expect("valid install id");
        assert_ne!(identity.device_id, identity.install_id);
        let debug = format!("{store:?}");
        assert_eq!(debug, "SodaDeviceStore { persistent: false, .. }");
        assert!(!debug.contains(&identity.device_id));
        assert!(!debug.contains(&identity.install_id));
    }

    #[test]
    fn persistent_identity_is_lazy_and_reused_after_restart() {
        let root = test_root("device-persistence");
        let path = root.join("soda-device.json");
        let first_store = SodaDeviceStore::new(Some(path.clone()));
        assert!(!path.exists());
        let first = first_store.snapshot().expect("create persistent identity");
        assert!(path.exists());
        let restored = SodaDeviceStore::new(Some(path.clone()))
            .snapshot()
            .expect("restore persistent identity");
        assert_eq!(first.device_id, restored.device_id);
        assert_eq!(first.install_id, restored.install_id);
        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[test]
    fn malformed_persisted_identity_is_rejected_without_silent_rotation() {
        let root = test_root("invalid-device");
        fs::create_dir_all(&root).expect("create test directory");
        let path = root.join("soda-device.json");
        fs::write(
            &path,
            br#"{"schema_version":1,"device_id":"123","install_id":"456","created_at_ms":1}"#,
        )
        .expect("write invalid identity");
        let error = SodaDeviceStore::new(Some(path))
            .initialize()
            .expect_err("reject malformed identity");
        assert_eq!(error.code, ErrorCode::InternalError);
        fs::remove_dir_all(root).expect("remove test directory");
    }

    #[cfg(windows)]
    #[test]
    fn interrupted_windows_publish_recovers_the_previous_identity() {
        let root = test_root("device-recovery");
        fs::create_dir_all(&root).expect("create test directory");
        let path = root.join("soda-device.json");
        let backup = path.with_extension("backup");
        let expected = SodaDeviceState::generate().expect("generate expected state");
        fs::write(
            &backup,
            serde_json::to_vec(&expected).expect("encode expected state"),
        )
        .expect("write interrupted backup");

        let identity = SodaDeviceStore::new(Some(path.clone()))
            .snapshot()
            .expect("recover identity");
        assert_eq!(identity.device_id, expected.device_id);
        assert!(path.exists());
        assert!(!backup.exists());
        fs::remove_dir_all(root).expect("remove test directory");
    }
}
