use std::{
    collections::BTreeMap,
    fmt,
    net::Ipv4Addr,
    process,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use md5::{Digest, Md5};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::{Client, ClientBuilder, Proxy, RequestBuilder, StatusCode, header, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tuneweave_core::{
    AccountCredentialStore, AntiCheatTokenVersion, ErrorCode, Platform, Result, TuneWeaveError,
};
use url::Url;

use crate::crypto::{
    build_xeapi_plaintext, decrypt_eapi_response, decrypt_xeapi_public_key, encode_form,
    encrypt_eapi, encrypt_linuxapi, encrypt_weapi, encrypt_xeapi, xeapi_sign,
};

const DEFAULT_BASE_URL: &str = "https://interface.music.163.com";
const DEFAULT_XEAPI_BASE_URL: &str = "https://interface3.music.163.com";
const DEFAULT_WEB_BASE_URL: &str = "https://music.163.com";
const DEFAULT_ANTI_CHEAT_V2_URL: &str = "https://ac.dun.163.com/v2/config/js?pn=YD00000558929251";
const DEFAULT_ANTI_CHEAT_V3_URL: &str = "https://ac.dun.163yun.com/v3/b?pn=YD00000558929251";
const IMAGE_UPLOAD_BASE_URL: &str = "https://nosup-hz1.127.net/yyimgs";
const CLOUD_UPLOAD_LBS_URL: &str = "https://wanproxy.127.net/lbs";
const MAX_VOICE_LYRIC_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_USER_AGENT: &str = "NeteaseMusic 9.0.90/5038 (iPhone; iOS 16.2; zh_CN)";
const DEFAULT_WEB_USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) \
AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36 Edg/124.0.0.0";
const LINUXAPI_USER_AGENT: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/60.0.3112.90 Safari/537.36";
const XEAPI_USER_AGENT: &str = "NeteaseMusic/9.1.65.240927161425(9001065);Dalvik/2.1.0 \
(Linux; U; Android 14; 23013RK75C Build/UKQ1.230804.001)";
const JAVASCRIPT_ENCODE_URI_COMPONENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'!')
    .remove(b'\'')
    .remove(b'(')
    .remove(b')')
    .remove(b'*')
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');
static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct NeteaseConfig {
    pub base_url: String,
    pub xeapi_base_url: String,
    pub web_base_url: String,
    pub anti_cheat_v2_url: String,
    /// Backward-compatible v3 registration URL override.
    pub anti_cheat_url: String,
    pub cookie: Option<String>,
    pub timeout: Duration,
    pub user_agent: String,
    pub web_user_agent: String,
    /// A server-controlled forward proxy. It is never accepted from an API request.
    pub proxy_url: Option<String>,
    /// A fixed server-controlled IPv4 identity sent as X-Real-IP and X-Forwarded-For.
    pub real_ip: Option<Ipv4Addr>,
    /// Select one Chinese IPv4 identity when the client starts and reuse it for all requests.
    pub random_cn_ip: bool,
    pub credential_store: Option<Arc<dyn AccountCredentialStore>>,
}

impl Default for NeteaseConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            xeapi_base_url: DEFAULT_XEAPI_BASE_URL.to_owned(),
            web_base_url: DEFAULT_WEB_BASE_URL.to_owned(),
            anti_cheat_v2_url: DEFAULT_ANTI_CHEAT_V2_URL.to_owned(),
            anti_cheat_url: DEFAULT_ANTI_CHEAT_V3_URL.to_owned(),
            cookie: None,
            timeout: Duration::from_secs(15),
            user_agent: DEFAULT_USER_AGENT.to_owned(),
            web_user_agent: DEFAULT_WEB_USER_AGENT.to_owned(),
            proxy_url: None,
            real_ip: None,
            random_cn_ip: false,
            credential_store: None,
        }
    }
}

#[derive(Clone)]
pub struct NeteaseClient {
    http: Client,
    asset_http: Client,
    base_url: String,
    xeapi_base_url: String,
    web_base_url: String,
    anti_cheat_v2_url: String,
    anti_cheat_v3_url: String,
    web_user_agent: String,
    cookie: Option<String>,
    device_id: String,
    web_client_id: String,
    real_ip: Option<Ipv4Addr>,
    xeapi_state: Arc<RwLock<XeapiState>>,
    anti_cheat_v2_token: Arc<RwLock<Option<String>>>,
    anti_cheat_v3_token: Arc<RwLock<Option<String>>>,
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

#[derive(Clone)]
pub struct NeteaseAnonymousRegistration {
    pub device_id: String,
    pub body: Value,
    session_cookie: String,
}

impl fmt::Debug for NeteaseAnonymousRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NeteaseAnonymousRegistration")
            .field("device_id", &self.device_id)
            .field("code", &response_code(&self.body))
            .field("has_session_cookie", &true)
            .finish()
    }
}

impl NeteaseAnonymousRegistration {
    #[must_use]
    pub fn session_cookie(&self) -> &str {
        &self.session_cookie
    }

