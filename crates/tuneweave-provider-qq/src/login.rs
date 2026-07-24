use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use reqwest::{Response, StatusCode, Url, header};
use serde_json::json;
use tokio::sync::Mutex as AsyncMutex;
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};

use crate::client::{QqApiRequest, QqClient, QqCredential};

const QQ_QR_SHOW_ENDPOINT: &str = "https://ssl.ptlogin2.qq.com/ptqrshow";
const QQ_QR_POLL_ENDPOINT: &str = "https://ssl.ptlogin2.qq.com/ptqrlogin";
const QQ_CHECK_SIG_ENDPOINT: &str = "https://ssl.ptlogin2.graph.qq.com/check_sig";
const QQ_OAUTH_ENDPOINT: &str = "https://graph.qq.com/oauth2.0/authorize";
const QQ_LOGIN_REFERER: &str = "https://xui.ptlogin2.qq.com/";
const WECHAT_QR_CONNECT_ENDPOINT: &str = "https://open.weixin.qq.com/connect/qrconnect";
const WECHAT_QR_IMAGE_ROOT: &str = "https://open.weixin.qq.com/connect/qrcode/";
const WECHAT_QR_POLL_ENDPOINT: &str = "https://lp.open.weixin.qq.com/connect/l/qrconnect";
const QR_TRANSACTION_TTL: Duration = Duration::from_secs(10 * 60);
const LOGIN_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const WECHAT_POLL_TIMEOUT: Duration = Duration::from_secs(35);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QqQrLoginKind {
    Qq,
    Wechat,
}

#[derive(Clone)]
pub(crate) struct QqQrStart {
    pub provider_transaction_id: String,
    pub image_mime: &'static str,
    pub image: Vec<u8>,
}

#[derive(Clone)]
pub(crate) enum QqQrPollOutcome {
    Waiting,
    Scanned,
    Confirmed(Box<QqCredential>),
    Expired,
    Failed(String),
}

#[derive(Clone, Default)]
pub(crate) struct QqQrTransactions {
    entries: Arc<Mutex<BTreeMap<String, QqQrEntry>>>,
}

#[derive(Clone)]
struct QqQrEntry {
    expires_at: Instant,
    state: Arc<AsyncMutex<QqQrTransaction>>,
}

struct QqQrTransaction {
    kind: QqQrLoginKind,
    identifier: String,
    cookies: BTreeMap<String, String>,
    terminal: Option<QqQrPollOutcome>,
}

struct CreatedQr {
    identifier: String,
    image_mime: &'static str,
    image: Vec<u8>,
    cookies: BTreeMap<String, String>,
}

impl QqQrTransactions {
    pub(crate) async fn start(&self, client: &QqClient, kind: QqQrLoginKind) -> Result<QqQrStart> {
        let created = match kind {
            QqQrLoginKind::Qq => create_qq_qr(client).await?,
            QqQrLoginKind::Wechat => create_wechat_qr(client).await?,
        };
        let transaction_id = self.insert(QqQrTransaction {
            kind,
            identifier: created.identifier,
            cookies: created.cookies,
            terminal: None,
        })?;
        Ok(QqQrStart {
            provider_transaction_id: transaction_id,
            image_mime: created.image_mime,
            image: created.image,
        })
    }

    pub(crate) async fn poll(
        &self,
        client: &QqClient,
        transaction_id: &str,
    ) -> Result<QqQrPollOutcome> {
        let state = self.entry(transaction_id)?;
        let mut transaction = state.lock().await;
        if let Some(outcome) = transaction.terminal.clone() {
            return Ok(outcome);
        }
        let outcome = match transaction.kind {
            QqQrLoginKind::Qq => poll_qq_qr(client, &mut transaction).await?,
            QqQrLoginKind::Wechat => poll_wechat_qr(client, &mut transaction).await?,
        };
        if matches!(
            outcome,
            QqQrPollOutcome::Confirmed(_) | QqQrPollOutcome::Expired | QqQrPollOutcome::Failed(_)
        ) {
            transaction.terminal = Some(outcome.clone());
        }
        Ok(outcome)
    }

