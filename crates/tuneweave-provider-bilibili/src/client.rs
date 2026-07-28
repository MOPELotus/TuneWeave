use std::{collections::BTreeMap, fmt, sync::Arc, time::Duration};

use aws_lc_rs::digest::{SHA256, digest};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD as BASE64, URL_SAFE_NO_PAD as BASE64_URL_SAFE_NO_PAD},
};
use num_bigint::BigUint;
use qrcode::{QrCode, render::svg};
use reqwest::{
    Client, Proxy, StatusCode,
    header::{COOKIE, HeaderMap, SET_COOKIE},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tuneweave_core::{AccountCredentialStore, ErrorCode, Platform, Result, TuneWeaveError};
use url::Url;

const PASSPORT_QR_GENERATE_ENDPOINT: &str =
    "https://passport.bilibili.com/x/passport-login/web/qrcode/generate?source=main-fe-header";
const PASSPORT_QR_POLL_ENDPOINT: &str =
    "https://passport.bilibili.com/x/passport-login/web/qrcode/poll";
const COOKIE_INFO_ENDPOINT: &str = "https://passport.bilibili.com/x/passport-login/web/cookie/info";
const COOKIE_CORRESPOND_ENDPOINT: &str = "https://www.bilibili.com/correspond/1/";
const COOKIE_REFRESH_ENDPOINT: &str =
    "https://passport.bilibili.com/x/passport-login/web/cookie/refresh";
const COOKIE_CONFIRM_ENDPOINT: &str =
    "https://passport.bilibili.com/x/passport-login/web/confirm/refresh";
const LOGOUT_ENDPOINT: &str = "https://passport.bilibili.com/login/exit/v2";
const NAV_ENDPOINT: &str = "https://api.bilibili.com/x/web-interface/nav";
const WEB_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const MAX_PASSPORT_RESPONSE_BYTES: usize = 1024 * 1024;
const COOKIE_REFRESH_RSA_MODULUS_BASE64URL: &str = concat!(
    "y4HdjgJHBlbaBN04VERG4qNBIFHP6a3GozCl75AihQloSWCXC5HDNgyinEnhaQ_4",
    "-gaMud_GF50elYXLlCToR9se9Z8z433U3KjM-3Yx7ptKkmQNAMggQwAVKgq3zYAoi",
    "dNEWuxpkY_mAitTSRLnsJW-NCTa0bqBFF6Wm1MxgfE",
);
const COOKIE_REFRESH_RSA_EXPONENT: u32 = 65_537;

#[derive(Clone, Default)]
pub struct BilibiliConfig {
    pub proxy_url: Option<String>,
    pub credential_store: Option<Arc<dyn AccountCredentialStore>>,
}

impl fmt::Debug for BilibiliConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BilibiliConfig")
            .field(
                "proxy_url",
                &self.proxy_url.as_ref().map(|_| "[configured]"),
            )
            .field(
                "credential_store_configured",
                &self.credential_store.is_some(),
            )
            .finish()
    }
}

#[derive(Clone)]
pub struct BilibiliClient {
    http: Client,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct BilibiliQrStart {
    pub qrcode_key: String,
    pub url: String,
    pub image_data_url: String,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) enum BilibiliQrPoll {
    Waiting,
    Scanned,
    Expired,
    Confirmed {
        credential: BilibiliCredential,
        timestamp_ms: Option<u64>,
    },
    Failed {
        code: i64,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BilibiliCredentialRefresh {
    pub credential: BilibiliCredential,
    pub status: BilibiliSessionStatus,
    pub refreshed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BilibiliLogoutOutcome {
    LoggedOut,
    CredentialExpired,
}

impl fmt::Debug for BilibiliQrPoll {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Waiting => formatter.write_str("Waiting"),
            Self::Scanned => formatter.write_str("Scanned"),
            Self::Expired => formatter.write_str("Expired"),
            Self::Confirmed { timestamp_ms, .. } => formatter
                .debug_struct("Confirmed")
                .field("credential", &"[redacted]")
                .field("timestamp_ms", timestamp_ms)
                .finish(),
            Self::Failed { code, message } => formatter
                .debug_struct("Failed")
                .field("code", code)
                .field("message", message)
                .finish(),
        }
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BilibiliCredential {
    pub dede_user_id: String,
    pub dede_user_id_ck_md5: String,
    pub sessdata: String,
    pub bili_jct: String,
    pub sid: Option<String>,
    pub refresh_token: String,
}

impl fmt::Debug for BilibiliCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BilibiliCredential")
            .field("user_id_configured", &(!self.dede_user_id.is_empty()))
            .field("has_sessdata", &(!self.sessdata.is_empty()))
            .field("has_refresh_token", &(!self.refresh_token.is_empty()))
            .finish()
    }
}

impl BilibiliCredential {
    pub(crate) fn normalize(mut self) -> Result<Self> {
        self.dede_user_id = self.dede_user_id.trim().to_owned();
        self.dede_user_id_ck_md5 = self.dede_user_id_ck_md5.trim().to_owned();
        self.sessdata = self.sessdata.trim().to_owned();
        self.bili_jct = self.bili_jct.trim().to_owned();
        self.sid = self
            .sid
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        self.refresh_token = self.refresh_token.trim().to_owned();
        if self
            .dede_user_id
            .parse::<u64>()
            .ok()
            .filter(|user_id| *user_id > 0)
            .is_none()
        {
            return Err(invalid_credential(
                "Bilibili credential has an invalid user ID",
            ));
        }
        if self.dede_user_id_ck_md5.len() != 16
            || !self
                .dede_user_id_ck_md5
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(invalid_credential(
                "Bilibili credential has an invalid user checksum",
            ));
        }
        if self.bili_jct.len() != 32 || !self.bili_jct.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(invalid_credential(
                "Bilibili credential has an invalid CSRF token",
            ));
        }
        for (name, value, limit) in [
            ("session", self.sessdata.as_str(), 4096),
            ("refresh token", self.refresh_token.as_str(), 4096),
            ("sid", self.sid.as_deref().unwrap_or(""), 128),
        ] {
            if value.is_empty() && name != "sid" {
                return Err(invalid_credential(format!(
                    "Bilibili credential is missing its {name}"
                )));
            }
            if value.len() > limit
                || value
                    .bytes()
                    .any(|byte| byte.is_ascii_control() || byte == b';')
            {
                return Err(invalid_credential(format!(
                    "Bilibili credential contains invalid {name} data"
                )));
            }
        }
        Ok(self)
    }

    pub(crate) fn user_id(&self) -> &str {
        &self.dede_user_id
    }

