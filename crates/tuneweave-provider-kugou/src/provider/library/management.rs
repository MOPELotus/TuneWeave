use super::*;
use crate::KugouLoginClient;
use crate::account::library::management::{Collection, ListAck, ListEdit, ListWrite};
use crate::provider::session::authentication_required;
use serde_json::Value;
use tuneweave_core::{
    PlaylistCreateRequest, PlaylistDeleteRequest, PlaylistDeleteResult, PlaylistKind,
    PlaylistMetadataUpdateVariant, PlaylistMutationAction, PlaylistMutationResult,
    PlaylistUpdateRequest, PlaylistVisibility, ResourceRef, SubscriptionResult,
};

mod cover;
mod visibility;

fn invalid(message: &'static str) -> TuneWeaveError {
    kugou_invalid_request(message)
}
fn unsupported(message: &'static str) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::CapabilityNotSupported, message).with_platform(Platform::Kugou)
}
fn denied() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::PermissionDenied,
        "KuGou management requires an ordinary playlist in the selected account's created library",
    )
    .with_platform(Platform::Kugou)
}
fn name(value: &str) -> Result<&str> {
    if value.trim().is_empty() || value.len() > 1024 || value.chars().any(char::is_control) {
        return Err(invalid("KuGou playlist name is invalid"));
    }
    Ok(value)
}
fn description(value: &str) -> Result<()> {
    if value.len() > 16384
        || value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
    {
        return Err(invalid("KuGou playlist description is invalid"));
    }
    Ok(())
}
fn tags(values: &[String]) -> Result<String> {
    if values.len() > 128
        || values.iter().any(|s| {
            s.trim().is_empty()
                || s.len() > 256
                || s.contains(',')
                || s.chars().any(char::is_control)
        })
    {
        return Err(invalid(
            "KuGou playlist tags cannot contain empty values, commas or control characters",
        ));
    }
    let joined = values.join(",");
    if joined.len() > 4096 {
        return Err(invalid("KuGou playlist tags are too long"));
    }
    Ok(joined)
}
fn ordinary(playlist: &Playlist) -> Result<()> {
    if parse_reference(&playlist.id)?.1 != 0 {
        return Err(denied());
    }
    match playlist.extensions.get("is_def").and_then(Value::as_u64) {
        Some(0) => Ok(()),
        Some(_) => Err(denied()),
        None => Err(unsupported(
            "KuGou playlist omitted its system-list classification",
        )),
    }
}
fn find<'a>(library: &'a LibrarySnapshot, id: &str) -> Result<&'a Playlist> {
    library
        .items
        .iter()
        .find(|(_, p)| p.id == id)
        .map(|(_, p)| p)
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::ResourceNotFound,
                "KuGou playlist is absent from the selected account library",
            )
            .with_platform(Platform::Kugou)
        })
}
fn stable(playlist: &Playlist) -> Playlist {
    let mut p = playlist.clone();
    for key in [
        "total_ver",
        "list_count",
        "collect_count",
        "album_count",
        "sort",
        "update_time",
    ] {
        p.extensions.remove(key);
    }
    p
}
fn unrelated(before: &LibrarySnapshot, after: &LibrarySnapshot, target: &str) -> Result<()> {
    unrelated_except(before, after, &BTreeSet::from([target]))
}
fn unrelated_except(
    before: &LibrarySnapshot,
    after: &LibrarySnapshot,
    targets: &BTreeSet<&str>,
) -> Result<()> {
    let retained = |s: &LibrarySnapshot| {
        s.items
            .iter()
            .filter(|(_, p)| !targets.contains(p.id.as_str()))
            .map(|(k, p)| (*k, stable(p)))
            .collect::<Vec<_>>()
    };
    let old = retained(before);
    let new = retained(after);
    let by_id = |rows: &[(u8, Playlist)]| {
        rows.iter()
            .map(|(kind, p)| (p.id.clone(), (*kind, p.clone())))
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    if by_id(&old) != by_id(&new) {
        return Err(library_changed());
    }
    for kind in [0, 1] {
        let positions = |s: &LibrarySnapshot| {
            s.items
                .iter()
                .filter(|(k, p)| *k == kind && !targets.contains(p.id.as_str()))
                .map(|(_, p)| {
                    (
                        p.extensions.get("sort").and_then(Value::as_u64),
                        p.id.clone(),
                    )
                })
                .collect::<Vec<_>>()
        };
        let mut old = positions(before);
        let mut new = positions(after);
        if old.iter().chain(&new).all(|(sort, _)| sort.is_some()) {
            old.sort_by_key(|(sort, _)| *sort);
            new.sort_by_key(|(sort, _)| *sort);
            if old
                .iter()
                .map(|(_, id)| id)
                .ne(new.iter().map(|(_, id)| id))
            {
                return Err(library_changed());
            }
        } else if old != new {
            return Err(library_changed());
        }
    }
    Ok(())
}
fn acknowledge(
    ack: &ListAck,
    before: &LibrarySnapshot,
    after: &LibrarySnapshot,
    target: Option<&Playlist>,
) -> Result<()> {
    if after.version.total_ver < before.version.total_ver
        || ack
            .previous_ver
            .is_some_and(|v| v != before.version.total_ver)
        || ack.total_ver.is_some_and(|v| v != after.version.total_ver)
        || ack.list_count.is_some_and(|v| {
            after
                .version
                .extensions()
                .get("list_count")
                .and_then(Value::as_u64)
                != Some(v)
        })
        || ack.gid.as_ref().is_some_and(|v| {
            target
                .and_then(|p| p.extensions.get("global_collection_id"))
                .and_then(Value::as_str)
                != Some(v.as_str())
        })
    {
        return Err(library_changed());
    }
    Ok(())
}
fn write_error(
    mut error: TuneWeaveError,
    dispatched: usize,
    confirmed: &[ResourceRef],
    attempted: &[ResourceRef],
    remaining: &[ResourceRef],
) -> TuneWeaveError {
    if dispatched == 0 {
        return error;
    }
    let mut details = error.details.as_object().cloned().unwrap_or_default();
    details.insert("operation".into(), json!("playlist_management"));
    details.insert("write_outcome".into(), json!("unconfirmed"));
    details.insert("write_requests_dispatched".into(), json!(dispatched));
    details.insert("confirmed_refs".into(), json!(confirmed));
    details.insert(
        "unconfirmed_refs".into(),
        json!(
            attempted
                .iter()
                .filter(|r| !confirmed.contains(r))
                .collect::<Vec<_>>()
        ),
    );
    details.insert("not_attempted_refs".into(), json!(remaining));
    error = error.retryable(false).with_details(Value::Object(details));
    error
}
fn result(
    playlist: Playlist,
    action: PlaylistMutationAction,
    dispatched: usize,
    version: u64,
) -> PlaylistMutationResult {
    PlaylistMutationResult {
        playlist_ref: playlist.resource_ref.clone(),
        playlist: Some(playlist),
        action,
        extensions: Extensions::from([
            ("verified_by".into(), json!("complete_native_library_delta")),
            ("changed".into(), json!(dispatched > 0)),
            ("write_requests_dispatched".into(), json!(dispatched)),
            ("total_ver".into(), json!(version)),
            ("atomic".into(), json!(false)),
        ]),
    }
}
fn prepared_edit(
    before: &Playlist,
    request: &PlaylistUpdateRequest,
    version: u64,
) -> Result<ListEdit> {
    let ext = &before.extensions;
    let missing = || {
        unsupported("KuGou metadata update cannot preserve fields missing from the current library")
    };
    let intro = request
        .description
        .clone()
        .or_else(|| {
            ext.get("native_metadata")?
                .get("intro")?
                .as_str()
                .map(str::to_owned)
        })
        .ok_or_else(missing)?;
    let tags = match &request.tags {
        Some(t) => tags(t)?,
        None => ext
            .get("native_metadata")
            .and_then(|m| m.get("tags"))
            .and_then(Value::as_str)
            .map(str::to_owned)
            .ok_or_else(missing)?,
    };
    Ok(ListEdit {
        name: request.name.clone().unwrap_or_else(|| before.name.clone()),
        intro,
        tags,
        private: ext
            .get("is_private")
            .and_then(Value::as_bool)
            .ok_or_else(missing)?,
        sort: ext
            .get("sort")
            .and_then(Value::as_u64)
            .ok_or_else(missing)?,
        total_ver: version,
    })
}
fn metadata_matches(before: &Playlist, after: &Playlist, edit: &ListEdit) -> Result<()> {
    let mut expected = before.clone();
    expected.name = edit.name.clone();
    expected.description = edit.intro.clone();
    // Read mapper normalizes blank values; exact originals remain in native_metadata.
    if expected.description.trim().is_empty() {
        expected.description.clear();
    }
    expected.tags = if edit.tags.trim().is_empty() {
        vec![]
    } else {
        edit.tags.split(',').map(str::to_owned).collect()
    };
    expected.extensions.insert(
        "native_metadata".into(),
        json!({"intro":edit.intro,"tags":edit.tags}),
    );
    expected
        .extensions
        .insert("is_private".into(), json!(edit.private));
    let mut a = stable(&expected);
    let mut b = stable(after);
    // Metadata edits may advance the per-list version while keeping all contents.
    a.extensions.remove("list_ver");
    b.extensions.remove("list_ver");
    if a != b || after.extensions.get("sort").and_then(Value::as_u64) != Some(edit.sort) {
        return Err(library_changed());
    }
    Ok(())
}

fn creation_visibility(
    client: KugouLoginClient,
    request: &PlaylistCreateRequest,
) -> Result<Option<bool>> {
    match (client, request.visibility) {
        (KugouLoginClient::Standard, PlaylistVisibility::Public) => Ok(Some(false)),
        (KugouLoginClient::Standard, PlaylistVisibility::Private) => Ok(Some(true)),
        (KugouLoginClient::Concept, PlaylistVisibility::PlatformDefault) => {
            if request.name.len() > 60 {
                return Err(invalid(
                    "KuGou Concept playlist names cannot exceed 60 UTF-8 bytes",
                ));
            }
            Ok(None)
        }
        (KugouLoginClient::Concept, _) => Err(unsupported(
            "KuGou Concept creation supports only platform-default visibility; public or private access is not guaranteed",
        )),
        _ => Err(unsupported(
            "KuGou Standard creation requires explicit public or private visibility",
        )),
    }
}

impl KugouProvider {
    pub(in crate::provider) async fn native_create_playlist(
        &self,
        request: &PlaylistCreateRequest,
    ) -> Result<PlaylistMutationResult> {
        name(&request.name)?;
        if request.kind != PlaylistKind::Normal {
            return Err(unsupported(
                "KuGou native creation supports ordinary song playlists",
            ));
        }
        let account = request.account.as_deref().unwrap_or("default");
        let (selected, _) = self
            .selected(account)?
            .ok_or_else(authentication_required)?;
        let client = match &selected {
            KugouCredential::Native(v) => v.session.client,
            KugouCredential::Web(_) => {
                return Err(unsupported(
                    "KuGou playlist creation requires a native app credential",
                ));
            }
        };
        creation_visibility(client, request)?;
        let mut read = self.begin_native_read(account, None).await?;
        let mut dispatched = 0;
        let outcome = async {
            // Recheck the refreshed selection after the no-network preflight.
            let private = creation_visibility(read.session()?.client, request)?;
            let before = self.read_native_library(&mut read).await?;
            self.check_account_read(&mut read)?;
            dispatched += 1;
            let ack = if let Some(private) = private {
                self.client
                    .native_write_list(
                        read.session()?,
                        ListWrite::Add {
                            name: &request.name,
                            private,
                            source: None,
                        },
                    )
                    .await?
            } else {
                let ack = self
                    .client
                    .native_create_concept_list(
                        read.session()?,
                        &request.name,
                        before.version.total_ver,
                    )
                    .await;
                self.check_account_read(&mut read)?;
                ack?
            };
            self.check_account_read(&mut read)?;
            let id = format!(
                "cloudlist:{}:0:{}",
                read.session()?.user_id,
                ack.list_id.ok_or_else(library_changed)?
            );
            if before.items.iter().any(|(_, p)| p.id == id) {
                return Err(library_changed());
            }
            let after = self.read_native_library(&mut read).await?;
            let created = find(&after, &id)?;
            if created.name != request.name
                || private.is_some_and(|private| {
                    created
                        .extensions
                        .get("is_private")
                        .and_then(Value::as_bool)
                        != Some(private)
                })
                || created.track_count != Some(0)
                || created
                    .extensions
                    .get("m_count")
                    .and_then(Value::as_u64)
                    .is_some_and(|n| n != 0)
            {
                return Err(library_changed());
            }
            unrelated(&before, &after, &id)?;
            acknowledge(&ack, &before, &after, Some(created))?;
            // This exception is limited to the item proven by this create ACK and the
            // complete, single-item library delta. Every later management operation
            // continues to require an explicit is_def=0 marker via ordinary().
            if created
                .extensions
                .get("is_def")
                .is_some_and(|marker| marker.as_u64() != Some(0))
            {
                return Err(denied());
            }
            let mut result = result(
                created.clone(),
                PlaylistMutationAction::Create,
                dispatched,
                after.version.total_ver,
            );
            if private.is_none() {
                result
                    .extensions
                    .insert("requested_visibility".into(), json!("platform_default"));
                result
                    .extensions
                    .insert("visibility_guaranteed".into(), json!(false));
            }
            Ok(result)
        }
        .await;
        self.finish_account_read(read, outcome)
            .map_err(|e| write_error(e, dispatched, &[], &[], &[]))
    }
    pub(in crate::provider) async fn native_update_playlist(
        &self,
        id: &str,
        request: &PlaylistUpdateRequest,
    ) -> Result<PlaylistMutationResult> {
        let (uid, kind, _) = parse_reference(id)?;
        if kind != 0 {
            return Err(denied());
        }
        if request.variant != PlaylistMetadataUpdateVariant::Default {
            return Err(unsupported(
                "KuGou supports the default metadata update protocol",
            ));
        }
        if request.name.is_none() && request.description.is_none() && request.tags.is_none() {
            return Err(invalid("KuGou metadata update requires at least one field"));
        }
        if let Some(v) = &request.name {
            name(v)?;
        }
        if let Some(v) = &request.description {
            description(v)?;
        }
        if let Some(v) = &request.tags {
            tags(v)?;
        }
        let mut read = self
            .begin_native_read(request.account.as_deref().unwrap_or("default"), Some(uid))
            .await?;
        let mut dispatched = 0;
        let outcome = async {
            let before = self.read_native_library(&mut read).await?;
            let selected = find(&before, id)?;
            ordinary(selected)?;
            let edit = prepared_edit(selected, request, before.version.total_ver)?;
            if metadata_matches(selected, selected, &edit).is_ok() {
                return Ok(result(
                    selected.clone(),
                    PlaylistMutationAction::Update,
                    0,
                    before.version.total_ver,
                ));
            }
            self.check_account_read(&mut read)?;
            dispatched += 1;
            let ack = self
                .client
                .native_write_list(
                    read.session()?,
                    ListWrite::Modify {
                        list_id: parse_reference(id)?.2,
                        edit: &edit,
                    },
                )
                .await;
            self.check_account_read(&mut read)?;
            let ack = ack?;
            let after = self.read_native_library(&mut read).await?;
            let updated = find(&after, id)?;
            metadata_matches(selected, updated, &edit)?;
            unrelated(&before, &after, id)?;
            acknowledge(&ack, &before, &after, Some(updated))?;
            Ok(result(
                updated.clone(),
                PlaylistMutationAction::Update,
                dispatched,
                after.version.total_ver,
            ))
        }
        .await;
        let refs = [ResourceRef::new(Platform::Kugou, id).map_err(|_| library_changed())?];
        self.finish_account_read(read, outcome)
            .map_err(|e| write_error(e, dispatched, &[], &refs, &[]))
    }
    pub(in crate::provider) async fn native_delete_playlists(
        &self,
        request: &PlaylistDeleteRequest,
    ) -> Result<PlaylistDeleteResult> {
        if request.playlist_refs.is_empty() || request.playlist_refs.len() > 100 {
            return Err(invalid(
                "KuGou playlist deletion requires 1 to 100 distinct references",
            ));
        }
        let mut seen = BTreeSet::new();
        let mut owner = None;
        for r in &request.playlist_refs {
            if r.platform() != Platform::Kugou || !seen.insert(r.id()) {
                return Err(invalid(
                    "KuGou playlist deletion references must be distinct KuGou playlists",
                ));
            }
            let (uid, kind, _) = parse_reference(r.id())?;
            if kind != 0 {
                return Err(denied());
            }
            if owner.is_some_and(|v| v != uid) {
                return Err(invalid("KuGou playlist deletion cannot span accounts"));
            }
            owner = Some(uid);
        }
        let account = request.account.as_deref().unwrap_or("default");
        let mut read = self.begin_native_read(account, owner).await?;
        let mut dispatched = 0;
        let mut confirmed = Vec::new();
        let mut attempted = Vec::new();
        let outcome = async {
            let concept = read.session()?.client == KugouLoginClient::Concept;
            let mut before = self.read_native_library(&mut read).await?;
            // Validate the whole batch before the first irreversible request.
            for r in &request.playlist_refs {
                ordinary(find(&before, r.id())?)?;
            }
            if concept && request.playlist_refs.len() > 1 {
                let ids = request
                    .playlist_refs
                    .iter()
                    .map(|r| parse_reference(r.id()).map(|(_, _, id)| id))
                    .collect::<Result<Vec<_>>>()?;
                let targets = request
                    .playlist_refs
                    .iter()
                    .map(|r| r.id())
                    .collect::<BTreeSet<_>>();
                self.check_account_read(&mut read)?;
                attempted.extend(request.playlist_refs.iter().cloned());
                dispatched += 1;
                let ack = self
                    .client
                    .native_delete_concept_lists(read.session()?, &ids, before.version.total_ver)
                    .await;
                self.check_account_read(&mut read)?;
                let ack = ack?;
                let after = self.read_native_library(&mut read).await?;
                if after
                    .items
                    .iter()
                    .any(|(_, p)| targets.contains(p.id.as_str()))
                {
                    return Err(library_changed());
                }
                unrelated_except(&before, &after, &targets)?;
                acknowledge(&ack, &before, &after, None)?;
                self.check_account_read(&mut read)?;
                confirmed.extend(request.playlist_refs.iter().cloned());
                return Ok(PlaylistDeleteResult {
                    playlist_refs: confirmed.clone(),
                    extensions: Extensions::from([
                        ("verified_by".into(), json!("complete_native_library_delta")),
                        ("atomic".into(), json!(false)),
                        ("write_requests_dispatched".into(), json!(dispatched)),
                        ("total_ver".into(), json!(after.version.total_ver)),
                    ]),
                });
            }
            for r in &request.playlist_refs {
                self.check_account_read(&mut read)?;
                attempted.push(r.clone());
                dispatched += 1;
                let ack = self
                    .client
                    .native_write_list(
                        read.session()?,
                        ListWrite::Delete {
                            list_id: parse_reference(r.id())?.2,
                            kind: 0,
                            total_ver: before.version.total_ver,
                        },
                    )
                    .await;
                if concept {
                    // A late failed write is as stale as a late successful ACK.
                    self.check_account_read(&mut read)?;
                }
                let ack = ack?;
                self.check_account_read(&mut read)?;
                let after = self.read_native_library(&mut read).await?;
                if after.items.iter().any(|(_, p)| p.id == r.id()) {
                    return Err(library_changed());
                }
                unrelated(&before, &after, r.id())?;
                acknowledge(&ack, &before, &after, Some(find(&before, r.id())?))?;
                self.check_account_read(&mut read)?;
                confirmed.push(r.clone());
                before = after;
            }
            Ok(PlaylistDeleteResult {
                playlist_refs: confirmed.clone(),
                extensions: Extensions::from([
                    ("verified_by".into(), json!("complete_native_library_delta")),
                    ("atomic".into(), json!(false)),
                    ("write_requests_dispatched".into(), json!(dispatched)),
                    ("total_ver".into(), json!(before.version.total_ver)),
                ]),
            })
        }
        .await;
        self.finish_account_read(read, outcome).map_err(|e| {
            write_error(
                e,
                dispatched,
                &confirmed,
                &attempted,
                &request.playlist_refs[attempted.len()..],
            )
        })
    }
}

fn collected<'a>(library: &'a LibrarySnapshot, gid: &str) -> Result<Option<&'a Playlist>> {
    let mut found = None;
    for (_, p) in library.items.iter().filter(|(kind, _)| *kind == 1) {
        let source=p.extensions.get("source_global_collection_id").and_then(Value::as_str)
            .ok_or_else(||unsupported("KuGou collection state cannot be established without source collection identities"))?;
        if source == gid {
            if found.is_some() {
                return Err(library_changed());
            }
            found = Some(p);
        }
    }
    Ok(found)
}
impl KugouProvider {
    pub(in crate::provider) async fn native_set_playlist_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        let qualified = if id.starts_with("cloudlist:") {
            let parsed = parse_reference(id)?;
            if parsed.1 != 1 {
                return Err(denied());
            }
            Some(parsed)
        } else {
            crate::client::validate_collection_id(id)?;
            None
        };
        let mut read = self
            .begin_native_read(account.unwrap_or("default"), qualified.map(|v| v.0))
            .await?;
        let mut dispatched = 0;
        let mut affected = Vec::new();
        let outcome = async {
            let before = self.read_native_library(&mut read).await?;
            let selected = if qualified.is_some() {
                Some(find(&before, id)?)
            } else {
                collected(&before, id)?
            };
            if selected.is_some() == subscribed {
                return Ok(SubscriptionResult {
                    resource_ref: ResourceRef::new(Platform::Kugou, id)
                        .map_err(|_| library_changed())?,
                    subscribed,
                    extensions: Extensions::from([
                        ("changed".into(), json!(false)),
                        ("verified_by".into(), json!("complete_native_library_state")),
                        ("write_requests_dispatched".into(), json!(0)),
                        (
                            "account_playlist_ref".into(),
                            json!(selected.map(|p| &p.resource_ref)),
                        ),
                    ]),
                });
            }
            let (after, account_ref) = if let Some(selected) = selected {
                let reference = selected.resource_ref.clone();
                affected.push(reference.clone());
                self.check_account_read(&mut read)?;
                dispatched += 1;
                let ack = self
                    .client
                    .native_write_list(
                        read.session()?,
                        ListWrite::Delete {
                            list_id: parse_reference(&selected.id)?.2,
                            kind: 1,
                            total_ver: before.version.total_ver,
                        },
                    )
                    .await;
                // A late failed Concept unsubscription must not invalidate a
                // replacement login or expose its predecessor's credentials.
                if read.session()?.client == KugouLoginClient::Concept {
                    self.check_account_read(&mut read)?;
                }
                let ack = ack?;
                self.check_account_read(&mut read)?;
                let after = self.read_native_library(&mut read).await?;
                if after.items.iter().any(|(_, p)| p.id == selected.id) {
                    return Err(library_changed());
                }
                if qualified.is_none() && collected(&after, id)?.is_some() {
                    return Err(library_changed());
                }
                unrelated(&before, &after, &selected.id)?;
                acknowledge(&ack, &before, &after, Some(selected))?;
                (after, reference)
            } else {
                // Resolve no-op membership from the complete account library first.
                // Concept new collections require a separate, unverified song-sync
                // protocol; do not look up the public source or dispatch a write.
                if read.session()?.client == KugouLoginClient::Concept {
                    return Err(unsupported(
                        "KuGou Concept does not support creating a new playlist subscription",
                    ));
                }
                let public = self.client.playlist_detail(id).await.map_err(|mut e| {
                    if matches!(
                        e.code,
                        ErrorCode::AuthenticationRequired | ErrorCode::Conflict
                    ) {
                        e.code = ErrorCode::UpstreamError;
                    }
                    e
                })?;
                self.check_account_read(&mut read)?;
                name(&public.name)?;
                let source_uid = public
                    .creator
                    .as_ref()
                    .and_then(|p| p.resource_ref.as_ref())
                    .filter(|r| r.platform() == Platform::Kugou)
                    .and_then(|r| r.id().parse::<u64>().ok())
                    .filter(|v| *v > 0)
                    .ok_or_else(|| {
                        unsupported("KuGou public playlist omitted its source owner identity")
                    })?;
                if source_uid.to_string() == read.session()?.user_id {
                    return Err(denied());
                }
                let current = self.read_native_library(&mut read).await?;
                if current.version != before.version || current.items != before.items {
                    return Err(library_changed());
                }
                self.check_account_read(&mut read)?;
                dispatched += 1;
                let ack = self
                    .client
                    .native_write_list(
                        read.session()?,
                        ListWrite::Add {
                            name: &public.name,
                            private: false,
                            source: Some(Collection {
                                user_id: source_uid,
                                list_id: 0,
                                gid: id,
                            }),
                        },
                    )
                    .await?;
                self.check_account_read(&mut read)?;
                let local_id = format!(
                    "cloudlist:{}:1:{}",
                    read.session()?.user_id,
                    ack.list_id.ok_or_else(library_changed)?
                );
                if before.items.iter().any(|(_, p)| p.id == local_id) {
                    return Err(library_changed());
                }
                let after = self.read_native_library(&mut read).await?;
                let added = find(&after, &local_id)?;
                if collected(&after, id)?.map(|p| p.id.as_str()) != Some(local_id.as_str())
                    || added
                        .extensions
                        .get("source_user_id")
                        .and_then(Value::as_str)
                        != Some(source_uid.to_string().as_str())
                {
                    return Err(library_changed());
                }
                unrelated(&before, &after, &local_id)?;
                acknowledge(&ack, &before, &after, Some(added))?;
                let reference = added.resource_ref.clone();
                (after, reference)
            };
            Ok(SubscriptionResult {
                resource_ref: ResourceRef::new(Platform::Kugou, id)
                    .map_err(|_| library_changed())?,
                subscribed,
                extensions: Extensions::from([
                    ("changed".into(), json!(true)),
                    ("verified_by".into(), json!("complete_native_library_delta")),
                    ("write_requests_dispatched".into(), json!(dispatched)),
                    ("atomic".into(), json!(false)),
                    ("account_playlist_ref".into(), json!(account_ref)),
                    ("total_ver".into(), json!(after.version.total_ver)),
                ]),
            })
        }
        .await;
        self.finish_account_read(read, outcome)
            .map_err(|e| write_error(e, dispatched, &[], &affected, &[]))
    }
}

#[cfg(test)]
mod tests;