    #[must_use]
    pub fn into_session_cookie(self) -> String {
        self.session_cookie
    }
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
        if config.random_cn_ip && config.real_ip.is_some() {
            return Err(TuneWeaveError::invalid_request(
                "NetEase fixed and random network identities are mutually exclusive",
            )
            .with_platform(Platform::Netease));
        }
        let http = configure_proxy(
            Client::builder()
                .timeout(config.timeout)
                .connect_timeout(Duration::from_secs(8))
                .user_agent(config.user_agent),
            config.proxy_url.as_deref(),
        )?
        .build()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                format!("failed to build NetEase HTTP client: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
        let asset_http = configure_proxy(
            Client::builder()
                .timeout(config.timeout)
                .connect_timeout(Duration::from_secs(8))
                .user_agent(&config.web_user_agent)
                .redirect(Policy::none()),
            config.proxy_url.as_deref(),
        )?
        .build()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                format!("failed to build NetEase asset HTTP client: {error}"),
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
        let web_client_id = format!(
            "{}.{}.01.0",
            (0..6)
                .map(|_| char::from(b'a' + rand::random_range(0_u8..26)))
                .collect::<String>(),
            unix_time_millis()
        );

        let real_ip = if config.random_cn_ip {
            Some(random_chinese_ipv4())
        } else {
            config.real_ip
        };

        Ok(Self {
            http,
            asset_http,
            base_url: config.base_url.trim_end_matches('/').to_owned(),
            xeapi_base_url: config.xeapi_base_url.trim_end_matches('/').to_owned(),
            web_base_url: config.web_base_url.trim_end_matches('/').to_owned(),
            anti_cheat_v2_url: config.anti_cheat_v2_url,
            anti_cheat_v3_url: config.anti_cheat_url,
            web_user_agent: config.web_user_agent,
            cookie: config.cookie,
            device_id,
            web_client_id,
            real_ip,
            xeapi_state: Arc::new(RwLock::new(XeapiState::default())),
            anti_cheat_v2_token: Arc::new(RwLock::new(None)),
            anti_cheat_v3_token: Arc::new(RwLock::new(None)),
        })
    }

    pub async fn request_eapi(&self, path: &str, payload: Value) -> Result<NeteaseResponse> {
        self.request_eapi_inner(path, payload, None).await
    }

    pub async fn request_eapi_with_check_token_v2(
        &self,
        path: &str,
        payload: Value,
    ) -> Result<NeteaseResponse> {
        let (token, _) = self
            .anti_cheat_token(AntiCheatTokenVersion::V2, false)
            .await?;
        self.request_eapi_inner(path, payload, Some(&token)).await
    }

    async fn request_eapi_inner(
        &self,
        path: &str,
        payload: Value,
        anti_cheat_token: Option<&str>,
    ) -> Result<NeteaseResponse> {
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
        let mut request = self.apply_network_identity(
            self.http
                .post(endpoint)
                .header(header::COOKIE, encode_cookie_header(&header)),
        );
        if let Some(token) = anti_cheat_token {
            request = request.header("X-antiCheatToken", token);
        }
        let response = request
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
        let request = self.apply_network_identity(
            self.http
                .post(endpoint)
                .header(header::REFERER, &self.web_base_url)
                .header(header::USER_AGENT, &self.web_user_agent)
                .header(
                    header::COOKIE,
                    weapi_cookie_header(
                        self.cookie.as_deref(),
                        &self.device_id,
                        &self.web_client_id,
                        path,
                    ),
                )
                .form(&[
                    ("params", encrypted.params),
                    ("encSecKey", encrypted.enc_sec_key),
                ]),
        );
        let response = request.send().await.map_err(request_error)?;
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
        let request = self.apply_network_identity(
            self.http
                .post(format!("{}{}", self.base_url, path))
                .header(header::COOKIE, encode_cookie_header(&header))
                .header(
                    header::CONTENT_TYPE,
                    "application/x-www-form-urlencoded;charset=utf-8",
                )
                .body(encode_form(&payload, false)),
        );
        let response = request.send().await.map_err(request_error)?;
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
            .apply_network_identity(self.http.get(endpoint))
            .send()
            .await
            .map_err(request_error)?;
        parse_response(response).await
    }

