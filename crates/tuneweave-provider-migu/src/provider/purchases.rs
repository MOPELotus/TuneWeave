use super::account_read::Read;
use super::*;
use crate::credential::error;
use sha1::{Digest, Sha1};
use std::time::Duration;
use tuneweave_core::{ErrorCode, PurchasedTrack};

const DEADLINE: Duration = Duration::from_secs(45);
const BACKEND: &str = "pacm_song_ordered_v2";

impl MiguProvider {
    pub(super) async fn read_purchased_tracks(
        &self,
        request: &PageRequest,
    ) -> Result<Page<PurchasedTrack>> {
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(migu_invalid_request("Migu purchase pagination is invalid"));
        }
        let mut read = Read::new(self, request.account.as_deref())?;
        let result = tokio::time::timeout(DEADLINE, async {
            read.start().await?;
            let ids = self.purchased_ids(&mut read).await?;
            let mut items = Vec::new();
            for (index, id) in ids
                .iter()
                .enumerate()
                .skip(request.offset as usize)
                .take(request.limit as usize)
            {
                // Public catalogue metadata is optional; purchase identity is retained
                // for explicitly missing resources. Other failures must remain errors.
                let track = match read.track(id).await {
                    Ok(mut track) => {
                        track.playable = None;
                        Some(track)
                    }
                    Err(failure) if failure.code == ErrorCode::ResourceNotFound => None,
                    Err(failure) => return Err(failure),
                };
                let extensions = Extensions::from([
                    ("backend".into(), json!(BACKEND)),
                    ("purchase_kind".into(), json!("tracks")),
                    ("content_id".into(), json!(id)),
                    ("resource_ref".into(), json!(format!("migu:{id}"))),
                    ("source_position".into(), json!(index)),
                    ("source_user_id".into(), json!(read.current.user_id())),
                    ("catalogue_scope".into(), json!("public")),
                    ("catalogue_resolved".into(), json!(track.is_some())),
                ]);
                items.push(PurchasedTrack {
                    name: track.as_ref().map(|t| t.name.clone()),
                    artists: track
                        .as_ref()
                        .map(|t| t.artists.clone())
                        .unwrap_or_default(),
                    cover_url: track
                        .as_ref()
                        .and_then(|t| t.album.as_ref())
                        .and_then(|a| a.cover_url.clone()),
                    track,
                    extensions,
                });
            }
            let confirmed = self.purchased_ids(&mut read).await?;
            if ids != confirmed {
                return Err(error(
                    ErrorCode::Conflict,
                    "Migu purchased tracks changed during the complete read",
                ));
            }
            let snapshot =
                serde_json::to_vec(&(BACKEND, read.current.user_id(), &ids)).map_err(|_| {
                    error(
                        ErrorCode::InternalError,
                        "Migu purchase snapshot could not be serialized",
                    )
                })?;
            let total = ids.len() as u64;
            let end = request.offset + items.len() as u32;
            let more = u64::from(end) < total;
            Ok(Page {
                items,
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(total),
                    has_more: more,
                    next_offset: more.then_some(end),
                    extensions: Extensions::from([
                        ("backend".into(), json!(BACKEND)),
                        ("purchase_kind".into(), json!("tracks")),
                        ("source_user_id".into(), json!(read.current.user_id())),
                        ("complete_read".into(), json!(true)),
                        ("consistency".into(), json!("two_complete_reads")),
                        (
                            "source_snapshot_id".into(),
                            json!(format!(
                                "migu-purchases-{}",
                                hex::encode(Sha1::digest(snapshot))
                            )),
                        ),
                    ]),
                },
            })
        })
        .await
        .map_err(|_| {
            error(
                ErrorCode::UpstreamTimeout,
                "Migu purchased tracks exceeded the total deadline",
            )
        })
        .and_then(|r| r);
        read.finish(result)
    }

    async fn purchased_ids(&self, read: &mut Read<'_>) -> Result<Vec<String>> {
        read.check()?;
        let response = self
            .client
            .account_purchased_track_ids(read.current.token(), read.current.user_id())
            .await;
        let ids = read.accept(response).await??;
        if ids
            .iter()
            .any(|id| read.secrets.iter().any(|token| id.contains(token)))
        {
            return Err(migu_upstream_error(
                "Migu purchase response reflected a credential",
            ));
        }
        Ok(ids)
    }
}

#[cfg(test)]
mod tests;
