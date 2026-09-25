use super::*;

const MAX_ALBUMS: usize = 128;
const MAX_TRACKS: usize = 10_000;
const TOTAL_BUDGET: std::time::Duration = std::time::Duration::from_secs(120);

pub(super) struct AlbumCollectionSource {
    pub(super) playlist: Playlist,
    tracks: Vec<Track>,
}

impl SodaProvider {
    /// Expand the selected account's saved albums using the authenticated, complete
    /// album endpoint. The surrounding directory reads detect collection changes;
    /// the fingerprint also binds album contents across separate Uni page calls.
    pub(super) async fn read_album_collection_source(
        &self,
        user_id: &str,
        account: Option<&str>,
    ) -> Result<AlbumCollectionSource> {
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let snapshot = tokio::time::timeout(TOTAL_BUDGET, async {
            let (mut source, mut stored) =
                self.verified_account_source(Some(user_id), account).await?;
            let result = async {
                let before = self
                    .saved_album_snapshot(account, &mut source, &mut stored)
                    .await?;
                if !before.absence_proven {
                    return Err(soda_upstream_error(
                        "Soda album source requires a complete counted collection directory",
                    ));
                }
                if before.albums.len() > MAX_ALBUMS {
                    return Err(soda_invalid_request(
                        "Soda album sources cannot expand more than 128 albums",
                    ));
                }
                let mut tracks = Vec::new();
                let mut metadata = Vec::with_capacity(before.albums.len());
                for album in &before.albums {
                    let response = self.client.account_album(&album.id, &source).await;
                    self.ensure_account_snapshot_current(account, &source)?;
                    let response = response?;
                    if response.page.tracks.len() > MAX_TRACKS.saturating_sub(tracks.len()) {
                        return Err(soda_invalid_request(
                            "Soda album sources cannot expand more than 10000 tracks",
                        ));
                    }
                    self.advance_library_credential(&mut source, &mut stored, response.credential)?;
                    metadata.push(response.page.album);
                    // Preserve each occurrence, including songs shared by different albums.
                    tracks.extend(response.page.tracks);
                }
                let after = self
                    .saved_album_snapshot(account, &mut source, &mut stored)
                    .await?;
                if !after.absence_proven || before.albums != after.albums {
                    return Err(TuneWeaveError::new(
                        ErrorCode::Conflict,
                        "Soda saved albums changed while expanding the collection",
                    )
                    .with_platform(Platform::Soda));
                }
                let material = serde_json::to_vec(&json!({
                    "version": 1, "source_type": "collected_albums", "user_id": user_id,
                    "collection": before.albums, "albums": metadata, "tracks": tracks,
                }))
                .map_err(|_| {
                    TuneWeaveError::new(
                        ErrorCode::InternalError,
                        "Soda album collection snapshot could not be encoded",
                    )
                    .with_platform(Platform::Soda)
                })?;
                let snapshot_id = format!(
                    "soda_collected_albums_v1_{}",
                    source.source_snapshot_fingerprint(&material)
                );
                Ok(AlbumCollectionSource {
                    playlist: Playlist {
                        resource_ref: tuneweave_core::ResourceRef::new(Platform::Soda, user_id)
                            .map_err(|_| {
                                soda_invalid_request("Invalid Soda album source identity")
                            })?,
                        platform: Platform::Soda,
                        id: user_id.to_owned(),
                        name: "收藏的专辑".to_owned(),
                        description: String::new(),
                        cover_url: before
                            .albums
                            .first()
                            .and_then(|album| album.cover_url.clone()),
                        creator: None,
                        track_count: Some(tracks.len() as u64),
                        tags: Vec::new(),
                        subscribed: None,
                        created_at: None,
                        updated_at: None,
                        extensions: Extensions::from([
                            ("source_type".to_owned(), json!("collected_albums")),
                            (
                                "backend".to_owned(),
                                json!("official_pc_collected_album_tracks"),
                            ),
                            ("source_user_id".to_owned(), json!(user_id)),
                            ("source_album_count".to_owned(), json!(before.albums.len())),
                            ("source_snapshot_id".to_owned(), json!(snapshot_id)),
                            ("complete_read".to_owned(), json!(true)),
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
                "Soda album collection expansion exceeded its total time budget",
            )
            .with_platform(Platform::Soda)
            .retryable(true)
        })??;
        pending.complete();
        Ok(snapshot)
    }

    pub(super) async fn album_collection_source_items(
        &self,
        user_id: &str,
        request: &PageRequest,
    ) -> Result<Page<Track>> {
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(soda_invalid_request(
                "Soda album collection source pagination is invalid",
            ));
        }
        let snapshot = self
            .read_album_collection_source(user_id, request.account.as_deref())
            .await?;
        let mut page = soda_album_track_page(snapshot.tracks, request);
        page.pagination
            .extensions
            .extend(snapshot.playlist.extensions);
        Ok(page)
    }
}
