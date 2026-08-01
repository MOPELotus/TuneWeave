use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};

const DEVICE_SCHEMA_VERSION: u8 = 2;
const DFID_LENGTH: usize = 24;

#[derive(Clone, Deserialize, Serialize)]
pub(crate) struct KugouDevice {
    schema_version: u8,
    guid: String,
    mid: String,
    dfid: Option<String>,
    registered_at: Option<u64>,
}

impl std::fmt::Debug for KugouDevice {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KugouDevice")
            .field("schema_version", &self.schema_version)
            .field("registered", &self.dfid.is_some())
            .field("registered_at", &self.registered_at)
            .finish()
    }
}

impl Default for KugouDevice {
    fn default() -> Self {
        let guid = generate_guid();
        let mid = derive_mid(&guid);
        Self {
            schema_version: DEVICE_SCHEMA_VERSION,
            guid,
            mid,
            dfid: None,
            registered_at: None,
        }
    }
}

impl KugouDevice {
    fn normalize(mut self) -> Result<Self> {
        if self.schema_version != DEVICE_SCHEMA_VERSION {
            return Err(device_data_error(
                "stored KuGou device state uses an unsupported schema",
            ));
        }
        if !valid_guid(&self.guid) {
            return Err(device_data_error(
                "stored KuGou device state contains an invalid GUID",
            ));
        }
        if self.mid != derive_mid(&self.guid)
            || self.mid.is_empty()
            || !self.mid.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(device_data_error(
                "stored KuGou device state contains an invalid MID",
            ));
        }
        if self.dfid.as_deref().is_some_and(|dfid| !valid_dfid(dfid)) {
            self.dfid = None;
            self.registered_at = None;
        }
        if self.dfid.is_none() {
            self.registered_at = None;
        }
        Ok(self)
    }

    pub(crate) fn identity(&self) -> KugouDeviceIdentity {
        KugouDeviceIdentity {
            guid: self.guid.clone(),
            mid: self.mid.clone(),
            dfid: self.dfid.clone(),
        }
    }

    pub(crate) const fn requires_registration(&self) -> bool {
        self.dfid.is_none()
    }

    fn register(&mut self, dfid: String, registered_at: u64) -> Result<()> {
        if !valid_dfid(&dfid) {
            return Err(device_data_error(
                "KuGou device registration returned an invalid dfid",
            ));
        }
        self.dfid = Some(dfid);
        self.registered_at = Some(registered_at);
        Ok(())
    }
}

#[derive(Clone)]
pub(crate) struct KugouDeviceIdentity {
    pub guid: String,
    pub mid: String,
    pub dfid: Option<String>,
}

impl std::fmt::Debug for KugouDeviceIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KugouDeviceIdentity")
            .field("registered", &self.dfid.is_some())
            .finish()
    }
}

impl KugouDeviceIdentity {
    pub(crate) fn dfid(&self) -> &str {
        self.dfid.as_deref().unwrap_or("-")
    }
}

pub(crate) struct DeviceStore {
    path: Option<PathBuf>,
    device: KugouDevice,
}

impl DeviceStore {
    pub(crate) fn open(path: Option<PathBuf>) -> Result<Self> {
        if let Some(path) = path.as_deref() {
            recover_interrupted_publish(path)?;
        }
        let device = path
            .as_deref()
            .filter(|path| path.exists())
            .map(read_device)
            .transpose()?
            .unwrap_or_default()
            .normalize()?;
        let store = Self { path, device };
        if store.path.as_deref().is_some_and(|path| !path.exists()) {
            store.save()?;
        }
        Ok(store)
    }

    pub(crate) const fn device(&self) -> &KugouDevice {
        &self.device
    }

    pub(crate) fn register(&mut self, dfid: String, registered_at: u64) -> Result<()> {
        let previous_dfid = self.device.dfid.clone();
        let previous_registered_at = self.device.registered_at;
        self.device.register(dfid, registered_at)?;
        if let Err(error) = self.save() {
            self.device.dfid = previous_dfid;
            self.device.registered_at = previous_registered_at;
            return Err(error);
        }
        Ok(())
    }

    pub(crate) fn rotate(&mut self) -> Result<KugouDeviceIdentity> {
        let previous = self.device.clone();
        self.device = KugouDevice::default();
        if let Err(error) = self.save() {
            self.device = previous;
            return Err(error);
        }
        Ok(self.device.identity())
    }

    pub(crate) fn save(&self) -> Result<()> {
        let Some(path) = self.path.as_deref() else {
            return Ok(());
        };
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty());
        if let Some(parent) = parent {
            create_private_dir(parent).map_err(|error| device_io_error("create", error))?;
        }
        let encoded = serde_json::to_vec(&self.device).map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "failed to encode KuGou device state",
            )
            .with_platform(Platform::Kugou)
        })?;
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
}

fn derive_mid(guid: &str) -> String {
    u128::from_be_bytes(Md5::digest(guid.as_bytes()).into()).to_string()
}

fn generate_guid() -> String {
    let mut bytes = rand::random::<[u8; 16]>();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let value = hex::encode(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &value[..8],
        &value[8..12],
        &value[12..16],
        &value[16..20],
        &value[20..]
    )
}

fn valid_dfid(value: &str) -> bool {
    value.len() == DFID_LENGTH && value.bytes().all(|byte| byte.is_ascii_alphanumeric())
}

fn valid_guid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            14 => byte == b'4',
            19 => matches!(byte, b'8' | b'9' | b'a' | b'b'),
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte),
        })
}

