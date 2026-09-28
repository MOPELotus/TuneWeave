use super::*;

const PLAYLIST_WRITE_BUDGET: std::time::Duration = std::time::Duration::from_secs(45);
const MAX_PLAYLIST_MEDIA_MUTATION_ITEMS: usize = 100;
// The ordinary PC-created playlist detail reads back with type 2; special
// collections use distinct types (for example favorites use 1 and 4).
const SODA_ORDINARY_PLAYLIST_TYPE: i64 = 2;

impl SodaProvider {
    pub(super) async fn mutate_owned_playlist_items(
        &self,
        id: &str,
        action: tuneweave_core::PlaylistItemMutationAction,
        request: &tuneweave_core::PlaylistItemMutationRequest,
    ) -> Result<tuneweave_core::PlaylistItemMutationResult> {
        self.mutate_owned_playlist_items_with_budget(id, action, request, PLAYLIST_WRITE_BUDGET)
            .await
    }

    async fn mutate_owned_playlist_items_with_budget(
        &self,
        id: &str,
        action: tuneweave_core::PlaylistItemMutationAction,
        request: &tuneweave_core::PlaylistItemMutationRequest,
        budget: std::time::Duration,
    ) -> Result<tuneweave_core::PlaylistItemMutationResult> {
        let playlist_id = parse_playlist_id(id)?;
        if request.kind != tuneweave_core::PlaylistItemKind::Track {
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "Soda playlist media writes support tracks only",
            )
            .with_platform(Platform::Soda));
        }
        if request.item_refs.is_empty()
            || request.item_refs.len() > MAX_PLAYLIST_MEDIA_MUTATION_ITEMS
        {
            return Err(soda_invalid_request(
                "Soda playlist media writes require 1 to 100 track references",
            ));
        }
        let mut seen = BTreeSet::new();
        let mut media_ids = Vec::with_capacity(request.item_refs.len());
        for item in &request.item_refs {
            if item.platform() != Platform::Soda {
                return Err(soda_invalid_request(
                    "Soda playlist media writes require Soda track references",
                ));
            }
            let identity = SodaTrackIdentity::parse(item.id())
                .map_err(|_| soda_invalid_request("Soda track reference is invalid"))?;
            if !seen.insert(identity.id().to_owned()) {
                return Err(soda_invalid_request(
                    "Soda playlist media writes require distinct track references",
                ));
            }
            media_ids.push(identity.id().to_owned());
        }

        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let account = request.account.as_deref();
        let alias = account.unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let mut dispatched = false;
        let outcome = tokio::time::timeout(budget, async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(account, &source)?;
            self.advance_library_credential(&mut source, &mut stored, verified?.credential)?;
            let source_user_id = source
                .user_id()
                .ok_or_else(soda_authentication_required)?
                .to_owned();

            let created_before = self
                .read_library_section_snapshot(LibrarySection::Created, &mut source, &mut stored)
                .await?;
            if !created_before.absence_proven {
                return Err(soda_upstream_error(
                    "Soda cannot prove playlist ownership from an incomplete Created library",
                ));
            }
            let listed_target = created_before
                .items
                .iter()
                .find(|playlist| playlist.id == playlist_id)
                .filter(|playlist| is_selected_account_playlist(playlist, &source_user_id))
                .ok_or_else(|| {
                    TuneWeaveError::new(
                        ErrorCode::PermissionDenied,
                        "Soda playlist media writes require a target in the selected account's Created library",
                    )
                    .with_platform(Platform::Soda)
                })?;
            if listed_target
                .extensions
                .get("playlist_type")
                .and_then(serde_json::Value::as_i64)
                .is_some_and(|kind| kind != SODA_ORDINARY_PLAYLIST_TYPE)
            {
                return Err(TuneWeaveError::new(
                    ErrorCode::CapabilityNotSupported,
                    "Soda playlist media writes support ordinary playlists only",
                )
                .with_platform(Platform::Soda));
            }
            let created_before_ids = created_playlist_ids(&created_before.items);

            let before = self
                .read_verified_account_playlist(
                    playlist_id,
                    account,
                    &mut source,
                    &mut stored,
                    None,
                )
                .await?;
            self.ensure_account_snapshot_current(account, &source)?;
            if !is_selected_account_playlist(&before.playlist, &source_user_id)
                || before
                    .playlist
                    .extensions
                    .get("playlist_type")
                    .and_then(serde_json::Value::as_i64)
                    != Some(SODA_ORDINARY_PLAYLIST_TYPE)
            {
                return Err(TuneWeaveError::new(
                    ErrorCode::PermissionDenied,
                    "Soda playlist media writes require a selected-account-owned ordinary playlist",
                )
                .with_platform(Platform::Soda));
            }
            if before.source_user_id() != source_user_id.as_str() {
                return Err(soda_upstream_error(
                    "Soda playlist detail did not match the selected account identity",
                ));
            }

            let before_ids = before.track_ids();
            let wanted = media_ids.iter().map(String::as_str).collect::<BTreeSet<_>>();
            let expected_ids = match action {
                tuneweave_core::PlaylistItemMutationAction::Add => {
                    let expected_len = before_ids
                        .len()
                        .checked_add(media_ids.len())
                        .filter(|len| {
                            *len <= UPSTREAM_PLAYLIST_PAGE_SIZE as usize
                                * MAX_UPSTREAM_PLAYLIST_PAGES as usize
                        })
                        .ok_or_else(|| {
                            soda_invalid_request(
                                "Soda playlist addition exceeds the complete-readback budget",
                            )
                        })?;
                    let mut expected = Vec::with_capacity(expected_len);
                    expected.extend(before_ids.iter().cloned());
                    expected.extend(media_ids.iter().cloned());
                    expected
                }
                tuneweave_core::PlaylistItemMutationAction::Remove => before_ids
                    .iter()
                    .filter(|track_id| !wanted.contains(track_id.as_str()))
                    .cloned()
                    .collect(),
            };

            // The official PC UI sends these IDs once in one request. A lost ACK or
            // failed complete readback leaves the result unknown, so this is never retried.
            dispatched = true;
            let refreshed = self
                .client
                .mutate_owned_playlist_media(
                    playlist_id,
                    &media_ids,
                    action == tuneweave_core::PlaylistItemMutationAction::Add,
                    &source,
                )
                .await;
            self.ensure_account_snapshot_current(account, &source)?;
            self.advance_library_credential(&mut source, &mut stored, refreshed?)?;

            let after = self
                .read_verified_account_playlist(
                    playlist_id,
                    account,
                    &mut source,
                    &mut stored,
                    None,
                )
                .await?;
            self.ensure_account_snapshot_current(account, &source)?;
            let after_ids = after.track_ids();
            if after_ids != expected_ids
                || !is_selected_account_playlist(&after.playlist, &source_user_id)
                || after
                    .playlist
                    .extensions
                    .get("playlist_type")
                    .and_then(serde_json::Value::as_i64)
                    != Some(SODA_ORDINARY_PLAYLIST_TYPE)
                || !playlist_metadata_preserved(&before.playlist, &after.playlist)
            {
                return Err(soda_upstream_error(
                    "Soda complete playlist readback did not confirm the requested track mutation and preserve other entries",
                ));
            }

            let created_after = self
                .read_library_section_snapshot(LibrarySection::Created, &mut source, &mut stored)
                .await?;
            if !created_after.absence_proven
                || created_playlist_ids(&created_after.items) != created_before_ids
                || !created_after.items.iter().any(|playlist| {
                    playlist.id == playlist_id
                        && is_selected_account_playlist(playlist, &source_user_id)
                })
            {
                return Err(soda_upstream_error(
                    "Soda Created-library readback changed during the playlist track mutation",
                ));
            }

            Ok(tuneweave_core::PlaylistItemMutationResult {
                playlist_ref: before.playlist.resource_ref.clone(),
                item_refs: request.item_refs.clone(),
                kind: tuneweave_core::PlaylistItemKind::Track,
                action,
                snapshot_id: after
                    .playlist
                    .extensions
                    .get("source_snapshot_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                cloud_track_count: after.playlist.track_count,
                extensions: Extensions::from([
                    (
                        "backend".to_owned(),
                        json!(match action {
                            tuneweave_core::PlaylistItemMutationAction::Add => {
                                "official_pc_playlist_media_append"
                            }
                            tuneweave_core::PlaylistItemMutationAction::Remove => {
                                "official_pc_playlist_media_delete"
                            }
                        }),
                    ),
                    (
                        "verified_by".to_owned(),
                        json!("complete_before_after_playlist_and_created_library_readback"),
                    ),
                    ("source_user_id".to_owned(), json!(source_user_id)),
                    ("existing_track_order_preserved".to_owned(), json!(true)),
                    ("write_requests_dispatched".to_owned(), json!(1)),
                    ("atomic".to_owned(), json!(false)),
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
                error.retryable(false).with_details(json!({
                    "operation": "playlist_track_mutation",
                    "write_outcome": "unconfirmed",
                    "playlist_ref": format!("soda:{playlist_id}"),
                    "action": action,
                }))
            } else {
                error
            }
        })
    }

    pub(super) async fn reorder_owned_playlist_tracks(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistTrackOrderRequest,
    ) -> Result<tuneweave_core::PlaylistTrackOrderResult> {
        self.reorder_owned_playlist_tracks_with_budget(id, request, PLAYLIST_WRITE_BUDGET)
            .await
    }

    async fn reorder_owned_playlist_tracks_with_budget(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistTrackOrderRequest,
        budget: std::time::Duration,
    ) -> Result<tuneweave_core::PlaylistTrackOrderResult> {
        let playlist_id = parse_playlist_id(id)?;
        if request.track_refs.is_empty()
            || request.track_refs.len()
                > UPSTREAM_PLAYLIST_PAGE_SIZE as usize * MAX_UPSTREAM_PLAYLIST_PAGES as usize
        {
            return Err(soda_invalid_request(
                "Soda playlist ordering requires 1 to 12,800 complete track references",
            ));
        }
        let mut desired_ids = Vec::with_capacity(request.track_refs.len());
        for reference in &request.track_refs {
            if reference.platform() != Platform::Soda {
                return Err(soda_invalid_request(
                    "Soda playlist ordering requires Soda track references",
                ));
            }
            let identity = SodaTrackIdentity::parse(reference.id())
                .map_err(|_| soda_invalid_request("Soda track reference is invalid"))?;
            desired_ids.push(identity.id().to_owned());
        }

        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let account = request.account.as_deref();
        let alias = account.unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let mut dispatched = false;
        let outcome = tokio::time::timeout(budget, async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(account, &source)?;
            self.advance_library_credential(&mut source, &mut stored, verified?.credential)?;
            let source_user_id = source
                .user_id()
                .ok_or_else(soda_authentication_required)?
                .to_owned();

            let created_before = self
                .read_library_section_snapshot(LibrarySection::Created, &mut source, &mut stored)
                .await?;
            if !created_before.absence_proven {
                return Err(soda_upstream_error(
                    "Soda cannot prove playlist ownership from an incomplete Created library",
                ));
            }
            let listed_target = created_before
                .items
                .iter()
                .find(|playlist| playlist.id == playlist_id)
                .filter(|playlist| is_selected_account_playlist(playlist, &source_user_id))
                .ok_or_else(|| {
                    TuneWeaveError::new(
                        ErrorCode::PermissionDenied,
                        "Soda playlist ordering requires a target in the selected account's Created library",
                    )
                    .with_platform(Platform::Soda)
                })?;
            if listed_target
                .extensions
                .get("playlist_type")
                .and_then(serde_json::Value::as_i64)
                .is_some_and(|kind| kind != SODA_ORDINARY_PLAYLIST_TYPE)
            {
                return Err(TuneWeaveError::new(
                    ErrorCode::CapabilityNotSupported,
                    "Soda playlist ordering supports ordinary playlists only",
                )
                .with_platform(Platform::Soda));
            }
            let created_before_ids = created_playlist_ids(&created_before.items);

            let before = self
                .read_verified_account_playlist(
                    playlist_id,
                    account,
                    &mut source,
                    &mut stored,
                    None,
                )
                .await?;
            self.ensure_account_snapshot_current(account, &source)?;
            if !is_selected_account_playlist(&before.playlist, &source_user_id)
                || before
                    .playlist
                    .extensions
                    .get("playlist_type")
                    .and_then(serde_json::Value::as_i64)
                    != Some(SODA_ORDINARY_PLAYLIST_TYPE)
            {
                return Err(TuneWeaveError::new(
                    ErrorCode::PermissionDenied,
                    "Soda playlist ordering requires a selected-account-owned ordinary playlist",
                )
                .with_platform(Platform::Soda));
            }
            if before.source_user_id() != source_user_id.as_str() {
                return Err(soda_upstream_error(
                    "Soda playlist detail did not match the selected account identity",
                ));
            }

            let before_ids = before.track_ids();
            if !same_track_multiset(&before_ids, &desired_ids) {
                return Err(soda_invalid_request(
                    "Soda playlist ordering must contain every current track occurrence exactly once",
                ));
            }

            if before_ids == desired_ids {
                return Ok(tuneweave_core::PlaylistTrackOrderResult {
                    playlist_ref: before.playlist.resource_ref.clone(),
                    track_refs: request.track_refs.clone(),
                    snapshot_id: before
                        .playlist
                        .extensions
                        .get("source_snapshot_id")
                        .and_then(serde_json::Value::as_str)
                        .map(str::to_owned),
                    extensions: Extensions::from([
                        ("backend".to_owned(), json!("official_android_playlist_media_sort")),
                        ("verified_by".to_owned(), json!("complete_playlist_readback")),
                        ("source_user_id".to_owned(), json!(source_user_id)),
                        ("write_requests_dispatched".to_owned(), json!(0)),
                        ("no_op".to_owned(), json!(true)),
                        ("atomic".to_owned(), json!(false)),
                    ]),
                });
            }

            let ordered_media = before.ordered_sort_media(&desired_ids)?;
            // Once sent, the server may commit even if the ACK or complete readback is lost.
            // This manual reordering request is issued once and is never retried.
            dispatched = true;
            let refreshed = self
                .client
                .sort_account_playlist_media(playlist_id, &ordered_media, &desired_ids, &source)
                .await;
            self.ensure_account_snapshot_current(account, &source)?;
            self.advance_library_credential(&mut source, &mut stored, refreshed?)?;

            let after = self
                .read_verified_account_playlist(
                    playlist_id,
                    account,
                    &mut source,
                    &mut stored,
                    None,
                )
                .await?;
            self.ensure_account_snapshot_current(account, &source)?;
            if after.track_ids() != desired_ids
                || !is_selected_account_playlist(&after.playlist, &source_user_id)
                || after
                    .playlist
                    .extensions
                    .get("playlist_type")
                    .and_then(serde_json::Value::as_i64)
                    != Some(SODA_ORDINARY_PLAYLIST_TYPE)
                || !playlist_metadata_preserved(&before.playlist, &after.playlist)
                || after.source_user_id() != source_user_id.as_str()
            {
                return Err(soda_upstream_error(
                    "Soda complete playlist readback did not confirm the requested order and preserve playlist metadata",
                ));
            }

            let created_after = self
                .read_library_section_snapshot(LibrarySection::Created, &mut source, &mut stored)
                .await?;
            if !created_after.absence_proven
                || created_playlist_ids(&created_after.items) != created_before_ids
                || !created_after.items.iter().any(|playlist| {
                    playlist.id == playlist_id
                        && is_selected_account_playlist(playlist, &source_user_id)
                })
            {
                return Err(soda_upstream_error(
                    "Soda Created-library readback changed during playlist ordering",
                ));
            }

            Ok(tuneweave_core::PlaylistTrackOrderResult {
                playlist_ref: after.playlist.resource_ref.clone(),
                track_refs: request.track_refs.clone(),
                snapshot_id: after
                    .playlist
                    .extensions
                    .get("source_snapshot_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                extensions: Extensions::from([
                    ("backend".to_owned(), json!("official_android_playlist_media_sort")),
                    (
                        "verified_by".to_owned(),
                        json!("sort_ack_and_complete_playlist_and_created_library_readback"),
                    ),
                    ("source_user_id".to_owned(), json!(source_user_id)),
                    ("write_requests_dispatched".to_owned(), json!(1)),
                    ("atomic".to_owned(), json!(false)),
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
                error.retryable(false).with_details(json!({
                    "operation": "playlist_track_order",
                    "write_outcome": "unconfirmed",
                    "playlist_ref": format!("soda:{playlist_id}"),
                    "requested_track_count": request.track_refs.len(),
                    "write_requests_dispatched": 1,
                }))
            } else {
                error
            }
        })
    }

    pub(super) async fn delete_owned_playlists(
        &self,
        request: &tuneweave_core::PlaylistDeleteRequest,
    ) -> Result<tuneweave_core::PlaylistDeleteResult> {
        self.delete_owned_playlists_with_budget(request, PLAYLIST_WRITE_BUDGET)
            .await
    }

    async fn delete_owned_playlists_with_budget(
        &self,
        request: &tuneweave_core::PlaylistDeleteRequest,
        budget: std::time::Duration,
    ) -> Result<tuneweave_core::PlaylistDeleteResult> {
        if request.playlist_refs.is_empty() {
            return Err(soda_invalid_request(
                "Soda playlist deletion requires one playlist reference",
            ));
        }
        if request.playlist_refs.len() != 1 {
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "Soda playlist deletion currently supports one playlist at a time",
            )
            .with_platform(Platform::Soda));
        }
        let target = &request.playlist_refs[0];
        if target.platform() != Platform::Soda {
            return Err(soda_invalid_request(
                "Soda playlist deletion requires Soda resource references",
            ));
        }
        let id = parse_playlist_id(target.id())?;

        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let alias = request.account.as_deref().unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let mut dispatched = false;
        let outcome = tokio::time::timeout(budget, async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            self.advance_library_credential(&mut source, &mut stored, verified?.credential)?;
            let source_user_id = source
                .user_id()
                .ok_or_else(soda_authentication_required)?
                .to_owned();

            let before_snapshot = self
                .read_library_section_snapshot(LibrarySection::Created, &mut source, &mut stored)
                .await?;
            if !before_snapshot.absence_proven {
                return Err(soda_upstream_error(
                    "Soda cannot prove playlist ownership from an incomplete Created library",
                ));
            }
            let owned = before_snapshot.items.iter().any(|playlist| {
                playlist.id == id
                    && playlist
                        .extensions
                        .get("owner_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(source_user_id.as_str())
                    && playlist
                        .extensions
                        .get("source_user_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(source_user_id.as_str())
            });
            if !owned {
                return Err(TuneWeaveError::new(
                    ErrorCode::PermissionDenied,
                    "Soda deletion requires a playlist proven to belong to the selected account",
                )
                .with_platform(Platform::Soda));
            }
            let expected_ids = before_snapshot
                .items
                .iter()
                .map(|playlist| playlist.id.clone())
                .filter(|playlist_id| playlist_id.as_str() != id)
                .collect::<BTreeSet<_>>();

            // A lost ACK cannot tell us whether this destructive request committed.
            // Send it once, then require an exact complete directory readback.
            dispatched = true;
            let refreshed = self.client.delete_owned_playlist(id, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            self.advance_library_credential(&mut source, &mut stored, refreshed?)?;

            let after_snapshot = self
                .read_library_section_snapshot(LibrarySection::Created, &mut source, &mut stored)
                .await?;
            if !after_snapshot.absence_proven {
                return Err(soda_upstream_error(
                    "Soda deletion lacks a complete Created-library readback",
                ));
            }
            let after_ids = after_snapshot
                .items
                .iter()
                .map(|playlist| playlist.id.clone())
                .collect::<BTreeSet<_>>();
            if after_ids != expected_ids {
                return Err(soda_upstream_error(
                    "Soda complete Created-library readback did not confirm exactly the requested deletion",
                ));
            }

            Ok(tuneweave_core::PlaylistDeleteResult {
                playlist_refs: vec![target.clone()],
                extensions: Extensions::from([
                    (
                        "backend".to_owned(),
                        json!("official_pc_playlist_delete"),
                    ),
                    (
                        "verified_by".to_owned(),
                        json!("complete_created_library_readback"),
                    ),
                    ("source_user_id".to_owned(), json!(source_user_id)),
                    ("write_requests_dispatched".to_owned(), json!(1)),
                    ("atomic".to_owned(), json!(false)),
                ]),
            })
        })
        .await;
        let timed_out = outcome.is_err();
        let result = self
            .ensure_account_snapshot_current(Some(alias), &source)
            .and_then(|()| outcome.unwrap_or_else(|_| Err(library_operations::library_timeout())));
        if let Err(error) = &result {
            self.discard_response_credential_after_error(error.code)?;
        }
        if !timed_out {
            pending.complete();
        }
        result.map_err(|error| {
            if dispatched {
                error.retryable(false).with_details(json!({
                    "operation": "playlist_delete",
                    "write_outcome": "unconfirmed",
                    "playlist_ref": target,
                }))
            } else {
                error
            }
        })
    }

    pub(super) async fn create_owned_playlist(
        &self,
        request: &tuneweave_core::PlaylistCreateRequest,
    ) -> Result<tuneweave_core::PlaylistMutationResult> {
        self.create_owned_playlist_with_budget(request, PLAYLIST_WRITE_BUDGET)
            .await
    }

    async fn create_owned_playlist_with_budget(
        &self,
        request: &tuneweave_core::PlaylistCreateRequest,
        budget: std::time::Duration,
    ) -> Result<tuneweave_core::PlaylistMutationResult> {
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        if request.name.is_empty()
            || request.name.encode_utf16().count() > 30
            || request.name.chars().any(char::is_control)
        {
            return Err(soda_invalid_request(
                "Soda playlist name must contain 1 to 30 non-control UTF-16 code units",
            ));
        }
        if request.kind != tuneweave_core::PlaylistKind::Normal {
            return Err(soda_invalid_request(
                "Soda playlist creation supports only normal playlists",
            ));
        }
        let is_private = match request.visibility {
            tuneweave_core::PlaylistVisibility::Public => false,
            tuneweave_core::PlaylistVisibility::Private => true,
            tuneweave_core::PlaylistVisibility::PlatformDefault => {
                return Err(soda_invalid_request(
                    "Soda playlist creation requires explicit public or private visibility",
                ));
            }
        };

        let alias = request.account.as_deref().unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let mut dispatched = false;
        let outcome = tokio::time::timeout(budget, async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            self.advance_library_credential(&mut source, &mut stored, verified?.credential)?;
            let source_user_id = source
                .user_id()
                .ok_or_else(soda_authentication_required)?
                .to_owned();

            // Once dispatched, the server may have committed even if its ACK or the
            // complete Created-library readback is lost. This operation is never retried.
            dispatched = true;
            let created = self
                .client
                .create_owned_playlist(&request.name, is_private, &source)
                .await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            let (id, refreshed) = created?;
            self.advance_library_credential(&mut source, &mut stored, refreshed)?;

            let snapshot = self
                .read_library_section_snapshot(LibrarySection::Created, &mut source, &mut stored)
                .await?;
            let matching_rows = snapshot
                .items
                .iter()
                .filter(|playlist| playlist.id == id)
                .collect::<Vec<_>>();
            let directory_playlist = matching_rows
                .first()
                .copied()
                .filter(|_| matching_rows.len() == 1)
                .filter(|playlist| {
                    playlist.name == request.name
                        && playlist.track_count.is_none_or(|count| count == 0)
                        && playlist
                            .extensions
                            .get("source_user_id")
                            .and_then(serde_json::Value::as_str)
                            == Some(source_user_id.as_str())
                })
                .ok_or_else(|| {
                    soda_upstream_error(
                        "Soda created playlist did not match the acknowledged playlist identity",
                    )
                })?;

            // The Created directory omits track counts on some valid rows. Confirm
            // emptiness from the complete, owner-scoped playlist detail instead of
            // treating an unknown directory count as either zero or nonzero.
            let detail = self
                .read_verified_account_playlist(&id, Some(alias), &mut source, &mut stored, None)
                .await?;
            let detail_identity_matches = detail.playlist.id == id;
            let detail_name_matches = detail.playlist.name == directory_playlist.name;
            let detail_owner_matches = detail
                .playlist
                .extensions
                .get("owner_id")
                .and_then(serde_json::Value::as_str)
                == Some(source_user_id.as_str());
            let detail_is_empty = detail.track_ids().is_empty();
            if !detail_identity_matches
                || !detail_name_matches
                || !detail_owner_matches
                || !detail_is_empty
            {
                return Err(soda_upstream_error(
                    "Soda created playlist detail did not confirm the acknowledged empty playlist",
                ));
            }
            let playlist = detail.playlist;

            Ok(tuneweave_core::PlaylistMutationResult {
                playlist_ref: tuneweave_core::ResourceRef::new(Platform::Soda, &id)
                    .map_err(|_| soda_upstream_error("Soda playlist identity is invalid"))?,
                action: tuneweave_core::PlaylistMutationAction::Create,
                playlist: Some(playlist),
                extensions: Extensions::from([
                    ("backend".to_owned(), json!("official_pc_create_playlist")),
                    (
                        "verified_by".to_owned(),
                        json!("complete_created_library_readback"),
                    ),
                    ("source_user_id".to_owned(), json!(source_user_id)),
                    (
                        "requested_visibility".to_owned(),
                        json!(if is_private { "private" } else { "public" }),
                    ),
                    // The official Created-library DTO has no privacy field, so the
                    // requested visibility is not represented as a readback fact.
                    ("visibility_verified".to_owned(), json!(false)),
                    ("empty_playlist_only".to_owned(), json!(true)),
                    ("write_requests_dispatched".to_owned(), json!(1)),
                    ("atomic".to_owned(), json!(false)),
                ]),
            })
        })
        .await;
        let timed_out = outcome.is_err();
        let result = self
            .ensure_account_snapshot_current(Some(alias), &source)
            .and_then(|()| outcome.unwrap_or_else(|_| Err(library_operations::library_timeout())));
        if let Err(error) = &result {
            self.discard_response_credential_after_error(error.code)?;
        }
        if !timed_out {
            pending.complete();
        }
        result.map_err(|error| {
            if dispatched {
                error.retryable(false).with_details(json!({
                    "operation": "playlist_create",
                    "write_outcome": "unconfirmed",
                }))
            } else {
                error
            }
        })
    }

    pub(super) async fn update_playlist_visibility(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistVisibilityUpdateRequest,
    ) -> Result<tuneweave_core::PlaylistMutationResult> {
        self.update_playlist_visibility_with_budget(id, request, PLAYLIST_WRITE_BUDGET)
            .await
    }

    async fn update_playlist_visibility_with_budget(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistVisibilityUpdateRequest,
        budget: std::time::Duration,
    ) -> Result<tuneweave_core::PlaylistMutationResult> {
        parse_playlist_id(id)?;
        request
            .validate()
            .map_err(|error| error.with_platform(Platform::Soda))?;
        let is_private = match request.visibility {
            tuneweave_core::PlaylistVisibility::Public => false,
            tuneweave_core::PlaylistVisibility::Private => true,
            tuneweave_core::PlaylistVisibility::PlatformDefault => {
                unreachable!("visibility validation rejects platform defaults before dispatch")
            }
        };

        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let alias = request.account.as_deref().unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let mut dispatched = false;
        let outcome = tokio::time::timeout(budget, async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            self.advance_library_credential(&mut source, &mut stored, verified?.credential)?;
            let source_user_id = source
                .user_id()
                .ok_or_else(soda_authentication_required)?
                .to_owned();

            // The update endpoint is scoped to the current account. Require a complete
            // Created-library listing with an explicit matching owner before dispatch.
            let created = self
                .read_library_section_snapshot(LibrarySection::Created, &mut source, &mut stored)
                .await?;
            if !created.absence_proven {
                return Err(soda_upstream_error(
                    "Soda cannot prove playlist ownership from an incomplete Created library",
                ));
            }
            let owned = created.items.iter().any(|playlist| {
                playlist.id == id
                    && playlist
                        .extensions
                        .get("owner_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(source_user_id.as_str())
                    && playlist
                        .extensions
                        .get("source_user_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(source_user_id.as_str())
            });
            if !owned {
                return Err(TuneWeaveError::new(
                    ErrorCode::PermissionDenied,
                    "Soda visibility can be changed only for a playlist proven to belong to the selected account",
                )
                .with_platform(Platform::Soda));
            }

            // The PC renderer treats a zero status_code as the mutation ACK, then
            // invalidates its playlist caches. Read the complete account playlist
            // detail afterward; its `is_private` field is what the PC menu consumes.
            dispatched = true;
            let refreshed = self
                .client
                .update_owned_playlist_visibility(id, is_private, &source)
                .await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            self.advance_library_credential(&mut source, &mut stored, refreshed?)?;

            let readback = self
                .read_verified_account_playlist(id, Some(alias), &mut source, &mut stored, None)
                .await?;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            let actual_owner = readback
                .playlist
                .extensions
                .get("owner_id")
                .and_then(serde_json::Value::as_str);
            let actual_visibility = readback
                .playlist
                .extensions
                .get("is_private")
                .and_then(serde_json::Value::as_bool);
            if readback.playlist.id != id
                || actual_owner != Some(source_user_id.as_str())
                || actual_visibility != Some(is_private)
            {
                return Err(soda_upstream_error(
                    "Soda playlist visibility readback did not confirm the selected owner's requested state",
                ));
            }

            Ok(tuneweave_core::PlaylistMutationResult {
                playlist_ref: tuneweave_core::ResourceRef::new(Platform::Soda, id)
                    .map_err(|_| soda_invalid_request("Soda playlist identity is invalid"))?,
                action: tuneweave_core::PlaylistMutationAction::Update,
                playlist: Some(readback.playlist),
                extensions: Extensions::from([
                    (
                        "backend".to_owned(),
                        json!("official_pc_update_playlist_visibility"),
                    ),
                    (
                        "verified_by".to_owned(),
                        json!("complete_account_playlist_detail_readback"),
                    ),
                    ("source_user_id".to_owned(), json!(source_user_id)),
                    (
                        "requested_visibility".to_owned(),
                        json!(if is_private { "private" } else { "public" }),
                    ),
                    (
                        "verified_visibility".to_owned(),
                        json!(if is_private { "private" } else { "public" }),
                    ),
                    ("visibility_verified".to_owned(), json!(true)),
                    (
                        "playlist_owner_verified_by".to_owned(),
                        json!("complete_created_library_readback"),
                    ),
                    ("write_requests_dispatched".to_owned(), json!(1)),
                    ("atomic".to_owned(), json!(false)),
                ]),
            })
        })
        .await;
        let timed_out = outcome.is_err();
        let result = self
            .ensure_account_snapshot_current(Some(alias), &source)
            .and_then(|()| outcome.unwrap_or_else(|_| Err(library_operations::library_timeout())));
        if let Err(error) = &result {
            self.discard_response_credential_after_error(error.code)?;
        }
        if !timed_out {
            pending.complete();
        }
        result.map_err(|error| {
            if dispatched {
                error.retryable(false).with_details(json!({
                    "operation": "playlist_visibility_update",
                    "write_outcome": "unconfirmed",
                }))
            } else {
                error
            }
        })
    }

    pub(super) async fn update_playlist_metadata(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistUpdateRequest,
    ) -> Result<tuneweave_core::PlaylistMutationResult> {
        self.update_playlist_metadata_with_budget(id, request, PLAYLIST_WRITE_BUDGET)
            .await
    }

    async fn update_playlist_metadata_with_budget(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistUpdateRequest,
        budget: std::time::Duration,
    ) -> Result<tuneweave_core::PlaylistMutationResult> {
        parse_playlist_id(id)?;
        if request.variant != tuneweave_core::PlaylistMetadataUpdateVariant::Default {
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "Soda playlist metadata supports only the default update variant",
            )
            .with_platform(Platform::Soda));
        }
        if request.tags.is_some() {
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "Soda playlist metadata updates do not support tags",
            )
            .with_platform(Platform::Soda));
        }
        let name = request.name.as_deref();
        let description = request.description.as_deref();
        if name.is_none() && description.is_none() {
            return Err(soda_invalid_request(
                "Soda playlist metadata update requires a title or description",
            ));
        }
        if let Some(name) = name
            && (name.is_empty()
                || name.encode_utf16().count() > 30
                || name.chars().any(char::is_control))
        {
            return Err(soda_invalid_request(
                "Soda playlist title must contain 1 to 30 non-control UTF-16 code units",
            ));
        }
        // The official PC editor initializes this field with `desc || ""` and
        // always submits it, so an explicit empty description is a real clear.
        let mut updated_fields = Vec::new();
        if name.is_some() {
            updated_fields.push("name");
        }
        if description.is_some() {
            updated_fields.push("description");
        }

        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let alias = request.account.as_deref().unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let mut dispatched = false;
        let outcome = tokio::time::timeout(budget, async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            self.advance_library_credential(&mut source, &mut stored, verified?.credential)?;
            let source_user_id = source
                .user_id()
                .ok_or_else(soda_authentication_required)?
                .to_owned();

            let before_snapshot = self
                .read_library_section_snapshot(LibrarySection::Created, &mut source, &mut stored)
                .await?;
            if !before_snapshot.absence_proven {
                return Err(soda_upstream_error(
                    "Soda cannot prove playlist ownership from an incomplete Created library",
                ));
            }
            let before = before_snapshot
                .items
                .into_iter()
                .find(|playlist| playlist.id == id)
                .filter(|playlist| {
                    playlist
                        .extensions
                        .get("owner_id")
                        .and_then(serde_json::Value::as_str)
                        == Some(source_user_id.as_str())
                        && playlist
                            .extensions
                            .get("source_user_id")
                            .and_then(serde_json::Value::as_str)
                            == Some(source_user_id.as_str())
                })
                .ok_or_else(|| {
                    TuneWeaveError::new(
                        ErrorCode::PermissionDenied,
                        "Soda playlist metadata update requires a target proven to belong to the selected account",
                    )
                    .with_platform(Platform::Soda)
                })?;

            dispatched = true;
            let refreshed = self
                .client
                .update_owned_playlist_metadata(id, name, description, &source)
                .await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            self.advance_library_credential(&mut source, &mut stored, refreshed?)?;

            let after_snapshot = self
                .read_library_section_snapshot(LibrarySection::Created, &mut source, &mut stored)
                .await?;
            if !after_snapshot.absence_proven {
                return Err(soda_upstream_error(
                    "Soda playlist metadata update lacks a complete Created-library readback",
                ));
            }
            let after = after_snapshot
                .items
                .into_iter()
                .find(|playlist| playlist.id == id)
                .filter(|playlist| {
                    let name_matches = match name {
                        Some(name) => playlist.name == name,
                        None => playlist.name == before.name,
                    };
                    let description_matches = match description {
                        Some(description) => playlist.description == description,
                        None => playlist.description == before.description,
                    };
                    name_matches
                        && description_matches
                        && playlist.track_count == before.track_count
                        && playlist
                            .extensions
                            .get("owner_id")
                            .and_then(serde_json::Value::as_str)
                            == Some(source_user_id.as_str())
                        && playlist
                            .extensions
                            .get("source_user_id")
                            .and_then(serde_json::Value::as_str)
                            == Some(source_user_id.as_str())
                })
                .ok_or_else(|| {
                    soda_upstream_error(
                        "Soda playlist metadata update readback did not match requested and preserved fields",
                    )
                })?;

            Ok(tuneweave_core::PlaylistMutationResult {
                playlist_ref: tuneweave_core::ResourceRef::new(Platform::Soda, id)
                    .map_err(|_| soda_invalid_request("Soda playlist identity is invalid"))?,
                action: tuneweave_core::PlaylistMutationAction::Update,
                playlist: Some(after),
                extensions: Extensions::from([
                    (
                        "backend".to_owned(),
                        json!("official_pc_update_playlist_info"),
                    ),
                    (
                        "verified_by".to_owned(),
                        json!("complete_created_library_readback"),
                    ),
                    ("source_user_id".to_owned(), json!(source_user_id)),
                    ("updated_fields".to_owned(), json!(updated_fields)),
                    ("write_requests_dispatched".to_owned(), json!(1)),
                    ("atomic".to_owned(), json!(false)),
                ]),
            })
        })
        .await;
        let timed_out = outcome.is_err();
        let result = self
            .ensure_account_snapshot_current(Some(alias), &source)
            .and_then(|()| outcome.unwrap_or_else(|_| Err(library_operations::library_timeout())));
        if let Err(error) = &result {
            self.discard_response_credential_after_error(error.code)?;
        }
        if !timed_out {
            pending.complete();
        }
        result.map_err(|error| {
            if dispatched {
                error.retryable(false).with_details(json!({
                    "operation": "playlist_metadata_update",
                    "write_outcome": "unconfirmed",
                }))
            } else {
                error
            }
        })
    }
}