    fn cookie_header(&self) -> String {
        let mut cookies = vec![
            format!("DedeUserID={}", self.dede_user_id),
            format!("DedeUserID__ckMd5={}", self.dede_user_id_ck_md5),
            format!("SESSDATA={}", self.sessdata),
            format!("bili_jct={}", self.bili_jct),
        ];
        if let Some(sid) = &self.sid {
            cookies.push(format!("sid={sid}"));
        }
        cookies.join("; ")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BilibiliSessionStatus {
    pub authenticated: bool,
    pub user_id: Option<String>,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub extensions: BTreeMap<String, Value>,
}

impl fmt::Debug for BilibiliQrStart {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BilibiliQrStart")
            .field("qrcode_key", &"[redacted]")
            .field("url", &"[redacted]")
            .field("has_image_data_url", &true)
            .finish()
    }
}

#[derive(Deserialize)]
struct PassportResponse<T> {
    code: i64,
    #[serde(default)]
    message: String,
    data: Option<T>,
}

#[derive(Deserialize)]
struct QrGenerateData {
    url: String,
    qrcode_key: String,
}

#[derive(Deserialize)]
struct QrPollData {
    #[serde(default)]
    url: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    timestamp: Option<u64>,
    code: i64,
    #[serde(default)]
    message: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct CookieInfoData {
    refresh: bool,
    timestamp: u64,
}

#[derive(Deserialize)]
struct CookieRefreshData {
    status: i64,
    #[serde(default)]
    message: String,
    refresh_token: String,
}

#[derive(Deserialize)]
struct LogoutResponse {
    code: i64,
    #[serde(default)]
    status: Option<bool>,
    #[serde(default)]
    message: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct NavData {
    #[serde(default, alias = "isLogin")]
    is_login: bool,
    #[serde(default)]
    email_verified: Option<i64>,
    #[serde(default)]
    face: Option<String>,
    #[serde(default)]
    level_info: Option<NavLevelInfo>,
    #[serde(default)]
    mid: Option<u64>,
    #[serde(default)]
    mobile_verified: Option<i64>,
    #[serde(default)]
    money: Option<f64>,
    #[serde(default)]
    moral: Option<f64>,
    #[serde(default)]
    official: Option<NavOfficial>,
    #[serde(default, alias = "officialVerify")]
    official_verify: Option<NavOfficialVerify>,
    #[serde(default)]
    pendant: Option<NavPendant>,
    #[serde(default)]
    scores: Option<i64>,
    #[serde(default)]
    uname: Option<String>,
    #[serde(default, alias = "vipDueDate")]
    vip_due_date_ms: Option<u64>,
    #[serde(default, alias = "vipStatus")]
    vip_status: Option<i64>,
    #[serde(default, alias = "vipType")]
    vip_type: Option<i64>,
    #[serde(default)]
    vip_pay_type: Option<i64>,
    #[serde(default)]
    vip_theme_type: Option<i64>,
    #[serde(default)]
    vip_label: Option<NavVipLabel>,
    #[serde(default)]
    vip_avatar_subscript: Option<i64>,
    #[serde(default)]
    vip_nickname_color: Option<String>,
    #[serde(default)]
    vip: Option<NavVip>,
    #[serde(default)]
    wallet: Option<NavWallet>,
    #[serde(default)]
    has_shop: Option<bool>,
    #[serde(default)]
    shop_url: Option<String>,
    #[serde(default)]
    allowance_count: Option<i64>,
    #[serde(default)]
    answer_status: Option<i64>,
    #[serde(default)]
    is_senior_member: Option<i64>,
    #[serde(default)]
    wbi_img: Option<NavWbiImage>,
    #[serde(default)]
    is_jury: Option<bool>,
    #[serde(default, flatten, skip_serializing)]
    _extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NavLevelInfo {
    current_level: i64,
    current_min: i64,
    current_exp: i64,
    next_exp: NavLevelThreshold,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum NavLevelThreshold {
    Number(i64),
    Text(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NavOfficial {
    role: i64,
    title: String,
    desc: String,
    #[serde(rename = "type")]
    kind: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NavOfficialVerify {
    #[serde(rename = "type")]
    kind: i64,
    desc: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NavPendant {
    pid: i64,
    name: String,
    image: String,
    expire: i64,
    #[serde(default)]
    image_enhance: Option<String>,
    #[serde(default)]
    image_enhance_frame: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NavVipLabel {
    #[serde(default)]
    path: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    label_theme: String,
    #[serde(default)]
    text_color: String,
    #[serde(default)]
    bg_style: Option<i64>,
    #[serde(default)]
    bg_color: String,
    #[serde(default)]
    border_color: String,
    #[serde(default)]
    use_img_label: Option<bool>,
    #[serde(default)]
    img_label_uri_hans: String,
    #[serde(default)]
    img_label_uri_hant: String,
    #[serde(default)]
    img_label_uri_hans_static: String,
    #[serde(default)]
    img_label_uri_hant_static: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NavVip {
    #[serde(rename = "type")]
    kind: i64,
    status: i64,
    due_date: u64,
    vip_pay_type: i64,
    theme_type: i64,
    label: NavVipLabel,
    avatar_subscript: i64,
    nickname_color: String,
    role: i64,
    avatar_subscript_url: String,
    tv_vip_status: i64,
    tv_vip_pay_type: i64,
    tv_due_date: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NavWallet {
    mid: u64,
    bcoin_balance: f64,
    coupon_balance: f64,
    coupon_due_time: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NavWbiImage {
    img_url: String,
    sub_url: String,
}

impl BilibiliClient {
    pub fn new(config: &BilibiliConfig) -> Result<Self> {
        let mut builder = Client::builder()
            .user_agent(WEB_USER_AGENT)
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20));
        if let Some(proxy_url) = config
            .proxy_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let proxy = Proxy::all(proxy_url).map_err(|_| {
                TuneWeaveError::invalid_request("Bilibili proxy URL is invalid")
                    .with_platform(Platform::Bilibili)
            })?;
            builder = builder.proxy(proxy);
        }
        let http = builder.build().map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "failed to build Bilibili HTTP client",
            )
            .with_platform(Platform::Bilibili)
        })?;
        Ok(Self { http })
    }

    pub(crate) async fn create_qr_login(&self) -> Result<BilibiliQrStart> {
        let response = self
            .http
            .get(PASSPORT_QR_GENERATE_ENDPOINT)
            .send()
            .await
            .map_err(passport_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(passport_http_error(status));
        }
        let bytes = response.bytes().await.map_err(passport_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili QR response exceeded the size limit",
            ));
        }
        parse_qr_generate_response(&bytes)
    }

    pub(crate) async fn poll_qr_login(&self, qrcode_key: &str) -> Result<BilibiliQrPoll> {
        validate_qrcode_key(qrcode_key)?;
        let mut endpoint = Url::parse(PASSPORT_QR_POLL_ENDPOINT).map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "Bilibili QR poll endpoint is invalid",
            )
            .with_platform(Platform::Bilibili)
        })?;
        endpoint
            .query_pairs_mut()
            .append_pair("qrcode_key", qrcode_key)
            .append_pair("source", "main-fe-header");
        let response = self
            .http
            .get(endpoint)
            .send()
            .await
            .map_err(passport_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(passport_http_error(status));
        }
        let headers = response.headers().clone();
        let bytes = response.bytes().await.map_err(passport_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili QR poll response exceeded the size limit",
            ));
        }
        parse_qr_poll_response(&bytes, &headers)
    }

