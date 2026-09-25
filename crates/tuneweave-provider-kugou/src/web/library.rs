//! The legacy website's network collections have their own opaque list identities.
use super::*;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use reqwest::header::SET_COOKIE;
use serde_json::Value;
use std::collections::BTreeSet;
use tuneweave_core::{Extensions, Playlist, ResourceRef};

const LIBRARY_HOST: &str = "www.kugou.com";
const LIBRARY_PATH: &str = "/uc/getdata.php";
pub(crate) const PREFIX: &str = "legacy_web_collection:";
const LIMIT: usize = 2 * 1024 * 1024;
const MAX_LISTS: usize = 4096;
const MAX_TRACKS: usize = 16_384;

pub(crate) struct WebLibrary {
    pub(crate) items: Vec<Playlist>,
    pub(crate) storage_bytes: u64,
    pub(crate) candidate: WebSession,
}

pub(crate) struct WebLibraryTrack {
    pub(crate) file_hash: String,
    pub(crate) file_name: String,
    pub(crate) duration_ms: u64,
}

pub(crate) struct WebLibraryTracks {
    pub(crate) items: Vec<WebLibraryTrack>,
    pub(crate) candidate: WebSession,
}

#[derive(Deserialize)]
struct Directory {
    #[serde(rename = "totalSize")]
    storage_bytes: Value,
    list: Vec<Entry>,
    status: Option<Value>,
    errno: Option<Value>,
    error_code: Option<Value>,
    error: Option<Value>,
}
#[derive(Deserialize)]
struct Entry {
    #[serde(rename = "listID")]
    id: Value,
    #[serde(rename = "listName")]
    name: String,
}

fn opaque_id(value: Value) -> Result<String> {
    let value = match value {
        Value::String(value) => value,
        Value::Number(value) => value.as_u64().ok_or_else(malformed)?.to_string(),
        _ => return Err(malformed()),
    };
    if value.is_empty()
        || value.len() > 512
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(malformed());
    }
    Ok(value)
}

pub(crate) fn parse_reference(id: &str) -> Result<(&str, String)> {
    let (uid, encoded) = id
        .strip_prefix(PREFIX)
        .and_then(|value| value.split_once(':'))
        .filter(|(uid, encoded)| valid_uid(uid) && encoded.len() <= 683)
        .ok_or_else(invalid)?;
    let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| invalid())?;
    if URL_SAFE_NO_PAD.encode(&bytes) != encoded {
        return Err(invalid());
    }
    let raw = String::from_utf8(bytes).map_err(|_| invalid())?;
    let raw = opaque_id(Value::String(raw)).map_err(|_| invalid())?;
    Ok((uid, raw))
}

fn parse(bytes: &[u8], uid: &str) -> Result<(Vec<Playlist>, u64)> {
    if bytes.len() > LIMIT || !valid_uid(uid) {
        return Err(malformed());
    }
    let directory: Directory = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    // No business-status envelope is consumed by this producer. A failure
    // envelope with incidental list fields must not become an empty success.
    if directory.status.is_some()
        || directory.errno.is_some()
        || directory.error_code.is_some()
        || directory.error.is_some()
    {
        return Err(malformed());
    }
    let storage_bytes = match directory.storage_bytes {
        Value::Number(n) => n.as_u64(),
        Value::String(s) => s.parse::<u64>().ok().filter(|n| n.to_string() == s),
        _ => None,
    }
    .ok_or_else(malformed)?;
    if directory.list.len() > MAX_LISTS {
        return Err(malformed());
    }
    let mut seen = BTreeSet::new();
    let mut items = Vec::with_capacity(directory.list.len());
    for entry in directory.list {
        let raw = opaque_id(entry.id)?;
        if !seen.insert(raw.clone())
            || entry.name.trim().is_empty()
            || entry.name.len() > 4096
            || entry.name.chars().any(char::is_control)
        {
            return Err(malformed());
        }
        let id = format!("{PREFIX}{uid}:{}", URL_SAFE_NO_PAD.encode(raw.as_bytes()));
        items.push(Playlist {
            resource_ref: ResourceRef::new(Platform::Kugou, &id).map_err(|_| malformed())?,
            platform: Platform::Kugou,
            id,
            name: entry.name,
            description: String::new(),
            cover_url: None,
            creator: None,
            track_count: None,
            tags: Vec::new(),
            subscribed: None,
            created_at: None,
            updated_at: None,
            extensions: Extensions::from([
                ("source".into(), serde_json::json!("legacy_web_collection")),
                ("legacy_list_id".into(), serde_json::json!(raw)),
            ]),
        });
    }
    Ok((items, storage_bytes))
}

#[derive(Deserialize)]
struct TrackEntry {
    #[serde(rename = "fileHash")]
    file_hash: String,
    #[serde(rename = "fileName")]
    file_name: String,
    #[serde(rename = "fileTimeLen")]
    duration_ms: Value,
}

