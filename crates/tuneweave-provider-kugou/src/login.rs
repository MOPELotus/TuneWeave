//! Low-level QR protocol. Receiving a token is not a verified TuneWeave session.

use std::{
    collections::BTreeMap,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use reqwest::{
    StatusCode,
    header::{CONTENT_LENGTH, CONTENT_TYPE, REFERER, USER_AGENT},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};
use url::Url;

use crate::{
    KugouClient,
    device::{KugouDevice, KugouDeviceIdentity},
    signing::web_signature,
};

const HOST: &str = "login-user.kugou.com";
const QR_PAGE: &str = "https://h5.kugou.com/apps/loginQRCode/html/index.html";
const CREATE_PATH: &str = "/v2/qrcode";
const POLL_PATH: &str = "/v2/get_userinfo_qrcode";
const NATIVE_PASSWORD_CHALLENGE_HOST: &str = "loginservice.kugou.com";
const NATIVE_PASSWORD_CHALLENGE_PATH: &str = "/v2/get_img_code_ex";
const NATIVE_PASSWORD_CHALLENGE_TYPE: &str = "LoginCheckCode";
const NATIVE_APP_ID: u16 = 1005;
const NATIVE_CLIENT_VERSION: u32 = 20489;
const NATIVE_USER_AGENT: &str = "Android15-1070-11083-46-0-DiscoveryDRADProtocol-wifi";
const RESPONSE_LIMIT: usize = 131_072;
const IMAGE_LIMIT: usize = 512 * 1024;
const LOCAL_LIFETIME: Duration = Duration::from_secs(300);
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const ACTIVE: u8 = 0;
const CANCELLED: u8 = 1;
const CONSUMED: u8 = 2;

/// The client receiving a QR authorization. It cannot be changed during polling.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KugouLoginClient {
    Standard,
    Concept,
    Web,
}

/// The challenge mode selected by the official native password-login client.
///
/// `Interactive` requests the native `codetype=3` flow, whose response normally
/// contains a browser verification target: an HTTPS script or a native captcha
/// descriptor. It is returned as data only; this SDK never opens a browser.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KugouNativePasswordChallengeKind {
    Image,
    Interactive,
}

impl KugouNativePasswordChallengeKind {
    fn code_type(self) -> u8 {
        match self {
            Self::Image => 0,
            Self::Interactive => 3,
        }
    }
}

/// A native KuGou password challenge returned by the official image-code
/// endpoint. This is unverified interaction material, never a login session.
/// The verification key and image are intentionally omitted from `Debug`.
#[derive(Clone, Eq, PartialEq)]
pub struct KugouNativePasswordChallenge {
    code_type: u8,
    verify_key: Option<String>,
    image_data_url: Option<String>,
    browser_target: Option<String>,
}

impl fmt::Debug for KugouNativePasswordChallenge {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KugouNativePasswordChallenge")
            .field("code_type", &self.code_type)
            .field("has_verify_key", &self.verify_key.is_some())
            .field("has_image", &self.image_data_url.is_some())
            .field("has_browser_target", &self.browser_target.is_some())
            .finish()
    }
}

impl KugouNativePasswordChallenge {
    #[must_use]
    pub const fn code_type(&self) -> u8 {
        self.code_type
    }

    #[must_use]
    pub fn verify_key(&self) -> Option<&str> {
        self.verify_key.as_deref()
    }

    #[must_use]
    pub fn image_data_url(&self) -> Option<&str> {
        self.image_data_url.as_deref()
    }

    #[must_use]
    pub fn browser_url(&self) -> Option<&str> {
        self.browser_target()
            .filter(|value| Url::parse(value).is_ok_and(|url| url.scheme() == "https"))
    }

    /// The upstream `serpath`, including `KGCodeTX|…` and `KGCodeGT|…`
    /// descriptors. Only URL targets are also exposed by `browser_url()`.
    #[must_use]
    pub fn browser_target(&self) -> Option<&str> {
        self.browser_target.as_deref()
    }
}

impl KugouLoginClient {
    pub(crate) fn appid(self) -> u16 {
        match self {
            Self::Standard => 1005,
            Self::Concept => 3116,
            Self::Web => 1014,
        }
    }

