use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use bytes::BytesMut;
use futures_util::{SinkExt, StreamExt};
use reqwest::{Response, StatusCode, Url, header};
use rumqttc::v5::mqttbytes::v5::{
    Connect, ConnectProperties, ConnectReturnCode, Filter, Packet, PingReq, Publish, Subscribe,
    SubscribeProperties, SubscribeReasonCode,
};
use rumqttc::v5::mqttbytes::{Error as MqttCodecError, QoS};
use serde_json::json;
use tokio::time::timeout;
use tokio::{
    sync::{Mutex as AsyncMutex, mpsc},
    task::JoinHandle,
};
use tokio_tungstenite::{
    WebSocketStream,
    tungstenite::{Message, handshake::derive_accept_key, protocol::Role},
};
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError, UpstreamBusinessClass};

use crate::client::{QqApiRequest, QqClient, QqCredential};

const QQ_QR_SHOW_ENDPOINT: &str = "https://ssl.ptlogin2.qq.com/ptqrshow";
const QQ_QR_POLL_ENDPOINT: &str = "https://ssl.ptlogin2.qq.com/ptqrlogin";
const QQ_CHECK_SIG_ENDPOINT: &str = "https://ssl.ptlogin2.graph.qq.com/check_sig";
const QQ_OAUTH_ENDPOINT: &str = "https://graph.qq.com/oauth2.0/authorize";
const QQ_LOGIN_REFERER: &str = "https://xui.ptlogin2.qq.com/";
const WECHAT_QR_CONNECT_ENDPOINT: &str = "https://open.weixin.qq.com/connect/qrconnect";
const WECHAT_QR_IMAGE_ROOT: &str = "https://open.weixin.qq.com/connect/qrcode/";
const WECHAT_QR_POLL_ENDPOINT: &str = "https://lp.open.weixin.qq.com/connect/l/qrconnect";
const MOBILE_MQTT_ROOT: &str = "https://mu.y.qq.com:443";
const MOBILE_MQTT_INITIAL_PATH: &str = "/ws/handshake";
const QR_TRANSACTION_TTL: Duration = Duration::from_secs(10 * 60);
const LOGIN_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const WECHAT_POLL_TIMEOUT: Duration = Duration::from_secs(35);
const MOBILE_MQTT_CONNECT_TIMEOUT: Duration = Duration::from_secs(20);
const MOBILE_MQTT_EVENT_TIMEOUT: Duration = Duration::from_secs(25);
const MOBILE_MQTT_MAX_PACKET_BYTES: u32 = 1024 * 1024;
const MOBILE_MQTT_MAX_REDIRECTS: usize = 3;
const QQ_SIGNATURE_MAX_REDIRECTS: usize = 10;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QqQrLoginKind {
    Qq,
    Wechat,
    Mobile,
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

enum ParsedQqQrPoll {
    Waiting,
    Scanned,
    Confirmed { uin: String, sigx: String },
    Expired,
    Failed,
}

enum ParsedWechatQrPoll {
    Waiting,
    Scanned,
    Confirmed { code: String },
    Expired,
    Failed,
}

enum QqSignatureStep {
    Complete,
    Redirect(Url),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QqLogoutOutcome {
    LoggedOut,
    CredentialExpired,
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
    mobile_listener: Option<MobileQrListener>,
    mobile_credentials: Option<(u64, String)>,
    terminal: Option<QqQrPollOutcome>,
}

struct CreatedQr {
    identifier: String,
    image_mime: &'static str,
    image: Vec<u8>,
    cookies: BTreeMap<String, String>,
}

enum QqPhonePrincipal {
    Plain(String),
    Encrypted(String),
}

impl QqQrTransactions {
    pub(crate) async fn start(&self, client: &QqClient, kind: QqQrLoginKind) -> Result<QqQrStart> {
        let created = match kind {
            QqQrLoginKind::Qq => create_qq_qr(client).await?,
            QqQrLoginKind::Wechat => create_wechat_qr(client).await?,
            QqQrLoginKind::Mobile => create_mobile_qr(client).await?,
        };
        let mobile_listener = if kind == QqQrLoginKind::Mobile {
            Some(start_mobile_listener(client, &created.identifier).await?)
        } else {
            None
        };
        let transaction_id = self.insert(QqQrTransaction {
            kind,
            identifier: created.identifier,
            cookies: created.cookies,
            mobile_listener,
            mobile_credentials: None,
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
            QqQrLoginKind::Qq => poll_qq_qr(client, &transaction).await?,
            QqQrLoginKind::Wechat => poll_wechat_qr(client, &mut transaction).await?,
            QqQrLoginKind::Mobile => poll_mobile_qr(client, &mut transaction).await?,
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
    let request = client
        .login_http()
        .get(endpoint)
        .header(header::REFERER, QQ_LOGIN_REFERER)
        .timeout(LOGIN_REQUEST_TIMEOUT);
    let started = Instant::now();
    let mut http_status = None;
    let outcome = async {
        let response = request.send().await.map_err(login_network_error)?;
        http_status = Some(response.status());
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
    .await;
    client.log_simple_upstream_request(
        "qq_qr_create",
        "ssl.ptlogin2.qq.com",
        "/ptqrshow",
        http_status,
        started,
        &outcome,
    );
    outcome
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
    let request = client
        .login_http()
        .get(endpoint)
        .timeout(LOGIN_REQUEST_TIMEOUT);
    let started = Instant::now();
    let mut http_status = None;
    let bootstrap_outcome = async {
        let response = request.send().await.map_err(login_network_error)?;
        http_status = Some(response.status());
        ensure_login_http_status(response.status(), "WeChat QR bootstrap")?;
        let cookies = response_cookies(&response)?;
        let html = response.text().await.map_err(login_network_error)?;
        let identifier = parse_wechat_uuid(&html)?;
        Ok((identifier, cookies))
    }
    .await;
    client.log_simple_upstream_request(
        "wechat_qr_bootstrap",
        "open.weixin.qq.com",
        "/connect/qrconnect",
        http_status,
        started,
        &bootstrap_outcome,
    );
    let (identifier, mut cookies) = bootstrap_outcome?;
    let image_endpoint = fixed_url(&format!("{WECHAT_QR_IMAGE_ROOT}{identifier}"))?;
    let request = client
        .login_http()
        .get(image_endpoint)
        .header(header::REFERER, WECHAT_QR_CONNECT_ENDPOINT)
        .header(header::COOKIE, cookie_header(&cookies)?)
        .timeout(LOGIN_REQUEST_TIMEOUT);
    let started = Instant::now();
    let mut http_status = None;
    let image_outcome = async {
        let response = request.send().await.map_err(login_network_error)?;
        http_status = Some(response.status());
        ensure_login_http_status(response.status(), "WeChat QR image")?;
        merge_response_cookies(&mut cookies, &response)?;
        let image = response
            .bytes()
            .await
            .map_err(login_network_error)?
            .to_vec();
        ensure_jpeg(&image, "WeChat QR image")?;
        Ok(image)
    }
    .await;
    client.log_simple_upstream_request(
        "wechat_qr_image",
        "open.weixin.qq.com",
        "/connect/qrcode/{uuid}",
        http_status,
        started,
        &image_outcome,
    );
    Ok(CreatedQr {
        identifier,
        image_mime: "image/jpeg",
        image: image_outcome?,
        cookies,
    })
}

async fn create_mobile_qr(client: &QqClient) -> Result<CreatedQr> {
    let response = client
        .request_android_with_comm(
            QqApiRequest::new(
                "music.login.LoginServer",
                "CreateQRCode",
                json!({
                    "tmeAppID": "qqmusic",
                    "ct": 11,
                    "cv": 14090008
                }),
            ),
            &[("ct", json!(23)), ("cv", json!(0))],
        )
        .await?;
    let identifier = response
        .data
        .get("qrcodeID")
        .and_then(value_as_nonempty_string)
        .ok_or_else(|| login_data_error("QQ mobile QR response is missing qrcodeID"))?;
    validate_identifier(&identifier, "QQ mobile QR identifier")?;
    let encoded = response
        .data
        .get("qrcode")
        .and_then(value_as_nonempty_string)
        .ok_or_else(|| login_data_error("QQ mobile QR response is missing image data"))?;
    let encoded = encoded
        .rsplit_once(',')
        .map_or(encoded.as_str(), |(_, data)| data);
    let image = BASE64.decode(encoded).map_err(|_| {
        login_data_error("QQ mobile QR response contains invalid Base64 image data")
    })?;
    ensure_png(&image, "QQ mobile QR image")?;
    Ok(CreatedQr {
        identifier,
        image_mime: "image/png",
        image,
        cookies: BTreeMap::new(),
    })
}

pub(crate) async fn send_phone_authcode(
    client: &QqClient,
    principal: &str,
    country_code: &str,
) -> Result<()> {
    let response = client
        .request_android_business(
            QqApiRequest::new(
                "music.login.LoginServer",
                "SendPhoneAuthCode",
                phone_authcode_send_param(principal, country_code)?,
            ),
            &[("tmeLoginMethod", json!(3))],
        )
        .await?;
    match response.code {
        0 => Ok(()),
        20_276 => {
            let security_url = response
                .data
                .get("securityURL")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty());
            Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "QQ phone login requires security verification",
            )
            .with_platform(Platform::Qq)
            .with_details(json!({
                "platform_code": response.code,
                "security_url": security_url
            })))
        }
        100_001 | 104_604 | 2001 => Err(TuneWeaveError::new(
            ErrorCode::RateLimited,
            "QQ phone verification code requests are rate limited",
        )
        .with_platform(Platform::Qq)
        .retryable(true)
        .with_details(json!({ "platform_code": response.code }))),
        code => Err(qq_login_business_error(
            code,
            "QQ failed to send the phone verification code",
        )),
    }
}

pub(crate) async fn login_with_phone_authcode(
    client: &QqClient,
    principal: &str,
    code: &str,
) -> Result<QqCredential> {
    let response = client
        .request_android_login(
            QqApiRequest::new(
                "music.login.LoginServer",
                "Login",
                phone_authcode_login_param(principal, code)?,
            ),
            0,
            Some(3),
        )
        .await?;
    parse_login_credential(response.data)
}

pub(crate) async fn refresh_qq_credential(
    client: &QqClient,
    credential: &QqCredential,
) -> Result<QqCredential> {
    let response = client
        .request_android_business_with_credential(
            QqApiRequest::new(
                "music.login.LoginServer",
                "Login",
                refresh_credential_param(credential),
            ),
            &[("tmeLoginType", json!(credential.login_type))],
            Some(credential),
        )
        .await?;
    if response.code != 0 {
        return Err(qq_login_business_error(
            response.code,
            "QQ failed to refresh the account credential",
        ));
    }
    parse_login_credential(response.data)
}

pub(crate) async fn logout_qq_credential(
    client: &QqClient,
    credential: &QqCredential,
) -> Result<QqLogoutOutcome> {
    let response = client
        .request_android_business_with_credential(
            QqApiRequest::new("music.login.LoginServer", "Logout", json!({})),
            &[],
            Some(credential),
        )
        .await?;
    logout_outcome(response.code)
}

async fn poll_qq_qr(client: &QqClient, transaction: &QqQrTransaction) -> Result<QqQrPollOutcome> {
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
    let request = client
        .login_http()
        .get(endpoint)
        .header(header::REFERER, QQ_LOGIN_REFERER)
        .header(
            header::COOKIE,
            cookie_header(&qq_qr_poll_cookies(&transaction.identifier)?)?,
        )
        .timeout(LOGIN_REQUEST_TIMEOUT);
    let started = Instant::now();
    let mut http_status = None;
    let poll_outcome = async {
        let response = request.send().await.map_err(login_network_error)?;
        http_status = Some(response.status());
        ensure_login_http_status(response.status(), "QQ QR status")?;
        let body = response.text().await.map_err(login_network_error)?;
        parse_qq_qr_poll(&body)
    }
    .await;
    let success_class = match &poll_outcome {
        Ok(ParsedQqQrPoll::Confirmed { .. }) => UpstreamBusinessClass::Success,
        Ok(_) | Err(_) => UpstreamBusinessClass::AllowedError,
    };
    client.log_typed_upstream_request(
        "qq_qr_poll",
        "ssl.ptlogin2.qq.com",
        "/ptqrlogin",
        http_status,
        started,
        success_class,
        &poll_outcome,
    );
    match poll_outcome? {
        ParsedQqQrPoll::Waiting => Ok(QqQrPollOutcome::Waiting),
        ParsedQqQrPoll::Scanned => Ok(QqQrPollOutcome::Scanned),
        ParsedQqQrPoll::Expired => Ok(QqQrPollOutcome::Expired),
        ParsedQqQrPoll::Failed => Ok(QqQrPollOutcome::Failed(
            "QQ QR login was refused".to_owned(),
        )),
        ParsedQqQrPoll::Confirmed { uin, sigx } => {
            let credential = authorize_qq_qr(client, &uin, &sigx).await?;
            Ok(QqQrPollOutcome::Confirmed(Box::new(credential)))
        }
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
    let request = client
        .login_http()
        .get(endpoint)
        .header(header::REFERER, "https://open.weixin.qq.com/")
        .header(header::COOKIE, cookie_header(&transaction.cookies)?)
        .timeout(WECHAT_POLL_TIMEOUT);
    let started = Instant::now();
    let mut http_status = None;
    let poll_outcome = async {
        let response = match request.send().await {
            Ok(response) => response,
            Err(error) if error.is_timeout() => return Ok(ParsedWechatQrPoll::Waiting),
            Err(error) => return Err(login_network_error(error)),
        };
        http_status = Some(response.status());
        ensure_login_http_status(response.status(), "WeChat QR status")?;
        merge_response_cookies(&mut transaction.cookies, &response)?;
        let body = response.text().await.map_err(login_network_error)?;
        parse_wechat_qr_poll(&body)
    }
    .await;
    let success_class = match &poll_outcome {
        Ok(ParsedWechatQrPoll::Confirmed { .. }) => UpstreamBusinessClass::Success,
        Ok(_) | Err(_) => UpstreamBusinessClass::AllowedError,
    };
    client.log_typed_upstream_request(
        "wechat_qr_poll",
        "lp.open.weixin.qq.com",
        "/connect/l/qrconnect",
        http_status,
        started,
        success_class,
        &poll_outcome,
    );
    match poll_outcome? {
        ParsedWechatQrPoll::Waiting => Ok(QqQrPollOutcome::Waiting),
        ParsedWechatQrPoll::Scanned => Ok(QqQrPollOutcome::Scanned),
        ParsedWechatQrPoll::Expired => Ok(QqQrPollOutcome::Expired),
        ParsedWechatQrPoll::Failed => Ok(QqQrPollOutcome::Failed(
            "WeChat QR login was refused".to_owned(),
        )),
        ParsedWechatQrPoll::Confirmed { code } => {
            let credential = authorize_wechat_qr(client, &code).await?;
            Ok(QqQrPollOutcome::Confirmed(Box::new(credential)))
        }
    }
}

async fn poll_mobile_qr(
    client: &QqClient,
    transaction: &mut QqQrTransaction,
) -> Result<QqQrPollOutcome> {
    if let Some((music_id, token)) = transaction.mobile_credentials.clone() {
        return authorize_mobile_qr(client, &transaction.identifier, music_id, &token).await;
    }
    let listener = transaction.mobile_listener.as_mut().ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::InternalError,
            "QQ mobile QR transaction is missing its MQTT listener",
        )
        .with_platform(Platform::Qq)
    })?;
    let event = match timeout(MOBILE_MQTT_EVENT_TIMEOUT, listener.receiver.recv()).await {
        Err(_) => return Ok(QqQrPollOutcome::Waiting),
        Ok(Some(event)) => event,
        Ok(None) => {
            return Err(TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "QQ mobile QR MQTT listener stopped unexpectedly",
            )
            .with_platform(Platform::Qq)
            .retryable(true));
        }
    };
    match event {
        MobileQrEvent::Scanned => Ok(QqQrPollOutcome::Scanned),
        MobileQrEvent::Canceled => Ok(QqQrPollOutcome::Failed(
            "QQ mobile QR login was canceled".to_owned(),
        )),
        MobileQrEvent::Expired => Ok(QqQrPollOutcome::Expired),
        MobileQrEvent::LoginFailed => Ok(QqQrPollOutcome::Failed(
            "QQ mobile QR login failed".to_owned(),
        )),
        MobileQrEvent::TransportFailed => Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "QQ mobile QR MQTT connection failed",
        )
        .with_platform(Platform::Qq)
        .retryable(true)),
        MobileQrEvent::Credentials { music_id, token } => {
            transaction.mobile_credentials = Some((music_id, token.clone()));
            authorize_mobile_qr(client, &transaction.identifier, music_id, &token).await
        }
    }
}