    fn insert(&self, transaction: QqQrTransaction) -> Result<String> {
        let mut entries = self.entries.lock().map_err(|_| qr_store_error())?;
        let now = Instant::now();
        entries.retain(|_, entry| entry.expires_at > now);
        for _ in 0..8 {
            let transaction_id = format!("qq-qr-{}", hex::encode(rand::random::<[u8; 16]>()));
            if !entries.contains_key(&transaction_id) {
                entries.insert(
                    transaction_id.clone(),
                    QqQrEntry {
                        expires_at: now + QR_TRANSACTION_TTL,
                        state: Arc::new(AsyncMutex::new(transaction)),
                    },
                );
                return Ok(transaction_id);
            }
        }
        Err(TuneWeaveError::new(
            ErrorCode::InternalError,
            "failed to allocate a QQ QR login transaction",
        )
        .with_platform(Platform::Qq))
    }

    fn entry(&self, transaction_id: &str) -> Result<Arc<AsyncMutex<QqQrTransaction>>> {
        let mut entries = self.entries.lock().map_err(|_| qr_store_error())?;
        let now = Instant::now();
        entries.retain(|_, entry| entry.expires_at > now);
        entries
            .get(transaction_id)
            .map(|entry| entry.state.clone())
            .ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::ResourceNotFound,
                    "QQ QR login transaction was not found or has expired",
                )
                .with_platform(Platform::Qq)
            })
    }
}

