use super::*;
use crate::client::{library::Section, playlist_write::Write};
use crate::credential::{MiguCredential, authentication_required, error};
use tuneweave_core::{
    ErrorCode, PlaylistCreateRequest, PlaylistDeleteRequest, PlaylistDeleteResult,
    PlaylistItemKind, PlaylistItemMutationAction, PlaylistItemMutationRequest,
    PlaylistItemMutationResult, PlaylistKind, PlaylistMetadataUpdateVariant,
    PlaylistMutationAction, PlaylistMutationResult, PlaylistUpdateRequest, PlaylistVisibility,
    ResourceRef, StoredAccountCredential,
};

mod cover;
mod metadata;
mod order;

struct WriteSession {
    alias: String,
    original: MiguCredential,
    current: MiguCredential,
    stored: Option<StoredAccountCredential>,
    dispatched: bool,
}
fn name(value: &str) -> Result<&str> {
    if value.trim().is_empty() || value.len() > 2048 || value.chars().any(char::is_control) {
        return Err(migu_invalid_request("Migu playlist name is invalid"));
    }
    Ok(value.trim())
}
fn refs(values: &[ResourceRef], tracks: bool) -> Result<Vec<String>> {
    if values.is_empty() || values.len() > 100 {
        return Err(migu_invalid_request(
            "Migu playlist mutations require 1 to 100 distinct references",
        ));
    }
    let mut seen = BTreeSet::new();
    for value in values {
        if value.platform() != Platform::Migu || !seen.insert(value.id()) {
            return Err(migu_invalid_request(
                "Migu playlist mutation references must be distinct and belong to Migu",
            ));
        }
        if tracks {
            parse_content_id(value.id())?;
        } else {
            parse_playlist_id(value.id())?;
        }
    }
    Ok(values.iter().map(|value| value.id().to_owned()).collect())
}
fn ids(playlists: &[Playlist]) -> BTreeSet<&str> {
    playlists
        .iter()
        .map(|playlist| playlist.id.as_str())
        .collect()
}
fn owner(playlist: &Playlist, uid: &str) -> Result<()> {
    if playlist
        .extensions
        .get("owner_id")
        .and_then(serde_json::Value::as_str)
        != Some(uid)
    {
        return Err(error(
            ErrorCode::PermissionDenied,
            "Migu playlist does not belong to the selected account",
        ));
    }
    Ok(())
}
fn ordinary(id: &str, favorite: &str, created: &[Playlist]) -> Result<()> {
    if id == favorite || !created.iter().any(|playlist| playlist.id == id) {
        return Err(error(
            ErrorCode::PermissionDenied,
            "Migu playlist mutation requires an ordinary playlist in the selected account's created library",
        ));
    }
    Ok(())
}
fn invariant_metadata(playlist: &Playlist) -> serde_json::Value {
    json!({"id":playlist.id,"owner":playlist.extensions.get("owner_id"),
        "description":playlist.description,"tags":playlist.tags,
        "type":playlist.extensions.get("playlist_type")})
}

