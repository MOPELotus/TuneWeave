//! Independent cloud writes, acknowledged only after selected-account readback.
use super::*;
use library::Section;
use tuneweave_core::{
    PlaylistCreateRequest, PlaylistDeleteRequest, PlaylistDeleteResult, PlaylistKind,
    PlaylistMutationAction, PlaylistMutationResult, PlaylistUpdateRequest, PlaylistVisibility,
    PlaylistVisibilityUpdateRequest, ProviderCredential,
};

pub(crate) mod collected_sort;
pub(crate) mod collections;
pub(crate) mod contribution;
pub(crate) mod cover;
pub(crate) mod edit;
pub(crate) mod favorites;
pub(crate) mod items;
pub(crate) mod library_sort;
pub(crate) mod sorting;
pub(crate) mod submission_delete;

#[cfg(test)]
pub(crate) mod tests;

const HOST: &str = "nplserver.kuwo.cn";
const ACK_LIMIT: usize = 64 * 1024;

fn playlist_name_width(name: &str) -> f32 {
    name.encode_utf16()
        .map(|unit| {
            if (19_968..=40_869).contains(&unit) {
                1.0
            } else {
                0.5
            }
        })
        .sum()
}

pub(crate) enum Mutation<'a> {
    Collection(&'a str, bool, Option<&'a str>),
    Create(&'a PlaylistCreateRequest),
    Delete(&'a PlaylistDeleteRequest),
    Update(&'a str, &'a PlaylistUpdateRequest),
    Visibility(&'a str, &'a PlaylistVisibilityUpdateRequest),
    Items(items::Change<'a>),
    Sort(&'a str, &'a tuneweave_core::PlaylistTrackOrderRequest),
    LibrarySort(&'a tuneweave_core::PlaylistOrderRequest),
    CollectedSort(&'a tuneweave_core::PlaylistOrderRequest),
}
pub(crate) enum Outcome {
    Playlist(Box<PlaylistMutationResult>),
    Deleted(PlaylistDeleteResult),
    Items(Box<tuneweave_core::PlaylistItemMutationResult>),
    TrackOrder(Box<tuneweave_core::PlaylistTrackOrderResult>),
    LibraryOrder(tuneweave_core::PlaylistOrderResult),
    Subscription(tuneweave_core::SubscriptionResult),
}
impl Mutation<'_> {
    pub(crate) fn validate(&self) -> Result<()> {
        match self {
            Self::Collection(id, _, _) => playlist::validate_id(id)?,
            Self::Items(change) => change.validate()?,
            Self::LibrarySort(request) => library_sort::validate(request)?,
            Self::CollectedSort(request) => collected_sort::validate(request)?,
            Self::Sort(id, request) => sorting::validate(id, request)?,
            Self::Create(r) => {
                if r.name.trim().is_empty()
                    || playlist_name_width(&r.name) > 20.0
                    || r.name.chars().any(char::is_control)
                {
                    return Err(kuwo_invalid_request("Kuwo playlist name is invalid"));
                }
                if r.kind != PlaylistKind::Normal || r.visibility != PlaylistVisibility::Public {
                    return Err(TuneWeaveError::new(
                        ErrorCode::CapabilityNotSupported,
                        "Kuwo native creation currently supports explicitly public ordinary playlists",
                    ).with_platform(Platform::Kuwo));
                }
            }
            Self::Delete(r) => {
                let mut seen = std::collections::BTreeSet::new();
                if !(1..=100).contains(&r.playlist_refs.len()) {
                    return Err(kuwo_invalid_request(
                        "Kuwo deletion requires 1 to 100 playlists",
                    ));
                }
                for reference in &r.playlist_refs {
                    if reference.platform() != Platform::Kuwo || !seen.insert(reference.id()) {
                        return Err(kuwo_invalid_request("Kuwo deletion references are invalid"));
                    }
                    playlist::validate_id(reference.id())?;
                }
            }
            Self::Update(id, r) => edit::validate_update(id, r)?,
            Self::Visibility(id, r) => {
                playlist::validate_id(id)?;
                r.validate().map_err(|e| e.with_platform(Platform::Kuwo))?;
            }
        }
        Ok(())
    }
    pub(crate) fn account(&self) -> Option<&str> {
        match self {
            Self::Collection(_, _, account) => *account,
            Self::Create(r) => r.account.as_deref(),
            Self::Delete(r) => r.account.as_deref(),
            Self::Update(_, r) => r.account.as_deref(),
            Self::Visibility(_, r) => r.account.as_deref(),
            Self::Items(change) => change.request.account.as_deref(),
            Self::Sort(_, request) => request.account.as_deref(),
            Self::LibrarySort(request) | Self::CollectedSort(request) => request.account.as_deref(),
        }
    }
}

impl KuwoClient {
    /// Creates one empty public ordinary playlist and verifies its new cloud ID.
    /// An unconfirmed error after dispatch does not mean no playlist was created.
    pub async fn native_create_playlist(
        &self,
        credential: &ProviderCredential,
        request: &PlaylistCreateRequest,
    ) -> Result<PlaylistMutationResult> {
        match self
            .native_mutation(credential, Mutation::Create(request))
            .await?
        {
            Outcome::Playlist(result) => Ok(*result),
            Outcome::Deleted(_)
            | Outcome::Items(_)
            | Outcome::TrackOrder(_)
            | Outcome::LibraryOrder(_)
            | Outcome::Subscription(_) => {
                unreachable!("create operation")
            }
        }
    }

    /// Deletes a batch only from the selected account's ordinary created lists.
    /// This performs one write and one readback; it is not an atomic transaction.
    pub async fn native_delete_playlists(
        &self,
        credential: &ProviderCredential,
        request: &PlaylistDeleteRequest,
    ) -> Result<PlaylistDeleteResult> {
        match self
            .native_mutation(credential, Mutation::Delete(request))
            .await?
        {
            Outcome::Deleted(result) => Ok(result),
            Outcome::Playlist(_)
            | Outcome::Items(_)
            | Outcome::TrackOrder(_)
            | Outcome::LibraryOrder(_)
            | Outcome::Subscription(_) => {
                unreachable!("delete operation")
            }
        }
    }

    async fn native_mutation(
        &self,
        credential: &ProviderCredential,
        mutation: Mutation<'_>,
    ) -> Result<Outcome> {
        mutation.validate()?;
        if mutation.account().is_some_and(|v| v != "default") {
            return Err(kuwo_invalid_request(
                "Kuwo SDK credentials cannot select a stored account",
            ));
        }
        let input = credential::NativeCredential::parse(credential)?.input()?;
        validate_session_metadata(&input)?;
        self.validate_native_session(&input).await?;
        let mut dispatched = false;
        self.perform_native_mutation(&input, mutation, &mut dispatched, || Ok(()))
            .await
            .map_err(|e| write_failure(e, dispatched))
    }

    pub(crate) async fn perform_native_mutation(
        &self,
        input: &KuwoNativeSessionInput,
        mutation: Mutation<'_>,
        dispatched: &mut bool,
        mut check: impl FnMut() -> Result<()> + Send,
    ) -> Result<Outcome> {
        tokio::time::timeout(Duration::from_secs(120), async {
            if let Mutation::CollectedSort(request) = &mutation {
                return self.perform_native_collected_sort(input, request, dispatched, &mut check).await.map(Outcome::LibraryOrder);
            }
            if let Mutation::Collection(id, subscribed, _) = &mutation {
                return self.perform_native_collection(input, id, *subscribed, dispatched, &mut check).await.map(Outcome::Subscription);
            }
            if let Mutation::LibrarySort(request) = &mutation {
                return self.perform_native_library_sort(input, request, dispatched, &mut check).await.map(Outcome::LibraryOrder);
            }
            if let Mutation::Sort(id, request) = &mutation {
                return self.perform_native_track_sort(input, id, request, dispatched, &mut check)
                    .await.map(|value| Outcome::TrackOrder(Box::new(value)));
            }
            if let Mutation::Items(change) = &mutation {
                return self.perform_native_item_mutation(input, change, dispatched, &mut check)
                    .await.map(|value| Outcome::Items(Box::new(value)));
            }
            check()?;
            let before = self.native_management_directory(input).await;
            check()?;
            let (before, before_ids) = before?;
            let edit = match &mutation {
                Mutation::Update(id, request) => Some((*id, edit::Edit::Metadata(request))),
                Mutation::Visibility(id, request) => Some((*id, edit::Edit::Visibility(request))),
                _ => None,
            };
            if let Some((id, request)) = edit {
                return self.perform_native_update(input, id, request, before, dispatched, &mut check)
                    .await.map(Outcome::Playlist);
            }
            let (operation, body) = match &mutation {
                Mutation::Create(r) => ("pl3_addlist", json!({
                    "title":r.name,"tag":"","pic":"","intro":"","data":[],"ispub":true,"turn":0
                })),
                Mutation::Delete(r) => {
                    // Preflight the entire batch before dispatching any mutation.
                    for reference in &r.playlist_refs {
                        if !before.iter().any(|p| p.id == reference.id()
                            && p.extensions.get("library_section") == Some(&json!("created"))) {
                            return Err(TuneWeaveError::new(ErrorCode::PermissionDenied,
                                "Kuwo deletion requires ordinary playlists owned by the selected account")
                                .with_platform(Platform::Kuwo));
                        }
                    }
                    let ids = r.playlist_refs.iter().map(|r| r.id().parse::<i64>()
                        .map_err(|_| kuwo_invalid_request("Kuwo playlist ID is invalid")))
                        .collect::<Result<Vec<_>>>()?;
                    ("pl3_deletelist", json!({"plist":ids}))
                }
                Mutation::Update(_, _) | Mutation::Visibility(_, _) | Mutation::Items(_) | Mutation::Sort(_, _) | Mutation::LibrarySort(_) | Mutation::CollectedSort(_) | Mutation::Collection(_, _, _) => unreachable!("update handled above"),
            };
            check()?;
            let result = self.native_cloud_write(input, operation, body, dispatched).await;
            check()?;
            let ack = result?;
            if matches!(mutation, Mutation::Create(_)) && ack.pid.as_ref().is_some_and(|id| before_ids.contains(id)) {
                return Err(unconfirmed());
            }
            check()?;
            let after = self.native_management_directory(input).await;
            check()?;
            let (after, after_ids) = after?;
            let extensions = Extensions::from([
                ("backend".into(),json!("native_account_library")),
                ("library_owner_id".into(),json!(input.user_id())),
                ("confirmed".into(),json!(true)),
                ("write_requests_dispatched".into(),json!(1)),
                ("consistency".into(),json!("write_ack_and_directory_readback")),
                ("atomic".into(),json!(false)),
            ]);
            match mutation {
                Mutation::Create(r) => {
                    let id = ack.pid.ok_or_else(unconfirmed)?;
                    let created = after.into_iter().find(|p| p.id == id).ok_or_else(unconfirmed)?;
                    if created.name != r.name || created.track_count != Some(0)
                        || created.extensions.get("library_section") != Some(&json!("created"))
                        || created.extensions.get("is_public") != Some(&json!(true)) {
                        return Err(unconfirmed());
                    }
                    Ok(Outcome::Playlist(Box::new(PlaylistMutationResult {
                        playlist_ref: created.resource_ref.clone(), action: PlaylistMutationAction::Create,
                        playlist: Some(created), extensions,
                    })))
                }
                Mutation::Delete(r) => {
                    if r.playlist_refs.iter().any(|r| after_ids.contains(r.id())) {
                        return Err(unconfirmed());
                    }
                    Ok(Outcome::Deleted(PlaylistDeleteResult {
                        playlist_refs:r.playlist_refs.clone(),extensions,
                    }))
                }
                Mutation::Update(_, _) | Mutation::Visibility(_, _) | Mutation::Items(_) | Mutation::Sort(_, _) | Mutation::LibrarySort(_) | Mutation::CollectedSort(_) | Mutation::Collection(_, _, _) => unreachable!("update handled above"),
            }
        }).await.map_err(|_| TuneWeaveError::new(ErrorCode::UpstreamTimeout,
            "Kuwo native playlist write timed out").with_platform(Platform::Kuwo))?
    }

    async fn native_management_directory(
        &self,
        input: &KuwoNativeSessionInput,
    ) -> Result<(Vec<Playlist>, std::collections::BTreeSet<String>)> {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid())?
            .as_millis()
            .to_string();
        let query = library::query(input, Section::Owned, 0, &time)?;
        self.native_get_with_metadata(
            HOST,
            library::OWNED_PATH,
            "native_management_directory",
            format!("{}?{query}", self.native_target(HOST, library::OWNED_PATH)),
            Some(session_metadata(input)?),
            |bytes| {
                let items = library::dto::parse(bytes, input, Section::Owned)?;
                let ids = library::dto::all_owned_ids(bytes)?;
                Ok((items, ids))
            },
        )
        .await
    }

    async fn native_cloud_write(
        &self,
        input: &KuwoNativeSessionInput,
        operation: &'static str,
        body: serde_json::Value,
        dispatched: &mut bool,
    ) -> Result<Ack> {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid())?
            .as_millis()
            .to_string();
        let source = library::query(input, Section::Owned, 0, &time)?;
        let query = {
            let mut q = url::form_urlencoded::Serializer::new(String::new());
            for (key, value) in url::form_urlencoded::parse(source.as_bytes()) {
                // s2.r6 uses G2 directly; only the pl3_* ordinary helper adds recommend.
                if operation == "pl3_sortfavorlist" && key == "recommend" {
                    continue;
                }
                q.append_pair(&key, if key == "op" { operation } else { &value });
            }
            q.finish()
        };
        let request = self
            .http
            .post(format!(
                "{}?{query}",
                self.native_target(HOST, library::OWNED_PATH)
            ))
            .header(ACCEPT, "application/json")
            .header("Cookies", session_metadata(input)?)
            // The official byte-array POST uses this default MIME with raw JSON bytes.
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(serde_json::to_vec(&body).map_err(|_| invalid())?);
        let started = Instant::now();
        let mut status = None;
        *dispatched = true;
        let result = async {
            let response = request
                .send()
                .await
                .map_err(|e| kuwo_network_error(e).retryable(false))?;
            let response_status = response.status();
            status = Some(response_status);
            let response_mime = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next())
                .map(str::trim);
            let response_mime_class = match response_mime {
                Some(value) if value.eq_ignore_ascii_case("application/json") => "json",
                Some(value) if value.eq_ignore_ascii_case("text/plain") => "text_plain",
                Some(value) if value.eq_ignore_ascii_case("text/html") => "text_html",
                Some(_) => "other",
                None => "missing",
            };
            diagnostic_write_response(operation, response_status.as_u16(), response_mime_class);
            // The official byte-array callbacks parse response text as JSON even
            // when these write ACKs are labeled text/html. Accept that MIME only
            // for create/delete; JSON parsing and strict ACK checks remain required.
            let html_json_ack = matches!(operation, "pl3_addlist" | "pl3_deletelist");
            let bytes = read_response(response, html_json_ack, false, ACK_LIMIT).await?;
            diagnostic_write_ack(&bytes, input, operation == "pl3_addlist");
            parse_ack(&bytes, input, operation == "pl3_addlist")
        }
        .await;
        self.log_upstream_request(
            operation,
            HOST,
            library::OWNED_PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

#[derive(Deserialize)]
struct Ack {
    #[serde(default, deserialize_with = "deserialize_code")]
    errcode: Option<String>,
    result: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    uid: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    pid: Option<String>,
}
fn parse_ack(bytes: &[u8], input: &KuwoNativeSessionInput, create: bool) -> Result<Ack> {
    let ack: Ack = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if ack.errcode.as_deref() != Some("0")
        || ack.result.as_deref().is_some_and(|v| v != "ok")
        || ack.uid.as_deref().is_some_and(|v| v != input.user_id())
    {
        return Err(invalid());
    }
    if create {
        playlist::validate_id(ack.pid.as_deref().ok_or_else(invalid)?).map_err(|_| invalid())?;
    }
    Ok(ack)
}

#[cfg(debug_assertions)]
fn diagnostic_write_response(operation: &str, status: u16, mime_class: &str) {
    if std::env::var_os("TUNEWEAVE_KUWO_WRITE_DIAGNOSTICS").is_some() {
        eprintln!(
            "DIAGNOSTIC kuwo_write_response operation={operation} status={status} mime_class={mime_class}"
        );
    }
}

#[cfg(not(debug_assertions))]
fn diagnostic_write_response(_: &str, _: u16, _: &str) {}

#[cfg(debug_assertions)]
fn diagnostic_write_ack(bytes: &[u8], input: &KuwoNativeSessionInput, create: bool) {
    if std::env::var_os("TUNEWEAVE_KUWO_WRITE_DIAGNOSTICS").is_none() {
        return;
    }
    let value: serde_json::Value = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(_) => {
            eprintln!(
                "DIAGNOSTIC kuwo_write_ack create={create} body_bytes={} json_object=false",
                bytes.len()
            );
            return;
        }
    };
    let Some(object) = value.as_object() else {
        eprintln!(
            "DIAGNOSTIC kuwo_write_ack create={create} body_bytes={} json_object=false",
            bytes.len()
        );
        return;
    };
    let code = match object.get("errcode") {
        Some(serde_json::Value::String(value)) if value == "0" => "zero",
        Some(serde_json::Value::Number(value)) if value.as_i64() == Some(0) => "zero",
        Some(serde_json::Value::String(_) | serde_json::Value::Number(_)) => "other",
        Some(_) => "invalid",
        None => "missing",
    };
    let result = match object.get("result") {
        None => "absent",
        Some(serde_json::Value::String(value)) if value == "ok" => "ok",
        Some(serde_json::Value::String(_)) => "other",
        Some(_) => "invalid",
    };
    let uid = match object.get("uid") {
        None => "absent",
        Some(serde_json::Value::String(value)) if value == input.user_id() => "matches",
        Some(serde_json::Value::Number(value)) if value.to_string() == input.user_id() => "matches",
        Some(serde_json::Value::String(_) | serde_json::Value::Number(_)) => "mismatch",
        Some(_) => "invalid",
    };
    let pid = match object.get("pid") {
        None => "absent",
        Some(serde_json::Value::String(value)) if playlist::validate_id(value).is_ok() => "valid",
        Some(serde_json::Value::Number(value))
            if playlist::validate_id(&value.to_string()).is_ok() =>
        {
            "valid"
        }
        Some(serde_json::Value::String(_) | serde_json::Value::Number(_)) => "invalid",
        Some(_) => "invalid",
    };
    eprintln!(
        "DIAGNOSTIC kuwo_write_ack create={create} body_bytes={} json_object=true errcode={code} result={result} uid={uid} pid={pid}",
        bytes.len()
    );
}

#[cfg(not(debug_assertions))]
fn diagnostic_write_ack(_: &[u8], _: &KuwoNativeSessionInput, _: bool) {}

pub(crate) fn write_failure(mut error: TuneWeaveError, dispatched: bool) -> TuneWeaveError {
    if dispatched {
        let details = error.details.as_object_mut();
        let mut details = details.map(std::mem::take).unwrap_or_default();
        details.insert("write_outcome".into(), json!("unconfirmed"));
        details.insert("write_requests_dispatched".into(), json!(1));
        details.insert("automatic_retry".into(), json!(false));
        error = error
            .retryable(false)
            .with_details(serde_json::Value::Object(details));
    }
    error
}
fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo playlist write acknowledgement is invalid or rejected")
}
fn unconfirmed() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::Conflict,
        "Kuwo playlist write could not be confirmed by directory readback",
    )
    .with_platform(Platform::Kuwo)
}
