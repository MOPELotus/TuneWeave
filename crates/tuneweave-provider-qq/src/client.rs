use std::{
    fmt,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use reqwest::{Client, Proxy, StatusCode, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha1::{Digest, Sha1};
use tokio::sync::Mutex as AsyncMutex;
use tuneweave_core::{AccountCredentialStore, ErrorCode, Platform, Result, TuneWeaveError};

use crate::{
    device::{DeviceStore, QqDevice, unix_seconds_now},
    qimei::request_qimei,
};

const API_ENDPOINT: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";
const SIGNED_API_ENDPOINT: &str = "https://u.y.qq.com/cgi-bin/musics.fcg";
const QUICK_SEARCH_ENDPOINT: &str = "https://c.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg";
const ANDROID_USER_AGENT: &str = "QQMusic 14090008(android 10)";
const WEB_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

#[derive(Clone, Default)]
pub struct QqConfig {
    pub proxy_url: Option<String>,
    pub device_path: Option<PathBuf>,
    pub credential_store: Option<Arc<dyn AccountCredentialStore>>,
}

impl fmt::Debug for QqConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QqConfig")
            .field(
                "proxy_url",
                &self.proxy_url.as_ref().map(|_| "[configured]"),
            )
            .field("device_path", &self.device_path)
            .field(
                "credential_store_configured",
                &self.credential_store.is_some(),
            )
            .finish()
    }
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct QqCredential {
    #[serde(default)]
    pub openid: String,
    #[serde(default, rename = "refresh_token", alias = "refreshToken")]
    pub refresh_token: String,
    #[serde(default, rename = "access_token", alias = "accessToken")]
    pub access_token: String,
    #[serde(default, rename = "expired_at", alias = "expiredAt")]
    pub expired_at: u64,
    #[serde(default, rename = "musicid", alias = "music_id")]
    pub music_id: u64,
    #[serde(default)]
    pub musickey: String,
    #[serde(default)]
    pub unionid: String,
    #[serde(default, rename = "str_musicid", alias = "strMusicid")]
    pub str_music_id: String,
    #[serde(default, rename = "refresh_key", alias = "refreshKey")]
    pub refresh_key: String,
    #[serde(default, rename = "musickeyCreateTime", alias = "musickey_create_time")]
    pub musickey_create_time: u64,
    #[serde(default, rename = "keyExpiresIn", alias = "key_expires_in")]
    pub key_expires_in: u64,
    #[serde(default, rename = "firstLogin", alias = "first_login")]
    pub first_login: i64,
    #[serde(default, rename = "bindAccountType", alias = "bind_account_type")]
    pub bind_account_type: i64,
    #[serde(default, rename = "needRefreshKeyIn", alias = "need_refresh_key_in")]
    pub need_refresh_key_in: i64,
    #[serde(default, rename = "encryptUin", alias = "encrypt_uin")]
    pub encrypt_uin: String,
    #[serde(default, rename = "loginType", alias = "login_type")]
    pub login_type: i64,
}

impl fmt::Debug for QqCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QqCredential")
            .field("music_id_configured", &(self.music_id > 0))
            .field("musickey_configured", &(!self.musickey.is_empty()))
            .field("login_type", &self.login_type)
            .finish()
    }
}

impl QqCredential {
    pub(crate) fn normalize(mut self) -> Result<Self> {
        self.str_music_id = self.str_music_id.trim().to_owned();
        self.musickey = self.musickey.trim().to_owned();
        if self.music_id == 0 && !self.str_music_id.is_empty() {
            self.music_id = self.str_music_id.parse().map_err(|_| {
                credential_data_error("stored QQ str_musicid is not an unsigned integer")
            })?;
        }
        if self.str_music_id.is_empty() && self.music_id > 0 {
            self.str_music_id = self.music_id.to_string();
        }
        if self.music_id == 0 || self.str_music_id.is_empty() || self.musickey.is_empty() {
            return Err(credential_data_error(
                "stored QQ credential is missing musicid or musickey",
            ));
        }
        if !self
            .str_music_id
            .chars()
            .all(|character| character.is_ascii_digit())
            || self
                .musickey
                .chars()
                .any(|character| character.is_ascii_control() || character == ';')
        {
            return Err(credential_data_error(
                "stored QQ credential contains invalid cookie data",
            ));
        }
        if self.login_type == 0 {
            self.login_type = if self.musickey.starts_with("W_X") {
                1
            } else {
                2
            };
        }
        Ok(self)
    }

