use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use md5::{Digest, Md5};
use qrcode::{QrCode, render::svg};
use reqwest::header::{ACCEPT, CONTENT_TYPE, COOKIE, HeaderMap, SET_COOKIE, USER_AGENT};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex as AsyncMutex;
use tuneweave_core::{
    CredentialMode, ErrorCode, Platform, QrVerification, QrVerificationAction, Result,
    TuneWeaveError,
};
use url::{Url, form_urlencoded};

use crate::client::{
    SodaClient, read_bounded_response, soda_http_error, soda_upstream_error, unix_rfc3339,
};
use crate::device::SodaDeviceState;
use crate::mfa::SodaMfa;

const QR_CREATE_ENDPOINT: &str = "https://api.qishui.com/passport/web/get_qrcode/";
const QR_POLL_ENDPOINT: &str = "https://api.qishui.com/passport/web/check_qrconnect/";
const TOKEN_BEAT_ENDPOINT: &str = "https://api.qishui.com/passport/token/beat/web/";
const PASSPORT_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) SodaMusic/3.7.0 Chrome/136.0.7103.59 Electron/36.4.0-rs.29.release.main.0 TTElectron/36.4.0-rs.29.release.main.0 Safari/537.36";
const PASSPORT_APP_ID: &str = "386088";
const PASSPORT_JSSDK_VERSION: &str = "2.4.13";
const PASSPORT_PBD_VERSION: &str = "1.0.0.41";
const PASSPORT_MFA_JSSDK_VERSION: &str = "5.1.2";
const PASSPORT_NEW_AUTHN_SDK_VERSION: &str = "1.0.0.428-web";
const PASSPORT_ACCOUNT_SDK_VERSION: &str = "1.2.14";
const PASSPORT_VERSION_CODE: &str = "3.7.0";
const PASSPORT_PZT: &str = "3.3.5";
const PASSPORT_P_VERSION: &str = "1.0.29";
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
    generation: String,
    cookies: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_id: Option<String>,
    #[serde(skip)]
    passport_context: Option<SodaPassportContext>,
}

#[derive(Clone, Eq, PartialEq)]
struct SodaPassportContext {
    account_sdk_source_info: String,
    trace_id: String,
    verify_portrait_id: String,
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
    pub(crate) fn import_cookie_header(raw: &str) -> Result<Self> {
        if raw.is_empty()
            || raw.len() > MAX_COOKIE_TOTAL_BYTES
            || !raw.is_ascii()
            || raw.bytes().any(|b| b.is_ascii_control())
        {
            return Err(soda_credential_error(
                "imported Cookie data is invalid or too large",
            ));
        }
        let mut cookies = BTreeMap::new();
        for pair in raw.split(';') {
            let (name, value) = pair
                .trim()
                .split_once('=')
                .ok_or_else(|| soda_credential_error("import expects Cookie name=value pairs"))?;
            if name != name.trim()
                || value.is_empty()
                || value != value.trim()
                || name.starts_with("passport_mfa_")
                || ["domain", "path", "expires", "max-age", "samesite"]
                    .iter()
                    .any(|attr| name.eq_ignore_ascii_case(attr))
            {
                return Err(soda_credential_error(
                    "imported Cookie data contains invalid or temporary fields",
                ));
            }
            validate_cookie_pair(name, value)?;
            if cookies.insert(name.to_owned(), value.to_owned()).is_some() {
                return Err(soda_credential_error(
                    "imported Cookie data contains duplicate names",
                ));
            }
        }
        Self::from_cookies(cookies)
    }

    pub(crate) fn same_login(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.user_id.is_some()
            && self.user_id == other.user_id
    }

    fn from_cookies(mut cookies: BTreeMap<String, String>) -> Result<Self> {
        cookies.retain(|name, _| !name.starts_with("passport_mfa_"));
        Self {
            version: SODA_CREDENTIAL_VERSION,
            generation: random_hex(32),
            cookies,
            user_id: None,
            passport_context: None,
        }
        .validate()
    }

    pub(crate) fn with_passport_context(
        mut self,
        account_sdk_source_info: &str,
        trace_id: &str,
        verify_portrait_id: &str,
    ) -> Result<Self> {
        if account_sdk_source_info.len() > MAX_SODA_CLIENT_CONTEXT_BYTES * 3
            || !account_sdk_source_info
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || trace_id.len() != 8
            || !trace_id.bytes().all(|byte| byte.is_ascii_hexdigit())
            || verify_portrait_id.len() != 42
            || !verify_portrait_id.ends_with(".login")
        {
            return Err(soda_credential_error(
                "Soda Passport continuation context is invalid",
            ));
        }
        self.passport_context = Some(SodaPassportContext {
            account_sdk_source_info: account_sdk_source_info.to_owned(),
            trace_id: trace_id.to_owned(),
            verify_portrait_id: verify_portrait_id.to_owned(),
        });
        Ok(self)
    }

    pub(crate) fn parse(secret: &str) -> Result<Self> {
        if secret.len() > 64 * 1024 {
            return Err(soda_credential_error("credential exceeds the size limit"));
        }
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
        if self.generation.len() != 64 || !self.generation.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(soda_credential_error("credential generation is invalid"));
        }
        validate_cookie_jar(&self.cookies)?;
        if self
            .cookies
            .keys()
            .any(|name| name.starts_with("passport_mfa_"))
        {
            return Err(soda_credential_error(
                "temporary verification cookies cannot be stored as account credentials",
            ));
        }
        if let Some(id) = &self.user_id {
            validate_user_id(id)?;
        }
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

    pub(crate) fn cookie_header(&self) -> Result<String> {
        cookie_header(&self.cookies)
    }

    pub(crate) fn user_id(&self) -> Option<&str> {
        self.user_id.as_deref()
    }

    /// A content fingerprint bound to this login, stable across Cookie rotations.
    /// It is not an authentication token and does not hash or export Cookie values.
    pub(crate) fn source_snapshot_fingerprint(&self, material: &[u8]) -> String {
        use sha1::{Digest, Sha1};
        let mut hash = Sha1::new();
        hash.update(b"Soda source snapshot v1\0");
        hash.update(self.generation.as_bytes());
        hash.update(b"\0");
        hash.update(material);
        hex::encode(hash.finalize())
    }

    #[cfg(test)]
    pub(crate) fn test_credential(value: &str) -> Self {
        Self::from_cookies(BTreeMap::from([(
            "sessionid_ss".to_owned(),
            value.to_owned(),
        )]))
        .expect("test session credential")
    }

