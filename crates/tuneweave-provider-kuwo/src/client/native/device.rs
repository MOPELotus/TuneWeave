//! Independent installation identity and anonymous app-device registration.
use super::*;
use rand::{TryRng, rngs::SysRng};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[cfg(test)]
mod tests;
mod transport;
const MAX_STATE_BYTES: u64 = 8192;
const LOCK_WAIT: Duration = Duration::from_secs(30);
pub(super) const FALLBACK_Q36: &str = "f2ce3c2ef68ddfd1b2bea7ed00001f314716";

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct NativeDeviceContext {
    pub(super) device_user: String,
    pub(super) android_id: String,
}
impl NativeDeviceContext {
    pub(super) fn valid(&self) -> bool {
        valid_id(&self.device_user)
            && valid_id(&self.android_id)
            && self.device_user != self.android_id
    }
}
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u8,
    context: NativeDeviceContext,
    app_uid: Option<String>,
    registered_at_ms: Option<u64>,
}
impl State {
    fn generate() -> Result<Self> {
        let state = Self {
            version: 1,
            context: NativeDeviceContext {
                device_user: random_id()?,
                android_id: random_id()?,
            },
            app_uid: None,
            registered_at_ms: None,
        };
        state.validate()?;
        Ok(state)
    }
    fn validate(&self) -> Result<()> {
        if self.version != 1
            || !self.context.valid()
            || self.app_uid.as_deref().is_some_and(|id| !valid_app_uid(id))
            || self.app_uid.is_some() != self.registered_at_ms.is_some()
            || self.registered_at_ms == Some(0)
        {
            return Err(state_error());
        }
        Ok(())
    }
    fn registered(&self) -> Result<KuwoNativeDevice> {
        Ok(KuwoNativeDevice {
            context: self.context.clone(),
            app_uid: self.app_uid.clone().ok_or_else(state_error)?,
            registered_at_ms: self.registered_at_ms.ok_or_else(state_error)?,
        })
    }
}

/// An anonymous registered installation, not a logged-in account.
#[derive(Clone, Eq, PartialEq)]
pub struct KuwoNativeDevice {
    context: NativeDeviceContext,
    app_uid: String,
    registered_at_ms: u64,
}
impl fmt::Debug for KuwoNativeDevice {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KuwoNativeDevice { registered: true, identifiers: [redacted] }")
    }
}
impl KuwoNativeDevice {
    #[must_use]
    pub fn app_uid(&self) -> &str {
        &self.app_uid
    }
    #[must_use]
    pub fn device_user(&self) -> &str {
        &self.context.device_user
    }
    #[must_use]
    pub fn android_id(&self) -> &str {
        &self.context.android_id
    }
    /// Local observation time, not an upstream expiry.
    #[must_use]
    pub const fn registered_at_ms(&self) -> u64 {
        self.registered_at_ms
    }
    /// Builds unverified session input with this installation's full context.
    pub fn session_input(&self, user_id: &str, session_id: &str) -> Result<KuwoNativeSessionInput> {
        let mut input =
            KuwoNativeSessionInput::new(user_id, session_id, self.app_uid(), self.device_user())?;
        input.context = Some(self.context.clone());
        Ok(input)
    }
}

/// Lazily creates an installation and optionally persists its identifiers locally.
///
/// The file never contains account credentials. Instances and cooperating
/// processes using the same path serialize registration with bounded lock waits.
pub struct KuwoNativeDeviceStore {
    path: Option<PathBuf>,
    state: Mutex<Option<State>>,
}
impl fmt::Debug for KuwoNativeDeviceStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KuwoNativeDeviceStore")
            .field("persistent", &self.path.is_some())
            .finish_non_exhaustive()
    }
}
impl Default for KuwoNativeDeviceStore {
    fn default() -> Self {
        Self::new(None)
    }
}
impl KuwoNativeDeviceStore {
    #[must_use]
    pub const fn new(path: Option<PathBuf>) -> Self {
        Self {
            path,
            state: Mutex::const_new(None),
        }
    }
    /// Initializes once; registered installations require no network call.
    pub async fn initialize(&self, client: &KuwoClient) -> Result<KuwoNativeDevice> {
        self.update(client, false).await
    }
    /// Explicitly refreshes registration using the same installation. Failure
    /// leaves its previous state intact; no automatic retry is performed.
    pub async fn refresh(&self, client: &KuwoClient) -> Result<KuwoNativeDevice> {
        self.update(client, true).await
    }
    async fn update(&self, client: &KuwoClient, refresh: bool) -> Result<KuwoNativeDevice> {
        let mut memory = tokio::time::timeout(LOCK_WAIT, self.state.lock())
            .await
            .map_err(|_| conflict())?;
        let _file_guard = if let Some(path) = &self.path {
            Some(lock_file(path, LOCK_WAIT).await?)
        } else {
            None
        };
        let state = match &self.path {
            Some(path) => match read_state(path)? {
                Some(state) => state,
                None => {
                    let generated = State::generate()?;
                    publish(path, None, &generated)?;
                    read_state(path)?.ok_or_else(state_error)?
                }
            },
            None => match memory.as_ref() {
                Some(state) => state.clone(),
                None => State::generate()?,
            },
        };
        state.validate()?;
        // Save the original identity before an await, including cancellation.
        *memory = Some(state.clone());
        if state.app_uid.is_some() && !refresh {
            return state.registered();
        }
        let app_uid = client
            .register_native_device(&state.context, state.app_uid.as_deref())
            .await?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| state_error())?
            .as_millis();
        let updated = State {
            app_uid: Some(app_uid),
            registered_at_ms: Some(u64::try_from(now).map_err(|_| state_error())?),
            ..state.clone()
        };
        updated.validate()?;
        if let Some(path) = &self.path {
            publish(path, Some(&state), &updated)?;
        }
        let device = updated.registered()?;
        *memory = Some(updated);
        Ok(device)
    }
}

