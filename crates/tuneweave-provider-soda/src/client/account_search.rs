use super::*;
use crate::login::SodaCredential;
use tuneweave_core::{SearchItem, SearchKind};

pub(crate) const BACKEND: &str = "official_pc_account_search";
const MAX_PAGE: u64 = 1024 * 1024;
pub(crate) fn search_id() -> String {
    let mut bytes = rand::random::<[u8; 16]>();
    bytes[6] = (bytes[6] & 15) | 64;
    bytes[8] = (bytes[8] & 63) | 128;
    let s = hex::encode(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &s[..8],
        &s[8..12],
        &s[12..16],
        &s[16..20],
        &s[20..]
    )
}
fn path(kind: SearchKind) -> Result<&'static str> {
    match kind {
        SearchKind::Track => Ok("/luna/pc/search/track"),
        SearchKind::Album => Ok("/luna/pc/search/album"),
        SearchKind::Artist => Ok("/luna/pc/search/artist"),
        SearchKind::Playlist => Ok("/luna/pc/search/playlist"),
        _ => Err(soda_invalid_request("Unsupported Soda account search kind")),
    }
}
fn invalid() -> TuneWeaveError {
    soda_upstream_error("Soda account search returned invalid data")
}
fn unauthenticated() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "Soda search session is not authenticated",
    )
    .with_platform(Platform::Soda)
}

impl SodaClient {
    pub(crate) async fn account_search_page(
        &self,
        kind: SearchKind,
        query: &str,
        cursor: u32,
        search_id: &str,
        credential: &SodaCredential,
        remaining: &mut u64,
    ) -> Result<(catalog::SodaCatalogPage, SodaCredential)> {
        let path = path(kind)?;
        let user_id = credential.user_id().ok_or_else(unauthenticated)?;
        let query = query.trim();
        if query.is_empty()
            || query.len() > 500
            || query.chars().any(char::is_control)
            || search_id.len() != 36
            || !search_id
                .bytes()
                .all(|b| b.is_ascii_hexdigit() || b == b'-')
        {
            return Err(soda_invalid_request(
                "Invalid Soda account search parameters",
            ));
        }
        if *remaining == 0 {
            return Err(invalid());
        }
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let device = self.login_device()?;
            let mut url =
                Url::parse(&format!("https://api.qishui.com{path}")).map_err(|_| invalid())?;
            url.query_pairs_mut()
                .append_pair("q", query)
                .append_pair("cursor", &cursor.to_string())
                .append_pair("search_id", search_id)
                .append_pair("search_method", "input")
                .append_pair("debug_params", "")
                .append_pair("from_search_id", "")
                .append_pair("search_scene", "")
                .append_pair("aid", SODA_APP_ID)
                .append_pair("app_name", "luna_pc")
                .append_pair("device_platform", "windows")
                .append_pair("version_name", "2.1.0")
                .append_pair("version_code", "20010000")
                .append_pair("channel", "official")
                .append_pair("device_id", &device.device_id)
                .append_pair("iid", &device.install_id)
                .append_pair("fp", &device.device_id);
            let mut response = self
                .login_request(reqwest::Method::GET, url)
                .header(reqwest::header::ACCEPT, "application/json")
                .header(reqwest::header::COOKIE, credential.cookie_header()?)
                .send()
                .await
                .map_err(soda_network_error)?;
            status = Some(response.status());
            if response.status() == StatusCode::UNAUTHORIZED {
                return Err(unauthenticated());
            }
            if response.status() != StatusCode::OK {
                return Err(soda_http_error(response.status()));
            }
            if response.headers().contains_key("bdturing-verify") {
                return Err(TuneWeaveError::new(
                    ErrorCode::CapabilityNotSupported,
                    "Soda search requires an additional platform verification challenge",
                )
                .with_platform(Platform::Soda));
            }
            if !response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
            {
                return Err(invalid());
            }
            let limit = MAX_PAGE.min(*remaining);
            if response.content_length().is_some_and(|n| n > limit) {
                return Err(invalid());
            }
            let headers = response.headers().clone();
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(soda_network_error)? {
                if bytes.len() as u64 + chunk.len() as u64 > limit {
                    return Err(invalid());
                }
                bytes.extend_from_slice(&chunk);
            }
            *remaining -= bytes.len() as u64;
            let mut page = parse_page(&bytes, kind, cursor)?;
            let updated = credential.with_response_cookies(&headers)?;
            for item in &mut page.items {
                mark(item_extensions(item), user_id);
            }
            Ok((page, updated))
        }
        .await;
        self.log_upstream_request(
            "account_search",
            "api.qishui.com",
            path,
            status,
            started,
            &result,
        );
        result
    }
}

