use std::{
    collections::BTreeMap,
    fmt, process,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use md5::{Digest, Md5};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::{Client, StatusCode, header};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};

use crate::crypto::{
    build_xeapi_plaintext, decrypt_eapi_response, decrypt_xeapi_public_key, encode_form,
    encrypt_eapi, encrypt_linuxapi, encrypt_weapi, encrypt_xeapi, xeapi_sign,
};

const DEFAULT_BASE_URL: &str = "https://interface.music.163.com";
const DEFAULT_XEAPI_BASE_URL: &str = "https://interface3.music.163.com";
const DEFAULT_WEB_BASE_URL: &str = "https://music.163.com";
const IMAGE_UPLOAD_BASE_URL: &str = "https://nosup-hz1.127.net/yyimgs";
const DEFAULT_USER_AGENT: &str = "NeteaseMusic 9.0.90/5038 (iPhone; iOS 16.2; zh_CN)";
const DEFAULT_WEB_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36 Edg/124.0.0.0";
const LINUXAPI_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/60.0.3112.90 Safari/537.36";
const XEAPI_USER_AGENT: &str = "NeteaseMusic/9.1.65.240927161425(9001065);Dalvik/2.1.0 \
(Linux; U; Android 14; 23013RK75C Build/UKQ1.230804.001)";
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct NeteaseConfig {
    pub base_url: String,
    pub xeapi_base_url: String,
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
            xeapi_base_url: DEFAULT_XEAPI_BASE_URL.to_owned(),
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
    xeapi_base_url: String,
    web_base_url: String,
    web_user_agent: String,
    cookie: Option<String>,
    device_id: String,
    xeapi_state: Arc<RwLock<XeapiState>>,
}

#[derive(Clone, Default)]
struct XeapiState {
    public_key: Option<XeapiPublicKey>,
    session_id: String,
    session_key: String,
}

#[derive(Clone)]
struct XeapiPublicKey {
    public_key: [u8; 32],
    version: String,
    server_key: String,
}

#[derive(Deserialize)]
struct XeapiPublicKeyWire {
    #[serde(rename = "publicKey")]
    public_key: String,
    version: String,
    sk: String,
}

#[derive(Serialize)]
struct LinuxApiPayload<'a> {
    method: &'static str,
    url: String,
    params: &'a Map<String, Value>,
}

#[derive(Clone)]
pub struct NeteaseResponse {
    pub status: StatusCode,
    pub body: Value,
    pub cookies: Vec<String>,
}

impl fmt::Debug for NeteaseResponse {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NeteaseResponse")
            .field("status", &self.status)
            .field("code", &response_code(&self.body))
            .field("cookie_count", &self.cookies.len())
            .finish_non_exhaustive()
    }
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

#[derive(Clone, Eq, PartialEq)]
pub struct NeteaseQrCheck {
    pub code: i64,
    pub state: NeteaseQrState,
    pub message: Option<String>,
    session_cookie: Option<String>,
}