    pub(crate) fn string_music_id(&self) -> &str {
        &self.str_music_id
    }

    pub(crate) fn expires_at_epoch(&self) -> Option<u64> {
        (self.musickey_create_time > 0 && self.key_expires_in > 0).then(|| {
            self.musickey_create_time
                .saturating_add(self.key_expires_in)
        })
    }

    pub(crate) fn is_locally_expired(&self) -> Result<Option<bool>> {
        self.expires_at_epoch()
            .map(|expires_at| unix_seconds_now().map(|now| now >= expires_at))
            .transpose()
    }

    fn cookie_header(&self) -> String {
        format!(
            "uin={0}; qqmusic_uin={0}; qm_keyst={1}; qqmusic_key={1}",
            self.str_music_id, self.musickey
        )
    }
}

#[derive(Clone)]
pub(crate) struct QqApiRequest {
    pub module: String,
    pub method: String,
    pub param: Value,
    preserve_booleans: bool,
}

impl QqApiRequest {
    pub(crate) fn new(module: &str, method: &str, param: Value) -> Self {
        Self {
            module: module.to_owned(),
            method: method.to_owned(),
            param,
            preserve_booleans: false,
        }
    }

    pub(crate) const fn preserving_booleans(mut self) -> Self {
        self.preserve_booleans = true;
        self
    }

    #[cfg(test)]
    pub(crate) const fn preserves_booleans(&self) -> bool {
        self.preserve_booleans
    }
}

#[derive(Clone)]
pub(crate) struct QqApiResponse {
    pub data: Value,
    pub raw: Value,
}

pub(crate) struct QqBusinessResponse {
    pub code: i64,
    pub data: Value,
    pub raw: Value,
}

#[derive(Clone)]
pub struct QqClient {
    http: Client,
    login_http: Client,
    device: Arc<Mutex<DeviceStore>>,
    qimei_refresh: Arc<AsyncMutex<()>>,
    session_refresh: Arc<AsyncMutex<()>>,
}