    pub(crate) fn with_response_cookies(&self, headers: &HeaderMap) -> Result<Self> {
        let mut refreshed = self.clone();
        merge_response_cookies(&mut refreshed.cookies, headers)?;
        refreshed
            .cookies
            .retain(|name, _| !name.starts_with("passport_mfa_"));
        if !refreshed.has_session() {
            return Err(TuneWeaveError::new(
                ErrorCode::AuthenticationRequired,
                "Soda session was expired by the platform",
            )
            .with_platform(Platform::Soda));
        }
        refreshed.validate()
    }

    pub(crate) fn bind_user(mut self, user_id: &str) -> Result<Self> {
        validate_user_id(user_id)?;
        if self
            .user_id
            .as_deref()
            .is_some_and(|stored| stored != user_id)
        {
            return Err(soda_upstream_error(
                "Soda session returned a different account identity",
            ));
        }
        self.user_id = Some(user_id.to_owned());
        Ok(self)
    }
}

fn validate_user_id(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 32
        || id.starts_with('0')
        || !id.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(soda_credential_error("Soda account identity is invalid"));
    }
    Ok(())
}

#[derive(Clone)]
pub(crate) struct SodaQrTransactions {
    entries: Arc<Mutex<BTreeMap<String, SodaQrEntry>>>,
    passport_trace_id: String,
    verify_portrait_id: String,
}

impl Default for SodaQrTransactions {
    fn default() -> Self {
        Self {
            entries: Arc::new(Mutex::new(BTreeMap::new())),
            passport_trace_id: random_hex(4),
            verify_portrait_id: format!("{}.login", random_uuid_v4()),
        }
    }
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
    access: Arc<AsyncMutex<SodaQrAccess>>,
}

pub(crate) struct CancelledSodaQrEntries {
    _entries: Vec<SodaQrEntry>,
}

#[derive(Default)]
pub(crate) struct SodaQrAccess {
    mode: CredentialMode,
    owner: Option<(String, CredentialMode)>,
    pub finished: bool,
    pub(crate) authentication: Option<crate::authentication::AuthLease>,
}

struct SodaQrTransaction {
    upstream_token: String,
    device: SodaDeviceState,
    account_sdk_source_info: String,
    browser_context_available: bool,
    cookies: BTreeMap<String, String>,
    last_upstream_poll: Option<Instant>,
    cooldown_until: Option<Instant>,
    last_outcome: SodaQrPollOutcome,
    terminal: Option<SodaQrPollOutcome>,
    mfa: Option<SodaMfa>,
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
    expire_time: Option<u64>,
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
    pub(crate) async fn access(
        &self,
        id: &str,
        account: &str,
        mode: CredentialMode,
    ) -> Result<tokio::sync::OwnedMutexGuard<SodaQrAccess>> {
        validate_transaction_id(id)?;
        let mut access = self.entry(id)?.access.lock_owned().await;
        if access.finished {
            return Err(TuneWeaveError::invalid_request(
                "Soda QR authentication was already consumed",
            )
            .with_platform(Platform::Soda));
        }
        if access.mode != mode {
            return Err(TuneWeaveError::invalid_request(
                "Soda QR credential ownership is fixed at creation",
            )
            .with_platform(Platform::Soda));
        }
        let owner = (account.to_owned(), mode);
        if access.owner.as_ref().is_some_and(|bound| bound != &owner) {
            return Err(TuneWeaveError::invalid_request(
                "Soda QR account and credential ownership cannot change",
            )
            .with_platform(Platform::Soda));
        }
        access.owner = Some(owner);
        Ok(access)
    }