impl fmt::Debug for NeteaseQrCheck {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NeteaseQrCheck")
            .field("code", &self.code)
            .field("state", &self.state)
            .field("message", &self.message)
            .field("has_session_cookie", &self.session_cookie.is_some())
            .finish()
    }
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
            xeapi_base_url: config.xeapi_base_url.trim_end_matches('/').to_owned(),
            web_base_url: config.web_base_url.trim_end_matches('/').to_owned(),
            web_user_agent: config.web_user_agent,
            cookie: config.cookie,
            device_id,
            xeapi_state: Arc::new(RwLock::new(XeapiState::default())),
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
        let encrypted_response = encrypted_response_requested(&payload);
        payload
            .entry("e_r".to_owned())
            .or_insert(Value::Bool(false));
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
        parse_response_with_encryption(response, encrypted_response).await
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
        let encrypted_response = encrypted_response_requested(&payload);
        payload
            .entry("e_r".to_owned())
            .or_insert(Value::Bool(false));
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
        parse_response_with_encryption(response, encrypted_response).await
    }

    pub async fn request_api(&self, path: &str, payload: Value) -> Result<NeteaseResponse> {
        validate_api_path(path, "API")?;
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
        payload
            .entry("e_r".to_owned())
            .or_insert(Value::Bool(false));
        let response = self
            .http
            .post(format!("{}{}", self.base_url, path))
            .header(header::COOKIE, encode_cookie_header(&header))
            .header(
                header::CONTENT_TYPE,
                "application/x-www-form-urlencoded;charset=utf-8",
            )
            .body(encode_form(&payload, false))
            .send()
            .await
            .map_err(request_error)?;
        parse_response(response).await
    }

    pub async fn match_audio(
        &self,
        fingerprint: &str,
        duration_seconds: u32,
    ) -> Result<NeteaseResponse> {
        let duration = duration_seconds.to_string();
        let fingerprint = utf8_percent_encode(fingerprint, NON_ALPHANUMERIC);
        let endpoint = format!(
            "{}/api/music/audio/match?sessionId=0123456789abcdef&algorithmCode=shazam_v2&duration={duration}&rawdata={fingerprint}&times=1&decrypt=1",
            self.base_url
        );
        let response = self
            .http
            .get(endpoint)
            .send()
            .await
            .map_err(request_error)?;
        parse_response(response).await
    }

    pub async fn allocate_image_upload(&self, filename: &str) -> Result<NeteaseResponse> {
        self.request_weapi(
            "/api/nos/token/alloc",
            json!({
                "bucket": "yyimgs",
                "ext": "jpg",
                "filename": filename,
                "local": false,
                "nos_product": 0,
                "return_body": r#"{"code":200,"size":"$(ObjectSize)"}"#,
                "type": "other"
            }),
        )
        .await
    }

    pub async fn upload_image(
        &self,
        object_key: &str,
        token: &str,
        content_type: &str,
        data: &[u8],
    ) -> Result<NeteaseResponse> {
        let object_key = object_key.trim();
        if object_key.is_empty() || object_key.contains(['?', '#']) {
            return Err(TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase returned an invalid image upload object key",
            )
            .with_platform(Platform::Netease));
        }
        let token = token.parse::<header::HeaderValue>().map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase returned an invalid image upload token",
            )
            .with_platform(Platform::Netease)
        })?;
        let content_type = content_type.parse::<header::HeaderValue>().map_err(|_| {
            TuneWeaveError::invalid_request("image content type is not a valid HTTP header")
                .with_platform(Platform::Netease)
        })?;
        let endpoint =
            format!("{IMAGE_UPLOAD_BASE_URL}/{object_key}?offset=0&complete=true&version=1.0");
        let response = self
            .http
            .post(endpoint)
            .header("x-nos-token", token)
            .header(header::CONTENT_TYPE, content_type)
            .body(data.to_vec())
            .send()
            .await
            .map_err(request_error)?;
        parse_response(response).await
    }

    pub async fn update_account_avatar(&self, image_id: Value) -> Result<NeteaseResponse> {
        self.request_eapi("/api/user/avatar/upload/v1", json!({ "imgid": image_id }))
            .await
    }

    pub async fn request_linuxapi(&self, path: &str, payload: Value) -> Result<NeteaseResponse> {
        validate_api_path(path, "LinuxAPI")?;
        let mut payload = payload_object(payload)?;
        payload
            .entry("e_r".to_owned())
            .or_insert(Value::Bool(false));
        let wrapper = LinuxApiPayload {
            method: "POST",
            url: format!("{}{}", self.web_base_url, path),
            params: &payload,
        };
        let plaintext = serde_json::to_string(&wrapper).map_err(json_error)?;
        let response = self
            .http
            .post(format!("{}/api/linux/forward", self.web_base_url))
            .header(header::USER_AGENT, LINUXAPI_USER_AGENT)
            .header(
                header::COOKIE,
                weapi_cookie_header(self.cookie.as_deref(), &self.device_id),
            )
            .form(&[("eparams", encrypt_linuxapi(&plaintext))])
            .send()
            .await
            .map_err(request_error)?;
        parse_response(response).await
    }

    pub async fn request_xeapi(&self, path: &str, payload: Value) -> Result<NeteaseResponse> {
        validate_api_path(path, "XEAPI")?;
        let payload = payload_object(payload)?;
        let public_key = self.xeapi_public_key().await?;
        let session = {
            let state = self.xeapi_state.read().map_err(|_| xeapi_state_error())?;
            (!state.session_key.is_empty())
                .then(|| (state.session_id.clone(), state.session_key.clone()))
        };
        let plaintext = build_xeapi_plaintext(path, &payload);
        let encrypted = encrypt_xeapi(
            &plaintext,
            &public_key.public_key,
            &public_key.version,
            &public_key.server_key,
            session
                .as_ref()
                .map(|(id, key)| (id.as_str(), key.as_str())),
        )
        .map_err(|error| xeapi_crypto_error("encrypt request", error))?;
        let build_version = unix_time_seconds().to_string();
        let cookie = CookieValues::parse(self.cookie.as_deref());
        let mut request = self
            .http
            .post(format!(
                "{}/xeapi/{}",
                self.xeapi_base_url,
                path.trim_start_matches("/api/")
            ))
            .header(header::USER_AGENT, XEAPI_USER_AGENT)
            .header("x-client-enc-state", "ENCRYPTED")
            .header("x-aeapi", "true")
            .header("x-deviceid", &self.device_id)
            .header("x-os", "android")
            .header("x-osver", "16")
            .header("x-appver", "9.1.65")
            .header("x-sdeviceid", &self.device_id)
            .header("x-buildver", &build_version)
            .header(
                header::COOKIE,
                xeapi_cookie_header(self.cookie.as_deref(), &self.device_id, &build_version),
            );
        if let Some(music_u) = cookie.music_u {
            request = request.header("x-music-u", music_u);
        }
        let response = request
            .form(&[("B", encrypted.b), ("S", encrypted.s), ("R", encrypted.r)])
            .send()
            .await
            .map_err(request_error)?;
        let next_session = response
            .headers()
            .get("x-encr-ssid")
            .and_then(|value| value.to_str().ok())
            .zip(
                response
                    .headers()
                    .get("x-encr-sskey")
                    .and_then(|value| value.to_str().ok()),
            )
            .map(|(id, key)| (id.to_owned(), key.to_owned()));
        if let Some((session_id, session_key)) = next_session
            && session_key.len() == 16
        {
            let mut state = self.xeapi_state.write().map_err(|_| xeapi_state_error())?;
            state.session_id = session_id;
            state.session_key = session_key;
        }
        parse_response_with_encryption(response, true).await
    }

    async fn xeapi_public_key(&self) -> Result<XeapiPublicKey> {
        if let Some(key) = self
            .xeapi_state
            .read()
            .map_err(|_| xeapi_state_error())?
            .public_key
            .clone()
        {
            return Ok(key);
        }

        let registered = self.register_xeapi_public_key().await?;
        let mut state = self.xeapi_state.write().map_err(|_| xeapi_state_error())?;
        Ok(state.public_key.get_or_insert(registered).clone())
    }

    async fn register_xeapi_public_key(&self) -> Result<XeapiPublicKey> {
        let nonce = (0..16)
            .map(|_| char::from(b'0' + rand::random_range(0_u8..10)))
            .collect::<String>();
        let timestamp = unix_time_millis().to_string();
        let signature = xeapi_sign(&timestamp, &nonce);
        let form = [
            ("appVersion", "9.1.65".to_owned()),
            ("currentKeyVersion", String::new()),
            ("deviceId", self.device_id.clone()),
            ("nonce", nonce.clone()),
            ("os", "android".to_owned()),
            ("requestType", "active".to_owned()),
            ("signature", signature),
            ("t1", String::new()),
            ("t2", String::new()),
            ("timestamp", timestamp),
            ("uid", String::new()),
        ];
        let response = self
            .http
            .post(format!(
                "{}/api/gorilla/anti/crawler/security/key/get",
                self.base_url
            ))
            .header(header::USER_AGENT, XEAPI_USER_AGENT)
            .header(header::COOKIE, format!("deviceId={}", self.device_id))
            .form(&form)
            .send()
            .await
            .map_err(request_error)?;
        let response = parse_response(response).await?;
        if response_code(&response.body) != Some(200) {
            return Err(TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase XEAPI public key registration failed",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "response": response.body })));
        }
        let data = response.body.get("data").ok_or_else(|| {
            xeapi_registration_error("response did not contain data", &response.body)
        })?;
        let response_timestamp = scalar_string(data.get("timestamp")).ok_or_else(|| {
            xeapi_registration_error("response did not contain a timestamp", &response.body)
        })?;
        let response_signature =
            data.get("signature")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    xeapi_registration_error("response did not contain a signature", &response.body)
                })?;
        if xeapi_sign(&response_timestamp, &nonce) != response_signature {
            return Err(xeapi_registration_error(
                "response signature did not match",
                &response.body,
            ));
        }
        let encrypted = data
            .get("encryptedData")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                xeapi_registration_error(
                    "response did not contain encrypted key data",
                    &response.body,
                )
            })?;
        let plaintext = decrypt_xeapi_public_key(encrypted)
            .map_err(|error| xeapi_crypto_error("decrypt public key", error))?;
        let wire: XeapiPublicKeyWire = serde_json::from_str(&plaintext).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase XEAPI public key is invalid: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
        if wire.sk.is_empty() {
            return Err(xeapi_registration_error(
                "decrypted public key did not contain sk",
                &response.body,
            ));
        }
        let public_key: [u8; 32] = BASE64
            .decode(wire.public_key)
            .map_err(|_| {
                xeapi_registration_error("public key is not valid base64", &response.body)
            })?
            .try_into()
            .map_err(|_| {
                xeapi_registration_error("public key must contain 32 bytes", &response.body)
            })?;
        Ok(XeapiPublicKey {
            public_key,
            version: wire.version,
            server_key: wire.sk,
        })
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
        has_authenticated_cookie(self.cookie.as_deref())
    }

    pub(crate) fn configured_cookie(&self) -> Option<&str> {
        self.cookie.as_deref()
    }

    pub(crate) fn with_cookie(&self, cookie: String) -> Self {
        let mut client = self.clone();
        client.cookie = Some(cookie);
        client
    }

    pub(crate) fn without_cookie(&self) -> Self {
        let mut client = self.clone();
        client.cookie = None;
        client
    }
}