impl MiguProvider {
    fn playlist_write_session(&self, account: Option<&str>) -> Result<WriteSession> {
        let alias = account.unwrap_or("default");
        let (current, stored) = self.selected(alias)?.ok_or_else(authentication_required)?;
        Ok(WriteSession {
            alias: alias.to_owned(),
            original: current.clone(),
            current,
            stored,
            dispatched: false,
        })
    }
    fn finish_playlist_write<T>(&self, session: &WriteSession, result: Result<T>) -> Result<T> {
        self.finish_account_read(
            &session.original,
            &session.current,
            session.stored.as_ref(),
            result,
        )
        .map_err(|error| {
            if !session.dispatched {
                return error;
            }
            let mut details = error.details.as_object().cloned().unwrap_or_default();
            details.insert("operation".into(), json!("playlist_mutation"));
            details.insert("write_outcome".into(), json!("unconfirmed"));
            error
                .retryable(false)
                .with_details(serde_json::Value::Object(details))
        })
    }
    async fn write_favorite_id(&self, s: &mut WriteSession) -> Result<String> {
        let response = self
            .client
            .account_favorite_id(s.current.token(), s.current.user_id())
            .await?;
        self.accept_playlist_read(&s.alias, &mut s.current, &mut s.stored, response)
            .await
    }
    async fn write_created(&self, s: &mut WriteSession) -> Result<Vec<Playlist>> {
        self.read_library_section(Section::Created, &s.alias, &mut s.current, &mut s.stored)
            .await
    }
    async fn write_preflight(&self, s: &mut WriteSession) -> Result<(String, Vec<Playlist>)> {
        self.verify_account_step(&s.alias, &mut s.current, &mut s.stored, s.original.token())
            .await?;
        let favorite = self.write_favorite_id(s).await?;
        let created = self.write_created(s).await?;
        Ok((favorite, created))
    }
    async fn write_owned_metadata(&self, id: &str, s: &mut WriteSession) -> Result<Playlist> {
        let response = self
            .client
            .account_playlist_detail(id, s.current.token(), s.current.user_id())
            .await?;
        let playlist = self
            .accept_playlist_read(&s.alias, &mut s.current, &mut s.stored, response)
            .await?;
        owner(&playlist, s.current.user_id())?;
        Ok(playlist)
    }
    async fn write_dispatch(
        &self,
        write: Write<'_>,
        s: &mut WriteSession,
    ) -> Result<Option<String>> {
        self.accept_read(&s.current, s.stored.as_ref(), &s.current)?;
        s.dispatched = true;
        let response = self
            .client
            .write_account_playlist(write, s.current.token(), s.current.user_id())
            .await?;
        self.accept_playlist_read(&s.alias, &mut s.current, &mut s.stored, response)
            .await
    }
    async fn write_finish_identity(&self, favorite: &str, s: &mut WriteSession) -> Result<()> {
        if self.write_favorite_id(s).await? != favorite {
            return Err(migu_upstream_error(
                "Migu favorite identity changed during the playlist mutation",
            ));
        }
        self.accept_read(&s.current, s.stored.as_ref(), &s.current)?;
        Ok(())
    }

    pub(super) async fn create_owned_playlist(
        &self,
        request: &PlaylistCreateRequest,
    ) -> Result<PlaylistMutationResult> {
        let title = name(&request.name)?;
        if request.kind != PlaylistKind::Normal
            || request.visibility != PlaylistVisibility::PlatformDefault
        {
            return Err(migu_invalid_request(
                "Migu playlist creation supports normal playlists with explicit platform_default visibility; public/private control is not available",
            ));
        }
        let mut s = self.playlist_write_session(request.account.as_deref())?;
        let result = async {
            let (favorite, before) = self.write_preflight(&mut s).await?;
            let acknowledged = self.write_dispatch(Write::Create(title), &mut s).await?;
            let after = self.write_created(&mut s).await?;
            let before_ids = ids(&before);
            let after_ids = ids(&after);
            if !before_ids.is_subset(&after_ids) || after_ids.len() != before_ids.len() + 1 {
                return Err(migu_upstream_error("Migu created library did not identify exactly one new playlist"));
            }
            let added = after.iter().find(|playlist| !before_ids.contains(playlist.id.as_str()))
                .ok_or_else(|| migu_upstream_error("Migu created playlist could not be identified"))?;
            ordinary(&added.id, &favorite, &after)?;
            if added.name != title || acknowledged.as_deref().is_some_and(|id| id != added.id) {
                return Err(migu_upstream_error("Migu created playlist disagrees with its requested name or acknowledgement"));
            }
            let mut playlist = self.write_owned_metadata(&added.id, &mut s).await?;
            if playlist.name != title || playlist.track_count != Some(0) {
                return Err(migu_upstream_error("Migu created playlist metadata did not confirm an empty playlist with the requested name"));
            }
            self.write_finish_identity(&favorite, &mut s).await?;
            playlist.extensions.insert("requested_visibility".into(), json!("platform_default"));
            Ok(PlaylistMutationResult {
                playlist_ref: playlist.resource_ref.clone(), action: PlaylistMutationAction::Create,
                playlist: Some(playlist), extensions: Extensions::from([
                    ("backend".into(), json!("official_pc_playlist_create")),
                    ("verified_by".into(), json!("unique_complete_created_library_delta_and_owned_metadata")),
                    ("source_user_id".into(), json!(s.current.user_id())),
                    ("visibility".into(), json!("platform_default")),
                ]),
            })
        }.await;
        self.finish_playlist_write(&s, result)
    }

