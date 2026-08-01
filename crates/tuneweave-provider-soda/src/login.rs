use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use qrcode::{QrCode, render::svg};
use reqwest::header::{ACCEPT, CONTENT_TYPE, COOKIE, HeaderMap, SET_COOKIE, USER_AGENT};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};
use url::{Url, form_urlencoded};

use crate::client::{
    SodaClient, read_bounded_response, soda_http_error, soda_network_error, soda_upstream_error,
    unix_rfc3339,
};
use crate::device::SodaDeviceState;

const QR_CREATE_ENDPOINT: &str = "https://api.qishui.com/passport/web/get_qrcode/";
const QR_POLL_ENDPOINT: &str = "https://api.qishui.com/passport/web/check_qrconnect/";
const PASSPORT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) SodaMusic/3.1.0 Chrome/136.0.7103.59 Electron/36.4.0-rs.22.release.main.1 TTElectron/36.4.0-rs.22.release.main.1 Safari/537.36";
const PASSPORT_APP_ID: &str = "386088";
const PASSPORT_JSSDK_VERSION: &str = "2.4.13";
const PASSPORT_VERSION_CODE: &str = "3.3.0";
const PASSPORT_PZT: &str = "3.3.5";
const PASSPORT_P_VERSION: &str = "1.0.29";
const PASSPORT_BUILD: &str = "1.0.0.41";
const QR_TRANSACTION_LIFETIME: Duration = Duration::from_secs(5 * 60);
const QR_POLL_MIN_INTERVAL: Duration = Duration::from_secs(2);
const QR_RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(60);
const MAX_QR_TRANSACTIONS: usize = 128;
const MAX_QR_IMAGE_BYTES: usize = 1024 * 1024;
const MAX_LOGIN_COOKIES: usize = 64;
const MAX_COOKIE_NAME_BYTES: usize = 128;
const MAX_COOKIE_VALUE_BYTES: usize = 4 * 1024;
const MAX_COOKIE_TOTAL_BYTES: usize = 32 * 1024;
const SODA_CREDENTIAL_VERSION: u8 = 1;

#[derive(Clone)]
pub(crate) struct SodaQrStart {
    pub provider_transaction_id: String,
    pub image_data_url: String,
    pub expires_at: Option<String>,
}

impl fmt::Debug for SodaQrStart {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SodaQrStart")
            .field("provider_transaction_id", &"[redacted]")
            .field("has_image_data_url", &true)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SodaQrPollOutcome {
    Waiting,
    Scanned,
    AdditionalVerificationRequired,
    Expired,
    Failed { code: i64 },
    Confirmed(SodaCredential),
}

#[derive(Clone, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SodaCredential {
    version: u8,
    cookies: BTreeMap<String, String>,
}

impl fmt::Debug for SodaCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SodaCredential")
            .field("version", &self.version)
            .field("cookie_count", &self.cookies.len())
            .field("has_session", &self.has_session())
            .finish()
    }
}

impl SodaCredential {
    fn from_cookies(cookies: BTreeMap<String, String>) -> Result<Self> {
        Self {
            version: SODA_CREDENTIAL_VERSION,
            cookies,
        }
        .validate()
    }

    pub(crate) fn parse(secret: &str) -> Result<Self> {
        serde_json::from_str::<Self>(secret)
            .map_err(|_| soda_credential_error("credential is malformed"))?
            .validate()
    }

    pub(crate) fn serialize(&self) -> Result<String> {
        serde_json::to_string(self)
            .map_err(|_| soda_credential_error("credential could not be encoded"))
    }

    fn validate(self) -> Result<Self> {
        if self.version != SODA_CREDENTIAL_VERSION {
            return Err(soda_credential_error(
                "credential uses an unsupported version",
            ));
        }
        validate_cookie_jar(&self.cookies)?;
        if !self.has_session() {
            return Err(soda_credential_error(
                "credential does not contain an authenticated session",
            ));
        }
        Ok(self)
    }

    fn has_session(&self) -> bool {
        has_session_cookie(&self.cookies)
    }
}

#[derive(Clone, Default)]
pub(crate) struct SodaQrTransactions {
    entries: Arc<Mutex<BTreeMap<String, SodaQrEntry>>>,
}

impl fmt::Debug for SodaQrTransactions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("SodaQrTransactions").finish()
    }
}

#[derive(Clone)]
struct SodaQrEntry {
    expires_at: Instant,
    state: Arc<AsyncMutex<SodaQrTransaction>>,
}