    pub(crate) async fn start(
        &self,
        client: &SodaClient,
        mode: CredentialMode,
        client_context: Option<serde_json::Value>,
    ) -> Result<SodaQrStart> {
        let device = client.login_device()?;
        let biz_trace_id = self.passport_trace_id.clone();
        let browser_context_available = client_context.is_some();
        let account_sdk_source_info = encode_account_sdk_source_info(client_context)?;
        let endpoint = passport_endpoint(
            QR_CREATE_ENDPOINT,
            &device,
            &biz_trace_id,
            &account_sdk_source_info,
            true,
        )?;
        let started = Instant::now();
        let mut http_status = None;
        let outcome = async {
            let response = client
                .send_login_request(
                    add_passport_runtime_headers(
                        client.login_request(reqwest::Method::GET, endpoint),
                        &biz_trace_id,
                        &self.verify_portrait_id,
                    )
                    .header(USER_AGENT, PASSPORT_USER_AGENT)
                    .header(ACCEPT, "application/json, text/javascript"),
                )
                .await?;
            http_status = Some(response.status());
            if !response.status().is_success() {
                return Err(soda_http_error(response.status()));
            }
            let headers = response.headers().clone();
            let body = read_bounded_response(response, "Soda QR creation").await?;
            let created = parse_qr_create_response(&body)?;
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| soda_credential_error("system clock is before the Unix epoch"))?
                .as_secs();
            let expires = created
                .data
                .expire_time
                .unwrap_or(now.saturating_add(QR_TRANSACTION_LIFETIME.as_secs()))
                .min(now.saturating_add(QR_TRANSACTION_LIFETIME.as_secs()));
            if expires <= now {
                return Err(soda_upstream_error(
                    "Soda QR creation returned an expired transaction",
                ));
            }
            let mut cookies = BTreeMap::new();
            merge_response_cookies(&mut cookies, &headers)?;
            let image_data_url = qr_image_data_url(
                &created.data.qrcode,
                &created.data.qrcode_index_url,
                &created.data.token,
            )?;
            let provider_transaction_id = self.insert(
                SodaQrTransaction {
                    upstream_token: created.data.token,
                    device,
                    account_sdk_source_info,
                    browser_context_available,
                    cookies,
                    last_upstream_poll: None,
                    cooldown_until: None,
                    last_outcome: SodaQrPollOutcome::Waiting,
                    terminal: None,
                    mfa: None,
                },
                Duration::from_secs(expires - now),
                mode,
            )?;
            let expires_at = unix_rfc3339(expires);
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
        {
            return Err(qr_rate_limit_error());
        }
        if transaction
            .last_upstream_poll
            .is_some_and(|last_poll| now.duration_since(last_poll) < QR_POLL_MIN_INTERVAL)
        {
            return Ok(transaction.last_outcome.clone());
        }
        transaction.last_upstream_poll = Some(now);

        let endpoint = passport_endpoint(
            QR_POLL_ENDPOINT,
            &transaction.device,
            &self.passport_trace_id,
            &transaction.account_sdk_source_info,
            false,
        )?;
        let mut body = qr_poll_form(&transaction.upstream_token);
        if let Some(mfa) = transaction.mfa.as_ref().filter(|mfa| mfa.validated) {
            let params = form_urlencoded::Serializer::new(String::new())
                .extend_pairs(&mfa.params)
                .finish();
            body.push('&');
            body.push_str(&params);
        }
        let cookie = passport_qr_cookie_header(&transaction.cookies)?;
        let started = Instant::now();
        let mut http_status = None;
        let outcome = async {
            let body_stub = passport_body_stub(&body);
            let mut request = add_passport_runtime_headers(
                client.login_request(reqwest::Method::POST, endpoint),
                &self.passport_trace_id,
                &self.verify_portrait_id,
            )
            .header(USER_AGENT, PASSPORT_USER_AGENT)
            .header(ACCEPT, "application/json, text/javascript")
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header("x-ss-stub", body_stub)
            .body(body);
            if !cookie.is_empty() {
                request = request.header(COOKIE, cookie);
            }
            let response = client.send_login_request(request).await?;
            http_status = Some(response.status());
            if !response.status().is_success() {
                if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    transaction.cooldown_until = Some(Instant::now() + QR_RATE_LIMIT_COOLDOWN);
                    return Err(qr_rate_limit_error());
                }
                return Err(soda_http_error(response.status()));
            }
            let headers = response.headers().clone();
            let body = read_bounded_response(response, "Soda QR polling").await?;
            if Instant::now() >= entry.expires_at {
                return Ok(SodaQrPollOutcome::Expired);
            }
            let parsed = parse_qr_poll_response(&body)?;
            if matches!(
                parsed,
                SodaQrPollOutcome::Failed { .. } | SodaQrPollOutcome::Expired
            ) {
                return Ok(parsed);
            }
            let mut cookies = transaction.cookies.clone();
            merge_response_cookies(&mut cookies, &headers)?;
            if matches!(parsed, SodaQrPollOutcome::AdditionalVerificationRequired) {
                let mut mfa = SodaMfa::parse(&body)?;
                if transaction
                    .mfa
                    .as_ref()
                    .is_some_and(|current| current.same_challenge(&mfa))
                {
                    if transaction
                        .mfa
                        .as_ref()
                        .is_some_and(|current| current.validated)
                    {
                        // This challenge has already been submitted successfully. Reopening it
                        // would offer actions that cannot succeed and invite repeated SMS sends.
                        return Ok(SodaQrPollOutcome::Failed { code: 2046 });
                    }
                    // Keep delivery cooldown and attempt count across ordinary QR polls.
                } else {
                    if let Some(current) = &transaction.mfa {
                        mfa.preserve_limits(current);
                    }
                    transaction.mfa = Some(mfa);
                }
            }
            transaction.cookies = cookies;
            if has_session_cookie(&transaction.cookies)
                && !matches!(parsed, SodaQrPollOutcome::AdditionalVerificationRequired)
            {
                let credential = SodaCredential::from_cookies(transaction.cookies.clone())?;
                let credential = if transaction.browser_context_available {
                    credential.with_passport_context(
                        &transaction.account_sdk_source_info,
                        &self.passport_trace_id,
                        &self.verify_portrait_id,
                    )?
                } else {
                    credential
                };
                return Ok(SodaQrPollOutcome::Confirmed(credential));
            }
            Ok(parsed)
        }
        .await;
        let outcome = match outcome {
            Ok(SodaQrPollOutcome::Failed { code: 7 }) => {
                transaction.cooldown_until = Some(Instant::now() + QR_RATE_LIMIT_COOLDOWN);
                Err(qr_rate_limit_error())
            }
            outcome => outcome,
        };
        client.log_upstream_request(
            "qr_login_poll",
            "api.qishui.com",
            "/passport/web/check_qrconnect/",
            http_status,
            started,
            &outcome,
        );
        let outcome = outcome?;
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

    pub(crate) async fn verification(&self, id: &str) -> Result<QrVerification> {
        let entry = self.entry(id)?;
        let state = entry.state.lock().await;
        state
            .mfa
            .as_ref()
            .map(SodaMfa::describe)
            .ok_or_else(|| soda_upstream_error("Soda QR verification instructions are unavailable"))
    }