#[cfg(unix)]
fn create_private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let already_exists = path.exists();
    fs::create_dir_all(path)?;
    if already_exists {
        Ok(())
    } else {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
    }
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

fn read_device(path: &Path) -> Result<KugouDevice> {
    let bytes = fs::read(path).map_err(|error| device_io_error("read", error))?;
    serde_json::from_slice(&bytes).map_err(|_| {
        TuneWeaveError::new(
            ErrorCode::InternalError,
            "failed to decode KuGou device state",
        )
        .with_platform(Platform::Kugou)
    })
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
        (false, true) => {
            fs::rename(&backup, path).map_err(|error| device_io_error("recover", error))
        }
        (true, true) => {
            fs::remove_file(&backup).map_err(|error| device_io_error("remove backup", error))
        }
        _ => Ok(()),
    }
}

#[cfg(windows)]
fn publish_file(temporary: &Path, path: &Path) -> Result<()> {
    let backup = path.with_extension("backup");
    if backup.exists() {
        fs::remove_file(&backup).map_err(|error| device_io_error("remove backup", error))?;
    }
    if path.exists() {
        fs::rename(path, &backup).map_err(|error| device_io_error("prepare publish", error))?;
    }
    match fs::rename(temporary, path) {
        Ok(()) => {
            if backup.exists() {
                fs::remove_file(&backup)
                    .map_err(|error| device_io_error("remove backup", error))?;
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

fn device_data_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::InternalError, message).with_platform(Platform::Kugou)
}

fn device_io_error(action: &str, error: std::io::Error) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        format!("failed to {action} KuGou device state: {error}"),
    )
    .with_platform(Platform::Kugou)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "tuneweave-kugou-{label}-{}-{}.json",
            std::process::id(),
            rand::random::<u64>()
        ))
    }

    #[test]
    fn device_identity_persists_guid_mid_and_registration_atomically() {
        let path = temporary_path("persist");
        let mut store = DeviceStore::open(Some(path.clone())).expect("create store");
        let before = store.device().identity();
        assert!(valid_guid(&before.guid));
        assert_eq!(before.mid, derive_mid(&before.guid));
        assert!(before.dfid.is_none());

        store
            .register("AbCdEf0123456789GhIjKlMn".to_owned(), 1_700_000_000)
            .expect("save registration");
        let reopened = DeviceStore::open(Some(path.clone())).expect("reopen store");
        let after = reopened.device().identity();
        assert_eq!(after.guid, before.guid);
        assert_eq!(after.mid, before.mid);
        assert_eq!(after.dfid.as_deref(), Some("AbCdEf0123456789GhIjKlMn"));

        let _ = fs::remove_file(path);
    }

    #[test]
    fn invalid_cached_dfid_is_discarded_without_rotating_the_stable_device() {
        let path = temporary_path("invalid-dfid");
        let store = DeviceStore::open(Some(path.clone())).expect("create store");
        let identity = store.device().identity();
        let malformed = serde_json::json!({
            "schema_version": DEVICE_SCHEMA_VERSION,
            "guid": identity.guid,
            "mid": identity.mid,
            "dfid": "invalid",
            "registered_at": 1_700_000_000_u64,
        });
        fs::write(
            &path,
            serde_json::to_vec(&malformed).expect("encode malformed state"),
        )
        .expect("write malformed state");

        let reopened = DeviceStore::open(Some(path.clone())).expect("reopen store");
        let recovered = reopened.device().identity();
        assert_eq!(recovered.guid, malformed["guid"]);
        assert_eq!(recovered.mid, malformed["mid"]);
        assert!(reopened.device().requires_registration());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn invalid_guid_or_derived_mid_is_rejected_instead_of_silently_rotated() {
        let path = temporary_path("invalid-mid");
        let device = KugouDevice::default();
        let compact_guid = device.guid.replace('-', "");
        let legacy = serde_json::json!({
            "schema_version": DEVICE_SCHEMA_VERSION,
            "guid": compact_guid,
            "mid": derive_mid(&compact_guid),
            "dfid": null,
            "registered_at": null,
        });
        fs::write(
            &path,
            serde_json::to_vec(&legacy).expect("encode legacy state"),
        )
        .expect("write legacy state");
        assert!(DeviceStore::open(Some(path.clone())).is_err());

        let malformed = serde_json::json!({
            "schema_version": DEVICE_SCHEMA_VERSION,
            "guid": device.guid,
            "mid": "1",
            "dfid": null,
            "registered_at": null,
        });
        fs::write(
            &path,
            serde_json::to_vec(&malformed).expect("encode malformed state"),
        )
        .expect("write malformed state");
        assert!(DeviceStore::open(Some(path.clone())).is_err());

        let _ = fs::remove_file(path);
    }

    #[test]
    fn explicit_rotation_replaces_the_complete_anonymous_identity() {
        let path = temporary_path("rotate");
        let mut store = DeviceStore::open(Some(path.clone())).expect("create store");
        store
            .register("AbCdEf0123456789GhIjKlMn".to_owned(), 1_700_000_000)
            .expect("save registration");
        let before = store.device().identity();
        let rotated = store.rotate().expect("rotate identity");
        assert_ne!(rotated.guid, before.guid);
        assert_ne!(rotated.mid, before.mid);
        assert!(rotated.dfid.is_none());

        let reopened = DeviceStore::open(Some(path.clone())).expect("reopen store");
        let persisted = reopened.device().identity();
        assert_eq!(persisted.guid, rotated.guid);
        assert_eq!(persisted.mid, rotated.mid);
        assert!(persisted.dfid.is_none());

        let _ = fs::remove_file(path);
    }
}