    pub(crate) async fn session_status(
        &self,
        credential: Option<&BilibiliCredential>,
    ) -> Result<BilibiliSessionStatus> {
        let mut request = self.http.get(NAV_ENDPOINT);
        if let Some(credential) = credential {
            request = request.header(COOKIE, credential.cookie_header());
        }
        let response = request.send().await.map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error("Bilibili session endpoint", status));
        }
        let bytes = response.bytes().await.map_err(bilibili_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili session response exceeded the size limit",
            ));
        }
        parse_session_response(&bytes, credential.map(BilibiliCredential::user_id))
    }

    pub(crate) async fn refresh_credential(
        &self,
        credential: &BilibiliCredential,
    ) -> Result<BilibiliCredentialRefresh> {
        let info = self.cookie_refresh_info(credential).await?;
        if !info.refresh {
            let status = require_authenticated_session(
                self.session_status(Some(credential)).await?,
                "Bilibili credential is no longer authenticated",
            )?;
            return Ok(BilibiliCredentialRefresh {
                credential: credential.clone(),
                status,
                refreshed: false,
            });
        }
        if info.timestamp == 0 {
            return Err(bilibili_upstream_error(
                "Bilibili cookie refresh did not return a valid timestamp",
            ));
        }

        let correspond_path = cookie_correspond_path(info.timestamp)?;
        let refresh_csrf = self
            .cookie_refresh_csrf(credential, &correspond_path)
            .await?;
        let refreshed = self.rotate_cookie(credential, &refresh_csrf).await?;
        if refreshed.user_id() != credential.user_id() {
            return Err(bilibili_upstream_error(
                "Bilibili cookie refresh returned a different account identity",
            ));
        }
        self.confirm_cookie_refresh(&refreshed, &credential.refresh_token)
            .await?;
        let status = require_authenticated_session(
            self.session_status(Some(&refreshed)).await?,
            "Bilibili refreshed credential was not authenticated",
        )?;
        Ok(BilibiliCredentialRefresh {
            credential: refreshed,
            status,
            refreshed: true,
        })
    }

    pub(crate) async fn logout(
        &self,
        credential: &BilibiliCredential,
    ) -> Result<BilibiliLogoutOutcome> {
        let response = self
            .http
            .post(LOGOUT_ENDPOINT)
            .header(COOKIE, credential.cookie_header())
            .form(&[("biliCSRF", credential.bili_jct.as_str())])
            .send()
            .await
            .map_err(passport_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(passport_http_error(status));
        }
        let bytes = response.bytes().await.map_err(passport_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili logout response exceeded the size limit",
            ));
        }
        parse_logout_response(&bytes)
    }

    async fn cookie_refresh_info(&self, credential: &BilibiliCredential) -> Result<CookieInfoData> {
        let response = self
            .http
            .get(COOKIE_INFO_ENDPOINT)
            .header(COOKIE, credential.cookie_header())
            .send()
            .await
            .map_err(passport_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(passport_http_error(status));
        }
        let bytes = response.bytes().await.map_err(passport_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili cookie status response exceeded the size limit",
            ));
        }
        parse_cookie_info_response(&bytes)
    }

    async fn cookie_refresh_csrf(
        &self,
        credential: &BilibiliCredential,
        correspond_path: &str,
    ) -> Result<String> {
        validate_correspond_path(correspond_path)?;
        let endpoint = format!("{COOKIE_CORRESPOND_ENDPOINT}{correspond_path}");
        let response = self
            .http
            .get(endpoint)
            .header(COOKIE, credential.cookie_header())
            .send()
            .await
            .map_err(passport_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(passport_http_error(status));
        }
        let bytes = response.bytes().await.map_err(passport_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili cookie correspondence response exceeded the size limit",
            ));
        }
        parse_refresh_csrf_html(&bytes)
    }

    async fn rotate_cookie(
        &self,
        credential: &BilibiliCredential,
        refresh_csrf: &str,
    ) -> Result<BilibiliCredential> {
        validate_refresh_csrf(refresh_csrf)?;
        let response = self
            .http
            .post(COOKIE_REFRESH_ENDPOINT)
            .header(COOKIE, credential.cookie_header())
            .form(&[
                ("csrf", credential.bili_jct.as_str()),
                ("refresh_csrf", refresh_csrf),
                ("source", "main_web"),
                ("refresh_token", credential.refresh_token.as_str()),
            ])
            .send()
            .await
            .map_err(passport_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(passport_http_error(status));
        }
        let headers = response.headers().clone();
        let bytes = response.bytes().await.map_err(passport_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili cookie refresh response exceeded the size limit",
            ));
        }
        parse_cookie_refresh_response(&bytes, &headers, credential.user_id())
    }

    async fn confirm_cookie_refresh(
        &self,
        credential: &BilibiliCredential,
        previous_refresh_token: &str,
    ) -> Result<()> {
        let response = self
            .http
            .post(COOKIE_CONFIRM_ENDPOINT)
            .header(COOKIE, credential.cookie_header())
            .form(&[
                ("csrf", credential.bili_jct.as_str()),
                ("refresh_token", previous_refresh_token),
            ])
            .send()
            .await
            .map_err(passport_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(passport_http_error(status));
        }
        let bytes = response.bytes().await.map_err(passport_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili cookie confirmation response exceeded the size limit",
            ));
        }
        parse_cookie_confirm_response(&bytes)
    }
}

fn parse_cookie_info_response(bytes: &[u8]) -> Result<CookieInfoData> {
    let response: PassportResponse<CookieInfoData> =
        serde_json::from_slice(bytes).map_err(|_| {
            bilibili_upstream_error("Bilibili cookie status endpoint returned invalid JSON")
        })?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili cookie status check",
            response.code,
            &response.message,
        ));
    }
    response
        .data
        .ok_or_else(|| bilibili_upstream_error("Bilibili cookie status did not contain data"))
}

fn cookie_correspond_path(timestamp_ms: u64) -> Result<String> {
    if timestamp_ms == 0 {
        return Err(bilibili_upstream_error(
            "Bilibili cookie refresh timestamp was invalid",
        ));
    }
    let modulus_bytes = BASE64_URL_SAFE_NO_PAD
        .decode(COOKIE_REFRESH_RSA_MODULUS_BASE64URL)
        .map_err(|_| bilibili_internal_error("Bilibili cookie refresh key is invalid"))?;
    if modulus_bytes.len() != 128 {
        return Err(bilibili_internal_error(
            "Bilibili cookie refresh key has an invalid size",
        ));
    }
    let plaintext = format!("refresh_{timestamp_ms}");
    let seed = rand::random::<[u8; 32]>();
    let encoded = oaep_sha256_encode(plaintext.as_bytes(), &seed, modulus_bytes.len())?;
    let modulus = BigUint::from_bytes_be(&modulus_bytes);
    let message = BigUint::from_bytes_be(&encoded);
    if message >= modulus {
        return Err(bilibili_internal_error(
            "Bilibili cookie refresh message exceeded its public key",
        ));
    }
    let encrypted = message.modpow(&BigUint::from(COOKIE_REFRESH_RSA_EXPONENT), &modulus);
    let encrypted = encrypted.to_bytes_be();
    if encrypted.len() > modulus_bytes.len() {
        return Err(bilibili_internal_error(
            "Bilibili cookie refresh ciphertext exceeded its public key",
        ));
    }
    let mut padded = vec![0_u8; modulus_bytes.len() - encrypted.len()];
    padded.extend_from_slice(&encrypted);
    let path = hex::encode(padded);
    validate_correspond_path(&path)?;
    Ok(path)
}