impl QqClient {
    pub fn new(config: QqConfig) -> Result<Self> {
        let mut builder = Client::builder().user_agent(ANDROID_USER_AGENT);
        let mut login_builder = Client::builder()
            .user_agent(WEB_USER_AGENT)
            .redirect(Policy::none());
        let proxy_url = config
            .proxy_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        if let Some(proxy_url) = proxy_url.as_deref() {
            let proxy = Proxy::all(proxy_url).map_err(|_| {
                TuneWeaveError::invalid_request("QQ proxy configuration is invalid")
                    .with_platform(Platform::Qq)
            })?;
            builder = builder.proxy(proxy.clone());
            login_builder = login_builder.proxy(proxy);
        }
        let http = builder.build().map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                format!("failed to build QQ HTTP client: {error}"),
            )
            .with_platform(Platform::Qq)
        })?;
        let login_http = login_builder.build().map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                format!("failed to build QQ login HTTP client: {error}"),
            )
            .with_platform(Platform::Qq)
        })?;
        let device = DeviceStore::open(config.device_path)?;
        Ok(Self {
            http,
            login_http,
            device: Arc::new(Mutex::new(device)),
            qimei_refresh: Arc::new(AsyncMutex::new(())),
            session_refresh: Arc::new(AsyncMutex::new(())),
        })
    }

    pub(crate) async fn request_android(
        &self,
        requests: &[QqApiRequest],
    ) -> Result<Vec<QqApiResponse>> {
        self.request_android_with_credential(requests, None).await
    }

    pub(crate) async fn request_android_with_credential(
        &self,
        requests: &[QqApiRequest],
        credential: Option<&QqCredential>,
    ) -> Result<Vec<QqApiResponse>> {
        if requests.is_empty() {
            return Err(TuneWeaveError::invalid_request(
                "QQ API batch must contain at least one request",
            )
            .with_platform(Platform::Qq));
        }
        self.ensure_android_session().await?;
        let device = self.lock_device()?.device().clone();
        let comm = android_comm(&device, credential);
        let response = self.post_api(&comm, requests, credential).await;
        if credential.is_none() && response.as_ref().is_err_and(is_anonymous_session_rejection) {
            self.invalidate_android_session()?;
            self.ensure_android_session().await?;
            let device = self.lock_device()?.device().clone();
            let comm = android_comm(&device, None);
            return self.post_api(&comm, requests, None).await;
        }
        response
    }

    pub(crate) async fn request_android_signed_with_credential(
        &self,
        requests: &[QqApiRequest],
        credential: &QqCredential,
    ) -> Result<Vec<QqApiResponse>> {
        if requests.is_empty() {
            return Err(TuneWeaveError::invalid_request(
                "QQ signed API batch must contain at least one request",
            )
            .with_platform(Platform::Qq));
        }
        self.ensure_android_session().await?;
        let device = self.lock_device()?.device().clone();
        let comm = android_comm(&device, Some(credential));
        self.post_api_signed(&comm, requests, credential).await
    }

    pub(crate) async fn request_android_login(
        &self,
        request: QqApiRequest,
        login_type: i64,
        login_method: Option<i64>,
    ) -> Result<QqApiResponse> {
        let mut comm_fields = vec![("tmeLoginType", json!(login_type))];
        if let Some(login_method) = login_method {
            comm_fields.push(("tmeLoginMethod", json!(login_method)));
        }
        self.ensure_android_session().await?;
        self.post_android_with_comm_fields(&request, &comm_fields)
            .await
    }

    pub(crate) async fn request_android_with_comm(
        &self,
        request: QqApiRequest,
        comm_fields: &[(&str, Value)],
    ) -> Result<QqApiResponse> {
        self.ensure_android_session().await?;
        let response = self
            .post_android_with_comm_fields(&request, comm_fields)
            .await;
        if response.as_ref().is_err_and(is_anonymous_session_rejection) {
            self.invalidate_android_session()?;
            self.ensure_android_session().await?;
            return self
                .post_android_with_comm_fields(&request, comm_fields)
                .await;
        }
        response
    }

    pub(crate) async fn request_with_exact_comm(
        &self,
        request: QqApiRequest,
        comm: &Value,
    ) -> Result<QqApiResponse> {
        if !comm.is_object() {
            return Err(TuneWeaveError::new(
                ErrorCode::InternalError,
                "QQ exact comm must be an object",
            )
            .with_platform(Platform::Qq));
        }
        self.post_api(comm, std::slice::from_ref(&request), None)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| qq_data_error("QQ exact-comm service returned no response"))
    }

    pub(crate) async fn request_with_exact_comm_allowing_business_errors(
        &self,
        request: QqApiRequest,
        comm: &Value,
    ) -> Result<QqBusinessResponse> {
        self.request_exact_comm_allowing_business_errors(request, comm, false)
            .await
    }

    pub(crate) async fn request_signed_with_exact_comm_allowing_business_errors(
        &self,
        request: QqApiRequest,
        comm: &Value,
    ) -> Result<QqBusinessResponse> {
        self.request_exact_comm_allowing_business_errors(request, comm, true)
            .await
    }

    async fn request_exact_comm_allowing_business_errors(
        &self,
        request: QqApiRequest,
        comm: &Value,
        signed: bool,
    ) -> Result<QqBusinessResponse> {
        if !comm.is_object() {
            return Err(TuneWeaveError::new(
                ErrorCode::InternalError,
                "QQ exact comm must be an object",
            )
            .with_platform(Platform::Qq));
        }
        let response = self
            .post_api_with_user_agent_options(
                comm,
                std::slice::from_ref(&request),
                None,
                None,
                true,
                signed,
            )
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| qq_data_error("QQ exact-comm business service returned no response"))?;
        let code = response
            .raw
            .get("code")
            .and_then(platform_code)
            .ok_or_else(|| qq_data_error("QQ exact-comm business response is missing a code"))?;
        Ok(QqBusinessResponse {
            code,
            data: response.data,
            raw: response.raw,
        })
    }

    pub(crate) async fn request_android_business(
        &self,
        request: QqApiRequest,
        comm_fields: &[(&str, Value)],
    ) -> Result<QqBusinessResponse> {
        self.request_android_business_with_credential(request, comm_fields, None)
            .await
    }

    pub(crate) async fn request_android_business_with_credential(
        &self,
        request: QqApiRequest,
        comm_fields: &[(&str, Value)],
        credential: Option<&QqCredential>,
    ) -> Result<QqBusinessResponse> {
        self.ensure_android_session().await?;
        let response = self
            .post_android_with_comm_fields_allowing_business_errors(
                &request,
                comm_fields,
                credential,
            )
            .await?;
        let code = response
            .raw
            .get("code")
            .and_then(platform_code)
            .ok_or_else(|| qq_data_error("QQ business response is missing a valid code"))?;
        Ok(QqBusinessResponse {
            code,
            data: response.data,
            raw: response.raw,
        })
    }

    async fn post_android_with_comm_fields(
        &self,
        request: &QqApiRequest,
        comm_fields: &[(&str, Value)],
    ) -> Result<QqApiResponse> {
        let device = self.lock_device()?.device().clone();
        let mut comm = android_comm(&device, None);
        {
            let fields = comm
                .as_object_mut()
                .expect("QQ Android comm is always an object");
            for (name, value) in comm_fields {
                fields.insert((*name).to_owned(), value.clone());
            }
        }
        self.post_api(&comm, std::slice::from_ref(request), None)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| qq_data_error("QQ login service returned no response"))
    }

    async fn post_android_with_comm_fields_allowing_business_errors(
        &self,
        request: &QqApiRequest,
        comm_fields: &[(&str, Value)],
        credential: Option<&QqCredential>,
    ) -> Result<QqApiResponse> {
        let device = self.lock_device()?.device().clone();
        let mut comm = android_comm(&device, credential);
        {
            let fields = comm
                .as_object_mut()
                .expect("QQ Android comm is always an object");
            for (name, value) in comm_fields {
                fields.insert((*name).to_owned(), value.clone());
            }
        }
        self.post_api_allowing_business_errors(&comm, std::slice::from_ref(request), credential)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| qq_data_error("QQ business service returned no response"))
    }

    pub(crate) fn login_http(&self) -> &Client {
        &self.login_http
    }

    pub(crate) async fn request_web(
        &self,
        requests: &[QqApiRequest],
    ) -> Result<Vec<QqApiResponse>> {
        if requests.is_empty() {
            return Err(TuneWeaveError::invalid_request(
                "QQ API batch must contain at least one request",
            )
            .with_platform(Platform::Qq));
        }
        self.post_api_with_user_agent(&web_comm(), requests, Some(WEB_USER_AGENT), None)
            .await
    }

    pub(crate) async fn request_quick_search(&self, keyword: &str) -> Result<Value> {
        let mut endpoint = reqwest::Url::parse(QUICK_SEARCH_ENDPOINT).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                format!("QQ quick search endpoint is invalid: {error}"),
            )
            .with_platform(Platform::Qq)
        })?;
        endpoint.query_pairs_mut().append_pair("key", keyword);
        let response = self
            .http
            .get(endpoint)
            .send()
            .await
            .map_err(network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(http_error(status));
        }
        response.json().await.map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("QQ quick search returned invalid JSON: {error}"),
            )
            .with_platform(Platform::Qq)
        })
    }

    async fn ensure_qimei(&self) -> Result<()> {
        let now = unix_seconds_now()?;
        if self.lock_device()?.device().has_current_qimei(now) {
            return Ok(());
        }
        let _refresh = self.qimei_refresh.lock().await;
        let now = unix_seconds_now()?;
        if self.lock_device()?.device().has_current_qimei(now) {
            return Ok(());
        }
        let device = self.lock_device()?.device().clone();
        let identity = request_qimei(&self.http, &device).await?;
        let mut store = self.lock_device()?;
        let previous = store.device().clone();
        let device = store.device_mut();
        device.qimei = Some(identity.q16);
        device.qimei36 = Some(identity.q36);
        device.qimei_saved_at = Some(unix_seconds_now()?);
        if let Err(error) = store.save() {
            *store.device_mut() = previous;
            return Err(error);
        }
        Ok(())
    }

    async fn ensure_android_session(&self) -> Result<()> {
        self.ensure_qimei().await?;
        let now = unix_seconds_now()?;
        if self.lock_device()?.device().has_current_session(now) {
            return Ok(());
        }
        let _refresh = self.session_refresh.lock().await;
        let now = unix_seconds_now()?;
        if self.lock_device()?.device().has_current_session(now) {
            return Ok(());
        }
        let device = self.lock_device()?.device().clone();
        let comm = android_comm(&device, None);
        let request = QqApiRequest::new(
            "music.getSession.session",
            "GetSession",
            json!({
                "uid": device.session_uid.unwrap_or_default(),
                "vkey": 0,
                "caller": 0
            }),
        );
        let response = self.post_api(&comm, &[request], None).await?;
        let session = response[0]
            .data
            .get("session")
            .and_then(Value::as_object)
            .ok_or_else(|| qq_data_error("QQ session response is missing session data"))?;
        let uid = value_as_string(session.get("uid"))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| qq_data_error("QQ session response is missing uid"))?;
        let sid = session
            .get("sid")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| qq_data_error("QQ session response is missing sid"))?;
        let vkey = session
            .get("vkey")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned);
        let mut store = self.lock_device()?;
        let previous = store.device().clone();
        let device = store.device_mut();
        device.session_uid = Some(uid);
        device.session_sid = Some(sid.to_owned());
        device.session_vkey = vkey;
        device.session_saved_at = Some(unix_seconds_now()?);
        if let Err(error) = store.save() {
            *store.device_mut() = previous;
            return Err(error);
        }
        Ok(())
    }

    async fn post_api(
        &self,
        comm: &Value,
        requests: &[QqApiRequest],
        credential: Option<&QqCredential>,
    ) -> Result<Vec<QqApiResponse>> {
        self.post_api_with_user_agent(comm, requests, None, credential)
            .await
    }

    async fn post_api_allowing_business_errors(
        &self,
        comm: &Value,
        requests: &[QqApiRequest],
        credential: Option<&QqCredential>,
    ) -> Result<Vec<QqApiResponse>> {
        self.post_api_with_user_agent_options(comm, requests, None, credential, true, false)
            .await
    }

    async fn post_api_with_user_agent(
        &self,
        comm: &Value,
        requests: &[QqApiRequest],
        user_agent: Option<&str>,
        credential: Option<&QqCredential>,
    ) -> Result<Vec<QqApiResponse>> {
        self.post_api_with_user_agent_options(comm, requests, user_agent, credential, false, false)
            .await
    }

    async fn post_api_signed(
        &self,
        comm: &Value,
        requests: &[QqApiRequest],
        credential: &QqCredential,
    ) -> Result<Vec<QqApiResponse>> {
        self.post_api_with_user_agent_options(comm, requests, None, Some(credential), false, true)
            .await
    }

    async fn post_api_with_user_agent_options(
        &self,
        comm: &Value,
        requests: &[QqApiRequest],
        user_agent: Option<&str>,
        credential: Option<&QqCredential>,
        allow_business_errors: bool,
        signed: bool,
    ) -> Result<Vec<QqApiResponse>> {
        let mut payload = Map::new();
        payload.insert("comm".to_owned(), comm.clone());
        for (index, request) in requests.iter().enumerate() {
            let param = request_param(request);
            payload.insert(
                format!("req_{index}"),
                json!({
                    "module": request.module,
                    "method": request.method,
                    "param": param
                }),
            );
        }
        let payload = Value::Object(payload);
        let mut request = if signed {
            let body = serde_json::to_vec(&payload).map_err(|error| {
                TuneWeaveError::new(
                    ErrorCode::InternalError,
                    format!("failed to serialize QQ signed request: {error}"),
                )
                .with_platform(Platform::Qq)
            })?;
            let mut endpoint = reqwest::Url::parse(SIGNED_API_ENDPOINT).map_err(|error| {
                TuneWeaveError::new(
                    ErrorCode::InternalError,
                    format!("QQ signed API endpoint is invalid: {error}"),
                )
                .with_platform(Platform::Qq)
            })?;
            endpoint
                .query_pairs_mut()
                .append_pair("_", &unix_millis_now()?.to_string())
                .append_pair("sign", &zzc_sign(&body));
            self.http
                .post(endpoint)
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body)
        } else {
            self.http.post(API_ENDPOINT).json(&payload)
        };
        if let Some(user_agent) = user_agent {
            request = request.header(reqwest::header::USER_AGENT, user_agent);
        }
        if let Some(credential) = credential {
            request = request.header(reqwest::header::COOKIE, credential.cookie_header());
        }
        let response = request.send().await.map_err(network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(http_error(status));
        }
        let raw: Value = response.json().await.map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("QQ API returned invalid JSON: {error}"),
            )
            .with_platform(Platform::Qq)
        })?;
        ensure_zero_code(&raw, "QQ API batch")?;
        requests
            .iter()
            .enumerate()
            .map(|(index, request)| {
                let key = format!("req_{index}");
                let response = raw
                    .get(&key)
                    .ok_or_else(|| qq_data_error(format!("QQ API response is missing {key}")))?;
                let code = response
                    .get("code")
                    .and_then(platform_code)
                    .ok_or_else(|| qq_data_error(format!("QQ API response {key} has no code")))?;
                if !allow_business_errors || code == 0 {
                    ensure_zero_code(
                        response,
                        &format!("QQ API {}.{}", request.module, request.method),
                    )?;
                }
                let data = match response.get("data").cloned() {
                    Some(data) => data,
                    None if allow_business_errors => json!({}),
                    None => {
                        return Err(qq_data_error(format!(
                            "QQ API response {key} is missing data"
                        )));
                    }
                };
                Ok(QqApiResponse {
                    data,
                    raw: response.clone(),
                })
            })
            .collect()
    }

    fn lock_device(&self) -> Result<MutexGuard<'_, DeviceStore>> {
        self.device.lock().map_err(|_| {
            TuneWeaveError::new(ErrorCode::InternalError, "QQ device state lock is poisoned")
                .with_platform(Platform::Qq)
        })
    }

    fn invalidate_android_session(&self) -> Result<()> {
        let mut store = self.lock_device()?;
        let previous = store.device().clone();
        let device = store.device_mut();
        device.session_uid = None;
        device.session_sid = None;
        device.session_vkey = None;
        device.session_saved_at = None;
        if let Err(error) = store.save() {
            *store.device_mut() = previous;
            return Err(error);
        }
        Ok(())
    }
}

