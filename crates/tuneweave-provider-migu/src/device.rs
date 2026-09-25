use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use rand::{TryRng, rngs::SysRng};
use serde::{Deserialize, Serialize};
use tuneweave_core::{ErrorCode, Result, TuneWeaveError};

use crate::credential::error;

const MAX_BYTES: u64 = 4096;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Device {
    version: u8,
    device_id: String,
}

#[derive(Default)]
pub(crate) struct MusicDeviceStore {
    path: Option<PathBuf>,
    state: Mutex<Option<String>>,
}
impl std::fmt::Debug for MusicDeviceStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MusicDeviceStore")
            .field("persistent", &self.path.is_some())
            .finish_non_exhaustive()
    }
}
impl MusicDeviceStore {
    pub(crate) const fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            state: Mutex::new(None),
        }
    }
    pub(crate) fn identity(&self) -> Result<String> {
        let mut cached = self.state.lock().map_err(|_| state_error())?;
        if let Some(id) = cached.as_ref() {
            return Ok(id.clone());
        }
        let id = if let Some(path) = &self.path {
            match read(path) {
                Ok(Some(id)) => id,
                Ok(None) => {
                    publish(path, &generate()?)?;
                    read(path)?.ok_or_else(state_error)?
                }
                Err(e) => return Err(e),
            }
        } else {
            generate()?
        };
        *cached = Some(id.clone());
        Ok(id)
    }
}

fn generate() -> Result<String> {
    let mut bytes = [0; 16];
    SysRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| state_error())?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex = hex::encode_upper(bytes);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}
fn valid_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    bytes.len() == 36
        && bytes[14] == b'4'
        && b"89AB".contains(&bytes[19])
        && bytes.iter().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                *b == b'-'
            } else {
                b.is_ascii_digit() || (b'A'..=b'F').contains(b)
            }
        })
}
fn read(path: &Path) -> Result<Option<String>> {
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() && metadata.len() <= MAX_BYTES => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        _ => return Err(state_error()),
    }
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(state_error()),
    };
    let metadata = file.metadata().map_err(|_| state_error())?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(state_error());
    }
    let mut data = Vec::new();
    file.take(MAX_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|_| state_error())?;
    if data.len() as u64 > MAX_BYTES {
        return Err(state_error());
    }
    let device: Device = serde_json::from_slice(&data).map_err(|_| state_error())?;
    if device.version != 1 || !valid_id(&device.device_id) {
        return Err(state_error());
    }
    Ok(Some(device.device_id))
}
fn publish(path: &Path, id: &str) -> Result<()> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(parent).map_err(|_| state_error())?;
    }
    let temp = path.with_extension(format!("tmp-{}", generate()?));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp).map_err(|_| state_error())?;
    let result = (|| {
        let data = serde_json::to_vec(&Device {
            version: 1,
            device_id: id.to_owned(),
        })
        .map_err(|_| state_error())?;
        file.write_all(&data)
            .and_then(|()| file.sync_all())
            .map_err(|_| state_error())?;
        drop(file);
        match fs::hard_link(&temp, path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            Err(_) => Err(state_error()),
        }
    })();
    let _ = fs::remove_file(&temp);
    result
}
fn state_error() -> TuneWeaveError {
    error(
        ErrorCode::InternalError,
        "Migu music device state is unavailable or invalid",
    )
}

#[cfg(test)]
mod tests;