    fn create_appid(self) -> u16 {
        match self {
            Self::Web => 1014,
            Self::Standard | Self::Concept => 1001,
        }
    }

    pub(crate) fn clientver(self) -> u32 {
        match self {
            Self::Standard => 20489,
            Self::Concept => 11440,
            Self::Web => 8131,
        }
    }

    fn parameters(
        self,
        identity: &KugouDeviceIdentity,
        now: Duration,
    ) -> BTreeMap<&'static str, String> {
        let (time, uuid) = if self == Self::Web {
            (now.as_millis().to_string(), identity.mid.clone())
        } else {
            (now.as_secs().to_string(), "-".to_owned())
        };
        BTreeMap::from([
            ("appid", self.appid().to_string()),
            ("clientver", self.clientver().to_string()),
            ("clienttime", time),
            ("srcappid", "2919".to_owned()),
            ("plat", "4".to_owned()),
            ("mid", identity.mid.clone()),
            ("uuid", uuid),
            ("dfid", identity.dfid().to_owned()),
        ])
    }
}

/// An in-memory, cancellable QR transaction. Clones share throttling and consumption.
/// The URL and device are private to this transaction and must not be logged.
#[derive(Clone)]
pub struct KugouQrSession {
    client: KugouClient,
    kind: KugouLoginClient,
    identity: KugouDeviceIdentity,
    key: String,
    url: String,
    deadline: Instant,
    completion: Arc<AtomicU8>,
    state: Arc<Mutex<PollState>>,
}

impl fmt::Debug for KugouQrSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KugouQrSession")
            .field("client", &self.kind)
            .finish_non_exhaustive()
    }
}

struct PollState {
    next_poll: Instant,
    terminal: Option<Terminal>,
}

#[derive(Clone, Copy)]
enum Terminal {
    Expired,
    Consumed,
}

/// Raw upstream progress, before the separate account verification/exchange step.
#[derive(Debug)]
pub enum KugouQrPoll {
    WaitingForScan,
    WaitingForConfirmation,
    Expired,
    AuthorizationReceived(KugouQrAuthorization),
}

/// Unverified authorization material returned once by QR polling.
///
/// This is deliberately not a `ProviderCredential` or an `AccountProfile`.
/// Web authorization additionally requires the official token exchange. Every
/// client requires a same-user authenticated read before establishing a session.
pub struct KugouQrAuthorization {
    kind: KugouLoginClient,
    identity: KugouDeviceIdentity,
    user_id: String,
    token: String,
}

impl fmt::Debug for KugouQrAuthorization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KugouQrAuthorization")
            .field("client", &self.kind)
            .finish_non_exhaustive()
    }
}

impl KugouQrAuthorization {
    #[cfg(test)]
    pub(crate) fn test_authorization(
        kind: KugouLoginClient,
        identity: KugouDeviceIdentity,
        user_id: String,
        token: String,
    ) -> Self {
        Self {
            kind,
            identity,
            user_id,
            token,
        }
    }

    pub(crate) fn into_parts(self) -> (KugouLoginClient, KugouDeviceIdentity, String, String) {
        (self.kind, self.identity, self.user_id, self.token)
    }
    #[must_use]
    pub const fn client_kind(&self) -> KugouLoginClient {
        self.kind
    }
    #[must_use]
    pub fn user_id(&self) -> &str {
        &self.user_id
    }
    /// Secret, unverified token; never include it in logs or documents.
    #[must_use]
    pub fn token(&self) -> &str {
        &self.token
    }
    #[must_use]
    pub fn device_guid(&self) -> &str {
        &self.identity.guid
    }
    #[must_use]
    pub fn device_mid(&self) -> &str {
        &self.identity.mid
    }
    #[must_use]
    pub fn device_dfid(&self) -> Option<&str> {
        self.identity.dfid.as_deref()
    }
}

impl KugouClient {
    /// Fetches one native password challenge without submitting credentials or
    /// retaining a pending login transaction. The returned key/image/URL must
    /// be bound by a caller to a separately verified native login flow.
    pub async fn fetch_native_password_challenge(
        &self,
        kind: KugouNativePasswordChallengeKind,
    ) -> Result<KugouNativePasswordChallenge> {
        self.native_password_challenge_for_version(kind, NATIVE_CLIENT_VERSION)
            .await
    }

