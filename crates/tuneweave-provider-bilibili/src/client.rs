use std::{fmt, sync::Arc, time::Duration};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use qrcode::{QrCode, render::svg};
use reqwest::{Client, Proxy, StatusCode, redirect::Policy};
use serde::Deserialize;
use serde_json::json;
use tuneweave_core::{AccountCredentialStore, ErrorCode, Platform, Result, TuneWeaveError};
use url::Url;

const PASSPORT_QR_GENERATE_ENDPOINT: &str =
    "https://passport.bilibili.com/x/passport-login/web/qrcode/generate?source=main-fe-header";
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

fn bilibili_upstream_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message)
        .with_platform(Platform::Bilibili)
        .retryable(true)
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
}
