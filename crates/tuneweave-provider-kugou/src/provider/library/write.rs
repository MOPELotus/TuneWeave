use super::tracks::{PlaylistSnapshot, Target};
use super::*;
use crate::KugouLoginClient;
use crate::account::library::write::{TrackInput, WRITE_BATCH_SIZE, Write, WriteAck};
use serde_json::Value;
use tuneweave_core::{
    PlaylistItemKind, PlaylistItemMutationAction, PlaylistItemMutationRequest,
    PlaylistItemMutationResult, ResourceRef, SubscriptionResult,
};

impl KugouProvider {
    pub(in crate::provider) async fn native_set_track_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        parse_album_audio_id(id)?;
        let result = self
            .write_native_playlist(
                Target::Favorites,
                &[id.to_owned()],
                if subscribed {
                    PlaylistItemMutationAction::Add
                } else {
                    PlaylistItemMutationAction::Remove
                },
                account,
            )
            .await?;
        let mut extensions = result.extensions;
        extensions.insert("favorite_playlist_ref".into(), json!(result.playlist_ref));
        Ok(SubscriptionResult {
            resource_ref: ResourceRef::new(Platform::Kugou, id)
                .map_err(|_| kugou_invalid_request("KuGou track ID is invalid"))?,
            subscribed,
            extensions,
        })
    }

    pub(in crate::provider) async fn native_mutate_playlist_tracks(
        &self,
        id: &str,
        action: PlaylistItemMutationAction,
        request: &PlaylistItemMutationRequest,
    ) -> Result<PlaylistItemMutationResult> {
        let (_, kind, _) = parse_reference(id)?;
        if kind != 0 {
            return Err(write_denied());
        }
        if request.kind != PlaylistItemKind::Track
            || request.item_refs.is_empty()
            || request.item_refs.len() > 100
        {
            return Err(kugou_invalid_request(
                "KuGou native playlist mutations require 1 to 100 distinct track references",
            ));
        }
        let mut seen = BTreeSet::new();
        let mut ids = Vec::new();
        for reference in &request.item_refs {
            if reference.platform() != Platform::Kugou || !seen.insert(reference.id()) {
                return Err(kugou_invalid_request(
                    "KuGou playlist write references must be distinct KuGou tracks",
                ));
            }
            parse_album_audio_id(reference.id())?;
            ids.push(reference.id().to_owned());
        }
        self.write_native_playlist(
            Target::Playlist(id),
            &ids,
            action,
            request.account.as_deref(),
        )
        .await
    }

    async fn write_native_playlist(
        &self,
        target: Target<'_>,
        ids: &[String],
        action: PlaylistItemMutationAction,
        account: Option<&str>,
    ) -> Result<PlaylistItemMutationResult> {
        let expected_uid = match target {
            Target::Playlist(id) => Some(parse_reference(id)?.0),
            Target::Favorites => None,
        };
        let account = account.unwrap_or("default");
        if action == PlaylistItemMutationAction::Add
            && matches!(self.selected(account)?, Some((KugouCredential::Native(v), _))
                if v.session.client == KugouLoginClient::Concept)
        {
            concept_add_scope(target, ids.len())?;
        }
        if action == PlaylistItemMutationAction::Remove
            && matches!(self.selected(account)?, Some((KugouCredential::Native(v), _))
                if v.session.client == KugouLoginClient::Concept)
        {
            concept_remove_scope(ids.len())?;
        }
        let mut read = self.begin_native_read(account, expected_uid).await?;
        let mut dispatched = 0usize;
        let result = async {
            let concept_remove = read.session()?.client == KugouLoginClient::Concept
                && action == PlaylistItemMutationAction::Remove;
            let concept_add = read.session()?.client == KugouLoginClient::Concept
                && action == PlaylistItemMutationAction::Add;
            if concept_add {
                concept_add_scope(target, ids.len())?;
            }
            if concept_remove {
                concept_remove_scope(ids.len())?;
            }
            let before = self.read_native_playlist(&mut read, target).await?;
            let (_, kind, list_id) = parse_reference(&before.playlist.id)?;
            if kind != 0 {
                return Err(write_denied());
            }
            if concept_add
                && before
                    .playlist
                    .extensions
                    .get("is_def")
                    .and_then(Value::as_u64)
                    != Some(0)
            {
                return Err(concept_add_unsupported());
            }
            let standard_ordinary_add = action == PlaylistItemMutationAction::Add
                && read.session()?.client == KugouLoginClient::Standard
                && matches!(target, Target::Playlist(_))
                && before
                    .playlist
                    .extensions
                    .get("is_def")
                    .and_then(Value::as_u64)
                    == Some(0);
            let standard_ordinary_remove = action == PlaylistItemMutationAction::Remove
                && read.session()?.client == KugouLoginClient::Standard
                && matches!(target, Target::Playlist(_))
                && before.playlist.extensions.get("is_def").and_then(Value::as_u64) == Some(0);
            if standard_ordinary_remove
                && before.playlist.extensions.get("is_mutual").and_then(Value::as_bool) != Some(false)
            {
                return Err(standard_remove_unsupported());
            }
            if concept_remove
                && before
                    .playlist
                    .extensions
                    .get("is_def")
                    .and_then(Value::as_u64)
                    != Some(match target {
                        Target::Playlist(_) => 0,
                        Target::Favorites => 2,
                    })
            {
                return Err(concept_remove_unsupported());
            }
            // h.e skips the second (cover) mutation for this exact builtin
            // name. Identity still comes solely from unique type0/is_def2.
            if concept_remove
                && matches!(target, Target::Favorites)
                && before.playlist.name != "我喜欢"
            {
                return Err(concept_remove_unsupported());
            }
            let wanted = ids.iter().map(String::as_str).collect::<BTreeSet<_>>();
            if concept_add && wanted.iter().any(|id| before.tracks.iter().filter(|t| t.id == *id).count() > 1) {
                return Err(concept_add_unsupported());
            }
            let present = before
                .tracks
                .iter()
                .map(|t| t.id.as_str())
                .collect::<BTreeSet<_>>();
            let added = ids
                .iter()
                .filter(|id| !present.contains(id.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            let removed = before
                .tracks
                .iter()
                .filter(|t| wanted.contains(t.id.as_str()))
                .map(file_id)
                .collect::<Result<Vec<_>>>()?;
            if concept_remove && removed.len() > 1 {
                return Err(concept_remove_unsupported());
            }
            let changed = match action {
                PlaylistItemMutationAction::Add => !added.is_empty(),
                PlaylistItemMutationAction::Remove => !removed.is_empty(),
            };
            if !changed {
                return mutation_result(before, ids, action, 0, 0);
            }
            let mut inputs = Vec::new();
            let mut concept_inputs = Vec::new();
            if action == PlaylistItemMutationAction::Add {
                if concept_add {
                    // Share the original lookup budget across the entire batch;
                    // neither bytes nor waiting grow by a factor of input count.
                    let mut used = 0usize;
                    let lookup = tokio::time::timeout(std::time::Duration::from_secs(30), async {
                        for id in &added {
                            let input = self.client.concept_album_track(parse_album_audio_id(id)?, |bytes| {
                                self.check_account_read(&mut read)?;
                                used = used.checked_add(bytes).ok_or_else(concept_lookup_budget)?;
                                if used > 4 * 1_048_576 { return Err(concept_lookup_budget()); }
                                Ok(())
                            }).await?;
                            concept_inputs.push(input);
                        }
                        Ok::<_, TuneWeaveError>(())
                    }).await;
                    self.check_account_read(&mut read)?;
                    lookup.map_err(|_| concept_lookup_budget())??;
                    let mut hashes = BTreeSet::new();
                    if concept_inputs.iter().any(|input| !hashes.insert(input.hash.to_ascii_lowercase())) {
                        return Err(concept_add_unsupported());
                    }
                }
                for id in &added {
                    if concept_add { continue; }
                    // This is public catalogue identity/asset lookup, not account playback.
                    let track = self
                        .client
                        .track_detail(parse_album_audio_id(id)?)
                        .await
                        .map_err(|mut e| {
                            if matches!(
                                e.code,
                                ErrorCode::AuthenticationRequired | ErrorCode::Conflict
                            ) {
                                e.code = ErrorCode::UpstreamError;
                            }
                            e
                        })?;
                    self.check_account_read(&mut read)?;
                    inputs.push(if standard_ordinary_add {
                        TrackInput::from_standard_catalogue(&track, id)?
                    } else {
                        TrackInput::from_catalogue(&track, id)?
                    });
                }
                // The catalogue lookup can take time. Recheck the complete source before
                // dispatch; no stale snapshot is used to decide which songs to add.
                let current = self.read_native_playlist(&mut read, target).await?;
                if revision(&current) != revision(&before) {
                    return Err(library_changed());
                }
            }
            let mut expected_version = before
                .playlist
                .extensions
                .get("list_ver")
                .and_then(Value::as_u64);
            if standard_ordinary_remove
                && (list_id > i32::MAX as u64
                    || expected_version.is_none_or(|v| v > i32::MAX as u64)
                    || removed.len() > WRITE_BATCH_SIZE
                    || removed.iter().any(|id| *id > i32::MAX as u64))
            {
                return Err(kugou_invalid_request(
                    "KuGou Standard removal requires a versioned playlist and at most 300 valid occurrences",
                ));
            }
            let mut last_ack = None;
            let mut concept_receipt = None;
            let mut standard_receipt = None;
            match action {
                PlaylistItemMutationAction::Add => {
                    self.check_account_read(&mut read)?;
                    let source = read.session()?;
                    let ack = if concept_add {
                        let version = expected_version.ok_or_else(concept_add_unsupported)?;
                        dispatched += 1;
                        let outcome = if let [input] = concept_inputs.as_slice() {
                            self.client.native_add_concept_track(source, list_id, version, input).await
                                .map(|ack| (ack.version, vec![(ack.file_id, ack.sort)]))
                        } else {
                            self.client.native_add_concept_tracks(source, list_id, version, &concept_inputs).await
                                .map(|(ack, items)| (ack, items.into_iter().map(|item| (item.file_id, item.sort)).collect()))
                        };
                        self.check_account_read(&mut read)?;
                        let (ack, receipts) = outcome?;
                        concept_receipt = Some(receipts);
                        ack
                    } else if standard_ordinary_add {
                        let version = expected_version.ok_or_else(library_changed)?;
                        dispatched += 1;
                        let outcome = if let [input] = inputs.as_slice() {
                            self.client
                                .native_add_standard_track(source, list_id, version, input)
                                .await
                                .map(|(ack, item)| (ack, vec![item]))
                        } else {
                            self.client
                                .native_add_standard_tracks(source, list_id, version, &inputs)
                                .await
                        };
                        self.check_account_read(&mut read)?;
                        let (ack, receipt) = outcome?;
                        standard_receipt = Some(receipt);
                        ack
                    } else {
                        dispatched += 1;
                        let outcome = self
                            .client
                            .native_write_tracks(source, list_id, Write::Add(&inputs))
                            .await;
                        self.check_account_read(&mut read)?;
                        outcome?
                    };
                    accept_ack(&ack, &mut expected_version)?;
                    last_ack = Some(ack);
                }
                PlaylistItemMutationAction::Remove => {
                    if concept_remove {
                        let version = expected_version.ok_or_else(concept_remove_unsupported)?;
                        self.check_account_read(&mut read)?;
                        dispatched += 1;
                        let ack = self
                            .client
                            .native_remove_concept_occurrence(
                                read.session()?,
                                list_id,
                                removed[0],
                                version,
                            )
                            .await;
                        self.check_account_read(&mut read)?;
                        let ack = ack?;
                        accept_ack(&ack, &mut expected_version)?;
                        last_ack = Some(ack);
                    } else if standard_ordinary_remove {
                        // Song removal is independent of Android's queued m0.k
                        // default-cover synchronization. Do not dispatch y1 or
                        // promise parity with that separate UI follow-up.
                        let version = expected_version.ok_or_else(library_changed)?;
                        self.check_account_read(&mut read)?;
                        dispatched += 1;
                        let outcome = self.client.native_remove_standard_tracks(
                            read.session()?, list_id, version, &removed,
                        ).await;
                        // Check source generation even when a delayed ACK fails.
                        self.check_account_read(&mut read)?;
                        let ack = outcome?;
                        accept_ack(&ack, &mut expected_version)?;
                        last_ack = Some(ack);
                    } else {
                        for chunk in removed.chunks(WRITE_BATCH_SIZE) {
                            self.check_account_read(&mut read)?;
                            let source = read.session()?;
                            dispatched += 1;
                            let ack = self
                                .client
                                .native_write_tracks(source, list_id, Write::Remove(chunk))
                                .await?;
                            self.check_account_read(&mut read)?;
                            accept_ack(&ack, &mut expected_version)?;
                            last_ack = Some(ack);
                        }
                    }
                }
            }
            let after = self.read_native_playlist(&mut read, target).await?;
            if standard_ordinary_remove
                && after.playlist.extensions.get("is_mutual").and_then(Value::as_bool) != Some(false)
            {
                return Err(library_changed());
            }
            verify_delta(&before, &after, &wanted, &added, action)?;
            if let Some(receipts) = standard_receipt {
                let mut position = None;
                for receipt in receipts {
                    let found = after
                        .tracks
                        .iter()
                        .position(|track| receipt.matches(track))
                        .ok_or_else(library_changed)?;
                    if position.is_some_and(|previous| previous >= found) {
                        return Err(library_changed());
                    }
                    position = Some(found);
                }
            }
            if let Some(receipts) = concept_receipt {
                let stable_track = |track: &Track| {
                    let mut track = track.clone();
                    for key in ["sort", "upstream_position", "playlist_position"] { track.extensions.remove(key); }
                    track
                };
                let old_tracks = before.tracks.iter().map(|t| Ok((file_id(t)?, stable_track(t))))
                    .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
                let retained = after.tracks.iter().filter_map(|track| match file_id(track) {
                    Ok(id) if old_tracks.contains_key(&id) => Some(Ok((id, stable_track(track)))),
                    Ok(_) => None,
                    Err(error) => Some(Err(error)),
                }).collect::<Result<std::collections::BTreeMap<_, _>>>()?;
                if retained != old_tracks { return Err(library_changed()); }
                if receipts.len() != concept_inputs.len() { return Err(library_changed()); }
                let mut previous_position = None;
                for ((file, sort), input) in receipts.into_iter().zip(&concept_inputs) {
                    let position = after.tracks.iter().position(|track|
                        track.extensions.get("file_id").and_then(Value::as_u64) == Some(file)
                    ).ok_or_else(library_changed)?;
                    let track = &after.tracks[position];
                    if previous_position.is_some_and(|previous| previous >= position)
                        || track.id != input.mixsongid.to_string()
                        || track.extensions.get("sort").and_then(Value::as_u64) != Some(sort)
                        || track.extensions.get("hash").and_then(Value::as_str).is_none_or(|hash| !hash.eq_ignore_ascii_case(&input.hash))
                        || track.album.as_ref().and_then(|album| album.resource_ref.as_ref())
                            .is_none_or(|album| album.platform() != Platform::Kugou || album.id() != input.album_id)
                    { return Err(library_changed()); }
                    previous_position = Some(position);
                }
            }
            if let Some(ack) = last_ack {
                if ack.version.is_some_and(|v| {
                    after
                        .playlist
                        .extensions
                        .get("list_ver")
                        .and_then(Value::as_u64)
                        != Some(v)
                }) || ack.count.is_some_and(|v| v != after.tracks.len() as u64)
                {
                    return Err(library_changed());
                }
            }
            mutation_result(
                after,
                ids,
                action,
                dispatched,
                match action {
                    PlaylistItemMutationAction::Add => added.len(),
                    PlaylistItemMutationAction::Remove => removed.len(),
                },
            )
        }
        .await;
        self.finish_account_read(read, result).map_err(|e| {
            if dispatched == 0 {
                return e;
            }
            let mut details = e.details.as_object().cloned().unwrap_or_default();
            details.insert("operation".into(), json!("playlist_tracks_mutation"));
            details.insert("write_outcome".into(), json!("unconfirmed"));
            details.insert("write_requests_dispatched".into(), json!(dispatched));
            e.retryable(false).with_details(Value::Object(details))
        })
    }
}

fn concept_lookup_budget() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        "KuGou Concept catalogue batch exceeded its shared read budget",
    )
    .with_platform(Platform::Kugou)
}