async fn authorize_mobile_qr(
    client: &QqClient,
    identifier: &str,
    music_id: u64,
    token: &str,
) -> Result<QqQrPollOutcome> {
    let response = client
        .request_android_login(
            QqApiRequest::new(
                "music.login.LoginServer",
                "Login",
                json!({
                    "musicid": music_id,
                    "qrCodeID": identifier,
                    "token": token
                }),
            ),
            6,
            None,
        )
        .await?;
    Ok(QqQrPollOutcome::Confirmed(Box::new(
        parse_login_credential(response.data)?,
    )))
}

struct MobileQrListener {
    receiver: mpsc::Receiver<MobileQrEvent>,
    task: JoinHandle<()>,
}

impl Drop for MobileQrListener {
    fn drop(&mut self) {
        self.task.abort();
    }
}

enum MobileQrEvent {
    Scanned,
    Canceled,
    Expired,
    LoginFailed,
    Credentials { music_id: u64, token: String },
    TransportFailed,
}

struct MobileMqttSocket {
    websocket: WebSocketStream<reqwest::Upgraded>,
    buffered: BytesMut,
}

enum MobileMqttConnectStep {
    Connected(Box<MobileMqttSocket>),
    Redirect(String),
}

impl MobileMqttSocket {
    async fn send_packet(&mut self, packet: Packet) -> Result<()> {
        let mut encoded = BytesMut::with_capacity(packet.size());
        packet
            .write(&mut encoded, Some(MOBILE_MQTT_MAX_PACKET_BYTES))
            .map_err(mqtt_codec_error)?;
        self.websocket
            .send(Message::Binary(encoded.freeze()))
            .await
            .map_err(|_| mobile_mqtt_error("failed to send MQTT packet"))
    }