    /// The provider holds this transaction's access guard throughout this action and any
    /// subsequent identity verification. Neither requests nor errors expose MFA material.
    pub(crate) async fn verify(
        &self,
        client: &SodaClient,
        id: &str,
        action: &QrVerificationAction,
    ) -> Result<SodaQrPollOutcome> {
        let entry = self.entry(id)?;
        let mut transaction = entry.state.lock().await;
        if Instant::now() >= entry.expires_at {
            return Ok(SodaQrPollOutcome::Expired);
        }
        if transaction.terminal.is_some() {
            return Err(
                TuneWeaveError::invalid_request("Soda QR transaction is already terminal")
                    .with_platform(Platform::Soda),
            );
        }
        let (endpoint, form) = transaction
            .mfa
            .as_mut()
            .ok_or_else(|| {
                TuneWeaveError::invalid_request("Soda QR transaction does not require verification")
                    .with_platform(Platform::Soda)
            })?
            .prepare(action)?;
        let path = endpoint
            .strip_prefix("https://api.qishui.com")
            .ok_or_else(qr_store_error)?;
        let cookie = passport_qr_cookie_header(&transaction.cookies)?;
        let endpoint =
            passport_verification_endpoint(endpoint, &transaction.device, &self.passport_trace_id)?;
        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            let body_stub = passport_body_stub(&form);
            let response = client
                .send_login_request(
                    add_passport_runtime_headers(
                        client.login_request(reqwest::Method::POST, endpoint),
                        &self.passport_trace_id,
                        &self.verify_portrait_id,
                    )
                    .header(USER_AGENT, PASSPORT_USER_AGENT)
                    .header(ACCEPT, "application/json, text/plain, */*")
                    .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .header("x-ss-stub", body_stub)
                    .header(COOKIE, cookie)
                    .body(form),
                )
                .await?;
            http_status = Some(response.status());
            if !response.status().is_success() {
                if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
                    && let Some(mfa) = transaction.mfa.as_mut()
                {
                    mfa.cool_down();
                }
                return Err(soda_http_error(response.status()));
            }
            let headers = response.headers().clone();
            let bytes = read_bounded_response(response, "Soda QR verification").await?;
            if Instant::now() >= entry.expires_at {
                return Ok(SodaQrPollOutcome::Expired);
            }
            let mut mfa = transaction.mfa.clone().ok_or_else(qr_store_error)?;
            if let Err(error) = mfa.accept(action, &bytes) {
                if error.code == ErrorCode::RateLimited
                    && let Some(current) = transaction.mfa.as_mut()
                {
                    current.cool_down();
                }
                return Err(error);
            }
            let mut cookies = transaction.cookies.clone();
            merge_response_cookies(&mut cookies, &headers)?;
            let outcome = if mfa.validated {
                SodaQrPollOutcome::Scanned
            } else {
                SodaQrPollOutcome::AdditionalVerificationRequired
            };
            transaction.cookies = cookies;
            transaction.mfa = Some(mfa);
            transaction.last_outcome = outcome.clone();
            // Validation does not grant authentication. The next poll must complete the
            // original QR exchange and the provider must then verify the official UID.
            if matches!(outcome, SodaQrPollOutcome::Scanned) {
                transaction.last_upstream_poll = None;
            }
            Ok(outcome)
        }
        .await;
        client.log_upstream_request(
            "qr_login_verification",
            "api.qishui.com",
            path,
            http_status,
            started,
            &result,
        );
        result
    }

    fn insert(
        &self,
        transaction: SodaQrTransaction,
        lifetime: Duration,
        mode: CredentialMode,
    ) -> Result<String> {
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
                expires_at: Instant::now() + lifetime.min(QR_TRANSACTION_LIFETIME),
                state: Arc::new(AsyncMutex::new(transaction)),
                access: Arc::new(AsyncMutex::new(SodaQrAccess {
                    mode,
                    ..SodaQrAccess::default()
                })),
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

    pub(crate) fn attach_authentication(
        &self,
        id: &str,
        lease: crate::authentication::AuthLease,
    ) -> Result<()> {
        let entry = self.entry(id)?;
        let mut access = entry.access.try_lock().map_err(|_| qr_store_error())?;
        access.owner = lease
            .account
            .as_ref()
            .map(|account| (account.clone(), lease.mode));
        access.authentication = Some(lease);
        Ok(())
    }

    pub(crate) fn expires_at(&self, id: &str) -> Result<Instant> {
        Ok(self.entry(id)?.expires_at)
    }

    pub(crate) fn remove(&self, transaction_id: &str) -> Result<()> {
        self.entries
            .lock()
            .map_err(|_| qr_store_error())?
            .remove(transaction_id);
        Ok(())
    }

    /// Discard server-owned QR challenges when the corresponding server alias changes.
    /// An unbound legacy Server/Both challenge snapshots the whole server store, so any
    /// server-alias change invalidates it. Client-owned challenges remain independent.
    pub(crate) fn cancel_server(&self, account: &str) -> Result<CancelledSodaQrEntries> {
        let mut entries = self.entries.lock().map_err(|_| qr_store_error())?;
        let cancelled_ids = entries
            .iter()
            .filter_map(|(id, entry)| {
                if Instant::now() >= entry.expires_at {
                    return Some(id.clone());
                }
                let Ok(access) = entry.access.try_lock() else {
                    // An in-flight operation holds this guard. Its AuthLease is cancelled
                    // under the same auth transaction lock and will remove the entry when
                    // the response returns, before it can be accepted or published.
                    return None;
                };
                if !access.mode.persists_on_server() {
                    return None;
                }
                match access.owner.as_ref() {
                    Some((owner, mode)) if mode.persists_on_server() && owner == account => {
                        Some(id.clone())
                    }
                    Some(_) => None,
                    None => Some(id.clone()),
                }
            })
            .collect::<Vec<_>>();
        let cancelled = cancelled_ids
            .into_iter()
            .filter_map(|id| entries.remove(&id))
            .collect();
        Ok(CancelledSodaQrEntries {
            _entries: cancelled,
        })
    }
}

fn passport_endpoint(
    endpoint: &str,
    device: &SodaDeviceState,
    biz_trace_id: &str,
    account_sdk_source_info: &str,
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
            .append_pair("biz_trace_id", biz_trace_id)
            .append_pair("account_sdk_source_info", account_sdk_source_info)
            .append_pair("p_js_v", PASSPORT_JSSDK_VERSION)
            .append_pair("p_js_t", "pro")
            .append_pair("p_zt", PASSPORT_PZT)
            .append_pair("p_ver", PASSPORT_P_VERSION)
            .append_pair("request_host", "app%3A%2F%2Fresources")
            .append_pair("p_bd", PASSPORT_PBD_VERSION)
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
                .append_pair("need_short_url", "false");
        }
    }
    Ok(endpoint)
}

fn passport_verification_endpoint(
    endpoint: &str,
    device: &SodaDeviceState,
    trace_id: &str,
) -> Result<Url> {
    let mut endpoint = Url::parse(endpoint)
        .map_err(|_| soda_credential_error("internal verification endpoint is invalid"))?;
    endpoint
        .query_pairs_mut()
        .append_pair("passport_jssdk_version", PASSPORT_MFA_JSSDK_VERSION)
        .append_pair("passport_jssdk_type", "lite")
        .append_pair("is_from_ttaccountsdk", "1")
        .append_pair("aid", PASSPORT_APP_ID)
        .append_pair("language", "zh")
        .append_pair("account_app_language", "en-US")
        .append_pair("new_authn_sdk_version", PASSPORT_NEW_AUTHN_SDK_VERSION)
        .append_pair("is_new_login", "1")
        .append_pair("is_from_iesaccountsaas", "1")
        .append_pair("device_id", &device.device_id)
        .append_pair("install_id", &device.install_id)
        .append_pair("did", &device.device_id)
        .append_pair("iid", &device.install_id)
        .append_pair("device_platform", "PC")
        .append_pair("version_code", PASSPORT_VERSION_CODE)
        .append_pair("biz_trace_id", trace_id);
    Ok(endpoint)
}

fn passport_token_beat_endpoint(
    device: &SodaDeviceState,
    context: &SodaPassportContext,
) -> Result<Url> {
    let mut endpoint = passport_endpoint(
        TOKEN_BEAT_ENDPOINT,
        device,
        &context.trace_id,
        &context.account_sdk_source_info,
        false,
    )?;
    endpoint
        .query_pairs_mut()
        .append_pair("scene", "boot")
        .append_pair("version", PASSPORT_ACCOUNT_SDK_VERSION);
    Ok(endpoint)
}

pub(crate) async fn token_beat(
    client: &SodaClient,
    credential: &SodaCredential,
) -> Result<SodaCredential> {
    let Some(context) = credential.passport_context.as_ref() else {
        return Ok(credential.clone());
    };
    let device = client.login_device()?;
    let endpoint = passport_token_beat_endpoint(&device, context)?;
    let started = Instant::now();
    let mut http_status = None;
    let outcome = async {
        let response = client
            .send_login_request(
                add_passport_runtime_headers(
                    client.login_request(reqwest::Method::GET, endpoint),
                    &context.trace_id,
                    &context.verify_portrait_id,
                )
                .header(USER_AGENT, PASSPORT_USER_AGENT)
                .header(ACCEPT, "application/json, text/javascript")
                .header(COOKIE, credential.cookie_header()?),
            )
            .await?;
        http_status = Some(response.status());
        if !response.status().is_success() {
            return Err(soda_http_error(response.status()));
        }
        let headers = response.headers().clone();
        let _body = read_bounded_response(response, "Soda Passport token beat").await?;
        let mut refreshed = credential.with_response_cookies(&headers)?;
        // This continuation belongs to the one QR login that created it. It is not
        // persisted into the credential and must not cause a token beat on every read.
        refreshed.passport_context = None;
        Ok(refreshed)
    }
    .await;
    client.log_upstream_request(
        "account_token_beat",
        "api.qishui.com",
        "/passport/token/beat/web/",
        http_status,
        started,
        &outcome,
    );
    outcome
}