async fn create_qq_qr(client: &QqClient) -> Result<CreatedQr> {
    let mut endpoint = fixed_url(QQ_QR_SHOW_ENDPOINT)?;
    endpoint.query_pairs_mut().extend_pairs([
        ("appid", "716027609"),
        ("e", "2"),
        ("l", "M"),
        ("s", "3"),
        ("d", "72"),
        ("v", "4"),
        ("t", &format!("0.{}", rand::random::<u64>())),
        ("daid", "383"),
        ("pt_3rd_aid", "100497308"),
    ]);
    let response = client
        .login_http()
        .get(endpoint)
        .header(header::REFERER, QQ_LOGIN_REFERER)
        .timeout(LOGIN_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(login_network_error)?;
    ensure_login_http_status(response.status(), "QQ QR image")?;
    let cookies = response_cookies(&response)?;
    let identifier = cookies
        .get("qrsig")
        .cloned()
        .ok_or_else(|| login_data_error("QQ QR image response is missing qrsig"))?;
    let image = response
        .bytes()
        .await
        .map_err(login_network_error)?
        .to_vec();
    ensure_png(&image, "QQ QR image")?;
    Ok(CreatedQr {
        identifier,
        image_mime: "image/png",
        image,
        cookies,
    })
}

async fn create_wechat_qr(client: &QqClient) -> Result<CreatedQr> {
    let mut endpoint = fixed_url(WECHAT_QR_CONNECT_ENDPOINT)?;
    endpoint.query_pairs_mut().extend_pairs([
        ("appid", "wx48db31d50e334801"),
        (
            "redirect_uri",
            "https://y.qq.com/portal/wx_redirect.html?login_type=2&surl=https://y.qq.com/",
        ),
        ("response_type", "code"),
        ("scope", "snsapi_login"),
        ("state", "STATE"),
        (
            "href",
            "https://y.qq.com/mediastyle/music_v17/src/css/popup_wechat.css#wechat_redirect",
        ),
    ]);
    let response = client
        .login_http()
        .get(endpoint)
        .timeout(LOGIN_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(login_network_error)?;
    ensure_login_http_status(response.status(), "WeChat QR bootstrap")?;
    let mut cookies = response_cookies(&response)?;
    let html = response.text().await.map_err(login_network_error)?;
    let identifier = parse_wechat_uuid(&html)?;
    let image_endpoint = fixed_url(&format!("{WECHAT_QR_IMAGE_ROOT}{identifier}"))?;
    let response = client
        .login_http()
        .get(image_endpoint)
        .header(header::REFERER, WECHAT_QR_CONNECT_ENDPOINT)
        .header(header::COOKIE, cookie_header(&cookies)?)
        .timeout(LOGIN_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(login_network_error)?;
    ensure_login_http_status(response.status(), "WeChat QR image")?;
    merge_response_cookies(&mut cookies, &response)?;
    let image = response
        .bytes()
        .await
        .map_err(login_network_error)?
        .to_vec();
    ensure_jpeg(&image, "WeChat QR image")?;
    Ok(CreatedQr {
        identifier,
        image_mime: "image/jpeg",
        image,
        cookies,
    })
}

async fn poll_qq_qr(
    client: &QqClient,
    transaction: &mut QqQrTransaction,
) -> Result<QqQrPollOutcome> {
    let mut endpoint = fixed_url(QQ_QR_POLL_ENDPOINT)?;
    let token = hash33(&transaction.identifier, 0);
    let action = format!("0-0-{}", unix_millis()?);
    endpoint.query_pairs_mut().extend_pairs([
        ("u1", "https://graph.qq.com/oauth2.0/login_jump"),
        ("ptqrtoken", &token.to_string()),
        ("ptredirect", "0"),
        ("h", "1"),
        ("t", "1"),
        ("g", "1"),
        ("from_ui", "1"),
        ("ptlang", "2052"),
        ("action", &action),
        ("js_ver", "20102616"),
        ("js_type", "1"),
        ("pt_uistyle", "40"),
        ("aid", "716027609"),
        ("daid", "383"),
        ("pt_3rd_aid", "100497308"),
        ("has_onekey", "1"),
    ]);
    let response = client
        .login_http()
        .get(endpoint)
        .header(header::REFERER, QQ_LOGIN_REFERER)
        .header(header::COOKIE, cookie_header(&transaction.cookies)?)
        .timeout(LOGIN_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(login_network_error)?;
    ensure_login_http_status(response.status(), "QQ QR status")?;
    merge_response_cookies(&mut transaction.cookies, &response)?;
    let body = response.text().await.map_err(login_network_error)?;
    let args = parse_qq_status_arguments(&body)?;
    let status = args[0]
        .parse::<u16>()
        .map_err(|_| login_data_error("QQ QR status returned an invalid state code"))?;
    match status {
        66 => Ok(QqQrPollOutcome::Waiting),
        67 => Ok(QqQrPollOutcome::Scanned),
        65 => Ok(QqQrPollOutcome::Expired),
        68 => Ok(QqQrPollOutcome::Failed(
            "QQ QR login was refused".to_owned(),
        )),
        0 => {
            let callback = args.get(2).ok_or_else(|| {
                login_data_error("QQ QR success response is missing callback URL")
            })?;
            let callback = Url::parse(callback)
                .map_err(|_| login_data_error("QQ QR success callback URL is invalid"))?;
            let query = callback.query_pairs().collect::<BTreeMap<_, _>>();
            let uin = query
                .get("uin")
                .map(|value| value.as_ref())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| login_data_error("QQ QR success callback is missing uin"))?;
            let sigx = query
                .get("ptsigx")
                .map(|value| value.as_ref())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| login_data_error("QQ QR success callback is missing ptsigx"))?;
            let credential = authorize_qq_qr(client, uin, sigx, &mut transaction.cookies).await?;
            Ok(QqQrPollOutcome::Confirmed(Box::new(credential)))
        }
        _ => Err(login_data_error(format!(
            "QQ QR status returned unsupported state code {status}"
        ))),
    }
}

async fn poll_wechat_qr(
    client: &QqClient,
    transaction: &mut QqQrTransaction,
) -> Result<QqQrPollOutcome> {
    let mut endpoint = fixed_url(WECHAT_QR_POLL_ENDPOINT)?;
    endpoint.query_pairs_mut().extend_pairs([
        ("uuid", transaction.identifier.as_str()),
        ("_", &unix_millis()?.to_string()),
    ]);
    let response = client
        .login_http()
        .get(endpoint)
        .header(header::REFERER, "https://open.weixin.qq.com/")
        .header(header::COOKIE, cookie_header(&transaction.cookies)?)
        .timeout(WECHAT_POLL_TIMEOUT)
        .send()
        .await;
    let response = match response {
        Ok(response) => response,
        Err(error) if error.is_timeout() => return Ok(QqQrPollOutcome::Waiting),
        Err(error) => return Err(login_network_error(error)),
    };
    ensure_login_http_status(response.status(), "WeChat QR status")?;
    merge_response_cookies(&mut transaction.cookies, &response)?;
    let body = response.text().await.map_err(login_network_error)?;
    let (status, code) = parse_wechat_status(&body)?;
    match status {
        408 => Ok(QqQrPollOutcome::Waiting),
        404 => Ok(QqQrPollOutcome::Scanned),
        402 => Ok(QqQrPollOutcome::Expired),
        403 => Ok(QqQrPollOutcome::Failed(
            "WeChat QR login was refused".to_owned(),
        )),
        405 => {
            let code = code
                .filter(|value| !value.is_empty())
                .ok_or_else(|| login_data_error("WeChat QR confirmation is missing OAuth code"))?;
            let credential = authorize_wechat_qr(client, code).await?;
            Ok(QqQrPollOutcome::Confirmed(Box::new(credential)))
        }
        _ => Err(login_data_error(format!(
            "WeChat QR status returned unsupported state code {status}"
        ))),
    }
}

async fn authorize_qq_qr(
    client: &QqClient,
    uin: &str,
    sigx: &str,
    cookies: &mut BTreeMap<String, String>,
) -> Result<QqCredential> {
    let mut endpoint = fixed_url(QQ_CHECK_SIG_ENDPOINT)?;
    endpoint.query_pairs_mut().extend_pairs([
        ("uin", uin),
        ("pttype", "1"),
        ("service", "ptqrlogin"),
        ("nodirect", "0"),
        ("ptsigx", sigx),
        ("s_url", "https://graph.qq.com/oauth2.0/login_jump"),
        ("ptlang", "2052"),
        ("ptredirect", "100"),
        ("aid", "716027609"),
        ("daid", "383"),
        ("j_later", "0"),
        ("low_login_hour", "0"),
        ("regmaster", "0"),
        ("pt_login_type", "3"),
        ("pt_aid", "0"),
        ("pt_aaid", "16"),
        ("pt_light", "0"),
        ("pt_3rd_aid", "100497308"),
    ]);
    let response = client
        .login_http()
        .get(endpoint)
        .header(header::REFERER, QQ_LOGIN_REFERER)
        .header(header::COOKIE, cookie_header(cookies)?)
        .timeout(LOGIN_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(login_network_error)?;
    ensure_redirect_or_success(response.status(), "QQ login signature exchange")?;
    merge_response_cookies(cookies, &response)?;
    let p_skey = cookies
        .get("p_skey")
        .cloned()
        .ok_or_else(|| login_data_error("QQ login signature exchange is missing p_skey"))?;

    let response = client
        .login_http()
        .post(QQ_OAUTH_ENDPOINT)
        .header(header::COOKIE, cookie_header(cookies)?)
        .form(&[
            ("response_type", "code".to_owned()),
            ("client_id", "100497308".to_owned()),
            (
                "redirect_uri",
                "https://y.qq.com/portal/wx_redirect.html?login_type=1&surl=https://y.qq.com/"
                    .to_owned(),
            ),
            ("scope", "get_user_info,get_app_friends".to_owned()),
            ("state", "state".to_owned()),
            ("switch", String::new()),
            ("from_ptlogin", "1".to_owned()),
            ("src", "1".to_owned()),
            ("update_auth", "1".to_owned()),
            ("openapi", "1010_1030".to_owned()),
            ("g_tk", hash33(&p_skey, 5381).to_string()),
            ("auth_time", unix_millis()?.to_string()),
            ("ui", random_uuid_v4()),
        ])
        .timeout(LOGIN_REQUEST_TIMEOUT)
        .send()
        .await
        .map_err(login_network_error)?;
    ensure_redirect_or_success(response.status(), "QQ OAuth authorization")?;
    merge_response_cookies(cookies, &response)?;
    let location = response
        .headers()
        .get(header::LOCATION)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| login_data_error("QQ OAuth authorization is missing redirect location"))?;
    let location = Url::parse(location)
        .map_err(|_| login_data_error("QQ OAuth redirect location is invalid"))?;
    let code = location
        .query_pairs()
        .find_map(|(name, value)| (name == "code").then(|| value.into_owned()))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| login_data_error("QQ OAuth redirect location is missing code"))?;

    let response = client
        .request_android_login(
            QqApiRequest::new(
                "QQConnectLogin.LoginServer",
                "QQLogin",
                json!({ "code": code }),
            ),
            2,
            None,
        )
        .await?;
    parse_login_credential(response.data)
}

async fn authorize_wechat_qr(client: &QqClient, code: &str) -> Result<QqCredential> {
    let response = client
        .request_android_login(
            QqApiRequest::new(
                "music.login.LoginServer",
                "Login",
                json!({
                    "code": code,
                    "strAppid": "wx48db31d50e334801"
                }),
            ),
            1,
            None,
        )
        .await?;
    parse_login_credential(response.data)
}

fn parse_login_credential(value: serde_json::Value) -> Result<QqCredential> {
    serde_json::from_value::<QqCredential>(value)
        .map_err(|_| login_data_error("QQ login returned a malformed credential"))?
        .normalize()
}

fn parse_qq_status_arguments(body: &str) -> Result<Vec<String>> {
    let start = body
        .find("ptuiCB(")
        .map(|index| index + "ptuiCB(".len())
        .ok_or_else(|| login_data_error("QQ QR status response is missing ptuiCB"))?;
    let end = body[start..]
        .find(')')
        .map(|index| start + index)
        .ok_or_else(|| login_data_error("QQ QR status response has an unterminated ptuiCB"))?;
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for character in body[start..end].chars() {
        if !quoted {
            if character == '\'' {
                quoted = true;
                current.clear();
            }
            continue;
        }
        if escaped {
            current.push(character);
            escaped = false;
        } else if character == '\\' {
            escaped = true;
        } else if character == '\'' {
            quoted = false;
            args.push(current.clone());
        } else {
            current.push(character);
        }
    }
    if quoted || escaped || args.is_empty() {
        return Err(login_data_error(
            "QQ QR status response has malformed callback arguments",
        ));
    }
    Ok(args)
}

fn parse_wechat_uuid(html: &str) -> Result<String> {
    let start = html
        .find("uuid=")
        .map(|index| index + "uuid=".len())
        .ok_or_else(|| login_data_error("WeChat QR bootstrap response is missing uuid"))?;
    let identifier = html[start..]
        .split(['"', '\'', '&', '<', '>', ' '])
        .next()
        .unwrap_or_default();
    validate_identifier(identifier, "WeChat QR uuid")?;
    Ok(identifier.to_owned())
}

fn parse_wechat_status(body: &str) -> Result<(u16, Option<&str>)> {
    let marker = "window.wx_errcode=";
    let start = body
        .find(marker)
        .map(|index| index + marker.len())
        .ok_or_else(|| login_data_error("WeChat QR status response is missing wx_errcode"))?;
    let status_end = body[start..]
        .find(';')
        .map(|index| start + index)
        .ok_or_else(|| login_data_error("WeChat QR status response has an invalid wx_errcode"))?;
    let status = body[start..status_end]
        .parse::<u16>()
        .map_err(|_| login_data_error("WeChat QR status response has a non-numeric wx_errcode"))?;
    let code_marker = "window.wx_code='";
    let code = body[status_end..].find(code_marker).and_then(|index| {
        let start = status_end + index + code_marker.len();
        body[start..]
            .find('\'')
            .map(|end| &body[start..start + end])
    });
    Ok((status, code))
}

fn response_cookies(response: &Response) -> Result<BTreeMap<String, String>> {
    let mut cookies = BTreeMap::new();
    merge_response_cookies(&mut cookies, response)?;
    Ok(cookies)
}

fn merge_response_cookies(
    cookies: &mut BTreeMap<String, String>,
    response: &Response,
) -> Result<()> {
    for value in response.headers().get_all(header::SET_COOKIE) {
        let value = value
            .to_str()
            .map_err(|_| login_data_error("QQ login response contains an invalid Set-Cookie"))?;
        let pair = value.split(';').next().unwrap_or_default();
        let (name, value) = pair
            .split_once('=')
            .ok_or_else(|| login_data_error("QQ login response contains a malformed cookie"))?;
        validate_cookie_name(name)?;
        validate_cookie_value(value)?;
        if value.is_empty() {
            cookies.remove(name);
        } else {
            cookies.insert(name.to_owned(), value.to_owned());
        }
    }
    Ok(())
}

fn cookie_header(cookies: &BTreeMap<String, String>) -> Result<String> {
    cookies
        .iter()
        .map(|(name, value)| {
            validate_cookie_name(name)?;
            validate_cookie_value(value)?;
            Ok(format!("{name}={value}"))
        })
        .collect::<Result<Vec<_>>>()
        .map(|pairs| pairs.join("; "))
}

fn validate_cookie_name(name: &str) -> Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(login_data_error(
            "QQ login response contains an unsafe cookie name",
        ));
    }
    Ok(())
}

