use std::{
    collections::BTreeMap,
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

use crate::crypto::{encrypt_eapi, encrypt_weapi};

const DEFAULT_BASE_URL: &str = "https://interface.music.163.com";
const DEFAULT_WEB_BASE_URL: &str = "https://music.163.com";
const DEFAULT_USER_AGENT: &str = "NeteaseMusic 9.0.90/5038 (iPhone; iOS 16.2; zh_CN)";
const DEFAULT_WEB_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36 Edg/124.0.0.0";
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct NeteaseConfig {
    pub base_url: String,
    pub web_base_url: String,
    pub cookie: Option<String>,
    pub timeout: Duration,
    pub user_agent: String,
    pub web_user_agent: String,
}

impl Default for NeteaseConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            web_base_url: DEFAULT_WEB_BASE_URL.to_owned(),
            cookie: None,
            timeout: Duration::from_secs(15),
            user_agent: DEFAULT_USER_AGENT.to_owned(),
            web_user_agent: DEFAULT_WEB_USER_AGENT.to_owned(),
        }
    }
}

#[derive(Clone)]
pub struct NeteaseClient {
    http: Client,
    base_url: String,
    web_base_url: String,
    web_user_agent: String,
    cookie: Option<String>,
    device_id: String,
}

#[derive(Clone, Debug)]
pub struct NeteaseResponse {
    pub status: StatusCode,
    pub body: Value,
    pub cookies: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NeteaseQrLogin {
    pub key: String,
    pub url: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NeteaseQrState {
    Waiting,
    Scanned,
    Confirmed,
    Expired,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NeteaseQrCheck {
    pub code: i64,
    pub state: NeteaseQrState,
    pub message: Option<String>,
    session_cookie: Option<String>,
}

impl NeteaseQrCheck {
    #[must_use]
    pub fn session_cookie(&self) -> Option<&str> {
        self.session_cookie.as_deref()
    }

    #[must_use]
    pub fn into_session_cookie(self) -> Option<String> {
        self.session_cookie
    }
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
            web_base_url: config.web_base_url.trim_end_matches('/').to_owned(),
            web_user_agent: config.web_user_agent,
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
        parse_response(response).await
    }

    pub async fn request_weapi(&self, path: &str, payload: Value) -> Result<NeteaseResponse> {
        if !path.starts_with("/api/") {
            return Err(TuneWeaveError::invalid_request(
                "NetEase WeAPI paths must start with /api/",
            )
            .with_platform(Platform::Netease));
        }

        let cookie = CookieValues::parse(self.cookie.as_deref());
        let mut payload = payload_object(payload)?;
        payload.insert(
            "csrf_token".to_owned(),
            Value::String(cookie.csrf.unwrap_or("").to_owned()),
        );
        let payload = serde_json::to_string(&payload).map_err(json_error)?;
        let encrypted = encrypt_weapi(&payload);
        let endpoint = format!(
            "{}/weapi/{}",
            self.web_base_url,
            path.trim_start_matches("/api/")
        );
        let response = self
            .http
            .post(endpoint)
            .header(header::REFERER, format!("{}/", self.web_base_url))
            .header(header::USER_AGENT, &self.web_user_agent)
            .header(
                header::COOKIE,
                weapi_cookie_header(self.cookie.as_deref(), &self.device_id),
            )
            .form(&[
                ("params", encrypted.params),
                ("encSecKey", encrypted.enc_sec_key),
            ])
            .send()
            .await
            .map_err(request_error)?;
        parse_response(response).await
    }

    pub async fn create_qr_login(&self) -> Result<NeteaseQrLogin> {
        let response = self
            .request_eapi("/api/login/qrcode/unikey", json!({ "type": 3 }))
            .await?;
        ensure_response_code(&response.body, 200, "create QR login")?;
        let key = response
            .body
            .get("unikey")
            .or_else(|| response.body.pointer("/data/unikey"))
            .and_then(Value::as_str)
            .filter(|key| !key.is_empty())
            .ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "NetEase QR login response did not contain a key",
                )
                .with_platform(Platform::Netease)
            })?
            .to_owned();
        let url = format!("https://music.163.com/login?codekey={key}");
        Ok(NeteaseQrLogin { key, url })
    }

    pub async fn check_qr_login(&self, key: &str) -> Result<NeteaseQrCheck> {
        let key = key.trim();
        if key.is_empty() {
            return Err(
                TuneWeaveError::invalid_request("NetEase QR login key cannot be empty")
                    .with_platform(Platform::Netease),
            );
        }
        let response = self
            .request_eapi(
                "/api/login/qrcode/client/login",
                json!({ "key": key, "type": 3 }),
            )
            .await?;
        let code = response_code(&response.body).ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase QR login check did not contain a status code",
            )
            .with_platform(Platform::Netease)
        })?;
        let state = qr_state(code);
        let session_cookie = (state == NeteaseQrState::Confirmed)
            .then(|| merge_cookie_headers(None, &response.cookies))
            .flatten();
        let message = response
            .body
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_owned);
        Ok(NeteaseQrCheck {
            code,
            state,
            message,
            session_cookie,
        })
    }

    #[must_use]
    pub fn is_authenticated(&self) -> bool {
        CookieValues::parse(self.cookie.as_deref())
            .music_u
            .is_some_and(|music_u| !music_u.is_empty())
    }
}