struct SodaQrTransaction {
    upstream_token: String,
    trace_id: String,
    device: SodaDeviceState,
    cookies: BTreeMap<String, String>,
    last_upstream_poll: Option<Instant>,
    cooldown_until: Option<Instant>,
    last_outcome: SodaQrPollOutcome,
    terminal: Option<SodaQrPollOutcome>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct QrCreateEnvelope {
    data: QrCreateData,
    message: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct QrCreateData {
    token: String,
    qrcode: String,
    qrcode_index_url: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct QrPollEnvelope {
    data: QrPollData,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct QrPollData {
    status: String,
    error_code: i64,
    account_flow: String,
}

impl SodaQrTransactions {
    pub(crate) async fn start(&self, client: &SodaClient) -> Result<SodaQrStart> {
        let device = client.login_device()?;
        let trace_id = random_hex(16);
        let endpoint = passport_endpoint(QR_CREATE_ENDPOINT, &device, &trace_id, true)?;
        let started = Instant::now();
        let mut http_status = None;
        let outcome = async {
            let response = client
                .login_http()
                .get(endpoint)
                .header(USER_AGENT, PASSPORT_USER_AGENT)
                .header(ACCEPT, "application/json, text/javascript")
                .send()
                .await
                .map_err(soda_network_error)?;
            http_status = Some(response.status());
            let headers = response.headers().clone();
            let body = read_bounded_response(response, "Soda QR creation").await?;
            let created = parse_qr_create_response(&body)?;
            let mut cookies = BTreeMap::new();
            merge_response_cookies(&mut cookies, &headers)?;
            let image_data_url = qr_image_data_url(
                &created.data.qrcode,
                &created.data.qrcode_index_url,
                &created.data.token,
            )?;
            let provider_transaction_id = self.insert(SodaQrTransaction {
                upstream_token: created.data.token,
                trace_id,
                device,
                cookies,
                last_upstream_poll: None,
                cooldown_until: None,
                last_outcome: SodaQrPollOutcome::Waiting,
                terminal: None,
            })?;
            let expires_at =
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .ok()
                    .and_then(|duration| {
                        unix_rfc3339(
                            duration
                                .as_secs()
                                .saturating_add(QR_TRANSACTION_LIFETIME.as_secs()),
                        )
                    });
            Ok(SodaQrStart {
                provider_transaction_id,
                image_data_url,
                expires_at,
            })
        }
        .await;
        client.log_upstream_request(
            "qr_login_start",
            "api.qishui.com",
            "/passport/web/get_qrcode/",
            http_status,
            started,
            &outcome,
        );
        outcome
    }

    pub(crate) async fn poll(
        &self,
        client: &SodaClient,
        provider_transaction_id: &str,
    ) -> Result<SodaQrPollOutcome> {
        validate_transaction_id(provider_transaction_id)?;
        let entry = self.entry(provider_transaction_id)?;
        if Instant::now() >= entry.expires_at {
            self.remove(provider_transaction_id)?;
            return Ok(SodaQrPollOutcome::Expired);
        }
        let mut transaction = entry.state.lock().await;
        if let Some(terminal) = &transaction.terminal {
            return Ok(terminal.clone());
        }
        let now = Instant::now();
        if transaction
            .cooldown_until
            .is_some_and(|cooldown_until| now < cooldown_until)
            || transaction
                .last_upstream_poll
                .is_some_and(|last_poll| now.duration_since(last_poll) < QR_POLL_MIN_INTERVAL)
        {
            return Ok(transaction.last_outcome.clone());
        }
        transaction.last_upstream_poll = Some(now);

        let endpoint = passport_endpoint(
            QR_POLL_ENDPOINT,
            &transaction.device,
            &transaction.trace_id,
            false,
        )?;
        let body = qr_poll_form(&transaction.upstream_token);
        let cookie = cookie_header(&transaction.cookies)?;
        let started = Instant::now();
        let mut http_status = None;
        let outcome = async {
            let mut request = client
                .login_http()
                .post(endpoint)
                .header(USER_AGENT, PASSPORT_USER_AGENT)
                .header(ACCEPT, "application/json, text/javascript")
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("sec-ch-ua", "\"Not.A/Brand\";v=\"99\", \"Chromium\";v=\"136\"")
                .header("sec-ch-ua-mobile", "?0")
                .header("sec-ch-ua-platform", "\"Windows\"")
                .header("bd-ticket-guard-version", "2")
                .header("bd-ticket-guard-iteration-version", "2")
                .header(
                    "bd-ticket-guard-ree-public-key",
                    "BAnIxKL96Jby5x+Um9i7HZ2c8O6lfZJRxm6yk73Mqcr06l2qIw2iqu2Mtm3U/6OI98usukA9dqxUlsctVWK9rKA=",
                )
                .header("bd-ticket-guard-server-cert-sn", "0")
                .body(body);
            if !cookie.is_empty() {
                request = request.header(COOKIE, cookie);
            }
            let response = request.send().await.map_err(soda_network_error)?;
            http_status = Some(response.status());
            if !response.status().is_success() {
                return Err(soda_http_error(response.status()));
            }
            let headers = response.headers().clone();
            let body = read_bounded_response(response, "Soda QR polling").await?;
            merge_response_cookies(&mut transaction.cookies, &headers)?;
            if has_session_cookie(&transaction.cookies) {
                return SodaCredential::from_cookies(transaction.cookies.clone())
                    .map(SodaQrPollOutcome::Confirmed);
            }
            parse_qr_poll_response(&body)
        }
        .await;
        client.log_upstream_request(
            "qr_login_poll",
            "api.qishui.com",
            "/passport/web/check_qrconnect/",
            http_status,
            started,
            &outcome,
        );
        let outcome = outcome?;
        if matches!(outcome, SodaQrPollOutcome::Failed { code: 7 }) {
            transaction.cooldown_until = Some(Instant::now() + QR_RATE_LIMIT_COOLDOWN);
            return Ok(transaction.last_outcome.clone());
        }
        transaction.last_outcome = outcome.clone();
        if matches!(
            outcome,
            SodaQrPollOutcome::Confirmed(_)
                | SodaQrPollOutcome::Expired
                | SodaQrPollOutcome::Failed { .. }
        ) {
            transaction.terminal = Some(outcome.clone());
        }
        Ok(outcome)
    }

    fn insert(&self, transaction: SodaQrTransaction) -> Result<String> {
        let mut entries = self.entries.lock().map_err(|_| qr_store_error())?;
        entries.retain(|_, entry| Instant::now() < entry.expires_at);
        if entries.len() >= MAX_QR_TRANSACTIONS {
            return Err(TuneWeaveError::new(
                ErrorCode::RateLimited,
                "Soda QR login transaction capacity has been reached",
            )
            .with_platform(Platform::Soda)
            .retryable(true));
        }
        let transaction_id = (0..8)
            .map(|_| random_hex(32))
            .find(|transaction_id| !entries.contains_key(transaction_id))
            .ok_or_else(qr_store_error)?;
        entries.insert(
            transaction_id.clone(),
            SodaQrEntry {
                expires_at: Instant::now() + QR_TRANSACTION_LIFETIME,
                state: Arc::new(AsyncMutex::new(transaction)),
            },
        );
        Ok(transaction_id)
    }

    fn entry(&self, transaction_id: &str) -> Result<SodaQrEntry> {
        self.entries
            .lock()
            .map_err(|_| qr_store_error())?
            .get(transaction_id)
            .cloned()
            .ok_or_else(|| {
                TuneWeaveError::invalid_request(
                    "Soda QR login transaction was not found or has expired",
                )
                .with_platform(Platform::Soda)
            })
    }

    fn remove(&self, transaction_id: &str) -> Result<()> {
        self.entries
            .lock()
            .map_err(|_| qr_store_error())?
            .remove(transaction_id);
        Ok(())
    }
}

fn passport_endpoint(
    endpoint: &str,
    device: &SodaDeviceState,
    trace_id: &str,
    create: bool,
) -> Result<Url> {
    let mut endpoint = Url::parse(endpoint)
        .map_err(|_| soda_credential_error("internal passport endpoint is invalid"))?;
    {
        let mut query = endpoint.query_pairs_mut();
        query
            .append_pair("passport_jssdk_version", PASSPORT_JSSDK_VERSION)
            .append_pair("passport_jssdk_type", "normal")
            .append_pair("is_from_ttaccountsdk", "1")
            .append_pair("aid", PASSPORT_APP_ID)
            .append_pair("language", "zh")
            .append_pair("account_sdk_source", "web")
            .append_pair("p_js_v", PASSPORT_JSSDK_VERSION)
            .append_pair("p_js_t", "pro")
            .append_pair("p_zt", PASSPORT_PZT)
            .append_pair("p_ver", PASSPORT_P_VERSION)
            .append_pair("request_host", "app%3A%2F%2Fresources")
            .append_pair("p_bd", PASSPORT_BUILD)
            .append_pair("biz_trace_id", trace_id)
            .append_pair("is_new_login", "1")
            .append_pair("is_from_iesaccountsaas", "1")
            .append_pair("device_id", &device.device_id)
            .append_pair("install_id", &device.install_id)
            .append_pair("did", &device.device_id)
            .append_pair("iid", &device.install_id)
            .append_pair("device_platform", "PC")
            .append_pair("version_code", PASSPORT_VERSION_CODE);
        if create {
            query
                .append_pair("next", "https://api.qishui.com")
                .append_pair("need_logo", "false")
                .append_pair("need_short_url", "false")
                .append_pair("is_frontier", "true");
        }
    }
    Ok(endpoint)
}

fn qr_poll_form(token: &str) -> String {
    form_urlencoded::Serializer::new(String::new())
        .append_pair("need_logo", "false")
        .append_pair("need_short_url", "false")
        .append_pair("is_frontier", "true")
        .append_pair("token", token)
        .append_pair("is_new_login", "1")
        .append_pair("next", "https://api.qishui.com")
        .finish()
}

fn parse_qr_create_response(bytes: &[u8]) -> Result<QrCreateEnvelope> {
    let response = serde_json::from_slice::<QrCreateEnvelope>(bytes)
        .map_err(|_| soda_upstream_error("Soda QR creation returned invalid JSON"))?;
    validate_upstream_token(&response.data.token)?;
    if !response.message.trim().is_empty()
        && !response.message.trim().eq_ignore_ascii_case("success")
    {
        return Err(soda_upstream_error("Soda QR creation was rejected"));
    }
    Ok(response)
}

fn parse_qr_poll_response(bytes: &[u8]) -> Result<SodaQrPollOutcome> {
    let response = serde_json::from_slice::<QrPollEnvelope>(bytes)
        .map_err(|_| soda_upstream_error("Soda QR polling returned invalid JSON"))?;
    let status = response.data.status.trim().to_ascii_lowercase();
    let account_flow = response.data.account_flow.trim().to_ascii_lowercase();
    if account_flow == "verify" || response.data.error_code == 2046 {
        return Ok(SodaQrPollOutcome::AdditionalVerificationRequired);
    }
    match status.as_str() {
        "new" | "" if response.data.error_code == 0 => Ok(SodaQrPollOutcome::Waiting),
        "scanned" | "confirmed" if response.data.error_code == 0 => Ok(SodaQrPollOutcome::Scanned),
        "expired" => Ok(SodaQrPollOutcome::Expired),
        "error" | "failed" => Ok(SodaQrPollOutcome::Failed {
            code: response.data.error_code,
        }),
        _ if response.data.error_code != 0 => Ok(SodaQrPollOutcome::Failed {
            code: response.data.error_code,
        }),
        _ => Ok(SodaQrPollOutcome::Waiting),
    }
}

fn qr_image_data_url(raw: &str, index_url: &str, token: &str) -> Result<String> {
    let raw = raw.trim();
    if !raw.is_empty() {
        let encoded = raw.strip_prefix("data:image/png;base64,").unwrap_or(raw);
        if encoded.starts_with("data:") || encoded.len() > MAX_QR_IMAGE_BYTES.saturating_mul(2) {
            return Err(soda_upstream_error(
                "Soda QR creation returned an unsupported image",
            ));
        }
        let decoded = STANDARD
            .decode(encoded)
            .map_err(|_| soda_upstream_error("Soda QR creation returned invalid image data"))?;
        if decoded.len() > MAX_QR_IMAGE_BYTES || !decoded.starts_with(b"\x89PNG\r\n\x1a\n") {
            return Err(soda_upstream_error(
                "Soda QR creation returned a non-PNG image",
            ));
        }
        return Ok(format!(
            "data:image/png;base64,{}",
            STANDARD.encode(decoded)
        ));
    }

    let index_url = validate_qr_index_url(index_url, token)?;
    let image = QrCode::new(index_url.as_str().as_bytes())
        .map_err(|_| soda_upstream_error("Soda QR login URL could not be encoded"))?
        .render::<svg::Color>()
        .min_dimensions(320, 320)
        .build();
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        STANDARD.encode(image.as_bytes())
    ))
}

fn validate_qr_index_url(raw: &str, token: &str) -> Result<Url> {
    let url = Url::parse(raw)
        .map_err(|_| soda_upstream_error("Soda QR creation omitted a usable image"))?;
    let trusted = url.scheme() == "https"
        && url.host_str() == Some("bff-pc.qishui.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.path() == "/ucenter_web/app/sdk-next"
        && url.fragment().is_none()
        && url
            .query_pairs()
            .any(|(key, value)| key == "token" && value == token);
    if !trusted {
        return Err(soda_upstream_error(
            "Soda QR creation returned an untrusted login URL",
        ));
    }
    Ok(url)
}

fn merge_response_cookies(
    cookies: &mut BTreeMap<String, String>,
    headers: &HeaderMap,
) -> Result<()> {
    for header in headers.get_all(SET_COOKIE) {
        let header = header
            .to_str()
            .map_err(|_| soda_upstream_error("Soda login returned an invalid cookie header"))?;
        let pair = header.split(';').next().unwrap_or_default();
        let Some((name, value)) = pair.split_once('=') else {
            return Err(soda_upstream_error(
                "Soda login returned a malformed cookie header",
            ));
        };
        let name = name.trim();
        let value = value.trim();
        validate_cookie_pair(name, value)?;
        if value.is_empty() {
            cookies.remove(name);
        } else {
            cookies.insert(name.to_owned(), value.to_owned());
        }
    }
    validate_cookie_jar(cookies)
}

fn validate_cookie_jar(cookies: &BTreeMap<String, String>) -> Result<()> {
    if cookies.len() > MAX_LOGIN_COOKIES {
        return Err(soda_credential_error(
            "credential contains too many cookies",
        ));
    }
    let mut total = 0_usize;
    for (name, value) in cookies {
        validate_cookie_pair(name, value)?;
        total = total.saturating_add(name.len()).saturating_add(value.len());
    }
    if total > MAX_COOKIE_TOTAL_BYTES {
        return Err(soda_credential_error(
            "credential cookie data exceeds the size limit",
        ));
    }
    Ok(())
}

fn validate_cookie_pair(name: &str, value: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > MAX_COOKIE_NAME_BYTES
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        || value.len() > MAX_COOKIE_VALUE_BYTES
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || matches!(byte, b';' | b','))
    {
        return Err(soda_credential_error(
            "credential contains an invalid cookie",
        ));
    }
    Ok(())
}

fn cookie_header(cookies: &BTreeMap<String, String>) -> Result<String> {
    validate_cookie_jar(cookies)?;
    Ok(cookies
        .iter()
        .map(|(name, value)| format!("{name}={value}"))
        .collect::<Vec<_>>()
        .join("; "))
}

fn has_session_cookie(cookies: &BTreeMap<String, String>) -> bool {
    ["sessionid", "sessionid_ss", "sid_tt", "sid_guard"]
        .iter()
        .any(|name| cookies.get(*name).is_some_and(|value| !value.is_empty()))
}

fn validate_upstream_token(token: &str) -> Result<()> {
    if !(16..=256).contains(&token.len())
        || !token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
    {
        return Err(soda_upstream_error(
            "Soda QR creation returned an invalid token",
        ));
    }
    Ok(())
}

fn validate_transaction_id(transaction_id: &str) -> Result<()> {
    if transaction_id.len() != 64 || !transaction_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(
            TuneWeaveError::invalid_request("Soda QR login transaction ID is invalid")
                .with_platform(Platform::Soda),
        );
    }
    Ok(())
}

fn random_hex(bytes: usize) -> String {
    (0..bytes)
        .map(|_| format!("{:02x}", rand::random::<u8>()))
        .collect()
}

fn qr_store_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        "Soda QR login transaction storage failed",
    )
    .with_platform(Platform::Soda)
}