    async fn next_packet(&mut self) -> Result<Packet> {
        loop {
            match Packet::read(&mut self.buffered, Some(MOBILE_MQTT_MAX_PACKET_BYTES)) {
                Ok(packet) => return Ok(packet),
                Err(MqttCodecError::InsufficientBytes(_)) => {}
                Err(error) => return Err(mqtt_codec_error(error)),
            }
            let message = self
                .websocket
                .next()
                .await
                .ok_or_else(|| mobile_mqtt_error("WebSocket closed without an MQTT packet"))?
                .map_err(|_| mobile_mqtt_error("WebSocket receive failed"))?;
            match message {
                Message::Binary(payload) => self.buffered.extend_from_slice(&payload),
                Message::Ping(payload) => self
                    .websocket
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|_| mobile_mqtt_error("failed to answer WebSocket ping"))?,
                Message::Pong(_) => {}
                Message::Close(_) => {
                    return Err(mobile_mqtt_error(
                        "WebSocket closed before the QR event completed",
                    ));
                }
                Message::Text(_) | Message::Frame(_) => {
                    return Err(mobile_mqtt_error(
                        "WebSocket returned a non-binary MQTT frame",
                    ));
                }
            }
        }
    }
}

async fn start_mobile_listener(client: &QqClient, identifier: &str) -> Result<MobileQrListener> {
    let (mut socket, redirect_count) = connect_mobile_mqtt(client, identifier).await?;
    let topic = format!("management.qrcode_login/{identifier}");
    let started = Instant::now();
    let subscribe_outcome = async {
        socket
            .send_packet(Packet::Subscribe(Subscribe {
                pkid: 1,
                filters: vec![Filter::new(&topic, QoS::AtMostOnce)],
                properties: Some(SubscribeProperties {
                    id: None,
                    user_properties: vec![
                        ("authorization".to_owned(), "tmelogin".to_owned()),
                        ("pubsub".to_owned(), "unicast".to_owned()),
                    ],
                }),
            }))
            .await?;
        let suback = timeout(MOBILE_MQTT_CONNECT_TIMEOUT, socket.next_packet())
            .await
            .map_err(|_| mobile_mqtt_timeout("MQTT subscribe timed out"))??;
        let Packet::SubAck(suback) = suback else {
            return Err(mobile_mqtt_error(
                "QQ mobile QR MQTT did not acknowledge the subscription",
            ));
        };
        if suback.pkid != 1
            || suback
                .return_codes
                .iter()
                .any(|code| !matches!(code, SubscribeReasonCode::Success(_)))
        {
            return Err(mobile_mqtt_error(
                "QQ mobile QR MQTT rejected the subscription",
            ));
        }
        Ok(())
    }
    .await;
    client.log_runtime_upstream_request(
        "mobile_mqtt_subscribe",
        "mu.y.qq.com",
        "/ws/{mqtt_node}",
        None,
        started,
        UpstreamBusinessClass::Success,
        redirect_count,
        redirect_count > 0,
        &subscribe_outcome,
    );
    subscribe_outcome?;

    let (sender, receiver) = mpsc::channel(8);
    let task = tokio::spawn(async move {
        let mut ping = tokio::time::interval(Duration::from_secs(20));
        ping.tick().await;
        let expiry = tokio::time::sleep(QR_TRANSACTION_TTL);
        tokio::pin!(expiry);
        loop {
            tokio::select! {
                packet = socket.next_packet() => {
                    match packet {
                        Ok(Packet::Publish(publish)) => {
                            match parse_mobile_publish(&topic, &publish) {
                                Ok(Some(event)) => {
                                    let terminal = matches!(
                                        event,
                                        MobileQrEvent::Canceled
                                            | MobileQrEvent::Expired
                                            | MobileQrEvent::LoginFailed
                                            | MobileQrEvent::Credentials { .. }
                                    );
                                    if sender.send(event).await.is_err() || terminal {
                                        return;
                                    }
                                }
                                Ok(None) => {}
                                Err(_) => {
                                    let _ = sender.send(MobileQrEvent::TransportFailed).await;
                                    return;
                                }
                            }
                        }
                        Ok(Packet::PingResp(_)) => {}
                        Ok(Packet::Disconnect(_)) | Err(_) => {
                            let _ = sender.send(MobileQrEvent::TransportFailed).await;
                            return;
                        }
                        Ok(_) => {}
                    }
                }
                _ = ping.tick() => {
                    if socket.send_packet(Packet::PingReq(PingReq)).await.is_err() {
                        let _ = sender.send(MobileQrEvent::TransportFailed).await;
                        return;
                    }
                }
                () = &mut expiry => {
                    let _ = sender.send(MobileQrEvent::Expired).await;
                    return;
                }
            }
        }
    });
    Ok(MobileQrListener { receiver, task })
}