fn concept_add_scope(target: Target<'_>, count: usize) -> Result<()> {
    if !(1..=100).contains(&count) || !matches!(target, Target::Playlist(_)) {
        return Err(concept_add_unsupported());
    }
    Ok(())
}
fn standard_remove_unsupported() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::CapabilityNotSupported,
        "KuGou Standard removal requires an explicitly non-collaborative ordinary owned playlist",
    )
    .with_platform(Platform::Kugou)
}
fn concept_add_unsupported() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::CapabilityNotSupported,
        "KuGou Concept addition requires 1-100 unambiguous catalogue tracks in an ordinary owned playlist",
    )
    .with_platform(Platform::Kugou)
}

fn concept_remove_scope(count: usize) -> Result<()> {
    if count != 1 {
        return Err(concept_remove_unsupported());
    }
    Ok(())
}
fn concept_remove_unsupported() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::CapabilityNotSupported,
        "KuGou Concept removal supports one uniquely occurring track in an ordinary owned or liked-tracks playlist",
    )
    .with_platform(Platform::Kugou)
}

fn revision(snapshot: &PlaylistSnapshot) -> Option<&Value> {
    snapshot.playlist.extensions.get("source_snapshot_id")
}
fn accept_ack(ack: &WriteAck, expected: &mut Option<u64>) -> Result<()> {
    if expected
        .zip(ack.previous_version)
        .is_some_and(|(a, b)| a != b)
    {
        return Err(library_changed());
    }
    *expected = ack.version;
    Ok(())
}
fn file_id(track: &Track) -> Result<u64> {
    track
        .extensions
        .get("file_id")
        .and_then(Value::as_u64)
        .filter(|v| *v > 0)
        .ok_or_else(|| write_error("KuGou playlist occurrence omitted its file ID"))
}
fn stable_metadata(playlist: &Playlist) -> Value {
    let mut value = json!({"ref":playlist.resource_ref,"name":playlist.name,
        "description":playlist.description,"tags":playlist.tags,"creator":playlist.creator});
    for field in [
        "library_owner_id",
        "list_id",
        "list_type",
        "is_def",
        "global_collection_id",
        "source_user_id",
        "source_list_id",
        "source_global_collection_id",
        "is_private",
        "create_time",
    ] {
        value[field] = playlist
            .extensions
            .get(field)
            .cloned()
            .unwrap_or(Value::Null);
    }
    value
}
fn verify_delta(
    before: &PlaylistSnapshot,
    after: &PlaylistSnapshot,
    wanted: &BTreeSet<&str>,
    added: &[String],
    action: PlaylistItemMutationAction,
) -> Result<()> {
    if stable_metadata(&before.playlist) != stable_metadata(&after.playlist) {
        return Err(library_changed());
    }
    let pairs = |s: &PlaylistSnapshot| {
        s.tracks
            .iter()
            .map(|t| Ok((file_id(t)?, t.id.clone())))
            .collect::<Result<Vec<_>>>()
    };
    let old = pairs(before)?;
    let new = pairs(after)?;
    match action {
        PlaylistItemMutationAction::Add => {
            let old_ids = old.iter().map(|(id, _)| *id).collect::<BTreeSet<_>>();
            let retained = new
                .iter()
                .filter(|(id, _)| old_ids.contains(id))
                .cloned()
                .collect::<Vec<_>>();
            let mut new_songs = new
                .iter()
                .filter(|(id, _)| !old_ids.contains(id))
                .map(|(_, id)| id.clone())
                .collect::<Vec<_>>();
            let mut expected = added.to_vec();
            new_songs.sort();
            expected.sort();
            if retained != old || new_songs != expected {
                return Err(write_error(
                    "KuGou add readback did not confirm the complete requested delta",
                ));
            }
        }
        PlaylistItemMutationAction::Remove => {
            let retained = old
                .into_iter()
                .filter(|(_, id)| !wanted.contains(id.as_str()))
                .collect::<Vec<_>>();
            if retained != new {
                return Err(write_error(
                    "KuGou remove readback did not confirm all matching occurrences and retained order",
                ));
            }
        }
    }
    Ok(())
}
fn mutation_result(
    snapshot: PlaylistSnapshot,
    ids: &[String],
    action: PlaylistItemMutationAction,
    requests: usize,
    affected: usize,
) -> Result<PlaylistItemMutationResult> {
    let item_refs = ids
        .iter()
        .map(|id| {
            ResourceRef::new(Platform::Kugou, id)
                .map_err(|_| write_error("KuGou mutation result identity was invalid"))
        })
        .collect::<Result<Vec<_>>>()?;
    let snapshot_id = revision(&snapshot)
        .and_then(Value::as_str)
        .map(str::to_owned);
    Ok(PlaylistItemMutationResult {
        playlist_ref: snapshot.playlist.resource_ref,
        item_refs,
        kind: PlaylistItemKind::Track,
        action,
        snapshot_id,
        cloud_track_count: Some(snapshot.tracks.len() as u64),
        extensions: Extensions::from([
            ("backend".into(), json!("native_cloudlist_track_write")),
            (
                "verified_by".into(),
                json!(if requests == 0 {
                    "complete_native_playlist_state"
                } else {
                    "complete_native_playlist_delta"
                }),
            ),
            ("changed".into(), json!(requests > 0)),
            ("affected_occurrences".into(), json!(affected)),
            ("write_requests_dispatched".into(), json!(requests)),
            ("atomic".into(), json!(false)),
        ]),
    })
}
fn write_denied() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::PermissionDenied,
        "KuGou track mutations require a playlist in the selected account's created library",
    )
    .with_platform(Platform::Kugou)
}
fn write_error(message: &'static str) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message).with_platform(Platform::Kugou)
}

#[cfg(test)]
mod tests;
