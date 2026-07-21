use std::{
    collections::{BTreeMap, BTreeSet},
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

use crate::{
    ErrorCode, Extensions, Page, PageMeta, Platform, Result, TuneWeaveError, UniPlaylist,
    UniPlaylistItem, UniPlaylistItemAddResult, UniPlaylistItemDeleteResult,
    UniPlaylistItemOrderResult,
};

const UNI_PLAYLIST_FILE_VERSION: u32 = 1;
static UNI_PLAYLIST_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

pub trait UniPlaylistStore: Send + Sync {
    fn create(&self, playlist: &UniPlaylist) -> Result<()>;
    fn create_with_items(&self, playlist: &UniPlaylist, items: &[UniPlaylistItem]) -> Result<()>;
    fn get(&self, id: &str) -> Result<Option<UniPlaylist>>;
    fn append_items(
        &self,
        playlist_id: &str,
        items: &[UniPlaylistItem],
    ) -> Result<UniPlaylistItemAddResult>;
    fn items(&self, playlist_id: &str, limit: u32, offset: u32) -> Result<Page<UniPlaylistItem>>;
    fn remove_item(
        &self,
        playlist_id: &str,
        item_id: &str,
        updated_at_ms: u64,
    ) -> Result<UniPlaylistItemDeleteResult>;
    fn reorder_items(
        &self,
        playlist_id: &str,
        item_ids: &[String],
        updated_at_ms: u64,
    ) -> Result<UniPlaylistItemOrderResult>;
}

#[derive(Clone, Debug, Default)]
pub struct MemoryUniPlaylistStore {
    database: Arc<RwLock<UniPlaylistDatabase>>,
}

impl UniPlaylistStore for MemoryUniPlaylistStore {
    fn create(&self, playlist: &UniPlaylist) -> Result<()> {
        validate_new_playlist(playlist, &[])?;
        let mut database = self
            .database
            .write()
            .map_err(|_| uni_playlist_lock_error())?;
        if database.playlists.contains_key(&playlist.id) {
            return Err(uni_playlist_conflict(&playlist.id));
        }
        database
            .playlists
            .insert(playlist.id.clone(), playlist.clone());
        database.items.insert(playlist.id.clone(), Vec::new());
        Ok(())
    }

    fn create_with_items(&self, playlist: &UniPlaylist, items: &[UniPlaylistItem]) -> Result<()> {
        validate_new_playlist(playlist, items)?;
        let mut database = self
            .database
            .write()
            .map_err(|_| uni_playlist_lock_error())?;
        if database.playlists.contains_key(&playlist.id) {
            return Err(uni_playlist_conflict(&playlist.id));
        }
        database
            .playlists
            .insert(playlist.id.clone(), playlist.clone());
        database.items.insert(playlist.id.clone(), items.to_vec());
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

    fn append_items(
        &self,
        playlist_id: &str,
        items: &[UniPlaylistItem],
    ) -> Result<UniPlaylistItemAddResult> {
        let mut database = self
            .database
            .write()
            .map_err(|_| uni_playlist_lock_error())?;
        append_items_to_database(&mut database, playlist_id, items)
    }

    fn items(&self, playlist_id: &str, limit: u32, offset: u32) -> Result<Page<UniPlaylistItem>> {
        let database = self
            .database
            .read()
            .map_err(|_| uni_playlist_lock_error())?;
        playlist_items_page(&database, playlist_id, limit, offset)
    }

    fn remove_item(
        &self,
        playlist_id: &str,
        item_id: &str,
        updated_at_ms: u64,
    ) -> Result<UniPlaylistItemDeleteResult> {
        let mut database = self
            .database
            .write()
            .map_err(|_| uni_playlist_lock_error())?;
        remove_item_from_database(&mut database, playlist_id, item_id, updated_at_ms)
    }

    fn reorder_items(
        &self,
        playlist_id: &str,
        item_ids: &[String],
        updated_at_ms: u64,
    ) -> Result<UniPlaylistItemOrderResult> {
        let mut database = self
            .database
            .write()
            .map_err(|_| uni_playlist_lock_error())?;
        reorder_items_in_database(&mut database, playlist_id, item_ids, updated_at_ms)
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
        validate_new_playlist(playlist, &[])?;
        let mut database = self
            .database
            .write()
            .map_err(|_| uni_playlist_lock_error())?;
        if database.playlists.contains_key(&playlist.id) {
            return Err(uni_playlist_conflict(&playlist.id));
        }
        let mut next = database.clone();
        next.playlists.insert(playlist.id.clone(), playlist.clone());
        next.items.insert(playlist.id.clone(), Vec::new());
        persist_database(&self.path, &next)?;
        *database = next;
        Ok(())
    }

    fn create_with_items(&self, playlist: &UniPlaylist, items: &[UniPlaylistItem]) -> Result<()> {
        validate_new_playlist(playlist, items)?;
        let mut database = self
            .database
            .write()
            .map_err(|_| uni_playlist_lock_error())?;
        if database.playlists.contains_key(&playlist.id) {
            return Err(uni_playlist_conflict(&playlist.id));
        }
        let mut next = database.clone();
        next.playlists.insert(playlist.id.clone(), playlist.clone());
        next.items.insert(playlist.id.clone(), items.to_vec());
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

    fn append_items(
        &self,
        playlist_id: &str,
        items: &[UniPlaylistItem],
    ) -> Result<UniPlaylistItemAddResult> {
        let mut database = self
            .database
            .write()
            .map_err(|_| uni_playlist_lock_error())?;
        let mut next = database.clone();
        let result = append_items_to_database(&mut next, playlist_id, items)?;
        persist_database(&self.path, &next)?;
        *database = next;
        Ok(result)
    }

    fn items(&self, playlist_id: &str, limit: u32, offset: u32) -> Result<Page<UniPlaylistItem>> {
        let database = self
            .database
            .read()
            .map_err(|_| uni_playlist_lock_error())?;
        playlist_items_page(&database, playlist_id, limit, offset)
    }

    fn remove_item(
        &self,
        playlist_id: &str,
        item_id: &str,
        updated_at_ms: u64,
    ) -> Result<UniPlaylistItemDeleteResult> {
        let mut database = self
            .database
            .write()
            .map_err(|_| uni_playlist_lock_error())?;
        let mut next = database.clone();
        let result = remove_item_from_database(&mut next, playlist_id, item_id, updated_at_ms)?;
        persist_database(&self.path, &next)?;
        *database = next;
        Ok(result)
    }

    fn reorder_items(
        &self,
        playlist_id: &str,
        item_ids: &[String],
        updated_at_ms: u64,
    ) -> Result<UniPlaylistItemOrderResult> {
        let mut database = self
            .database
            .write()
            .map_err(|_| uni_playlist_lock_error())?;
        let mut next = database.clone();
        let result = reorder_items_in_database(&mut next, playlist_id, item_ids, updated_at_ms)?;
        if result.changed {
            persist_database(&self.path, &next)?;
            *database = next;
        }
        Ok(result)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UniPlaylistDatabase {
    version: u32,
    playlists: BTreeMap<String, UniPlaylist>,
    #[serde(default)]
    items: BTreeMap<String, Vec<UniPlaylistItem>>,
}

impl Default for UniPlaylistDatabase {
    fn default() -> Self {
        Self {
            version: UNI_PLAYLIST_FILE_VERSION,
            playlists: BTreeMap::new(),
            items: BTreeMap::new(),
        }
    }
}

fn append_items_to_database(
    database: &mut UniPlaylistDatabase,
    playlist_id: &str,
    new_items: &[UniPlaylistItem],
) -> Result<UniPlaylistItemAddResult> {
    validate_uni_playlist_id(playlist_id)?;
    if new_items.is_empty() || new_items.len() > 100 {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist item append must contain between 1 and 100 items",
        ));
    }
    for item in new_items {
        validate_uni_playlist_item(item)?;
    }
    let playlist = database
        .playlists
        .get_mut(playlist_id)
        .ok_or_else(|| uni_playlist_not_found(playlist_id))?;
    let stored_items = database.items.entry(playlist_id.to_owned()).or_default();
    let mut item_ids = stored_items
        .iter()
        .map(|item| item.id.as_str())
        .collect::<BTreeSet<_>>();
    for item in new_items {
        if !item_ids.insert(&item.id) {
            return Err(TuneWeaveError::new(
                ErrorCode::Conflict,
                "a Uni Playlist item with this id already exists",
            )
            .with_details(json!({ "item_id": item.id })));
        }
    }
    let previous_item_count = u64::try_from(stored_items.len()).unwrap_or(u64::MAX);
    let mut appended = Vec::with_capacity(new_items.len());
    for item in new_items {
        let mut item = item.clone();
        item.position = u64::try_from(stored_items.len()).unwrap_or(u64::MAX);
        playlist.updated_at_ms = playlist.updated_at_ms.max(item.added_at_ms);
        stored_items.push(item.clone());
        appended.push(item);
    }
    playlist.item_count = u64::try_from(stored_items.len()).unwrap_or(u64::MAX);
    Ok(UniPlaylistItemAddResult {
        playlist: playlist.clone(),
        items: appended,
        extensions: Extensions::from([
            ("previous_item_count".to_owned(), json!(previous_item_count)),
            ("duplicates_preserved".to_owned(), json!(true)),
        ]),
    })
}

fn playlist_items_page(
    database: &UniPlaylistDatabase,
    playlist_id: &str,
    limit: u32,
    offset: u32,
) -> Result<Page<UniPlaylistItem>> {
    validate_uni_playlist_id(playlist_id)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist item limit must be between 1 and 100",
        ));
    }
    if !database.playlists.contains_key(playlist_id) {
        return Err(uni_playlist_not_found(playlist_id));
    }
    let stored_items = database
        .items
        .get(playlist_id)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let total = u64::try_from(stored_items.len()).unwrap_or(u64::MAX);
    let items = stored_items
        .iter()
        .skip(offset as usize)
        .take(limit as usize)
        .cloned()
        .collect::<Vec<_>>();
    let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let candidate_next = offset.saturating_add(consumed);
    let has_more = u64::from(candidate_next) < total;
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total: Some(total),
            next_offset: has_more.then_some(candidate_next),
            has_more,
            extensions: Extensions::from([("duplicates_preserved".to_owned(), json!(true))]),
        },
    })
}

