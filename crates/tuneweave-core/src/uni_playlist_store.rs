use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
    process,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{ErrorCode, Platform, Result, TuneWeaveError, UniPlaylist};

const UNI_PLAYLIST_FILE_VERSION: u32 = 1;
static UNI_PLAYLIST_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub trait UniPlaylistStore: Send + Sync {
    fn create(&self, playlist: &UniPlaylist) -> Result<()>;
    fn get(&self, id: &str) -> Result<Option<UniPlaylist>>;
}

#[derive(Clone, Debug, Default)]
pub struct MemoryUniPlaylistStore {
    playlists: Arc<RwLock<BTreeMap<String, UniPlaylist>>>,
}

impl UniPlaylistStore for MemoryUniPlaylistStore {
    fn create(&self, playlist: &UniPlaylist) -> Result<()> {
        validate_uni_playlist(playlist)?;
        let mut playlists = self
            .playlists
            .write()
            .map_err(|_| uni_playlist_lock_error())?;
        if playlists.contains_key(&playlist.id) {
            return Err(uni_playlist_conflict(&playlist.id));
        }
        playlists.insert(playlist.id.clone(), playlist.clone());
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Option<UniPlaylist>> {
        validate_uni_playlist_id(id)?;
        Ok(self
            .playlists
            .read()
            .map_err(|_| uni_playlist_lock_error())?
            .get(id)
            .cloned())
    }
}

#[derive(Clone, Debug)]
pub struct FileUniPlaylistStore {
    path: PathBuf,
    database: Arc<RwLock<UniPlaylistDatabase>>,
}

impl FileUniPlaylistStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        recover_interrupted_publish(&path)?;
        let database = load_database(&path)?;
        Ok(Self {
            path,
            database: Arc::new(RwLock::new(database)),
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl UniPlaylistStore for FileUniPlaylistStore {
    fn create(&self, playlist: &UniPlaylist) -> Result<()> {
        validate_uni_playlist(playlist)?;
        let mut database = self
            .database
            .write()
            .map_err(|_| uni_playlist_lock_error())?;
        if database.playlists.contains_key(&playlist.id) {
            return Err(uni_playlist_conflict(&playlist.id));
        }
        let mut next = database.clone();
        next.playlists.insert(playlist.id.clone(), playlist.clone());
        persist_database(&self.path, &next)?;
        *database = next;
        Ok(())
    }

    fn get(&self, id: &str) -> Result<Option<UniPlaylist>> {
        validate_uni_playlist_id(id)?;
        Ok(self
            .database
            .read()
            .map_err(|_| uni_playlist_lock_error())?
            .playlists
            .get(id)
            .cloned())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UniPlaylistDatabase {
    version: u32,
    playlists: BTreeMap<String, UniPlaylist>,
}

impl Default for UniPlaylistDatabase {
    fn default() -> Self {
        Self {
            version: UNI_PLAYLIST_FILE_VERSION,
            playlists: BTreeMap::new(),
        }
    }
}

fn load_database(path: &Path) -> Result<UniPlaylistDatabase> {
    let encoded = match fs::read(path) {
        Ok(encoded) => encoded,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Default::default()),
        Err(error) => return Err(store_io_error("read Uni Playlist database", error)),
    };
    let database = serde_json::from_slice::<UniPlaylistDatabase>(&encoded).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::InternalError,
            format!("failed to decode Uni Playlist database: {error}"),
        )
    })?;
    if database.version != UNI_PLAYLIST_FILE_VERSION {
        return Err(TuneWeaveError::new(
            ErrorCode::InternalError,
            format!(
                "unsupported Uni Playlist database version: {}",
                database.version
            ),
        ));
    }
    for (id, playlist) in &database.playlists {
        if id != &playlist.id || validate_uni_playlist(playlist).is_err() {
            return Err(TuneWeaveError::new(
                ErrorCode::InternalError,
                "Uni Playlist database contains an invalid playlist record",
            ));
        }
    }
    Ok(database)
}

fn persist_database(path: &Path, database: &UniPlaylistDatabase) -> Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty());
    if let Some(parent) = parent {
        fs::create_dir_all(parent)
            .map_err(|error| store_io_error("create Uni Playlist data directory", error))?;
    }
    let encoded = serde_json::to_vec(database).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::InternalError,
            format!("failed to serialize Uni Playlist database: {error}"),
        )
    })?;
    let sequence = UNI_PLAYLIST_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("uni-playlists.json");
    let temporary_path =
        path.with_file_name(format!(".{file_name}.{}.{}.tmp", process::id(), sequence));
    if let Err(error) = write_private_file(&temporary_path, &encoded) {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }
    if let Err(error) = publish_database(&temporary_path, path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }
    Ok(())
}

fn write_private_file(path: &Path, encoded: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| store_io_error("create temporary Uni Playlist database", error))?;
    file.write_all(encoded)
        .map_err(|error| store_io_error("write Uni Playlist database", error))?;
    file.sync_all()
        .map_err(|error| store_io_error("sync Uni Playlist database", error))
}

#[cfg(not(windows))]
fn publish_database(temporary_path: &Path, path: &Path) -> Result<()> {
    fs::rename(temporary_path, path)
        .map_err(|error| store_io_error("publish Uni Playlist database", error))
}

