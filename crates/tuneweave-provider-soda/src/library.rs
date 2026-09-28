use std::{collections::BTreeSet, time::Instant};

use reqwest::{
    Method,
    header::{CONTENT_TYPE, COOKIE},
};
use serde::Deserialize;
use serde_json::json;
use tuneweave_core::{
    Album, ArtistSummary, ErrorCode, Extensions, Platform, Playlist, ResourceRef, Result,
    TuneWeaveError,
};
use url::Url;

use crate::{
    account::{add_luna_pc_headers, luna_pc_endpoint},
    client::{
        SodaClient, SodaImage, normalize_image, read_bounded_response, soda_http_error,
        soda_upstream_error,
    },
    login::SodaCredential,
};

pub(crate) const LIBRARY_PAGE_SIZE: usize = 500;
mod albums;
const MAX_LIBRARY_PAGES: usize = 64;
const MAX_LIBRARY_ITEMS: usize = 10_000;

#[derive(Clone, Copy)]
pub(crate) enum LibrarySection {
    Created,
    Saved,
}

impl LibrarySection {
    fn path(self) -> &'static str {
        match self {
            Self::Created => "/luna/pc/me/playlist",
            Self::Saved => "/luna/pc/me/collection/mixed",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Saved => "saved",
        }
    }
}

#[derive(Deserialize)]
struct LibraryEnvelope {
    playlists: Option<Vec<LibraryPlaylist>>,
    mixed_collections: Option<Vec<MixedCollection>>,
    total_num: Option<u64>,
    next_cursor: Option<String>,
    has_more: Option<bool>,
}

