//! Standard Android password login. A successful reply is followed by an
//! authenticated profile read before a caller credential can be issued.

use std::{collections::BTreeMap, time::Instant};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use md5::{Digest, Md5};
use serde::Deserialize;
use serde_json::{Value, json};
use tuneweave_core::{
    ErrorCode, PasswordLoginRequest, Platform, ProviderAuthResult, Result, TuneWeaveError,
};

use super::crypto::{self, ExchangeCipher, native_standard::Fingerprint};
use crate::{
    KugouClient, KugouLoginClient, KugouNativePasswordChallengeKind,
    account::{network_error, now_ms, read_response},
    credential::{KugouCredential, NativeSession, valid_secret, valid_uid},
    device::{KugouDevice, KugouDeviceIdentity},
    signing::{ANDROID_SALT, android_signature},
};

// The legacy login.user hostname does not present a matching TLS certificate.
// Use the same verified HTTPS gateway service mapping as native token exchange.
const HOST: &str = "gateway.kugou.com";
const PATH: &str = "/login.user/v9/login_by_pwd";
pub(crate) const CLIENT_VERSION: u32 = 20809;

#[derive(Debug)]
pub(crate) enum NativePasswordOutcome {
    Session(Box<NativeSession>),
    Verification {
        kind: KugouNativePasswordChallengeKind,
        code: i64,
    },
    Phone {
        target: super::native_secondary::Target,
        code: i64,
    },
}

impl NativePasswordOutcome {
    fn into_session(self) -> Result<NativeSession> {
        match self {
            Self::Session(session) => Ok(*session),
            Self::Verification { code, .. } | Self::Phone { code, .. } => Err(rejected(code)),
        }
    }
}

pub(crate) struct NativePasswordAnswer<'a> {
    pub key: &'a str,
    pub answer: &'a str,
}

impl KugouClient {
    /// Logs in through Standard Android's native password protocol and reads the
    /// authenticated profile. Returns a caller-owned `default` native credential.
    /// Passwords and temporary request state are never persisted.
    ///
    /// Supports plain passwords and username/email/domestic phone principals.
    /// Additional image, browser or phone verification returns an error; this
    /// method does not complete those interactions or fall back to Web login.
    /// Use the provider's `begin_password_login` and `advance_password_login`
    /// methods for bound image, browser, and secondary SMS verification flows.
    /// Real-account acceptance is separate from the automated protocol tests.
    pub async fn login_native_password(
        &self,
        request: &PasswordLoginRequest,
    ) -> Result<ProviderAuthResult> {
        if request.account != "default" {
            return Err(TuneWeaveError::invalid_request(
                "Native KuGou SDK login requires the default caller account",
            )
            .with_platform(Platform::Kugou));
        }
        let session = self.authenticate_native_password(request).await?;
        let profile = self.native_profile(&session).await?;
        let credential = KugouCredential::verified(session)?.caller()?;
        Ok(ProviderAuthResult {
            profile,
            credential: Some(credential),
        })
    }

    // This unverified session must pass an authenticated profile read before use.
    pub(crate) async fn authenticate_native_password(
        &self,
        request: &PasswordLoginRequest,
    ) -> Result<NativeSession> {
        self.native_password_attempt(request, &KugouDevice::default().identity(), None)
            .await?
            .into_session()
    }

    pub(crate) async fn native_password_attempt(
        &self,
        request: &PasswordLoginRequest,
        device: &KugouDeviceIdentity,
        answer: Option<NativePasswordAnswer<'_>>,
    ) -> Result<NativePasswordOutcome> {
        validate(request)?;
        let cipher = ExchangeCipher::random()?;
        #[cfg(test)]
        let cipher = self
            .password_test_seed
            .as_deref()
            .map(ExchangeCipher::for_test)
            .unwrap_or(cipher);
        self.native_password_with_cipher(request, device, answer, &cipher)
            .await
    }

