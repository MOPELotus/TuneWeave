use std::{
    process,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use md5::{Digest, Md5};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::{Client, StatusCode, header};
use serde::Serialize;
use serde_json::{Map, Value, json};
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};

use crate::crypto::encrypt_eapi;

const DEFAULT_BASE_URL: &str = "https://interface.music.163.com";
const DEFAULT_USER_AGENT: &str = "NeteaseMusic 9.0.90/5038 (iPhone; iOS 16.2; zh_CN)";
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct NeteaseConfig {
    pub base_url: String,
    pub cookie: Option<String>,
    pub timeout: Duration,
    pub user_agent: String,
}

impl Default for NeteaseConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            cookie: None,
            timeout: Duration::from_secs(15),
            user_agent: DEFAULT_USER_AGENT.to_owned(),
        }
    }
}

#[derive(Clone)]
pub struct NeteaseClient {
    http: Client,
    base_url: String,
    cookie: Option<String>,
    device_id: String,
}

#[derive(Clone, Debug)]
pub struct NeteaseResponse {
    pub status: StatusCode,
    pub body: Value,
    pub cookies: Vec<String>,
}

#[derive(Serialize)]
struct EapiHeader<'a> {
    osver: &'a str,
    #[serde(rename = "deviceId")]
    device_id: &'a str,
    os: &'a str,
    appver: &'a str,
    versioncode: &'a str,
    mobilename: &'a str,
    buildver: String,
    resolution: &'a str,
    __csrf: &'a str,
    channel: &'a str,
    #[serde(rename = "requestId")]
    request_id: String,
    #[serde(rename = "MUSIC_U", skip_serializing_if = "Option::is_none")]
    music_u: Option<&'a str>,
    #[serde(rename = "MUSIC_A", skip_serializing_if = "Option::is_none")]
    music_a: Option<&'a str>,
}

impl NeteaseClient {
    pub fn new(config: NeteaseConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(config.timeout)
            .connect_timeout(Duration::from_secs(8))
            .user_agent(config.user_agent)
            .build()
            .map_err(|error| {
                TuneWeaveError::new(
                    ErrorCode::InternalError,
                    format!("failed to build NetEase HTTP client: {error}"),
                )
                .with_platform(Platform::Netease)
            })?;

        let seed = format!(
            "{}:{}:{}",
            process::id(),
            unix_time_millis(),
            REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let device_id = hex::encode(Md5::digest(seed.as_bytes()));

        Ok(Self {
            http,
            base_url: config.base_url.trim_end_matches('/').to_owned(),
            cookie: config.cookie,
            device_id,
        })
    }

    pub async fn request_eapi(&self, path: &str, payload: Value) -> Result<NeteaseResponse> {
        if !path.starts_with("/api/") {
            return Err(TuneWeaveError::invalid_request(
                "NetEase EAPI paths must start with /api/",
            )
            .with_platform(Platform::Netease));
        }

        let cookie = CookieValues::parse(self.cookie.as_deref());
        let header = EapiHeader {
            osver: "Microsoft-Windows-10-Professional-build-19045-64bit",
            device_id: &self.device_id,
            os: "pc",
            appver: "3.1.17.204416",
            versioncode: "140",
            mobilename: "",
            buildver: unix_time_seconds().to_string(),
            resolution: "1920x1080",
            __csrf: cookie.csrf.unwrap_or(""),
            channel: "netease",
            request_id: request_id(),
            music_u: cookie.music_u,
            music_a: cookie.music_a,
        };
        let mut payload = payload_object(payload)?;
        payload.insert("e_r".to_owned(), Value::Bool(false));
        payload.insert(
            "header".to_owned(),
            serde_json::to_value(&header).map_err(json_error)?,
        );
        let payload = serde_json::to_string(&payload).map_err(json_error)?;
        let params = encrypt_eapi(path, &payload);
        let endpoint = format!(
            "{}/eapi/{}",
            self.base_url,
            path.trim_start_matches("/api/")
        );
        let response = self
            .http
            .post(endpoint)
            .header(header::COOKIE, encode_cookie_header(&header))
            .form(&[("params", params)])
            .send()
            .await
            .map_err(request_error)?;

        let status = response.status();
        let cookies = response
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|value| value.to_str().ok().map(str::to_owned))
            .collect();
        let bytes = response.bytes().await.map_err(request_error)?;
        if !status.is_success() {
            return Err(TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned HTTP {status}"),
            )
            .with_platform(Platform::Netease)
            .retryable(status.is_server_error())
            .with_details(json!({ "status": status.as_u16() })));
        }