async fn parse_response(response: reqwest::Response) -> Result<NeteaseResponse> {
    parse_response_with_encryption(response, false).await
}

async fn parse_response_with_encryption(
    response: reqwest::Response,
    encrypted: bool,
) -> Result<NeteaseResponse> {
    let status = response.status();
    let cookies = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok().map(str::to_owned))
        .collect();
    let mut bytes = response.bytes().await.map_err(request_error)?.to_vec();
    if !status.is_success() {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned HTTP {status}"),
        )
        .with_platform(Platform::Netease)
        .retryable(status.is_server_error())
        .with_details(json!({ "status": status.as_u16() })));
    }

    if encrypted {
        bytes = decrypt_eapi_response(&bytes).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("failed to decrypt NetEase response: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
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

pub(crate) fn response_code(body: &Value) -> Option<i64> {
    body.get("code")
        .or_else(|| body.pointer("/data/code"))
        .and_then(Value::as_i64)
}

pub(crate) fn ensure_response_code(body: &Value, expected: i64, operation: &str) -> Result<()> {
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

pub(crate) fn merge_cookie_headers(base: Option<&str>, set_cookie: &[String]) -> Option<String> {
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

pub(crate) fn has_authenticated_cookie(cookie: Option<&str>) -> bool {
    CookieValues::parse(cookie)
        .music_u
        .is_some_and(|music_u| !music_u.is_empty())
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
        TuneWeaveError::invalid_request("NetEase API payload must be a JSON object")
            .with_platform(Platform::Netease)
    })
}

fn validate_api_path(path: &str, protocol: &str) -> Result<()> {
    if path.starts_with("/api/") {
        return Ok(());
    }
    Err(
        TuneWeaveError::invalid_request(format!("NetEase {protocol} paths must start with /api/"))
            .with_platform(Platform::Netease),
    )
}

fn encrypted_response_requested(payload: &Map<String, Value>) -> bool {
    match payload.get("e_r") {
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_i64().is_some_and(|value| value != 0),
        Some(Value::String(value)) => value.eq_ignore_ascii_case("true") || value == "1",
        _ => false,
    }
}

fn scalar_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn xeapi_registration_error(message: &str, body: &Value) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("NetEase XEAPI public key {message}"),
    )
    .with_platform(Platform::Netease)
    .with_details(json!({ "response": body }))
}