async fn connect_mobile_mqtt(
    client: &QqClient,
    identifier: &str,
) -> Result<(MobileMqttSocket, u8)> {
    let mut path = MOBILE_MQTT_INITIAL_PATH.to_owned();
    for redirect_count in 0..=MOBILE_MQTT_MAX_REDIRECTS {
        let attempt = u8::try_from(redirect_count).unwrap_or(u8::MAX);
        let mut socket = open_mobile_mqtt_websocket(client, &path, attempt).await?;
        let started = Instant::now();
        let connect_outcome = async {
            socket
                .send_packet(Packet::Connect(
                    Connect {
                        keep_alive: 45,
                        client_id: format!(
                            "{}{}",
                            unix_millis()?,
                            rand::random_range(1000_u16..=9999)
                        ),
                        clean_start: true,
                        properties: Some(ConnectProperties {
                            session_expiry_interval: None,
                            receive_maximum: None,
                            max_packet_size: None,
                            topic_alias_max: None,
                            request_response_info: None,
                            request_problem_info: None,
                            user_properties: vec![
                                ("tmeAppID".to_owned(), "qqmusic".to_owned()),
                                ("business".to_owned(), "management".to_owned()),
                                ("hashTag".to_owned(), identifier.to_owned()),
                                ("clientTag".to_owned(), "management.user".to_owned()),
                                ("userID".to_owned(), identifier.to_owned()),
                            ],
                            authentication_method: Some("pass".to_owned()),
                            authentication_data: None,
                        }),
                    },
                    None,
                    None,
                ))
                .await?;
            let connack = timeout(MOBILE_MQTT_CONNECT_TIMEOUT, socket.next_packet())
                .await
                .map_err(|_| mobile_mqtt_timeout("MQTT connect acknowledgement timed out"))??;
            let Packet::ConnAck(connack) = connack else {
                return Err(mobile_mqtt_error(
                    "QQ mobile QR MQTT did not return CONNACK",
                ));
            };
            match connack.code {
                ConnectReturnCode::Success => {
                    Ok(MobileMqttConnectStep::Connected(Box::new(socket)))
                }
                ConnectReturnCode::UseAnotherServer | ConnectReturnCode::ServerMoved => {
                    let reference = connack
                        .properties
                        .as_ref()
                        .and_then(|properties| properties.server_reference.as_deref())
                        .ok_or_else(|| {
                            mobile_mqtt_error("MQTT redirect is missing server reference")
                        })?;
                    if redirect_count == MOBILE_MQTT_MAX_REDIRECTS {
                        return Err(mobile_mqtt_error("MQTT redirect limit was exceeded"));
                    }
                    Ok(MobileMqttConnectStep::Redirect(mobile_redirect_path(
                        &path, reference,
                    )?))
                }
                _ => Err(mobile_mqtt_error(format!(
                    "QQ mobile QR MQTT rejected CONNECT with {:?}",
                    connack.code
                ))),
            }
        }
        .await;
        let success_class = match &connect_outcome {
            Ok(MobileMqttConnectStep::Redirect(_)) => UpstreamBusinessClass::AllowedError,
            Ok(MobileMqttConnectStep::Connected(_)) | Err(_) => UpstreamBusinessClass::Success,
        };
        client.log_runtime_upstream_request(
            "mobile_mqtt_connect",
            "mu.y.qq.com",
            "/ws/{mqtt_node}",
            None,
            started,
            success_class,
            attempt,
            redirect_count > 0,
            &connect_outcome,
        );
        match connect_outcome? {
            MobileMqttConnectStep::Connected(socket) => return Ok((*socket, attempt)),
            MobileMqttConnectStep::Redirect(next) => path = next,
        }
    }
    Err(mobile_mqtt_error("MQTT redirect limit was exceeded"))
}

async fn open_mobile_mqtt_websocket(
    client: &QqClient,
    path: &str,
    redirect_count: u8,
) -> Result<MobileMqttSocket> {
    if !path.starts_with('/') || path.contains(['?', '#']) {
        return Err(TuneWeaveError::new(
            ErrorCode::InternalError,
            "QQ mobile MQTT path is invalid",
        )
        .with_platform(Platform::Qq));
    }
    let endpoint = fixed_url(&format!("{MOBILE_MQTT_ROOT}{path}"))?;
    let key = BASE64.encode(rand::random::<[u8; 16]>());
    let request = client
        .login_http()
        .get(endpoint)
        .version(reqwest::Version::HTTP_11)
        .header(header::CONNECTION, "Upgrade")
        .header(header::UPGRADE, "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", &key)
        .header("sec-websocket-protocol", "mqtt")
        .header(header::ORIGIN, "https://y.qq.com")
        .header(header::REFERER, "https://y.qq.com/")
        .header(
            header::USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/123.0.0.0 Safari/537.36",
        )
        .timeout(MOBILE_MQTT_CONNECT_TIMEOUT);
    let started = Instant::now();
    let mut http_status = None;
    let outcome = match timeout(MOBILE_MQTT_CONNECT_TIMEOUT, async {
        let response = request.send().await.map_err(login_network_error)?;
        http_status = Some(response.status());
        if response.status() != StatusCode::SWITCHING_PROTOCOLS
            || !header_contains_token(response.headers(), header::UPGRADE, "websocket")
            || !header_contains_token(response.headers(), header::CONNECTION, "upgrade")
            || !header_contains_token(
                response.headers(),
                header::HeaderName::from_static("sec-websocket-protocol"),
                "mqtt",
            )
        {
            return Err(mobile_mqtt_error(
                "QQ mobile MQTT WebSocket handshake was rejected",
            ));
        }
        let expected_accept = derive_accept_key(key.as_bytes());
        let actual_accept = response
            .headers()
            .get("sec-websocket-accept")
            .and_then(|value| value.to_str().ok())
            .map(str::trim);
        if actual_accept != Some(expected_accept.as_str()) {
            return Err(mobile_mqtt_error(
                "QQ mobile MQTT WebSocket accept key is invalid",
            ));
        }
        let upgraded = response
            .upgrade()
            .await
            .map_err(|_| mobile_mqtt_error("QQ mobile MQTT WebSocket upgrade failed"))?;
        let websocket = WebSocketStream::from_raw_socket(upgraded, Role::Client, None).await;
        Ok(MobileMqttSocket {
            websocket,
            buffered: BytesMut::new(),
        })
    })
    .await
    {
        Ok(outcome) => outcome,
        Err(_) => Err(mobile_mqtt_timeout("WebSocket handshake timed out")),
    };
    client.log_runtime_upstream_request(
        "mobile_websocket_handshake",
        "mu.y.qq.com",
        "/ws/{mqtt_node}",
        http_status,
        started,
        UpstreamBusinessClass::Success,
        redirect_count,
        redirect_count > 0,
        &outcome,
    );
    outcome
}

