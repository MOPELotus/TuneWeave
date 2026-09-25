use super::{CollectedAlbum, CollectionScan, MAX_PAGES};
use crate::credential::{error, validate_uid};
use crate::provider::{account_read::Read, purchased_album_source::reject_reflection, *};
use sha1::{Digest, Sha1};
use std::time::Duration;
use tuneweave_core::{ErrorCode, PlaylistPlayableItem, ResourceRef};

const SOURCE_TYPE: &str = "favorite_albums";
const MAX_ALBUMS: usize = 128;
const MAX_TRACKS: usize = 10_000;
const MAX_SNAPSHOT_BYTES: usize = 16 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(120);

struct SourceSnapshot {
    tracks: Vec<Track>,
    extensions: Extensions,
}

impl MiguProvider {
    pub(in crate::provider) async fn favorite_albums_source(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        let snapshot = self.favorite_album_snapshot(id, account, DEADLINE).await?;
        Ok(Playlist {
            resource_ref: ResourceRef::new(Platform::Migu, id)
                .map_err(|_| migu_invalid_request("Invalid Migu favorite album source identity"))?,
            platform: Platform::Migu,
            id: id.into(),
            name: "Favorite album tracks".into(),
            description: String::new(),
            cover_url: None,
            creator: None,
            track_count: Some(snapshot.tracks.len() as u64),
            tags: Vec::new(),
            subscribed: None,
            created_at: None,
            updated_at: None,
            extensions: snapshot.extensions,
        })
    }

    pub(in crate::provider) async fn favorite_albums_source_items(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<PlaylistPlayableItem>> {
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(migu_invalid_request(
                "Migu favorite album source pagination is invalid",
            ));
        }
        let snapshot = self
            .favorite_album_snapshot(id, request.account.as_deref(), DEADLINE)
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
                has_more,
                next_offset: has_more.then_some(end),
                extensions: snapshot.extensions,
            },
        })
    }

    async fn favorite_album_snapshot(
        &self,
        id: &str,
        account: Option<&str>,
        deadline: Duration,
    ) -> Result<SourceSnapshot> {
        validate_uid(id)?;
        let mut read = Read::new(self, account)?;
        if read.current.user_id() != id {
            return Err(error(
                ErrorCode::PermissionDenied,
                "Migu favorite album source belongs to another account",
            ));
        }
        let result = tokio::time::timeout(deadline, async {
            read.start().await?;
            let before = self.favorite_album_directory(&mut read).await?;
            let directory = directory_material(&before)?;
            let mut tracks = Vec::new();
            let mut catalogue_pages = 0_u32;
            for (album_position, album) in before.items.iter().enumerate() {
                let (resource_type, album_id) = album.identity();
                // The mixed collection rejects duplicate typed identities. The
                // same numeric ID in ordinary/digital namespaces remains distinct.
                let digital = matches!(album, CollectedAlbum::Digital(_));
                let (items, pages) = self
                    .complete_album_tracks(album_id, digital, Some(&read))
                    .await?;
                catalogue_pages += pages;
                if items.len() > MAX_TRACKS.saturating_sub(tracks.len()) {
                    return Err(migu_upstream_error(
                        "Migu favorite album source exceeded its track limit",
                    ));
                }
                for (album_track_position, mut track) in items.into_iter().enumerate() {
                    read.mark(&mut track.extensions);
                    track.playable = None;
                    track.extensions.extend(Extensions::from([
                        ("source_type".into(), json!(SOURCE_TYPE)),
                        ("source_position".into(), json!(tracks.len())),
                        ("favorite_album_id".into(), json!(album_id)),
                        ("favorite_album_resource_type".into(), json!(resource_type)),
                        (
                            "favorite_album_kind".into(),
                            json!(if digital { "digital_album" } else { "album" }),
                        ),
                        ("favorite_album_position".into(), json!(album_position)),
                        (
                            "favorite_album_track_position".into(),
                            json!(album_track_position),
                        ),
                    ]));
                    tracks.push(track);
                }
            }
            let after = self.favorite_album_directory(&mut read).await?;
            if directory != directory_material(&after)? {
                return Err(error(
                    ErrorCode::Conflict,
                    "Migu favorite album directory changed while expanding its tracks",
                ));
            }
            let material = json!((SOURCE_TYPE, id, directory, &tracks));
            // Check even tokens first learned during the final directory read.
            reject_reflection(&material, &read.secrets)?;
            let bytes = serde_json::to_vec(&material).map_err(|_| {
                migu_upstream_error("Migu favorite album source could not be serialized")
            })?;
            if bytes.len() > MAX_SNAPSHOT_BYTES {
                return Err(migu_upstream_error(
                    "Migu favorite album source exceeded its snapshot size limit",
                ));
            }
            let extensions = Extensions::from([
                (
                    "backend".into(),
                    json!("pc_album_collections_public_tracks"),
                ),
                ("source_type".into(), json!(SOURCE_TYPE)),
                ("source_user_id".into(), json!(id)),
                ("catalogue_scope".into(), json!("public")),
                ("complete_read".into(), json!(true)),
                (
                    "consistency".into(),
                    json!("two_collection_reads_catalogue_once"),
                ),
                ("album_count".into(), json!(before.items.len())),
                ("catalogue_pages_fetched".into(), json!(catalogue_pages)),
                (
                    "source_snapshot_id".into(),
                    json!(format!(
                        "migu-favorite-album-tracks-{}",
                        hex::encode(Sha1::digest(bytes))
                    )),
                ),
            ]);
            // This digest compares complete local observations across source/page
            // calls. It is not an upstream atomic snapshot or a playback grant.
            Ok(SourceSnapshot { tracks, extensions })
        })
        .await
        .map_err(|_| {
            error(
                ErrorCode::UpstreamTimeout,
                "Migu favorite album source exceeded its total deadline",
            )
        })
        .and_then(|result| result);
        read.finish(result)
    }

    async fn favorite_album_directory(&self, read: &mut Read<'_>) -> Result<CollectionScan> {
        let mut scan = CollectionScan::default();
        for page in 1..=MAX_PAGES {
            read.check()?;
            let response = self
                .client
                .account_album_collections(page, read.current.token(), read.current.user_id())
                .await;
            let finished = scan.push(read.accept(response).await??)?;
            if scan.items.len() > MAX_ALBUMS {
                return Err(migu_upstream_error(
                    "Migu favorite album source exceeded its album limit",
                ));
            }
            if finished {
                return Ok(scan);
            }
        }
        Err(migu_upstream_error(
            "Migu favorite album directory exceeded its complete-read page limit",
        ))
    }
}

fn directory_material(scan: &CollectionScan) -> Result<serde_json::Value> {
    let items = scan
        .items
        .iter()
        .map(|item| match item {
            CollectedAlbum::Ordinary(album) => serde_json::to_value(("2003", album)),
            CollectedAlbum::Digital(album) => serde_json::to_value(("5", album)),
        })
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| {
            migu_upstream_error("Migu favorite album directory could not be serialized")
        })?;
    Ok(json!((scan.expected_total, items)))
}

#[cfg(test)]
mod tests;
