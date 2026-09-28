use super::*;
use sha1::{Digest, Sha1};

pub(super) struct AccountPlaylistSnapshot {
    pub playlist: Playlist,
    tracks: Vec<Track>,
    total: u64,
    raw_total: Option<u64>,
    pages_fetched: u32,
    final_cursor: u64,
    source_user_id: String,
}

impl AccountPlaylistSnapshot {
    pub(super) fn first_track_cover_url(&self) -> Option<&str> {
        self.tracks
            .first()
            .and_then(|track| track.album.as_ref())
            .and_then(|album| album.cover_url.as_deref())
    }

    pub(super) fn contains_track(&self, id: &str) -> bool {
        self.tracks.iter().any(|track| track.id == id)
    }

    pub(super) fn track_ids(&self) -> Vec<String> {
        self.tracks.iter().map(|track| track.id.clone()).collect()
    }

    pub(super) fn ordered_sort_media(&self, track_ids: &[String]) -> Result<Vec<(String, i32)>> {
        let mut durations = BTreeMap::<String, std::collections::VecDeque<i32>>::new();
        for track in &self.tracks {
            let duration = track
                .duration_ms
                .and_then(|duration| i32::try_from(duration).ok())
                .ok_or_else(|| {
                    soda_upstream_error(
                        "Soda playlist detail omitted a duration required by Android NetMedia",
                    )
                })?;
            durations
                .entry(track.id.clone())
                .or_default()
                .push_back(duration);
        }

        let mut ordered = Vec::with_capacity(track_ids.len());
        for id in track_ids {
            let duration = durations
                .get_mut(id)
                .and_then(|values| values.pop_front())
                .ok_or_else(|| {
                    soda_upstream_error(
                        "Soda playlist sort references did not match the complete source list",
                    )
                })?;
            ordered.push((id.clone(), duration));
        }
        if durations.values().any(|values| !values.is_empty()) {
            return Err(soda_upstream_error(
                "Soda playlist sort references did not include every source occurrence",
            ));
        }
        Ok(ordered)
    }

    pub(super) fn source_user_id(&self) -> &str {
        &self.source_user_id
    }

    pub(super) fn into_page(self, request: &PageRequest) -> Page<Track> {
        let tracks = self
            .tracks
            .into_iter()
            .skip(request.offset as usize)
            .take(request.limit as usize)
            .collect();
        let mut page = soda_playlist_page(
            tracks,
            request,
            self.total,
            self.pages_fetched,
            self.final_cursor,
            self.raw_total,
        );
        page.pagination.extensions.insert(
            "backend".to_owned(),
            json!("official_pc_account_playlist_detail"),
        );
        page.pagination
            .extensions
            .insert("complete_snapshot".to_owned(), json!(true));
        page.pagination
            .extensions
            .insert("source_user_id".to_owned(), json!(self.source_user_id));
        page.pagination.extensions.insert(
            "source_snapshot_id".to_owned(),
            self.playlist.extensions["source_snapshot_id"].clone(),
        );
        for key in ["source_type", "favorite_kind"] {
            if let Some(value) = self.playlist.extensions.get(key) {
                page.pagination
                    .extensions
                    .insert(key.to_owned(), value.clone());
            }
        }
        page
    }
}

impl SodaProvider {
    pub(super) async fn read_account_playlist(
        &self,
        playlist_id: &str,
        account: Option<&str>,
    ) -> Result<Option<AccountPlaylistSnapshot>> {
        self.read_account_playlist_with_budget(
            playlist_id,
            account,
            std::time::Duration::from_secs(45),
        )
        .await
    }

