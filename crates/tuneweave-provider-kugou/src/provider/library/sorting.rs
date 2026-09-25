use super::tracks::{RawPlaylistSnapshot, Target};
use super::*;
use crate::account::cloud::{ListPosition, SortAck};
use serde_json::Value;
use std::collections::{BTreeMap, VecDeque};
use tuneweave_core::{
    PlaylistOccurrenceOrderRequest, PlaylistOccurrenceOrderResult, PlaylistOrderRequest,
    PlaylistOrderResult, PlaylistTrackOccurrence, PlaylistTrackOrderRequest,
    PlaylistTrackOrderResult,
};

fn invalid() -> TuneWeaveError {
    kugou_invalid_request(
        "KuGou ordering requires a complete permutation of the selected playlist occurrences or library categories",
    )
}
fn unsupported() -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::CapabilityNotSupported,"KuGou catalogue ordering requires every occurrence to have a track reference; use track-occurrences for unresolved entries").with_platform(Platform::Kugou)
}
fn occurrence_id(id: &str, file_id: u64) -> String {
    format!(
        "entry:{}:{file_id}",
        id.strip_prefix("cloudlist:").unwrap_or(id)
    )
}
fn parse_occurrence(id: &str, playlist: &str) -> Result<u64> {
    let prefix = occurrence_id(playlist, 0);
    let prefix = prefix.strip_suffix('0').ok_or_else(invalid)?;
    id.strip_prefix(prefix)
        .filter(|s| crate::credential::valid_uid(s))
        .and_then(|s| s.parse().ok())
        .ok_or_else(invalid)
}
fn unconfirmed(mut e: TuneWeaveError, dispatched: bool) -> TuneWeaveError {
    if dispatched {
        let mut details = e.details.as_object().cloned().unwrap_or_default();
        details.extend([
            ("operation".into(), json!("playlist_order")),
            ("write_outcome".into(), json!("unconfirmed")),
            ("write_requests_dispatched".into(), json!(1)),
        ]);
        e = e.retryable(false).with_details(Value::Object(details));
    }
    e
}
fn list_version(s: &RawPlaylistSnapshot) -> Result<u64> {
    s.playlist
        .extensions
        .get("list_ver")
        .and_then(Value::as_u64)
        .ok_or_else(library_changed)
}
fn check_ack(ack: &SortAck, before: u64, after: u64) -> Result<()> {
    if after < before
        || ack.previous_version.is_some_and(|n| n != before)
        || ack.version.is_some_and(|n| n != after)
    {
        return Err(library_changed());
    }
    Ok(())
}
fn canonical(p: &Playlist, remove: &[&str]) -> Playlist {
    let mut p = p.clone();
    for field in remove {
        p.extensions.remove(*field);
    }
    p
}
fn same_library_counts(before: &LibrarySnapshot, after: &LibrarySnapshot) -> bool {
    let counts = |s: &LibrarySnapshot| {
        let mut e = s.version.extensions();
        e.remove("total_ver");
        e
    };
    before.deleted == after.deleted && counts(before) == counts(after)
}
fn verify_tracks(
    before: &RawPlaylistSnapshot,
    after: &RawPlaylistSnapshot,
    wanted: &[u64],
    ack: &SortAck,
) -> Result<()> {
    check_ack(ack, list_version(before)?, list_version(after)?)?;
    if before.library.version.total_ver > after.library.version.total_ver
        || !same_library_counts(&before.library, &after.library)
        || after
            .rows
            .iter()
            .map(|r| r.file_id)
            .ne(wanted.iter().copied())
        || after
            .rows
            .iter()
            .enumerate()
            .any(|(i, r)| r.sort != Some(i as u64))
    {
        return Err(library_changed());
    }
    let metadata = |s: &RawPlaylistSnapshot| {
        s.rows
            .iter()
            .map(|r| (r.file_id, r.metadata_fingerprint.clone()))
            .collect::<BTreeMap<_, _>>()
    };
    if metadata(before) != metadata(after) {
        return Err(library_changed());
    }
    let library = |s: &LibrarySnapshot| {
        s.items
            .iter()
            .map(|(kind, p)| {
                let fields = if p.id == before.playlist.id {
                    &["total_ver", "list_ver", "update_time"][..]
                } else {
                    &["total_ver"][..]
                };
                (p.id.clone(), (*kind, canonical(p, fields)))
            })
            .collect::<BTreeMap<_, _>>()
    };
    if library(&before.library) != library(&after.library) {
        return Err(library_changed());
    }
    Ok(())
}
fn result(snapshot: RawPlaylistSnapshot, dispatched: bool) -> PlaylistOccurrenceOrderResult {
    let ids = snapshot
        .rows
        .iter()
        .map(|r| occurrence_id(&snapshot.playlist.id, r.file_id))
        .collect();
    PlaylistOccurrenceOrderResult {
        playlist_ref: snapshot.playlist.resource_ref,
        occurrence_ids: ids,
        snapshot_id: snapshot.snapshot_id,
        extensions: Extensions::from([
            ("backend".into(), json!("native_cloud_order")),
            ("complete_read".into(), json!(true)),
            ("confirmed".into(), json!(true)),
            ("atomic".into(), json!(false)),
            (
                "write_requests_dispatched".into(),
                json!(usize::from(dispatched)),
            ),
        ]),
    }
}
enum Order<'a> {
    Occurrences(&'a PlaylistOccurrenceOrderRequest),
    Tracks(&'a PlaylistTrackOrderRequest),
}
impl KugouProvider {
    pub(in crate::provider) async fn native_track_occurrences(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<PlaylistTrackOccurrence>> {
        let (uid, _, _) = parse_reference(id)?;
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(invalid());
        }
        let mut read = self
            .begin_native_read(request.account.as_deref().unwrap_or("default"), Some(uid))
            .await?;
        let result = async {
            let snapshot = self
                .read_native_occurrences(&mut read, Target::Playlist(id), false)
                .await?;
            let total = snapshot.rows.len() as u64;
            let items = snapshot
                .rows
                .into_iter()
                .enumerate()
                .skip(request.offset as usize)
                .take(request.limit as usize)
                .map(|(position, mut row)| {
                    if let Some(track) = row.track.as_mut() {
                        track
                            .extensions
                            .insert("playlist_position".into(), json!(position));
                    }
                    PlaylistTrackOccurrence {
                        id: occurrence_id(id, row.file_id),
                        position: position as u64,
                        track: row.track,
                        extensions: Extensions::from([("file_id".into(), json!(row.file_id))]),
                    }
                })
                .collect::<Vec<_>>();
            let end = request.offset + items.len() as u32;
            let more = u64::from(end) < total;
            let mut extensions = snapshot.playlist.extensions;
            extensions.extend([
                ("source_snapshot_id".into(), json!(snapshot.snapshot_id)),
                ("playlist_ref".into(), json!(snapshot.playlist.resource_ref)),
                ("complete_read".into(), json!(true)),
                ("ordering".into(), json!(snapshot.ordering)),
                ("upstream_pages_fetched".into(), json!(snapshot.pages)),
                ("backend".into(), json!("native_cloudlist_occurrences_v3")),
            ]);
            Ok(Page {
                items,
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(total),
                    has_more: more,
                    next_offset: more.then_some(end),
                    extensions,
                },
            })
        }
        .await;
        self.finish_account_read(read, result)
    }
    pub(in crate::provider) async fn native_reorder_occurrences(
        &self,
        id: &str,
        request: &PlaylistOccurrenceOrderRequest,
    ) -> Result<PlaylistOccurrenceOrderResult> {
        let digest = request
            .snapshot_id
            .strip_prefix("kugou-cloudlist-raw-")
            .ok_or_else(invalid)?;
        if digest.len() != 32
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid());
        }
        self.order_native_tracks(id, Order::Occurrences(request), request.account.as_deref())
            .await
    }
    pub(in crate::provider) async fn native_reorder_tracks(
        &self,
        id: &str,
        request: &PlaylistTrackOrderRequest,
    ) -> Result<PlaylistTrackOrderResult> {
        let sorted = self
            .order_native_tracks(id, Order::Tracks(request), request.account.as_deref())
            .await?;
        Ok(PlaylistTrackOrderResult {
            playlist_ref: sorted.playlist_ref,
            track_refs: request.track_refs.clone(),
            snapshot_id: Some(sorted.snapshot_id),
            extensions: sorted.extensions,
        })
    }
    async fn order_native_tracks(
        &self,
        id: &str,
        order: Order<'_>,
        account: Option<&str>,
    ) -> Result<PlaylistOccurrenceOrderResult> {
        let (uid, kind, listid) = parse_reference(id)?;
        let length = match &order {
            Order::Occurrences(r) => {
                let ids = r
                    .occurrence_ids
                    .iter()
                    .map(|s| parse_occurrence(s, id))
                    .collect::<Result<BTreeSet<_>>>()?;
                if ids.len() != r.occurrence_ids.len() {
                    return Err(invalid());
                }
                ids.len()
            }
            Order::Tracks(r) => {
                for reference in &r.track_refs {
                    if reference.platform() != Platform::Kugou {
                        return Err(invalid());
                    }
                    parse_album_audio_id(reference.id())?;
                }
                r.track_refs.len()
            }
        };
        if length
            > crate::account::library::tracks::TRACK_PAGE_SIZE
                * crate::account::library::tracks::MAX_TRACK_PAGES as usize
        {
            return Err(invalid());
        }
        let mut read = self
            .begin_native_read(account.unwrap_or("default"), Some(uid))
            .await?;
        let mut dispatched = false;
        let operation = async {
            let before = self
                .read_native_occurrences(&mut read, Target::Playlist(id), false)
                .await?;
            if length != before.rows.len() {
                return Err(invalid());
            }
            let wanted = match order {
                Order::Occurrences(r) => {
                    if before.snapshot_id != r.snapshot_id {
                        return Err(library_changed());
                    }
                    r.occurrence_ids
                        .iter()
                        .map(|s| parse_occurrence(s, id))
                        .collect::<Result<Vec<_>>>()?
                }
                Order::Tracks(r) => {
                    let mut occurrences: BTreeMap<&str, VecDeque<u64>> = BTreeMap::new();
                    for row in &before.rows {
                        let track = row.track.as_ref().ok_or_else(unsupported)?;
                        occurrences
                            .entry(&track.id)
                            .or_default()
                            .push_back(row.file_id);
                    }
                    r.track_refs
                        .iter()
                        .map(|r| {
                            occurrences
                                .get_mut(r.id())
                                .and_then(VecDeque::pop_front)
                                .ok_or_else(invalid)
                        })
                        .collect::<Result<Vec<_>>>()?
                }
            };
            if wanted.iter().copied().collect::<BTreeSet<_>>()
                != before
                    .rows
                    .iter()
                    .map(|r| r.file_id)
                    .collect::<BTreeSet<_>>()
            {
                return Err(invalid());
            }
            if before
                .rows
                .iter()
                .map(|r| r.file_id)
                .eq(wanted.iter().copied())
            {
                return Ok(result(before, false));
            }
            self.check_account_read(&mut read)?;
            let version = list_version(&before)?;
            dispatched = true;
            let ack = self
                .client
                .native_reorder_files(read.session()?, listid, kind, version, &wanted)
                .await?;
            self.check_account_read(&mut read)?;
            let after = self
                .read_native_occurrences(&mut read, Target::Playlist(id), false)
                .await?;
            verify_tracks(&before, &after, &wanted, &ack)?;
            Ok(result(after, true))
        }
        .await;
        self.finish_account_read(read, operation)
            .map_err(|e| unconfirmed(e, dispatched))
    }
    pub(in crate::provider) async fn native_reorder_library(
        &self,
        request: &PlaylistOrderRequest,
    ) -> Result<PlaylistOrderResult> {
        if request.playlist_refs.is_empty()
            || request.playlist_refs.len() > PAGE_SIZE * MAX_PAGES as usize
        {
            return Err(invalid());
        }
        let mut uid = None;
        let mut selected = BTreeSet::new();
        let mut unique = BTreeSet::new();
        let mut desired = BTreeMap::<u8, Vec<String>>::new();
        for reference in &request.playlist_refs {
            if reference.platform() != Platform::Kugou || !unique.insert(reference.id()) {
                return Err(invalid());
            }
            let (owner, kind, _) = parse_reference(reference.id())?;
            if uid.is_some_and(|u| u != owner) {
                return Err(invalid());
            }
            uid = Some(owner);
            selected.insert(kind);
            desired.entry(kind).or_default().push(reference.id().into());
        }
        let mut read = self
            .begin_native_read(request.account.as_deref().unwrap_or("default"), uid)
            .await?;
        let mut dispatched = false;
        let operation = async {
            let before = self.read_native_library(&mut read).await?;
            let present = before
                .items
                .iter()
                .filter(|(k, _)| selected.contains(k))
                .map(|(_, p)| p.id.as_str())
                .collect::<BTreeSet<_>>();
            if unique != present {
                return Err(invalid());
            }
            let noop = desired
                .iter()
                .all(|(k, w)| category_order(&before, *k).as_ref() == Some(w));
            let after = if noop {
                before
            } else {
                let positions = desired
                    .iter()
                    .flat_map(|(k, ids)| {
                        ids.iter().enumerate().map(move |(sort, id)| (k, sort, id))
                    })
                    .map(|(k, sort, id)| {
                        Ok(ListPosition {
                            listid: parse_reference(id)?.2,
                            kind: *k,
                            sort: sort as u64,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?;
                self.check_account_read(&mut read)?;
                dispatched = true;
                let ack = self
                    .client
                    .native_reorder_lists(read.session()?, before.version.total_ver, &positions)
                    .await?;
                self.check_account_read(&mut read)?;
                let after = self.read_native_library(&mut read).await?;
                check_ack(&ack, before.version.total_ver, after.version.total_ver)?;
                let metadata = |s: &LibrarySnapshot| {
                    s.items
                        .iter()
                        .map(|(k, p)| {
                            let fields = if selected.contains(k) {
                                &["total_ver", "sort", "update_time"][..]
                            } else {
                                &["total_ver"][..]
                            };
                            (p.id.clone(), (*k, canonical(p, fields)))
                        })
                        .collect::<BTreeMap<_, _>>()
                };
                if !same_library_counts(&before, &after) || metadata(&before) != metadata(&after) {
                    return Err(library_changed());
                }
                for (k, w) in &desired {
                    if category_order(&after, *k).as_ref() != Some(w)
                        || after
                            .items
                            .iter()
                            .filter(|(kind, _)| kind == k)
                            .any(|(_, p)| {
                                p.extensions.get("sort").and_then(Value::as_u64)
                                    != w.iter().position(|id| id == &p.id).map(|n| n as u64)
                            })
                    {
                        return Err(library_changed());
                    }
                }
                for kind in [0, 1] {
                    if !selected.contains(&kind) {
                        let raw = |s: &LibrarySnapshot| {
                            s.items
                                .iter()
                                .filter(|(k, _)| *k == kind)
                                .map(|(_, p)| p.id.as_str().to_owned())
                                .collect::<Vec<_>>()
                        };
                        if category_order(&before, kind) != category_order(&after, kind)
                            || (category_order(&before, kind).is_none()
                                && raw(&before) != raw(&after))
                        {
                            return Err(library_changed());
                        }
                    }
                }
                after
            };
            let mut extensions = after.version.extensions();
            extensions.extend([
                ("backend".into(), json!("native_cloud_order")),
                ("confirmed".into(), json!(true)),
                ("complete_read".into(), json!(true)),
                ("atomic".into(), json!(false)),
                (
                    "write_requests_dispatched".into(),
                    json!(usize::from(dispatched)),
                ),
            ]);
            Ok(PlaylistOrderResult {
                playlist_refs: request.playlist_refs.clone(),
                extensions,
            })
        }
        .await;
        self.finish_account_read(read, operation)
            .map_err(|e| unconfirmed(e, dispatched))
    }
}
fn category_order(snapshot: &LibrarySnapshot, kind: u8) -> Option<Vec<String>> {
    let mut rows = snapshot
        .items
        .iter()
        .filter(|(k, _)| *k == kind)
        .map(|(_, p)| Some((p.extensions.get("sort")?.as_u64()?, p.id.clone())))
        .collect::<Option<Vec<_>>>()?;
    let mut seen = BTreeSet::new();
    if rows.iter().any(|(sort, _)| !seen.insert(*sort)) {
        return None;
    }
    rows.sort_by_key(|(sort, _)| *sort);
    Some(rows.into_iter().map(|(_, id)| id).collect())
}
#[cfg(test)]
mod tests;