fn remove_item_from_database(
    database: &mut UniPlaylistDatabase,
    playlist_id: &str,
    item_id: &str,
    updated_at_ms: u64,
) -> Result<UniPlaylistItemDeleteResult> {
    validate_uni_playlist_id(playlist_id)?;
    validate_uni_playlist_item_id(item_id)?;
    if !database.playlists.contains_key(playlist_id) {
        return Err(uni_playlist_not_found(playlist_id));
    }
    let stored_items = database.items.entry(playlist_id.to_owned()).or_default();
    let position = stored_items
        .iter()
        .position(|item| item.id == item_id)
        .ok_or_else(|| uni_playlist_item_not_found(playlist_id, item_id))?;
    let previous_item_count = u64::try_from(stored_items.len()).unwrap_or(u64::MAX);
    let removed = stored_items.remove(position);
    for (position, item) in stored_items.iter_mut().enumerate() {
        item.position = u64::try_from(position).unwrap_or(u64::MAX);
    }
    let playlist = database
        .playlists
        .get_mut(playlist_id)
        .expect("playlist existence checked before item removal");
    playlist.item_count = u64::try_from(stored_items.len()).unwrap_or(u64::MAX);
    playlist.updated_at_ms = playlist.updated_at_ms.max(updated_at_ms);
    Ok(UniPlaylistItemDeleteResult {
        playlist: playlist.clone(),
        item: removed,
        extensions: Extensions::from([
            ("previous_item_count".to_owned(), json!(previous_item_count)),
            ("duplicates_preserved".to_owned(), json!(true)),
        ]),
    })
}