fn validate_cookie_value(value: &str) -> Result<()> {
    if value.len() > 4096
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || matches!(byte, b';' | b','))
    {
        return Err(login_data_error(
            "QQ login response contains an unsafe cookie value",
        ));
    }
    Ok(())
}

fn validate_identifier(identifier: &str, context: &str) -> Result<()> {
    if identifier.is_empty()
        || identifier.len() > 256
        || identifier
            .bytes()
            .any(|byte| byte.is_ascii_control() || matches!(byte, b'/' | b'?' | b'#' | b'&'))
    {
        return Err(login_data_error(format!("{context} is invalid")));
    }
    Ok(())
}

fn ensure_png(image: &[u8], context: &str) -> Result<()> {
    if image.starts_with(b"\x89PNG\r\n\x1a\n") {
        Ok(())
    } else {
        Err(login_data_error(format!(
            "{context} did not return a PNG image"
        )))
    }
}

fn ensure_jpeg(image: &[u8], context: &str) -> Result<()> {
    if image.starts_with(&[0xff, 0xd8, 0xff]) {
        Ok(())
    } else {
        Err(login_data_error(format!(
            "{context} did not return a JPEG image"
        )))
    }
}

fn ensure_login_http_status(status: StatusCode, context: &str) -> Result<()> {
    if status.is_success() {
        Ok(())
    } else {
        Err(login_http_error(status, context))
    }
}