fn parse_page(bytes: &[u8], kind: SearchKind, cursor: u32) -> Result<catalog::SodaCatalogPage> {
    let value: serde_json::Value = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    for code in [
        value.get("status_code"),
        value.get("status_info").and_then(|v| v.get("status_code")),
    ]
    .into_iter()
    .flatten()
    {
        let code = code.as_i64().ok_or_else(invalid)?;
        if code == 1_000_016 {
            return Err(unauthenticated());
        }
        if code != 0 {
            return Err(invalid());
        }
    }
    let now = value["status_info"]["now"]
        .as_u64()
        .filter(|n| *n > 0)
        .ok_or_else(invalid)?;
    if value["status_info"]["now_ts_ms"].as_u64().map(|n| n / 1000) != Some(now) {
        return Err(invalid());
    }
    let (group, entity) = match kind {
        SearchKind::Track => ("tracks", "track"),
        SearchKind::Album => ("albums", "album"),
        SearchKind::Artist => ("artists", "artist"),
        SearchKind::Playlist => ("playlists", "playlist"),
        _ => return Err(invalid()),
    };
    let groups = value
        .get("result_groups")
        .and_then(serde_json::Value::as_array)
        .filter(|v| v.len() <= 32)
        .ok_or_else(invalid)?;
    let matching = groups
        .iter()
        .filter(|g| g.get("id").and_then(serde_json::Value::as_str) == Some(group))
        .collect::<Vec<_>>();
    if matching.len() > 1 {
        return Err(invalid());
    }
    if let Some(group) = matching.first() {
        let rows = group
            .get("data")
            .and_then(serde_json::Value::as_array)
            .filter(|v| v.len() <= 20)
            .ok_or_else(invalid)?;
        let more = group
            .get("has_more")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(invalid)?;
        if more {
            let next = group
                .get("next_cursor")
                .and_then(|v| {
                    v.as_str()
                        .and_then(|s| s.parse::<u32>().ok())
                        .or_else(|| v.as_u64().and_then(|n| n.try_into().ok()))
                })
                .ok_or_else(invalid)?;
            if next <= cursor {
                return Err(invalid());
            }
            if rows.is_empty() {
                return Ok(catalog::SodaCatalogPage {
                    items: Vec::new(),
                    next_cursor: Some(next),
                    has_more: true,
                });
            }
        }
        for row in rows {
            let data = row
                .get("entity")
                .and_then(serde_json::Value::as_object)
                .ok_or_else(invalid)?;
            if !data.get(entity).is_some_and(serde_json::Value::is_object)
                || ["track", "album", "artist", "playlist"]
                    .iter()
                    .any(|k| *k != entity && data.get(*k).is_some_and(|v| !v.is_null()))
            {
                return Err(invalid());
            }
        }
    }
    let page = match kind {
        SearchKind::Track => {
            let p = super::parse_search_response(bytes, cursor)?;
            catalog::SodaCatalogPage {
                items: p.tracks.into_iter().map(SearchItem::Track).collect(),
                next_cursor: p.next_cursor,
                has_more: p.has_more,
            }
        }
        SearchKind::Album => catalog::parse_catalog(bytes, SodaCatalogKind::Album, cursor)?,
        SearchKind::Artist => catalog::parse_catalog(bytes, SodaCatalogKind::Artist, cursor)?,
        SearchKind::Playlist => catalog::parse_catalog(bytes, SodaCatalogKind::Playlist, cursor)?,
        _ => return Err(invalid()),
    };
    Ok(page)
}
pub(crate) fn item_extensions(item: &mut SearchItem) -> &mut Extensions {
    match item {
        SearchItem::Track(v) => &mut v.extensions,
        SearchItem::Album(v) => &mut v.extensions,
        SearchItem::Artist(v) => &mut v.extensions,
        SearchItem::Playlist(v) => &mut v.extensions,
        _ => unreachable!("PC search only maps four supported media types"),
    }
}
pub(crate) fn mark(extensions: &mut Extensions, user_id: &str) {
    extensions.insert("backend".into(), json!(BACKEND));
    extensions.insert("source_user_id".into(), json!(user_id));
    extensions.insert("authenticated".into(), json!(true));
}
pub(crate) use crate::account::reject_secrets;

#[cfg(test)]
pub(crate) mod tests;
