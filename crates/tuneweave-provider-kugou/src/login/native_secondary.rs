//! Native secondary phone authentication; distinct from phone binding and Web SMS.
use std::{collections::BTreeMap, fmt, time::Instant};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use md5::{Digest, Md5};
use serde::Deserialize;
use serde_json::{Value, json};
use tuneweave_core::{AuthAccountChoice, ErrorCode, Platform, Result, TuneWeaveError};

use super::{
    crypto::{self, ExchangeCipher, native_standard::Fingerprint},
    native_password::{CLIENT_VERSION, native_phone, parse_session},
};
use crate::{
    KugouClient, KugouLoginClient,
    account::{network_error, now_ms, read_response},
    credential::{NativeSession, valid_uid},
    device::KugouDeviceIdentity,
    signing::{ANDROID_SALT, android_signature},
};

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct Target {
    value: String,
    phone: bool,
}
impl fmt::Debug for Target {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SecondaryPhoneTarget")
            .field("phone", &self.phone)
            .finish_non_exhaustive()
    }
}
impl Target {
    pub(crate) fn parse(value: Option<Value>) -> Result<Self> {
        let value = match value {
            Some(Value::String(s)) => s,
            Some(Value::Number(n)) => n.as_i64().ok_or_else(malformed)?.to_string(),
            _ => return Err(malformed()),
        };
        let phone = native_phone(&value);
        if !valid_uid(&value) || value.parse::<i64>().is_err() {
            return Err(malformed());
        }
        Ok(Self { value, phone })
    }
    pub(crate) fn masked_destination(&self) -> String {
        if self.phone {
            mask(&self.value)
        } else {
            "账号绑定手机".into()
        }
    }
    pub(crate) fn permits_user(&self, user_id: &str) -> bool {
        self.phone || self.value == user_id
    }
}

pub(crate) enum Outcome {
    Session(Box<NativeSession>),
    InvalidCode,
    ChooseAccount,
}
#[derive(Clone, Copy)]
enum Endpoint {
    Send,
    Login,
    Accounts,
}
impl Endpoint {
    fn host(self) -> &'static str {
        match self {
            Self::Send => "loginservice.kugou.com",
            Self::Login => "gateway.kugou.com",
            Self::Accounts => "userinfoservice.kugou.com",
        }
    }
    fn path(self) -> &'static str {
        match self {
            Self::Send => "/v8/send_mobile_code",
            Self::Login => "/login.user/v7/login_by_verifycode",
            Self::Accounts => "/v4/check_mobile",
        }
    }
    fn operation(self) -> &'static str {
        match self {
            Self::Send => "native_secondary_sms_send",
            Self::Login => "native_secondary_sms_verify",
            Self::Accounts => "native_secondary_sms_accounts",
        }
    }
}
#[derive(Deserialize)]
struct Envelope {
    status: i64,
    #[serde(default)]
    error_code: i64,
    data: Option<Value>,
}
impl Envelope {
    fn success(&self) -> bool {
        self.status == 1 && self.error_code == 0
    }
}