fn reorder_items_in_database(
    database: &mut UniPlaylistDatabase,
    playlist_id: &str,
    item_ids: &[String],
    updated_at_ms: u64,
) -> Result<UniPlaylistItemOrderResult> {
    validate_uni_playlist_id(playlist_id)?;
    if !database.playlists.contains_key(playlist_id) {
        return Err(uni_playlist_not_found(playlist_id));
    }
    let stored_items = database
        .items
        .get(playlist_id)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut submitted = BTreeSet::new();
    for item_id in item_ids {
        validate_uni_playlist_item_id(item_id)?;
        if !submitted.insert(item_id.as_str()) {
            return Err(TuneWeaveError::invalid_request(
                "Uni Playlist item order cannot contain duplicate item ids",
            )
            .with_details(json!({ "item_id": item_id })));
        }
    }
    let stored_ids = stored_items
        .iter()
        .map(|item| item.id.as_str())
        .collect::<BTreeSet<_>>();
    if submitted != stored_ids {
        let missing = stored_ids
            .difference(&submitted)
            .copied()
            .collect::<Vec<_>>();
        let unknown = submitted
            .difference(&stored_ids)
            .copied()
            .collect::<Vec<_>>();
        return Err(TuneWeaveError::invalid_request(
            "item_ids must contain every current Uni Playlist item id exactly once",
        )
        .with_details(json!({ "missing_item_ids": missing, "unknown_item_ids": unknown })));
    }
    let changed = stored_items
        .iter()
        .map(|item| item.id.as_str())
        .ne(item_ids.iter().map(String::as_str));
    if changed {
        let by_id = stored_items
            .iter()
            .cloned()
            .map(|item| (item.id.clone(), item))
            .collect::<BTreeMap<_, _>>();
        let reordered = item_ids
            .iter()
            .enumerate()
            .map(|(position, item_id)| {
                let mut item = by_id
                    .get(item_id)
                    .expect("submitted item ids validated against stored ids")
                    .clone();
                item.position = u64::try_from(position).unwrap_or(u64::MAX);
                item
            })
            .collect::<Vec<_>>();
        database.items.insert(playlist_id.to_owned(), reordered);
        let playlist = database
            .playlists
            .get_mut(playlist_id)
            .expect("playlist existence checked before item reordering");
        playlist.updated_at_ms = playlist.updated_at_ms.max(updated_at_ms);
    }
    let playlist = database
        .playlists
        .get(playlist_id)
        .expect("playlist existence checked before item reordering")
        .clone();
    let items = database.items.get(playlist_id).cloned().unwrap_or_default();
    Ok(UniPlaylistItemOrderResult {
        playlist,
        items,
        changed,
        extensions: Extensions::from([
            ("complete_order_required".to_owned(), json!(true)),
            ("duplicates_preserved".to_owned(), json!(true)),
        ]),
    })
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
        let items = database
            .items
            .get(id)
            .map(Vec::as_slice)
            .unwrap_or_default();
        let item_count = u64::try_from(items.len()).unwrap_or(u64::MAX);
        let items_valid = items.iter().enumerate().all(|(position, item)| {
            item.position == u64::try_from(position).unwrap_or(u64::MAX)
                && item.added_at_ms <= playlist.updated_at_ms
                && validate_uni_playlist_item(item).is_ok()
        });
        if id != &playlist.id
            || validate_uni_playlist(playlist).is_err()
            || playlist.item_count != item_count
            || !items_valid
        {
            return Err(TuneWeaveError::new(
                ErrorCode::InternalError,
                "Uni Playlist database contains an invalid playlist record",
            ));
        }
    }
    if database
        .items
        .keys()
        .any(|id| !database.playlists.contains_key(id))
    {
        return Err(TuneWeaveError::new(
            ErrorCode::InternalError,
            "Uni Playlist database contains items without a playlist",
        ));
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

fn validate_new_playlist(playlist: &UniPlaylist, items: &[UniPlaylistItem]) -> Result<()> {
    validate_uni_playlist(playlist)?;
    let item_count = u64::try_from(items.len()).unwrap_or(u64::MAX);
    if playlist.item_count != item_count {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist item_count must match the initial item sequence",
        ));
    }
    let mut item_ids = BTreeSet::new();
    for (position, item) in items.iter().enumerate() {
        validate_uni_playlist_item(item)?;
        if item.position != u64::try_from(position).unwrap_or(u64::MAX) {
            return Err(TuneWeaveError::invalid_request(
                "initial Uni Playlist item positions must be zero-based and contiguous",
            ));
        }
        if item.added_at_ms > playlist.updated_at_ms {
            return Err(TuneWeaveError::invalid_request(
                "initial Uni Playlist items cannot be newer than the playlist",
            ));
        }
        if !item_ids.insert(item.id.as_str()) {
            return Err(TuneWeaveError::new(
                ErrorCode::Conflict,
                "initial Uni Playlist item ids must be unique",
            )
            .with_details(json!({ "item_id": item.id })));
        }
    }
    Ok(())
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