fn parse_mobile_publish(topic: &str, publish: &Publish) -> Result<Option<MobileQrEvent>> {
    let publish_topic = std::str::from_utf8(&publish.topic)
        .map_err(|_| mobile_mqtt_error("QQ mobile MQTT topic is not UTF-8"))?;
    if publish_topic != topic {
        return Ok(None);
    }
    let event_type = publish.properties.as_ref().and_then(|properties| {
        properties
            .user_properties
            .iter()
            .rev()
            .find_map(|(name, value)| (name == "type").then_some(value.as_str()))
    });
    match event_type {
        Some("scanned") => Ok(Some(MobileQrEvent::Scanned)),
        Some("canceled") => Ok(Some(MobileQrEvent::Canceled)),
        Some("timeout") => Ok(Some(MobileQrEvent::Expired)),
        Some("loginFailed") => Ok(Some(MobileQrEvent::LoginFailed)),
        Some("cookies") => {
            let payload: serde_json::Value = serde_json::from_slice(&publish.payload)
                .map_err(|_| mobile_mqtt_error("QQ mobile MQTT cookie payload is invalid JSON"))?;
            let cookies = payload
                .get("cookies")
                .and_then(serde_json::Value::as_object)
                .ok_or_else(|| mobile_mqtt_error("QQ mobile MQTT payload is missing cookies"))?;
            let music_id = cookies
                .get("qqmusic_uin")
                .and_then(|value| value.get("value"))
                .and_then(value_as_nonempty_string)
                .ok_or_else(|| mobile_mqtt_error("QQ mobile MQTT payload is missing music ID"))?
                .parse::<u64>()
                .map_err(|_| mobile_mqtt_error("QQ mobile MQTT music ID is invalid"))?;
            let token = cookies
                .get("qqmusic_key")
                .and_then(|value| value.get("value"))
                .and_then(value_as_nonempty_string)
                .ok_or_else(|| {
                    mobile_mqtt_error("QQ mobile MQTT payload is missing login token")
                })?;
            if token.len() > 4096
                || token
                    .bytes()
                    .any(|byte| byte.is_ascii_control() || byte == b';')
            {
                return Err(mobile_mqtt_error(
                    "QQ mobile MQTT payload contains an unsafe login token",
                ));
            }
            Ok(Some(MobileQrEvent::Credentials { music_id, token }))
        }
        _ => Ok(None),
    }
}

fn mobile_redirect_path(current_path: &str, reference: &str) -> Result<String> {
    if reference.is_empty()
        || reference.len() > 256
        || !reference
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':'))
    {
        return Err(mobile_mqtt_error(
            "QQ mobile MQTT redirect reference is invalid",
        ));
    }
    let mut parts = current_path
        .trim_end_matches('/')
        .split('/')
        .collect::<Vec<_>>();
    if parts.last().is_some_and(|part| part.contains(':')) {
        let last = parts.len() - 1;
        parts[last] = reference;
        Ok(parts.join("/"))
    } else {
        Ok(format!(
            "{}/{}",
            current_path.trim_end_matches('/'),
            reference
        ))
    }
}

fn header_contains_token(
    headers: &header::HeaderMap,
    name: header::HeaderName,
    expected: &str,
) -> bool {
    headers.get_all(name).iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value
                .split(',')
                .any(|token| token.trim().eq_ignore_ascii_case(expected))
        })
    })
}

async fn authorize_qq_qr(client: &QqClient, uin: &str, sigx: &str) -> Result<QqCredential> {
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
    let oauth_cookies = collect_qq_signature_cookies(client, endpoint).await?;
    let p_skey = oauth_cookies
        .get("p_skey")
        .cloned()
        .ok_or_else(|| login_data_error("QQ login signature exchange is missing p_skey"))?;

    let request = client
        .login_http()
        .post(QQ_OAUTH_ENDPOINT)
        .header(header::COOKIE, cookie_header(&oauth_cookies)?)
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
        .timeout(LOGIN_REQUEST_TIMEOUT);
    let started = Instant::now();
    let mut http_status = None;
    let oauth_outcome = async {
        let response = request.send().await.map_err(login_network_error)?;
        http_status = Some(response.status());
        ensure_redirect_or_success(response.status(), "QQ OAuth authorization")?;
        let location = response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| {
                login_data_error("QQ OAuth authorization is missing redirect location")
            })?;
        parse_qq_oauth_code(location)
    }
    .await;
    client.log_simple_upstream_request(
        "qq_oauth_authorize",
        "graph.qq.com",
        "/oauth2.0/authorize",
        http_status,
        started,
        &oauth_outcome,
    );
    let code = oauth_outcome?;

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

async fn collect_qq_signature_cookies(
    client: &QqClient,
    mut endpoint: Url,
) -> Result<BTreeMap<String, String>> {
    validate_qq_signature_redirect(&endpoint)?;
    let mut cookies = BTreeMap::new();
    for redirect_count in 0..=QQ_SIGNATURE_MAX_REDIRECTS {
        let upstream_host = qq_signature_upstream_host(&endpoint);
        let mut request = client
            .login_http()
            .get(endpoint.clone())
            .header(header::REFERER, QQ_LOGIN_REFERER)
            .timeout(LOGIN_REQUEST_TIMEOUT);
        if !cookies.is_empty() {
            request = request.header(header::COOKIE, cookie_header(&cookies)?);
        }
        let started = Instant::now();
        let mut http_status = None;
        let step_outcome = async {
            let response = request.send().await.map_err(login_network_error)?;
            http_status = Some(response.status());
            ensure_redirect_or_success(response.status(), "QQ login signature exchange")?;
            merge_nonempty_response_cookies(&mut cookies, &response)?;
            if !response.status().is_redirection() {
                if !cookies
                    .get("p_skey")
                    .is_some_and(|value| !value.trim().is_empty())
                {
                    return Err(login_data_error(
                        "QQ login signature exchange is missing p_skey",
                    ));
                }
                return Ok(QqSignatureStep::Complete);
            }
            if redirect_count == QQ_SIGNATURE_MAX_REDIRECTS {
                return Err(login_data_error(
                    "QQ login signature exchange exceeded its redirect limit",
                ));
            }
            let location = response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .ok_or_else(|| {
                    login_data_error("QQ login signature exchange redirect is missing location")
                })?;
            let next = endpoint.join(location).map_err(|_| {
                login_data_error("QQ login signature exchange redirect is malformed")
            })?;
            validate_qq_signature_redirect(&next)?;
            Ok(QqSignatureStep::Redirect(next))
        }
        .await;
        let success_class = match &step_outcome {
            Ok(QqSignatureStep::Redirect(_)) => UpstreamBusinessClass::AllowedError,
            Ok(QqSignatureStep::Complete) | Err(_) => UpstreamBusinessClass::Success,
        };
        client.log_typed_upstream_request(
            "qq_signature_exchange",
            upstream_host,
            "/{signature_exchange}",
            http_status,
            started,
            success_class,
            &step_outcome,
        );
        match step_outcome? {
            QqSignatureStep::Complete => return Ok(cookies),
            QqSignatureStep::Redirect(next) => endpoint = next,
        }
    }
    Err(login_data_error(
        "QQ login signature exchange exceeded its redirect limit",
    ))
}