async fn parse_response(response: reqwest::Response) -> Result<NeteaseResponse> {
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

fn response_code(body: &Value) -> Option<i64> {
    body.get("code")
        .or_else(|| body.pointer("/data/code"))
        .and_then(Value::as_i64)
}

fn ensure_response_code(body: &Value, expected: i64, operation: &str) -> Result<()> {
    let code = response_code(body).ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase {operation} response did not contain a status code"),
        )
        .with_platform(Platform::Netease)
    })?;
    if code == expected {
        return Ok(());
    }
    Err(TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("NetEase {operation} failed with code {code}"),
    )
    .with_platform(Platform::Netease)
    .with_details(json!({ "code": code })))
}

fn qr_state(code: i64) -> NeteaseQrState {
    match code {
        800 => NeteaseQrState::Expired,
        801 => NeteaseQrState::Waiting,
        802 => NeteaseQrState::Scanned,
        803 => NeteaseQrState::Confirmed,
        _ => NeteaseQrState::Failed,
    }
}

fn merge_cookie_headers(base: Option<&str>, set_cookie: &[String]) -> Option<String> {
    let mut cookies = BTreeMap::new();
    for part in base.unwrap_or_default().split(';') {
        insert_cookie_pair(&mut cookies, part);
    }
    for header in set_cookie {
        if let Some(pair) = header.split(';').next() {
            insert_cookie_pair(&mut cookies, pair);
        }
    }
    (!cookies.is_empty()).then(|| {
        cookies
            .into_iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    })
}

fn insert_cookie_pair(cookies: &mut BTreeMap<String, String>, pair: &str) {
    let Some((name, value)) = pair.trim().split_once('=') else {
        return;
    };
    let name = name.trim();
    if name.is_empty() {
        return;
    }
    let value = value.trim();
    if value.is_empty() {
        cookies.remove(name);
    } else {
        cookies.insert(name.to_owned(), value.to_owned());
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

fn weapi_cookie_header(cookie: Option<&str>, device_id: &str) -> String {
    let mut cookies = BTreeMap::from([
        ("appver".to_owned(), "3.1.17.204416".to_owned()),
        ("deviceId".to_owned(), device_id.to_owned()),
        ("os".to_owned(), "pc".to_owned()),
        (
            "osver".to_owned(),
            "Microsoft-Windows-10-Professional-build-19045-64bit".to_owned(),
        ),
    ]);
    for part in cookie.unwrap_or_default().split(';') {
        insert_cookie_pair(&mut cookies, part);
    }
    cookies
        .into_iter()
        .map(|(name, value)| format!("{name}={value}"))
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

    #[test]
    fn distinguishes_account_and_anonymous_cookies() {
        let account = NeteaseClient::new(NeteaseConfig {
            cookie: Some("MUSIC_U=account-session".to_owned()),
            ..NeteaseConfig::default()
        })
        .expect("build account client");
        let anonymous = NeteaseClient::new(NeteaseConfig {
            cookie: Some("MUSIC_A=anonymous-session".to_owned()),
            ..NeteaseConfig::default()
        })
        .expect("build anonymous client");
        assert!(account.is_authenticated());
        assert!(!anonymous.is_authenticated());
    }

    #[test]
    fn maps_qr_status_codes_without_treating_waiting_as_an_error() {
        assert_eq!(qr_state(800), NeteaseQrState::Expired);
        assert_eq!(qr_state(801), NeteaseQrState::Waiting);
        assert_eq!(qr_state(802), NeteaseQrState::Scanned);
        assert_eq!(qr_state(803), NeteaseQrState::Confirmed);
        assert_eq!(qr_state(500), NeteaseQrState::Failed);
    }

    #[test]
    fn extracts_cookie_pairs_and_ignores_set_cookie_attributes() {
        let cookie = merge_cookie_headers(
            Some("MUSIC_A=anonymous; stale=remove-me"),
            &[
                "MUSIC_U=account-token==; Path=/; Domain=.music.163.com; HttpOnly".to_owned(),
                "__csrf=csrf-token; Max-Age=1296000".to_owned(),
                "stale=; Max-Age=0".to_owned(),
            ],
        )
        .expect("merged cookie");
        assert_eq!(
            cookie,
            "MUSIC_A=anonymous; MUSIC_U=account-token==; __csrf=csrf-token"
        );
    }

    #[test]
    fn weapi_cookie_header_keeps_session_values_and_device_defaults() {
        let cookie = weapi_cookie_header(
            Some("MUSIC_U=account-session; __csrf=csrf-token; os=android"),
            "device-id",
        );
        assert!(cookie.contains("MUSIC_U=account-session"));
        assert!(cookie.contains("__csrf=csrf-token"));
        assert!(cookie.contains("deviceId=device-id"));
        assert!(cookie.contains("os=android"));
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

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_weapi_login_status_returns_a_business_code() {
        let client = NeteaseClient::new(NeteaseConfig::default()).expect("build client");
        let response = client
            .request_weapi("/api/w/nuser/account/get", json!({}))
            .await
            .expect("live WeAPI request succeeds");
        assert!(response.body["code"].is_number());
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_qr_login_starts_in_a_non_terminal_state() {
        let client = NeteaseClient::new(NeteaseConfig::default()).expect("build client");
        let login = client.create_qr_login().await.expect("create QR login");
        assert!(!login.key.is_empty());
        assert!(login.url.contains(&login.key));
        let check = client
            .check_qr_login(&login.key)
            .await
            .expect("check QR login");
        assert!(matches!(
            check.state,
            NeteaseQrState::Waiting | NeteaseQrState::Scanned
        ));
        assert!(check.session_cookie().is_none());
    }
}
