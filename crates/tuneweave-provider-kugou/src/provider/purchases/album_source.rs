//! Purchased catalogue albums expanded into ordered public tracks, without a rights grant.
use super::*;
use std::{collections::BTreeMap, time::Duration};
use tuneweave_core::{PlaylistPlayableItem, ResourceRef};

const SOURCE: &str = "purchased_albums";
#[derive(Clone, Copy)]
struct Limits {
    albums: usize,
    tracks: usize,
    bytes: usize,
    time: Duration,
}
const LIMITS: Limits = Limits {
    albums: 128,
    tracks: 10_000,
    bytes: 16 * 1024 * 1024,
    time: Duration::from_secs(120),
};

#[derive(Debug)]
struct Snapshot {
    tracks: Vec<Track>,
    extensions: Extensions,
}

fn timed_out() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamTimeout,
        "KuGou purchased albums exceeded the complete-source deadline",
    )
    .with_platform(Platform::Kugou)
}

impl KugouProvider {
    pub(in crate::provider) async fn purchased_albums_source(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        let snapshot = self.purchased_album_snapshot(id, account, LIMITS).await?;
        Ok(Playlist {
            resource_ref: ResourceRef::new(Platform::Kugou, id).map_err(|_| changed())?,
            platform: Platform::Kugou,
            id: id.to_owned(),
            name: "Purchased album tracks".into(),
            description: String::new(),
            cover_url: None,
            creator: None,
            track_count: Some(snapshot.tracks.len() as u64),
            tags: vec![],
            subscribed: None,
            created_at: None,
            updated_at: None,
            extensions: snapshot.extensions,
        })
    }

    pub(in crate::provider) async fn purchased_albums_source_items(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<PlaylistPlayableItem>> {
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kugou_invalid_request(
                "KuGou purchased album source pagination is invalid",
            ));
        }
        let snapshot = self
            .purchased_album_snapshot(id, request.account.as_deref(), LIMITS)
            .await?;
        let total = snapshot.tracks.len() as u64;
        let items: Vec<_> = snapshot
            .tracks
            .into_iter()
            .skip(request.offset as usize)
            .take(request.limit as usize)
            .map(PlaylistPlayableItem::Track)
            .collect();
        let end = request.offset + items.len() as u32;
        let has_more = u64::from(end) < total;
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(total),
                next_offset: has_more.then_some(end),
                has_more,
                extensions: snapshot.extensions,
            },
        })
    }

    async fn purchased_album_snapshot(
        &self,
        id: &str,
        account: Option<&str>,
        limits: Limits,
    ) -> Result<Snapshot> {
        let deadline = tokio::time::Instant::now() + limits.time;
        // Preserve the existing exchange/profile transaction. Its two requests
        // are individually bounded; charge their time to the source deadline.
        let mut read = self
            .begin_native_read(account.unwrap_or("default"), Some(id))
            .await?;
        let result = if tokio::time::Instant::now() >= deadline {
            Err(timed_out())
        } else {
            tokio::time::timeout_at(deadline, async {
                let mut remaining = limits.bytes;
                let (albums, purchase_pages) = self
                    .scan_purchases_bounded(&mut read, Kind::Albums, limits.albums, &mut remaining)
                    .await?;
                let unresolved = albums
                    .iter()
                    .filter(|item| item.catalogue_id().is_none())
                    .count();
                if unresolved != 0 {
                    return Err(TuneWeaveError::new(
                        ErrorCode::UpstreamError,
                        "KuGou purchased albums contain unresolved catalogue records",
                    )
                    .with_platform(Platform::Kugou)
                    .with_details(json!({"source_type":SOURCE,"unresolved_entries":unresolved})));
                }
                let mut catalogue = BTreeMap::new();
                let mut tracks = Vec::new();
                let mut catalogue_pages = 0_u32;
                for (album_position, purchase) in albums.iter().enumerate() {
                    self.check_account_read(&mut read)?;
                    let album_id = purchase.catalogue_id().ok_or_else(changed)?;
                    if let std::collections::btree_map::Entry::Vacant(entry) =
                        catalogue.entry(album_id.to_owned())
                    {
                        let parsed_id = album_id.parse::<u64>().map_err(|_| changed())?;
                        let album = self
                            .client
                            .complete_album_tracks_observed(parsed_id, |bytes| {
                                self.check_account_read(&mut read)?;
                                charge_bytes(&mut remaining, bytes)
                            })
                            .await?;
                        catalogue_pages += album.pages;
                        entry.insert(album.tracks);
                    }
                    let items = &catalogue[album_id];
                    if items.len() > limits.tracks.saturating_sub(tracks.len()) {
                        return Err(budget_exceeded());
                    }
                    // A second goods record still contributes a second occurrence.
                    // buy_total is an aggregate quantity, not a catalogue repeat count.
                    for (album_track_position, mut track) in items.iter().cloned().enumerate() {
                        track.playable = None;
                        track.extensions.extend(Extensions::from([
                            ("source_type".into(), json!(SOURCE)),
                            ("source_position".into(), json!(tracks.len())),
                            ("purchase_album_id".into(), json!(album_id)),
                            ("purchase_album_position".into(), json!(album_position)),
                            (
                                "purchase_album_track_position".into(),
                                json!(album_track_position),
                            ),
                        ]));
                        tracks.push(track);
                    }
                }
                let (confirmation, confirmation_pages) = self
                    .scan_purchases_bounded(&mut read, Kind::Albums, limits.albums, &mut remaining)
                    .await?;
                if albums != confirmation || purchase_pages != confirmation_pages {
                    return Err(changed());
                }
                let bytes =
                    serde_json::to_vec(&(SOURCE, id, &albums, &tracks)).map_err(|_| changed())?;
                let extensions = Extensions::from([
                    (
                        "backend".into(),
                        json!("native_purchased_albums_public_tracks"),
                    ),
                    ("source_type".into(), json!(SOURCE)),
                    ("source_user_id".into(), json!(id)),
                    ("library_owner_id".into(), json!(id)),
                    ("purchase_kind".into(), json!("albums")),
                    ("catalogue_scope".into(), json!("public")),
                    ("complete_read".into(), json!(true)),
                    (
                        "consistency".into(),
                        json!("two_complete_purchase_reads_catalogue_once"),
                    ),
                    ("album_count".into(), json!(albums.len())),
                    ("unique_album_count".into(), json!(catalogue.len())),
                    ("unresolved_entries".into(), json!(0)),
                    ("upstream_page_size".into(), json!(Kind::Albums.page_size())),
                    (
                        "purchase_pages_fetched".into(),
                        json!(purchase_pages + confirmation_pages),
                    ),
                    ("catalogue_pages_fetched".into(), json!(catalogue_pages)),
                    (
                        "catalogue_requests".into(),
                        json!(catalogue_pages as usize + catalogue.len()),
                    ),
                    ("response_bytes".into(), json!(limits.bytes - remaining)),
                    (
                        "source_snapshot_id".into(),
                        json!(format!(
                            "kugou-purchased-album-tracks-{:x}",
                            Md5::digest(bytes)
                        )),
                    ),
                ]);
                Ok(Snapshot { tracks, extensions })
            })
            .await
            .map_err(|_| timed_out())
            .and_then(|result| result)
        };
        // Timeout/errors still retain valid caller rotations and protect a new
        // login installed under the same alias. No catalogue work is detached.
        self.finish_account_read(read, result)
    }
}

#[cfg(test)]
mod tests;