fn validate_qq_signature_redirect(endpoint: &Url) -> Result<()> {
    let host = endpoint.host_str().unwrap_or_default();
    if endpoint.scheme() != "https"
        || endpoint.port_or_known_default() != Some(443)
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.fragment().is_some()
        || !matches!(
            host,
            "ssl.ptlogin2.graph.qq.com" | "ptlogin2.graph.qq.com" | "graph.qq.com"
        )
    {
        return Err(login_data_error(
            "QQ login signature exchange redirected outside the fixed QQ Graph hosts",
        ));
    }
    Ok(())
}

fn qq_signature_upstream_host(endpoint: &Url) -> &'static str {
    match endpoint.host_str() {
        Some("ssl.ptlogin2.graph.qq.com") => "ssl.ptlogin2.graph.qq.com",
        Some("ptlogin2.graph.qq.com") => "ptlogin2.graph.qq.com",
        _ => "graph.qq.com",
    }
}

fn parse_qq_oauth_code(location: &str) -> Result<String> {
    let location = Url::parse(location)
        .map_err(|_| login_data_error("QQ OAuth redirect location is invalid"))?;
    if location.scheme() != "https"
        || location.host_str() != Some("y.qq.com")
        || location.port_or_known_default() != Some(443)
        || !location.username().is_empty()
        || location.password().is_some()
        || location.path() != "/portal/wx_redirect.html"
        || location.fragment().is_some()
    {
        return Err(login_data_error(
            "QQ OAuth redirect location is outside the allowed destination",
        ));
    }
    let code = location
        .query_pairs()
        .find_map(|(name, value)| (name == "code").then(|| value.into_owned()))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| login_data_error("QQ OAuth redirect location is missing code"))?;
    validate_identifier(&code, "QQ OAuth code")?;
    Ok(code)
}

fn qq_qr_poll_cookies(qrsig: &str) -> Result<BTreeMap<String, String>> {
    validate_cookie_value(qrsig)?;
    Ok(BTreeMap::from([("qrsig".to_owned(), qrsig.to_owned())]))
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

fn value_as_nonempty_string(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_owned())
        }
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn parse_phone_principal(value: &str) -> Result<QqPhonePrincipal> {
    let value = value.trim();
    if let Some(encrypted) = value.strip_prefix("encrypted:") {
        if encrypted.is_empty()
            || encrypted.len() > 512
            || encrypted.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err(
                TuneWeaveError::invalid_request("QQ encrypted phone principal is invalid")
                    .with_platform(Platform::Qq),
            );
        }
        return Ok(QqPhonePrincipal::Encrypted(encrypted.to_owned()));
    }
    if !(5..=32).contains(&value.len()) || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(TuneWeaveError::invalid_request(
            "QQ phone principal must contain 5 to 32 digits",
        )
        .with_platform(Platform::Qq));
    }
    Ok(QqPhonePrincipal::Plain(value.to_owned()))
}

fn phone_authcode_send_param(principal: &str, country_code: &str) -> Result<serde_json::Value> {
    let principal = parse_phone_principal(principal)?;
    let country_code = validate_country_code(country_code)?;
    let mut param = serde_json::Map::from_iter([
        ("tmeAppid".to_owned(), json!("qqmusic")),
        ("areaCode".to_owned(), json!(country_code)),
    ]);
    insert_phone_principal(&mut param, principal);
    Ok(serde_json::Value::Object(param))
}

fn phone_authcode_login_param(principal: &str, code: &str) -> Result<serde_json::Value> {
    let principal = parse_phone_principal(principal)?;
    let code = code.trim();
    if code.is_empty() || code.len() > 32 || code.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(
            TuneWeaveError::invalid_request("QQ phone verification code is invalid")
                .with_platform(Platform::Qq),
        );
    }
    let mut param = serde_json::Map::from_iter([
        ("code".to_owned(), json!(code)),
        ("loginMode".to_owned(), json!(1)),
    ]);
    insert_phone_principal(&mut param, principal);
    Ok(serde_json::Value::Object(param))
}

fn refresh_credential_param(credential: &QqCredential) -> serde_json::Value {
    match credential.login_type {
        1 => json!({
            "openid": credential.openid,
            "refresh_token": credential.refresh_token,
            "str_musicid": credential.str_music_id,
            "musickey": credential.musickey,
            "unionid": credential.unionid,
            "refresh_key": credential.refresh_key,
            "loginMode": 2
        }),
        2 => json!({
            "openid": credential.openid,
            "access_token": credential.access_token,
            "refresh_token": credential.refresh_token,
            "expired_in": credential.expired_at,
            "musicid": credential.music_id,
            "musickey": credential.musickey,
            "refresh_key": credential.refresh_key,
            "loginMode": 2
        }),
        _ => json!({
            "openid": credential.openid,
            "access_token": credential.access_token,
            "refresh_token": credential.refresh_token,
            "expired_in": credential.expired_at,
            "str_musicid": credential.str_music_id,
            "musicid": credential.music_id,
            "musickey": credential.musickey,
            "unionid": credential.unionid,
            "refresh_key": credential.refresh_key,
            "loginMode": 2
        }),
    }
}

fn logout_outcome(code: i64) -> Result<QqLogoutOutcome> {
    match code {
        0 => Ok(QqLogoutOutcome::LoggedOut),
        1000 | 104_400 | 104_401 => Ok(QqLogoutOutcome::CredentialExpired),
        code => Err(qq_login_business_error(
            code,
            "QQ failed to log out the account",
        )),
    }
}

fn insert_phone_principal(
    param: &mut serde_json::Map<String, serde_json::Value>,
    principal: QqPhonePrincipal,
) {
    match principal {
        QqPhonePrincipal::Plain(phone) => {
            param.insert("phoneNo".to_owned(), json!(phone));
        }
        QqPhonePrincipal::Encrypted(phone) => {
            param.insert("encryptedPhoneNo".to_owned(), json!(phone));
        }
    }
}

fn validate_country_code(value: &str) -> Result<&str> {
    let value = value.trim();
    if value.is_empty() || value.len() > 6 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(TuneWeaveError::invalid_request(
            "QQ phone country code must contain 1 to 6 digits",
        )
        .with_platform(Platform::Qq));
    }
    Ok(value)
}

