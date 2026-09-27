use super::*;
use crate::client::account_download::NativeAuthorization;
use crate::client::playlist_tags::{PlaylistTag, TagChange};
use crate::provider::account_playlists::Snapshot;
use std::time::Duration;

pub(super) struct PendingCallerUpdate<'a> {
    pub(super) provider: &'a MiguProvider,
    pub(super) finished: bool,
}
impl Drop for PendingCallerUpdate<'_> {
    fn drop(&mut self) {
        if !self.finished
            && self.provider.caller_credential.is_some()
            && let Ok(mut response) = self.provider.response_credential.lock()
        {
            *response = None;
        }
    }
}

fn validate_description(value: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.len() > 4000
        || value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(migu_invalid_request(
            "Migu playlist description must be nonempty and at most 4000 UTF-8 bytes; clearing it is not supported",
        ));
    }
    Ok(())
}

fn validate_requested_tags(tags: &[String]) -> Result<()> {
    if tags.len() > 128 {
        return Err(migu_invalid_request(
            "Migu playlist updates accept at most 128 tags",
        ));
    }
    let mut unique = BTreeSet::new();
    for tag in tags {
        if tag.trim().is_empty()
            || tag.len() > 256
            || tag.chars().any(|c| c.is_control() || c == '|')
            || !unique.insert(tag.as_str())
        {
            return Err(migu_invalid_request(
                "Migu playlist tags must be distinct, nonempty, and contain no control characters or pipe delimiters",
            ));
        }
    }
    Ok(())
}

fn read_tags(playlist: &Playlist) -> Result<Vec<PlaylistTag>> {
    let Some(value) = playlist.extensions.get("tag_items") else {
        if playlist.tags.is_empty() {
            return Ok(Vec::new());
        }
        return Err(migu_upstream_error(
            "Migu playlist omitted tag identities required for safe tag updates",
        ));
    };
    let tags: Vec<PlaylistTag> = serde_json::from_value(value.clone())
        .map_err(|_| migu_upstream_error("Migu playlist tag identities are invalid"))?;
    if tags.len() != playlist.tags.len() || tags.len() > 128 {
        return Err(migu_upstream_error(
            "Migu playlist tag identities do not match its tag names",
        ));
    }
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    for tag in &tags {
        let numeric = tag.tag_id.parse::<u64>().ok();
        if numeric.is_none_or(|id| id == 0 || id.to_string() != tag.tag_id)
            || tag.tag_name.trim().is_empty()
            || tag.tag_name.len() > 256
            || tag.tag_name.chars().any(|c| c.is_control() || c == '|')
            || !ids.insert(tag.tag_id.as_str())
            || !names.insert(tag.tag_name.as_str())
        {
            return Err(migu_upstream_error(
                "Migu playlist tag identities are ambiguous",
            ));
        }
    }
    if tags
        .iter()
        .map(|tag| tag.tag_name.as_str())
        .ne(playlist.tags.iter().map(String::as_str))
    {
        return Err(migu_upstream_error(
            "Migu playlist tag names do not match its tag identities",
        ));
    }
    Ok(tags)
}

fn tag_plan(
    current: &[PlaylistTag],
    desired: &[String],
) -> Result<(Vec<PlaylistTag>, Vec<String>)> {
    // The official mutation exposes removal and append, not a reorder field.
    // Retain the longest requested prefix already in the current relative
    // order, then explicitly remove/re-add the rest. Each step is independently
    // confirmed below; a failure can leave a partially changed label list.
    let mut retained_names = BTreeSet::new();
    let mut current_position = 0;
    for name in desired {
        let Some(offset) = current[current_position..]
            .iter()
            .position(|tag| tag.tag_name == *name)
        else {
            break;
        };
        current_position += offset + 1;
        retained_names.insert(name.as_str());
    }
    let additions = desired[retained_names.len()..].to_vec();
    // The official picker permits six selected labels. Existing oversized
    // playlists may still be cleaned up without introducing another label.
    if !additions.is_empty() && desired.len() > 6 {
        return Err(migu_invalid_request(
            "Migu playlist tag additions permit at most six selected tags",
        ));
    }
    Ok((
        current
            .iter()
            .filter(|tag| !retained_names.contains(tag.tag_name.as_str()))
            .cloned()
            .collect(),
        additions,
    ))
}

fn untouched_metadata(playlist: &Playlist) -> serde_json::Value {
    json!({
        "id":playlist.id,"owner":playlist.extensions.get("owner_id"),
        "cover":playlist.cover_url,"created_at":playlist.created_at,
        "type":playlist.extensions.get("playlist_type"),
        "status":playlist.extensions.get("status"),
        "have_private_picture":playlist.extensions.get("have_private_picture"),
    })
}

