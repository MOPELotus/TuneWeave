use super::*;
use crate::client::{
    account::AccountData,
    account_playlist::{MAX_PAGES, PAGE_SIZE},
};
use crate::credential::{MiguCredential, authentication_required, error, validate_uid};
use sha1::{Digest, Sha1};
use tuneweave_core::StoredAccountCredential;

pub(super) struct Snapshot {
    pub playlist: Playlist,
    tracks: Vec<Track>,
    pub(super) order_fields: Vec<Option<crate::client::playlist_order::NativePlaylistSong>>,
    pub(super) order_metadata_stable: bool,
}
impl Snapshot {
    pub(super) fn ordered_tracks(&self) -> &[Track] {
        &self.tracks
    }

    pub(super) fn track_ids(&self) -> Vec<&str> {
        self.tracks.iter().map(|track| track.id.as_str()).collect()
    }

    pub(super) fn contains_track(&self, id: &str) -> bool {
        self.tracks.iter().any(|track| track.id == id)
    }

    pub(super) fn into_page(self, request: &PageRequest) -> Page<Track> {
        let total = self.tracks.len() as u64;
        let items: Vec<_> = self
            .tracks
            .into_iter()
            .skip(request.offset as usize)
            .take(request.limit as usize)
            .collect();
        let end = u64::from(request.offset) + items.len() as u64;
        Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(total),
                has_more: end < total,
                next_offset: (end < total).then_some(end as u32),
                extensions: self
                    .playlist
                    .extensions
                    .into_iter()
                    .filter(|(key, _)| {
                        matches!(
                            key.as_str(),
                            "backend"
                                | "source_user_id"
                                | "source_snapshot_id"
                                | "complete_read"
                                | "source_type"
                                | "favorite_kind"
                        )
                    })
                    .collect(),
            },
        }
    }
}
fn metadata(playlist: &Playlist) -> serde_json::Value {
    json!({"id":playlist.id,"name":playlist.name,"description":playlist.description,
        "owner_id":playlist.extensions.get("owner_id"),"type":playlist.extensions.get("playlist_type"),
        "status":playlist.extensions.get("status"),"total":playlist.track_count,
        "created_at":playlist.created_at,"tags":playlist.tags})
}
pub(super) fn validate_page(request: &PageRequest) -> Result<()> {
    if !(1..=100).contains(&request.limit) || request.offset.checked_add(request.limit).is_none() {
        return Err(migu_invalid_request(
            "Migu account playlist pagination is invalid",
        ));
    }
    Ok(())
}
impl MiguProvider {
    pub(super) async fn accept_playlist_read<T>(
        &self,
        account: &str,
        current: &mut MiguCredential,
        stored: &mut Option<StoredAccountCredential>,
        response: AccountData<T>,
    ) -> Result<T> {
        self.accept_read(current, stored.as_ref(), current)?;
        self.verify_account_step(account, current, stored, &response.token)
            .await?;
        response.data
    }

