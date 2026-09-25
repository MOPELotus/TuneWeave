//! Native SMS login. Unregistered numbers may create an account on the platform.
use super::*;
use md5::{Digest, Md5};
use tuneweave_core::ProviderAuthResult;

#[cfg(test)]
mod tests;

const SEND_PATH: &str = "/US_NEW/kuwo/send_sms";
const LOGIN_PATH: &str = "/US_NEW/kuwo/login_sms";
const BUDGET: Duration = Duration::from_secs(300);
const SEND_TIMEOUT: Duration = Duration::from_secs(20);

/// Native mainland-China SMS login input. The official login flow can create an
/// account for an unregistered number; consent must be explicit before delivery.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct KuwoNativeSmsRequest {
    pub phone: String,
    pub allow_account_creation: bool,
}
impl fmt::Debug for KuwoNativeSmsRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KuwoNativeSmsRequest")
            .field("phone", &"[redacted]")
            .field("allow_account_creation", &self.allow_account_creation)
            .finish()
    }
}

/// An in-memory, single-use SMS receipt bound to its original installation.
/// This is neither a credential nor proof that the user owns the phone number.
/// It cannot be cloned or serialized. The SDK consumes it on a login attempt.
pub struct KuwoNativeSmsChallenge {
    phone: String,
    server_tm: String,
    device: KuwoNativeDevice,
    deadline: Instant,
    resend_at: Instant,
}
impl fmt::Debug for KuwoNativeSmsChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KuwoNativeSmsChallenge { fields: [redacted], authenticated: false }")
    }
}
impl KuwoNativeSmsChallenge {
    /// Remaining local receipt lifetime, not the platform's SMS-code expiry.
    #[must_use]
    pub fn expires_in_secs(&self) -> u64 {
        self.deadline
            .saturating_duration_since(Instant::now())
            .as_secs()
    }

    /// Remaining delay from the official client's 60-second resend policy.
    /// Applications must enforce this across delivery requests for the same phone.
    #[must_use]
    pub fn resend_after_secs(&self) -> u64 {
        let remaining = self.resend_at.saturating_duration_since(Instant::now());
        remaining
            .as_secs()
            .saturating_add(u64::from(remaining.subsec_nanos() > 0))
    }

    fn check(&self) -> Result<()> {
        if Instant::now() >= self.deadline {
            return Err(
                TuneWeaveError::new(ErrorCode::Conflict, "Kuwo SMS receipt has expired")
                    .with_platform(Platform::Kuwo),
            );
        }
        Ok(())
    }
}

impl KuwoClient {
    /// Sends one login SMS without automatic retries. Only mainland numbers are
    /// supported. Explicit `allow_account_creation` is required: the platform can
    /// register an unregistered number when its SMS login is completed.
    ///
    /// The low-level SDK does not manage account aliases or cross-request resend
    /// limits. The caller must retain this receipt and enforce delivery throttling.
    pub async fn send_native_login_sms(
        &self,
        request: &KuwoNativeSmsRequest,
        device: &KuwoNativeDevice,
    ) -> Result<KuwoNativeSmsChallenge> {
        validate_request(request)?;
        let deadline = Instant::now() + BUDGET;
        let tm = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| invalid())?
            .as_millis()
            .to_string();
        let key = self.native_response_key()?;
        let plain = send_query(request, device, &tm, &key);
        let encoded = codec::seal_query(plain.as_bytes())?;
        let target = format!(
            "{}?f=ar&q={encoded}",
            self.native_target(EXCHANGE_HOST, SEND_PATH)
        );
        let result = tokio::time::timeout(
            SEND_TIMEOUT,
            self.native_get_with_metadata(
                EXCHANGE_HOST,
                SEND_PATH,
                "native_sms_send",
                target,
                Some(password::device_metadata(device)),
                |body| {
                    let decoded = codec::open_response(body, &key)?;
                    parse_send(&decoded)
                },
            ),
        )
        .await
        .map_err(|_| timeout())?;
        let server_tm = result.map_err(|e| e.retryable(false))?;
        let receipt = KuwoNativeSmsChallenge {
            phone: request.phone.clone(),
            server_tm,
            device: device.clone(),
            deadline,
            resend_at: Instant::now() + Duration::from_secs(60),
        };
        receipt.check()?;
        Ok(receipt)
    }

    /// Consumes the receipt and submits a five-digit code once. Independent
    /// UID/SID validation must succeed before any authenticated output is issued.
    /// Failed, expired or cancelled attempts never return a login credential.
    pub async fn login_native_sms(
        &self,
        receipt: KuwoNativeSmsChallenge,
        code: &str,
    ) -> Result<ProviderAuthResult> {
        let exchange = self.authenticate_native_sms(&receipt, code).await?;
        let result = tokio::time::timeout_at(
            tokio::time::Instant::from_std(receipt.deadline),
            self.validate_native_session(exchange.session()),
        )
        .await
        .map_err(|_| timeout())?;
        receipt.check()?;
        result?;
        Ok(ProviderAuthResult {
            profile: credential::profile(
                exchange.session(),
                exchange.nickname().map(str::to_owned),
            ),
            credential: Some(credential::NativeCredential::verified(exchange.session())?.caller()?),
        })
    }

    /// This primitive is for a provider that checks source ownership at each
    /// boundary. Its exchange output still requires independent validation.
    pub(crate) async fn authenticate_native_sms(
        &self,
        receipt: &KuwoNativeSmsChallenge,
        code: &str,
    ) -> Result<KuwoNativeSessionExchange> {
        validate_code(code)?;
        receipt.check()?;
        let key = self.native_response_key()?;
        let plain = login_query(receipt, code, &key);
        let encoded = codec::seal_query(plain.as_bytes())?;
        let target = format!(
            "{}?f=ar&q={encoded}",
            self.native_target(EXCHANGE_HOST, LOGIN_PATH)
        );
        let result = tokio::time::timeout_at(
            tokio::time::Instant::from_std(receipt.deadline),
            self.native_get_with_metadata(
                EXCHANGE_HOST,
                LOGIN_PATH,
                "native_sms_login",
                target,
                Some(password::device_metadata(&receipt.device)),
                |body| {
                    let decoded = codec::open_response(body, &key)?;
                    parse_login(&decoded, receipt, code)
                },
            ),
        )
        .await
        .map_err(|_| timeout())?;
        receipt.check()?;
        result.map_err(|e| e.retryable(false))
    }
}

