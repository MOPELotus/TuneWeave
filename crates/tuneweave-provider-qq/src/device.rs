use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};

#[derive(Clone, Deserialize, Serialize)]
#[serde(default)]
pub(crate) struct QqDevice {
    pub open_udid: String,
    pub android_id: String,
    pub imei: String,
    pub fingerprint: String,
    pub proc_version: String,
    pub qimei: Option<String>,
    pub qimei36: Option<String>,
    pub qimei_saved_at: Option<u64>,
    pub session_uid: Option<String>,
    pub session_sid: Option<String>,
    pub session_vkey: Option<String>,
    pub session_saved_at: Option<u64>,
}

impl Default for QqDevice {
    fn default() -> Self {
        Self {
            open_udid: random_hex(16),
            android_id: random_hex(8),
            imei: random_imei(),
            fingerprint: format!(
                "xiaomi/iarim/sagit:10/eomam.200122.001/{}:user/release-keys",
                rand::random_range(1_000_000_u32..=9_999_999)
            ),
            proc_version: format!(
                "Linux 5.4.0-54-generic-{} (android-build@google.com)",
                random_alphanumeric(8)
            ),
            qimei: None,
            qimei36: None,
            qimei_saved_at: None,
            session_uid: None,
            session_sid: None,
            session_vkey: None,
            session_saved_at: None,
        }
    }
}

impl QqDevice {
    pub(crate) fn has_current_qimei(&self, now: u64) -> bool {
        self.qimei.as_deref().is_some_and(|value| !value.is_empty())
            && self
                .qimei36
                .as_deref()
                .is_some_and(|value| !value.is_empty())
            && self
                .qimei_saved_at
                .is_some_and(|saved_at| now.saturating_sub(saved_at) < 86_400)
    }

    pub(crate) fn has_current_session(&self, now: u64) -> bool {
        self.session_uid
            .as_deref()
            .is_some_and(|value| !value.is_empty())
            && self
                .session_sid
                .as_deref()
                .is_some_and(|value| !value.is_empty())
            && self
                .session_saved_at
                .is_some_and(|saved_at| now.saturating_sub(saved_at) < 86_400)
    }
}

pub(crate) struct DeviceStore {
    path: Option<PathBuf>,
    device: QqDevice,
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
            .unwrap_or_default();
        let store = Self { path, device };
        if store.path.as_deref().is_some_and(|path| !path.exists()) {
            store.save()?;
        }
        Ok(store)
    }

    pub(crate) const fn device(&self) -> &QqDevice {
        &self.device
    }

    pub(crate) const fn device_mut(&mut self) -> &mut QqDevice {
        &mut self.device
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
        let encoded = serde_json::to_vec(&self.device).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                format!("failed to encode QQ device state: {error}"),
            )
            .with_platform(Platform::Qq)
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

fn read_device(path: &Path) -> Result<QqDevice> {
    let bytes = fs::read(path).map_err(|error| device_io_error("read", error))?;
    serde_json::from_slice(&bytes).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::InternalError,
            format!("failed to decode QQ device state: {error}"),
        )
        .with_platform(Platform::Qq)
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

fn device_io_error(action: &str, error: std::io::Error) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        format!("failed to {action} QQ device state: {error}"),
    )
    .with_platform(Platform::Qq)
}

pub(crate) fn unix_seconds_now() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                format!("system clock is before the Unix epoch: {error}"),
            )
            .with_platform(Platform::Qq)
        })
}

fn random_hex(bytes: usize) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    (0..bytes.saturating_mul(2))
        .map(|_| char::from(HEX[rand::random_range(0..HEX.len())]))
        .collect()
}

fn random_alphanumeric(length: usize) -> String {
    const CHARS: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    (0..length)
        .map(|_| char::from(CHARS[rand::random_range(0..CHARS.len())]))
        .collect()
}

fn random_imei() -> String {
    let mut digits = (0..14)
        .map(|_| rand::random_range(0_u8..10))
        .collect::<Vec<_>>();
    let sum = digits
        .iter()
        .enumerate()
        .map(|(index, digit)| {
            if index % 2 == 1 {
                let doubled = digit * 2;
                if doubled > 9 { doubled - 9 } else { doubled }
            } else {
                *digit
            }
        })
        .sum::<u8>();
    digits.push((10 - sum % 10) % 10);
    digits
        .into_iter()
        .map(|digit| char::from(b'0' + digit))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_imei_has_valid_luhn_checksum() {
        let imei = random_imei();
        assert_eq!(imei.len(), 15);
        let sum = imei
            .bytes()
            .enumerate()
            .map(|(index, value)| {
                let digit = value - b'0';
                if index % 2 == 1 && index < 14 {
                    let doubled = digit * 2;
                    if doubled > 9 { doubled - 9 } else { doubled }
                } else {
                    digit
                }
            })
            .sum::<u8>();
        assert_eq!(sum % 10, 0);
    }

    #[test]
    fn freshness_requires_both_identity_values() {
        let mut device = QqDevice {
            qimei: Some("q16".to_owned()),
            qimei36: Some("q36".to_owned()),
            qimei_saved_at: Some(100),
            ..QqDevice::default()
        };
        assert!(device.has_current_qimei(86_499));
        assert!(!device.has_current_qimei(86_500));
        device.qimei36 = None;
        assert!(!device.has_current_qimei(101));
    }

    #[cfg(windows)]
    #[test]
    fn interrupted_windows_publish_recovers_the_previous_device() {
        let root = std::env::temp_dir().join(format!(
            "tuneweave-qq-device-recovery-{}-{}",
            std::process::id(),
            rand::random::<u64>()
        ));
        fs::create_dir_all(&root).expect("create test directory");
        let path = root.join("qq-device.json");
        let backup = path.with_extension("backup");
        let expected = QqDevice {
            qimei: Some("persisted-q16".to_owned()),
            qimei36: Some("persisted-q36".to_owned()),
            qimei_saved_at: Some(100),
            ..QqDevice::default()
        };
        fs::write(
            &backup,
            serde_json::to_vec(&expected).expect("encode device"),
        )
        .expect("write interrupted backup");

        let store = DeviceStore::open(Some(path.clone())).expect("recover store");
        assert_eq!(store.device().qimei.as_deref(), Some("persisted-q16"));
        assert!(path.exists());
        assert!(!backup.exists());

        fs::remove_dir_all(root).expect("remove test directory");
    }
}