fn ensure_redirect_or_success(status: StatusCode, context: &str) -> Result<()> {
    if status.is_success() || status.is_redirection() {
        Ok(())
    } else {
        Err(login_http_error(status, context))
    }
}

fn fixed_url(value: &str) -> Result<Url> {
    Url::parse(value).map_err(|_| {
        TuneWeaveError::new(ErrorCode::InternalError, "QQ login endpoint is invalid")
            .with_platform(Platform::Qq)
    })
}

fn unix_millis() -> Result<u128> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                format!("system clock is before Unix epoch: {error}"),
            )
            .with_platform(Platform::Qq)
        })
}

fn hash33(value: &str, seed: u32) -> u32 {
    value.chars().fold(seed, |hash, character| {
        hash.wrapping_mul(33).wrapping_add(character as u32) & 0x7fff_ffff
    })
}

fn random_uuid_v4() -> String {
    let mut bytes = rand::random::<[u8; 16]>();
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let encoded = hex::encode(bytes);
    format!(
        "{}-{}-{}-{}-{}",
        &encoded[..8],
        &encoded[8..12],
        &encoded[12..16],
        &encoded[16..20],
        &encoded[20..]
    )
}

fn qr_store_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        "QQ QR login transaction store lock is poisoned",
    )
    .with_platform(Platform::Qq)
}