#[cfg(windows)]
fn publish_database(temporary_path: &Path, path: &Path) -> Result<()> {
    let backup_path = backup_path(path);
    if backup_path.exists() {
        fs::remove_file(&backup_path)
            .map_err(|error| store_io_error("remove stale Uni Playlist backup", error))?;
    }
    if path.exists() {
        fs::rename(path, &backup_path)
            .map_err(|error| store_io_error("prepare Uni Playlist database replacement", error))?;
    }
    match fs::rename(temporary_path, path) {
        Ok(()) => {
            if backup_path.exists() {
                fs::remove_file(&backup_path)
                    .map_err(|error| store_io_error("remove Uni Playlist backup", error))?;
            }
            Ok(())
        }
        Err(error) => {
            if backup_path.exists() {
                let _ = fs::rename(&backup_path, path);
            }
            Err(store_io_error("publish Uni Playlist database", error))
        }
    }
}

fn recover_interrupted_publish(path: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        let backup_path = backup_path(path);
        match (path.exists(), backup_path.exists()) {
            (false, true) => fs::rename(&backup_path, path)
                .map_err(|error| store_io_error("recover Uni Playlist database", error))?,
            (true, true) => fs::remove_file(&backup_path)
                .map_err(|error| store_io_error("remove recovered Uni Playlist backup", error))?,
            _ => {}
        }
    }
    #[cfg(not(windows))]
    let _ = path;
    Ok(())
}

#[cfg(windows)]
fn backup_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("uni-playlists.json");
    path.with_file_name(format!(".{file_name}.backup"))
}

fn validate_uni_playlist(playlist: &UniPlaylist) -> Result<()> {
    validate_uni_playlist_id(&playlist.id)?;
    if playlist.platform != Platform::Uni
        || playlist.resource_ref.platform() != Platform::Uni
        || playlist.resource_ref.id() != playlist.id
    {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist identity must use one matching uni:<id> reference",
        ));
    }
    if playlist.name.trim().is_empty() || playlist.name.len() > 200 {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist name must contain at most 200 bytes",
        ));
    }
    if playlist.description.len() > 4_000 {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist description cannot exceed 4000 bytes",
        ));
    }
    if playlist.updated_at_ms < playlist.created_at_ms {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist updated_at_ms cannot precede created_at_ms",
        ));
    }
    Ok(())
}

fn validate_uni_playlist_id(id: &str) -> Result<()> {
    if !(16..=64).contains(&id.len())
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist id must be 16 to 64 URL-safe ASCII characters",
        )
        .with_details(json!({ "id": id })));
    }
    Ok(())
}

fn uni_playlist_conflict(id: &str) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::Conflict,
        "a Uni Playlist with this id already exists",
    )
    .with_details(json!({ "ref": format!("uni:{id}") }))
}

fn uni_playlist_lock_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        "Uni Playlist store lock is poisoned",
    )
}

fn store_io_error(operation: &str, error: std::io::Error) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        format!("failed to {operation}: {error}"),
    )
}

#[cfg(test)]
mod tests {
    use std::{env, fs, path::PathBuf};

    use super::*;
    use crate::ResourceRef;

    struct TempDirectory(PathBuf);

    impl TempDirectory {
        fn new() -> Self {
            let sequence = UNI_PLAYLIST_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "tuneweave-uni-playlist-store-{}-{sequence}",
                process::id()
            ));
            fs::create_dir_all(&path).expect("create temporary directory");
            Self(path)
        }
    }

    impl Drop for TempDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn sample_playlist(id: &str) -> UniPlaylist {
        UniPlaylist::new(
            ResourceRef::new(Platform::Uni, id).expect("valid Uni Playlist reference"),
            "Cross-platform favorites",
            "An ordered mixed-platform playlist",
            1_753_137_600_000,
        )
    }

    #[test]
    fn memory_store_round_trips_and_rejects_duplicate_ids() {
        let store = MemoryUniPlaylistStore::default();
        let playlist = sample_playlist("pl_01abcdefghijklmnop");
        store.create(&playlist).expect("create playlist");
        assert_eq!(
            store.get(&playlist.id).expect("get playlist"),
            Some(playlist.clone())
        );
        assert_eq!(
            store.create(&playlist).expect_err("reject duplicate").code,
            ErrorCode::Conflict
        );
    }

    #[test]
    fn file_store_uses_one_reloadable_database_file() {
        let directory = TempDirectory::new();
        let path = directory.0.join("uni-playlists.json");
        let playlist = sample_playlist("pl_01abcdefghijklmnop");
        let store = FileUniPlaylistStore::open(&path).expect("open file store");
        store.create(&playlist).expect("persist playlist");
        assert!(path.is_file());
        assert_eq!(
            fs::read_dir(&directory.0)
                .expect("read data directory")
                .filter_map(std::result::Result::ok)
                .filter(|entry| entry.path().is_file())
                .count(),
            1
        );

        let reopened = FileUniPlaylistStore::open(&path).expect("reopen file store");
        assert_eq!(
            reopened.get(&playlist.id).expect("reload playlist"),
            Some(playlist)
        );
    }

    #[test]
    fn file_store_refuses_unknown_versions_without_overwriting_them() {
        let directory = TempDirectory::new();
        let path = directory.0.join("uni-playlists.json");
        fs::write(&path, br#"{"version":2,"playlists":{}}"#)
            .expect("write future database fixture");
        let error = FileUniPlaylistStore::open(&path).expect_err("reject future version");
        assert_eq!(error.code, ErrorCode::InternalError);
        assert!(
            fs::read_to_string(path)
                .expect("database remains")
                .contains("\"version\":2")
        );
    }
}