fn unix_millis_now() -> Result<u128> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "system clock is before the Unix epoch",
            )
            .with_platform(Platform::Qq)
        })
}

fn zzc_sign(payload: &[u8]) -> String {
    const PART_1_INDEXES: [usize; 7] = [23, 14, 6, 36, 16, 7, 19];
    const PART_2_INDEXES: [usize; 8] = [16, 1, 32, 12, 19, 27, 8, 5];
    const SCRAMBLE_VALUES: [u8; 20] = [
        89, 39, 179, 150, 218, 82, 58, 252, 177, 52, 186, 123, 120, 64, 242, 133, 143, 161, 121,
        179,
    ];

    let hash = Sha1::digest(payload);
    let hash_hex = hex::encode_upper(hash);
    let part_1 = PART_1_INDEXES
        .into_iter()
        .map(|index| char::from(hash_hex.as_bytes()[index]))
        .collect::<String>();
    let part_2 = PART_2_INDEXES
        .into_iter()
        .map(|index| char::from(hash_hex.as_bytes()[index]))
        .collect::<String>();
    let scrambled = std::array::from_fn::<_, 20, _>(|index| SCRAMBLE_VALUES[index] ^ hash[index]);
    let encoded = BASE64
        .encode(scrambled)
        .chars()
        .filter(|character| !matches!(character, '/' | '\\' | '+' | '='))
        .collect::<String>();
    format!("zzc{part_1}{encoded}{part_2}").to_ascii_lowercase()
}