    pub(super) async fn rename_owned_playlist(
        &self,
        id: &str,
        request: &PlaylistUpdateRequest,
    ) -> Result<PlaylistMutationResult> {
        parse_playlist_id(id)?;
        if request.description.is_some() {
            return self.update_owned_playlist_metadata(id, request).await;
        }
        let title = name(
            request
                .name
                .as_deref()
                .ok_or_else(|| migu_invalid_request("Migu playlist update requires a name"))?,
        )?;
        if request.description.is_some()
            || request.tags.is_some()
            || !matches!(
                request.variant,
                PlaylistMetadataUpdateVariant::Default | PlaylistMetadataUpdateVariant::Individual
            )
        {
            return Err(migu_invalid_request(
                "Migu playlist update currently supports only the individual name field",
            ));
        }
        let mut s = self.playlist_write_session(request.account.as_deref())?;
        let result = async {
            let (favorite, before) = self.write_preflight(&mut s).await?;
            ordinary(id, &favorite, &before)?;
            let old = self.write_owned_metadata(id, &mut s).await?;
            let acknowledged = self
                .write_dispatch(Write::Rename(id, title), &mut s)
                .await?;
            if acknowledged.as_deref().is_some_and(|value| value != id) {
                return Err(migu_upstream_error(
                    "Migu rename acknowledgement referred to another playlist",
                ));
            }
            let after = self.write_created(&mut s).await?;
            if ids(&before) != ids(&after)
                || !after
                    .iter()
                    .any(|playlist| playlist.id == id && playlist.name == title)
            {
                return Err(migu_upstream_error(
                    "Migu complete created library did not confirm the playlist rename",
                ));
            }
            let playlist = self.write_owned_metadata(id, &mut s).await?;
            if playlist.name != title
                || playlist.track_count != old.track_count
                || invariant_metadata(&playlist) != invariant_metadata(&old)
            {
                return Err(migu_upstream_error(
                    "Migu playlist rename changed or failed to confirm other metadata",
                ));
            }
            self.write_finish_identity(&favorite, &mut s).await?;
            Ok(PlaylistMutationResult {
                playlist_ref: playlist.resource_ref.clone(),
                action: PlaylistMutationAction::Update,
                playlist: Some(playlist),
                extensions: Extensions::from([
                    ("backend".into(), json!("official_pc_playlist_rename")),
                    ("source_user_id".into(), json!(s.current.user_id())),
                ]),
            })
        }
        .await;
        self.finish_playlist_write(&s, result)
    }

    pub(super) async fn delete_owned_playlists(
        &self,
        request: &PlaylistDeleteRequest,
    ) -> Result<PlaylistDeleteResult> {
        let requested = refs(&request.playlist_refs, false)?;
        let mut s = self.playlist_write_session(request.account.as_deref())?;
        let mut completed: Vec<ResourceRef> = Vec::new();
        let result = async {
            let (favorite, mut created) = self.write_preflight(&mut s).await?;
            // Validate the whole batch before its first destructive request.
            for id in &requested { ordinary(id, &favorite, &created)?; }
            for id in &requested { self.write_owned_metadata(id, &mut s).await?; }
            for (index, id) in requested.iter().enumerate() {
                let acknowledged = self.write_dispatch(Write::Delete(id), &mut s).await?;
                if acknowledged.as_deref().is_some_and(|value| value != id) {
                    return Err(migu_upstream_error("Migu deletion acknowledgement referred to another playlist"));
                }
                let after = self.write_created(&mut s).await?;
                let mut expected = ids(&created);
                expected.remove(id.as_str());
                if ids(&after) != expected {
                    return Err(migu_upstream_error("Migu complete created library did not confirm exactly the requested deletion"));
                }
                self.write_finish_identity(&favorite, &mut s).await?;
                completed.push(request.playlist_refs[index].clone());
                created = after;
            }
            Ok(PlaylistDeleteResult { playlist_refs: completed.clone(), extensions: Extensions::from([
                ("backend".into(), json!("official_pc_playlist_delete")),
                ("source_user_id".into(), json!(s.current.user_id())),
                ("atomic".into(), json!(false)),
            ]) })
        }.await;
        let result = self.finish_playlist_write(&s, result);
        result.map_err(|error| {
            if !s.dispatched {
                return error;
            }
            let mut details = error.details.as_object().cloned().unwrap_or_default();
            details.insert("atomic".into(), json!(false));
            details.insert("completed_refs".into(), json!(completed));
            details.insert(
                "failed_ref".into(),
                json!(request.playlist_refs.get(completed.len())),
            );
            details.insert(
                "remaining_refs".into(),
                json!(
                    &request.playlist_refs[completed
                        .len()
                        .saturating_add(1)
                        .min(request.playlist_refs.len())..]
                ),
            );
            error.with_details(serde_json::Value::Object(details))
        })
    }

