use std::{collections::BTreeSet, time::Instant};

use reqwest::{
    Method,
    header::{ACCEPT, CONTENT_TYPE, COOKIE},
};
use serde::Deserialize;
use serde_json::json;
use tuneweave_core::{
    Album, ArtistSummary, ErrorCode, Extensions, Platform, Playlist, ResourceRef, Result,
    TuneWeaveError,
};
use url::Url;

use crate::{
    client::{
        SodaClient, SodaImage, normalize_image, read_bounded_response, soda_http_error,
        soda_network_error, soda_upstream_error,
    },
    login::SodaCredential,
};

pub(crate) const LIBRARY_PAGE_SIZE: usize = 100;
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
struct CollectionWriteEnvelope {
    status_code: Option<i64>,
    status_info: Option<StatusInfo>,
}

#[derive(Deserialize)]
struct PlaylistCreateEnvelope {
    status_code: Option<i64>,
    status_info: Option<StatusInfo>,
    playlist: Option<CreatedPlaylist>,
}

#[derive(Deserialize)]
struct CreatedPlaylist {
    id: Option<String>,
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
        let mut url = Url::parse("https://api.qishui.com")
            .map_err(|_| soda_upstream_error("Soda library endpoint is invalid"))?;
        url.set_path(path);
        url.query_pairs_mut()
            .append_pair("aid", "386088")
            .append_pair("iid", &device.install_id)
            .append_pair("device_id", &device.device_id)
            .append_pair("version_code", "30050100");
        if let Some(cursor) = cursor {
            url.query_pairs_mut()
                .append_pair("cursor", cursor)
                .append_pair("count", &LIBRARY_PAGE_SIZE.to_string());
        }
        Ok(url)
    }

    fn pc_playlist_write_url(&self, path: &str) -> Result<Url> {
        let device = self.login_device()?;
        let mut url = Url::parse("https://api.qishui.com")
            .map_err(|_| soda_upstream_error("Soda playlist write endpoint is invalid"))?;
        url.set_path(path);
        url.query_pairs_mut()
            .append_pair("aid", "386088")
            .append_pair("app_name", "luna_pc")
            .append_pair("device_platform", "windows")
            .append_pair("version_name", "2.1.0")
            .append_pair("version_code", "20010000")
            .append_pair("channel", "official")
            .append_pair("device_id", &device.device_id)
            .append_pair("iid", &device.install_id)
            .append_pair("fp", &device.device_id);
        Ok(url)
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
                .login_request(Method::POST, self.pc_playlist_write_url(path)?)
                .header(ACCEPT, "application/json")
                .header(COOKIE, credential.cookie_header()?)
                .json(&json!({
                    "name": name,
                    "is_private": is_private,
                    "track_ids": [],
                }))
                .send()
                .await
                .map_err(soda_network_error)?;
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
            let id = parse_playlist_create_ack(&body)?;
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
                .login_request(Method::POST, self.pc_playlist_write_url(path)?)
                .header(ACCEPT, "application/json")
                .header(COOKIE, credential.cookie_header()?)
                .json(&body)
                .send()
                .await
                .map_err(soda_network_error)?;
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
            validate_playlist_update_ack(&response_body)?;
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
                .login_request(Method::POST, self.pc_playlist_write_url(path)?)
                .header(ACCEPT, "application/json")
                .header(COOKIE, credential.cookie_header()?)
                .json(&json!({"playlist_ids": [id]}))
                .send()
                .await
                .map_err(soda_network_error)?;
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
            validate_playlist_delete_ack(&response_body)?;
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
                .login_request(Method::POST, self.pc_playlist_write_url(path)?)
                .header(ACCEPT, "application/json")
                .header(COOKIE, credential.cookie_header()?)
                .json(&json!({"playlist_id": id, "media": media}))
                .send()
                .await
                .map_err(soda_network_error)?;
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
            validate_playlist_media_write_ack(&response_body)?;
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
                .login_request(Method::POST, self.library_url(path, None)?)
                .header(ACCEPT, "application/json")
                .header(COOKIE, credential.cookie_header()?)
                .json(&body)
                .send()
                .await
                .map_err(soda_network_error)?;
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
                .login_request(Method::GET, url)
                .header(ACCEPT, "application/json")
                .header(COOKIE, credential.cookie_header()?)
                .send()
                .await
                .map_err(soda_network_error)?;
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
            let mut page = parse(section, owner, credential, &body)?;
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
    let envelope: PlaylistCreateEnvelope = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda playlist creation returned invalid data"))?;
    let status_code = envelope
        .status_code
        .ok_or_else(|| soda_upstream_error("Soda playlist creation omitted its result status"))?;
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
    let id = envelope
        .playlist
        .and_then(|playlist| playlist.id)
        .filter(|id| valid_id(id))
        .ok_or_else(|| {
            soda_upstream_error("Soda playlist creation omitted a valid playlist identity")
        })?;
    Ok(id)
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

fn validate_playlist_delete_ack(body: &[u8]) -> Result<()> {
    let envelope: CollectionWriteEnvelope = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda playlist deletion returned invalid data"))?;
    let status_code = envelope
        .status_code
        .ok_or_else(|| soda_upstream_error("Soda playlist deletion omitted its result status"))?;
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
    Ok(())
}

fn validate_playlist_media_write_ack(body: &[u8]) -> Result<()> {
    let envelope: CollectionWriteEnvelope = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda playlist media write returned invalid data"))?;
    let status_code = envelope.status_code.ok_or_else(|| {
        soda_upstream_error("Soda playlist media write omitted its result status")
    })?;
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
    Ok(LibraryPage {
        items: sources
            .into_iter()
            .map(|item| map_playlist(item, section, owner))
            .collect::<Result<_>>()?,
        credential: credential.clone(),
        raw_count,
        total: envelope.total_num,
        next_cursor: envelope.next_cursor,
        has_more: envelope.has_more,
        unclassified_items,
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
            validate_playlist_delete_ack(body).unwrap();
        }
        for body in [
            br#"{}"#.as_slice(),
            br#"{"status_info":{"status_code":0}}"#,
            br#"{"status_code":7}"#,
            br#"{"status_code":0,"status_info":{"status_code":7}}"#,
            br#"{"status_code":"0"}"#,
        ] {
            assert_eq!(
                validate_playlist_delete_ack(body).unwrap_err().code,
                ErrorCode::UpstreamError,
                "{}",
                String::from_utf8_lossy(body)
            );
        }
        assert_eq!(
            validate_playlist_delete_ack(br#"{"status_code":1000016}"#)
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
    }

    #[test]
    fn playlist_media_write_ack_requires_the_pc_renderer_success_status() {
        for body in [
            br#"{"status_code":0}"#.as_slice(),
            br#"{"status_code":0,"status_info":{"status_code":0}}"#,
        ] {
            validate_playlist_media_write_ack(body).unwrap();
        }
        for body in [
            br#"{}"#.as_slice(),
            br#"{"status_info":{"status_code":0}}"#,
            br#"{"status_code":7}"#,
            br#"{"status_code":0,"status_info":{"status_code":7}}"#,
            br#"{"status_code":"0"}"#,
        ] {
            assert_eq!(
                validate_playlist_media_write_ack(body).unwrap_err().code,
                ErrorCode::UpstreamError,
                "{}",
                String::from_utf8_lossy(body)
            );
        }
        assert_eq!(
            validate_playlist_media_write_ack(br#"{"status_code":1000016}"#)
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
        assert_eq!(
            validate_playlist_media_write_ack(
                br#"{"status_code":0,"status_info":{"status_code":1000016}}"#
            )
            .unwrap_err()
            .code,
            ErrorCode::AuthenticationRequired
        );
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
            assert_eq!(query["version_name"], "2.1.0");
            assert_eq!(query["version_code"], "20010000");
            assert_eq!(query["channel"], "official");
            assert_eq!(query["fp"], query["device_id"]);
            assert!(!query["device_id"].is_empty());
            assert!(!query["iid"].is_empty());
            assert_ne!(query["device_id"], query["iid"]);
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
        assert_eq!(query["version_name"], "2.1.0");
        assert_eq!(query["version_code"], "20010000");
        assert_eq!(query["channel"], "official");
        assert_eq!(query["fp"], query["device_id"]);
        assert!(!query["device_id"].is_empty());
        assert!(!query["iid"].is_empty());
        assert_ne!(query["device_id"], query["iid"]);
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
        assert_eq!(query["version_name"], "2.1.0");
        assert_eq!(query["version_code"], "20010000");
        assert_eq!(query["channel"], "official");
        assert_eq!(query["fp"], query["device_id"]);
        assert!(!query["device_id"].is_empty());
        assert!(!query["iid"].is_empty());
        assert_ne!(query["device_id"], query["iid"]);
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
            assert_eq!(query["version_name"], "2.1.0");
            assert_eq!(query["version_code"], "20010000");
            assert_eq!(query["channel"], "official");
            assert_eq!(query["fp"], query["device_id"]);
            assert!(!query["device_id"].is_empty());
            assert!(!query["iid"].is_empty());
            assert_ne!(query["device_id"], query["iid"]);
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