        let body = serde_json::from_slice(&bytes).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned invalid JSON: {error}"),
            )
            .with_platform(Platform::Netease)
            .with_details(json!({
                "response_preview": String::from_utf8_lossy(&bytes[..bytes.len().min(256)])
            }))
        })?;
        Ok(NeteaseResponse {
            status,
            body,
            cookies,
        })
    }
}

struct CookieValues<'a> {
    csrf: Option<&'a str>,
    music_u: Option<&'a str>,
    music_a: Option<&'a str>,
}

impl<'a> CookieValues<'a> {
    fn parse(cookie: Option<&'a str>) -> Self {
        let mut values = Self {
            csrf: None,
            music_u: None,
            music_a: None,
        };
        for part in cookie.unwrap_or_default().split(';') {
            let Some((name, value)) = part.trim().split_once('=') else {
                continue;
            };
            match name {
                "__csrf" => values.csrf = Some(value),
                "MUSIC_U" => values.music_u = Some(value),
                "MUSIC_A" => values.music_a = Some(value),
                _ => {}
            }
        }
        values
    }
}

fn payload_object(payload: Value) -> Result<Map<String, Value>> {
    payload.as_object().cloned().ok_or_else(|| {
        TuneWeaveError::invalid_request("NetEase EAPI payload must be a JSON object")
            .with_platform(Platform::Netease)
    })
}

fn encode_cookie_header(header: &EapiHeader<'_>) -> String {
    let value = serde_json::to_value(header).expect("EAPI header always serializes");
    value
        .as_object()
        .expect("serialized EAPI header is an object")
        .iter()
        .filter_map(|(name, value)| value.as_str().map(|value| (name, value)))
        .map(|(name, value)| {
            format!(
                "{}={}",
                utf8_percent_encode(name, NON_ALPHANUMERIC),
                utf8_percent_encode(value, NON_ALPHANUMERIC)
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn request_error(error: reqwest::Error) -> TuneWeaveError {
    let code = if error.is_timeout() {
        ErrorCode::UpstreamTimeout
    } else {
        ErrorCode::UpstreamError
    };
    TuneWeaveError::new(code, format!("NetEase request failed: {error}"))
        .with_platform(Platform::Netease)
        .retryable(true)
}

fn json_error(error: serde_json::Error) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        format!("failed to serialize NetEase request: {error}"),
    )
    .with_platform(Platform::Netease)
}

fn request_id() -> String {
    let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed) % 10_000;
    format!("{}_{sequence:04}", unix_time_millis())
}

fn unix_time_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn unix_time_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_only_authentication_cookie_values() {
        let values = CookieValues::parse(Some(
            "MUSIC_U=music-user; __csrf=csrf-token; ignored=value; MUSIC_A=anonymous",
        ));
        assert_eq!(values.music_u, Some("music-user"));
        assert_eq!(values.csrf, Some("csrf-token"));
        assert_eq!(values.music_a, Some("anonymous"));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_eapi_search_returns_songs() {
        let client = NeteaseClient::new(NeteaseConfig::default()).expect("build client");
        let response = client
            .request_eapi(
                "/api/search/get",
                json!({
                    "s": "反方向的钟",
                    "type": 1,
                    "limit": 2,
                    "offset": 0
                }),
            )
            .await
            .expect("live search succeeds");
        assert_eq!(response.body["code"], 200);
        assert!(
            response.body["result"]["songs"]
                .as_array()
                .is_some_and(|songs| !songs.is_empty())
        );
    }
}