    pub(super) async fn read_account_playlist(
        &self,
        requested_id: Option<&str>,
        requested_user: Option<&str>,
        account: Option<&str>,
    ) -> Result<Snapshot> {
        if let Some(id) = requested_id {
            parse_playlist_id(id)?;
        }
        if let Some(uid) = requested_user {
            validate_uid(uid)?;
        }
        let alias = account.unwrap_or("default");
        let (mut current, mut stored) =
            self.selected(alias)?.ok_or_else(authentication_required)?;
        if requested_user.is_some_and(|uid| uid != current.user_id()) {
            return Err(error(
                tuneweave_core::ErrorCode::PermissionDenied,
                "Migu favorites are available only for the selected account",
            ));
        }
        let original = current.clone();
        let result = self
            .read_selected_account_playlist(requested_id, alias, &mut current, &mut stored)
            .await;
        self.finish_account_read(&original, &current, stored.as_ref(), result)
    }
    pub(super) async fn read_selected_account_playlist(
        &self,
        requested_id: Option<&str>,
        alias: &str,
        current: &mut MiguCredential,
        stored: &mut Option<StoredAccountCredential>,
    ) -> Result<Snapshot> {
        let token = current.token().to_owned();
        self.verify_account_step(alias, current, stored, &token)
            .await?;
        let favorite = requested_id.is_none();
        let id = if let Some(id) = requested_id {
            id.to_owned()
        } else {
            let response = self
                .client
                .account_favorite_id(current.token(), current.user_id())
                .await?;
            self.accept_playlist_read(alias, current, stored, response)
                .await?
        };
        parse_playlist_id(&id)
            .map_err(|_| migu_upstream_error("Migu account returned an invalid playlist ID"))?;
        let response = self
            .client
            .account_playlist_detail(&id, current.token(), current.user_id())
            .await?;
        let mut playlist = self
            .accept_playlist_read(alias, current, stored, response)
            .await?;
        let expected = metadata(&playlist);
        if favorite
            && playlist
                .extensions
                .get("owner_id")
                .and_then(serde_json::Value::as_str)
                != Some(current.user_id())
        {
            return Err(migu_upstream_error(
                "Migu favorite playlist does not belong to the selected user",
            ));
        }
        let total = playlist
            .track_count
            .ok_or_else(|| migu_upstream_error("Migu account playlist omitted its total"))?;
        let mut tracks = Vec::new();
        let mut order_fields = Vec::new();
        let mut publication = None;
        let mut pages = BTreeSet::new();
        for index in 1..=MAX_PAGES {
            let response = self
                .client
                .account_playlist_tracks(&id, index, current.token(), current.user_id())
                .await?;
            let response = self
                .accept_playlist_read(alias, current, stored, response)
                .await?;
            let page = response.page;
            if response.owner.as_deref().is_some_and(|owner| {
                playlist
                    .extensions
                    .get("owner_id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|expected| expected != owner)
            }) || favorite
                && response
                    .owner
                    .as_deref()
                    .is_some_and(|owner| owner != current.user_id())
            {
                return Err(migu_upstream_error(
                    "Migu account playlist page owner changed",
                ));
            }
            if page.total != total {
                return Err(migu_upstream_error("Migu account playlist total changed"));
            }
            if let Some(expected) = &publication {
                if expected != &page.publish_time {
                    return Err(migu_upstream_error(
                        "Migu playlist publication changed between pages",
                    ));
                }
            } else {
                publication = Some(page.publish_time);
            }
            let count = page.tracks.len();
            order_fields.extend(response.order_fields);
            let consumed = tracks.len() as u64 + count as u64;
            if consumed > total || consumed < total && count != PAGE_SIZE as usize {
                return Err(migu_upstream_error(
                    "Migu account playlist ended before its total",
                ));
            }
            let page_ids: Vec<_> = page
                .tracks
                .iter()
                .map(|track| track.resource_ref.to_string())
                .collect();
            if !page_ids.is_empty() && !pages.insert(page_ids) {
                return Err(migu_upstream_error(
                    "Migu account playlist repeated an entire page",
                ));
            }
            for mut track in page.tracks {
                track
                    .extensions
                    .insert("playlist_position".into(), json!(tracks.len()));
                track
                    .extensions
                    .insert("backend".into(), json!("official_pc_account_playlist"));
                tracks.push(track);
            }
            if consumed == total {
                break;
            }
        }
        if tracks.len() as u64 != total {
            return Err(migu_upstream_error(
                "Migu account playlist exceeded its complete-read budget",
            ));
        }
        let response = self
            .client
            .account_playlist_detail(&id, current.token(), current.user_id())
            .await?;
        let final_playlist = self
            .accept_playlist_read(alias, current, stored, response)
            .await?;
        if metadata(&final_playlist) != expected {
            return Err(migu_upstream_error(
                "Migu account playlist changed during its read",
            ));
        }
        let order_metadata_stable = playlist.cover_url == final_playlist.cover_url
            && playlist.extensions.get("tag_items") == final_playlist.extensions.get("tag_items")
            && playlist.extensions.get("have_private_picture")
                == final_playlist.extensions.get("have_private_picture");
        if favorite {
            let response = self
                .client
                .account_favorite_id(current.token(), current.user_id())
                .await?;
            let final_id = self
                .accept_playlist_read(alias, current, stored, response)
                .await?;
            if final_id != id {
                return Err(migu_upstream_error(
                    "Migu account favorite playlist identity changed",
                ));
            }
        }
        self.accept_read(current, stored.as_ref(), current)?;
        // This content fingerprint is for joining successive complete reads,
        // not an authentication proof or an upstream transactional revision.
        let material=serde_json::to_vec(&json!({"version":1,"user_id":current.user_id(),"favorite":favorite,
                "metadata":expected,"publication":publication,"tracks":tracks.iter().map(|t|json!({
                    "ref":t.resource_ref,"song_id":t.extensions.get("song_id"),"copyright_id":t.extensions.get("copyright_id")
                })).collect::<Vec<_>>()
            })).map_err(|_|error(tuneweave_core::ErrorCode::InternalError,"Migu playlist fingerprint could not be encoded"))?;
        playlist.extensions.insert(
            "source_snapshot_id".into(),
            json!(format!(
                "migu_account_playlist_v1_{}",
                hex::encode(Sha1::digest(material))
            )),
        );
        playlist
            .extensions
            .insert("source_user_id".into(), json!(current.user_id()));
        playlist
            .extensions
            .insert("complete_read".into(), json!(true));
        if favorite {
            playlist
                .extensions
                .insert("source_type".into(), json!("favorite_tracks"));
            playlist
                .extensions
                .insert("favorite_kind".into(), json!("official_private_navigation"));
        }
        Ok(Snapshot {
            playlist,
            tracks,
            order_fields,
            order_metadata_stable,
        })
    }
}

#[cfg(test)]
mod tests;