pub(crate) fn qq_login_business_error(code: i64, message: &str) -> TuneWeaveError {
    let error_code = match code {
        1000 | 104_400 | 104_401 => ErrorCode::AuthenticationRequired,
        20_261 => ErrorCode::InvalidRequest,
        20_271 | 20_277 | 20_278 | 20_450 => ErrorCode::PermissionDenied,
        20_272 | 20_274 | 20_279 => ErrorCode::Conflict,
        100_001 | 104_604 | 2001 => ErrorCode::RateLimited,
        _ => ErrorCode::UpstreamError,
    };
    TuneWeaveError::new(error_code, format!("{message} (code {code})"))
        .with_platform(Platform::Qq)
        .retryable(matches!(code, 100_001 | 104_604 | 2001))
        .with_details(json!({ "platform_code": code }))
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

fn parse_qq_qr_poll(body: &str) -> Result<ParsedQqQrPoll> {
    let args = parse_qq_status_arguments(body)?;
    let status = args[0]
        .parse::<u16>()
        .map_err(|_| login_data_error("QQ QR status returned an invalid state code"))?;
    match status {
        66 => Ok(ParsedQqQrPoll::Waiting),
        67 => Ok(ParsedQqQrPoll::Scanned),
        65 => Ok(ParsedQqQrPoll::Expired),
        68 => Ok(ParsedQqQrPoll::Failed),
        0 => {
            let callback = args.get(2).ok_or_else(|| {
                login_data_error("QQ QR success response is missing callback URL")
            })?;
            let callback = Url::parse(callback)
                .map_err(|_| login_data_error("QQ QR success callback URL is invalid"))?;
            if callback.scheme() != "https"
                || callback.host_str() != Some("graph.qq.com")
                || !callback.username().is_empty()
                || callback.password().is_some()
                || callback.port().is_some()
                || callback.fragment().is_some()
            {
                return Err(login_data_error(
                    "QQ QR success callback URL is outside the allowed destination",
                ));
            }
            let query = callback.query_pairs().collect::<BTreeMap<_, _>>();
            let uin = query
                .get("uin")
                .map(|value| value.as_ref())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| login_data_error("QQ QR success callback is missing uin"))?;
            if !uin.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(login_data_error(
                    "QQ QR success callback contains an invalid uin",
                ));
            }
            let sigx = query
                .get("ptsigx")
                .map(|value| value.as_ref())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| login_data_error("QQ QR success callback is missing ptsigx"))?;
            validate_identifier(sigx, "QQ QR success ptsigx")?;
            Ok(ParsedQqQrPoll::Confirmed {
                uin: uin.to_owned(),
                sigx: sigx.to_owned(),
            })
        }
        _ => Err(login_data_error(format!(
            "QQ QR status returned unsupported state code {status}"
        ))),
    }
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

fn parse_wechat_qr_poll(body: &str) -> Result<ParsedWechatQrPoll> {
    let (status, code) = parse_wechat_status(body)?;
    match status {
        408 => Ok(ParsedWechatQrPoll::Waiting),
        404 => Ok(ParsedWechatQrPoll::Scanned),
        402 => Ok(ParsedWechatQrPoll::Expired),
        403 => Ok(ParsedWechatQrPoll::Failed),
        405 => {
            let code = code
                .filter(|value| !value.is_empty())
                .ok_or_else(|| login_data_error("WeChat QR confirmation is missing OAuth code"))?;
            validate_identifier(code, "WeChat OAuth code")?;
            Ok(ParsedWechatQrPoll::Confirmed {
                code: code.to_owned(),
            })
        }
        _ => Err(login_data_error(format!(
            "WeChat QR status returned unsupported state code {status}"
        ))),
    }
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
    merge_cookie_headers(cookies, response.headers(), true)
}

fn merge_nonempty_response_cookies(
    cookies: &mut BTreeMap<String, String>,
    response: &Response,
) -> Result<()> {
    merge_cookie_headers(cookies, response.headers(), false)
}