    pub(crate) async fn native_password_challenge_for_version(
        &self,
        kind: KugouNativePasswordChallengeKind,
        client_version: u32,
    ) -> Result<KugouNativePasswordChallenge> {
        let clienttime = now()?.as_secs();
        let params = BTreeMap::from([
            ("appid", NATIVE_APP_ID.to_string()),
            ("clienttime", clienttime.to_string()),
            ("clientver", client_version.to_string()),
            ("type", NATIVE_PASSWORD_CHALLENGE_TYPE.to_owned()),
            ("codetype", kind.code_type().to_string()),
            ("client_type", "4".to_owned()),
        ]);
        let endpoint =
            format!("https://{NATIVE_PASSWORD_CHALLENGE_HOST}{NATIVE_PASSWORD_CHALLENGE_PATH}");
        #[cfg(test)]
        let endpoint = self
            .login_test_origin
            .as_ref()
            .map(|origin| {
                origin
                    .join(NATIVE_PASSWORD_CHALLENGE_PATH)
                    .unwrap()
                    .to_string()
            })
            .unwrap_or(endpoint);
        let started = Instant::now();
        let mut status = None;
        let outcome = async {
            let response = self
                .http
                .get(endpoint)
                .header(USER_AGENT, NATIVE_USER_AGENT)
                .header(REFERER, "https://www.kugou.com/")
                .query(&params)
                .send()
                .await
                .map_err(network_error)?;
            status = Some(response.status());
            let bytes = read_response(response).await?;
            parse_native_password_challenge(&bytes, kind.code_type())
        }
        .await;
        self.log_upstream_request(
            "native_password_challenge",
            NATIVE_PASSWORD_CHALLENGE_HOST,
            NATIVE_PASSWORD_CHALLENGE_PATH,
            status,
            started,
            0,
            false,
            &outcome,
        );
        outcome
    }

    /// Creates an isolated low-level QR transaction, without logging in or writing
    /// account/device state to disk. No credentials are sent with this request.
    pub async fn create_login_qr(&self, kind: KugouLoginClient) -> Result<KugouQrSession> {
        let started = Instant::now();
        // A future account inherits this snapshot; anonymous registration/rotation
        // and another login can never alter it.
        let identity = KugouDevice::default().identity();
        let identity = if kind == KugouLoginClient::Web {
            identity.into_web()
        } else {
            identity
        };
        let mut params = kind.parameters(&identity, now()?);
        params.insert("appid", kind.create_appid().to_string());
        params.insert("type", "1".to_owned());
        params.insert("qrcode_txt", format!("{QR_PAGE}?appid={}&", kind.appid()));
        let key = self.qr_get(CREATE_PATH, params, parse_create).await?;
        let deadline = started + LOCAL_LIFETIME;
        if Instant::now() >= deadline {
            return Err(error(
                ErrorCode::UpstreamTimeout,
                "KuGou QR creation exceeded its local lifetime",
            ));
        }
        let url = format!("{QR_PAGE}?appid={}&qrcode={key}", kind.appid());
        Ok(KugouQrSession {
            client: self.clone(),
            kind,
            identity,
            key,
            url,
            deadline,
            completion: Arc::new(AtomicU8::new(ACTIVE)),
            state: Arc::new(Mutex::new(PollState {
                next_poll: Instant::now(),
                terminal: None,
            })),
        })
    }

    async fn qr_get<T>(
        &self,
        path: &'static str,
        mut params: BTreeMap<&str, String>,
        decode: impl FnOnce(&[u8]) -> Result<T>,
    ) -> Result<T> {
        params.insert("signature", web_signature(&params, &[]));
        let endpoint = format!("https://{HOST}{path}");
        #[cfg(test)]
        let endpoint = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(path).unwrap().to_string())
            .unwrap_or(endpoint);
        let started = Instant::now();
        let mut status = None;
        let outcome = async {
            let response = self
                .http
                .get(endpoint)
                .header(REFERER, "https://www.kugou.com/")
                .header("accept", "application/json")
                .query(&params)
                .send()
                .await
                .map_err(network_error)?;
            status = Some(response.status());
            let bytes = read_response(response).await?;
            decode(&bytes)
        }
        .await;
        self.log_upstream_request(
            if path == CREATE_PATH {
                "qr_create"
            } else {
                "qr_poll"
            },
            HOST,
            path,
            status,
            started,
            0,
            false,
            &outcome,
        );
        outcome
    }
}