impl KugouClient {
    fn secondary_cipher(&self) -> Result<ExchangeCipher> {
        #[cfg(test)]
        if let Some(seed) = &self.password_test_seed {
            return Ok(ExchangeCipher::for_test(seed));
        }
        ExchangeCipher::random()
    }
    pub(crate) async fn native_secondary_send(
        &self,
        device: &KugouDeviceIdentity,
        target: &Target,
    ) -> Result<()> {
        let cipher = self.secondary_cipher()?;
        let now = now_ms()?;
        let body = send_body(target, now, &cipher)?;
        let response = self
            .secondary_post(Endpoint::Send, device, now, body)
            .await?;
        if response.success() {
            Ok(())
        } else {
            Err(rejected(response.error_code))
        }
    }
    pub(crate) async fn native_secondary_login(
        &self,
        device: &KugouDeviceIdentity,
        target: &Target,
        code: &str,
        selected: Option<&str>,
    ) -> Result<Outcome> {
        validate_code(code)?;
        if selected.is_some_and(|uid| !valid_uid(uid) || !target.permits_user(uid)) {
            return Err(invalid());
        }
        let cipher = self.secondary_cipher()?;
        let now = now_ms()?;
        let body = login_body(target, code, selected, device, now, &cipher)?;
        let response = self
            .secondary_post(Endpoint::Login, device, now, body)
            .await?;
        if response.success() {
            let session = parse_session(response.data, device, &cipher)?;
            if !target.permits_user(&session.user_id)
                || selected.is_some_and(|uid| uid != session.user_id)
            {
                return Err(TuneWeaveError::new(
                    ErrorCode::PermissionDenied,
                    "KuGou secondary verification confirmed a different account",
                )
                .with_platform(Platform::Kugou));
            }
            return Ok(Outcome::Session(Box::new(session)));
        }
        match (response.status, response.error_code) {
            (0, 20020 | 20021) => Ok(Outcome::InvalidCode),
            (0, 34175) => Ok(Outcome::ChooseAccount),
            _ => Err(rejected(response.error_code)),
        }
    }
    pub(crate) async fn native_secondary_accounts(
        &self,
        device: &KugouDeviceIdentity,
        target: &Target,
        code: &str,
    ) -> Result<Vec<AuthAccountChoice>> {
        validate_code(code)?;
        let cipher = self.secondary_cipher()?;
        let now = now_ms()?;
        let body = accounts_body(target, code, now, &cipher)?;
        let response = self
            .secondary_post(Endpoint::Accounts, device, now, body)
            .await?;
        if !response.success() {
            return Err(rejected(response.error_code));
        }
        parse_accounts(response.data, target)
    }
    async fn secondary_post(
        &self,
        endpoint: Endpoint,
        device: &KugouDeviceIdentity,
        milliseconds: u64,
        body: Vec<u8>,
    ) -> Result<Envelope> {
        let query = signed_query(device, milliseconds, &body);
        let url = format!("https://{}{}", endpoint.host(), endpoint.path());
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(endpoint.path()).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .post(url)
                .header("content-type", "application/json; charset=UTF-8")
                .header("accept", "application/json")
                .header("user-agent", super::NATIVE_USER_AGENT)
                .header("support-calm", "1")
                .query(&query)
                .body(body)
                .send()
                .await
                .map_err(network_error)?;
            status = Some(response.status());
            serde_json::from_slice(&read_response(response).await?).map_err(|_| malformed())
        }
        .await;
        self.log_upstream_request(
            endpoint.operation(),
            endpoint.host(),
            endpoint.path(),
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

fn send_body(target: &Target, milliseconds: u64, cipher: &ExchangeCipher) -> Result<Vec<u8>> {
    // The native v8 sender copies numeric query seconds into this oddly named field.
    let seconds = milliseconds / 1000;
    let mut body = json!({"plat":"1", "businessid":5, "clienttime_ms":seconds,
        "pk":cipher.pk(KugouLoginClient::Standard, seconds)?});
    let mobile = if target.phone {
        body["mobile"] = json!(mask(&target.value));
        target.value.as_str()
    } else {
        body["userid"] = json!(target.value.parse::<i64>().map_err(|_| malformed())?);
        ""
    };
    body["params"] = json!(cipher.encrypt(&crypto::encode(&json!({"mobile":mobile}))?)?);
    crypto::encode(&body)
}
fn login_body(
    target: &Target,
    code: &str,
    selected: Option<&str>,
    device: &KugouDeviceIdentity,
    milliseconds: u64,
    cipher: &ExchangeCipher,
) -> Result<Vec<u8>> {
    let fingerprint = Fingerprint::fresh_desktop(milliseconds)?;
    let secret = json!({"mobile":target.value, "code":code,
        "mobile_data":{"access_id":"300011877488", "access_key":"663160D210B5D9250C3661DC9A854328"}});
    let key = hex::encode(Md5::digest(format!(
        "1005{ANDROID_SALT}{CLIENT_VERSION}{}",
        fingerprint.timestamp
    )));
    let mut body = json!({
        "dfid":device.dfid(), "plat":1,
        "t1":fingerprint.t1, "t2":fingerprint.t2,
        "t3":BASE64.encode("0,0,0,0,0,65530,0,0,0"),
        "clienttime_ms":fingerprint.timestamp,
        "pk":cipher.password_pk(&fingerprint.timestamp)?,
        "params":cipher.encrypt(&crypto::encode(&secret)?)?,
        "mobile":mask(&target.value), "support_multi":1,
        "key":key, "dev":crypto::DESKTOP_MODEL, "gitversion":"0000000",
        "need_toneinfo":1, "busi_type":"kid"
    });
    if let Some(uid) = selected {
        body["userid"] = json!(uid);
    }
    crypto::encode(&body)
}
fn accounts_body(
    target: &Target,
    code: &str,
    now: u64,
    cipher: &ExchangeCipher,
) -> Result<Vec<u8>> {
    crypto::encode(
        &json!({"plat":"1", "mobile":if target.phone {mask(&target.value)} else {target.value.clone()}, "businessid":5,
        "query":{"duration":1,"p_grade":1}, "clienttime_ms":now.to_string(),
        "pk":cipher.pk(KugouLoginClient::Standard, now)?,
        "params":cipher.encrypt(&crypto::encode(&json!({"mobile":target.value,"code":code}))?)?}),
    )
}
fn signed_query(
    device: &KugouDeviceIdentity,
    now: u64,
    body: &[u8],
) -> BTreeMap<&'static str, String> {
    let mut query = BTreeMap::from([
        ("appid", "1005".into()),
        ("clientver", CLIENT_VERSION.to_string()),
        ("clienttime", (now / 1000).to_string()),
        ("mid", device.mid.clone()),
        ("dfid", device.dfid().to_owned()),
        ("uuid", "-".into()),
    ]);
    query.insert("signature", android_signature(&query, body));
    query
}
fn mask(value: &str) -> String {
    if value.len() == 11 {
        format!("{}*****{}", &value[..3], &value[8..])
    } else {
        value.into()
    }
}
pub(crate) fn validate_code(code: &str) -> Result<()> {
    if !(4..=8).contains(&code.len()) || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    Ok(())
}
fn parse_accounts(data: Option<Value>, target: &Target) -> Result<Vec<AuthAccountChoice>> {
    #[derive(Deserialize)]
    struct Accounts {
        isreg: u8,
        info_list: Vec<Account>,
    }
    #[derive(Deserialize)]
    struct Account {
        userid: Value,
        nickname: Option<String>,
        pic: Option<String>,
    }
    let data = match data {
        Some(Value::String(s)) => serde_json::from_str(&s).map_err(|_| malformed())?,
        Some(data) => data,
        None => return Err(malformed()),
    };
    let data: Accounts = serde_json::from_value(data).map_err(|_| malformed())?;
    if data.isreg != 1 || data.info_list.is_empty() || data.info_list.len() > 100 {
        return Err(malformed());
    }
    let mut seen = std::collections::BTreeSet::new();
    data.info_list
        .into_iter()
        .map(|a| {
            let uid = match a.userid {
                Value::String(s) => s,
                Value::Number(n) => n.as_u64().ok_or_else(malformed)?.to_string(),
                _ => return Err(malformed()),
            };
            if !valid_uid(&uid) || !target.permits_user(&uid) || !seen.insert(uid.clone()) {
                return Err(malformed());
            }
            if a.nickname
                .as_ref()
                .is_some_and(|s| s.len() > 1024 || s.chars().any(char::is_control))
            {
                return Err(malformed());
            }
            let avatar_url = a.pic.filter(|s| {
                s.len() <= 8192
                    && url::Url::parse(s).is_ok_and(|u| {
                        matches!(u.scheme(), "https" | "http")
                            && u.host_str().is_some()
                            && u.username().is_empty()
                            && u.password().is_none()
                    })
            });
            Ok(AuthAccountChoice {
                user_id: uid,
                nickname: a.nickname.filter(|s| !s.is_empty()),
                avatar_url,
            })
        })
        .collect()
}
fn invalid() -> TuneWeaveError {
    TuneWeaveError::invalid_request("Invalid KuGou secondary SMS verification input")
        .with_platform(Platform::Kugou)
}
fn malformed() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        "Invalid KuGou secondary SMS response",
    )
    .with_platform(Platform::Kugou)
}
fn rejected(code: i64) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::PermissionDenied,
        "KuGou secondary SMS verification was rejected or requires another interaction",
    )
    .with_platform(Platform::Kugou)
    .with_details(json!({"provider_code":code}))
    .retryable(false)
}