fn oaep_sha256_encode(message: &[u8], seed: &[u8; 32], encoded_len: usize) -> Result<Vec<u8>> {
    const HASH_LEN: usize = 32;
    let minimum = HASH_LEN
        .checked_mul(2)
        .and_then(|value| value.checked_add(2))
        .ok_or_else(|| bilibili_internal_error("Bilibili OAEP size calculation overflowed"))?;
    let padding_len = encoded_len
        .checked_sub(minimum)
        .and_then(|value| value.checked_sub(message.len()))
        .ok_or_else(|| {
            bilibili_internal_error("Bilibili cookie refresh message is too large for its key")
        })?;

    let mut database = Vec::with_capacity(encoded_len - HASH_LEN - 1);
    database.extend_from_slice(digest(&SHA256, b"").as_ref());
    database.resize(database.len() + padding_len, 0);
    database.push(1);
    database.extend_from_slice(message);

    let database_mask = mgf1_sha256(seed, database.len())?;
    for (value, mask) in database.iter_mut().zip(database_mask) {
        *value ^= mask;
    }
    let seed_mask = mgf1_sha256(&database, HASH_LEN)?;
    let mut masked_seed = *seed;
    for (value, mask) in masked_seed.iter_mut().zip(seed_mask) {
        *value ^= mask;
    }

    let mut encoded = Vec::with_capacity(encoded_len);
    encoded.push(0);
    encoded.extend_from_slice(&masked_seed);
    encoded.extend_from_slice(&database);
    if encoded.len() != encoded_len {
        return Err(bilibili_internal_error(
            "Bilibili OAEP encoding produced an invalid size",
        ));
    }
    Ok(encoded)
}

fn mgf1_sha256(seed: &[u8], output_len: usize) -> Result<Vec<u8>> {
    let blocks = output_len.div_ceil(32);
    if blocks > u32::MAX as usize {
        return Err(bilibili_internal_error(
            "Bilibili OAEP mask length exceeded its limit",
        ));
    }
    let mut output = Vec::with_capacity(blocks.saturating_mul(32));
    let mut input = Vec::with_capacity(seed.len().saturating_add(4));
    for counter in 0..blocks {
        input.clear();
        input.extend_from_slice(seed);
        input.extend_from_slice(&(counter as u32).to_be_bytes());
        output.extend_from_slice(digest(&SHA256, &input).as_ref());
    }
    output.truncate(output_len);
    Ok(output)
}

fn validate_correspond_path(value: &str) -> Result<()> {
    if value.len() != 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(bilibili_upstream_error(
            "Bilibili cookie correspondence path was invalid",
        ));
    }
    Ok(())
}

fn parse_refresh_csrf_html(bytes: &[u8]) -> Result<String> {
    let html = std::str::from_utf8(bytes).map_err(|_| {
        bilibili_upstream_error("Bilibili cookie correspondence returned invalid text")
    })?;
    let attribute = ["id=\"1-name\"", "id='1-name'"]
        .into_iter()
        .filter_map(|marker| html.find(marker))
        .min()
        .ok_or_else(|| {
            bilibili_upstream_error(
                "Bilibili cookie correspondence did not contain a refresh token",
            )
        })?;
    let content_start = html[attribute..]
        .find('>')
        .map(|offset| attribute + offset + 1)
        .ok_or_else(|| {
            bilibili_upstream_error("Bilibili cookie correspondence returned malformed HTML")
        })?;
    let content_end = html[content_start..]
        .find("</div>")
        .map(|offset| content_start + offset)
        .ok_or_else(|| {
            bilibili_upstream_error("Bilibili cookie correspondence returned malformed HTML")
        })?;
    let refresh_csrf = html[content_start..content_end].trim().to_owned();
    validate_refresh_csrf(&refresh_csrf)?;
    Ok(refresh_csrf)
}

fn validate_refresh_csrf(value: &str) -> Result<()> {
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(bilibili_upstream_error(
            "Bilibili cookie correspondence returned an invalid refresh token",
        ));
    }
    Ok(())
}

fn parse_cookie_refresh_response(
    bytes: &[u8],
    headers: &HeaderMap,
    expected_user_id: &str,
) -> Result<BilibiliCredential> {
    let response: PassportResponse<CookieRefreshData> =
        serde_json::from_slice(bytes).map_err(|_| {
            bilibili_upstream_error("Bilibili cookie refresh endpoint returned invalid JSON")
        })?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili cookie refresh",
            response.code,
            &response.message,
        ));
    }
    let data = response
        .data
        .ok_or_else(|| bilibili_upstream_error("Bilibili cookie refresh did not contain data"))?;
    if data.status != 0 {
        return Err(platform_business_error(
            "Bilibili cookie refresh",
            data.status,
            &data.message,
        ));
    }
    let mut values = response_cookie_pairs(headers)?;
    let credential = BilibiliCredential {
        dede_user_id: take_cookie(&mut values, "DedeUserID")?,
        dede_user_id_ck_md5: take_cookie(&mut values, "DedeUserID__ckMd5")?,
        sessdata: take_cookie(&mut values, "SESSDATA")?,
        bili_jct: take_cookie(&mut values, "bili_jct")?,
        sid: values.remove("sid"),
        refresh_token: data.refresh_token,
    }
    .normalize()
    .map_err(|_| bilibili_upstream_error("Bilibili cookie refresh returned invalid credentials"))?;
    if credential.user_id() != expected_user_id {
        return Err(bilibili_upstream_error(
            "Bilibili cookie refresh returned a different account identity",
        ));
    }
    Ok(credential)
}

fn parse_cookie_confirm_response(bytes: &[u8]) -> Result<()> {
    let response: PassportResponse<Value> = serde_json::from_slice(bytes).map_err(|_| {
        bilibili_upstream_error("Bilibili cookie confirmation endpoint returned invalid JSON")
    })?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili cookie confirmation",
            response.code,
            &response.message,
        ));
    }
    Ok(())
}

fn parse_logout_response(bytes: &[u8]) -> Result<BilibiliLogoutOutcome> {
    let response = match serde_json::from_slice::<LogoutResponse>(bytes) {
        Ok(response) => response,
        Err(_) if looks_like_html(bytes) => return Ok(BilibiliLogoutOutcome::CredentialExpired),
        Err(_) => {
            return Err(bilibili_upstream_error(
                "Bilibili logout endpoint returned invalid JSON",
            ));
        }
    };
    if response.code == -101 {
        return Ok(BilibiliLogoutOutcome::CredentialExpired);
    }
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili logout",
            response.code,
            &response.message,
        ));
    }
    if response.status != Some(true) {
        return Err(bilibili_upstream_error(
            "Bilibili logout did not confirm success",
        ));
    }
    Ok(BilibiliLogoutOutcome::LoggedOut)
}

fn looks_like_html(bytes: &[u8]) -> bool {
    let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    prefix.contains("<!doctype html") || prefix.contains("<html")
}

fn require_authenticated_session(
    status: BilibiliSessionStatus,
    message: &str,
) -> Result<BilibiliSessionStatus> {
    if status.authenticated {
        Ok(status)
    } else {
        Err(
            TuneWeaveError::new(ErrorCode::AuthenticationRequired, message)
                .with_platform(Platform::Bilibili),
        )
    }
}

