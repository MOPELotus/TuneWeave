//! Password login follows the current official H5 consumer's plaintext-to-AES contract.
use super::*;
use crate::{credential::AccountCredential, device::KugouDevice};
use serde_json::{Value, json};
use tuneweave_core::{PasswordFormat, PasswordLoginRequest, PrincipalType};

pub(crate) enum WebPasswordOutcome {
    Session(WebSession),
    Phone(String),
}

impl KugouClient {
    /// Authenticates a caller-owned Web account without retaining its password or writing
    /// any account/device state. Only `default` is accepted by this low-level SDK method;
    /// named server accounts and credential ownership belong to KugouProvider.
    pub async fn login_web_password(
        &self,
        request: &PasswordLoginRequest,
    ) -> Result<ProviderAuthResult> {
        if request.account != "default" {
            return Err(invalid());
        }
        let session = self.authenticate_web_password(request).await?;
        Ok(ProviderAuthResult {
            profile: session.profile()?,
            credential: Some(AccountCredential::verified_web(session)?.caller()?),
        })
    }

    // The provider owns the device and transaction deadline across the SMS continuation.
    pub(crate) async fn web_password_attempt(
        &self,
        request: &PasswordLoginRequest,
        device: &KugouDeviceIdentity,
    ) -> Result<WebPasswordOutcome> {
        validate(request)?;
        let cipher = crypto::WebCipher::random()?;
        let params = parameters(request, device, crate::account::now_ms()?, &cipher)?;
        self.web_post(Endpoint::Password, params, None, |headers, bytes| {
            let envelope: Envelope<IgnoredAny> =
                serde_json::from_slice(bytes).map_err(|_| malformed())?;
            if envelope.status == 0 && matches!(envelope.error_code, 30767 | 30768) {
                let envelope: Envelope<String> =
                    serde_json::from_slice(bytes).map_err(|_| malformed())?;
                let phone = envelope.data.ok_or_else(malformed)?;
                super::sms::validate_phone(&phone).map_err(|_| malformed())?;
                return Ok(WebPasswordOutcome::Phone(phone));
            }
            parse(headers, bytes, device).map(WebPasswordOutcome::Session)
        })
        .await
    }

    pub(crate) async fn authenticate_web_password(
        &self,
        request: &PasswordLoginRequest,
    ) -> Result<WebSession> {
        validate(request)?;
        let device = KugouDevice::default().identity().into_web();
        let milliseconds = crate::account::now_ms()?;
        let cipher = crypto::WebCipher::random()?;
        let params = parameters(request, &device, milliseconds, &cipher)?;
        self.web_post(Endpoint::Password, params, None, |headers, bytes| {
            parse(headers, bytes, &device)
        })
        .await
    }
}

pub(crate) fn validate(request: &PasswordLoginRequest) -> Result<()> {
    request.require_backend(Platform::Kugou, tuneweave_core::PasswordLoginBackend::Web)?;
    validate_input(request)
}

pub(crate) fn validate_input(request: &PasswordLoginRequest) -> Result<()> {
    if request.password_format != PasswordFormat::Plain
        || request.secure_captcha.is_some()
        || request.principal.is_empty()
        || request.principal.len() > 256
        || request.principal.trim() != request.principal
        || request.principal.chars().any(char::is_control)
        || request.password.is_empty()
        || request.password.len() > 1024
        || request.password.trim() != request.password
        || request.password.chars().any(char::is_control)
    {
        return Err(invalid());
    }
    match request.principal_type {
        PrincipalType::Phone => {
            if !is_phone(&request.principal)
                || request
                    .country_code
                    .as_deref()
                    .is_some_and(|v| !matches!(v, "86" | "+86"))
            {
                return Err(invalid());
            }
        }
        PrincipalType::Email => {
            let Some((local, domain)) = request.principal.split_once('@') else {
                return Err(invalid());
            };
            if local.is_empty()
                || domain.is_empty()
                || domain.contains('@')
                || request.principal.chars().any(char::is_whitespace)
                || request.country_code.is_some()
            {
                return Err(invalid());
            }
        }
        PrincipalType::Username => {
            if request.country_code.is_some() {
                return Err(invalid());
            }
        }
    }
    Ok(())
}