fn validate_uni_playlist_item(item: &UniPlaylistItem) -> Result<()> {
    validate_uni_playlist_item_id(&item.id)?;
    if item.source_ref.platform() == Platform::Uni {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist items must reference an external platform resource",
        ));
    }
    if item.snapshot.title.trim().is_empty() || item.snapshot.title.len() > 500 {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist item title must contain at most 500 bytes",
        ));
    }
    if item.snapshot.artists.len() > 100
        || item
            .snapshot
            .artists
            .iter()
            .any(|artist| artist.trim().is_empty() || artist.len() > 200)
    {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist item artists must contain at most 100 non-empty names",
        ));
    }
    if item
        .snapshot
        .album
        .as_ref()
        .is_some_and(|album| album.len() > 500)
        || item
            .snapshot
            .isrc
            .as_ref()
            .is_some_and(|isrc| isrc.len() > 64)
        || item
            .snapshot
            .cover_url
            .as_ref()
            .is_some_and(|url| url.len() > 4_096)
        || item.snapshot.version_tags.len() > 100
        || item
            .snapshot
            .version_tags
            .iter()
            .any(|tag| tag.trim().is_empty() || tag.len() > 200)
    {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist item snapshot metadata exceeds its size boundary",
        ));
    }
    Ok(())
}