fn parse_qr_generate_response(bytes: &[u8]) -> Result<BilibiliQrStart> {
    let response: PassportResponse<QrGenerateData> = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili QR endpoint returned invalid JSON"))?;
    if response.code != 0 {
        let message = response.message.trim();
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            if message.is_empty() {
                format!("Bilibili QR creation failed with code {}", response.code)
            } else {
                format!("Bilibili QR creation failed: {message}")
            },
        )
        .with_platform(Platform::Bilibili)
        .with_details(json!({ "platform_code": response.code })));
    }
    let data = response
        .data
        .ok_or_else(|| bilibili_upstream_error("Bilibili QR response did not contain data"))?;
    validate_qrcode_key(&data.qrcode_key)?;
    validate_qr_login_url(&data.url, &data.qrcode_key)?;
    let image_data_url = qr_image_data_url(&data.url)?;
    Ok(BilibiliQrStart {
        qrcode_key: data.qrcode_key,
        url: data.url,
        image_data_url,
    })
}

fn parse_qr_poll_response(bytes: &[u8], headers: &HeaderMap) -> Result<BilibiliQrPoll> {
    let response: PassportResponse<QrPollData> = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili QR poll endpoint returned invalid JSON"))?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili QR polling",
            response.code,
            &response.message,
        ));
    }
    let data = response
        .data
        .ok_or_else(|| bilibili_upstream_error("Bilibili QR poll response did not contain data"))?;
    match data.code {
        86101 => Ok(BilibiliQrPoll::Waiting),
        86090 => Ok(BilibiliQrPoll::Scanned),
        86038 => Ok(BilibiliQrPoll::Expired),
        0 => {
            let credential =
                credential_from_qr_confirmation(headers, &data.url, &data.refresh_token)?;
            Ok(BilibiliQrPoll::Confirmed {
                credential,
                timestamp_ms: data.timestamp.filter(|timestamp| *timestamp > 0),
            })
        }
        code => Ok(BilibiliQrPoll::Failed {
            code,
            message: if data.message.trim().is_empty() {
                format!("Bilibili QR login failed with code {code}")
            } else {
                data.message
            },
        }),
    }
}

fn parse_session_response(
    bytes: &[u8],
    expected_user_id: Option<&str>,
) -> Result<BilibiliSessionStatus> {
    let response: PassportResponse<NavData> = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili session endpoint returned invalid JSON"))?;
    if response.code == -101 {
        return Ok(unauthenticated_session(
            expected_user_id,
            response.code,
            None,
        ));
    }
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili session check",
            response.code,
            &response.message,
        ));
    }
    let data = response
        .data
        .ok_or_else(|| bilibili_upstream_error("Bilibili session response did not contain data"))?;
    if !data.is_login {
        return Ok(unauthenticated_session(
            expected_user_id,
            response.code,
            Some(data),
        ));
    }
    let user_id = data
        .mid
        .filter(|mid| *mid > 0)
        .map(|mid| mid.to_string())
        .ok_or_else(|| {
            bilibili_upstream_error(
                "Bilibili authenticated session did not contain a valid user ID",
            )
        })?;
    if expected_user_id.is_some_and(|expected| expected != user_id) {
        return Err(bilibili_upstream_error(
            "Bilibili session user does not match the selected credential",
        ));
    }
    let nickname = validated_display_text(data.uname.as_deref(), "nickname")?;
    let avatar_url = validated_image_url(data.face.as_deref(), "avatar")?;
    validate_binary_flag(data.email_verified, "email verification")?;
    validate_binary_flag(data.mobile_verified, "mobile verification")?;
    validate_binary_flag(data.vip_status, "VIP status")?;
    let mut extensions = session_extensions(&data)?;
    extensions.insert("platform_code".to_owned(), json!(response.code));
    Ok(BilibiliSessionStatus {
        authenticated: true,
        user_id: Some(user_id),
        nickname,
        avatar_url,
        extensions,
    })
}

fn unauthenticated_session(
    expected_user_id: Option<&str>,
    platform_code: i64,
    data: Option<NavData>,
) -> BilibiliSessionStatus {
    let mut extensions = BTreeMap::from([("platform_code".to_owned(), json!(platform_code))]);
    if let Some(data) = data
        && let Ok(value) = serde_json::to_value(data)
    {
        extensions.insert("nav".to_owned(), value);
    }
    BilibiliSessionStatus {
        authenticated: false,
        user_id: expected_user_id.map(str::to_owned),
        nickname: None,
        avatar_url: None,
        extensions,
    }
}

fn session_extensions(data: &NavData) -> Result<BTreeMap<String, Value>> {
    let nav = serde_json::to_value(data).map_err(|_| {
        TuneWeaveError::new(
            ErrorCode::InternalError,
            "failed to serialize Bilibili account details",
        )
        .with_platform(Platform::Bilibili)
    })?;
    Ok(BTreeMap::from([("nav".to_owned(), nav)]))
}

fn validate_binary_flag(value: Option<i64>, context: &str) -> Result<()> {
    if value.is_some_and(|value| !matches!(value, 0 | 1)) {
        return Err(bilibili_upstream_error(format!(
            "Bilibili session returned an invalid {context} flag"
        )));
    }
    Ok(())
}

fn validated_display_text(value: Option<&str>, context: &str) -> Result<Option<String>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if value.len() > 512 || value.chars().any(char::is_control) {
        return Err(bilibili_upstream_error(format!(
            "Bilibili session returned an invalid {context}"
        )));
    }
    Ok(Some(value.to_owned()))
}

fn validated_image_url(value: Option<&str>, context: &str) -> Result<Option<String>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let url = Url::parse(value).map_err(|_| {
        bilibili_upstream_error(format!(
            "Bilibili session returned an invalid {context} URL"
        ))
    })?;
    let trusted_host = url
        .host_str()
        .is_some_and(|host| host == "hdslb.com" || host.ends_with(".hdslb.com"));
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !trusted_host
    {
        return Err(bilibili_upstream_error(format!(
            "Bilibili session returned an unsafe {context} URL"
        )));
    }
    Ok(Some(value.to_owned()))
}

fn credential_from_qr_confirmation(
    headers: &HeaderMap,
    confirmation_url: &str,
    refresh_token: &str,
) -> Result<BilibiliCredential> {
    let mut values = response_cookie_pairs(headers)?;
    let missing_required_cookie = ["DedeUserID", "DedeUserID__ckMd5", "SESSDATA", "bili_jct"]
        .into_iter()
        .any(|name| !values.contains_key(name));
    if missing_required_cookie && !confirmation_url.trim().is_empty() {
        for (name, value) in trusted_confirmation_query(confirmation_url)? {
            values.entry(name).or_insert(value);
        }
    }
    BilibiliCredential {
        dede_user_id: take_cookie(&mut values, "DedeUserID")?,
        dede_user_id_ck_md5: take_cookie(&mut values, "DedeUserID__ckMd5")?,
        sessdata: take_cookie(&mut values, "SESSDATA")?,
        bili_jct: take_cookie(&mut values, "bili_jct")?,
        sid: values.remove("sid"),
        refresh_token: refresh_token.to_owned(),
    }
    .normalize()
    .map_err(|_| {
        bilibili_upstream_error("Bilibili confirmed QR login returned invalid credentials")
    })
}

