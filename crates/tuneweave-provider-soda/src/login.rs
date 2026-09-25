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
use tuneweave_core::{
    CredentialMode, ErrorCode, Platform, QrVerification, QrVerificationAction, Result,
    TuneWeaveError,
};
use url::{Url, form_urlencoded};

use crate::client::{
    SodaClient, read_bounded_response, soda_http_error, soda_network_error, soda_upstream_error,
    unix_rfc3339,
};
use crate::device::SodaDeviceState;
use crate::mfa::SodaMfa;

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
    generation: String,
    cookies: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user_id: Option<String>,
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
        }
        .validate()
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
    trace_id: String,
    device: SodaDeviceState,
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
    ) -> Result<SodaQrStart> {
        let device = client.login_device()?;
        let trace_id = random_hex(16);
        let endpoint = passport_endpoint(QR_CREATE_ENDPOINT, &device, &trace_id, true)?;
        let started = Instant::now();
        let mut http_status = None;
        let outcome = async {
            let response = client
                .login_request(reqwest::Method::GET, endpoint)
                .header(USER_AGENT, PASSPORT_USER_AGENT)
                .header(ACCEPT, "application/json, text/javascript")
                .send()
                .await
                .map_err(soda_network_error)?;
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
                    trace_id,
                    device,
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
        let mut body = qr_poll_form(&transaction.upstream_token);
        if let Some(mfa) = transaction.mfa.as_ref().filter(|mfa| mfa.validated) {
            let params = form_urlencoded::Serializer::new(String::new())
                .extend_pairs(&mfa.params)
                .finish();
            body.push('&');
            body.push_str(&params);
        }
        let cookie = cookie_header(&transaction.cookies)?;
        let started = Instant::now();
        let mut http_status = None;
        let outcome = async {
            let mut request = client
                .login_request(reqwest::Method::POST, endpoint)
                .header(USER_AGENT, PASSPORT_USER_AGENT)
                .header(ACCEPT, "application/json, text/javascript")
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(body);
            if !cookie.is_empty() {
                request = request.header(COOKIE, cookie);
            }
            let response = request.send().await.map_err(soda_network_error)?;
            http_status = Some(response.status());
            if !response.status().is_success() {
                if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    transaction.cooldown_until = Some(Instant::now() + QR_RATE_LIMIT_COOLDOWN);
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
                return SodaCredential::from_cookies(transaction.cookies.clone())
                    .map(SodaQrPollOutcome::Confirmed);
            }
            Ok(parsed)
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
        let endpoint =
            passport_endpoint(endpoint, &transaction.device, &transaction.trace_id, false)?;
        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            let response = client
                .login_request(reqwest::Method::POST, endpoint)
                .header(USER_AGENT, PASSPORT_USER_AGENT)
                .header(ACCEPT, "application/json")
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(COOKIE, cookie_header(&transaction.cookies)?)
                .body(form)
                .send()
                .await
                .map_err(soda_network_error)?;
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
            trace_id: "0123456789abcdef".to_owned(),
            device: SodaDeviceState {
                schema_version: 1,
                device_id: "1234567890123456789".to_owned(),
                install_id: "2234567890123456789".to_owned(),
                created_at_ms: 1,
            },
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