fn validate_uni_playlist_item_id(id: &str) -> Result<()> {
    if !(16..=64).contains(&id.len())
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(TuneWeaveError::invalid_request(
            "Uni Playlist item id must be 16 to 64 URL-safe ASCII characters",
        )
        .with_details(json!({ "item_id": id })));
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

fn uni_playlist_not_found(id: &str) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::ResourceNotFound, "Uni Playlist was not found")
        .with_details(json!({ "ref": format!("uni:{id}") }))
}

fn uni_playlist_item_not_found(playlist_id: &str, item_id: &str) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::ResourceNotFound,
        "Uni Playlist item was not found",
    )
    .with_details(json!({
        "playlist_ref": format!("uni:{playlist_id}"),
        "item_id": item_id,
    }))
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
    use crate::{ResourceRef, UniPlaylistItemKind, UniPlaylistItemSnapshot};

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

    fn sample_item(id: &str, source_id: &str, added_at_ms: u64) -> UniPlaylistItem {
        let mut snapshot = UniPlaylistItemSnapshot::new("反方向的钟");
        snapshot.artists = vec!["周杰伦".to_owned()];
        snapshot.album = Some("Jay".to_owned());
        snapshot.duration_ms = Some(258_000);
        snapshot.isrc = Some("TWK970000101".to_owned());
        UniPlaylistItem {
            id: id.to_owned(),
            position: 99,
            kind: UniPlaylistItemKind::Track,
            source_ref: ResourceRef::new(Platform::Netease, source_id)
                .expect("valid source reference"),
            snapshot,
            added_at_ms,
            extensions: Extensions::new(),
        }
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
    fn imported_playlist_and_items_are_created_as_one_validated_unit() {
        let store = MemoryUniPlaylistStore::default();
        let mut playlist = sample_playlist("pl_01abcdefghijklmnop");
        playlist.item_count = 2;
        playlist.updated_at_ms = 1_753_137_600_200;
        let first = UniPlaylistItem {
            position: 0,
            ..sample_item("item_01abcdefghijklmnop", "185809", 1_753_137_600_100)
        };
        let second = UniPlaylistItem {
            position: 1,
            ..sample_item("item_02abcdefghijklmnop", "185809", 1_753_137_600_200)
        };
        store
            .create_with_items(&playlist, &[first.clone(), second.clone()])
            .expect("create imported playlist atomically");
        assert_eq!(
            store
                .items(&playlist.id, 100, 0)
                .expect("imported items")
                .items,
            vec![first, second]
        );

        let mut invalid = sample_playlist("pl_02abcdefghijklmnop");
        invalid.item_count = 1;
        let bad_item = sample_item("item_03abcdefghijklmnop", "200001", 1_753_137_600_100);
        let error = store
            .create_with_items(&invalid, &[bad_item])
            .expect_err("reject non-contiguous initial position");
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert_eq!(store.get(&invalid.id).expect("no partial playlist"), None);
    }

    #[test]
    fn item_append_preserves_duplicate_sources_and_pages_by_stable_item_id() {
        let store = MemoryUniPlaylistStore::default();
        let playlist = sample_playlist("pl_01abcdefghijklmnop");
        store.create(&playlist).expect("create playlist");
        let first = sample_item("item_01abcdefghijklmnop", "185809", 1_753_137_600_100);
        let second = sample_item("item_02abcdefghijklmnop", "185809", 1_753_137_600_200);
        let result = store
            .append_items(&playlist.id, &[first.clone(), second.clone()])
            .expect("append duplicate source items");
        assert_eq!(result.playlist.item_count, 2);
        assert_eq!(result.playlist.updated_at_ms, second.added_at_ms);
        assert_eq!(result.items[0].position, 0);
        assert_eq!(result.items[1].position, 1);
        assert_eq!(result.items[0].source_ref, result.items[1].source_ref);
        assert_eq!(result.extensions["duplicates_preserved"], true);

        let page = store.items(&playlist.id, 1, 1).expect("read item page");
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, second.id);
        assert_eq!(page.pagination.total, Some(2));
        assert!(!page.pagination.has_more);
    }

    #[test]
    fn item_delete_targets_one_stable_id_and_reindexes_remaining_duplicates() {
        let store = MemoryUniPlaylistStore::default();
        let playlist = sample_playlist("pl_01abcdefghijklmnop");
        store.create(&playlist).expect("create playlist");
        let first = sample_item("item_01abcdefghijklmnop", "185809", 1_753_137_600_100);
        let second = sample_item("item_02abcdefghijklmnop", "185809", 1_753_137_600_200);
        let third = sample_item("item_03abcdefghijklmnop", "200001", 1_753_137_600_300);
        store
            .append_items(
                &playlist.id,
                &[first.clone(), second.clone(), third.clone()],
            )
            .expect("append items");

        let deleted = store
            .remove_item(&playlist.id, &first.id, 1_753_137_600_400)
            .expect("remove one duplicate occurrence");
        assert_eq!(deleted.item.id, first.id);
        assert_eq!(deleted.item.position, 0);
        assert_eq!(deleted.playlist.item_count, 2);
        assert_eq!(deleted.playlist.updated_at_ms, 1_753_137_600_400);
        assert_eq!(deleted.extensions["previous_item_count"], 3);

        let remaining = store.items(&playlist.id, 100, 0).expect("remaining items");
        assert_eq!(remaining.items[0].id, second.id);
        assert_eq!(remaining.items[0].source_ref, first.source_ref);
        assert_eq!(remaining.items[0].position, 0);
        assert_eq!(remaining.items[1].id, third.id);
        assert_eq!(remaining.items[1].position, 1);

        let error = store
            .remove_item(&playlist.id, "item_04abcdefghijklmnop", 1_753_137_600_500)
            .expect_err("reject missing item id");
        assert_eq!(error.code, ErrorCode::ResourceNotFound);
        assert_eq!(
            store.items(&playlist.id, 100, 0).expect("unchanged items"),
            remaining
        );
    }

    #[test]
    fn item_reorder_requires_the_exact_complete_id_set_and_is_atomic() {
        let store = MemoryUniPlaylistStore::default();
        let playlist = sample_playlist("pl_01abcdefghijklmnop");
        store.create(&playlist).expect("create playlist");
        let first = sample_item("item_01abcdefghijklmnop", "185809", 1_753_137_600_100);
        let second = sample_item("item_02abcdefghijklmnop", "185809", 1_753_137_600_200);
        let third = sample_item("item_03abcdefghijklmnop", "200001", 1_753_137_600_300);
        store
            .append_items(
                &playlist.id,
                &[first.clone(), second.clone(), third.clone()],
            )
            .expect("append items");
        let original = store.items(&playlist.id, 100, 0).expect("original order");

        for invalid_order in [
            vec![first.id.clone(), second.id.clone()],
            vec![first.id.clone(), first.id.clone(), third.id.clone()],
            vec![
                first.id.clone(),
                second.id.clone(),
                "item_04abcdefghijklmnop".to_owned(),
            ],
        ] {
            let error = store
                .reorder_items(&playlist.id, &invalid_order, 1_753_137_600_400)
                .expect_err("reject non-exact item order");
            assert_eq!(error.code, ErrorCode::InvalidRequest);
            assert_eq!(
                store.items(&playlist.id, 100, 0).expect("unchanged order"),
                original
            );
        }

        let order = vec![third.id.clone(), first.id.clone(), second.id.clone()];
        let reordered = store
            .reorder_items(&playlist.id, &order, 1_753_137_600_400)
            .expect("reorder all items");
        assert!(reordered.changed);
        assert_eq!(reordered.playlist.updated_at_ms, 1_753_137_600_400);
        assert_eq!(
            reordered
                .items
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            order.iter().map(String::as_str).collect::<Vec<_>>()
        );
        assert_eq!(
            reordered
                .items
                .iter()
                .map(|item| item.position)
                .collect::<Vec<_>>(),
            vec![0, 1, 2]
        );

        let unchanged = store
            .reorder_items(&playlist.id, &order, 1_753_137_600_500)
            .expect("accept explicit no-op order");
        assert!(!unchanged.changed);
        assert_eq!(unchanged.playlist.updated_at_ms, 1_753_137_600_400);
    }

    #[test]
    fn file_store_uses_one_reloadable_database_file() {
        let directory = TempDirectory::new();
        let path = directory.0.join("uni-playlists.json");
        let playlist = sample_playlist("pl_01abcdefghijklmnop");
        let store = FileUniPlaylistStore::open(&path).expect("open file store");
        store.create(&playlist).expect("persist playlist");
        let item = sample_item("item_01abcdefghijklmnop", "185809", 1_753_137_600_100);
        store
            .append_items(&playlist.id, std::slice::from_ref(&item))
            .expect("persist item");
        let second = sample_item("item_02abcdefghijklmnop", "200001", 1_753_137_600_200);
        let third = sample_item("item_03abcdefghijklmnop", "300001", 1_753_137_600_300);
        store
            .append_items(&playlist.id, &[second.clone(), third.clone()])
            .expect("persist more items");
        store
            .reorder_items(
                &playlist.id,
                &[third.id.clone(), item.id.clone(), second.id.clone()],
                1_753_137_600_400,
            )
            .expect("persist order");
        store
            .remove_item(&playlist.id, &item.id, 1_753_137_600_500)
            .expect("persist deletion");
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
            reopened
                .get(&playlist.id)
                .expect("reload playlist")
                .expect("stored playlist")
                .item_count,
            2
        );
        assert_eq!(
            reopened
                .items(&playlist.id, 25, 0)
                .expect("reload item page")
                .items,
            vec![
                UniPlaylistItem {
                    position: 0,
                    ..third
                },
                UniPlaylistItem {
                    position: 1,
                    ..second
                },
            ]
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