fn response_cookie_pairs(headers: &HeaderMap) -> Result<BTreeMap<String, String>> {
    let mut cookies = BTreeMap::new();
    for header in headers.get_all(SET_COOKIE) {
        let value = header.to_str().map_err(|_| {
            bilibili_upstream_error("Bilibili response returned an invalid cookie header")
        })?;
        let Some((name, value)) = value
            .split(';')
            .next()
            .and_then(|pair| pair.split_once('='))
        else {
            continue;
        };
        if matches!(
            name,
            "DedeUserID" | "DedeUserID__ckMd5" | "SESSDATA" | "bili_jct" | "sid"
        ) && !value.is_empty()
        {
            cookies.insert(name.to_owned(), value.to_owned());
        }
    }
    Ok(cookies)
}

fn trusted_confirmation_query(value: &str) -> Result<BTreeMap<String, String>> {
    let url = Url::parse(value)
        .map_err(|_| bilibili_upstream_error("Bilibili QR confirmation returned an invalid URL"))?;
    let trusted_target = matches!(
        (url.host_str(), url.path()),
        (Some("passport.biligame.com"), "/crossDomain")
            | (
                Some("passport.bilibili.com"),
                "/x/passport-login/web/crossDomain"
            )
    );
    if url.scheme() != "https" || url.port().is_some() || !trusted_target {
        return Err(bilibili_upstream_error(
            "Bilibili QR confirmation returned an untrusted URL",
        ));
    }
    Ok(url
        .query_pairs()
        .filter(|(name, value)| {
            matches!(
                name.as_ref(),
                "DedeUserID" | "DedeUserID__ckMd5" | "SESSDATA" | "bili_jct" | "sid"
            ) && !value.is_empty()
        })
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect())
}

fn take_cookie(values: &mut BTreeMap<String, String>, name: &str) -> Result<String> {
    values.remove(name).ok_or_else(|| {
        bilibili_upstream_error(format!(
            "Bilibili authentication response did not return {name}"
        ))
    })
}

fn validate_qrcode_key(value: &str) -> Result<()> {
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(bilibili_upstream_error(
            "Bilibili QR response contained an invalid key",
        ));
    }
    Ok(())
}

fn validate_qr_login_url(value: &str, qrcode_key: &str) -> Result<()> {
    let url = Url::parse(value)
        .map_err(|_| bilibili_upstream_error("Bilibili QR response contained an invalid URL"))?;
    let trusted_path = matches!(
        (url.host_str(), url.path()),
        (Some("passport.bilibili.com"), "/h5-app/passport/login/scan")
            | (Some("account.bilibili.com"), "/h5/account-h5/auth/scan-web")
    );
    let trusted = url.scheme() == "https" && url.port().is_none() && trusted_path;
    let matching_key = url
        .query_pairs()
        .any(|(name, value)| name == "qrcode_key" && value == qrcode_key);
    if !trusted || !matching_key {
        return Err(bilibili_upstream_error(
            "Bilibili QR response contained an untrusted login URL",
        ));
    }
    Ok(())
}

fn qr_image_data_url(url: &str) -> Result<String> {
    let code = QrCode::new(url.as_bytes()).map_err(|_| {
        bilibili_upstream_error("Bilibili QR login URL could not be encoded as an image")
    })?;
    let image = code
        .render::<svg::Color>()
        .min_dimensions(320, 320)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build();
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        BASE64.encode(image.as_bytes())
    ))
}

fn passport_network_error(error: reqwest::Error) -> TuneWeaveError {
    let code = if error.is_timeout() {
        ErrorCode::UpstreamTimeout
    } else {
        ErrorCode::UpstreamError
    };
    TuneWeaveError::new(code, "Bilibili Passport request failed")
        .with_platform(Platform::Bilibili)
        .retryable(true)
}

fn passport_http_error(status: StatusCode) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("Bilibili Passport returned HTTP {status}"),
    )
    .with_platform(Platform::Bilibili)
    .retryable(status.is_server_error())
}

fn bilibili_network_error(error: reqwest::Error) -> TuneWeaveError {
    let code = if error.is_timeout() {
        ErrorCode::UpstreamTimeout
    } else {
        ErrorCode::UpstreamError
    };
    TuneWeaveError::new(code, "Bilibili API request failed")
        .with_platform(Platform::Bilibili)
        .retryable(true)
}

fn bilibili_http_error(context: &str, status: StatusCode) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("{context} returned HTTP {status}"),
    )
    .with_platform(Platform::Bilibili)
    .retryable(status.is_server_error())
}

fn platform_business_error(context: &str, code: i64, message: &str) -> TuneWeaveError {
    let error_code = match code {
        -101 | -111 | -400 | 2202 | 86038 | 86095 => ErrorCode::AuthenticationRequired,
        -412 => ErrorCode::RateLimited,
        _ => ErrorCode::UpstreamError,
    };
    let message = if message.trim().is_empty() {
        format!("{context} failed with code {code}")
    } else {
        format!("{context} failed: {message}")
    };
    TuneWeaveError::new(error_code, message)
        .with_platform(Platform::Bilibili)
        .retryable(matches!(code, -412))
        .with_details(json!({ "platform_code": code }))
}

fn bilibili_upstream_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message)
        .with_platform(Platform::Bilibili)
        .retryable(true)
}

fn bilibili_internal_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::InternalError, message).with_platform(Platform::Bilibili)
}

fn invalid_credential(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Bilibili)
}

#[cfg(test)]
mod tests {
    use super::*;

    const QR_KEY: &str = "8587cf8106a0b863c46d6bab913537f6";