fn android_comm(device: &QqDevice, credential: Option<&QqCredential>) -> Value {
    let mut comm = json!({
        "ct": 11,
        "cv": 14090008,
        "v": 14090008,
        "chid": "10003505",
        "tmeAppID": "qqmusic",
        "QIMEI": device.qimei.as_deref().unwrap_or_default(),
        "QIMEI36": device.qimei36.as_deref().unwrap_or_default(),
        "OpenUDID": device.open_udid,
        "udid": device.open_udid,
        "OpenUDID2": device.open_udid,
        "aid": device.android_id,
        "os_ver": "10",
        "phonetype": "MI 6",
        "devicelevel": "29",
        "newdevicelevel": "29",
        "rom": device.fingerprint
    });
    let object = comm
        .as_object_mut()
        .expect("QQ Android comm is always an object");
    if let Some(uid) = device
        .session_uid
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        object.insert("uid".to_owned(), Value::String(uid.to_owned()));
    }
    if let Some(sid) = device
        .session_sid
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        object.insert("sid".to_owned(), Value::String(sid.to_owned()));
    }
    if let Some(credential) = credential {
        object.insert(
            "qq".to_owned(),
            Value::String(credential.music_id.to_string()),
        );
        object.insert(
            "authst".to_owned(),
            Value::String(credential.musickey.clone()),
        );
        object.insert("tmeLoginType".to_owned(), json!(credential.login_type));
    }
    comm
}