impl KugouQrSession {
    #[cfg(test)]
    pub(crate) async fn allow_test_poll(&self) {
        self.state.lock().await.next_poll = Instant::now();
    }

    /// Text to encode as a QR image. Upstream image/redirect URLs are never fetched.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }
    #[must_use]
    pub const fn client_kind(&self) -> KugouLoginClient {
        self.kind
    }
    /// Remaining local five-minute transaction budget, not a claimed upstream TTL.
    #[must_use]
    pub fn expires_in(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }
    /// Cancels all clones immediately, including discarding an in-flight response.
    pub fn cancel(&self) {
        let _ =
            self.completion
                .compare_exchange(ACTIVE, CANCELLED, Ordering::SeqCst, Ordering::SeqCst);
    }

    pub async fn poll(&self) -> Result<KugouQrPoll> {
        self.check_cancelled()?;
        let mut state = self.state.try_lock().map_err(|_| rate_limited())?;
        self.check_cancelled()?;
        match state.terminal {
            Some(Terminal::Expired) => return Ok(KugouQrPoll::Expired),
            Some(Terminal::Consumed) => {
                return Err(error(
                    ErrorCode::Conflict,
                    "KuGou QR authorization was already consumed",
                ));
            }
            None => {}
        }
        if Instant::now() >= self.deadline {
            state.terminal = Some(Terminal::Expired);
            return Ok(KugouQrPoll::Expired);
        }
        if Instant::now() < state.next_poll {
            let remaining = state
                .next_poll
                .saturating_duration_since(Instant::now())
                .as_secs()
                .saturating_add(1);
            return Err(rate_limited().with_details(json!({"retry_after_secs": remaining})));
        }
        state.next_poll = Instant::now() + POLL_INTERVAL;
        let mut params = self.kind.parameters(&self.identity, now()?);
        params.insert("qrcode", self.key.clone());
        let parsed = self
            .client
            .qr_get(POLL_PATH, params, |bytes| {
                parse_poll(bytes, &self.key, self.kind, &self.identity)
            })
            .await;
        self.check_cancelled()?;
        if Instant::now() >= self.deadline {
            state.terminal = Some(Terminal::Expired);
            return Ok(KugouQrPoll::Expired);
        }
        let parsed = match parsed {
            Ok(parsed) => parsed,
            Err(error) => {
                if error.code == ErrorCode::RateLimited {
                    let delay = error
                        .details
                        .get("retry_after_secs")
                        .and_then(Value::as_u64)
                        .unwrap_or(POLL_INTERVAL.as_secs())
                        .clamp(POLL_INTERVAL.as_secs(), LOCAL_LIFETIME.as_secs());
                    state.next_poll = Instant::now() + Duration::from_secs(delay);
                }
                return Err(error);
            }
        };
        match &parsed {
            KugouQrPoll::Expired => state.terminal = Some(Terminal::Expired),
            KugouQrPoll::AuthorizationReceived(_) => {
                self.completion
                    .compare_exchange(ACTIVE, CONSUMED, Ordering::SeqCst, Ordering::SeqCst)
                    .map_err(|_| {
                        error(
                            ErrorCode::Conflict,
                            "KuGou QR authorization was cancelled or consumed",
                        )
                    })?;
                state.terminal = Some(Terminal::Consumed);
            }
            _ => {}
        }
        Ok(parsed)
    }

    fn check_cancelled(&self) -> Result<()> {
        if self.completion.load(Ordering::SeqCst) == CANCELLED {
            Err(error(
                ErrorCode::Conflict,
                "KuGou QR transaction was cancelled",
            ))
        } else {
            Ok(())
        }
    }
}

#[derive(Deserialize)]
struct Envelope<T> {
    status: i64,
    error_code: i64,
    data: Option<T>,
}

