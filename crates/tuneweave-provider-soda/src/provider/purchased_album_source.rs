use super::*;

const MAX_ALBUMS: usize = 128;
const MAX_TRACKS: usize = 10_000;
const TOTAL_BUDGET: std::time::Duration = std::time::Duration::from_secs(120);

pub(super) struct PurchasedAlbumSource {
    pub(super) playlist: Playlist,
    tracks: Vec<Track>,
}

impl SodaProvider {
    /// Expand the selected account's digital-album directory through the official
    /// account album-detail route. The before/after catalog comparison prevents a
    /// partially expanded source from being reported as complete.
    pub(super) async fn read_purchased_album_source(
        &self,
        user_id: &str,
        account: Option<&str>,
    ) -> Result<PurchasedAlbumSource> {
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let snapshot = tokio::time::timeout(TOTAL_BUDGET, async {
            let (mut source, mut stored) =
                self.verified_account_source(Some(user_id), account).await?;
            let initial = source.clone();
            let result = async {
                let before = self
                    .purchased_album_catalog(account, &mut source, &mut stored)
                    .await?;
                if before.len() > MAX_ALBUMS {
                    return Err(soda_invalid_request(
                        "Soda purchased-album sources cannot expand more than 128 albums",
                    ));
                }

                let mut tracks = Vec::new();
                let mut details = Vec::with_capacity(before.len());
                for (album_position, album) in before.iter().enumerate() {
                    let response = self.client.account_album(&album.id, &source).await;
                    self.ensure_account_snapshot_current(account, &source)?;
                    let response = response?;
                    self.advance_library_credential(&mut source, &mut stored, response.credential)?;
                    if response.page.album.id != album.id {
                        return Err(soda_upstream_error(
                            "Soda purchased-album detail returned a different album identity",
                        ));
                    }
                    if response.page.tracks.len() > MAX_TRACKS.saturating_sub(tracks.len()) {
                        return Err(soda_invalid_request(
                            "Soda purchased-album sources cannot expand more than 10000 tracks",
                        ));
                    }
                    details.push(response.page.album);
                    for mut track in response.page.tracks {
                        // Ownership in the catalog does not establish current playback rights.
                        track.playable = None;
                        track
                            .extensions
                            .insert("source_album_position".to_owned(), json!(album_position));
                        tracks.push(track);
                    }
                }

                let after = self
                    .purchased_album_catalog(account, &mut source, &mut stored)
                    .await?;
                if before != after {
                    return Err(TuneWeaveError::new(
                        ErrorCode::Conflict,
                        "Soda purchased digital albums changed while expanding the collection",
                    )
                    .with_platform(Platform::Soda));
                }

                let source_user_id = source.user_id().ok_or_else(soda_authentication_required)?;
                if source_user_id != user_id {
                    return Err(soda_session_changed());
                }
                let material = serde_json::to_value(json!({
                    "version": 1,
                    "source_type": "purchased_albums",
                    "source_user_id": user_id,
                    "catalog": &before,
                    "albums": &details,
                    "tracks": &tracks,
                }))
                .map_err(|_| {
                    TuneWeaveError::new(
                        ErrorCode::InternalError,
                        "Soda purchased-album source snapshot could not be encoded",
                    )
                    .with_platform(Platform::Soda)
                })?;
                crate::account::reject_secrets(&material, &[initial, source.clone()])?;
                let material = serde_json::to_vec(&material).map_err(|_| {
                    TuneWeaveError::new(
                        ErrorCode::InternalError,
                        "Soda purchased-album source snapshot could not be fingerprinted",
                    )
                    .with_platform(Platform::Soda)
                })?;
                let snapshot_id = format!(
                    "soda_purchased_albums_v1_{}",
                    source.source_snapshot_fingerprint(&material)
                );
                let cover_url = before.first().and_then(|album| album.cover_url.clone());
                Ok(PurchasedAlbumSource {
                    playlist: Playlist {
                        resource_ref: tuneweave_core::ResourceRef::new(Platform::Soda, user_id)
                            .map_err(|_| {
                                soda_invalid_request("Invalid Soda album source identity")
                            })?,
                        platform: Platform::Soda,
                        id: user_id.to_owned(),
                        name: "已购数字专辑".to_owned(),
                        description: String::new(),
                        cover_url,
                        creator: None,
                        track_count: Some(tracks.len() as u64),
                        tags: Vec::new(),
                        subscribed: None,
                        created_at: None,
                        updated_at: None,
                        extensions: Extensions::from([
                            ("source_type".to_owned(), json!("purchased_albums")),
                            (
                                "backend".to_owned(),
                                json!("official_pc_purchased_digital_album_tracks"),
                            ),
                            ("source_user_id".to_owned(), json!(user_id)),
                            ("source_album_count".to_owned(), json!(before.len())),
                            ("source_snapshot_id".to_owned(), json!(snapshot_id)),
                            ("complete_read".to_owned(), json!(true)),
                            ("playback_entitlement_verified".to_owned(), json!(false)),
                        ]),
                    },
                    tracks,
                })
            }
            .await;
            self.ensure_account_snapshot_current(account, &source)?;
            result
        })
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Soda purchased-album expansion exceeded its total time budget",
            )
            .with_platform(Platform::Soda)
            .retryable(true)
        })??;
        pending.complete();
        Ok(snapshot)
    }

    async fn purchased_album_catalog(
        &self,
        account: Option<&str>,
        source: &mut SodaCredential,
        stored: &mut Option<StoredAccountCredential>,
    ) -> Result<Vec<DigitalAlbum>> {
        let response = self.client.account_digital_albums(source).await;
        self.ensure_account_snapshot_current(account, source)?;
        let response = response?;
        self.advance_library_credential(source, stored, response.credential)?;
        Ok(response.albums)
    }

    pub(super) async fn purchased_album_source_items(
        &self,
        user_id: &str,
        request: &PageRequest,
    ) -> Result<Page<Track>> {
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(soda_invalid_request(
                "Soda purchased-album source pagination is invalid",
            ));
        }
        let snapshot = self
            .read_purchased_album_source(user_id, request.account.as_deref())
            .await?;
        let mut page = soda_album_track_page(snapshot.tracks, request);
        page.pagination
            .extensions
            .extend(snapshot.playlist.extensions);
        Ok(page)
    }
}