    pub async fn fetch_voice_lyric_document(&self, url: &str) -> Result<Value> {
        let url = validate_voice_lyric_url(url)?;
        let mut response = self
            .asset_http
            .get(url)
            .send()
            .await
            .map_err(request_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase voice lyric asset returned HTTP {status}"),
            )
            .with_platform(Platform::Netease)
            .retryable(status.is_server_error())
            .with_details(json!({ "status": status.as_u16() })));
        }
        if response.content_length().is_some_and(|length| {
            length > u64::try_from(MAX_VOICE_LYRIC_DOCUMENT_BYTES).unwrap_or(u64::MAX)
        }) {
            return Err(voice_lyric_document_too_large());
        }

        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(request_error)? {
            let next_length = bytes.len().checked_add(chunk.len()).ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "NetEase voice lyric asset size overflowed",
                )
                .with_platform(Platform::Netease)
            })?;
            if next_length > MAX_VOICE_LYRIC_DOCUMENT_BYTES {
                return Err(voice_lyric_document_too_large());
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase voice lyric asset returned invalid JSON: {error}"),
            )
            .with_platform(Platform::Netease)
        })
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

    pub async fn cloud_upload_servers(&self, bucket: &str) -> Result<NeteaseResponse> {
        let mut url = Url::parse(CLOUD_UPLOAD_LBS_URL).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                format!("invalid NetEase cloud LBS URL: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
        url.query_pairs_mut()
            .append_pair("version", "1.0")
            .append_pair("bucketname", bucket);
        let response = self.http.get(url).send().await.map_err(request_error)?;
        parse_response(response).await
    }

    pub async fn upload_cloud_audio(
        &self,
        upload_url: &str,
        token: &str,
        md5: &str,
        content_type: &str,
        data: &[u8],
    ) -> Result<NeteaseResponse> {
        validate_cloud_upload_url(upload_url)?;
        let token = token.parse::<header::HeaderValue>().map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase returned an invalid cloud upload token",
            )
            .with_platform(Platform::Netease)
        })?;
        let md5 = md5.parse::<header::HeaderValue>().map_err(|_| {
            TuneWeaveError::invalid_request("cloud audio MD5 is not a valid HTTP header")
                .with_platform(Platform::Netease)
        })?;
        let content_type = content_type.parse::<header::HeaderValue>().map_err(|_| {
            TuneWeaveError::invalid_request("cloud audio content type is not a valid HTTP header")
                .with_platform(Platform::Netease)
        })?;
        let response = self
            .http
            .post(upload_url)
            .timeout(Duration::from_secs(300))
            .header("x-nos-token", token)
            .header("Content-MD5", md5)
            .header(header::CONTENT_TYPE, content_type)
            .header(header::CONTENT_LENGTH, data.len())
            .body(data.to_vec())
            .send()
            .await
            .map_err(request_error)?;
        parse_response(response).await
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
        let request = self.apply_network_identity(
            self.http
                .post(format!("{}/api/linux/forward", self.web_base_url))
                .header(header::USER_AGENT, LINUXAPI_USER_AGENT)
                .header(
                    header::COOKIE,
                    weapi_cookie_header(
                        self.cookie.as_deref(),
                        &self.device_id,
                        &self.web_client_id,
                        path,
                    ),
                )
                .form(&[("eparams", encrypt_linuxapi(&plaintext))]),
        );
        let response = request.send().await.map_err(request_error)?;
        parse_response(response).await
    }

    pub async fn anti_cheat_token(
        &self,
        version: AntiCheatTokenVersion,
        refresh: bool,
    ) -> Result<(String, bool)> {
        let (url, cache) = match version {
            AntiCheatTokenVersion::V2 => (&self.anti_cheat_v2_url, &self.anti_cheat_v2_token),
            AntiCheatTokenVersion::V3 => (&self.anti_cheat_v3_url, &self.anti_cheat_v3_token),
        };
        if !refresh && let Some(token) = cache.read().map_err(|_| anti_cheat_state_error())?.clone()
        {
            return Ok((token, false));
        }

        let response = self.http.get(url).send().await.map_err(request_error)?;
        let status = response.status();
        let body = response.text().await.map_err(request_error)?;
        if !status.is_success() {
            return Err(TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase anti-cheat token registration failed",
            )
            .with_platform(Platform::Netease)
            .retryable(status.is_server_error())
            .with_details(json!({ "http_status": status.as_u16() })));
        }
        let token = match version {
            AntiCheatTokenVersion::V2 => parse_anti_cheat_token_v2(&body)?,
            AntiCheatTokenVersion::V3 => parse_anti_cheat_token_v3(&body)?,
        };
        token.parse::<header::HeaderValue>().map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase anti-cheat service returned a token that is not safe for an HTTP header",
            )
            .with_platform(Platform::Netease)
        })?;
        *cache.write().map_err(|_| anti_cheat_state_error())? = Some(token.clone());
        Ok((token, true))
    }

    pub async fn request_xeapi(&self, path: &str, payload: Value) -> Result<NeteaseResponse> {
        self.request_xeapi_inner(path, payload, None).await
    }

    pub async fn register_anonymous(&self) -> Result<NeteaseAnonymousRegistration> {
        let device_id = generate_anonymous_device_id();
        let client = self.without_cookie().with_device_id(device_id.clone());
        let response = client
            .request_xeapi(
                "/api/register/anonimous",
                json!({ "username": anonymous_username(&device_id) }),
            )
            .await?;
        ensure_response_code(&response.body, 200, "anonymous registration")?;
        let session_cookie = merge_cookie_headers(None, &response.cookies)
            .filter(|cookie| has_anonymous_cookie(Some(cookie)))
            .ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "NetEase anonymous registration did not return MUSIC_A",
                )
                .with_platform(Platform::Netease)
            })?;
        Ok(NeteaseAnonymousRegistration {
            device_id,
            body: response.body,
            session_cookie,
        })
    }

    pub async fn request_xeapi_with_check_token(
        &self,
        path: &str,
        payload: Value,
    ) -> Result<NeteaseResponse> {
        let (token, _) = self
            .anti_cheat_token(AntiCheatTokenVersion::V3, false)
            .await?;
        self.request_xeapi_inner(path, payload, Some(&token)).await
    }

    async fn request_xeapi_inner(
        &self,
        path: &str,
        payload: Value,
        anti_cheat_token: Option<&str>,
    ) -> Result<NeteaseResponse> {
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
        let mut request = self.apply_network_identity(
            self.http
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
                ),
        );
        if let Some(music_u) = cookie.music_u {
            request = request.header("x-music-u", music_u);
        }
        if let Some(token) = anti_cheat_token {
            request = request.header("X-antiCheatToken", token);
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
        let request = self.apply_network_identity(
            self.http
                .post(format!(
                    "{}/api/gorilla/anti/crawler/security/key/get",
                    self.base_url
                ))
                .header(header::USER_AGENT, XEAPI_USER_AGENT)
                .header(header::COOKIE, format!("deviceId={}", self.device_id))
                .form(&form),
        );
        let response = request.send().await.map_err(request_error)?;
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
        let key = qr_login_key(&response.body).ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase QR login response did not contain a key",
            )
            .with_platform(Platform::Netease)
        })?;
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

    fn apply_network_identity(&self, request: RequestBuilder) -> RequestBuilder {
        let Some(ip) = self.real_ip else {
            return request;
        };
        let ip = ip.to_string();
        request
            .header("X-Real-IP", &ip)
            .header("X-Forwarded-For", ip)
    }

    pub(crate) fn configured_cookie(&self) -> Option<&str> {
        self.cookie.as_deref()
    }

    pub(crate) fn with_cookie(&self, cookie: String) -> Self {
        let mut client = self.clone();
        client.cookie = Some(cookie);
        client
    }

    pub(crate) fn with_device_id(&self, device_id: String) -> Self {
        let mut client = self.clone();
        client.device_id = device_id;
        client
    }

    pub(crate) fn device_id(&self) -> &str {
        &self.device_id
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
    [body.get("code"), body.pointer("/data/code")]
        .into_iter()
        .flatten()
        .find_map(|value| {
            value
                .as_i64()
                .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
        })
}