fn login_network_error(error: reqwest::Error) -> TuneWeaveError {
    let timed_out = error.is_timeout();
    let code = if timed_out {
        ErrorCode::UpstreamTimeout
    } else {
        ErrorCode::UpstreamError
    };
    let message = if timed_out {
        "QQ login request timed out"
    } else {
        "QQ login request failed"
    };
    TuneWeaveError::new(code, message)
        .with_platform(Platform::Qq)
        .retryable(true)
}

fn login_http_error(status: StatusCode, context: &str) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("{context} returned HTTP {status}"),
    )
    .with_platform(Platform::Qq)
    .retryable(status.is_server_error())
}

fn login_data_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message)
        .with_platform(Platform::Qq)
        .retryable(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_qq_qr_callback_without_exposing_unquoted_text() {
        let args = parse_qq_status_arguments(
            "ptuiCB('0','0','https://graph.qq.com/?uin=123&service=x&ptsigx=sig&s_url=y','0','ok','nick');",
        )
        .expect("QQ callback");
        assert_eq!(args[0], "0");
        assert!(args[2].contains("ptsigx=sig"));
        assert_eq!(args[5], "nick");
    }

    #[test]
    fn parses_wechat_bootstrap_and_poll_states() {
        assert_eq!(
            parse_wechat_uuid("<iframe src=\"/connect/confirm?uuid=071abcXYZ\"></iframe>")
                .expect("uuid"),
            "071abcXYZ"
        );
        assert_eq!(
            parse_wechat_status("window.wx_errcode=405;window.wx_code='oauth-code'")
                .expect("status"),
            (405, Some("oauth-code"))
        );
    }

    #[test]
    fn hash33_matches_reference_vectors() {
        assert_eq!(hash33("abc", 0), 108_966);
        assert_eq!(hash33("abc", 5381), 193_485_963);
    }

    #[test]
    fn cookie_jar_rejects_header_injection() {
        assert!(validate_cookie_name("p_skey").is_ok());
        assert!(validate_cookie_name("bad name").is_err());
        assert!(validate_cookie_value("safe/value").is_ok());
        assert!(validate_cookie_value("unsafe;next=value").is_err());
        assert!(validate_cookie_value("unsafe\r\nheader").is_err());
    }

    #[test]
    fn generated_oauth_ui_is_a_uuid_v4() {
        let value = random_uuid_v4();
        assert_eq!(value.len(), 36);
        assert_eq!(&value[14..15], "4");
        assert!(matches!(&value[19..20], "8" | "9" | "a" | "b"));
    }

    #[tokio::test]
    #[ignore = "requires live QQ and WeChat login services"]
    async fn live_standard_qr_logins_create_images_and_report_waiting() {
        let client = QqClient::new(crate::QqConfig::default()).expect("QQ client");
        for (kind, mime) in [
            (QqQrLoginKind::Qq, "image/png"),
            (QqQrLoginKind::Wechat, "image/jpeg"),
        ] {
            let transactions = QqQrTransactions::default();
            let start = transactions.start(&client, kind).await.expect("QR start");
            assert_eq!(start.image_mime, mime);
            assert!(start.image.len() > 100);
            let outcome = transactions
                .poll(&client, &start.provider_transaction_id)
                .await
                .expect("QR poll");
            assert!(matches!(
                outcome,
                QqQrPollOutcome::Waiting | QqQrPollOutcome::Scanned
            ));
        }
    }
}
