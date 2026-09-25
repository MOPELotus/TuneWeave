use super::*;
use crate::account::library::tracks::{MAX_TRACK_PAGES, TRACK_PAGE_SIZE, TrackRow};
use md5::{Digest, Md5};
use serde_json::Value;

pub(super) struct PlaylistSnapshot {
    pub(super) playlist: Playlist,
    pub(super) tracks: Vec<Track>,
    pages: u32,
    ordering: &'static str,
}

pub(super) struct RawPlaylistSnapshot {
    pub(super) library: LibrarySnapshot,
    pub(super) playlist: Playlist,
    pub(super) rows: Vec<TrackRow>,
    pub(super) pages: u32,
    pub(super) ordering: &'static str,
    pub(super) snapshot_id: String,
}

impl PlaylistSnapshot {
    pub(super) fn into_page(self, request: &PageRequest) -> Page<Track> {
        let total = self.tracks.len() as u64;
        let items: Vec<_> = self
            .tracks
            .into_iter()
            .skip(request.offset as usize)
            .take(request.limit as usize)
            .collect();
        let end = request.offset + items.len() as u32;
        let has_more = u64::from(end) < total;
        let mut extensions = self.playlist.extensions;
        extensions.extend([
            ("backend".into(), json!("native_cloudlist_tracks_v3")),
            ("playlist_ref".into(), json!(self.playlist.id)),
            ("upstream_page_size".into(), json!(TRACK_PAGE_SIZE)),
            ("upstream_pages_fetched".into(), json!(self.pages)),
            ("ordering".into(), json!(self.ordering)),
            ("complete_read".into(), json!(true)),
        ]);
        Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(total),
                next_offset: has_more.then_some(end),
                has_more,
                extensions,
            },
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Target<'a> {
    Playlist(&'a str),
    Favorites,
}

impl KugouProvider {
    pub(in crate::provider) async fn native_playlist_tracks(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<Track>> {
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kugou_invalid_request(
                "KuGou native playlist pagination is invalid",
            ));
        }
        let snapshot = self
            .native_playlist_snapshot(id, request.account.as_deref())
            .await?;
        Ok(snapshot.into_page(request))
    }