fn soda_credential_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::InternalError, message).with_platform(Platform::Soda)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_cookies() -> BTreeMap<String, String> {
        BTreeMap::from([
            ("passport_csrf_token".to_owned(), "csrf-value".to_owned()),
            ("sessionid_ss".to_owned(), "session-secret".to_owned()),
        ])
    }

    #[test]
    fn credentials_round_trip_without_debug_secret_exposure() {
        let credential = SodaCredential::from_cookies(session_cookies()).expect("credential");
        let encoded = credential.serialize().expect("serialize credential");
        assert_eq!(SodaCredential::parse(&encoded).expect("parse"), credential);
        let debug = format!("{credential:?}");
        assert!(debug.contains("has_session: true"));
        assert!(!debug.contains("session-secret"));
    }

    #[test]
    fn credentials_reject_missing_sessions_and_cookie_injection() {
        assert!(
            SodaCredential::from_cookies(BTreeMap::from([(
                "passport_csrf_token".to_owned(),
                "csrf-value".to_owned(),
            )]))
            .is_err()
        );
        assert!(
            SodaCredential::from_cookies(BTreeMap::from([(
                "sessionid".to_owned(),
                "secret; injected=value".to_owned(),
            )]))
            .is_err()
        );
    }

    #[test]
    fn create_and_poll_responses_preserve_all_observed_states() {
        let created = parse_qr_create_response(
            br#"{"message":"success","data":{"token":"01234567890123456789012345678901234","qrcode":"","qrcode_index_url":""}}"#,
        )
        .expect("parse create response");
        assert_eq!(created.data.token.len(), 35);
        assert_eq!(
            parse_qr_poll_response(br#"{"data":{"status":"new","error_code":0}}"#)
                .expect("waiting"),
            SodaQrPollOutcome::Waiting
        );
        assert_eq!(
            parse_qr_poll_response(br#"{"data":{"status":"scanned","error_code":0}}"#)
                .expect("scanned"),
            SodaQrPollOutcome::Scanned
        );
        assert_eq!(
            parse_qr_poll_response(
                br#"{"data":{"status":"","error_code":2046,"account_flow":"verify"}}"#,
            )
            .expect("mfa"),
            SodaQrPollOutcome::AdditionalVerificationRequired
        );
        assert_eq!(
            parse_qr_poll_response(br#"{"data":{"status":"expired","error_code":0}}"#)
                .expect("expired"),
            SodaQrPollOutcome::Expired
        );
        assert_eq!(
            parse_qr_poll_response(br#"{"data":{"status":"new","error_code":7}}"#)
                .expect("limited"),
            SodaQrPollOutcome::Failed { code: 7 }
        );
    }

    #[test]
    fn qr_images_accept_bounded_png_and_reject_untrusted_fallbacks() {
        let png = b"\x89PNG\r\n\x1a\nsmall-test-payload";
        let data =
            qr_image_data_url(&STANDARD.encode(png), "", "unused").expect("normalize QR image");
        assert!(data.starts_with("data:image/png;base64,"));
        assert!(
            qr_image_data_url(
                "",
                "https://example.test/ucenter_web/app/sdk-next?token=secret",
                "secret",
            )
            .is_err()
        );
    }

    #[test]
    fn passport_requests_use_persistent_ids_without_signature_placeholders() {
        let device = SodaDeviceState {
            schema_version: 1,
            device_id: "1234567890123456789".to_owned(),
            install_id: "2234567890123456789".to_owned(),
            created_at_ms: 1,
        };
        let endpoint = passport_endpoint(QR_CREATE_ENDPOINT, &device, "0123456789abcdef", true)
            .expect("passport endpoint");
        let query = endpoint.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(
            query.get("device_id").map(|value| value.as_ref()),
            Some("1234567890123456789")
        );
        assert_eq!(
            query.get("install_id").map(|value| value.as_ref()),
            Some("2234567890123456789")
        );
        assert_eq!(
            query.get("aid").map(|value| value.as_ref()),
            Some(PASSPORT_APP_ID)
        );
        assert!(!query.contains_key("msToken"));
        assert!(!query.contains_key("a_bogus"));
    }

    #[tokio::test]
    async fn transaction_ids_hide_upstream_tokens_and_throttle_local_polls() {
        let store = SodaQrTransactions::default();
        let upstream_token = "0123456789abcdefghijklmnopqrstuvw".to_owned();
        let transaction_id = store
            .insert(SodaQrTransaction {
                upstream_token: upstream_token.clone(),
                trace_id: "0123456789abcdef".to_owned(),
                device: SodaDeviceState {
                    schema_version: 1,
                    device_id: "1234567890123456789".to_owned(),
                    install_id: "2234567890123456789".to_owned(),
                    created_at_ms: 1,
                },
                cookies: BTreeMap::new(),
                last_upstream_poll: Some(Instant::now()),
                cooldown_until: None,
                last_outcome: SodaQrPollOutcome::Scanned,
                terminal: None,
            })
            .expect("insert transaction");
        assert_eq!(transaction_id.len(), 64);
        assert!(!transaction_id.contains(&upstream_token));
        let client = SodaClient::test_client();
        assert_eq!(
            store
                .poll(&client, &transaction_id)
                .await
                .expect("cached poll"),
            SodaQrPollOutcome::Scanned
        );
    }
}