fn random_id() -> Result<String> {
    let mut bytes = [0; 16];
    SysRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| state_error())?;
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let mut value = String::with_capacity(32);
    for byte in bytes {
        write!(value, "{byte:02x}").map_err(|_| state_error())?;
    }
    Ok(value)
}

#[cfg(test)]
pub(super) fn fixture_device() -> KuwoNativeDevice {
    KuwoNativeDevice {
        context: NativeDeviceContext {
            device_user: "00112233445546778899aabbccddeeff".into(),
            android_id: "ffeeddccbbaa49888776655443322110".into(),
        },
        app_uid: "1234567890".into(),
        registered_at_ms: 1,
    }
}
fn valid_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 32
        && bytes[12] == b'4'
        && b"89ab".contains(&bytes[16])
        && bytes
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}
fn valid_app_uid(value: &str) -> bool {
    !value.starts_with('0')
        && value.bytes().all(|b| b.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|v| v > 0)
}
fn state_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        "Kuwo native device state is unavailable or invalid",
    )
    .with_platform(Platform::Kuwo)
}
fn conflict() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::Conflict,
        "Kuwo native device state changed or is busy",
    )
    .with_platform(Platform::Kuwo)
}
fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
}
fn named_suffix(path: &Path, suffix: &str) -> Result<PathBuf> {
    let mut name = path.file_name().ok_or_else(state_error)?.to_os_string();
    name.push(suffix);
    Ok(path.with_file_name(name))
}
fn prepare_parent(path: &Path) -> Result<()> {
    if path.file_name().is_none() {
        return Err(state_error());
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(parent(path)).map_err(|_| state_error())
}
fn checked_metadata(path: &Path, file: &File, empty: bool) -> Result<()> {
    let on_path = fs::symlink_metadata(path).map_err(|_| state_error())?;
    let opened = file.metadata().map_err(|_| state_error())?;
    if !on_path.is_file()
        || on_path.file_type().is_symlink()
        || !opened.is_file()
        || on_path.len() > MAX_STATE_BYTES
        || opened.len() > MAX_STATE_BYTES
        || (empty && (on_path.len() != 0 || opened.len() != 0))
    {
        return Err(state_error());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if on_path.dev() != opened.dev() || on_path.ino() != opened.ino() || opened.nlink() != 1 {
            return Err(state_error());
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| state_error())?;
    }
    Ok(())
}
async fn lock_file(path: &Path, wait: Duration) -> Result<File> {
    prepare_parent(path)?;
    let lock = named_suffix(path, ".lock")?;
    let mut options = OpenOptions::new();
    options.read(true).write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = match options.open(&lock) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(&lock).map_err(|_| state_error())?;
            if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() != 0 {
                return Err(state_error());
            }
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(&lock)
                .map_err(|_| state_error())?
        }
        Err(_) => return Err(state_error()),
    };
    checked_metadata(&lock, &file, true)?;
    let deadline = Instant::now() + wait;
    loop {
        match fs4::FileExt::try_lock(&file) {
            Ok(()) => {
                checked_metadata(&lock, &file, true)?;
                return Ok(file);
            }
            Err(fs4::TryLockError::WouldBlock) if Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(25)).await
            }
            Err(fs4::TryLockError::WouldBlock) => return Err(conflict()),
            Err(_) => return Err(state_error()),
        }
    }
}
fn read_state(path: &Path) -> Result<Option<State>> {
    match fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Ok(meta)
            if meta.is_file()
                && !meta.file_type().is_symlink()
                && meta.len() <= MAX_STATE_BYTES => {}
        _ => return Err(state_error()),
    }
    let file = File::open(path).map_err(|_| state_error())?;
    checked_metadata(path, &file, false)?;
    let mut data = Vec::new();
    file.take(MAX_STATE_BYTES + 1)
        .read_to_end(&mut data)
        .map_err(|_| state_error())?;
    if data.len() as u64 > MAX_STATE_BYTES {
        return Err(state_error());
    }
    let state: State = serde_json::from_slice(&data).map_err(|_| state_error())?;
    state.validate()?;
    Ok(Some(state))
}
fn publish(path: &Path, expected: Option<&State>, value: &State) -> Result<()> {
    value.validate()?;
    if read_state(path)?.as_ref() != expected {
        return Err(conflict());
    }
    let temporary = named_suffix(path, &format!(".tmp-{}", random_id()?))?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|_| state_error())?;
    let result = (|| {
        let data = serde_json::to_vec(value).map_err(|_| state_error())?;
        file.write_all(&data)
            .and_then(|()| file.sync_all())
            .map_err(|_| state_error())?;
        drop(file);
        if read_state(path)?.as_ref() != expected {
            return Err(conflict());
        }
        if expected.is_none() {
            fs::hard_link(&temporary, path).map_err(|_| conflict())?;
            fs::remove_file(&temporary).map_err(|_| state_error())?;
        } else {
            fs::rename(&temporary, path).map_err(|_| state_error())?;
        }
        #[cfg(unix)]
        File::open(parent(path))
            .and_then(|file| file.sync_all())
            .map_err(|_| state_error())?;
        Ok(())
    })();
    let _ = fs::remove_file(&temporary);
    result
}
