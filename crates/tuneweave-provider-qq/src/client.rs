use std::{
    fmt,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};

use reqwest::{Client, Proxy, StatusCode};
use serde_json::{Map, Value, json};
use tokio::sync::Mutex as AsyncMutex;
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};

use crate::{
    device::{DeviceStore, QqDevice, unix_seconds_now},
    qimei::request_qimei,
};

const API_ENDPOINT: &str = "https://u.y.qq.com/cgi-bin/musicu.fcg";
const QUICK_SEARCH_ENDPOINT: &str = "https://c.y.qq.com/splcloud/fcgi-bin/smartbox_new.fcg";
const ANDROID_USER_AGENT: &str = "QQMusic 14090008(android 10)";
const WEB_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

#[derive(Clone, Default)]
pub struct QqConfig {
    pub proxy_url: Option<String>,
    pub device_path: Option<PathBuf>,
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
            .finish()
    }
}

#[derive(Clone)]
pub(crate) struct QqApiRequest {
    pub module: String,
    pub method: String,
    pub param: Value,
}

impl QqApiRequest {
    pub(crate) fn new(module: &str, method: &str, param: Value) -> Self {
        Self {
            module: module.to_owned(),
            method: method.to_owned(),
            param,
        }
    }
}

#[derive(Clone)]
pub(crate) struct QqApiResponse {
    pub data: Value,
    pub raw: Value,
}

#[derive(Clone)]
pub struct QqClient {
    http: Client,
    device: Arc<Mutex<DeviceStore>>,
    qimei_refresh: Arc<AsyncMutex<()>>,
    session_refresh: Arc<AsyncMutex<()>>,
}

impl QqClient {
    pub fn new(config: QqConfig) -> Result<Self> {
        let mut builder = Client::builder().user_agent(ANDROID_USER_AGENT);
        if let Some(proxy_url) = config
            .proxy_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let proxy = Proxy::all(proxy_url).map_err(|_| {
                TuneWeaveError::invalid_request("QQ proxy configuration is invalid")
                    .with_platform(Platform::Qq)
            })?;
            builder = builder.proxy(proxy);
        }
        let http = builder.build().map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                format!("failed to build QQ HTTP client: {error}"),
            )
            .with_platform(Platform::Qq)
        })?;
        let device = DeviceStore::open(config.device_path)?;
        Ok(Self {
            http,
            device: Arc::new(Mutex::new(device)),
            qimei_refresh: Arc::new(AsyncMutex::new(())),
            session_refresh: Arc::new(AsyncMutex::new(())),
        })
    }

    pub(crate) async fn request_android(
        &self,
        requests: &[QqApiRequest],
    ) -> Result<Vec<QqApiResponse>> {
        if requests.is_empty() {
            return Err(TuneWeaveError::invalid_request(
                "QQ API batch must contain at least one request",
            )
            .with_platform(Platform::Qq));
        }
        self.ensure_android_session().await?;
        let device = self.lock_device()?.device().clone();
        let comm = android_comm(&device);
        self.post_api(&comm, requests).await
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
        self.post_api_with_user_agent(&web_comm(), requests, Some(WEB_USER_AGENT))
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
        let comm = android_comm(&device);
        let request = QqApiRequest::new(
            "music.getSession.session",
            "GetSession",
            json!({
                "uid": device.session_uid.unwrap_or_default(),
                "vkey": 0,
                "caller": 0
            }),
        );
        let response = self.post_api(&comm, &[request]).await?;
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
    ) -> Result<Vec<QqApiResponse>> {
        self.post_api_with_user_agent(comm, requests, None).await
    }

    async fn post_api_with_user_agent(
        &self,
        comm: &Value,
        requests: &[QqApiRequest],
        user_agent: Option<&str>,
    ) -> Result<Vec<QqApiResponse>> {
        let mut payload = Map::new();
        payload.insert("comm".to_owned(), comm.clone());
        for (index, request) in requests.iter().enumerate() {
            payload.insert(
                format!("req_{index}"),
                json!({
                    "module": request.module,
                    "method": request.method,
                    "param": booleans_to_integers(request.param.clone())
                }),
            );
        }
        let mut request = self.http.post(API_ENDPOINT).json(&Value::Object(payload));
        if let Some(user_agent) = user_agent {
            request = request.header(reqwest::header::USER_AGENT, user_agent);
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
                ensure_zero_code(
                    response,
                    &format!("QQ API {}.{}", request.module, request.method),
                )?;
                let data = response.get("data").cloned().ok_or_else(|| {
                    qq_data_error(format!("QQ API response {key} is missing data"))
                })?;
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
}

fn android_comm(device: &QqDevice) -> Value {
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
    comm
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
    Err(TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("{context} failed with code {code}"),
    )
    .with_platform(Platform::Qq)
    .with_details(json!({ "platform_code": code })))
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
    fn booleans_are_normalized_recursively_without_touching_numbers() {
        assert_eq!(
            booleans_to_integers(json!({"top": true, "nested": [false, 3]})),
            json!({"top": 1, "nested": [0, 3]})
        );
    }

    #[test]
    fn android_comm_does_not_invent_session_fields() {
        let device = QqDevice {
            qimei: Some("q16".to_owned()),
            qimei36: Some("q36".to_owned()),
            ..QqDevice::default()
        };
        let comm = android_comm(&device);
        assert_eq!(comm["QIMEI"], "q16");
        assert_eq!(comm["QIMEI36"], "q36");
        assert!(comm.get("uid").is_none());
        assert!(comm.get("sid").is_none());
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
    fn missing_batch_code_is_not_assumed_to_be_success() {
        let error = ensure_zero_code(&json!({"data": {}}), "search").expect_err("failure");
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(error.message.contains("missing a valid code"));
    }
}