#[derive(Deserialize)]
struct StatusInfo {
    status_code: Option<i64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptySavedLibraryResponse {
    status_info: EmptySavedLibraryStatusInfo,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptySavedLibraryStatusInfo {
    log_id: String,
    now: u64,
    now_ts_ms: u64,
}

#[derive(Deserialize)]
struct CollectionWriteEnvelope {
    status_code: Option<i64>,
    status_info: Option<StatusInfo>,
}

#[derive(Deserialize)]
struct PlaylistCreateEnvelope {
    status_code: Option<i64>,
    status_info: Option<StatusInfo>,
}

#[derive(Deserialize)]
struct MixedCollection {
    item_type: String,
    playlist: Option<LibraryPlaylist>,
}

#[derive(Deserialize)]
struct LibraryPlaylist {
    id: String,
    title: String,
    public_title: Option<String>,
    desc: Option<String>,
    #[serde(default)]
    url_cover: SodaImage,
    owner: Option<Owner>,
    count_tracks: Option<u64>,
    #[serde(rename = "type")]
    playlist_type: Option<i64>,
}

#[derive(Deserialize)]
struct Owner {
    id: String,
    nickname: Option<String>,
    public_name: Option<String>,
}

pub(crate) struct LibraryPage<T = Playlist> {
    pub items: Vec<T>,
    pub credential: SodaCredential,
    raw_count: usize,
    total: Option<u64>,
    next_cursor: Option<String>,
    has_more: Option<bool>,
    unclassified_items: usize,
}

pub(crate) trait LibraryItem {
    fn library_id(&self) -> &str;
}
impl LibraryItem for Playlist {
    fn library_id(&self) -> &str {
        &self.id
    }
}
impl LibraryItem for Album {
    fn library_id(&self) -> &str {
        &self.id
    }
}
type LibraryPageParser<T> =
    fn(LibrarySection, &str, &SodaCredential, &[u8]) -> Result<LibraryPage<T>>;

impl SodaClient {
    fn library_url(&self, path: &str, cursor: Option<&str>) -> Result<Url> {
        let device = self.login_device()?;
        let mut url = luna_pc_endpoint(path, &device)?;
        if let Some(cursor) = cursor {
            let count = if path == "/luna/pc/me/playlist" {
                50
            } else {
                500
            };
            url.query_pairs_mut()
                .append_pair("cursor", if cursor == "0" { "" } else { cursor })
                .append_pair("count", &count.to_string());
        }
        Ok(url)
    }

    fn pc_playlist_write_url(&self, path: &str) -> Result<Url> {
        let device = self.login_device()?;
        luna_pc_endpoint(path, &device)
    }

    pub(crate) async fn create_owned_playlist(
        &self,
        name: &str,
        is_private: bool,
        credential: &SodaCredential,
    ) -> Result<(String, SodaCredential)> {
        let path = "/luna/pc/me/playlist";
        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            if credential.user_id().is_none() {
                return Err(authentication_required());
            }
            let response = self
                .send_login_request(
                    add_luna_pc_headers(
                        self.login_request(Method::POST, self.pc_playlist_write_url(path)?),
                    )
                    .header(COOKIE, credential.cookie_header()?)
                    .json(&json!({
                        "name": name,
                        "is_private": is_private,
                        "track_ids": [],
                    })),
                )
                .await?;
            http_status = Some(response.status());
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(authentication_required());
            }
            if !response.status().is_success() {
                return Err(soda_http_error(response.status()));
            }
            if response.headers().get(CONTENT_TYPE).is_some_and(|value| {
                value.to_str().map_or(true, |value| {
                    !value
                        .split(';')
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .eq_ignore_ascii_case("application/json")
                })
            }) {
                return Err(soda_upstream_error(
                    "Soda playlist creation returned an unexpected content type",
                ));
            }
            let headers = response.headers().clone();
            let body = read_bounded_response(response, "Soda playlist creation").await?;
            let id = match parse_playlist_create_ack(&body) {
                Ok(id) => id,
                Err(error) => {
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "DIAGNOSTIC soda_playlist_write_ack operation=playlist_create shape={}",
                        safe_library_shape(&body)
                    );
                    return Err(error);
                }
            };
            let refreshed = credential.with_response_cookies(&headers)?;
            Ok((id, refreshed))
        }
        .await;
        self.log_upstream_request(
            "playlist_create",
            "api.qishui.com",
            path,
            http_status,
            started,
            &result,
        );
        result
    }

    pub(crate) async fn update_owned_playlist_visibility(
        &self,
        id: &str,
        is_private: bool,
        credential: &SodaCredential,
    ) -> Result<SodaCredential> {
        self.update_owned_playlist_info(
            id,
            json!({"playlist_id": id, "is_private": is_private}),
            credential,
            "playlist_visibility_update",
            "Soda playlist visibility update",
        )
        .await
    }

    pub(crate) async fn update_owned_playlist_metadata(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<&str>,
        credential: &SodaCredential,
    ) -> Result<SodaCredential> {
        let mut body = json!({"playlist_id": id});
        if let Some(name) = name {
            body["name"] = json!(name);
        }
        if let Some(description) = description {
            body["description"] = json!(description);
        }
        self.update_owned_playlist_info(
            id,
            body,
            credential,
            "playlist_metadata_update",
            "Soda playlist metadata update",
        )
        .await
    }

    async fn update_owned_playlist_info(
        &self,
        id: &str,
        body: serde_json::Value,
        credential: &SodaCredential,
        operation: &'static str,
        description: &'static str,
    ) -> Result<SodaCredential> {
        let path = "/luna/pc/me/playlist/update";
        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            if !valid_id(id) {
                return Err(TuneWeaveError::invalid_request(
                    "Soda playlist ID must be a canonical positive integer",
                )
                .with_platform(Platform::Soda));
            }
            if credential.user_id().is_none() {
                return Err(authentication_required());
            }
            let response = self
                .send_login_request(
                    add_luna_pc_headers(
                        self.login_request(Method::POST, self.pc_playlist_write_url(path)?),
                    )
                    .header(COOKIE, credential.cookie_header()?)
                    .json(&body),
                )
                .await?;
            http_status = Some(response.status());
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(authentication_required());
            }
            if !response.status().is_success() {
                return Err(soda_http_error(response.status()));
            }
            if response.headers().get(CONTENT_TYPE).is_some_and(|value| {
                value.to_str().map_or(true, |value| {
                    !value
                        .split(';')
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .eq_ignore_ascii_case("application/json")
                })
            }) {
                return Err(soda_upstream_error(
                    "Soda playlist update returned an unexpected content type",
                ));
            }
            let headers = response.headers().clone();
            let response_body = read_bounded_response(response, description).await?;
            if let Err(error) = validate_playlist_update_ack(&response_body) {
                #[cfg(debug_assertions)]
                eprintln!(
                    "DIAGNOSTIC soda_playlist_write_ack operation={} shape={}",
                    operation,
                    safe_library_shape(&response_body)
                );
                return Err(error);
            }
            let refreshed = credential.with_response_cookies(&headers)?;
            Ok(refreshed)
        }
        .await;
        self.log_upstream_request(
            operation,
            "api.qishui.com",
            path,
            http_status,
            started,
            &result,
        );
        result
    }

    pub(crate) async fn delete_owned_playlist(
        &self,
        id: &str,
        credential: &SodaCredential,
    ) -> Result<SodaCredential> {
        let path = "/luna/pc/me/playlist/delete";
        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            if !valid_id(id) {
                return Err(TuneWeaveError::invalid_request(
                    "Soda playlist ID must be a canonical positive integer",
                )
                .with_platform(Platform::Soda));
            }
            if credential.user_id().is_none() {
                return Err(authentication_required());
            }
            let response = self
                .send_login_request(
                    add_luna_pc_headers(
                        self.login_request(Method::POST, self.pc_playlist_write_url(path)?),
                    )
                    .header(COOKIE, credential.cookie_header()?)
                    .json(&json!({"playlist_ids": [id]})),
                )
                .await?;
            http_status = Some(response.status());
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(authentication_required());
            }
            if !response.status().is_success() {
                return Err(soda_http_error(response.status()));
            }
            if response.headers().get(CONTENT_TYPE).is_some_and(|value| {
                value.to_str().map_or(true, |value| {
                    !value
                        .split(';')
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .eq_ignore_ascii_case("application/json")
                })
            }) {
                return Err(soda_upstream_error(
                    "Soda playlist deletion returned an unexpected content type",
                ));
            }
            let headers = response.headers().clone();
            let response_body = read_bounded_response(response, "Soda playlist deletion").await?;
            if let Err(error) = validate_playlist_delete_ack(&response_body, id) {
                #[cfg(debug_assertions)]
                eprintln!(
                    "DIAGNOSTIC soda_playlist_write_ack operation=playlist_delete shape={}",
                    safe_library_shape(&response_body)
                );
                return Err(error);
            }
            let refreshed = credential.with_response_cookies(&headers)?;
            Ok(refreshed)
        }
        .await;
        self.log_upstream_request(
            "playlist_delete",
            "api.qishui.com",
            path,
            http_status,
            started,
            &result,
        );
        result
    }

    pub(crate) async fn mutate_owned_playlist_media(
        &self,
        id: &str,
        media_ids: &[String],
        add: bool,
        credential: &SodaCredential,
    ) -> Result<SodaCredential> {
        let (path, operation, description) = if add {
            (
                "/luna/pc/me/playlist/media/append",
                "playlist_media_append",
                "Soda playlist track addition",
            )
        } else {
            (
                "/luna/pc/me/playlist/media/delete",
                "playlist_media_delete",
                "Soda playlist track removal",
            )
        };
        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            if !valid_id(id)
                || media_ids.is_empty()
                || media_ids.len() > 100
                || media_ids.iter().any(|media_id| !valid_id(media_id))
            {
                return Err(TuneWeaveError::invalid_request(
                    "Soda playlist media writes require a valid playlist and 1 to 100 tracks",
                )
                .with_platform(Platform::Soda));
            }
            let unique_ids = media_ids.iter().collect::<BTreeSet<_>>();
            if unique_ids.len() != media_ids.len() {
                return Err(TuneWeaveError::invalid_request(
                    "Soda playlist media writes require distinct track IDs",
                )
                .with_platform(Platform::Soda));
            }
            if credential.user_id().is_none() {
                return Err(authentication_required());
            }
            let media = media_ids
                .iter()
                .map(|id| json!({"id": id, "type": "track"}))
                .collect::<Vec<_>>();
            let response = self
                .send_login_request(
                    add_luna_pc_headers(
                        self.login_request(Method::POST, self.pc_playlist_write_url(path)?),
                    )
                    .header(COOKIE, credential.cookie_header()?)
                    .json(&json!({"playlist_id": id, "media": media})),
                )
                .await?;
            http_status = Some(response.status());
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(authentication_required());
            }
            if !response.status().is_success() {
                return Err(soda_http_error(response.status()));
            }
            if response.headers().get(CONTENT_TYPE).is_some_and(|value| {
                value.to_str().map_or(true, |value| {
                    !value
                        .split(';')
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .eq_ignore_ascii_case("application/json")
                })
            }) {
                return Err(soda_upstream_error(
                    "Soda playlist media write returned an unexpected content type",
                ));
            }
            let headers = response.headers().clone();
            let response_body = read_bounded_response(response, description).await?;
            if let Err(error) = validate_playlist_media_write_ack(&response_body, id) {
                #[cfg(debug_assertions)]
                eprintln!(
                    "DIAGNOSTIC soda_playlist_write_ack operation={operation} shape={}",
                    safe_library_shape(&response_body)
                );
                return Err(error);
            }
            credential.with_response_cookies(&headers)
        }
        .await;
        self.log_upstream_request(
            operation,
            "api.qishui.com",
            path,
            http_status,
            started,
            &result,
        );
        result
    }

    pub(crate) async fn write_playlist_collection(
        &self,
        id: &str,
        subscribed: bool,
        credential: &SodaCredential,
    ) -> Result<SodaCredential> {
        let path = if subscribed {
            "/luna/pc/me/collection/playlist"
        } else {
            "/luna/pc/me/collection/playlist/delete"
        };
        self.write_collection(
            path,
            "playlist_collection_write",
            json!({"playlist_ids": [id]}),
            credential,
        )
        .await
    }

    pub(crate) async fn write_track_collection(
        &self,
        id: &str,
        subscribed: bool,
        credential: &SodaCredential,
    ) -> Result<SodaCredential> {
        let path = if subscribed {
            "/luna/pc/me/collection/media"
        } else {
            "/luna/pc/me/collection/media/delete"
        };
        self.write_collection(
            path,
            "track_collection_write",
            json!({"media": [{"type":"track", "id":id}], "scene":""}),
            credential,
        )
        .await
    }

    async fn write_collection(
        &self,
        path: &'static str,
        operation: &'static str,
        body: serde_json::Value,
        credential: &SodaCredential,
    ) -> Result<SodaCredential> {
        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            if credential.user_id().is_none() {
                return Err(authentication_required());
            }
            let response = self
                .send_login_request(
                    add_luna_pc_headers(
                        self.login_request(Method::POST, self.library_url(path, None)?),
                    )
                    .header(COOKIE, credential.cookie_header()?)
                    .json(&body),
                )
                .await?;
            http_status = Some(response.status());
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(authentication_required());
            }
            if !response.status().is_success() {
                return Err(soda_http_error(response.status()));
            }
            if response.headers().get(CONTENT_TYPE).is_some_and(|value| {
                value.to_str().map_or(true, |value| {
                    !value
                        .split(';')
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .eq_ignore_ascii_case("application/json")
                })
            }) {
                return Err(soda_upstream_error(
                    "Soda collection write returned an unexpected content type",
                ));
            }
            let headers = response.headers().clone();
            let body = read_bounded_response(response, "Soda collection write").await?;
            validate_collection_write(&body)?;
            credential.with_response_cookies(&headers)
        }
        .await;
        self.log_upstream_request(
            operation,
            "api.qishui.com",
            path,
            http_status,
            started,
            &result,
        );
        result
    }

    pub(crate) async fn library_page(
        &self,
        section: LibrarySection,
        cursor: &str,
        credential: &SodaCredential,
    ) -> Result<LibraryPage> {
        self.read_library_page(section, cursor, credential, "account_playlists", parse_page)
            .await
    }

    async fn read_library_page<T>(
        &self,
        section: LibrarySection,
        cursor: &str,
        credential: &SodaCredential,
        operation: &'static str,
        parse: LibraryPageParser<T>,
    ) -> Result<LibraryPage<T>> {
        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            let owner = credential.user_id().ok_or_else(|| {
                soda_upstream_error("Soda library requires a verified account identity")
            })?;
            let url = self.library_url(section.path(), Some(cursor))?;
            let response = self
                .send_login_request(
                    add_luna_pc_headers(self.login_request(Method::GET, url))
                        .header(COOKIE, credential.cookie_header()?),
                )
                .await?;
            http_status = Some(response.status());
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(authentication_required());
            }
            if !response.status().is_success() {
                return Err(soda_http_error(response.status()));
            }
            if response.headers().get(CONTENT_TYPE).is_some_and(|value| {
                value.to_str().map_or(true, |value| {
                    !value
                        .split(';')
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .eq_ignore_ascii_case("application/json")
                })
            }) {
                return Err(soda_upstream_error(
                    "Soda account library returned an unexpected content type",
                ));
            }
            let headers = response.headers().clone();
            let body = read_bounded_response(response, "Soda account library").await?;
            let mut page = match parse(section, owner, credential, &body) {
                Ok(page) => page,
                Err(error) => {
                    #[cfg(debug_assertions)]
                    eprintln!(
                        "DIAGNOSTIC soda_library_shape section={} bytes={} shape={}",
                        section.name(),
                        body.len(),
                        safe_library_shape(&body)
                    );
                    return Err(error);
                }
            };
            page.credential = credential.with_response_cookies(&headers)?;
            Ok(page)
        }
        .await;
        self.log_upstream_request(
            operation,
            "api.qishui.com",
            section.path(),
            http_status,
            started,
            &result,
        );
        result
    }
}