fn merge_cookie_headers(
    cookies: &mut BTreeMap<String, String>,
    headers: &header::HeaderMap,
    remove_empty: bool,
) -> Result<()> {
    for value in headers.get_all(header::SET_COOKIE) {
        let value = value
            .to_str()
            .map_err(|_| login_data_error("QQ login response contains an invalid Set-Cookie"))?;
        let pair = value.split(';').next().unwrap_or_default();
        let (name, value) = pair
            .split_once('=')
            .ok_or_else(|| login_data_error("QQ login response contains a malformed cookie"))?;
        validate_cookie_name(name)?;
        validate_cookie_value(value)?;
        if value.is_empty() && remove_empty {
            cookies.remove(name);
        } else if !value.is_empty() {
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

fn mqtt_codec_error(error: MqttCodecError) -> TuneWeaveError {
    mobile_mqtt_error(format!("QQ mobile MQTT packet is invalid: {error}"))
}

fn mobile_mqtt_timeout(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamTimeout, message)
        .with_platform(Platform::Qq)
        .retryable(true)
}

fn mobile_mqtt_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message)
        .with_platform(Platform::Qq)
        .retryable(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rumqttc::v5::mqttbytes::v5::PublishProperties;

    #[test]
    fn parses_qq_qr_callback_without_exposing_unquoted_text() {
        let body = "ptuiCB('0','0','https://graph.qq.com/?uin=123&service=x&ptsigx=sig&s_url=y','0','ok','nick');";
        let args = parse_qq_status_arguments(body).expect("QQ callback");
        assert_eq!(args[0], "0");
        assert!(args[2].contains("ptsigx=sig"));
        assert_eq!(args[5], "nick");
        assert!(matches!(
            parse_qq_qr_poll(body).expect("QQ confirmation"),
            ParsedQqQrPoll::Confirmed { uin, sigx } if uin == "123" && sigx == "sig"
        ));
        assert!(matches!(
            parse_qq_qr_poll("ptuiCB('66','0','','0','waiting','');").expect("QQ waiting state"),
            ParsedQqQrPoll::Waiting
        ));
        assert!(
            parse_qq_qr_poll(
                "ptuiCB('0','0','https://evil.example/?uin=123&ptsigx=secret','0','ok','');"
            )
            .is_err()
        );
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
        assert!(matches!(
            parse_wechat_qr_poll("window.wx_errcode=405;window.wx_code='oauth-code'")
                .expect("confirmation"),
            ParsedWechatQrPoll::Confirmed { code } if code == "oauth-code"
        ));
        assert!(matches!(
            parse_wechat_qr_poll("window.wx_errcode=408;").expect("waiting state"),
            ParsedWechatQrPoll::Waiting
        ));
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
    fn signature_cookie_collection_keeps_valid_domain_value_before_empty_cleanup() {
        let mut headers = header::HeaderMap::new();
        headers.append(
            header::SET_COOKIE,
            header::HeaderValue::from_static(
                "p_skey=oauth-value; Domain=graph.qq.com; Path=/; Secure",
            ),
        );
        headers.append(
            header::SET_COOKIE,
            header::HeaderValue::from_static("p_skey=; Domain=qq.com; Path=/; Max-Age=0"),
        );

        let mut signature_cookies = BTreeMap::new();
        merge_cookie_headers(&mut signature_cookies, &headers, false)
            .expect("collect QQ signature cookies");
        assert_eq!(
            signature_cookies.get("p_skey").map(String::as_str),
            Some("oauth-value")
        );

        let mut ordinary_cookies = BTreeMap::new();
        merge_cookie_headers(&mut ordinary_cookies, &headers, true)
            .expect("merge ordinary QQ cookies");
        assert!(!ordinary_cookies.contains_key("p_skey"));
    }

    #[test]
    fn signature_redirects_stay_on_fixed_https_qq_graph_hosts() {
        for endpoint in [
            "https://ssl.ptlogin2.graph.qq.com/check_sig",
            "https://ptlogin2.graph.qq.com/check_sig",
            "https://graph.qq.com/oauth2.0/login_jump",
        ] {
            let endpoint = Url::parse(endpoint).expect("QQ Graph URL");
            validate_qq_signature_redirect(&endpoint).expect("allowed QQ Graph redirect");
            assert_ne!(qq_signature_upstream_host(&endpoint), "");
        }
        for endpoint in [
            "http://graph.qq.com/oauth2.0/login_jump",
            "https://graph.qq.com.evil.test/oauth2.0/login_jump",
            "https://user@graph.qq.com/oauth2.0/login_jump",
            "https://graph.qq.com:444/oauth2.0/login_jump",
            "https://graph.qq.com/oauth2.0/login_jump#secret",
        ] {
            assert!(
                validate_qq_signature_redirect(&Url::parse(endpoint).expect("test URL")).is_err(),
                "unsafe QQ Graph redirect was accepted: {endpoint}"
            );
        }
    }

    #[test]
    fn qq_oauth_code_only_accepts_the_fixed_music_redirect() {
        assert_eq!(
            parse_qq_oauth_code(
                "https://y.qq.com/portal/wx_redirect.html?login_type=1&code=oauth-code"
            )
            .expect("valid QQ OAuth redirect"),
            "oauth-code"
        );
        for location in [
            "https://evil.example/portal/wx_redirect.html?code=secret",
            "http://y.qq.com/portal/wx_redirect.html?code=secret",
            "https://y.qq.com/other?code=secret",
            "https://y.qq.com/portal/wx_redirect.html",
            "https://y.qq.com/portal/wx_redirect.html?code=secret#fragment",
        ] {
            assert!(
                parse_qq_oauth_code(location).is_err(),
                "unsafe QQ OAuth redirect was accepted: {location}"
            );
        }
    }

    #[test]
    fn qq_qr_polling_isolates_qrsig_from_accumulated_login_cookies() {
        let cookies = qq_qr_poll_cookies("qr-signature").expect("QQ QR poll cookies");
        assert_eq!(
            cookies,
            BTreeMap::from([("qrsig".to_owned(), "qr-signature".to_owned())])
        );
        assert_eq!(
            cookie_header(&cookies).expect("QQ QR poll cookie header"),
            "qrsig=qr-signature"
        );
        assert!(qq_qr_poll_cookies("unsafe;cookie").is_err());
    }

    #[test]
    fn generated_oauth_ui_is_a_uuid_v4() {
        let value = random_uuid_v4();
        assert_eq!(value.len(), 36);
        assert_eq!(&value[14..15], "4");
        assert!(matches!(&value[19..20], "8" | "9" | "a" | "b"));
    }

    #[test]
    fn mobile_redirects_append_then_replace_the_server_node() {
        let redirected =
            mobile_redirect_path("/ws/handshake", "node.example:443").expect("append redirect");
        assert_eq!(redirected, "/ws/handshake/node.example:443");
        assert_eq!(
            mobile_redirect_path(&redirected, "next.example:443").expect("replace redirect"),
            "/ws/handshake/next.example:443"
        );
        assert!(mobile_redirect_path("/ws/handshake", "../unsafe").is_err());
    }

    #[test]
    fn mobile_cookie_publish_extracts_only_typed_login_fields() {
        let topic = "management.qrcode_login/qr-id";
        let publish = Publish::new(
            topic,
            QoS::AtMostOnce,
            br#"{"cookies":{"qqmusic_uin":{"value":"123456"},"qqmusic_key":{"value":"mobile-key"}},"ignored":"secret"}"#
                .as_slice(),
            Some(PublishProperties {
                user_properties: vec![("type".to_owned(), "cookies".to_owned())],
                ..PublishProperties::default()
            }),
        );
        let event = parse_mobile_publish(topic, &publish)
            .expect("mobile publish")
            .expect("mobile event");
        let MobileQrEvent::Credentials { music_id, token } = event else {
            panic!("expected mobile credentials");
        };
        assert_eq!(music_id, 123456);
        assert_eq!(token, "mobile-key");
    }

    #[test]
    fn phone_authcode_parameters_preserve_plain_and_encrypted_branches() {
        assert_eq!(
            phone_authcode_send_param("13800138000", "86").expect("plain send"),
            json!({
                "tmeAppid": "qqmusic",
                "areaCode": "86",
                "phoneNo": "13800138000"
            })
        );
        assert_eq!(
            phone_authcode_login_param("encrypted:cipher-phone", "123456")
                .expect("encrypted login"),
            json!({
                "code": "123456",
                "loginMode": 1,
                "encryptedPhoneNo": "cipher-phone"
            })
        );
        assert!(phone_authcode_send_param("+8613800138000", "86").is_err());
        assert!(phone_authcode_login_param("13800138000", "\r\n").is_err());
    }

    #[test]
    fn phone_login_business_codes_keep_stable_error_classes() {
        assert_eq!(
            qq_login_business_error(20_271, "login").code,
            ErrorCode::PermissionDenied
        );
        assert_eq!(
            qq_login_business_error(20_279, "login").code,
            ErrorCode::Conflict
        );
        let limited = qq_login_business_error(104_604, "login");
        assert_eq!(limited.code, ErrorCode::RateLimited);
        assert!(limited.retryable);
    }

    #[test]
    fn credential_refresh_preserves_every_login_type_parameter_branch() {
        let credential = QqCredential {
            openid: "open-id".to_owned(),
            refresh_token: "refresh-token".to_owned(),
            access_token: "access-token".to_owned(),
            expired_at: 123,
            music_id: 456,
            musickey: "Q_H_L_private".to_owned(),
            unionid: "union-id".to_owned(),
            str_music_id: "456".to_owned(),
            refresh_key: "refresh-key".to_owned(),
            ..serde_json::from_value(json!({})).expect("empty credential")
        };

        let wechat = refresh_credential_param(&QqCredential {
            login_type: 1,
            ..credential.clone()
        });
        assert_eq!(
            wechat,
            json!({
                "openid": "open-id",
                "refresh_token": "refresh-token",
                "str_musicid": "456",
                "musickey": "Q_H_L_private",
                "unionid": "union-id",
                "refresh_key": "refresh-key",
                "loginMode": 2
            })
        );
        assert!(wechat.get("access_token").is_none());
        assert!(wechat.get("musicid").is_none());

        let qq = refresh_credential_param(&QqCredential {
            login_type: 2,
            ..credential.clone()
        });
        assert_eq!(
            qq,
            json!({
                "openid": "open-id",
                "access_token": "access-token",
                "refresh_token": "refresh-token",
                "expired_in": 123,
                "musicid": 456,
                "musickey": "Q_H_L_private",
                "refresh_key": "refresh-key",
                "loginMode": 2
            })
        );
        assert!(qq.get("str_musicid").is_none());
        assert!(qq.get("unionid").is_none());

        let mobile = refresh_credential_param(&QqCredential {
            login_type: 6,
            ..credential
        });
        assert_eq!(
            mobile,
            json!({
                "openid": "open-id",
                "access_token": "access-token",
                "refresh_token": "refresh-token",
                "expired_in": 123,
                "str_musicid": "456",
                "musicid": 456,
                "musickey": "Q_H_L_private",
                "unionid": "union-id",
                "refresh_key": "refresh-key",
                "loginMode": 2
            })
        );
    }

    #[test]
    fn logout_only_accepts_success_or_an_already_expired_credential() {
        assert_eq!(
            logout_outcome(0).expect("logged out"),
            QqLogoutOutcome::LoggedOut
        );
        assert_eq!(
            logout_outcome(104_401).expect("expired credential"),
            QqLogoutOutcome::CredentialExpired
        );
        assert_eq!(
            logout_outcome(104_604).expect_err("rate limited").code,
            ErrorCode::RateLimited
        );
        assert_eq!(
            logout_outcome(20_261).expect_err("invalid request").code,
            ErrorCode::InvalidRequest
        );
    }

    #[tokio::test]
    #[ignore = "requires live QQ and WeChat login services"]
    async fn live_standard_qr_logins_create_images_and_report_waiting() {
        let client = QqClient::new(crate::QqConfig::default()).expect("QQ client");
        for (kind, mime) in [
            (QqQrLoginKind::Qq, "image/png"),
            (QqQrLoginKind::Wechat, "image/jpeg"),
            (QqQrLoginKind::Mobile, "image/png"),
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
