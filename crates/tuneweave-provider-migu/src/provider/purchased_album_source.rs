use super::account_read::Read;
use super::*;
use crate::credential::{error, validate_uid};
use sha1::{Digest, Sha1};
use std::{collections::BTreeMap, time::Duration};
use tuneweave_core::{ErrorCode, PlaylistPlayableItem, ResourceRef};

const SOURCE_TYPE: &str = "purchased_albums";
const MAX_ALBUMS: usize = 128;
const MAX_TRACKS: usize = 10_000;
const MAX_PURCHASE_BYTES: u64 = 16 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(120);

struct Snapshot {
    tracks: Vec<Track>,
    extensions: Extensions,
}

impl MiguProvider {
    pub(super) async fn purchased_albums_source(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        let snapshot = self.purchased_album_snapshot(id, account, DEADLINE).await?;
        let resource_ref = ResourceRef::new(Platform::Migu, id)
            .map_err(|_| migu_invalid_request("Invalid Migu purchase source identity"))?;
        Ok(Playlist {
            resource_ref,
            platform: Platform::Migu,
            id: id.to_owned(),
            name: "Purchased album tracks".into(),
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

    pub(super) async fn purchased_albums_source_items(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<PlaylistPlayableItem>> {
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(migu_invalid_request(
                "Migu purchase source pagination is invalid",
            ));
        }
        let snapshot = self
            .purchased_album_snapshot(id, request.account.as_deref(), DEADLINE)
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

    async fn purchased_album_snapshot(
        &self,
        id: &str,
        account: Option<&str>,
        deadline: Duration,
    ) -> Result<Snapshot> {
        validate_uid(id)?;
        let mut read = Read::new(self, account)?;
        if read.current.user_id() != id {
            return Err(error(
                ErrorCode::PermissionDenied,
                "Migu purchased album source belongs to another account",
            ));
        }
        let result = tokio::time::timeout(deadline, async {
            read.start().await?;
            let mut remaining = MAX_PURCHASE_BYTES;
            let (albums, sizes) = self
                .scan_purchased_albums(&mut read, &mut remaining)
                .await?;
            if albums.len() > MAX_ALBUMS {
                return Err(migu_upstream_error(
                    "Migu purchase source exceeded its album limit",
                ));
            }
            // Cache only within this read. Distinct ordinary and digital IDs remain
            // distinct; repeated subscriptions still expand at every source position.
            let mut catalogue = BTreeMap::new();
            let mut tracks = Vec::new();
            let mut catalogue_pages = 0_u32;
            for (album_position, album) in albums.iter().enumerate() {
                let album_id = album.extensions["content_id"].as_str().ok_or_else(|| {
                    migu_upstream_error("Migu purchase omitted its album identity")
                })?;
                let digital = match album.extensions["resource_type"].as_str() {
                    Some("2003") => false,
                    Some("5") => true,
                    _ => {
                        return Err(migu_upstream_error(
                            "Migu purchase has an unknown album type",
                        ));
                    }
                };
                let key = (digital, album_id.to_owned());
                if let std::collections::btree_map::Entry::Vacant(entry) =
                    catalogue.entry(key.clone())
                {
                    let (items, pages) = self
                        .complete_album_tracks(album_id, digital, Some(&read))
                        .await?;
                    catalogue_pages += pages;
                    entry.insert(items);
                }
                let items = &catalogue[&key];
                if items.len() > MAX_TRACKS.saturating_sub(tracks.len()) {
                    return Err(migu_upstream_error(
                        "Migu purchase source exceeded its track limit",
                    ));
                }
                for (album_track_position, mut track) in items.iter().cloned().enumerate() {
                    read.mark(&mut track.extensions);
                    // Public metadata and a subscription record are not playback grants.
                    track.playable = None;
                    track.extensions.extend(Extensions::from([
                        ("source_type".into(), json!(SOURCE_TYPE)),
                        ("source_position".into(), json!(tracks.len())),
                        ("purchase_album_id".into(), json!(album_id)),
                        (
                            "purchase_album_kind".into(),
                            json!(if digital { "digital_album" } else { "album" }),
                        ),
                        ("purchase_album_position".into(), json!(album_position)),
                        (
                            "purchase_album_track_position".into(),
                            json!(album_track_position),
                        ),
                    ]));
                    tracks.push(track);
                }
            }
            let (confirmed, confirmed_sizes) = self
                .scan_purchased_albums(&mut read, &mut remaining)
                .await?;
            if albums != confirmed || sizes != confirmed_sizes {
                return Err(error(
                    ErrorCode::Conflict,
                    "Migu album subscriptions changed while expanding their tracks",
                ));
            }
            let value =
                serde_json::to_value((SOURCE_TYPE, id, &albums, &tracks)).map_err(|_| {
                    migu_upstream_error("Migu purchase source snapshot could not be serialized")
                })?;
            // Includes credentials learned during the final subscription confirmation.
            reject_reflection(&value, &read.secrets)?;
            let bytes = serde_json::to_vec(&value).map_err(|_| {
                migu_upstream_error("Migu purchase source snapshot could not be serialized")
            })?;
            let extensions = Extensions::from([
                (
                    "backend".into(),
                    json!("pc_album_subscription_public_tracks"),
                ),
                ("source_type".into(), json!(SOURCE_TYPE)),
                ("source_user_id".into(), json!(id)),
                ("purchase_kind".into(), json!("album_subscriptions")),
                ("catalogue_scope".into(), json!("public")),
                ("complete_read".into(), json!(true)),
                (
                    "consistency".into(),
                    json!("two_subscription_reads_catalogue_once"),
                ),
                ("album_count".into(), json!(albums.len())),
                ("unique_album_count".into(), json!(catalogue.len())),
                ("catalogue_pages_fetched".into(), json!(catalogue_pages)),
                (
                    "source_snapshot_id".into(),
                    json!(format!(
                        "migu-purchased-album-tracks-{}",
                        hex::encode(Sha1::digest(bytes))
                    )),
                ),
            ]);
            Ok(Snapshot { tracks, extensions })
        })
        .await
        .map_err(|_| {
            error(
                ErrorCode::UpstreamTimeout,
                "Migu purchase source exceeded its total deadline",
            )
        })
        .and_then(|result| result);
        read.finish(result)
    }
}

pub(super) fn reject_reflection(value: &serde_json::Value, secrets: &[String]) -> Result<()> {
    match value {
        serde_json::Value::String(value) => {
            for secret in secrets {
                if value.contains(secret) {
                    return Err(migu_upstream_error(
                        "Migu album source reflected a credential",
                    ));
                }
                if value.starts_with("https://") {
                    crate::client::account_media::reject_url_secret(value, secret)?;
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                reject_reflection(value, secrets)?;
            }
        }
        serde_json::Value::Object(values) => {
            for value in values.values() {
                reject_reflection(value, secrets)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests;