fn parse_tracks(bytes: &[u8]) -> Result<Vec<WebLibraryTrack>> {
    if bytes.len() > LIMIT {
        return Err(malformed());
    }
    let entries: Vec<TrackEntry> = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if entries.len() > MAX_TRACKS {
        return Err(malformed());
    }
    entries
        .into_iter()
        .map(|entry| {
            let duration_ms = match entry.duration_ms {
                Value::Number(value) => value.as_u64(),
                Value::String(value) if !value.is_empty() && value.trim() == value => value
                    .parse::<u64>()
                    .ok()
                    .filter(|parsed| parsed.to_string() == value),
                _ => None,
            }
            .ok_or_else(malformed)?;
            if entry.file_hash.is_empty()
                || entry.file_hash.len() > 512
                || entry.file_hash.trim() != entry.file_hash
                || entry.file_hash.chars().any(char::is_control)
                || entry.file_name.trim().is_empty()
                || entry.file_name.len() > 4096
                || entry.file_name.chars().any(char::is_control)
            {
                return Err(malformed());
            }
            Ok(WebLibraryTrack {
                file_hash: entry.file_hash,
                file_name: entry.file_name,
                duration_ms,
            })
        })
        .collect()
}

fn response_candidate(
    session: &WebSession,
    headers: &reqwest::header::HeaderMap,
) -> Result<WebSession> {
    let mut candidate = session.clone();
    if headers.get_all(SET_COOKIE).iter().count() > 64 {
        return Err(malformed());
    }
    let mut changed = false;
    for header in headers.get_all(SET_COOKIE) {
        let raw = header.to_str().map_err(|_| malformed())?;
        if raw.len() > 20_480 {
            return Err(malformed());
        }
        changed |= raw
            .split_once('=')
            .is_some_and(|(name, _)| name.trim() == "KuGoo");
    }
    if changed {
        let cookie = WebCookie::received(headers, crate::account::now_ms()? / 1000)?;
        cookie.media_token(crate::account::now_ms()? / 1000)?;
        if cookie.identity()?.user_id != session.user_id {
            return Err(conflict());
        }
        candidate.cookie = cookie;
    }
    Ok(candidate)
}

impl KugouClient {
    pub(crate) async fn legacy_web_library(&self, session: &WebSession) -> Result<WebLibrary> {
        if !session.valid() {
            return Err(invalid());
        }
        let now = crate::account::now_ms()? / 1000;
        // Only an explicitly shared root Cookie is available to www.kugou.com.
        session.cookie.media_token(now)?;
        let cookie = session.cookie.header(now)?;
        let url = format!("https://{LIBRARY_HOST}{LIBRARY_PATH}");
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(LIBRARY_PATH).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .post(url)
                .query(&[("type", "16")])
                .header(COOKIE, cookie)
                .header(ORIGIN, "https://www.kugou.com")
                .header(REFERER, "https://www.kugou.com/uc/view/ikugou.html")
                .form(&[("n", rand::random::<f64>().to_string())])
                .send()
                .await
                .map_err(crate::account::network_error)?;
            status = Some(response.status());
            let headers = response.headers().clone();
            // The official XHR parses responseText, including PHP's default MIME.
            // These are always strict JSON bytes; HTML/scripts are never executed.
            let bytes = crate::account::read_response_with_types(
                response,
                LIMIT,
                &["application/json", "text/plain", "text/html"],
            )
            .await?;
            let (items, storage_bytes) = parse(&bytes, &session.user_id)?;
            let candidate = response_candidate(session, &headers)?;
            Ok(WebLibrary {
                items,
                storage_bytes,
                candidate,
            })
        }
        .await;
        self.log_upstream_request(
            "legacy_web_collection_directory",
            LIBRARY_HOST,
            LIBRARY_PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }

    pub(crate) async fn legacy_web_library_tracks(
        &self,
        session: &WebSession,
        list_id: &str,
    ) -> Result<WebLibraryTracks> {
        if !session.valid() {
            return Err(invalid());
        }
        let list_id = opaque_id(Value::String(list_id.to_owned()))?;
        let now = crate::account::now_ms()? / 1000;
        session.cookie.media_token(now)?;
        let cookie = session.cookie.header(now)?;
        let url = format!("https://{LIBRARY_HOST}{LIBRARY_PATH}");
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(LIBRARY_PATH).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .post(url)
                .query(&[("type", "17"), ("listid", list_id.as_str())])
                .header(COOKIE, cookie)
                .header(ORIGIN, "https://www.kugou.com")
                .header(REFERER, "https://www.kugou.com/uc/view/ikugou.html")
                .form(&[("n", rand::random::<f64>().to_string())])
                .send()
                .await
                .map_err(crate::account::network_error)?;
            status = Some(response.status());
            let headers = response.headers().clone();
            let bytes = crate::account::read_response_with_types(
                response,
                LIMIT,
                &["application/json", "text/plain", "text/html"],
            )
            .await?;
            let items = parse_tracks(&bytes)?;
            let candidate = response_candidate(session, &headers)?;
            Ok(WebLibraryTracks { items, candidate })
        }
        .await;
        self.log_upstream_request(
            "legacy_web_collection_tracks",
            LIBRARY_HOST,
            LIBRARY_PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

#[cfg(test)]
mod tests;