fn qr_login_key(body: &Value) -> Option<String> {
    [body.get("unikey"), body.pointer("/data/unikey")]
        .into_iter()
        .flatten()
        .find_map(|value| {
            value
                .as_str()
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .map(str::to_owned)
        })
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

pub(crate) fn has_anonymous_cookie(cookie: Option<&str>) -> bool {
    CookieValues::parse(cookie)
        .music_a
        .is_some_and(|music_a| !music_a.is_empty())
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

fn validate_cloud_upload_url(upload_url: &str) -> Result<()> {
    let url = Url::parse(upload_url).map_err(|_| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase returned an invalid cloud upload URL",
        )
        .with_platform(Platform::Netease)
    })?;
    let host = url.host_str().unwrap_or_default();
    let query_pairs = url.query_pairs().collect::<Vec<_>>();
    let query = query_pairs.iter().cloned().collect::<BTreeMap<_, _>>();
    let valid_query = query_pairs.len() == 3
        && query.len() == 3
        && query.get("offset").is_some_and(|value| value == "0")
        && query.get("complete").is_some_and(|value| value == "true")
        && query.get("version").is_some_and(|value| value == "1.0");
    if !matches!(url.scheme(), "http" | "https")
        || !host.ends_with(".127.net")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
        || !valid_query
    {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase cloud upload URL is outside the allowed NOS destination",
        )
        .with_platform(Platform::Netease));
    }
    Ok(())
}

fn validate_voice_lyric_url(lyric_url: &str) -> Result<Url> {
    let url = Url::parse(lyric_url).map_err(|_| voice_lyric_url_error())?;
    let host = url.host_str().unwrap_or_default();
    let valid_host = ["music.126.net", "music.163.com"]
        .into_iter()
        .any(|domain| host == domain || host.ends_with(&format!(".{domain}")));
    if !matches!(url.scheme(), "http" | "https")
        || !valid_host
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return Err(voice_lyric_url_error());
    }
    Ok(url)
}

fn voice_lyric_url_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        "NetEase voice lyric URL is outside the allowed asset destinations",
    )
    .with_platform(Platform::Netease)
}

fn voice_lyric_document_too_large() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        "NetEase voice lyric asset exceeds the 16 MiB limit",
    )
    .with_platform(Platform::Netease)
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

fn parse_anti_cheat_token_v2(body: &str) -> Result<String> {
    let response =
        serde_json::from_str::<Value>(body).map_err(|_| anti_cheat_registration_error())?;
    let code = response
        .get("code")
        .and_then(|value| value.as_i64().or_else(|| value.as_str()?.parse().ok()));
    let token = response
        .pointer("/result/conf")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|token| !token.is_empty());
    match (code, token) {
        (Some(200), Some(token)) => Ok(token.to_owned()),
        _ => Err(anti_cheat_registration_error()),
    }
}