    pub(super) async fn read_account_playlist_with_budget(
        &self,
        playlist_id: &str,
        account: Option<&str>,
        budget: std::time::Duration,
    ) -> Result<Option<AccountPlaylistSnapshot>> {
        if account.is_none() && self.caller_credential.is_none() {
            return Ok(None);
        }
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let alias = account.unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let outcome = tokio::time::timeout(budget, async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(account, &source)?;
            self.advance_library_credential(&mut source, &mut stored, verified?.credential)?;
            self.read_verified_account_playlist(
                playlist_id,
                account,
                &mut source,
                &mut stored,
                None,
            )
            .await
            .map(Some)
        })
        .await;
        self.ensure_account_snapshot_current(account, &source)?;
        let result = outcome.map_err(|_| library_operations::library_timeout())?;
        if let Err(error) = &result {
            self.discard_response_credential_after_error(error.code)?;
        }
        pending.complete();
        result
    }

    // Continue the exact verified credential chain, including when resolving a special
    // collection. Selecting the account again could cross a concurrent login generation.
    pub(super) async fn read_verified_account_playlist(
        &self,
        playlist_id: &str,
        account: Option<&str>,
        source: &mut SodaCredential,
        stored: &mut Option<StoredAccountCredential>,
        expected_type: Option<i64>,
    ) -> Result<AccountPlaylistSnapshot> {
        let user_id = source
            .user_id()
            .ok_or_else(soda_authentication_required)?
            .to_owned();
        let mut tracks = Vec::new();
        let mut cursor = 0;
        let mut visited = BTreeSet::new();
        let mut expected_metadata = None;
        let mut playlist = None;
        for index in 0..MAX_UPSTREAM_PLAYLIST_PAGES {
            if !visited.insert(cursor) {
                return Err(soda_upstream_error(
                    "Soda account playlist repeated its cursor",
                ));
            }
            self.ensure_account_snapshot_current(account, source)?;
            let visible_before = tracks.len();
            let response = self
                .client
                .account_playlist_page(
                    playlist_id,
                    cursor,
                    UPSTREAM_ACCOUNT_PLAYLIST_PAGE_SIZE,
                    visible_before,
                    source,
                )
                .await;
            self.ensure_account_snapshot_current(account, source)?;
            let crate::client::SodaAccountPlaylistPage { page, credential } = response?;
            if expected_type.is_some_and(|kind| {
                page.playlist
                    .extensions
                    .get("playlist_type")
                    .and_then(serde_json::Value::as_i64)
                    != Some(kind)
                    || page
                        .playlist
                        .extensions
                        .get("owner_id")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|owner| owner != user_id)
            }) {
                return Err(soda_upstream_error(
                    "Soda account collection detail disagrees with its type or owner",
                ));
            }
            let metadata = json!({
                "id":page.playlist.id,"name":page.playlist.name,"description":page.playlist.description,
                "owner_id":page.playlist.extensions.get("owner_id"),
                "is_private":page.playlist.extensions.get("is_private"),
                "sort_type":page.playlist.extensions.get("current_sort_type"),
                "playlist_type":page.playlist.extensions.get("playlist_type"),
                "total":page.total,"raw_total":page.raw_total,"updated_at":page.updated_at,
            });
            if expected_metadata
                .as_ref()
                .is_some_and(|expected| expected != &metadata)
            {
                return Err(soda_upstream_error(
                    "Soda account playlist changed during pagination",
                ));
            }
            let visible = tracks
                .len()
                .checked_add(page.tracks.len())
                .ok_or_else(|| soda_upstream_error("Soda account playlist count overflowed"))?;
            if visible as u64 > page.total || (!page.has_more && visible as u64 != page.total) {
                return Err(soda_upstream_error(
                    "Soda account playlist pages do not match the visible track total",
                ));
            }
            // Empty physical pages can be valid: the cursor counts raw positions, including
            // unavailable entries. Explicit advancing cursors and the page budget bound them.
            if expected_metadata.is_none() {
                expected_metadata = Some(metadata);
            }
            self.advance_library_credential(source, stored, credential)?;
            if playlist.is_none() {
                playlist = Some(page.playlist);
            }
            for mut track in page.tracks {
                track.extensions.insert(
                    "backend".to_owned(),
                    json!("official_pc_account_playlist_detail"),
                );
                track
                    .extensions
                    .insert("playlist_position".to_owned(), json!(tracks.len()));
                tracks.push(track);
            }
            if !page.has_more {
                self.ensure_account_snapshot_current(account, source)?;
                let mut playlist = playlist.expect("first playlist page was validated");
                // This is a content/version fingerprint, not an authentication token. It
                // prevents Uni from assembling pages belonging to different complete reads.
                let material = serde_json::to_vec(&json!({
                    "version":1,"user_id":user_id,"metadata":expected_metadata,
                    "tracks":tracks.iter().map(|track| &track.resource_ref).collect::<Vec<_>>(),
                }))
                .map_err(|_| {
                    TuneWeaveError::new(
                        ErrorCode::InternalError,
                        "Soda playlist snapshot could not be encoded",
                    )
                    .with_platform(Platform::Soda)
                })?;
                let snapshot_id =
                    format!("soda_playlist_v1_{}", hex::encode(Sha1::digest(material)));
                playlist.extensions.insert(
                    "backend".to_owned(),
                    json!("official_pc_account_playlist_detail"),
                );
                playlist
                    .extensions
                    .insert("source_user_id".to_owned(), json!(user_id));
                playlist
                    .extensions
                    .insert("complete_snapshot".to_owned(), json!(true));
                playlist
                    .extensions
                    .insert("source_snapshot_id".to_owned(), json!(snapshot_id));
                return Ok(AccountPlaylistSnapshot {
                    playlist,
                    tracks,
                    total: page.total,
                    raw_total: page.raw_total,
                    pages_fetched: index + 1,
                    final_cursor: cursor,
                    source_user_id: user_id,
                });
            }
            cursor = page.next_cursor.ok_or_else(|| {
                soda_upstream_error("Soda account playlist lost its continuation cursor")
            })?;
        }
        Err(soda_upstream_error(
            "Soda account playlist exceeded the bounded upstream page count",
        ))
    }
}