fn xeapi_crypto_error(operation: &str, message: &str) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("failed to {operation} for NetEase XEAPI: {message}"),
    )
    .with_platform(Platform::Netease)
}

fn xeapi_state_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        "NetEase XEAPI state lock is poisoned",
    )
    .with_platform(Platform::Netease)
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

fn xeapi_cookie_header(cookie: Option<&str>, device_id: &str, build_version: &str) -> String {
    let mut cookies = BTreeMap::from([
        ("appver".to_owned(), "9.1.65".to_owned()),
        ("buildver".to_owned(), build_version.to_owned()),
        ("deviceId".to_owned(), device_id.to_owned()),
        ("os".to_owned(), "android".to_owned()),
        ("osver".to_owned(), "16".to_owned()),
        ("sDeviceId".to_owned(), device_id.to_owned()),
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
    fn debug_output_redacts_response_and_qr_cookies() {
        let response = NeteaseResponse {
            status: StatusCode::OK,
            body: json!({ "code": 200, "cookie": "body-secret" }),
            cookies: vec!["MUSIC_U=header-secret".to_owned()],
        };
        let check = NeteaseQrCheck {
            code: 803,
            state: NeteaseQrState::Confirmed,
            message: None,
            session_cookie: Some("MUSIC_U=qr-secret".to_owned()),
        };
        let output = format!("{response:?} {check:?}");
        assert!(!output.contains("body-secret"));
        assert!(!output.contains("header-secret"));
        assert!(!output.contains("qr-secret"));
        assert!(output.contains("has_session_cookie: true"));
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

    #[test]
    fn xeapi_uses_fixed_netease_domains_and_android_cookie_defaults() {
        let config = NeteaseConfig::default();
        assert_eq!(config.base_url, "https://interface.music.163.com");
        assert_eq!(config.xeapi_base_url, "https://interface3.music.163.com");
        assert_eq!(config.web_base_url, "https://music.163.com");

        let cookie = xeapi_cookie_header(
            Some("MUSIC_U=account-session; os=pc"),
            "device-id",
            "1784194692",
        );
        assert!(cookie.contains("MUSIC_U=account-session"));
        assert!(cookie.contains("deviceId=device-id"));
        assert!(cookie.contains("sDeviceId=device-id"));
        assert!(cookie.contains("buildver=1784194692"));
        assert!(cookie.contains("os=pc"));
    }

    #[test]
    fn encrypted_response_flag_accepts_reference_boolean_forms() {
        assert!(encrypted_response_requested(
            json!({ "e_r": true }).as_object().expect("object")
        ));
        assert!(encrypted_response_requested(
            json!({ "e_r": "true" }).as_object().expect("object")
        ));
        assert!(encrypted_response_requested(
            json!({ "e_r": 1 }).as_object().expect("object")
        ));
        assert!(!encrypted_response_requested(
            json!({ "e_r": false }).as_object().expect("object")
        ));
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
    async fn live_eapi_encrypted_response_is_decrypted() {
        let client = NeteaseClient::new(NeteaseConfig::default()).expect("build client");
        let response = client
            .request_eapi(
                "/api/search/get",
                json!({
                    "s": "TuneWeave",
                    "type": 1,
                    "limit": 1,
                    "offset": 0,
                    "e_r": true
                }),
            )
            .await
            .expect("live encrypted response succeeds");
        assert_eq!(response_code(&response.body), Some(200));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_plain_api_search_returns_a_business_response() {
        let client = NeteaseClient::new(NeteaseConfig::default()).expect("build client");
        let response = client
            .request_api(
                "/api/search/get",
                json!({ "s": "TuneWeave", "type": 1, "limit": 1, "offset": 0 }),
            )
            .await
            .expect("live plaintext API request succeeds");
        assert!(response_code(&response.body).is_some());
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_linuxapi_search_returns_songs() {
        let client = NeteaseClient::new(NeteaseConfig::default()).expect("build client");
        let response = client
            .request_linuxapi(
                "/api/search/get",
                json!({ "s": "TuneWeave", "type": 1, "limit": 1, "offset": 0 }),
            )
            .await
            .expect("live LinuxAPI search succeeds");
        assert_eq!(response_code(&response.body), Some(200));
        assert!(
            response.body["result"]["songs"]
                .as_array()
                .is_some_and(|songs| !songs.is_empty())
        );
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_xeapi_registers_a_key_and_returns_a_business_response() {
        let client = NeteaseClient::new(NeteaseConfig::default()).expect("build client");
        let public_key = client
            .xeapi_public_key()
            .await
            .expect("register XEAPI public key");
        assert!(!public_key.version.is_empty());
        assert!(!public_key.server_key.is_empty());

        let response = client
            .request_xeapi(
                "/api/search/get",
                json!({ "s": "TuneWeave", "type": 1, "limit": 1, "offset": 0 }),
            )
            .await
            .expect("live XEAPI request succeeds");
        assert!(response_code(&response.body).is_some());
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_weapi_login_status_returns_a_business_code() {
        let client = NeteaseClient::new(NeteaseConfig::default()).expect("build client");
        let status = client
            .session_status()
            .await
            .expect("live session status succeeds");
        assert!(!status.authenticated);
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