fn parse_anti_cheat_token_v3(body: &str) -> Result<String> {
    let body = body.trim();
    if !body.starts_with("null(") || !body.ends_with(')') {
        return Err(anti_cheat_registration_error());
    }
    let start = body.find('[').ok_or_else(anti_cheat_registration_error)?;
    let end = body.rfind(']').ok_or_else(anti_cheat_registration_error)?;
    if end <= start {
        return Err(anti_cheat_registration_error());
    }
    let response = serde_json::from_str::<Value>(&body[start..=end])
        .map_err(|_| anti_cheat_registration_error())?;
    let values = response
        .as_array()
        .filter(|values| values.len() >= 3)
        .ok_or_else(anti_cheat_registration_error)?;
    let code = values[0]
        .as_i64()
        .or_else(|| values[0].as_str().and_then(|value| value.parse().ok()));
    let token = values[2]
        .as_str()
        .map(str::trim)
        .filter(|token| !token.is_empty());
    match (code, token) {
        (Some(200), Some(token)) => Ok(token.to_owned()),
        _ => Err(anti_cheat_registration_error()),
    }
}

fn anti_cheat_registration_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        "NetEase anti-cheat token response is invalid",
    )
    .with_platform(Platform::Netease)
}

fn anti_cheat_state_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        "NetEase anti-cheat token state lock is poisoned",
    )
    .with_platform(Platform::Netease)
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
                utf8_percent_encode(name, JAVASCRIPT_ENCODE_URI_COMPONENT),
                utf8_percent_encode(value, JAVASCRIPT_ENCODE_URI_COMPONENT)
            )
        })
        .collect::<Vec<_>>()
        .join("; ")
}

fn weapi_cookie_header(
    cookie: Option<&str>,
    device_id: &str,
    web_client_id: &str,
    path: &str,
) -> String {
    let mut cookies = BTreeMap::new();
    for part in cookie.unwrap_or_default().split(';') {
        insert_cookie_pair(&mut cookies, part);
    }
    cookies.insert("__remember_me".to_owned(), "true".to_owned());
    cookies.insert("ntes_kaola_ad".to_owned(), "1".to_owned());
    let nuid = cookies
        .entry("_ntes_nuid".to_owned())
        .or_insert_with(|| hex::encode(rand::random::<[u8; 32]>()))
        .clone();
    cookies
        .entry("_ntes_nnid".to_owned())
        .or_insert_with(|| format!("{nuid},{}", unix_time_millis()));
    cookies
        .entry("WNMCID".to_owned())
        .or_insert_with(|| web_client_id.to_owned());
    cookies
        .entry("WEVNSM".to_owned())
        .or_insert_with(|| "1.0.0".to_owned());
    cookies
        .entry("appver".to_owned())
        .or_insert_with(|| "3.1.17.204416".to_owned());
    cookies
        .entry("channel".to_owned())
        .or_insert_with(|| "netease".to_owned());
    cookies
        .entry("deviceId".to_owned())
        .or_insert_with(|| device_id.to_owned());
    cookies
        .entry("os".to_owned())
        .or_insert_with(|| "pc".to_owned());
    cookies
        .entry("osver".to_owned())
        .or_insert_with(|| "Microsoft-Windows-10-Professional-build-19045-64bit".to_owned());
    if !path.contains("login") {
        cookies.insert("NMTID".to_owned(), hex::encode(rand::random::<[u8; 16]>()));
    }
    cookies
        .into_iter()
        .map(|(name, value)| {
            format!(
                "{}={}",
                utf8_percent_encode(&name, JAVASCRIPT_ENCODE_URI_COMPONENT),
                utf8_percent_encode(&value, JAVASCRIPT_ENCODE_URI_COMPONENT)
            )
        })
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

fn configure_proxy(builder: ClientBuilder, proxy_url: Option<&str>) -> Result<ClientBuilder> {
    let Some(proxy_url) = proxy_url.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(builder);
    };
    let url = Url::parse(proxy_url).map_err(|_| proxy_configuration_error())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
    {
        return Err(proxy_configuration_error());
    }
    let proxy = Proxy::all(proxy_url).map_err(|_| proxy_configuration_error())?;
    Ok(builder.proxy(proxy))
}

fn proxy_configuration_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InvalidRequest,
        "NetEase server proxy must be an HTTP(S) proxy URL without a path, query, or fragment",
    )
    .with_platform(Platform::Netease)
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

fn generate_anonymous_device_id() -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    (0..52)
        .map(|_| char::from(HEX[rand::random_range(0_usize..HEX.len())]))
        .collect()
}

fn random_chinese_ipv4() -> Ipv4Addr {
    // Compact fallback range retained from the reference implementation. Keeping the generator
    // avoids embedding its 4,000+ entry CIDR data file in TuneWeave's binary or package.
    Ipv4Addr::new(
        116,
        rand::random_range(25_u8..=94),
        rand::random_range(1_u8..=255),
        rand::random_range(1_u8..=255),
    )
}