    async fn native_password_with_cipher(
        &self,
        request: &PasswordLoginRequest,
        device: &KugouDeviceIdentity,
        answer: Option<NativePasswordAnswer<'_>>,
        cipher: &ExchangeCipher,
    ) -> Result<NativePasswordOutcome> {
        let (query, body) = parameters(request, device, now_ms()?, cipher, answer)?;
        let url = format!("https://{HOST}{PATH}");
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(PATH).unwrap().to_string())
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
            parse(&read_response(response).await?, device, cipher)
        }
        .await;
        self.log_upstream_request(
            "native_password_login",
            HOST,
            PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

pub(crate) fn validate(request: &PasswordLoginRequest) -> Result<()> {
    request.require_backend(
        Platform::Kugou,
        tuneweave_core::PasswordLoginBackend::Native,
    )?;
    crate::web::password::validate_input(request).map_err(|_| {
        TuneWeaveError::invalid_request("KuGou native password login input is invalid")
            .with_platform(Platform::Kugou)
    })
}

fn parameters(
    request: &PasswordLoginRequest,
    device: &KugouDeviceIdentity,
    milliseconds: u64,
    cipher: &ExchangeCipher,
    answer: Option<NativePasswordAnswer<'_>>,
) -> Result<(BTreeMap<&'static str, String>, Vec<u8>)> {
    let fingerprint = Fingerprint::fresh_desktop(milliseconds)?;
    let principal = native_username(&request.principal);
    let mut secret = json!({
        "clienttime_ms":fingerprint.timestamp,
        "pwd":request.password,
        "mobile_data":{
            "access_id":"300011877488",
            "access_key":"663160D210B5D9250C3661DC9A854328"
        }
    });
    let username = if native_phone(&principal) {
        secret["username"] = json!(principal);
        format!("{}*****{}", &principal[..3], &principal[8..])
    } else {
        principal
    };
    if let Some(answer) = &answer {
        secret["verifycode"] = json!(answer.answer);
    }
    let key = hex::encode(Md5::digest(format!(
        "1005{ANDROID_SALT}{CLIENT_VERSION}{}",
        fingerprint.timestamp
    )));
    let mut body = json!({
        "dfid":device.dfid(), "plat":1,
        "t1":fingerprint.t1, "t2":fingerprint.t2,
        // A fresh, unlogged-in context: qo5.c's membership-state defaults.
        "t3":BASE64.encode("0,0,0,0,0,65530,0,0,0"),
        "clienttime_ms":fingerprint.timestamp,
        "pk":cipher.password_pk(&fingerprint.timestamp)?,
        "params":cipher.encrypt(&crypto::encode(&secret)?)?,
        "username":username,
        "support_third":"3", "support_multi":1, "support_verify":1,
        "key":key, "dev":crypto::DESKTOP_MODEL, "gitversion":"0000000",
        "need_toneinfo":1, "busi_type":"kid"
    });
    if let Some(answer) = answer {
        body["verifykey"] = json!(answer.key);
    }
    let body = crypto::encode(&body)?;
    let mut query = BTreeMap::from([
        ("appid", "1005".to_owned()),
        ("clientver", CLIENT_VERSION.to_string()),
        ("clienttime", (milliseconds / 1000).to_string()),
        ("mid", device.mid.clone()),
        ("dfid", device.dfid().to_owned()),
        ("uuid", "-".to_owned()),
    ]);
    query.insert("signature", android_signature(&query, &body));
    Ok((query, body))
}

fn native_username(value: &str) -> String {
    if value
        .chars()
        .any(|ch| ('\u{4e00}'..='\u{9fa5}').contains(&ch))
    {
        value
            .encode_utf16()
            .map(|unit| format!("\\u{unit:04x}"))
            .collect()
    } else {
        value.to_owned()
    }
}

pub(super) fn native_phone(value: &str) -> bool {
    value.len() == 11
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && ["13", "14", "15", "17", "18", "19"]
            .iter()
            .any(|prefix| value.starts_with(prefix))
}

#[derive(Deserialize)]
struct Envelope {
    status: i64,
    #[serde(default)]
    error_code: i64,
    data: Option<Value>,
}

#[derive(Deserialize)]
struct LoggedIn {
    userid: Value,
    secu_params: String,
    t1: Option<String>,
    vip_token: Option<String>,
}

fn parse(
    bytes: &[u8],
    device: &KugouDeviceIdentity,
    cipher: &ExchangeCipher,
) -> Result<NativePasswordOutcome> {
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if envelope.status != 1 || envelope.error_code != 0 {
        if envelope.status == 0 && matches!(envelope.error_code, 30768 | 34216) {
            return Ok(NativePasswordOutcome::Phone {
                target: super::native_secondary::Target::parse(envelope.data)?,
                code: envelope.error_code,
            });
        }
        let kind = match envelope.error_code {
            30709 | 20020 | 20021 => Some(KugouNativePasswordChallengeKind::Image),
            30791 => Some(KugouNativePasswordChallengeKind::Interactive),
            _ => None,
        };
        return match kind {
            Some(kind) => Ok(NativePasswordOutcome::Verification {
                kind,
                code: envelope.error_code,
            }),
            None => Err(rejected(envelope.error_code)),
        };
    }
    parse_session(envelope.data, device, cipher)
        .map(|session| NativePasswordOutcome::Session(Box::new(session)))
}

pub(super) fn parse_session(
    data: Option<Value>,
    device: &KugouDeviceIdentity,
    cipher: &ExchangeCipher,
) -> Result<NativeSession> {
    let login: LoggedIn =
        serde_json::from_value(data.ok_or_else(malformed)?).map_err(|_| malformed())?;
    let user_id = match login.userid {
        Value::String(value) => value,
        Value::Number(value) => value.as_u64().ok_or_else(malformed)?.to_string(),
        _ => return Err(malformed()),
    };
    #[derive(Deserialize)]
    struct Token {
        token: String,
    }
    let token: Token =
        serde_json::from_slice(&cipher.decrypt(&login.secu_params)?).map_err(|_| malformed())?;
    if !valid_uid(&user_id) || !valid_secret(&token.token) {
        return Err(malformed());
    }
    let session = NativeSession {
        client: KugouLoginClient::Standard,
        device: device.clone(),
        user_id,
        token: token.token,
        t1: login.t1.filter(|value| !value.is_empty()),
        vip_token: login.vip_token.filter(|value| !value.is_empty()),
    };
    if !session.valid() {
        return Err(malformed());
    }
    Ok(session)
}

fn rejected(code: i64) -> TuneWeaveError {
    TuneWeaveError::new(
        if matches!(code, 30701..=30703) {
            ErrorCode::AuthenticationRequired
        } else {
            ErrorCode::PermissionDenied
        },
        "KuGou native password login was rejected or requires further verification",
    )
    .with_platform(Platform::Kugou)
    .with_details(json!({"provider_code":code}))
}

fn malformed() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        "Invalid native KuGou login response",
    )
    .with_platform(Platform::Kugou)
}

#[cfg(test)]
mod tests;