#[cfg(debug_assertions)]
fn safe_library_shape(body: &[u8]) -> serde_json::Value {
    use serde_json::{Map, Value, json};

    fn kind(value: &Value) -> &'static str {
        match value {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        }
    }

    fn fields(value: &Value) -> Map<String, Value> {
        value.as_object().map_or_else(Map::new, |object| {
            object
                .iter()
                .map(|(key, value)| (key.clone(), json!(kind(value))))
                .collect()
        })
    }

    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return json!({"kind":"non_json","bytes":body.len()});
    };
    let mut summary = Map::new();
    summary.insert("top_level".into(), Value::Object(fields(&value)));
    if let Some(info) = value.get("status_info") {
        summary.insert("status_info_fields".into(), Value::Object(fields(info)));
    }
    if let Some(playlist) = value.get("playlist") {
        summary.insert("playlist_fields".into(), Value::Object(fields(playlist)));
    }
    if let Some(deleted) = value.get("deleted_playlists").and_then(Value::as_array) {
        summary.insert("deleted_playlists_count".into(), json!(deleted.len()));
        if let Some(first) = deleted.first() {
            summary.insert("deleted_playlists_first_kind".into(), json!(kind(first)));
            if first.is_object() {
                summary.insert(
                    "deleted_playlists_first_fields".into(),
                    Value::Object(fields(first)),
                );
            }
        }
    }
    for name in ["status_code", "total_num"] {
        if let Some(field) = value.get(name).filter(|field| field.is_number()) {
            summary.insert(name.into(), field.clone());
        }
    }
    if let Some(code) = value
        .get("status_info")
        .and_then(|info| info.get("status_code"))
        .filter(|field| field.is_number())
    {
        summary.insert("status_info_code".into(), code.clone());
    }
    for name in ["playlists", "mixed_collections", "collections", "data"] {
        if let Some(items) = value.get(name).and_then(Value::as_array) {
            summary.insert(format!("{name}_count"), json!(items.len()));
            if let Some(first) = items.first() {
                summary.insert(format!("{name}_first_fields"), Value::Object(fields(first)));
            }
        }
    }
    Value::Object(summary)
}

#[cfg(debug_assertions)]
fn safe_created_owner_counts(items: &[Playlist], source_user_id: &str) -> serde_json::Value {
    use serde_json::json;

    let mut owner_present_count = 0;
    let mut owner_match_count = 0;
    let mut owner_mismatch_count = 0;
    for item in items {
        if let Some(owner_id) = item
            .extensions
            .get("owner_id")
            .and_then(serde_json::Value::as_str)
        {
            owner_present_count += 1;
            if owner_id == source_user_id {
                owner_match_count += 1;
            } else {
                owner_mismatch_count += 1;
            }
        }
    }
    json!({
        "item_count": items.len(),
        "owner_present_count": owner_present_count,
        "owner_match_count": owner_match_count,
        "owner_mismatch_count": owner_mismatch_count,
    })
}

fn validate_collection_write(body: &[u8]) -> Result<()> {
    let envelope: CollectionWriteEnvelope = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda collection write returned invalid data"))?;
    let codes = [
        envelope.status_code,
        envelope.status_info.and_then(|info| info.status_code),
    ];
    if codes.contains(&Some(1_000_016)) {
        return Err(authentication_required());
    }
    if !codes.contains(&Some(0)) || codes.into_iter().flatten().any(|code| code != 0) {
        return Err(soda_upstream_error(
            "Soda collection write did not report success",
        ));
    }
    Ok(())
}

fn parse_playlist_create_ack(body: &[u8]) -> Result<String> {
    let response: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda playlist creation returned invalid data"))?;
    let envelope: PlaylistCreateEnvelope = serde_json::from_value(response.clone())
        .map_err(|_| soda_upstream_error("Soda playlist creation returned invalid data"))?;
    if let Some(status_code) = envelope.status_code {
        if status_code == 1_000_016 {
            return Err(authentication_required());
        }
        if status_code != 0 {
            return Err(soda_upstream_error("Soda playlist creation was rejected"));
        }
        if let Some(status_code) = envelope.status_info.and_then(|info| info.status_code) {
            if status_code == 1_000_016 {
                return Err(authentication_required());
            }
            if status_code != 0 {
                return Err(soda_upstream_error(
                    "Soda playlist creation returned conflicting statuses",
                ));
            }
        }
        return extract_created_playlist_id(&response);
    }

    // The account API also returns a statusless create response with exactly
    // `playlist` and the standard request `status_info`. Keep this separate
    // from the documented status-code variants and accept only the observed
    // direct playlist identity shape.
    if !has_exact_fields(&response, &["playlist", "status_info"]) {
        return Err(soda_upstream_error(
            "Soda playlist creation omitted its result status",
        ));
    }
    validate_statusless_write_info(&response, "Soda playlist creation")?;
    let id = response
        .get("playlist")
        .and_then(|playlist| playlist.get("id"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            soda_upstream_error("Soda playlist creation omitted a valid playlist identity")
        })?;
    if !valid_id(id) {
        return Err(soda_upstream_error(
            "Soda playlist creation returned an invalid playlist identity",
        ));
    }
    Ok(id.to_owned())
}

fn has_exact_fields(value: &serde_json::Value, expected: &[&str]) -> bool {
    value.as_object().is_some_and(|object| {
        object.len() == expected.len() && expected.iter().all(|field| object.contains_key(*field))
    })
}

fn validate_statusless_write_info(value: &serde_json::Value, operation: &str) -> Result<()> {
    let info: EmptySavedLibraryStatusInfo = value
        .get("status_info")
        .cloned()
        .ok_or_else(|| soda_upstream_error(format!("{operation} returned invalid status info")))
        .and_then(|info| {
            serde_json::from_value(info).map_err(|_| {
                soda_upstream_error(format!("{operation} returned invalid status info"))
            })
        })?;
    if info.log_id.trim().is_empty()
        || info.log_id.len() > 256
        || info.log_id.chars().any(char::is_control)
        || info.now == 0
        || info.now_ts_ms == 0
    {
        return Err(soda_upstream_error(format!(
            "{operation} returned invalid status info"
        )));
    }
    Ok(())
}

