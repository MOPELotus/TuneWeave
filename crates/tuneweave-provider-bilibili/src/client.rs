use std::{collections::BTreeMap, fmt, sync::Arc, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use qrcode::{QrCode, render::svg};
use reqwest::{
    Client, Proxy, StatusCode,
    header::{HeaderMap, SET_COOKIE},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tuneweave_core::{AccountCredentialStore, ErrorCode, Platform, Result, TuneWeaveError};
use url::Url;

const PASSPORT_QR_GENERATE_ENDPOINT: &str =
    "https://passport.bilibili.com/x/passport-login/web/qrcode/generate?source=main-fe-header";
const PASSPORT_QR_POLL_ENDPOINT: &str =
    "https://passport.bilibili.com/x/passport-login/web/qrcode/poll";
const WEB_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const MAX_PASSPORT_RESPONSE_BYTES: usize = 1024 * 1024;

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
            bilibili_upstream_error("Bilibili QR confirmation returned an invalid cookie header")
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
        bilibili_upstream_error(format!("Bilibili confirmed QR login did not return {name}"))
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

fn platform_business_error(context: &str, code: i64, message: &str) -> TuneWeaveError {
    let error_code = match code {
        -101 | -111 | -400 | 86038 => ErrorCode::AuthenticationRequired,
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
}