    pub(super) async fn native_playlist_snapshot(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<PlaylistSnapshot> {
        let (uid, _, _) = parse_reference(id)?;
        let mut read = self
            .begin_native_read(account.unwrap_or("default"), Some(uid))
            .await?;
        let result = self
            .read_native_playlist(&mut read, Target::Playlist(id))
            .await;
        self.finish_account_read(read, result)
    }

    pub(super) async fn read_native_playlist(
        &self,
        read: &mut session::AccountRead,
        target: Target<'_>,
    ) -> Result<PlaylistSnapshot> {
        let RawPlaylistSnapshot {
            mut playlist,
            rows,
            pages,
            ordering,
            ..
        } = self.read_native_occurrences(read, target, true).await?;
        let tracks = rows
            .into_iter()
            .enumerate()
            .map(|(position, row)| {
                let mut track = row
                    .track
                    .ok_or_else(|| track_error("KuGou track occurrence was unresolved"))?;
                track
                    .extensions
                    .insert("playlist_position".into(), json!(position));
                Ok(track)
            })
            .collect::<Result<Vec<_>>>()?;
        playlist.track_count = Some(tracks.len() as u64);
        playlist
            .extensions
            .insert("track_list_count".into(), json!(tracks.len()));
        playlist
            .extensions
            .insert("track_ordering".into(), json!(ordering));
        // A content revision for change detection only, not an authentication signature.
        // Tokens, account aliases and login generations never enter this input.
        let bytes = serde_json::to_vec(&(&playlist, &tracks))
            .map_err(|_| track_error("KuGou playlist revision could not be represented"))?;
        playlist.extensions.insert(
            "source_snapshot_id".into(),
            json!(format!("kugou-cloudlist-v3-{:x}", Md5::digest(bytes))),
        );
        Ok(PlaylistSnapshot {
            playlist,
            tracks,
            pages,
            ordering,
        })
    }
    pub(super) async fn read_native_occurrences(
        &self,
        read: &mut session::AccountRead,
        target: Target<'_>,
        require_tracks: bool,
    ) -> Result<RawPlaylistSnapshot> {
        let library = self.read_native_library(read).await?;
        let id = match target {
            Target::Playlist(id) => id.to_owned(),
            Target::Favorites => favorite_in_library(&library)?.id.clone(),
        };
        let (uid, kind, list_id) = parse_reference(&id)?;
        if uid != read.session()?.user_id {
            return Err(library_changed());
        }
        let playlist = find_playlist(&library, kind, list_id)?.clone();
        let version = playlist
            .extensions
            .get("list_ver")
            .and_then(Value::as_u64)
            .ok_or_else(|| track_error("KuGou native playlist omitted its version"))?;
        let mut total = None;
        let mut rows = Vec::new();
        let mut seen = BTreeSet::new();
        let mut pages = 0;
        for page in 1..=MAX_TRACK_PAGES {
            let response = self
                .client
                .native_library_tracks_page(read.session()?, list_id, kind, page)
                .await?;
            self.check_account_read(read)?;
            if response.version != version || total.is_some_and(|n| n != response.total) {
                return Err(library_changed());
            }
            let expected = response.total;
            if expected > u64::from(MAX_TRACK_PAGES) * TRACK_PAGE_SIZE as u64 {
                return Err(track_error(
                    "KuGou native playlist exceeded its complete-read budget",
                ));
            }
            total = Some(expected);
            let returned = response.rows.len();
            for mut row in response.rows {
                if !seen.insert(row.file_id) {
                    return Err(library_changed());
                }
                if let Some(track) = row.track.as_mut() {
                    track.extensions.extend([
                        ("upstream_position".into(), json!(rows.len())),
                        ("playlist_ref".into(), json!(id)),
                    ]);
                }
                rows.push(row);
            }
            pages = page;
            let loaded = rows.len() as u64;
            if loaded > expected || (loaded < expected && returned != TRACK_PAGE_SIZE) {
                return Err(track_error(
                    "KuGou native playlist returned incomplete or excessive rows",
                ));
            }
            if loaded == expected {
                break;
            }
        }
        if total != Some(rows.len() as u64) {
            return Err(track_error(
                "KuGou native playlist did not reach its declared end",
            ));
        }
        let ordering = order_rows(&mut rows)?;
        let unresolved = rows.iter().filter(|r| r.track.is_none()).count();
        if require_tracks && unresolved > 0 {
            // The current public model requires a catalogue reference. Never drop an
            // unresolved occurrence, fabricate an ID or claim its truncated list is complete.
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "KuGou native playlist contains occurrences without resolvable track metadata",
            )
            .with_platform(Platform::Kugou)
            .with_details(json!({"unresolved_occurrences":unresolved,"raw_count":rows.len()})));
        }
        let after = self.read_native_library(read).await?;
        if (matches!(target, Target::Favorites)
            && favorite_in_library(&after).map(|p| p.id.as_str()).ok() != Some(id.as_str()))
            || library.version != after.version
            || find_playlist(&after, kind, list_id).ok() != Some(&playlist)
        {
            return Err(library_changed());
        }
        let identities = rows
            .iter()
            .map(|row| (row.file_id, row.sort, &row.metadata_fingerprint))
            .collect::<Vec<_>>();
        let revision = serde_json::to_vec(&(&playlist, identities))
            .map_err(|_| track_error("KuGou raw playlist revision could not be represented"))?;
        let snapshot_id = format!("kugou-cloudlist-raw-{:x}", Md5::digest(revision));
        Ok(RawPlaylistSnapshot {
            library: after,
            playlist,
            rows,
            pages,
            ordering,
            snapshot_id,
        })
    }
}

pub(super) fn favorite_in_library(library: &LibrarySnapshot) -> Result<&Playlist> {
    let mut favorites = library.items.iter().filter_map(|(kind, playlist)| {
        (*kind == 0 && playlist.extensions.get("is_def") == Some(&json!(2))).then_some(playlist)
    });
    let favorite = favorites.next().ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::ResourceNotFound,
            "KuGou selected account did not expose its liked-tracks playlist",
        )
        .with_platform(Platform::Kugou)
    })?;
    if favorites.next().is_some() {
        return Err(library_changed());
    }
    Ok(favorite)
}

fn find_playlist(library: &LibrarySnapshot, kind: u8, list_id: u64) -> Result<&Playlist> {
    library
        .items
        .iter()
        .find_map(|(k, playlist)| {
            (*k == kind && playlist.extensions.get("list_id") == Some(&json!(list_id)))
                .then_some(playlist)
        })
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::ResourceNotFound,
                "KuGou playlist is absent from the selected account library",
            )
            .with_platform(Platform::Kugou)
        })
}

fn order_rows(rows: &mut [TrackRow]) -> Result<&'static str> {
    let sorted = rows.iter().filter(|r| r.sort.is_some()).count();
    if sorted == 0 {
        return Ok("upstream_order");
    }
    if sorted != rows.len() {
        return Err(track_error(
            "KuGou native playlist omitted some ordering positions",
        ));
    }
    // Stable global ordering retains repeated songs and ties in their original row order.
    rows.sort_by_key(|r| r.sort);
    Ok("sort_ascending")
}
fn track_error(message: &'static str) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message).with_platform(Platform::Kugou)
}

#[cfg(test)]
mod tests;