fn extract_created_playlist_id(response: &serde_json::Value) -> Result<String> {
    // These are the finite response variants supported by libresoda at the
    // pinned upstream revision. Do not recursively search arbitrary response
    // fields: an unrelated nested ID must never become a playlist target.
    const PATHS: &[&[&str]] = &[
        &["data", "playlist_id"],
        &["data", "playlist", "id"],
        &["data", "id"],
        &["playlist", "id"],
        &["playlist_id"],
    ];

    let mut id = None;
    for path in PATHS {
        let mut candidate = Some(response);
        for key in *path {
            candidate = candidate.and_then(|value| value.get(*key));
        }
        let Some(candidate) = candidate else {
            continue;
        };
        let value = match candidate {
            serde_json::Value::Null => continue,
            serde_json::Value::String(value) if value.trim().is_empty() => continue,
            serde_json::Value::String(value) => value.trim().to_owned(),
            serde_json::Value::Number(value) => value
                .as_i64()
                .map(|value| value.to_string())
                .ok_or_else(|| {
                    soda_upstream_error(
                        "Soda playlist creation returned an invalid playlist identity",
                    )
                })?,
            _ => {
                return Err(soda_upstream_error(
                    "Soda playlist creation returned an invalid playlist identity",
                ));
            }
        };
        if !valid_id(&value) {
            return Err(soda_upstream_error(
                "Soda playlist creation returned an invalid playlist identity",
            ));
        }
        if id.as_ref().is_some_and(|existing| existing != &value) {
            return Err(soda_upstream_error(
                "Soda playlist creation returned conflicting playlist identities",
            ));
        }
        id = Some(value);
    }

    id.ok_or_else(|| {
        soda_upstream_error("Soda playlist creation omitted a valid playlist identity")
    })
}

fn validate_playlist_update_ack(body: &[u8]) -> Result<()> {
    let envelope: CollectionWriteEnvelope = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda playlist update returned invalid data"))?;
    let status_code = envelope
        .status_code
        .ok_or_else(|| soda_upstream_error("Soda playlist update omitted its result status"))?;
    for code in [
        Some(status_code),
        envelope.status_info.and_then(|info| info.status_code),
    ]
    .into_iter()
    .flatten()
    {
        if code == 1_000_016 {
            return Err(authentication_required());
        }
        if code != 0 {
            return Err(soda_upstream_error("Soda playlist update was rejected"));
        }
    }
    Ok(())
}

fn validate_playlist_delete_ack(body: &[u8], requested_id: &str) -> Result<()> {
    let envelope: CollectionWriteEnvelope = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda playlist deletion returned invalid data"))?;
    if let Some(status_code) = envelope.status_code {
        for code in [
            Some(status_code),
            envelope.status_info.and_then(|info| info.status_code),
        ]
        .into_iter()
        .flatten()
        {
            if code == 1_000_016 {
                return Err(authentication_required());
            }
            if code != 0 {
                return Err(soda_upstream_error("Soda playlist deletion was rejected"));
            }
        }
        return Ok(());
    }

    let response: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda playlist deletion returned invalid data"))?;
    if !has_exact_fields(&response, &["deleted_playlists", "status_info"]) {
        return Err(soda_upstream_error(
            "Soda playlist deletion omitted its result status",
        ));
    }
    validate_statusless_write_info(&response, "Soda playlist deletion")?;
    let deleted = response
        .get("deleted_playlists")
        .and_then(serde_json::Value::as_array)
        .filter(|items| items.len() == 1)
        .and_then(|items| items.first())
        .and_then(serde_json::Value::as_str)
        .filter(|id| valid_id(id) && *id == requested_id)
        .ok_or_else(|| {
            soda_upstream_error("Soda playlist deletion did not confirm the requested identity")
        })?;
    let _ = deleted;
    Ok(())
}

fn validate_playlist_media_write_ack(body: &[u8], requested_id: &str) -> Result<()> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda playlist media write returned invalid data"))?;
    let envelope: CollectionWriteEnvelope = serde_json::from_value(value.clone())
        .map_err(|_| soda_upstream_error("Soda playlist media write returned invalid data"))?;
    if let Some(status_code) = envelope.status_code {
        for code in [
            Some(status_code),
            envelope.status_info.and_then(|info| info.status_code),
        ]
        .into_iter()
        .flatten()
        {
            if code == 1_000_016 {
                return Err(authentication_required());
            }
            if code != 0 {
                return Err(soda_upstream_error(
                    "Soda playlist media write was rejected",
                ));
            }
        }
        return Ok(());
    }

    // The PC delete endpoint returns an identity-bound playlist snapshot with
    // status_info but no status_code. This is only an ACK candidate: the provider
    // still requires a separate complete before/after readback before reporting success.
    validate_statusless_write_info(&value, "Soda playlist media write")?;
    let response_playlist_id = value
        .pointer("/playlist/id")
        .and_then(serde_json::Value::as_str);
    let media = value.get("media").and_then(serde_json::Value::as_array);
    if response_playlist_id != Some(requested_id) || media.is_none() {
        return Err(soda_upstream_error(
            "Soda playlist media write omitted its result status",
        ));
    }
    Ok(())
}

fn authentication_required() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "Soda account library session is not authenticated",
    )
    .with_platform(Platform::Soda)
}

fn parse_page(
    section: LibrarySection,
    owner: &str,
    credential: &SodaCredential,
    bytes: &[u8],
) -> Result<LibraryPage> {
    validate_library_status(bytes)?;
    if matches!(section, LibrarySection::Saved)
        && let Some(page) = empty_saved_library_page(credential, bytes)
    {
        return Ok(page);
    }
    let envelope: LibraryEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| soda_upstream_error("Soda account playlists returned invalid data"))?;
    let mut unclassified_items = 0;
    let (raw_count, sources) = match section {
        LibrarySection::Created => {
            let items = envelope
                .playlists
                .ok_or_else(|| soda_upstream_error("Soda created playlists are missing"))?;
            (items.len(), items)
        }
        LibrarySection::Saved => {
            let items = envelope
                .mixed_collections
                .ok_or_else(|| soda_upstream_error("Soda saved collections are missing"))?;
            let count = items.len();
            let mut playlists = Vec::new();
            for item in items {
                if item.item_type.is_empty()
                    || item.item_type.len() > 64
                    || item.item_type.chars().any(char::is_control)
                {
                    return Err(soda_upstream_error("Soda saved collection type is invalid"));
                }
                if item.item_type == "playlist" {
                    playlists
                        .push(item.playlist.ok_or_else(|| {
                            soda_upstream_error("Soda saved playlist is missing")
                        })?);
                } else if item.item_type != "album" {
                    unclassified_items += 1;
                }
            }
            (count, playlists)
        }
    };
    if raw_count > LIBRARY_PAGE_SIZE
        || envelope
            .total_num
            .is_some_and(|total| total > MAX_LIBRARY_ITEMS as u64)
    {
        return Err(soda_upstream_error(
            "Soda account library exceeds the supported size",
        ));
    }
    let items = sources
        .into_iter()
        .map(|item| map_playlist(item, section, owner))
        .collect::<Result<Vec<_>>>()?;
    #[cfg(debug_assertions)]
    if matches!(section, LibrarySection::Created) {
        eprintln!(
            "DIAGNOSTIC soda_created_library_owner_counts {}",
            safe_created_owner_counts(&items, owner)
        );
    }
    Ok(LibraryPage {
        items,
        credential: credential.clone(),
        raw_count,
        total: envelope.total_num,
        next_cursor: envelope.next_cursor,
        has_more: envelope.has_more,
        unclassified_items,
    })
}

fn empty_saved_library_page(credential: &SodaCredential, bytes: &[u8]) -> Option<LibraryPage> {
    let response = serde_json::from_slice::<EmptySavedLibraryResponse>(bytes).ok()?;
    if response.status_info.log_id.trim().is_empty()
        || response.status_info.log_id.len() > 256
        || response.status_info.log_id.chars().any(char::is_control)
        || response.status_info.now == 0
        || response.status_info.now_ts_ms == 0
    {
        return None;
    }
    Some(LibraryPage {
        items: Vec::new(),
        credential: credential.clone(),
        raw_count: 0,
        total: Some(0),
        next_cursor: None,
        has_more: Some(false),
        unclassified_items: 0,
    })
}