fn data<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    let response: Envelope<T> = serde_json::from_slice(bytes)
        .map_err(|_| malformed("KuGou QR response is not a valid envelope"))?;
    if response.status != 1 || response.error_code != 0 {
        return Err(malformed("KuGou QR request was rejected")
            .with_details(json!({"platform_code": response.error_code})));
    }
    response
        .data
        .ok_or_else(|| malformed("KuGou QR response omitted its data object"))
}

#[derive(Deserialize)]
struct Created {
    qrcode: String,
}

fn parse_create(bytes: &[u8]) -> Result<String> {
    let response: Created = data(bytes)?;
    if !valid_key(&response.qrcode) {
        return Err(malformed("KuGou QR creation returned an invalid identity"));
    }
    Ok(response.qrcode)
}

#[derive(Deserialize)]
struct Polled {
    status: u8,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    userid: Option<Value>,
    #[serde(default)]
    appid: Option<u16>,
    #[serde(default)]
    qrcode: Option<String>,
}

fn parse_poll(
    bytes: &[u8],
    key: &str,
    kind: KugouLoginClient,
    identity: &KugouDeviceIdentity,
) -> Result<KugouQrPoll> {
    let response: Polled = data(bytes)?;
    if response.appid.is_some_and(|id| id != kind.appid())
        || response.qrcode.as_deref().is_some_and(|id| id != key)
    {
        return Err(malformed(
            "KuGou QR polling returned a different transaction or client",
        ));
    }
    if response.status != 4
        && response
            .token
            .as_ref()
            .is_some_and(|value| !value.is_empty())
    {
        return Err(malformed(
            "KuGou QR polling returned authorization in an inconsistent state",
        ));
    }
    match response.status {
        0 => Ok(KugouQrPoll::Expired),
        1 => Ok(KugouQrPoll::WaitingForScan),
        2 => Ok(KugouQrPoll::WaitingForConfirmation),
        4 => {
            let user_id = user_id(response.userid.as_ref())?;
            let token = response
                .token
                .filter(|v| {
                    !v.is_empty()
                        && v.len() <= 16384
                        && !matches!(v.as_str(), "null" | "undefined")
                        && v.bytes().all(|b| (0x21..=0x7e).contains(&b))
                })
                .ok_or_else(|| malformed("KuGou QR authorization omitted a valid token"))?;
            Ok(KugouQrPoll::AuthorizationReceived(KugouQrAuthorization {
                kind,
                identity: identity.clone(),
                user_id,
                token,
            }))
        }
        _ => Err(malformed("KuGou QR polling returned an unknown state")),
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct NativePasswordChallengeEnvelope {
    status: i64,
    data: Option<NativePasswordChallengeData>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct NativePasswordChallengeData {
    verifykey: Option<String>,
    verifycode: Option<String>,
    serpath: Option<String>,
}

fn parse_native_password_challenge(
    bytes: &[u8],
    code_type: u8,
) -> Result<KugouNativePasswordChallenge> {
    let response: NativePasswordChallengeEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| malformed("KuGou native password challenge is not valid JSON"))?;
    if response.status == 0 {
        return Err(malformed("KuGou native password challenge was rejected")
            .with_details(json!({"platform_code": response.status})));
    }
    let data = response
        .data
        .ok_or_else(|| malformed("KuGou native password challenge omitted its data object"))?;
    let verify_key = data
        .verifykey
        .filter(|value| !value.is_empty())
        .map(|value| {
            if valid_opaque_text(&value) {
                Ok(value)
            } else {
                Err(malformed(
                    "KuGou native password challenge returned an invalid verify key",
                ))
            }
        })
        .transpose()?;
    let image_data_url = data
        .verifycode
        .filter(|value| !value.is_empty())
        .map(|encoded| {
            let bytes = BASE64.decode(encoded.as_bytes()).map_err(|_| {
                malformed("KuGou native password challenge image is not valid Base64")
            })?;
            if bytes.is_empty() || bytes.len() > IMAGE_LIMIT {
                return Err(malformed(
                    "KuGou native password challenge image has an invalid size",
                ));
            }
            image_data_url_for(&bytes)
        })
        .transpose()?;
    let browser_target = data
        .serpath
        .filter(|value| !value.is_empty())
        .map(|value| native_browser::validate_target(&value).map(|()| value))
        .transpose()?;
    if verify_key.is_none() && image_data_url.is_none() && browser_target.is_none() {
        return Err(malformed(
            "KuGou native password challenge omitted verification material",
        ));
    }
    Ok(KugouNativePasswordChallenge {
        code_type,
        verify_key,
        image_data_url,
        browser_target,
    })
}

fn valid_opaque_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 1024
        && value.bytes().all(|byte| (0x21..=0x7e).contains(&byte))
}

fn image_data_url_for(bytes: &[u8]) -> Result<String> {
    let mime = if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        "image/jpeg"
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else if bytes.starts_with(b"BM") {
        "image/bmp"
    } else {
        return Err(malformed(
            "KuGou native password challenge image has an unsupported format",
        ));
    };
    Ok(format!("data:{mime};base64,{}", BASE64.encode(bytes)))
}

fn user_id(value: Option<&Value>) -> Result<String> {
    let id = match value {
        Some(Value::String(v)) => v.clone(),
        Some(Value::Number(n)) => n.as_u64().map(|n| n.to_string()).unwrap_or_default(),
        _ => String::new(),
    };
    if id.starts_with('0') || !id.bytes().all(|b| b.is_ascii_digit()) || id.parse::<u64>().is_err()
    {
        return Err(malformed(
            "KuGou QR authorization omitted a valid user identity",
        ));
    }
    Ok(id)
}

fn valid_key(key: &str) -> bool {
    // Current QR keys are 36 alphanumeric bytes, not UUIDs. Treat the identity
    // as bounded opaque text; do not infer GUID segments or rewrite its case.
    (16..=128).contains(&key.len())
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

async fn read_response(mut response: reqwest::Response) -> Result<Vec<u8>> {
    let status = response.status();
    if !status.is_success() {
        let delay = response
            .headers()
            .get("retry-after")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(POLL_INTERVAL.as_secs())
            .clamp(POLL_INTERVAL.as_secs(), LOCAL_LIFETIME.as_secs());
        let details = if status == StatusCode::TOO_MANY_REQUESTS {
            json!({"http_status": status.as_u16(), "retry_after_secs": delay})
        } else {
            json!({"http_status": status.as_u16()})
        };
        return Err(error(
            if status == StatusCode::TOO_MANY_REQUESTS {
                ErrorCode::RateLimited
            } else {
                ErrorCode::UpstreamError
            },
            "KuGou QR HTTP request failed",
        )
        .with_details(details));
    }
    let json_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(';').next())
        .is_some_and(|s| s.trim().eq_ignore_ascii_case("application/json"));
    if !json_type {
        return Err(malformed(
            "KuGou QR response has an unexpected content type",
        ));
    }
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|n| n > RESPONSE_LIMIT as u64)
    {
        return Err(malformed("KuGou QR response exceeded its size limit"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if bytes.len().saturating_add(chunk.len()) > RESPONSE_LIMIT {
            return Err(malformed("KuGou QR response exceeded its size limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn now() -> Result<Duration> {
    SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
        error(
            ErrorCode::InternalError,
            "KuGou QR requires a valid system clock",
        )
    })
}
fn error(code: ErrorCode, message: &'static str) -> TuneWeaveError {
    TuneWeaveError::new(code, message).with_platform(Platform::Kugou)
}
fn malformed(message: &'static str) -> TuneWeaveError {
    error(ErrorCode::UpstreamError, message)
}
fn network_error(e: reqwest::Error) -> TuneWeaveError {
    error(
        if e.is_timeout() {
            ErrorCode::UpstreamTimeout
        } else {
            ErrorCode::UpstreamError
        },
        "KuGou QR transport failed",
    )
}
fn rate_limited() -> TuneWeaveError {
    error(
        ErrorCode::RateLimited,
        "KuGou QR polling must wait before the next request",
    )
    .with_details(json!({"retry_after_secs": POLL_INTERVAL.as_secs()}))
}

#[cfg(test)]
mod tests;

pub(crate) mod crypto;
pub(crate) mod native_browser;
pub(crate) mod native_password;
pub(crate) mod native_secondary;