fn confirm_readback(
    before: &Snapshot,
    after: &Snapshot,
    expected_tags: &[PlaylistTag],
    expected_title: &str,
    expected_description: &str,
) -> Result<()> {
    let actual_tags = read_tags(&after.playlist)?;
    let expected_names = expected_tags
        .iter()
        .map(|tag| tag.tag_name.as_str())
        .collect::<Vec<_>>();
    let actual_names = after
        .playlist
        .tags
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    if actual_tags != expected_tags
        || actual_names != expected_names
        || after.playlist.name != expected_title
        || after.playlist.description != expected_description
        || before.track_ids() != after.track_ids()
        || untouched_metadata(&before.playlist) != untouched_metadata(&after.playlist)
    {
        return Err(migu_upstream_error(
            "Migu native playlist metadata update did not preserve and confirm the complete playlist",
        ));
    }
    Ok(())
}

impl MiguProvider {
    pub(super) async fn write_native_authorization(
        &self,
        s: &mut WriteSession,
    ) -> Result<NativeAuthorization> {
        self.accept_read(&s.current, s.stored.as_ref(), &s.current)?;
        let read = self
            .client
            .account_profile(&s.alias, s.current.token(), Some(s.current.user_id()))
            .await?;
        self.accept_read(&s.current, s.stored.as_ref(), &s.current)?;
        self.accept_account_step(&mut s.current, &mut s.stored, read.token)?;
        read.profile?;
        let native_session =
            crate::client::account::require_native_exchange_session(read.native_session)?;
        let response = self
            .client
            .account_h5_token(s.current.token(), &native_session)
            .await?;
        let candidate = self
            .accept_playlist_read(&s.alias, &mut s.current, &mut s.stored, response)
            .await?;
        let authorization = self
            .client
            .validate_native_token(candidate, s.current.user_id())
            .await;
        self.accept_read(&s.current, s.stored.as_ref(), &s.current)?;
        authorization
    }