fn credential_data_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::InternalError, message).with_platform(Platform::Qq)
}

fn web_comm() -> Value {
    json!({
        "ct": 24,
        "cv": 4747474,
        "platform": "yqq.json",
        "chid": "0",
        "uin": 0,
        "g_tk": 5381,
        "g_tk_new_20200303": 5381,
        "format": "json",
        "inCharset": "utf-8",
        "outCharset": "utf-8",
        "notice": 0,
        "needNewCode": 1
    })
}

fn booleans_to_integers(value: Value) -> Value {
    match value {
        Value::Bool(value) => Value::Number(u64::from(value).into()),
        Value::Array(values) => {
            Value::Array(values.into_iter().map(booleans_to_integers).collect())
        }
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, booleans_to_integers(value)))
                .collect(),
        ),
        value => value,
    }
}

fn request_param(request: &QqApiRequest) -> Value {
    if request.preserve_booleans {
        request.param.clone()
    } else {
        booleans_to_integers(request.param.clone())
    }
}

fn value_as_string(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(value)) => Some(value.trim().to_owned()),
        Some(Value::Number(value)) => Some(value.to_string()),
        _ => None,
    }
}

fn ensure_zero_code(value: &Value, context: &str) -> Result<()> {
    let code = value
        .get("code")
        .and_then(platform_code)
        .ok_or_else(|| qq_data_error(format!("{context} is missing a valid code")))?;
    if code == 0 {
        return Ok(());
    }
    let error_code = match code {
        1000 | 104_400 | 104_401 => ErrorCode::AuthenticationRequired,
        20_261 => ErrorCode::InvalidRequest,
        20_271 | 20_277 | 20_278 | 20_450 => ErrorCode::PermissionDenied,
        20_272 | 20_274 | 20_279 => ErrorCode::Conflict,
        100_001 | 104_604 | 2001 => ErrorCode::RateLimited,
        _ => ErrorCode::UpstreamError,
    };
    Err(
        TuneWeaveError::new(error_code, format!("{context} failed with code {code}"))
            .with_platform(Platform::Qq)
            .retryable(matches!(code, 100_001 | 104_604 | 2001))
            .with_details(json!({ "platform_code": code })),
    )
}