fn anonymous_username(device_id: &str) -> String {
    const XOR_KEY: &[u8] = b"3go8&$8*3*3h0k(2)2";
    let xored = device_id
        .as_bytes()
        .iter()
        .enumerate()
        .map(|(index, byte)| byte ^ XOR_KEY[index % XOR_KEY.len()])
        .collect::<Vec<_>>();
    let digest = BASE64.encode(Md5::digest(&xored));
    BASE64.encode(format!("{device_id} {digest}"))
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
    fn voice_lyric_assets_only_allow_netease_media_hosts() {
        for url in [
            "http://d1.music.126.net/voice/lyric.json?token=opaque",
            "https://music.163.com/voice/lyric.json",
            "https://cdn.music.163.com/voice/lyric.json",
        ] {
            assert!(validate_voice_lyric_url(url).is_ok(), "{url}");
        }
        for url in [
            "file:///private/lyric.json",
            "http://127.0.0.1/lyric.json",
            "https://music.126.net.evil.test/lyric.json",
            "https://user@d1.music.126.net/lyric.json",
            "https://d1.music.126.net:8443/lyric.json",
            "https://d1.music.126.net/lyric.json#fragment",
        ] {
            assert!(validate_voice_lyric_url(url).is_err(), "{url}");
        }
    }

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
    fn anonymous_registration_identity_matches_the_reference_encoding() {
        let device_id = "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123";
        assert_eq!(
            anonymous_username(device_id),
            "MDEyMzQ1Njc4OUFCQ0RFRjAxMjM0NTY3ODlBQkNERUYwMTIzNDU2Nzg5QUJDREVGMDEyMyBYa2pIc2o5dnlXcTVRNDdCdXYyVWNnPT0="
        );
        for _ in 0..16 {
            let generated = generate_anonymous_device_id();
            assert_eq!(generated.len(), 52);
            assert!(
                generated
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
            );
        }
    }

    #[test]
    fn anonymous_registration_debug_output_redacts_the_cookie() {
        let registration = NeteaseAnonymousRegistration {
            device_id: "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123".to_owned(),
            body: json!({"code": 200}),
            session_cookie: "MUSIC_A=must-not-appear".to_owned(),
        };
        let debug = format!("{registration:?}");
        assert!(debug.contains("has_session_cookie: true"));
        assert!(!debug.contains("must-not-appear"));
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
    fn qr_login_and_response_codes_skip_empty_primary_aliases() {
        assert_eq!(
            qr_login_key(&json!({
                "unikey": " ",
                "data": {"unikey": "fallback-key"}
            }))
            .as_deref(),
            Some("fallback-key")
        );
        assert_eq!(
            response_code(&json!({"code": null, "data": {"code": "200"}})),
            Some(200)
        );
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
            Some("MUSIC_U=account=session/+_; __csrf=csrf-token; os=android"),
            "device-id",
            "client.1784194692000.01.0",
            "/api/cloud/del",
        );
        assert!(cookie.contains("MUSIC_U=account%3Dsession%2F%2B_"));
        assert!(cookie.contains("__csrf=csrf-token"));
        assert!(cookie.contains("deviceId=device-id"));
        assert!(cookie.contains("os=android"));
        assert!(cookie.contains("__remember_me=true"));
        assert!(cookie.contains("ntes_kaola_ad=1"));
        assert!(cookie.contains("WNMCID=client.1784194692000.01.0"));
        assert!(cookie.contains("WEVNSM=1.0.0"));
        assert!(cookie.contains("channel=netease"));
        assert!(cookie.contains("_ntes_nuid="));
        assert!(cookie.contains("_ntes_nnid="));
        assert!(cookie.contains("NMTID="));
    }

    #[test]
    fn eapi_cookie_header_matches_javascript_encode_uri_component() {
        let header = EapiHeader {
            osver: "Windows 10",
            device_id: "device_id",
            os: "pc",
            appver: "3.1.17.204416",
            versioncode: "140",
            mobilename: "",
            buildver: "1784194692".to_owned(),
            resolution: "1920x1080",
            __csrf: "csrf_token",
            channel: "netease",
            request_id: "1784194692000_0001".to_owned(),
            music_u: Some("token=_-!~*'() /+"),
            music_a: None,
        };
        let cookie = encode_cookie_header(&header);

        assert!(cookie.contains("__csrf=csrf_token"));
        assert!(cookie.contains("deviceId=device_id"));
        assert!(cookie.contains("MUSIC_U=token%3D_-!~*'()%20%2F%2B"));
        assert!(!cookie.contains("%5F"));
    }

    #[test]
    fn xeapi_uses_fixed_netease_domains_and_android_cookie_defaults() {
        let config = NeteaseConfig::default();
        assert_eq!(config.base_url, "https://interface.music.163.com");
        assert_eq!(config.xeapi_base_url, "https://interface3.music.163.com");
        assert_eq!(config.web_base_url, "https://music.163.com");
        assert_eq!(
            config.anti_cheat_v2_url,
            "https://ac.dun.163.com/v2/config/js?pn=YD00000558929251"
        );
        assert_eq!(
            config.anti_cheat_url,
            "https://ac.dun.163yun.com/v3/b?pn=YD00000558929251"
        );
        assert!(config.proxy_url.is_none());
        assert!(config.real_ip.is_none());
        assert!(!config.random_cn_ip);

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
    fn server_controlled_network_identity_sets_both_headers_without_client_input() {
        let fixed = NeteaseClient::new(NeteaseConfig {
            real_ip: Some(Ipv4Addr::new(116, 25, 1, 2)),
            ..NeteaseConfig::default()
        })
        .expect("build fixed-IP client");
        let request = fixed
            .apply_network_identity(fixed.http.get("https://example.test/api"))
            .build()
            .expect("build fixed-IP request");
        assert_eq!(request.headers()["X-Real-IP"], "116.25.1.2");
        assert_eq!(request.headers()["X-Forwarded-For"], "116.25.1.2");

        let plain = NeteaseClient::new(NeteaseConfig::default()).expect("build plain client");
        let request = plain
            .apply_network_identity(plain.http.get("https://example.test/api"))
            .build()
            .expect("build plain request");
        assert!(!request.headers().contains_key("X-Real-IP"));
        assert!(!request.headers().contains_key("X-Forwarded-For"));
    }

    #[tokio::test]
    async fn fixed_network_identity_reaches_the_actual_platform_http_request() {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind test server");
        let address = listener.local_addr().expect("test server address");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept platform request");
            let mut request = [0_u8; 16 * 1024];
            let length = stream.read(&mut request).expect("read platform request");
            let request = String::from_utf8_lossy(&request[..length]).to_ascii_lowercase();
            assert!(request.contains("x-real-ip: 116.25.1.2\r\n"));
            assert!(request.contains("x-forwarded-for: 116.25.1.2\r\n"));
            let body = r#"{"code":200}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .expect("write platform response");
        });
        let client = NeteaseClient::new(NeteaseConfig {
            base_url: format!("http://{address}"),
            real_ip: Some(Ipv4Addr::new(116, 25, 1, 2)),
            ..NeteaseConfig::default()
        })
        .expect("build fixed-IP client");
        let response = client
            .request_api("/api/header/test", json!({"value": 1}))
            .await
            .expect("send fixed-IP platform request");
        assert_eq!(response_code(&response.body), Some(200));
        server.join().expect("join test server");
    }

    #[test]
    fn random_chinese_network_identity_is_selected_once_per_client() {
        let client = NeteaseClient::new(NeteaseConfig {
            random_cn_ip: true,
            ..NeteaseConfig::default()
        })
        .expect("build random-IP client");
        let mut selected = None;
        for _ in 0..256 {
            let request = client
                .apply_network_identity(client.http.get("https://example.test/api"))
                .build()
                .expect("build random-IP request");
            let real = request.headers()["X-Real-IP"]
                .to_str()
                .expect("ASCII IP header")
                .parse::<Ipv4Addr>()
                .expect("IPv4 header");
            let forwarded = request.headers()["X-Forwarded-For"]
                .to_str()
                .expect("ASCII forwarded header");
            assert_eq!(forwarded, real.to_string());
            let octets = real.octets();
            assert_eq!(octets[0], 116);
            assert!((25..=94).contains(&octets[1]));
            assert_ne!(octets[2], 0);
            assert_ne!(octets[3], 0);
            assert_eq!(*selected.get_or_insert(real), real);
        }
    }

    #[test]
    fn network_identity_conflicts_and_unsafe_proxy_urls_fail_without_echoing_secrets() {
        let conflict = NeteaseClient::new(NeteaseConfig {
            real_ip: Some(Ipv4Addr::new(116, 25, 1, 2)),
            random_cn_ip: true,
            ..NeteaseConfig::default()
        })
        .err()
        .expect("conflicting network identity");
        assert_eq!(conflict.code, ErrorCode::InvalidRequest);

        let proxy_secret = "super-secret-proxy-password";
        let invalid = NeteaseClient::new(NeteaseConfig {
            proxy_url: Some(format!(
                "http://user:{proxy_secret}@proxy.example.test/unexpected-path"
            )),
            ..NeteaseConfig::default()
        })
        .err()
        .expect("invalid proxy URL");
        assert_eq!(invalid.code, ErrorCode::InvalidRequest);
        assert!(!format!("{invalid:?}").contains(proxy_secret));

        NeteaseClient::new(NeteaseConfig {
            proxy_url: Some("http://user:password@127.0.0.1:8080".to_owned()),
            ..NeteaseConfig::default()
        })
        .expect("valid server proxy configuration");
    }

    #[test]
    fn anti_cheat_token_parsers_accept_both_versions_and_reject_malformed_values() {
        assert_eq!(
            parse_anti_cheat_token_v2(r#"{"code":200,"result":{"conf":"v2-token"}}"#)
                .expect("parse v2 anti-cheat token"),
            "v2-token"
        );
        assert_eq!(
            parse_anti_cheat_token_v2(r#"{"code":"200","result":{"conf":" token-2 "}}"#)
                .expect("parse compatible v2 anti-cheat token"),
            "token-2"
        );
        assert_eq!(
            parse_anti_cheat_token_v3("null([200,1784194692,\"opaque-token\"])")
                .expect("parse anti-cheat token"),
            "opaque-token"
        );
        assert_eq!(
            parse_anti_cheat_token_v3(" null([\"200\",0,\"token-2\",\"ignored\"]) ")
                .expect("parse compatible anti-cheat token"),
            "token-2"
        );
        for body in [
            "",
            "[200,0,\"token\"]",
            "null([500,0,\"token\"])",
            "null([200,0,\"\"])",
            "null({\"code\":200})",
            "null([200])",
        ] {
            assert_eq!(
                parse_anti_cheat_token_v3(body)
                    .expect_err("malformed anti-cheat response")
                    .code,
                ErrorCode::UpstreamError,
                "{body}"
            );
        }
        for body in [
            "",
            r#"{"code":500,"result":{"conf":"token"}}"#,
            r#"{"code":200,"result":{"conf":""}}"#,
            r#"{"code":200,"result":{}}"#,
            r#"{"code":200,"result":{"conf":1}}"#,
            r#"{"result":{"conf":"token"}}"#,
        ] {
            assert_eq!(
                parse_anti_cheat_token_v2(body)
                    .expect_err("malformed v2 anti-cheat response")
                    .code,
                ErrorCode::UpstreamError,
                "{body}"
            );
        }
    }

    #[tokio::test]
    async fn anti_cheat_token_cache_is_shared_by_account_clients_without_refreshing() {
        let client = NeteaseClient::new(NeteaseConfig::default()).expect("build client");
        *client
            .anti_cheat_v2_token
            .write()
            .expect("write v2 anti-cheat cache") = Some("cached-v2-token".to_owned());
        *client
            .anti_cheat_v3_token
            .write()
            .expect("write v3 anti-cheat cache") = Some("cached-v3-token".to_owned());
        let account = client.with_cookie("MUSIC_U=account-session".to_owned());
        let (v2, refreshed) = account
            .anti_cheat_token(AntiCheatTokenVersion::V2, false)
            .await
            .expect("read cached v2 anti-cheat token");
        assert_eq!(v2, "cached-v2-token");
        assert!(!refreshed);
        let (v3, refreshed) = account
            .anti_cheat_token(AntiCheatTokenVersion::V3, false)
            .await
            .expect("read cached v3 anti-cheat token");
        assert_eq!(v3, "cached-v3-token");
        assert!(!refreshed);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase anti-cheat access"]
    async fn live_anti_cheat_token_registration_returns_and_refreshes_both_versions() {
        let client = NeteaseClient::new(NeteaseConfig::default()).expect("build client");
        for version in [AntiCheatTokenVersion::V2, AntiCheatTokenVersion::V3] {
            let (first, refreshed) = client
                .anti_cheat_token(version, false)
                .await
                .expect("register anti-cheat token");
            assert!(refreshed);
            assert!(!first.is_empty());
            first
                .parse::<header::HeaderValue>()
                .expect("safe token header");

            let (cached, refreshed) = client
                .anti_cheat_token(version, false)
                .await
                .expect("read cached anti-cheat token");
            assert_eq!(cached, first);
            assert!(!refreshed);

            let (second, refreshed) = client
                .anti_cheat_token(version, true)
                .await
                .expect("refresh anti-cheat token");
            assert!(refreshed);
            assert!(!second.is_empty());
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_anonymous_registration_preserves_success_or_the_current_business_boundary() {
        let client = NeteaseClient::new(NeteaseConfig::default()).expect("build client");
        match client.register_anonymous().await {
            Ok(registration) => {
                assert_eq!(response_code(&registration.body), Some(200));
                assert_eq!(registration.device_id.len(), 52);
                assert!(has_anonymous_cookie(Some(registration.session_cookie())));
                assert!(!has_authenticated_cookie(Some(
                    registration.session_cookie()
                )));
            }
            Err(error) => {
                assert_eq!(error.code, ErrorCode::UpstreamError);
                assert_eq!(error.details["code"], 400);
            }
        }
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

    #[test]
    fn cloud_upload_urls_are_restricted_to_exact_nos_destinations() {
        for valid in [
            "http://nosup-jd1.127.net/bucket/folder%2Fsong?offset=0&complete=true&version=1.0",
            "https://nosup-hz1.127.net/bucket/song?version=1.0&complete=true&offset=0",
        ] {
            validate_cloud_upload_url(valid).expect("valid NOS upload URL");
        }

        for invalid in [
            "ftp://nosup-jd1.127.net/bucket/song?offset=0&complete=true&version=1.0",
            "https://127.net/bucket/song?offset=0&complete=true&version=1.0",
            "https://nosup-jd1.127.net.evil.test/bucket/song?offset=0&complete=true&version=1.0",
            "https://user@nosup-jd1.127.net/bucket/song?offset=0&complete=true&version=1.0",
            "https://nosup-jd1.127.net:8443/bucket/song?offset=0&complete=true&version=1.0",
            "https://nosup-jd1.127.net/bucket/song?offset=0&complete=true",
            "https://nosup-jd1.127.net/bucket/song?offset=0&offset=0&complete=true&version=1.0",
            "https://nosup-jd1.127.net/bucket/song?offset=1&complete=true&version=1.0",
            "https://nosup-jd1.127.net/bucket/song?offset=0&complete=true&version=1.0#fragment",
        ] {
            let error = validate_cloud_upload_url(invalid).expect_err("invalid NOS upload URL");
            assert_eq!(error.code, ErrorCode::UpstreamError, "{invalid}");
        }
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