fn add_passport_runtime_headers(
    request: reqwest::RequestBuilder,
    trace_id: &str,
    verify_portrait_id: &str,
) -> reqwest::RequestBuilder {
    request
        .header("accept-language", "en-US")
        .header("priority", "u=1, i")
        .header(
            "sec-ch-ua",
            "\"Not.A/Brand\";v=\"99\", \"Chromium\";v=\"136\"",
        )
        .header("sec-ch-ua-mobile", "?0")
        .header("sec-ch-ua-platform", "\"Windows\"")
        .header("sec-fetch-dest", "empty")
        .header("sec-fetch-mode", "cors")
        .header("sec-fetch-site", "cross-site")
        .header("sec-fetch-storage-access", "active")
        .header("x-tt-trace-id", passport_request_trace_id())
        .header("x-tt-passport-csrf-token", "")
        .header("x-tt-passport-trace-id", trace_id)
        .header("x-tt-passport-verify-portrait", verify_portrait_id)
}

fn passport_request_trace_id() -> String {
    format!("00-{}-{}-01", random_hex(16), random_hex(8))
}

fn passport_body_stub(body: &str) -> String {
    format!("{:X}", Md5::digest(body.as_bytes()))
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

const MAX_SODA_CLIENT_CONTEXT_BYTES: usize = 16 * 1024;
const SODA_CLIENT_CONTEXT_FIELDS: &[&str] = &[
    "hardwareConcurrency",
    "webdriver",
    "chromedriver",
    "shelldriver",
    "plugins",
    "permissions",
    "innerHeight",
    "innerWidth",
    "outerHeight",
    "outerWidth",
    "stoargeStatus",
    "webgl",
    "notificationPermission",
    "performance",
    "request_host",
    "request_pathname",
    "browser",
];

fn encode_account_sdk_source_info(client_context: Option<serde_json::Value>) -> Result<String> {
    let context = client_context.unwrap_or_else(|| serde_json::json!({}));
    let Some(fields) = context.as_object() else {
        return Err(soda_credential_error(
            "Soda browser login context must be an object",
        ));
    };
    if fields
        .keys()
        .any(|key| !SODA_CLIENT_CONTEXT_FIELDS.contains(&key.as_str()))
    {
        return Err(soda_credential_error(
            "Soda browser login context contains an unsupported field",
        ));
    }
    let mut nodes = 0usize;
    if !safe_client_context_value(&context, 0, &mut nodes) {
        return Err(soda_credential_error(
            "Soda browser login context is invalid",
        ));
    }
    let json = serde_json::to_string(&context)
        .map_err(|_| soda_credential_error("Soda browser login context is invalid"))?;
    if json.len() > MAX_SODA_CLIENT_CONTEXT_BYTES {
        return Err(soda_credential_error(
            "Soda browser login context is too large",
        ));
    }
    Ok(json
        .as_bytes()
        .iter()
        .map(|byte| format!("{:x}", byte ^ 5))
        .collect())
}

fn safe_client_context_value(value: &serde_json::Value, depth: usize, nodes: &mut usize) -> bool {
    *nodes += 1;
    if depth > 8 || *nodes > 512 {
        return false;
    }
    match value {
        serde_json::Value::Object(object) => {
            object.len() <= 64
                && object.iter().all(|(key, value)| {
                    let key = key.to_ascii_lowercase();
                    !["cookie", "password", "token", "authorization", "secret"]
                        .iter()
                        .any(|forbidden| key.contains(forbidden))
                        && key.len() <= 128
                        && safe_client_context_value(value, depth + 1, nodes)
                })
        }
        serde_json::Value::Array(items) => {
            items.len() <= 128
                && items
                    .iter()
                    .all(|item| safe_client_context_value(item, depth + 1, nodes))
        }
        serde_json::Value::String(text) => {
            text.len() <= 2048 && !text.chars().any(char::is_control)
        }
        serde_json::Value::Number(_) | serde_json::Value::Bool(_) | serde_json::Value::Null => true,
    }
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
        "new" if response.data.error_code == 0 => Ok(SodaQrPollOutcome::Waiting),
        "scanned" | "confirmed" if response.data.error_code == 0 => Ok(SodaQrPollOutcome::Scanned),
        "expired" => Ok(SodaQrPollOutcome::Expired),
        "error" | "failed" => Ok(SodaQrPollOutcome::Failed {
            code: response.data.error_code,
        }),
        _ if response.data.error_code != 0 => Ok(SodaQrPollOutcome::Failed {
            code: response.data.error_code,
        }),
        _ => Err(soda_upstream_error(
            "Soda QR polling returned an unknown state",
        )),
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
    let mut updated = cookies.clone();
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
        let mut max_age = None;
        let mut expires = None;
        for attribute in header.split(';').skip(1) {
            if let Some((key, value)) = attribute.trim().split_once('=') {
                if key.trim().eq_ignore_ascii_case("max-age") {
                    if let Ok(age) = value.trim().parse::<i64>() {
                        max_age = Some(age);
                    }
                } else if key.trim().eq_ignore_ascii_case("expires") {
                    expires = httpdate::parse_http_date(value.trim()).ok();
                }
            }
        }
        let removed = max_age.map_or_else(
            || expires.is_some_and(|expiry| expiry <= SystemTime::now()),
            |age| age <= 0,
        );
        if value.is_empty() || removed {
            updated.remove(name);
        } else {
            updated.insert(name.to_owned(), value.to_owned());
        }
    }
    validate_cookie_jar(&updated)?;
    *cookies = updated;
    Ok(())
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

fn passport_qr_cookie_header(cookies: &BTreeMap<String, String>) -> Result<String> {
    let browser_scoped = cookies
        .iter()
        .filter(|(name, _)| name.as_str() != "passport_csrf_token_default")
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();
    cookie_header(&browser_scoped)
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

fn random_uuid_v4() -> String {
    let mut bytes = [0_u8; 16];
    for byte in &mut bytes {
        *byte = rand::random();
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15],
    )
}

fn qr_rate_limit_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::RateLimited,
        "Soda QR login requests are too frequent; pause polling before retrying",
    )
    .with_platform(Platform::Soda)
    .with_details(serde_json::json!({
        "retry_after_secs": QR_RATE_LIMIT_COOLDOWN.as_secs()
    }))
    .retryable(true)
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
    fn cookie_updates_are_atomic_and_honor_revocation() {
        let initial = session_cookies();
        let mut cookies = initial.clone();
        let mut headers = HeaderMap::new();
        headers.append(SET_COOKIE, "sessionid_ss=rotated".parse().unwrap());
        headers.append(SET_COOKIE, "not-a-cookie".parse().unwrap());
        assert!(merge_response_cookies(&mut cookies, &headers).is_err());
        assert_eq!(cookies, initial);
        headers.clear();
        headers.insert(
            SET_COOKIE,
            "sessionid_ss=expired; Max-Age=0; Path=/".parse().unwrap(),
        );
        let credential = SodaCredential::from_cookies(cookies).unwrap();
        assert_eq!(
            credential.with_response_cookies(&headers).unwrap_err().code,
            ErrorCode::AuthenticationRequired
        );
        assert!(
            credential
                .cookie_header()
                .unwrap()
                .contains("session-secret")
        );
    }

    #[test]
    fn expiry_honors_max_age_precedence_and_drops_temporary_mfa_cookies() {
        let credential = SodaCredential::from_cookies(session_cookies()).unwrap();
        let mut headers = HeaderMap::new();
        headers.insert(
            SET_COOKIE,
            "sessionid_ss=expired; Expires=Thu, 01 Jan 1970 00:00:00 GMT"
                .parse()
                .unwrap(),
        );
        assert_eq!(
            credential.with_response_cookies(&headers).unwrap_err().code,
            ErrorCode::AuthenticationRequired
        );
        headers.insert(
            SET_COOKIE,
            "sessionid_ss=renewed; Max-Age=60; Expires=Thu, 01 Jan 1970 00:00:00 GMT"
                .parse()
                .unwrap(),
        );
        headers.append(
            SET_COOKIE,
            "passport_mfa_token=temporary-secret".parse().unwrap(),
        );
        let refreshed = credential.with_response_cookies(&headers).unwrap();
        assert!(refreshed.cookie_header().unwrap().contains("renewed"));
        assert!(!refreshed.serialize().unwrap().contains("temporary-secret"));
        let mut cookies = session_cookies();
        cookies.insert(
            "passport_mfa_token".to_owned(),
            "temporary-secret".to_owned(),
        );
        assert!(
            !SodaCredential::from_cookies(cookies)
                .unwrap()
                .serialize()
                .unwrap()
                .contains("temporary-secret")
        );
    }

    #[test]
    fn qr_responses_do_not_turn_missing_or_unknown_states_into_waiting() {
        for body in [
            br#"{}"#.as_slice(),
            br#"{"data":{"status":""}}"#,
            br#"{"data":{"status":"unexpected"}}"#,
        ] {
            assert_eq!(
                parse_qr_poll_response(body).unwrap_err().code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn qr_poll_form_matches_the_captured_windows_request_fields() {
        let fields = form_urlencoded::parse(qr_poll_form("qr-token").as_bytes())
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect::<Vec<_>>();

        assert_eq!(
            fields,
            [
                ("need_logo".to_owned(), "false".to_owned()),
                ("need_short_url".to_owned(), "false".to_owned()),
                ("is_frontier".to_owned(), "true".to_owned()),
                ("token".to_owned(), "qr-token".to_owned()),
                ("is_new_login".to_owned(), "1".to_owned()),
                ("next".to_owned(), "https://api.qishui.com".to_owned()),
            ]
        );
    }

    #[test]
    fn passport_body_stub_is_uppercase_md5_of_the_exact_wire_body() {
        assert_eq!(
            passport_body_stub("abc"),
            "900150983CD24FB0D6963F7D28E17F72"
        );
        assert_ne!(
            passport_body_stub("a=one+two"),
            passport_body_stub("a=one%20two")
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
    fn imported_cookies_are_strict_and_start_a_new_login_generation() {
        for value in [
            "sessionid_ss=a; sessionid_ss=b",
            "sessionid_ss=",
            "Cookie: sessionid_ss=a",
            "sessionid_ss=a\r\nx-forwarded-host=other",
            "sessionid_ss=a; Path=/",
            "sessionid_ss=a; passport_mfa_token=temporary",
            "sessionid_ss=a; Secure",
            "sessionid_ss=a; ",
            "anonymous=only",
        ] {
            assert!(
                SodaCredential::import_cookie_header(value).is_err(),
                "{value}"
            );
        }
        let raw = "sessionid_ss=import-secret; sid_guard=value%7Cvalue; csrf=a==";
        let first = SodaCredential::import_cookie_header(raw)
            .unwrap()
            .bind_user("123456")
            .unwrap();
        let second = SodaCredential::import_cookie_header(raw)
            .unwrap()
            .bind_user("123456")
            .unwrap();
        assert!(!first.same_login(&second));
        assert!(!format!("{first:?}").contains("import-secret"));
        let mut headers = HeaderMap::new();
        headers.insert(SET_COOKIE, "sessionid_ss=rotated".parse().unwrap());
        assert!(first.same_login(&first.with_response_cookies(&headers).unwrap()));
        assert!(
            SodaCredential::import_cookie_header(&format!("sessionid_ss={}", "a".repeat(32769)))
                .is_err()
        );
    }

    #[test]
    fn passport_requests_match_windows_runtime_metadata_without_faking_dynamic_signatures() {
        let device = SodaDeviceState {
            schema_version: 1,
            device_id: "1234567890123456789".to_owned(),
            install_id: "2234567890123456789".to_owned(),
            created_at_ms: 1,
        };
        let source_info = encode_account_sdk_source_info(Some(serde_json::json!({
            "hardwareConcurrency": 8,
            "webdriver": false,
            "request_host": "resources",
            "request_pathname": "/harness.html",
        })))
        .expect("encoded browser context");
        let endpoint =
            passport_endpoint(QR_CREATE_ENDPOINT, &device, "a1b2c3d4", &source_info, true)
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
        assert_eq!(
            query.get("p_bd").map(|value| value.as_ref()),
            Some(PASSPORT_PBD_VERSION)
        );
        assert_eq!(
            query.get("biz_trace_id").map(|value| value.as_ref()),
            Some("a1b2c3d4")
        );
        let poll_endpoint =
            passport_endpoint(QR_POLL_ENDPOINT, &device, "a1b2c3d4", &source_info, false)
                .expect("poll endpoint");
        let poll_query = poll_endpoint.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(
            poll_query.get("biz_trace_id").map(|value| value.as_ref()),
            query.get("biz_trace_id").map(|value| value.as_ref())
        );
        assert_eq!(
            query.get("version_code").map(|value| value.as_ref()),
            Some("3.7.0")
        );
        assert_eq!(
            query
                .get("account_sdk_source_info")
                .map(|value| value.as_ref()),
            Some(source_info.as_str())
        );
        assert_eq!(
            query.get("request_host").map(|value| value.as_ref()),
            Some("app%3A%2F%2Fresources")
        );
        assert!(!query.contains_key("is_frontier"));
        assert!(!query.contains_key("msToken"));
        // The official app injects these per-request anti-abuse values dynamically.
        // Never place a captured value or a constant placeholder in source code.
        assert!(!query.contains_key("a_bogus"));
    }

    #[test]
    fn mfa_and_token_beat_use_their_observed_sdk_profiles() {
        let device = SodaDeviceState {
            schema_version: 1,
            device_id: "1234567890123456789".to_owned(),
            install_id: "2234567890123456789".to_owned(),
            created_at_ms: 1,
        };
        let mfa = passport_verification_endpoint(
            "https://api.qishui.com/passport/web/send_code/",
            &device,
            "a1b2c3d4",
        )
        .unwrap();
        let query = mfa.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(query.get("passport_jssdk_type").unwrap(), "lite");
        assert_eq!(
            query.get("passport_jssdk_version").unwrap(),
            PASSPORT_MFA_JSSDK_VERSION
        );
        assert_eq!(
            query.get("new_authn_sdk_version").unwrap(),
            PASSPORT_NEW_AUTHN_SDK_VERSION
        );
        assert_eq!(query.get("account_app_language").unwrap(), "en-US");
        assert_eq!(query.get("biz_trace_id").unwrap(), "a1b2c3d4");
        assert!(!query.contains_key("p_js_v"));
        assert!(!query.contains_key("p_bd"));

        let context = SodaPassportContext {
            account_sdk_source_info: "7b7e".to_owned(),
            trace_id: "a1b2c3d4".to_owned(),
            verify_portrait_id: "00000000-0000-4000-8000-000000000000.login".to_owned(),
        };
        let beat = passport_token_beat_endpoint(&device, &context).unwrap();
        let query = beat.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(query.get("scene").unwrap(), "boot");
        assert_eq!(query.get("version").unwrap(), PASSPORT_ACCOUNT_SDK_VERSION);
        assert_eq!(query.get("p_bd").unwrap(), PASSPORT_PBD_VERSION);
        assert_eq!(
            query.get("account_sdk_source_info").unwrap(),
            &context.account_sdk_source_info
        );
    }

    #[test]
    fn passport_runtime_headers_follow_sdk_trace_and_portrait_shape() {
        let transactions = SodaQrTransactions::default();
        let cloned = transactions.clone();
        assert_eq!(transactions.passport_trace_id, cloned.passport_trace_id);
        assert_eq!(transactions.verify_portrait_id, cloned.verify_portrait_id);
        assert_eq!(transactions.passport_trace_id.len(), 8);
        assert!(
            transactions
                .passport_trace_id
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );

        let portrait = &transactions.verify_portrait_id;
        assert_eq!(portrait.len(), 42);
        assert!(portrait.ends_with(".login"));
        let uuid = &portrait[..36];
        assert_eq!(&uuid[8..9], "-");
        assert_eq!(&uuid[13..14], "-");
        assert_eq!(&uuid[18..19], "-");
        assert_eq!(&uuid[23..24], "-");
        assert_eq!(&uuid[14..15], "4");
        assert!(matches!(&uuid[19..20], "8" | "9" | "a" | "b"));

        let request = add_passport_runtime_headers(
            reqwest::Client::new().get(QR_POLL_ENDPOINT),
            &transactions.passport_trace_id,
            portrait,
        )
        .build()
        .expect("build passport request");
        let headers = request.headers();
        assert_eq!(
            headers
                .get("x-tt-passport-trace-id")
                .unwrap()
                .to_str()
                .unwrap(),
            transactions.passport_trace_id
        );
        assert_eq!(
            headers
                .get("x-tt-passport-verify-portrait")
                .unwrap()
                .to_str()
                .unwrap(),
            portrait
        );
        assert_eq!(headers.get("x-tt-passport-csrf-token").unwrap(), "");
        assert_eq!(headers.get("sec-fetch-site").unwrap(), "cross-site");
        assert_eq!(headers.get("sec-ch-ua-platform").unwrap(), "\"Windows\"");
        let trace = headers.get("x-tt-trace-id").unwrap().to_str().unwrap();
        let trace_parts = trace.split('-').collect::<Vec<_>>();
        assert_eq!(trace_parts.len(), 4);
        assert_eq!(trace_parts[0], "00");
        assert_eq!(trace_parts[1].len(), 32);
        assert_eq!(trace_parts[2].len(), 16);
        assert_eq!(trace_parts[3], "01");
        assert!(trace_parts[1..3].iter().all(|part| {
            part.bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        }));
    }

    #[test]
    fn qr_poll_cookies_match_the_captured_cross_site_browser_scope() {
        let cookies = BTreeMap::from([
            ("passport_csrf_token".to_owned(), "csrf".to_owned()),
            (
                "passport_csrf_token_default".to_owned(),
                "same_site_only".to_owned(),
            ),
            ("sessionid_ss".to_owned(), "session".to_owned()),
        ]);
        assert_eq!(
            passport_qr_cookie_header(&cookies).unwrap(),
            "passport_csrf_token=csrf; sessionid_ss=session"
        );
    }

    #[test]
    fn account_sdk_source_info_uses_official_xor_hex_encoding_and_rejects_secrets() {
        let context = serde_json::json!({
            "webdriver": false,
            "request_host": "resources",
            "request_pathname": "/harness.html",
        });
        let encoded = encode_account_sdk_source_info(Some(context.clone())).unwrap();
        let decoded = encoded
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let pair = std::str::from_utf8(pair).unwrap();
                u8::from_str_radix(pair, 16).unwrap() ^ 5
            })
            .collect::<Vec<_>>();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&decoded).unwrap(),
            context
        );
        assert!(
            encode_account_sdk_source_info(Some(serde_json::json!({
                "cookie": "must-not-be-forwarded",
            })))
            .is_err()
        );
    }

    #[tokio::test]
    async fn a_completed_verification_challenge_cannot_reopen_itself() {
        let mut transaction = pending_mfa_transaction();
        let mfa = transaction.mfa.as_mut().unwrap();
        mfa.prepare(&QrVerificationAction::SendSms).unwrap();
        mfa.accept(
            &QrVerificationAction::SendSms,
            br#"{"message":"success","data":{}}"#,
        )
        .unwrap();
        let action = QrVerificationAction::SubmitSms {
            code: "864209".to_owned(),
        };
        mfa.prepare(&action).unwrap();
        mfa.accept(
            &action,
            br#"{"message":"success","data":{"ticket":"accepted"}}"#,
        )
        .unwrap();
        let store = SodaQrTransactions::default();
        let id = store
            .insert(transaction, QR_TRANSACTION_LIFETIME, CredentialMode::Client)
            .unwrap();
        let repeated = String::from_utf8(crate::mfa::tests::sms_fixture()).unwrap();
        let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
            &repeated,
            Some("sessionid_ss=must-not-be-used"),
        )])
        .await;
        let client = SodaClient::test_client().with_auth_test_origin(origin);
        assert_eq!(
            store.poll(&client, &id).await.unwrap(),
            SodaQrPollOutcome::Failed { code: 2046 }
        );
        assert!(
            store
                .verify(&client, &id, &QrVerificationAction::SendSms)
                .await
                .is_err()
        );
        let entry = store.entry(&id).unwrap();
        assert!(
            !entry
                .state
                .lock()
                .await
                .cookies
                .contains_key("sessionid_ss")
        );
        assert_eq!(server.await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn expired_verification_never_sends_sms() {
        let store = SodaQrTransactions::default();
        let id = store
            .insert(
                pending_mfa_transaction(),
                Duration::ZERO,
                CredentialMode::Client,
            )
            .unwrap();
        let (origin, server) = crate::test_http::serve(vec![]).await;
        let client = SodaClient::test_client().with_auth_test_origin(origin);
        assert_eq!(
            store
                .verify(&client, &id, &QrVerificationAction::SendSms)
                .await
                .unwrap(),
            SodaQrPollOutcome::Expired
        );
        assert!(server.await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn qr_poll_rate_limits_remain_visible_during_cooldown() {
        let business_limit = crate::test_http::json(
            r#"{"message":"error","data":{"error_code":7}}"#,
            Some("sessionid_ss=must-not-be-used"),
        );
        let http_limit = business_limit.replacen("200 OK", "429 Too Many Requests", 1);
        for response in [business_limit, http_limit] {
            let (origin, server) = crate::test_http::serve(vec![response]).await;
            let client = SodaClient::test_client().with_auth_test_origin(origin);
            let store = SodaQrTransactions::default();
            let mut transaction = pending_mfa_transaction();
            transaction.mfa = None;
            transaction.last_outcome = SodaQrPollOutcome::Scanned;
            let id = store
                .insert(transaction, QR_TRANSACTION_LIFETIME, CredentialMode::Client)
                .unwrap();
            for _ in 0..2 {
                let error = store.poll(&client, &id).await.unwrap_err();
                assert_eq!(error.code, ErrorCode::RateLimited);
                assert!(error.retryable);
            }
            let entry = store.entry(&id).unwrap();
            let state = entry.state.lock().await;
            assert!(state.terminal.is_none());
            assert!(!has_session_cookie(&state.cookies));
            assert_eq!(server.await.unwrap().len(), 1);
        }
    }

    #[tokio::test]
    async fn verification_rate_limits_survive_the_short_attempt_interval_and_reject_failure_cookies()
     {
        let business_limit = crate::test_http::json(
            r#"{"message":"error","data":{"error_code":7}}"#,
            Some("sessionid_ss=must-not-be-used"),
        );
        let http_limit = business_limit.replacen("200 OK", "429 Too Many Requests", 1);
        for response in [business_limit, http_limit] {
            let (origin, server) = crate::test_http::serve(vec![response]).await;
            let client = SodaClient::test_client().with_auth_test_origin(origin);
            let store = SodaQrTransactions::default();
            let mut transaction = pending_mfa_transaction();
            let mfa = transaction.mfa.as_mut().unwrap();
            mfa.prepare(&QrVerificationAction::SendSms).unwrap();
            mfa.accept(
                &QrVerificationAction::SendSms,
                br#"{"message":"success","data":{}}"#,
            )
            .unwrap();
            let id = store
                .insert(transaction, QR_TRANSACTION_LIFETIME, CredentialMode::Client)
                .unwrap();
            let action = QrVerificationAction::SubmitSms {
                code: "864209".to_owned(),
            };
            assert_eq!(
                store.verify(&client, &id, &action).await.unwrap_err().code,
                ErrorCode::RateLimited
            );
            // The normal two-second attempt interval is insufficient after an upstream 429/7.
            tokio::time::sleep(Duration::from_millis(2100)).await;
            assert_eq!(
                store.verify(&client, &id, &action).await.unwrap_err().code,
                ErrorCode::RateLimited
            );
            let entry = store.entry(&id).unwrap();
            let transaction = entry.state.lock().await;
            assert_eq!(
                transaction.cookies.get("passport_mfa_token").unwrap(),
                "temporary"
            );
            assert!(!transaction.cookies.contains_key("sessionid_ss"));
            assert!(!transaction.mfa.as_ref().unwrap().validated);
            assert_eq!(server.await.unwrap().len(), 1);
        }
    }

    fn pending_mfa_transaction() -> SodaQrTransaction {
        SodaQrTransaction {
            upstream_token: "0123456789abcdefghijklmnopqrstuvw".to_owned(),
            device: SodaDeviceState {
                schema_version: 1,
                device_id: "1234567890123456789".to_owned(),
                install_id: "2234567890123456789".to_owned(),
                created_at_ms: 1,
            },
            account_sdk_source_info: encode_account_sdk_source_info(None).unwrap(),
            browser_context_available: false,
            cookies: BTreeMap::from([("passport_mfa_token".to_owned(), "temporary".to_owned())]),
            last_upstream_poll: None,
            cooldown_until: None,
            last_outcome: SodaQrPollOutcome::AdditionalVerificationRequired,
            terminal: None,
            mfa: Some(SodaMfa::parse(&crate::mfa::tests::sms_fixture()).unwrap()),
        }
    }

    #[tokio::test]
    async fn transaction_ids_hide_upstream_tokens_and_throttle_local_polls() {
        let store = SodaQrTransactions::default();
        let upstream_token = "0123456789abcdefghijklmnopqrstuvw".to_owned();
        let transaction_id = store
            .insert(
                SodaQrTransaction {
                    upstream_token: upstream_token.clone(),
                    device: SodaDeviceState {
                        schema_version: 1,
                        device_id: "1234567890123456789".to_owned(),
                        install_id: "2234567890123456789".to_owned(),
                        created_at_ms: 1,
                    },
                    account_sdk_source_info: encode_account_sdk_source_info(None).unwrap(),
                    browser_context_available: false,
                    cookies: BTreeMap::new(),
                    last_upstream_poll: Some(Instant::now()),
                    cooldown_until: None,
                    last_outcome: SodaQrPollOutcome::Scanned,
                    terminal: None,
                    mfa: None,
                },
                QR_TRANSACTION_LIFETIME,
                CredentialMode::Server,
            )
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