fn is_anonymous_session_rejection(error: &TuneWeaveError) -> bool {
    error.code == ErrorCode::AuthenticationRequired
        && matches!(
            error.details.get("platform_code").and_then(Value::as_i64),
            Some(1000 | 104_400 | 104_401)
        )
}

fn platform_code(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn network_error(error: reqwest::Error) -> TuneWeaveError {
    let code = if error.is_timeout() {
        ErrorCode::UpstreamTimeout
    } else {
        ErrorCode::UpstreamError
    };
    TuneWeaveError::new(code, format!("QQ API request failed: {error}"))
        .with_platform(Platform::Qq)
        .retryable(true)
}

fn http_error(status: StatusCode) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("QQ API returned HTTP {status}"),
    )
    .with_platform(Platform::Qq)
    .retryable(status.is_server_error())
}

fn qq_data_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message)
        .with_platform(Platform::Qq)
        .retryable(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zzc_signature_matches_independent_python_vectors() {
        for (payload, expected) in [
            (
                b"".as_slice(),
                "zzcf0e03e5gx4qeiq5cfgdyqwu7sdqfsb5fro3aa45053",
            ),
            (
                b"{}".as_slice(),
                "zzcf8e26805gyafigxmxjehoe02mvsjjgtwzw6f1a05f9",
            ),
            (
                br#"{"comm":{"ct":11},"req_0":{"module":"music.feedback.FeedbackBlack","method":"GetDislikeList","param":{"Cmd":3,"Page":1}}}"#.as_slice(),
                "zzc70553ebaldyicnjfqbpwu8na9rh9iin831f1b65e",
            ),
        ] {
            assert_eq!(zzc_sign(payload), expected);
        }
    }

    #[test]
    fn booleans_are_normalized_recursively_without_touching_numbers() {
        assert_eq!(
            booleans_to_integers(json!({"top": true, "nested": [false, 3]})),
            json!({"top": 1, "nested": [0, 3]})
        );
    }

    #[test]
    fn individual_requests_can_preserve_protocol_boolean_values() {
        let regular = QqApiRequest::new("module", "method", json!({"flag": false}));
        let preserved =
            QqApiRequest::new("module", "method", json!({"flag": false})).preserving_booleans();
        assert_eq!(request_param(&regular), json!({"flag": 0}));
        assert_eq!(request_param(&preserved), json!({"flag": false}));
    }

    #[test]
    fn android_comm_does_not_invent_session_fields() {
        let device = QqDevice {
            qimei: Some("q16".to_owned()),
            qimei36: Some("q36".to_owned()),
            ..QqDevice::default()
        };
        let comm = android_comm(&device, None);
        assert_eq!(comm["QIMEI"], "q16");
        assert_eq!(comm["QIMEI36"], "q36");
        assert!(comm.get("uid").is_none());
        assert!(comm.get("sid").is_none());
    }

    #[test]
    fn stored_credentials_infer_login_type_and_only_enter_authenticated_comm() {
        let credential: QqCredential = serde_json::from_value(json!({
            "musicid": 123456,
            "musickey": "Q_H_L_private",
            "str_musicid": "123456"
        }))
        .expect("credential shape");
        let credential = credential.normalize().expect("valid credential");
        assert_eq!(credential.login_type, 2);
        assert_eq!(credential.string_music_id(), "123456");
        let debug = format!("{credential:?}");
        assert!(!debug.contains("Q_H_L_private"));

        let device = QqDevice::default();
        let anonymous = android_comm(&device, None);
        assert!(anonymous.get("qq").is_none());
        assert!(anonymous.get("authst").is_none());
        let authenticated = android_comm(&device, Some(&credential));
        assert_eq!(authenticated["qq"], "123456");
        assert_eq!(authenticated["authst"], "Q_H_L_private");
        assert_eq!(authenticated["tmeLoginType"], 2);
        assert!(credential.cookie_header().contains("qqmusic_uin=123456"));
    }

    #[test]
    fn stored_credentials_reject_missing_or_cookie_unsafe_secrets() {
        for value in [
            json!({"musicid": 123456, "musickey": ""}),
            json!({"musicid": 123456, "musickey": "secret; injected=true"}),
            json!({"str_musicid": "not-a-number", "musickey": "secret"}),
        ] {
            let credential: QqCredential = serde_json::from_value(value).expect("credential shape");
            let error = credential.normalize().expect_err("invalid credential");
            assert_eq!(error.code, ErrorCode::InternalError);
            assert!(!error.message.contains("secret; injected=true"));
        }
    }

    #[test]
    fn credential_expiration_is_unknown_without_both_server_timestamps() {
        let unknown = QqCredential {
            music_id: 123456,
            str_music_id: "123456".to_owned(),
            musickey: "Q_H_L_private".to_owned(),
            ..serde_json::from_value(json!({})).expect("empty credential")
        };
        assert_eq!(unknown.expires_at_epoch(), None);
        assert_eq!(unknown.is_locally_expired().expect("local status"), None);

        let known = QqCredential {
            musickey_create_time: 100,
            key_expires_in: 50,
            ..unknown
        };
        assert_eq!(known.expires_at_epoch(), Some(150));
        assert_eq!(
            known.is_locally_expired().expect("local status"),
            Some(true)
        );
    }

    #[test]
    fn web_comm_matches_the_reference_profile_without_device_identity() {
        assert_eq!(
            web_comm(),
            json!({
                "ct": 24,
                "cv": 4747474,
                "platform": "yqq.json",
                "chid": "0",
                "uin": 0,
                "g_tk": 5381,
                "g_tk_new_20200303": 5381,
                "format": "json",
                "inCharset": "utf-8",
                "outCharset": "utf-8",
                "notice": 0,
                "needNewCode": 1
            })
        );
    }

    #[test]
    fn nonzero_subrequest_code_is_not_hidden_by_batch_success() {
        let error = ensure_zero_code(&json!({"code": 1001}), "search").expect_err("failure");
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert_eq!(error.details["platform_code"], 1001);
    }

    #[test]
    fn credential_and_rate_limit_codes_keep_their_stable_error_classes() {
        for code in [1000, 104_400, 104_401] {
            let error =
                ensure_zero_code(&json!({"code": code}), "audio").expect_err("credential failure");
            assert_eq!(error.code, ErrorCode::AuthenticationRequired);
            assert!(is_anonymous_session_rejection(&error));
            assert!(!error.retryable);
        }
        let limited = ensure_zero_code(&json!({"code": 2001}), "audio").expect_err("rate limited");
        assert_eq!(limited.code, ErrorCode::RateLimited);
        assert!(limited.retryable);
        assert!(!is_anonymous_session_rejection(&limited));
        assert_eq!(
            ensure_zero_code(&json!({"code": 20_271}), "login")
                .expect_err("verification code failure")
                .code,
            ErrorCode::PermissionDenied
        );
        assert_eq!(
            ensure_zero_code(&json!({"code": 20_279}), "login")
                .expect_err("device limit failure")
                .code,
            ErrorCode::Conflict
        );
    }

    #[test]
    fn missing_batch_code_is_not_assumed_to_be_success() {
        let error = ensure_zero_code(&json!({"data": {}}), "search").expect_err("failure");
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(error.message.contains("missing a valid code"));
    }
}
