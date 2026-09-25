use super::account_playlists::AccountPlaylistSnapshot;
use super::*;

const FAVORITE_PLAYLIST_TYPE: i64 = 1;
const FAVORITE_BUDGET: std::time::Duration = std::time::Duration::from_secs(45);

impl SodaProvider {
    async fn favorite_source(
        &self,
        source: &mut SodaCredential,
        stored: &mut Option<StoredAccountCredential>,
    ) -> Result<String> {
        let playlists = self
            .read_library_section(LibrarySection::Created, source, stored)
            .await?;
        // Official PC client selects type 1 for favorites and type 4 for Douyin favorites.
        // Titles are mutable and cannot identify either collection.
        let mut favorites = playlists.into_iter().filter(|playlist| {
            playlist
                .extensions
                .get("playlist_type")
                .and_then(serde_json::Value::as_i64)
                == Some(FAVORITE_PLAYLIST_TYPE)
        });
        let favorite = favorites.next().ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::ResourceNotFound,
                "Soda account did not expose its favorite playlist",
            )
            .with_platform(Platform::Soda)
        })?;
        if favorites.next().is_some() {
            return Err(soda_upstream_error(
                "Soda account returned multiple favorite playlist identities",
            ));
        }
        parse_playlist_id(&favorite.id)
            .map_err(|_| soda_upstream_error("Soda favorite playlist identity is invalid"))?;
        Ok(favorite.id)
    }

    pub(super) async fn read_favorite_snapshot(
        &self,
        requested_user: Option<&str>,
        account: Option<&str>,
    ) -> Result<AccountPlaylistSnapshot> {
        self.read_favorite_snapshot_with_budget(requested_user, account, FAVORITE_BUDGET)
            .await
    }

    pub(super) async fn read_favorite_snapshot_with_budget(
        &self,
        requested_user: Option<&str>,
        account: Option<&str>,
        budget: std::time::Duration,
    ) -> Result<AccountPlaylistSnapshot> {
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let (mut source, mut stored) = self.collection_source(requested_user, account)?;
        let outcome = tokio::time::timeout(budget, async {
            self.verify_collection_source(requested_user, account, &mut source, &mut stored)
                .await?;
            let id = self.favorite_source(&mut source, &mut stored).await?;
            favorite_snapshot(
                self.read_verified_account_playlist(
                    &id,
                    account,
                    &mut source,
                    &mut stored,
                    Some(FAVORITE_PLAYLIST_TYPE),
                )
                .await?,
            )
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

    pub(super) async fn set_favorite_track(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        self.set_favorite_track_with_budget(id, subscribed, account, FAVORITE_BUDGET)
            .await
    }

    pub(super) async fn set_favorite_track_with_budget(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
        budget: std::time::Duration,
    ) -> Result<SubscriptionResult> {
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let identity = SodaTrackIdentity::parse(id)?;
        let (mut source, mut stored) = self.collection_source(None, account)?;
        let mut dispatched = false;
        let outcome = tokio::time::timeout(budget, async {
            self.verify_collection_source(None, account, &mut source, &mut stored)
                .await?;
            let favorite_id = self.favorite_source(&mut source, &mut stored).await?;
            let user_id = source.user_id().map(str::to_owned);
            self.ensure_account_snapshot_current(account, &source)?;
            dispatched = true;
            let refreshed = self
                .client
                .write_track_collection(identity.id(), subscribed, &source)
                .await;
            self.ensure_account_snapshot_current(account, &source)?;
            self.advance_library_credential(&mut source, &mut stored, refreshed?)?;
            let snapshot = favorite_snapshot(
                self.read_verified_account_playlist(
                    &favorite_id,
                    account,
                    &mut source,
                    &mut stored,
                    Some(FAVORITE_PLAYLIST_TYPE),
                )
                .await?,
            )?;
            if snapshot.contains_track(identity.id()) != subscribed {
                return Err(soda_upstream_error(
                    "Soda favorite track readback did not match the requested state",
                ));
            }
            // Playlist pages can omit unavailable tracks. Absence alone cannot prove
            // removal; require the selected account's explicit per-track state as well.
            if !subscribed {
                let track = self.client.account_track(&identity, &source).await;
                self.ensure_account_snapshot_current(account, &source)?;
                let track = track?;
                self.advance_library_credential(&mut source, &mut stored, track.credential)?;
                if track.collected != Some(false) {
                    return Err(soda_upstream_error(
                        "Soda did not explicitly confirm the track is no longer collected",
                    ));
                }
            }
            Ok(SubscriptionResult {
                resource_ref: tuneweave_core::ResourceRef::new(Platform::Soda, identity.id())
                    .map_err(|_| soda_invalid_request("Soda track identity is invalid"))?,
                subscribed,
                extensions: Extensions::from([
                    ("backend".to_owned(), json!("official_pc_media_collection")),
                    (
                        "verified_by".to_owned(),
                        json!(if subscribed {
                            "complete_favorite_playlist_readback"
                        } else {
                            "complete_favorite_playlist_and_track_state"
                        }),
                    ),
                    (
                        "favorite_playlist_ref".to_owned(),
                        json!(snapshot.playlist.resource_ref),
                    ),
                    ("source_user_id".to_owned(), json!(user_id)),
                ]),
            })
        })
        .await;
        let timed_out = outcome.is_err();
        let result = self
            .ensure_account_snapshot_current(account, &source)
            .and_then(|()| outcome.unwrap_or_else(|_| Err(library_operations::library_timeout())));
        if let Err(error) = &result {
            self.discard_response_credential_after_error(error.code)?;
        }
        if !timed_out {
            pending.complete();
        }
        result.map_err(|error| {
            if dispatched {
                error.retryable(false).with_details(
                    json!({"operation":"track_subscription", "write_outcome":"unconfirmed"}),
                )
            } else {
                error
            }
        })
    }
}

fn favorite_snapshot(mut snapshot: AccountPlaylistSnapshot) -> Result<AccountPlaylistSnapshot> {
    if snapshot
        .playlist
        .extensions
        .get("playlist_type")
        .and_then(serde_json::Value::as_i64)
        != Some(FAVORITE_PLAYLIST_TYPE)
    {
        return Err(soda_upstream_error(
            "Soda favorite playlist detail disagrees with its collection identity",
        ));
    }
    snapshot
        .playlist
        .extensions
        .insert("source_type".to_owned(), json!("favorite_tracks"));
    snapshot
        .playlist
        .extensions
        .insert("favorite_kind".to_owned(), json!("soda"));
    Ok(snapshot)
}