fn is_selected_account_playlist(playlist: &Playlist, user_id: &str) -> bool {
    playlist
        .extensions
        .get("owner_id")
        .and_then(serde_json::Value::as_str)
        == Some(user_id)
        && playlist
            .extensions
            .get("source_user_id")
            .and_then(serde_json::Value::as_str)
            == Some(user_id)
}

fn created_playlist_ids(playlists: &[Playlist]) -> BTreeSet<String> {
    playlists
        .iter()
        .map(|playlist| playlist.id.clone())
        .collect()
}

fn playlist_metadata_preserved(before: &Playlist, after: &Playlist) -> bool {
    before.id == after.id
        && before.name == after.name
        && before.description == after.description
        && before.cover_url == after.cover_url
        && before.creator == after.creator
        && before.extensions.get("owner_id") == after.extensions.get("owner_id")
        && before.extensions.get("playlist_type") == after.extensions.get("playlist_type")
        && before.extensions.get("current_sort_type") == after.extensions.get("current_sort_type")
}

fn same_track_multiset(left: &[String], right: &[String]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut counts = BTreeMap::<String, isize>::new();
    for id in left {
        *counts.entry(id.clone()).or_default() += 1;
    }
    for id in right {
        *counts.entry(id.clone()).or_default() -= 1;
    }
    counts.values().all(|count| *count == 0)
}