fn is_phone(value: &str) -> bool {
    value.len() == 11
        && value.starts_with('1')
        && value != "10000000000"
        && value.bytes().all(|b| b.is_ascii_digit())
}

fn parameters(
    request: &PasswordLoginRequest,
    device: &KugouDeviceIdentity,
    milliseconds: u64,
    cipher: &crypto::WebCipher,
) -> Result<BTreeMap<&'static str, String>> {
    #[derive(Serialize)]
    struct Secret<'a> {
        pwd: &'a str,
        #[serde(skip_serializing_if = "Option::is_none")]
        username: Option<&'a str>,
    }
    let phone = is_phone(&request.principal);
    let username = if phone {
        format!(
            "{}********{}",
            &request.principal[..2],
            &request.principal[10..]
        )
    } else {
        request.principal.clone()
    };
    let mut params = BTreeMap::from([
        ("appid", "1014".to_owned()),
        ("srcappid", "2919".to_owned()),
        ("clientver", "1000".to_owned()),
        ("clienttime", milliseconds.to_string()),
        ("clienttime_ms", milliseconds.to_string()),
        ("mid", device.mid.clone()),
        ("uuid", device.mid.clone()),
        ("dfid", device.dfid().to_owned()),
        ("dev", "web".to_owned()),
        ("plat", "4".to_owned()),
        ("support_third", "0".to_owned()),
        ("support_multi", "0".to_owned()),
        ("support_face_verify", "0".to_owned()),
        ("support_verify", "1".to_owned()),
        ("from_sdk", "1".to_owned()),
        ("expire_day", "1".to_owned()),
        ("autologin", "false".to_owned()),
        ("username", username),
        ("pk", cipher.pk(milliseconds)?),
        (
            "params",
            cipher.encrypt(&Secret {
                pwd: &request.password,
                username: phone.then_some(request.principal.as_str()),
            })?,
        ),
    ]);
    params.insert("signature", web_signature(&params, b""));
    Ok(params)
}

#[derive(Deserialize)]
struct Envelope<T> {
    status: i64,
    error_code: i64,
    data: Option<T>,
}
#[derive(Deserialize)]
struct Identity {
    userid: Value,
}

fn parse(
    headers: &reqwest::header::HeaderMap,
    bytes: &[u8],
    device: &KugouDeviceIdentity,
) -> Result<WebSession> {
    let envelope: Envelope<IgnoredAny> = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if envelope.status != 1 || envelope.error_code != 0 {
        let code = envelope.error_code;
        let verification = match code {
            20020 | 20021 => Some("image"),
            30791 => Some("interactive"),
            30767 | 30768 => Some("phone"),
            30798 | 30733 | 34172 => Some("binding"),
            _ => None,
        };
        let mut details = json!({"platform_code":code});
        let category = if let Some(kind) = verification {
            details["additional_verification_required"] = json!(true);
            details["verification_kind"] = json!(kind);
            ErrorCode::PermissionDenied
        } else if matches!(code, 30701..=30703) {
            details["invalid_credentials"] = json!(true);
            ErrorCode::AuthenticationRequired
        } else {
            ErrorCode::UpstreamError
        };
        return Err(error(category, "KuGou Web password login was rejected").with_details(details));
    }
    let envelope: Envelope<Identity> = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    let user_id = match envelope.data.ok_or_else(malformed)?.userid {
        Value::String(v) if valid_uid(&v) => v,
        Value::Number(v) => v
            .as_u64()
            .filter(|v| *v > 0)
            .map(|v| v.to_string())
            .ok_or_else(malformed)?,
        _ => return Err(malformed()),
    };
    let cookie = WebCookie::received(headers, crate::account::now_ms()? / 1000)?;
    if cookie.identity()?.user_id != user_id {
        return Err(conflict());
    }
    Ok(WebSession {
        device: device.clone(),
        user_id,
        cookie,
    })
}

#[cfg(test)]
mod tests;