fn validate_library_status(body: &[u8]) -> Result<()> {
    let envelope: CollectionWriteEnvelope = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda account library returned malformed data"))?;
    let codes = [
        envelope.status_code,
        envelope.status_info.and_then(|info| info.status_code),
    ];
    if codes.contains(&Some(1_000_016)) {
        return Err(authentication_required());
    }
    if codes.into_iter().flatten().any(|code| code != 0) {
        return Err(soda_upstream_error("Soda account library was rejected"));
    }
    Ok(())
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn valid_text(value: &str, limit: usize) -> bool {
    value.len() <= limit
        && !value
            .chars()
            .any(|ch| ch.is_control() && ch != '\n' && ch != '\t')
}

fn map_playlist(
    item: LibraryPlaylist,
    section: LibrarySection,
    source_user_id: &str,
) -> Result<Playlist> {
    if !valid_id(&item.id)
        || item.title.trim().is_empty()
        || !valid_text(&item.title, 2_000)
        || item
            .public_title
            .as_deref()
            .is_some_and(|text| !valid_text(text, 2_000))
        || item
            .desc
            .as_deref()
            .is_some_and(|text| !valid_text(text, 64 * 1024))
        || item.count_tracks.is_some_and(|count| count > 1_000_000)
    {
        return Err(soda_upstream_error(
            "Soda account playlist metadata is invalid",
        ));
    }
    let mut extensions = Extensions::from([
        ("library_section".to_owned(), json!(section.name())),
        ("source_user_id".to_owned(), json!(source_user_id)),
    ]);
    let mut creator = None;
    if let Some(owner) = item.owner {
        if !valid_id(&owner.id)
            || owner
                .nickname
                .as_deref()
                .is_some_and(|text| !valid_text(text, 1_000))
            || owner
                .public_name
                .as_deref()
                .is_some_and(|text| !valid_text(text, 1_000))
        {
            return Err(soda_upstream_error(
                "Soda account playlist owner is invalid",
            ));
        }
        if matches!(section, LibrarySection::Created) && owner.id != source_user_id {
            return Err(soda_upstream_error(
                "Soda created playlist belongs to a different account",
            ));
        }
        extensions.insert("owner_id".to_owned(), json!(owner.id));
        creator = owner
            .public_name
            .filter(|name| !name.trim().is_empty())
            .or(owner.nickname.filter(|name| !name.trim().is_empty()))
            .map(|name| ArtistSummary {
                resource_ref: None,
                name,
            });
    }
    if let Some(kind) = item.playlist_type {
        extensions.insert("playlist_type".to_owned(), json!(kind));
    }
    if let Some(title) = item.public_title.filter(|title| !title.trim().is_empty()) {
        extensions.insert("public_title".to_owned(), json!(title));
    }
    Ok(Playlist {
        resource_ref: ResourceRef::new(Platform::Soda, &item.id)
            .map_err(|_| soda_upstream_error("Soda playlist identity is invalid"))?,
        platform: Platform::Soda,
        id: item.id,
        name: item.title,
        description: item.desc.unwrap_or_default(),
        cover_url: normalize_image(&item.url_cover),
        creator,
        track_count: item.count_tracks,
        tags: Vec::new(),
        subscribed: Some(matches!(section, LibrarySection::Saved)),
        created_at: None,
        updated_at: None,
        extensions,
    })
}

/// A library page is accepted only when continuation or completion is explicit.
/// `total_num` on mixed collections counts all kinds, before playlist filtering.
#[derive(Default)]
pub(crate) struct LibraryPagination {
    pages: usize,
    raw_count: usize,
    total: Option<u64>,
    cursors: BTreeSet<String>,
    item_ids: BTreeSet<String>,
    unclassified_items: usize,
}

impl LibraryPagination {
    pub(crate) fn accept<T: LibraryItem>(
        &mut self,
        cursor: &str,
        page: &LibraryPage<T>,
    ) -> Result<Option<String>> {
        self.pages += 1;
        self.raw_count += page.raw_count;
        self.unclassified_items += page.unclassified_items;
        if self.pages > MAX_LIBRARY_PAGES || self.raw_count > MAX_LIBRARY_ITEMS {
            return Err(soda_upstream_error(
                "Soda account library request budget was exhausted",
            ));
        }
        if !self.cursors.insert(cursor.to_owned())
            || page
                .items
                .iter()
                .any(|item| !self.item_ids.insert(item.library_id().to_owned()))
        {
            return Err(soda_upstream_error(
                "Soda account library repeated a page or collection item",
            ));
        }
        if let Some(total) = page.total {
            if self.total.is_some_and(|previous| previous != total) || total < self.raw_count as u64
            {
                return Err(soda_upstream_error(
                    "Soda account library totals changed during pagination",
                ));
            }
            self.total = Some(total);
        }
        let more = match page.has_more {
            Some(more) => more,
            None => self
                .total
                .map(|total| total > self.raw_count as u64)
                .ok_or_else(|| {
                    soda_upstream_error(
                        "Soda account library omitted pagination completeness information",
                    )
                })?,
        };
        if !more {
            if self
                .total
                .is_some_and(|total| total != self.raw_count as u64)
            {
                return Err(soda_upstream_error(
                    "Soda account library ended before its reported total",
                ));
            }
            return Ok(None);
        }
        if page.raw_count == 0
            || self
                .total
                .is_some_and(|total| total <= self.raw_count as u64)
            || self.pages >= MAX_LIBRARY_PAGES
        {
            return Err(soda_upstream_error(
                "Soda account library cannot continue within its pagination bounds",
            ));
        }
        let next = page
            .next_cursor
            .as_deref()
            .filter(|cursor| {
                !cursor.is_empty()
                    && cursor.len() <= 256
                    && cursor.bytes().all(|byte| byte.is_ascii_graphic())
            })
            .ok_or_else(|| {
                soda_upstream_error("Soda account library omitted a valid continuation cursor")
            })?;
        if self.cursors.contains(next) {
            return Err(soda_upstream_error(
                "Soda account library repeated a continuation cursor",
            ));
        }
        Ok(Some(next.to_owned()))
    }

    // Call only after explicit completion. Unlike track playlists, this count covers
    // every mixed collection entry. Unknown item kinds cannot prove a collection is absent.
    pub(crate) fn absence_is_proven(&self) -> bool {
        self.total == Some(self.raw_count as u64) && self.unclassified_items == 0
    }

    pub(crate) fn raw_total(&self) -> Option<u64> {
        self.total
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[cfg(debug_assertions)]
    #[test]
    fn account_library_diagnostics_report_shape_and_counts_without_values() {
        let body = br#"{"status_info":{"now":123,"cookie":"private"},"playlist":{"id":"private-id","title":"private title"},"deleted_playlists":[{"id":"private-id"}],"mixed_collections":[{"item_type":"playlist","id":"private-id"}],"total_num":1,"token":"private-token"}"#;
        let summary = safe_library_shape(body).to_string();
        assert!(summary.contains("\"mixed_collections_count\":1"));
        assert!(summary.contains("\"status_info\":\"object\""));
        assert!(summary.contains("\"playlist_fields\""));
        assert!(summary.contains("\"deleted_playlists_first_fields\""));
        assert!(!summary.contains("private"));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn created_library_owner_diagnostics_report_only_counts() {
        let created = page(
            LibrarySection::Created,
            json!({
                "playlists": [
                    {"id":"11","title":"private title","owner":{"id":"123456"}},
                    {"id":"12","title":"another private title"}
                ],
                "total_num":2
            }),
        )
        .unwrap();
        let summary = safe_created_owner_counts(&created.items, "123456");
        assert_eq!(summary["item_count"], 2);
        assert_eq!(summary["owner_present_count"], 1);
        assert_eq!(summary["owner_match_count"], 1);
        assert_eq!(summary["owner_mismatch_count"], 0);
        let serialized = summary.to_string();
        assert!(!serialized.contains("123456"));
        assert!(!serialized.contains("private title"));
    }

    #[test]
    fn collection_writes_require_explicit_success_without_conflicting_statuses() {
        for body in [
            r#"{"status_code":0}"#,
            r#"{"status_info":{"status_code":0}}"#,
            r#"{"status_code":0,"status_info":{"status_code":0}}"#,
        ] {
            validate_collection_write(body.as_bytes()).unwrap();
        }
        for body in [
            r#"{}"#,
            r#"{"status_info":{}}"#,
            r#"{"ok":true}"#,
            r#"{"status_code":7}"#,
            r#"{"status_code":0,"status_info":{"status_code":99}}"#,
            r#"{"status_code":99,"status_info":{"status_code":0}}"#,
            r#"{"status_code":"0"}"#,
        ] {
            assert_eq!(
                validate_collection_write(body.as_bytes()).unwrap_err().code,
                ErrorCode::UpstreamError,
                "{body}"
            );
        }
        for body in [
            r#"{"status_code":1000016}"#,
            r#"{"status_code":0,"status_info":{"status_code":1000016}}"#,
        ] {
            assert_eq!(
                validate_collection_write(body.as_bytes()).unwrap_err().code,
                ErrorCode::AuthenticationRequired
            );
        }
    }

    #[test]
    fn playlist_create_ack_requires_success_and_a_canonical_returned_identity() {
        assert_eq!(
            parse_playlist_create_ack(br#"{"status_code":0,"playlist":{"id":"42"}}"#).unwrap(),
            "42"
        );
        for body in [
            br#"{}"#.as_slice(),
            br#"{"status_info":{"status_code":0},"playlist":{"id":"42"}}"#,
            br#"{"status_code":0}"#,
            br#"{"status_code":0,"playlist":{"id":"042"}}"#,
            br#"{"status_code":0,"playlist":{"id":"not-an-id"}}"#,
            br#"{"status_code":0,"status_info":{"status_code":7},"playlist":{"id":"42"}}"#,
            br#"{"status_code":9,"playlist":{"id":"42"}}"#,
        ] {
            assert_eq!(
                parse_playlist_create_ack(body).unwrap_err().code,
                ErrorCode::UpstreamError
            );
        }
        assert_eq!(
            parse_playlist_create_ack(br#"{"status_code":1000016,"playlist":{"id":"42"}}"#)
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
    }

    #[test]
    fn playlist_create_ack_accepts_only_the_observed_statusless_response_shape() {
        let valid = br#"{"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456},"playlist":{"id":"42","title":"test","type":0}}"#;
        assert_eq!(parse_playlist_create_ack(valid).unwrap(), "42");

        for body in [
            br#"{"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456},"playlist":{"id":"042"}}"#.as_slice(),
            br#"{"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456},"playlist":{"id":42}}"#,
            br#"{"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456},"playlist":{"id":"42"},"unexpected":true}"#,
            br#"{"status_info":{"log_id":"request-log","now":0,"now_ts_ms":123456},"playlist":{"id":"42"}}"#,
            br#"{"status_info":{"log_id":"request-log","now":123,"now_ts_ms":0},"playlist":{"id":"42"}}"#,
            br#"{"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456,"status_code":0},"playlist":{"id":"42"}}"#,
            br#"{"status_info":{"log_id":"","now":123,"now_ts_ms":123456},"playlist":{"id":"42"}}"#,
        ] {
            assert_eq!(
                parse_playlist_create_ack(body).unwrap_err().code,
                ErrorCode::UpstreamError,
                "{}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn playlist_create_ack_supports_only_the_pinned_upstream_id_paths() {
        for body in [
            br#"{"status_code":0,"data":{"playlist_id":"42"}}"#.as_slice(),
            br#"{"status_code":0,"data":{"playlist":{"id":42}}}"#,
            br#"{"status_code":0,"data":{"id":"42"}}"#,
            br#"{"status_code":0,"playlist":{"id":"42"}}"#,
            br#"{"status_code":0,"playlist_id":42}"#,
        ] {
            assert_eq!(parse_playlist_create_ack(body).unwrap(), "42");
        }

        for body in [
            br#"{"status_code":0,"result":{"playlist_id":"42"}}"#.as_slice(),
            br#"{"status_code":0,"data":{"playlist_id":"042"}}"#,
            br#"{"status_code":0,"data":{"playlist_id":0}}"#,
            br#"{"status_code":0,"data":{"playlist_id":"42"},"playlist_id":"43"}"#,
        ] {
            assert_eq!(
                parse_playlist_create_ack(body).unwrap_err().code,
                ErrorCode::UpstreamError
            );
        }

        let bounded_id = "1".repeat(64);
        let bounded_body = format!(r#"{{"status_code":0,"playlist_id":"{bounded_id}"}}"#);
        assert_eq!(
            parse_playlist_create_ack(bounded_body.as_bytes()).unwrap(),
            bounded_id
        );
        let oversized_id = "1".repeat(65);
        let oversized_body = format!(r#"{{"status_code":0,"playlist_id":"{oversized_id}"}}"#);
        assert_eq!(
            parse_playlist_create_ack(oversized_body.as_bytes())
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
    }

    #[test]
    fn playlist_update_ack_requires_the_status_consumed_by_the_pc_renderer() {
        for body in [
            br#"{"status_code":0}"#.as_slice(),
            br#"{"status_code":0,"status_info":{"status_code":0}}"#,
        ] {
            validate_playlist_update_ack(body).unwrap();
        }
        for body in [
            br#"{}"#.as_slice(),
            br#"{"status_info":{"status_code":0}}"#,
            br#"{"status_code":7}"#,
            br#"{"status_code":0,"status_info":{"status_code":7}}"#,
            br#"{"status_code":"0"}"#,
        ] {
            assert_eq!(
                validate_playlist_update_ack(body).unwrap_err().code,
                ErrorCode::UpstreamError,
                "{}",
                String::from_utf8_lossy(body)
            );
        }
        assert_eq!(
            validate_playlist_update_ack(br#"{"status_code":1000016}"#)
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
        assert_eq!(
            validate_playlist_update_ack(
                br#"{"status_code":0,"status_info":{"status_code":1000016}}"#
            )
            .unwrap_err()
            .code,
            ErrorCode::AuthenticationRequired
        );
    }

    #[test]
    fn playlist_delete_ack_requires_the_status_consumed_by_the_pc_renderer() {
        for body in [
            br#"{"status_code":0}"#.as_slice(),
            br#"{"status_code":0,"status_info":{"status_code":0}}"#,
        ] {
            validate_playlist_delete_ack(body, "42").unwrap();
        }
        for body in [
            br#"{}"#.as_slice(),
            br#"{"status_info":{"status_code":0}}"#,
            br#"{"status_code":7}"#,
            br#"{"status_code":0,"status_info":{"status_code":7}}"#,
            br#"{"status_code":"0"}"#,
        ] {
            assert_eq!(
                validate_playlist_delete_ack(body, "42").unwrap_err().code,
                ErrorCode::UpstreamError,
                "{}",
                String::from_utf8_lossy(body)
            );
        }
        assert_eq!(
            validate_playlist_delete_ack(br#"{"status_code":1000016}"#, "42")
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
    }

    #[test]
    fn playlist_delete_ack_accepts_only_the_requested_id_in_the_statusless_variant() {
        let valid = br#"{"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456},"deleted_playlists":["42"]}"#;
        validate_playlist_delete_ack(valid, "42").unwrap();

        for body in [
            br#"{"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456},"deleted_playlists":[]}"#.as_slice(),
            br#"{"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456},"deleted_playlists":["43"]}"#,
            br#"{"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456},"deleted_playlists":["42","43"]}"#,
            br#"{"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456},"deleted_playlists":[42]}"#,
            br#"{"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456},"deleted_playlists":["042"]}"#,
            br#"{"status_info":{"log_id":"request-log","now":123,"now_ts_ms":0},"deleted_playlists":["42"]}"#,
            br#"{"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456,"status_code":0},"deleted_playlists":["42"]}"#,
            br#"{"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456},"deleted_playlists":["42"],"unexpected":true}"#,
        ] {
            assert_eq!(
                validate_playlist_delete_ack(body, "42").unwrap_err().code,
                ErrorCode::UpstreamError,
                "{}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn playlist_media_write_ack_requires_the_pc_renderer_success_status() {
        for body in [
            br#"{"status_code":0}"#.as_slice(),
            br#"{"status_code":0,"status_info":{"status_code":0}}"#,
        ] {
            validate_playlist_media_write_ack(body, "42").unwrap();
        }
        for body in [
            br#"{}"#.as_slice(),
            br#"{"status_info":{"status_code":0}}"#,
            br#"{"status_code":7}"#,
            br#"{"status_code":0,"status_info":{"status_code":7}}"#,
            br#"{"status_code":"0"}"#,
        ] {
            assert_eq!(
                validate_playlist_media_write_ack(body, "42")
                    .unwrap_err()
                    .code,
                ErrorCode::UpstreamError,
                "{}",
                String::from_utf8_lossy(body)
            );
        }
        assert_eq!(
            validate_playlist_media_write_ack(br#"{"status_code":1000016}"#, "42")
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
        assert_eq!(
            validate_playlist_media_write_ack(
                br#"{"status_code":0,"status_info":{"status_code":1000016}}"#,
                "42"
            )
            .unwrap_err()
            .code,
            ErrorCode::AuthenticationRequired
        );
        validate_playlist_media_write_ack(
            br#"{"status_info":{"log_id":"safe-log","now":1,"now_ts_ms":1000},"playlist":{"id":"42"},"media":[]}"#,
            "42",
        )
        .expect("the PC delete response can use its identity-bound statusless snapshot");
        for (body, requested_id) in [
            (
                br#"{"status_info":{"log_id":"safe-log","now":1,"now_ts_ms":1000},"playlist":{"id":"43"},"media":[]}"#.as_slice(),
                "42",
            ),
            (
                br#"{"status_info":{"log_id":"safe-log","now":1,"now_ts_ms":1000},"playlist":{"id":"42"}}"#,
                "42",
            ),
            (
                br#"{"status_info":{"log_id":"safe-log","now":0,"now_ts_ms":1000},"playlist":{"id":"42"},"media":[]}"#,
                "42",
            ),
        ] {
            assert_eq!(
                validate_playlist_media_write_ack(body, requested_id)
                    .unwrap_err()
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[tokio::test]
    async fn playlist_create_sdk_posts_an_empty_list_with_selected_cookie_and_visibility() {
        for is_private in [false, true] {
            let response = crate::test_http::json(
                r#"{"status_code":0,"playlist":{"id":"42"}}"#,
                Some("sessionid_ss=rotated; Path=/"),
            );
            let (origin, server) = crate::test_http::serve(vec![response]).await;
            let client = SodaClient::new(&crate::client::SodaConfig::default())
                .unwrap()
                .with_auth_test_origin(origin.clone());
            let credential = SodaCredential::test_credential("selected-secret")
                .bind_user("123456")
                .unwrap();
            let (id, refreshed) = client
                .create_owned_playlist("新歌单", is_private, &credential)
                .await
                .unwrap();
            assert_eq!(id, "42");
            assert!(refreshed.cookie_header().unwrap().contains("rotated"));
            assert!(
                credential
                    .cookie_header()
                    .unwrap()
                    .contains("selected-secret")
            );
            assert!(refreshed.same_login(&credential));

            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 1);
            let request = &requests[0];
            assert!(request.starts_with("POST /luna/pc/me/playlist?"));
            let lower_request = request.to_ascii_lowercase();
            assert!(lower_request.contains("cookie: sessionid_ss=selected-secret"));
            assert!(lower_request.contains("content-type: application/json"));
            let body = request.split_once("\r\n\r\n").unwrap().1;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(body).unwrap(),
                json!({"name":"新歌单", "is_private":is_private, "track_ids":[]})
            );
            let url = origin
                .join(
                    request
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap(),
                )
                .unwrap();
            let query = url.query_pairs().collect::<BTreeMap<_, _>>();
            assert_eq!(query["aid"], "386088");
            assert_eq!(query["app_name"], "luna_pc");
            assert_eq!(query["device_platform"], "windows");
            assert_eq!(query["version_name"], "3.7.0");
            assert_eq!(query["version_code"], "30070000");
            assert_eq!(query["channel"], "official");
            assert_eq!(query["fp"], query["device_id"]);
            assert!(!query["device_id"].is_empty());
            assert_eq!(query["iid"], "");
            assert!(!query.contains_key("install_id"));
            assert!(!query.contains_key("user_id"));
        }
    }

    #[tokio::test]
    async fn playlist_visibility_sdk_posts_only_id_and_visibility_to_pc_update() {
        let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
            r#"{"status_code":0}"#,
            Some("sessionid_ss=rotated; Path=/"),
        )])
        .await;
        let client = SodaClient::new(&crate::client::SodaConfig::default())
            .unwrap()
            .with_auth_test_origin(origin.clone());
        let credential = SodaCredential::test_credential("selected-secret")
            .bind_user("123456")
            .unwrap();
        let refreshed = client
            .update_owned_playlist_visibility("42", true, &credential)
            .await
            .unwrap();
        assert!(refreshed.cookie_header().unwrap().contains("rotated"));
        assert!(refreshed.same_login(&credential));

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert!(request.starts_with("POST /luna/pc/me/playlist/update?"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("cookie: sessionid_ss=selected-secret")
        );
        let body = request.split_once("\r\n\r\n").unwrap().1;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            json!({"playlist_id":"42", "is_private":true})
        );
        let url = origin
            .join(
                request
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap(),
            )
            .unwrap();
        let query = url.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(query["aid"], "386088");
        assert_eq!(query["app_name"], "luna_pc");
        assert_eq!(query["device_platform"], "windows");
        assert_eq!(query["version_name"], "3.7.0");
        assert_eq!(query["version_code"], "30070000");
        assert_eq!(query["channel"], "official");
        assert_eq!(query["fp"], query["device_id"]);
        assert!(!query["device_id"].is_empty());
        assert_eq!(query["iid"], "");
        assert!(!query.contains_key("install_id"));
        assert!(!query.contains_key("user_id"));
    }

    #[tokio::test]
    async fn playlist_metadata_sdk_updates_only_the_selected_pc_fields() {
        let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
            r#"{"status_code":0}"#,
            Some("sessionid_ss=rotated; Path=/"),
        )])
        .await;
        let client = SodaClient::new(&crate::client::SodaConfig::default())
            .unwrap()
            .with_auth_test_origin(origin.clone());
        let credential = SodaCredential::test_credential("selected-secret")
            .bind_user("123456")
            .unwrap();
        let refreshed = client
            .update_owned_playlist_metadata("42", Some("新标题"), None, &credential)
            .await
            .unwrap();
        assert!(refreshed.cookie_header().unwrap().contains("rotated"));
        assert!(refreshed.same_login(&credential));

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert!(request.starts_with("POST /luna/pc/me/playlist/update?"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("cookie: sessionid_ss=selected-secret")
        );
        let body = request.split_once("\r\n\r\n").unwrap().1;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            json!({"playlist_id":"42", "name":"新标题"})
        );
        let url = origin
            .join(
                request
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap(),
            )
            .unwrap();
        let query = url.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(query["aid"], "386088");
        assert_eq!(query["app_name"], "luna_pc");
        assert_eq!(query["device_platform"], "windows");
        assert_eq!(query["version_name"], "3.7.0");
        assert_eq!(query["version_code"], "30070000");
        assert_eq!(query["channel"], "official");
        assert_eq!(query["fp"], query["device_id"]);
        assert!(!query["device_id"].is_empty());
        assert_eq!(query["iid"], "");
        assert!(!query.contains_key("install_id"));
        assert!(!query.contains_key("user_id"));
    }

    #[tokio::test]
    async fn playlist_media_write_sdk_uses_the_selected_cookie_and_official_pc_wire_shape() {
        for add in [true, false] {
            let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
                r#"{"status_code":0}"#,
                Some("sessionid_ss=rotated; Path=/"),
            )])
            .await;
            let client = SodaClient::new(&crate::client::SodaConfig::default())
                .unwrap()
                .with_auth_test_origin(origin.clone());
            let credential = SodaCredential::test_credential("selected-secret")
                .bind_user("123456")
                .unwrap();
            let refreshed = client
                .mutate_owned_playlist_media(
                    "42",
                    &["101".to_owned(), "202".to_owned()],
                    add,
                    &credential,
                )
                .await
                .unwrap();
            assert!(refreshed.cookie_header().unwrap().contains("rotated"));
            assert!(refreshed.same_login(&credential));

            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 1);
            let request = &requests[0];
            assert!(request.starts_with(if add {
                "POST /luna/pc/me/playlist/media/append?"
            } else {
                "POST /luna/pc/me/playlist/media/delete?"
            }));
            assert!(
                request
                    .to_ascii_lowercase()
                    .contains("cookie: sessionid_ss=selected-secret")
            );
            let body = request.split_once("\r\n\r\n").unwrap().1;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(body).unwrap(),
                json!({
                    "playlist_id":"42",
                    "media":[
                        {"id":"101", "type":"track"},
                        {"id":"202", "type":"track"}
                    ]
                })
            );
            let url = origin
                .join(
                    request
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap(),
                )
                .unwrap();
            let query = url.query_pairs().collect::<BTreeMap<_, _>>();
            assert_eq!(query["aid"], "386088");
            assert_eq!(query["app_name"], "luna_pc");
            assert_eq!(query["device_platform"], "windows");
            assert_eq!(query["version_name"], "3.7.0");
            assert_eq!(query["version_code"], "30070000");
            assert_eq!(query["channel"], "official");
            assert_eq!(query["fp"], query["device_id"]);
            assert!(!query["device_id"].is_empty());
            assert_eq!(query["iid"], "");
            assert!(!query.contains_key("install_id"));
            assert!(!query.contains_key("user_id"));
        }
    }

    fn page(section: LibrarySection, value: serde_json::Value) -> Result<LibraryPage> {
        parse_page(
            section,
            "123456",
            &SodaCredential::test_credential("secret"),
            &serde_json::to_vec(&value).unwrap(),
        )
    }

    #[test]
    fn library_sections_preserve_actual_owners_and_filter_mixed_kinds_before_paging() {
        let page = page(LibrarySection::Saved, json!({"mixed_collections":[
            {"item_type":"album"},
            {"item_type":"playlist","playlist":{"id":"11","title":"saved","owner":{"id":"654321","nickname":"creator"}}},
            {"item_type":"playlist","playlist":{"id":"12","title":"unknown owner","type":7}}
        ],"total_num":3})).unwrap();
        assert_eq!(page.raw_count, 3);
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].extensions["owner_id"], "654321");
        assert_eq!(page.items[0].extensions["source_user_id"], "123456");
        assert_eq!(page.items[0].subscribed, Some(true));
        assert!(page.items[1].creator.is_none());
        assert!(!page.items[1].extensions.contains_key("owner_id"));
        assert!(!page.items[1].extensions.contains_key("visibility"));
        assert_eq!(page.items[1].extensions["playlist_type"], 7);
        assert!(
            LibraryPagination::default()
                .accept("0", &page)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn saved_library_accepts_only_the_official_status_only_empty_response() {
        let empty = page(
            LibrarySection::Saved,
            json!({"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456}}),
        )
        .unwrap();
        assert!(empty.items.is_empty());
        assert_eq!(empty.raw_count, 0);
        assert_eq!(empty.total, Some(0));
        assert_eq!(empty.has_more, Some(false));
        assert!(
            LibraryPagination::default()
                .accept("0", &empty)
                .unwrap()
                .is_none()
        );

        for invalid in [
            json!({"status_info":{"now":123,"now_ts_ms":123456}}),
            json!({"status_info":{"log_id":"request-log","now":0,"now_ts_ms":123456}}),
            json!({"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456},"total_num":0}),
        ] {
            assert!(page(LibrarySection::Saved, invalid).is_err());
        }
        assert!(
            page(
                LibrarySection::Created,
                json!({"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456}}),
            )
            .is_err()
        );
    }

    #[test]
    fn library_rejects_missing_data_invalid_owners_and_error_envelopes() {
        for body in [
            json!({}),
            json!({"status_code":0,"total_num":0}),
            json!({"status_code":9,"playlists":[],"total_num":0}),
            json!({"status_info":{"status_code":9},"playlists":[],"total_num":0}),
            json!({"playlists":[{"id":"01","title":"invalid"}],"total_num":1}),
            json!({"playlists":[{"id":"11","title":"wrong owner","owner":{"id":"654321"}}],"total_num":1}),
            json!({"playlists":[],"total_num":10001}),
        ] {
            assert!(page(LibrarySection::Created, body).is_err());
        }
        assert!(
            page(
                LibrarySection::Saved,
                json!({"mixed_collections":[{"item_type":"playlist"}],"total_num":1})
            )
            .is_err()
        );
        let error = page(LibrarySection::Created, json!({"status_code":1000016}))
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
        let valid = page(
            LibrarySection::Created,
            json!({"playlists":[],"total_num":0}),
        )
        .unwrap();
        assert!(
            LibraryPagination::default()
                .accept("0", &valid)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn library_pagination_requires_provable_completion_and_nonrepeating_progress() {
        for body in [
            json!({"playlists":[]}),
            json!({"playlists":[],"total_num":1}),
            json!({"playlists":[],"has_more":true,"next_cursor":"1"}),
            json!({"playlists":[{"id":"11","title":"one"}],"total_num":2,"has_more":false}),
            json!({"playlists":[{"id":"11","title":"one"}],"has_more":true,"next_cursor":"0"}),
            json!({"playlists":[{"id":"11","title":"one"}],"has_more":true,"next_cursor":"bad cursor"}),
            json!({"playlists":[{"id":"11","title":"one"}],"has_more":true}),
            json!({"playlists":[{"id":"11","title":"one"}],"has_more":true,"total_num":1,"next_cursor":"1"}),
        ] {
            let page = page(LibrarySection::Created, body).unwrap();
            assert!(LibraryPagination::default().accept("0", &page).is_err());
        }
        let first = page(LibrarySection::Created, json!({"playlists":[{"id":"11","title":"one"}],"has_more":true,"next_cursor":"opaque-1","total_num":2})).unwrap();
        for body in [
            json!({"playlists":[{"id":"12","title":"two"}],"has_more":false,"total_num":3}),
            json!({"playlists":[{"id":"11","title":"one"}],"has_more":false,"total_num":2}),
            json!({"playlists":[{"id":"12","title":"two"}],"has_more":true,"next_cursor":"0"}),
        ] {
            let mut state = LibraryPagination::default();
            assert_eq!(
                state.accept("0", &first).unwrap().as_deref(),
                Some("opaque-1")
            );
            assert!(
                state
                    .accept("opaque-1", &page(LibrarySection::Created, body).unwrap())
                    .is_err()
            );
        }
        let mut state = LibraryPagination::default();
        state.accept("0", &first).unwrap();
        assert!(
            state
                .accept(
                    "opaque-1",
                    &page(
                        LibrarySection::Created,
                        json!({"playlists":[{"id":"12","title":"two"}],"has_more":false})
                    )
                    .unwrap()
                )
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn library_pagination_stops_at_request_budget_without_claiming_success() {
        let mut state = LibraryPagination::default();
        for index in 0..MAX_LIBRARY_PAGES {
            let page = page(LibrarySection::Created, json!({"playlists":[{"id":(index+1).to_string(),"title":"test"}],"has_more":true,"next_cursor":(index+1).to_string()})).unwrap();
            let result = state.accept(&index.to_string(), &page);
            if index + 1 == MAX_LIBRARY_PAGES {
                assert!(result.is_err());
            } else {
                assert_eq!(result.unwrap(), Some((index + 1).to_string()));
            }
        }
    }
}