pub(crate) fn validate_request(request: &KuwoNativeSmsRequest) -> Result<()> {
    let phone = request.phone.as_bytes();
    if !request.allow_account_creation {
        return Err(kuwo_invalid_request(
            "Kuwo SMS login can create an account for an unregistered number; explicit consent is required",
        ));
    }
    if phone.len() != 11
        || phone[0] != b'1'
        || !(b'3'..=b'9').contains(&phone[1])
        || !phone.iter().all(u8::is_ascii_digit)
    {
        return Err(kuwo_invalid_request(
            "Kuwo SMS login requires a mainland China mobile number",
        ));
    }
    Ok(())
}
pub(crate) fn validate_code(code: &str) -> Result<()> {
    if code.len() != 5 || !code.bytes().all(|c| c.is_ascii_digit()) {
        return Err(kuwo_invalid_request(
            "Kuwo SMS login requires a five-digit code",
        ));
    }
    Ok(())
}

fn send_query(
    request: &KuwoNativeSmsRequest,
    device: &KuwoNativeDevice,
    tm: &str,
    key: &[u8; 8],
) -> String {
    let digest = |s: &str| format!("{:X}", Md5::digest(s.as_bytes()));
    let secret = digest(&format!(
        "{}{}",
        digest("imbadboy@!153"),
        digest(&format!("{}4{tm}", request.phone))
    ));
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.extend_pairs([
        ("mobile", request.phone.as_str()),
        ("type", "4"),
        ("tm", tm),
        ("secret", secret.as_str()),
        ("src", CLIENT_SOURCE),
        ("version", CLIENT_VERSION),
        ("dev_id", device.app_uid()),
        ("user", device.device_user()),
        ("dev_name", "TuneWeave SDK client"),
        ("devType", "SDK"),
        (
            "sx",
            std::str::from_utf8(key).expect("numeric protocol key"),
        ),
        ("from", "android"),
        ("devResolution", "0*0"),
    ]);
    query.finish()
}
fn login_query(receipt: &KuwoNativeSmsChallenge, code: &str, key: &[u8; 8]) -> String {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.extend_pairs([
        ("mobile", receipt.phone.as_str()),
        ("code", code),
        ("tm", receipt.server_tm.as_str()),
        ("src", CLIENT_SOURCE),
        ("version", CLIENT_VERSION),
        ("dev_id", receipt.device.app_uid()),
        ("dev_name", "TuneWeave SDK client"),
        ("devType", "SDK"),
        (
            "sx",
            std::str::from_utf8(key).expect("numeric protocol key"),
        ),
        ("from", "android"),
        ("devResolution", "0*0"),
    ]);
    query.finish()
}
#[derive(Deserialize)]
struct SendBody {
    #[serde(deserialize_with = "deserialize_code")]
    status: Option<String>,
    #[serde(default, deserialize_with = "deserialize_tm")]
    tm: Option<String>,
    ret: Option<String>,
    result: Option<String>,
}
fn deserialize_tm<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<String>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    let value = match value {
        None => return Ok(None),
        Some(serde_json::Value::String(v)) => v,
        Some(serde_json::Value::Number(v)) if v.is_u64() => v.to_string(),
        _ => return Err(serde::de::Error::custom("invalid SMS receipt")),
    };
    if !printable(&value, 256) || matches!(value.as_str(), "0" | "null" | "undefined") {
        return Err(serde::de::Error::custom("invalid SMS receipt"));
    }
    Ok(Some(value))
}
fn parse_send(bytes: &[u8]) -> Result<String> {
    let body: SendBody = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if body.status.as_deref() != Some("200")
        || body.ret.as_deref().is_some_and(|s| s != "succ")
        || body.result.as_deref().is_some_and(|s| s != "succ")
    {
        return Err(invalid());
    }
    body.tm.ok_or_else(invalid)
}
fn parse_login(
    bytes: &[u8],
    receipt: &KuwoNativeSmsChallenge,
    code: &str,
) -> Result<KuwoNativeSessionExchange> {
    let body: ExchangeBody = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let (user, sid) = parse_session_body(body)?;
    let reflected = |v: &str| {
        echoes_secret(v, code)
            || echoes_secret(v, &receipt.phone)
            || (receipt.server_tm.len() >= 5 && echoes_secret(v, &receipt.server_tm))
    };
    if reflected(&sid)
        || user.nickname.as_ref().is_some_and(|name| {
            !valid_nickname(name) || echoes_secret(name, &sid) || reflected(name)
        })
    {
        return Err(invalid());
    }
    Ok(KuwoNativeSessionExchange {
        session: receipt
            .device
            .session_input(&user.uid, &sid)
            .map_err(|_| invalid())?,
        nickname: user.nickname.filter(|n| !n.is_empty()),
    })
}
fn timeout() -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, "Kuwo SMS request timed out")
        .with_platform(Platform::Kuwo)
        .retryable(false)
}