    fn qr_fixture(url: &str, key: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "code": 0,
            "message": "0",
            "ttl": 1,
            "data": {
                "url": url,
                "qrcode_key": key
            }
        }))
        .expect("serialize QR fixture")
    }

    fn poll_fixture(code: i64, message: &str, url: &str, refresh_token: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "code": 0,
            "message": "0",
            "ttl": 1,
            "data": {
                "url": url,
                "refresh_token": refresh_token,
                "timestamp": if code == 0 { 1_662_363_009_601_u64 } else { 0 },
                "code": code,
                "message": message
            }
        }))
        .expect("serialize QR poll fixture")
    }

    fn nav_fixture(code: i64, data: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "code": code,
            "message": if code == 0 { "0" } else { "账号未登录" },
            "ttl": 1,
            "data": data
        }))
        .expect("serialize nav fixture")
    }

    fn rotated_cookie_headers(user_id: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for cookie in [
            format!("DedeUserID={user_id}; Path=/"),
            "DedeUserID__ckMd5=abcdef0123456789; Path=/".to_owned(),
            "SESSDATA=rotated%2Csession; Path=/; HttpOnly; Secure".to_owned(),
            "bili_jct=abcdef0123456789abcdef0123456789; Path=/".to_owned(),
            "sid=rotated-sid; Path=/".to_owned(),
        ] {
            headers.append(SET_COOKIE, cookie.parse().expect("cookie header"));
        }
        headers
    }

    #[test]
    fn qr_creation_maps_the_reference_response_without_exposing_the_key() {
        let url = format!(
            "https://passport.bilibili.com/h5-app/passport/login/scan?navhide=1&qrcode_key={QR_KEY}&from="
        );
        let start = parse_qr_generate_response(&qr_fixture(&url, QR_KEY)).expect("valid QR start");
        assert_eq!(start.qrcode_key, QR_KEY);
        assert_eq!(start.url, url);
        assert!(
            start
                .image_data_url
                .starts_with("data:image/svg+xml;base64,")
        );
        let debug = format!("{start:?}");
        assert!(!debug.contains(QR_KEY));
        assert!(!debug.contains("passport/login/scan"));

        let current_url = format!(
            "https://account.bilibili.com/h5/account-h5/auth/scan-web?navhide=1&callback=close&qrcode_key={QR_KEY}&from="
        );
        let current = parse_qr_generate_response(&qr_fixture(&current_url, QR_KEY))
            .expect("current QR start");
        assert_eq!(current.url, current_url);
    }

    #[test]
    fn qr_creation_rejects_untrusted_urls_keys_and_business_errors() {
        for (url, key) in [
            (
                format!("https://example.com/h5-app/passport/login/scan?qrcode_key={QR_KEY}"),
                QR_KEY,
            ),
            (
                "https://passport.bilibili.com/h5-app/passport/login/scan?qrcode_key=other"
                    .to_owned(),
                QR_KEY,
            ),
            (
                "https://passport.bilibili.com/h5-app/passport/login/scan?qrcode_key=short"
                    .to_owned(),
                "short",
            ),
        ] {
            let error = parse_qr_generate_response(&qr_fixture(&url, key))
                .expect_err("invalid QR response must fail");
            assert_eq!(error.code, ErrorCode::UpstreamError);
        }

        let error =
            parse_qr_generate_response(br#"{"code":-1,"message":"rate limited","data":null}"#)
                .expect_err("business error must fail");
        assert_eq!(error.details["platform_code"], -1);
    }

    #[test]
    fn qr_poll_preserves_all_reference_states() {
        let headers = HeaderMap::new();
        assert_eq!(
            parse_qr_poll_response(&poll_fixture(86101, "未扫码", "", ""), &headers)
                .expect("waiting state"),
            BilibiliQrPoll::Waiting
        );
        assert_eq!(
            parse_qr_poll_response(&poll_fixture(86090, "二维码已扫码未确认", "", ""), &headers)
                .expect("scanned state"),
            BilibiliQrPoll::Scanned
        );
        assert_eq!(
            parse_qr_poll_response(&poll_fixture(86038, "二维码已失效", "", ""), &headers)
                .expect("expired state"),
            BilibiliQrPoll::Expired
        );
        assert_eq!(
            parse_qr_poll_response(&poll_fixture(12345, "unknown state", "", ""), &headers)
                .expect("failed state"),
            BilibiliQrPoll::Failed {
                code: 12345,
                message: "unknown state".to_owned()
            }
        );
    }

    #[test]
    fn qr_confirmation_prefers_response_cookies_and_redacts_credentials() {
        let mut headers = HeaderMap::new();
        for cookie in [
            "SESSDATA=private%2Csession; Path=/; HttpOnly; Secure",
            "bili_jct=0123456789abcdef0123456789abcdef; Path=/",
            "DedeUserID=47275982; Path=/",
            "DedeUserID__ckMd5=0123456789abcdef; Path=/",
            "sid=private-sid; Path=/",
        ] {
            headers.append(SET_COOKIE, cookie.parse().expect("cookie header"));
        }
        let outcome = parse_qr_poll_response(
            &poll_fixture(
                0,
                "",
                "https://passport.biligame.com/crossDomain?DedeUserID=999&SESSDATA=fallback",
                "private-refresh",
            ),
            &headers,
        )
        .expect("confirmed state");
        let BilibiliQrPoll::Confirmed {
            credential,
            timestamp_ms,
        } = outcome
        else {
            panic!("expected confirmed QR state");
        };
        assert_eq!(credential.user_id(), "47275982");
        assert_eq!(credential.sessdata, "private%2Csession");
        assert_eq!(timestamp_ms, Some(1_662_363_009_601));
        let debug = format!("{credential:?}");
        assert!(!debug.contains("private"));
        assert!(
            !format!(
                "{:?}",
                BilibiliQrPoll::Confirmed {
                    credential,
                    timestamp_ms
                }
            )
            .contains("private")
        );
    }

    #[test]
    fn qr_confirmation_rejects_missing_or_untrusted_credential_sources() {
        let headers = HeaderMap::new();
        for url in [
            "",
            "https://example.com/crossDomain?DedeUserID=47275982",
            "http://passport.biligame.com/crossDomain?DedeUserID=47275982",
        ] {
            let error =
                parse_qr_poll_response(&poll_fixture(0, "", url, "private-refresh"), &headers)
                    .expect_err("incomplete confirmation must fail");
            assert_eq!(error.code, ErrorCode::UpstreamError);
            assert!(!format!("{error:?}").contains("private-refresh"));
        }
    }

    #[test]
    fn credential_validation_rejects_zero_padded_zero_user_ids() {
        let error = BilibiliCredential {
            dede_user_id: "00".to_owned(),
            dede_user_id_ck_md5: "0123456789abcdef".to_owned(),
            sessdata: "session".to_owned(),
            bili_jct: "0123456789abcdef0123456789abcdef".to_owned(),
            sid: None,
            refresh_token: "refresh".to_owned(),
        }
        .normalize()
        .expect_err("zero-valued user IDs must fail");
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn session_status_maps_authenticated_and_anonymous_responses() {
        let authenticated = parse_session_response(
            &nav_fixture(
                0,
                json!({
                    "isLogin": true,
                    "email_verified": 1,
                    "mobile_verified": 0,
                    "face": "https://i0.hdslb.com/bfs/face/avatar.jpg",
                    "mid": 47275982,
                    "uname": "Lotus",
                    "vipDueDate": 1_700_000_000_000_u64,
                    "vipStatus": 1,
                    "vipType": 2,
                    "wbi_img": {
                        "img_url": "https://i0.hdslb.com/bfs/wbi/image.png",
                        "sub_url": "https://i0.hdslb.com/bfs/wbi/sub.png"
                    },
                    "new_upstream_field": { "preserved_in_parser_only": true }
                }),
            ),
            Some("47275982"),
        )
        .expect("authenticated nav response");
        assert!(authenticated.authenticated);
        assert_eq!(authenticated.user_id.as_deref(), Some("47275982"));
        assert_eq!(authenticated.nickname.as_deref(), Some("Lotus"));
        assert_eq!(
            authenticated.avatar_url.as_deref(),
            Some("https://i0.hdslb.com/bfs/face/avatar.jpg")
        );
        assert_eq!(authenticated.extensions["platform_code"], 0);
        assert_eq!(authenticated.extensions["nav"]["vip_status"], 1);
        assert!(
            authenticated.extensions["nav"]
                .get("new_upstream_field")
                .is_none()
        );

        let anonymous = parse_session_response(
            &nav_fixture(-101, json!({ "isLogin": false })),
            Some("47275982"),
        )
        .expect("anonymous nav response");
        assert!(!anonymous.authenticated);
        assert_eq!(anonymous.user_id.as_deref(), Some("47275982"));
        assert_eq!(anonymous.extensions["platform_code"], -101);
    }

    #[test]
    fn session_status_rejects_identity_mismatch_flags_and_unsafe_avatar_urls() {
        for (data, expected_message) in [
            (
                json!({ "isLogin": true, "mid": 1, "uname": "other" }),
                "does not match",
            ),
            (
                json!({ "isLogin": true, "mid": 47275982, "email_verified": 2 }),
                "verification flag",
            ),
            (
                json!({
                    "isLogin": true,
                    "mid": 47275982,
                    "face": "javascript:alert(1)"
                }),
                "avatar URL",
            ),
        ] {
            let error = parse_session_response(&nav_fixture(0, data), Some("47275982"))
                .expect_err("invalid session response must fail");
            assert_eq!(error.code, ErrorCode::UpstreamError);
            assert!(error.message.contains(expected_message));
        }
    }

    #[test]
    fn session_cookie_excludes_refresh_material_and_debug_redacts_secrets() {
        let credential = BilibiliCredential {
            dede_user_id: "47275982".to_owned(),
            dede_user_id_ck_md5: "0123456789abcdef".to_owned(),
            sessdata: "private-session".to_owned(),
            bili_jct: "0123456789abcdef0123456789abcdef".to_owned(),
            sid: Some("private-sid".to_owned()),
            refresh_token: "private-refresh".to_owned(),
        }
        .normalize()
        .expect("credential");
        let cookie = credential.cookie_header();
        assert!(cookie.contains("SESSDATA=private-session"));
        assert!(cookie.contains("sid=private-sid"));
        assert!(!cookie.contains("private-refresh"));
        let debug = format!("{credential:?}");
        assert!(!debug.contains("private"));
    }

    #[test]
    fn cookie_refresh_status_preserves_required_flag_and_server_timestamp() {
        let needed = parse_cookie_info_response(
            br#"{"code":0,"message":"0","data":{"refresh":true,"timestamp":1684466082562}}"#,
        )
        .expect("refresh status");
        assert!(needed.refresh);
        assert_eq!(needed.timestamp, 1_684_466_082_562);

        let current = parse_cookie_info_response(
            br#"{"code":0,"message":"0","data":{"refresh":false,"timestamp":1684466082562}}"#,
        )
        .expect("current status");
        assert!(!current.refresh);

        let error =
            parse_cookie_info_response(br#"{"code":-101,"message":"not logged in","data":null}"#)
                .expect_err("expired cookie must fail refresh status");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[test]
    fn correspond_path_uses_fixed_rsa_oaep_shape() {
        let first = cookie_correspond_path(1_684_466_082_562).expect("first path");
        let second = cookie_correspond_path(1_684_466_082_562).expect("second path");
        assert_eq!(first.len(), 256);
        assert!(
            first
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        );
        assert_ne!(first, second, "OAEP encryption must use fresh randomness");
    }

    #[test]
    fn cookie_correspondence_extracts_only_a_valid_refresh_csrf() {
        let csrf = parse_refresh_csrf_html(
            br#"<html><body><div class="token" id="1-name">b0cc8411ded2f9db2cff2edb3123acac</div></body></html>"#,
        )
        .expect("refresh csrf");
        assert_eq!(csrf, "b0cc8411ded2f9db2cff2edb3123acac");

        for invalid in [
            br#"<html><div id="other">b0cc8411ded2f9db2cff2edb3123acac</div></html>"#.as_slice(),
            br#"<html><div id='1-name'>not-a-token</div></html>"#.as_slice(),
            b"\xff\xfe".as_slice(),
        ] {
            assert!(parse_refresh_csrf_html(invalid).is_err());
        }
    }

    #[test]
    fn cookie_rotation_requires_a_complete_same_identity_generation() {
        let response = br#"{"code":0,"message":"0","data":{"status":0,"message":"","refresh_token":"rotated-refresh"}}"#;
        let credential = parse_cookie_refresh_response(
            response,
            &rotated_cookie_headers("47275982"),
            "47275982",
        )
        .expect("rotated credential");
        assert_eq!(credential.user_id(), "47275982");
        assert_eq!(credential.sessdata, "rotated%2Csession");
        assert_eq!(credential.refresh_token, "rotated-refresh");
        assert!(!format!("{credential:?}").contains("rotated-refresh"));

        let mismatch =
            parse_cookie_refresh_response(response, &rotated_cookie_headers("999"), "47275982")
                .expect_err("identity mismatch must fail");
        assert!(mismatch.message.contains("different account identity"));

        let missing = parse_cookie_refresh_response(response, &HeaderMap::new(), "47275982")
            .expect_err("missing cookies must fail");
        assert_eq!(missing.code, ErrorCode::UpstreamError);
    }

    #[test]
    fn cookie_confirmation_and_logout_preserve_terminal_semantics() {
        parse_cookie_confirm_response(br#"{"code":0,"message":"0","ttl":1}"#)
            .expect("refresh confirmation");
        let confirmation_error =
            parse_cookie_confirm_response(br#"{"code":-111,"message":"csrf rejected","ttl":1}"#)
                .expect_err("confirmation failure");
        assert_eq!(confirmation_error.code, ErrorCode::AuthenticationRequired);

        assert_eq!(
            parse_logout_response(br#"{"code":0,"status":true,"ts":1663034005}"#)
                .expect("logged out"),
            BilibiliLogoutOutcome::LoggedOut
        );
        assert_eq!(
            parse_logout_response(b"<!DOCTYPE html><html><body>login</body></html>")
                .expect("expired HTML session"),
            BilibiliLogoutOutcome::CredentialExpired
        );
        assert_eq!(
            parse_logout_response(br#"{"code":-101,"message":"not logged in"}"#)
                .expect("expired JSON session"),
            BilibiliLogoutOutcome::CredentialExpired
        );
        let csrf_error =
            parse_logout_response(br#"{"code":2202,"status":false,"message":"csrf rejected"}"#)
                .expect_err("invalid csrf must fail without claiming logout");
        assert_eq!(csrf_error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili Passport access"]
    async fn live_qr_creation_returns_a_trusted_scannable_url() {
        let client = BilibiliClient::new(&BilibiliConfig::default()).expect("Bilibili client");
        let start = client.create_qr_login().await.expect("live QR creation");
        validate_qrcode_key(&start.qrcode_key).expect("valid live QR key");
        validate_qr_login_url(&start.url, &start.qrcode_key).expect("trusted live QR URL");
        assert!(
            start
                .image_data_url
                .starts_with("data:image/svg+xml;base64,")
        );
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili Passport access"]
    async fn live_new_qr_poll_starts_in_the_waiting_state() {
        let client = BilibiliClient::new(&BilibiliConfig::default()).expect("Bilibili client");
        let start = client.create_qr_login().await.expect("live QR creation");
        let state = client
            .poll_qr_login(&start.qrcode_key)
            .await
            .expect("live QR polling");
        assert_eq!(state, BilibiliQrPoll::Waiting);
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili API access"]
    async fn live_anonymous_session_reports_logged_out() {
        let client = BilibiliClient::new(&BilibiliConfig::default()).expect("Bilibili client");
        let session = client
            .session_status(None)
            .await
            .expect("live anonymous session status");
        assert!(!session.authenticated);
        assert!(session.user_id.is_none());
    }
}