    pub(super) async fn write_owned_playlist_tracks(
        &self,
        id: &str,
        action: PlaylistItemMutationAction,
        request: &PlaylistItemMutationRequest,
    ) -> Result<PlaylistItemMutationResult> {
        parse_playlist_id(id)?;
        if request.kind != PlaylistItemKind::Track {
            return Err(migu_invalid_request(
                "Migu playlist mutations accept track content IDs only",
            ));
        }
        let requested = refs(&request.item_refs, true)?;
        let mut s = self.playlist_write_session(request.account.as_deref())?;
        let result = async {
            let (favorite, created) = self.write_preflight(&mut s).await?;
            ordinary(id, &favorite, &created)?;
            let before = self.read_selected_account_playlist(Some(id), &s.alias, &mut s.current, &mut s.stored).await?;
            owner(&before.playlist, s.current.user_id())?;
            if action == PlaylistItemMutationAction::Add
                && before.track_ids().len() + requested.iter().filter(|id| !before.contains_track(id)).count() > crate::client::account_playlist::MAX_TRACKS as usize {
                return Err(migu_invalid_request("Migu playlist addition exceeds the complete-read track budget"));
            }
            let write = match action { PlaylistItemMutationAction::Add => Write::Add(id, &requested), PlaylistItemMutationAction::Remove => Write::Remove(id, &requested) };
            let acknowledged = self.write_dispatch(write, &mut s).await?;
            if acknowledged.as_deref().is_some_and(|value| value != id) {
                return Err(migu_upstream_error("Migu track mutation acknowledgement referred to another playlist"));
            }
            let after = self.read_selected_account_playlist(Some(id), &s.alias, &mut s.current, &mut s.stored).await?;
            owner(&after.playlist, s.current.user_id())?;
            let old_ids = before.track_ids();
            let new_ids = after.track_ids();
            let wanted: BTreeSet<_> = requested.iter().map(String::as_str).collect();
            let confirmed = match action {
                PlaylistItemMutationAction::Remove => new_ids == old_ids.iter().copied().filter(|id| !wanted.contains(id)).collect::<Vec<_>>(),
                PlaylistItemMutationAction::Add => {
                    let added: BTreeSet<_> = wanted.iter().copied().filter(|id| !old_ids.contains(id)).collect();
                    new_ids.iter().copied().filter(|id| !added.contains(id)).collect::<Vec<_>>() == old_ids
                        && added.iter().all(|id| new_ids.iter().filter(|value| *value == id).count() == 1)
                }
            };
            if !confirmed || before.playlist.name != after.playlist.name || invariant_metadata(&before.playlist) != invariant_metadata(&after.playlist) {
                return Err(migu_upstream_error("Migu full playlist readback did not confirm the requested track mutation and preserve other entries"));
            }
            let current_created = self.write_created(&mut s).await?;
            if ids(&created) != ids(&current_created) {
                return Err(migu_upstream_error("Migu created library changed during the track mutation"));
            }
            self.write_finish_identity(&favorite, &mut s).await?;
            Ok(PlaylistItemMutationResult { playlist_ref: after.playlist.resource_ref, item_refs: request.item_refs.clone(), kind: PlaylistItemKind::Track, action,
                snapshot_id: after.playlist.extensions.get("source_snapshot_id").and_then(serde_json::Value::as_str).map(str::to_owned), cloud_track_count: None,
                extensions: Extensions::from([
                    ("backend".into(), json!("official_pc_playlist_tracks_write")),
                    ("verified_by".into(), json!("complete_before_after_tracks_and_created_library")),
                    ("source_user_id".into(), json!(s.current.user_id())),
                    ("existing_track_order_preserved".into(), json!(true)),
                ]) })
        }.await;
        self.finish_playlist_write(&s, result)
    }
}

#[cfg(test)]
mod tests;