    pub(in crate::provider) async fn update_owned_playlist_metadata(
        &self,
        id: &str,
        request: &PlaylistUpdateRequest,
    ) -> Result<PlaylistMutationResult> {
        let description = request.description.as_deref();
        if let Some(description) = description {
            validate_description(description)?;
        }
        let title = request.name.as_deref().map(name).transpose()?;
        let desired_tags = request.tags.as_deref();
        if let Some(tags) = desired_tags {
            validate_requested_tags(tags)?;
        }
        let fields = usize::from(title.is_some())
            + usize::from(description.is_some())
            + usize::from(desired_tags.is_some());
        if fields == 0
            || request.variant == PlaylistMetadataUpdateVariant::Individual && fields != 1
        {
            return Err(migu_invalid_request(
                "Migu native playlist updates require at least one supported field; individual updates require exactly one field",
            ));
        }
        let mut s = self.playlist_write_session(request.account.as_deref())?;
        let mut pending = PendingCallerUpdate {
            provider: self,
            finished: false,
        };
        let mut confirmed_tag_removals = Vec::new();
        let mut confirmed_tag_additions = Vec::new();
        let result = tokio::time::timeout(Duration::from_secs(90), async {
            let (favorite, created) = self.write_preflight(&mut s).await?;
            ordinary(id, favorite.as_deref(), &created)?;
            let before = self
                .read_selected_account_playlist(Some(id), &s.alias, &mut s.current, &mut s.stored)
                .await?;
            owner(&before.playlist, s.current.user_id())?;
            let expected_title = title.unwrap_or(&before.playlist.name);
            let expected_description = description.unwrap_or(&before.playlist.description);
            let mut expected_tags = read_tags(&before.playlist)?;
            let (removals, addition_names) = if let Some(desired) = desired_tags {
                tag_plan(&expected_tags, desired)?
            } else {
                (Vec::new(), Vec::new())
            };
            if removals.len() + addition_names.len() > 16 {
                return Err(error(
                    ErrorCode::CapabilityNotSupported,
                    "Migu playlist updates are limited to 16 individually verified tag changes",
                ));
            }
            let mut additions = Vec::new();
            if !addition_names.is_empty() {
                self.accept_read(&s.current, s.stored.as_ref(), &s.current)?;
                let catalogue = self.client.playlist_tag_catalogue().await;
                self.accept_read(&s.current, s.stored.as_ref(), &s.current)?;
                let catalogue = catalogue?;
                for name in addition_names {
                    let tag = catalogue
                        .iter()
                        .find(|tag| tag.tag_name == name)
                        .ok_or_else(|| {
                            migu_invalid_request(
                                "Migu requested playlist tag is absent from the official catalogue",
                            )
                        })?;
                    if expected_tags.iter().any(|old| {
                        (old.tag_id == tag.tag_id && old.tag_name != tag.tag_name)
                            || (old.tag_name == tag.tag_name && old.tag_id != tag.tag_id)
                    }) {
                        return Err(migu_upstream_error(
                            "Migu playlist tag identity conflicts with the official catalogue",
                        ));
                    }
                    additions.push(tag.clone());
                }
            }
            let changes = removals
                .iter()
                .map(TagChange::Remove)
                .chain(additions.iter().map(TagChange::Add))
                .collect::<Vec<_>>();
            let mut final_playlist = before.playlist.clone();
            if !changes.is_empty() || title.is_some() || description.is_some() {
                let authorization = self.write_native_authorization(&mut s).await?;
                if changes.is_empty() {
                    self.accept_read(&s.current, s.stored.as_ref(), &s.current)?;
                    s.dispatched = true;
                    self.client
                        .write_native_playlist_metadata(
                            &authorization,
                            id,
                            title,
                            description,
                            None,
                        )
                        .await?;
                    self.accept_read(&s.current, s.stored.as_ref(), &s.current)?;
                    let after = self
                        .read_selected_account_playlist(
                            Some(id),
                            &s.alias,
                            &mut s.current,
                            &mut s.stored,
                        )
                        .await?;
                    owner(&after.playlist, s.current.user_id())?;
                    confirm_readback(
                        &before,
                        &after,
                        &expected_tags,
                        expected_title,
                        expected_description,
                    )?;
                    final_playlist = after.playlist;
                } else {
                    for (index, change) in changes.iter().copied().enumerate() {
                        let last = index + 1 == changes.len();
                        self.accept_read(&s.current, s.stored.as_ref(), &s.current)?;
                        s.dispatched = true;
                        self.client
                            .write_native_playlist_metadata(
                                &authorization,
                                id,
                                if last { title } else { None },
                                if last { description } else { None },
                                Some(change),
                            )
                            .await?;
                        self.accept_read(&s.current, s.stored.as_ref(), &s.current)?;
                        match change {
                            TagChange::Remove(removed) => {
                                expected_tags.retain(|tag| tag != removed)
                            }
                            TagChange::Add(added) => expected_tags.push(added.clone()),
                        }
                        let after = self
                            .read_selected_account_playlist(
                                Some(id),
                                &s.alias,
                                &mut s.current,
                                &mut s.stored,
                            )
                            .await?;
                        owner(&after.playlist, s.current.user_id())?;
                        confirm_readback(
                            &before,
                            &after,
                            &expected_tags,
                            if last {
                                expected_title
                            } else {
                                &before.playlist.name
                            },
                            if last {
                                expected_description
                            } else {
                                &before.playlist.description
                            },
                        )?;
                        match change {
                            TagChange::Remove(removed) => {
                                confirmed_tag_removals.push(removed.tag_name.clone())
                            }
                            TagChange::Add(added) => {
                                confirmed_tag_additions.push(added.tag_name.clone())
                            }
                        }
                        final_playlist = after.playlist;
                    }
                }
            }
            let after_created = self.write_created(&mut s).await?;
            if ids(&created) != ids(&after_created)
                || !after_created
                    .iter()
                    .any(|playlist| playlist.id == id && playlist.name == expected_title)
            {
                return Err(migu_upstream_error(
                    "Migu complete created library did not confirm the native playlist update",
                ));
            }
            self.write_finish_identity(favorite.as_deref(), &mut s)
                .await?;
            Ok(PlaylistMutationResult {
                playlist_ref: final_playlist.resource_ref.clone(),
                action: PlaylistMutationAction::Update,
                playlist: Some(final_playlist),
                extensions: Extensions::from([
                    (
                        "backend".into(),
                        json!("official_native_playlist_metadata_update"),
                    ),
                    (
                        "verified_by".into(),
                        json!("native_uid_and_complete_before_after_playlist_and_created_library"),
                    ),
                    ("source_user_id".into(), json!(s.current.user_id())),
                    ("existing_track_order_preserved".into(), json!(true)),
                    (
                        "confirmed_tag_removals".into(),
                        json!(confirmed_tag_removals.clone()),
                    ),
                    (
                        "confirmed_tag_additions".into(),
                        json!(confirmed_tag_additions.clone()),
                    ),
                    ("atomic".into(), json!(changes.len() <= 1)),
                ]),
            })
        })
        .await
        .map_err(|_| {
            error(
                ErrorCode::UpstreamTimeout,
                "Migu playlist metadata update exceeded its total deadline",
            )
        })
        .and_then(|value| value);
        let result = self.finish_playlist_write(&s, result).map_err(|failure| {
            if confirmed_tag_removals.is_empty() && confirmed_tag_additions.is_empty() {
                return failure;
            }
            let mut details = failure.details.as_object().cloned().unwrap_or_default();
            details.insert(
                "confirmed_tag_removals".into(),
                json!(confirmed_tag_removals),
            );
            details.insert(
                "confirmed_tag_additions".into(),
                json!(confirmed_tag_additions),
            );
            failure.with_details(serde_json::Value::Object(details))
        });
        pending.finished = true;
        result
    }
}
