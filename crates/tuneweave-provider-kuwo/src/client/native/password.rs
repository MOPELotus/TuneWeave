use super::*;
use credential::NativeCredential;
use tuneweave_core::{
    AccountProfile, PasswordFormat, PasswordLoginRequest, PrincipalType, ProviderAuthResult,
    ProviderCredential,
};

#[cfg(test)]
mod tests;
const PATH: &str = "/US_NEW/kuwo/login_kw";

impl KuwoClient {
    /// Logs in with a plain password, then independently validates the returned
    /// native UID/SID before issuing a caller credential. Only `default` is
    /// accepted here; named accounts and ownership belong to the provider layer.
    pub async fn login_native_password(
        &self,
        request: &PasswordLoginRequest,
        device: &KuwoNativeDevice,
    ) -> Result<ProviderAuthResult> {
        if request.account != "default" {
            return Err(invalid_input());
        }
        let authorization = self.authenticate_native_password(request, device).await?;
        self.validate_native_session(authorization.session())
            .await?;
        let credential = NativeCredential::verified(authorization.session())?.caller()?;
        Ok(ProviderAuthResult {
            profile: credential::profile(
                authorization.session(),
                authorization.nickname().map(str::to_owned),
            ),
            credential: Some(credential),
        })
    }

    /// Independently checks a caller's native identity without rotating the SID.
    /// Only UID is returned; membership, nickname and avatar need separate reads.
    pub async fn validate_native_login(
        &self,
        credential: &ProviderCredential,
    ) -> Result<AccountProfile> {
        let session = NativeCredential::parse(credential)?.input()?;
        self.validate_native_session(&session).await?;
        Ok(credential::profile(&session, None))
    }

    /// The returned authorization must still pass independent validation.
    pub(crate) async fn authenticate_native_password(
        &self,
        request: &PasswordLoginRequest,
        device: &KuwoNativeDevice,
    ) -> Result<KuwoNativeSessionExchange> {
        validate_request(request)?;
        let key = self.native_response_key()?;
        let plain = password_query(request, device, &key);
        let encoded = codec::seal_query(plain.as_bytes())?;
        let target = format!(
            "{}?f=ar&q={encoded}",
            self.native_target(EXCHANGE_HOST, PATH)
        );
        self.native_get_with_metadata(
            EXCHANGE_HOST,
            PATH,
            "native_password_login",
            target,
            Some(device_metadata(device)),
            |body| {
                let decoded = codec::open_response(body, &key)?;
                parse_password(&decoded, request, device)
            },
        )
        .await
    }
}

pub(crate) fn validate_request(request: &PasswordLoginRequest) -> Result<()> {
    request.require_backend(Platform::Kuwo, tuneweave_core::PasswordLoginBackend::Native)?;
    // The official account and phone password pages share login_kw's username
    // field. These are password identifiers, never SMS login or UID inputs.
    let principal_supported = match request.principal_type {
        PrincipalType::Username => request.country_code.is_none(),
        PrincipalType::Phone => {
            let phone = request.principal.as_bytes();
            matches!(request.country_code.as_deref(), None | Some("86" | "+86"))
                && phone.len() == 11
                && phone[0] == b'1'
                && (b'3'..=b'9').contains(&phone[1])
                && phone.iter().all(u8::is_ascii_digit)
        }
        PrincipalType::Email => {
            request.country_code.is_none()
                && !request.principal.chars().any(char::is_whitespace)
                && request
                    .principal
                    .split_once('@')
                    .is_some_and(|(local, domain)| {
                        !local.is_empty() && !domain.is_empty() && !domain.contains('@')
                    })
        }
    };
    if !principal_supported
        || request.password_format != PasswordFormat::Plain
        || request.secure_captcha.is_some()
        || request.principal.is_empty()
        || request.principal.len() > 256
        || request.principal.trim() != request.principal
        || request.principal.chars().any(char::is_control)
        || request.password.is_empty()
        || request.password.len() > 1024
        || request.password.chars().any(char::is_control)
    {
        return Err(invalid_input());
    }
    Ok(())
}
fn password_query(
    request: &PasswordLoginRequest,
    device: &KuwoNativeDevice,
    key: &[u8; 8],
) -> String {
    let password = BASE64_STANDARD.encode(request.password.as_bytes());
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    query.extend_pairs([
        ("username", request.principal.as_str()),
        ("password", password.as_str()),
        ("dev_id", device.app_uid()),
        ("user", device.device_user()),
        ("dev_name", "TuneWeave SDK client"),
        ("urlencode", "0"),
        ("src", CLIENT_SOURCE),
        ("devResolution", "0*0"),
        ("from", "android"),
        ("devType", "SDK"),
        (
            "sx",
            std::str::from_utf8(key).expect("numeric protocol key"),
        ),
        ("version", CLIENT_VERSION),
    ]);
    query.finish()
}
pub(super) fn device_metadata(device: &KuwoNativeDevice) -> String {
    // This plural header carries native metadata, not Web cookies. A fresh login
    // never inherits another account's UID/SID or collects hardware information.
    format!(
        "user={},ct=11,cv=12220,chid=newpcguanwangmobile,QIMEI36={},tmeAppID=kwplayer,loginUid=0,loginSid=0,appUid={},rom=TuneWeave/SDK/SDK,",
        device.device_user(),
        device::FALLBACK_Q36,
        device.app_uid()
    )
}
fn parse_password(
    bytes: &[u8],
    request: &PasswordLoginRequest,
    device: &KuwoNativeDevice,
) -> Result<KuwoNativeSessionExchange> {
    let body: ExchangeBody = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let (user, sid) = parse_session_body(body)?;
    let encoded_password = BASE64_STANDARD.encode(request.password.as_bytes());
    let reflects_password = |value: &str| {
        password_echo(value, &request.password) || password_echo(value, &encoded_password)
    };
    if reflects_password(&sid) {
        return Err(invalid());
    }
    let session = device
        .session_input(&user.uid, &sid)
        .map_err(|_| invalid())?;
    if user.nickname.as_ref().is_some_and(|name| {
        !valid_nickname(name) || echoes_secret(name, &sid) || reflects_password(name)
    }) {
        return Err(invalid());
    }
    Ok(KuwoNativeSessionExchange {
        session,
        nickname: user.nickname.filter(|name| !name.is_empty()),
    })
}
fn password_echo(value: &str, secret: &str) -> bool {
    // Short passwords may naturally share characters with an opaque SID. Only
    // exact short echoes are meaningful; larger reflected secrets are rejected.
    let matches =
        |candidate: &str| candidate == secret || (secret.len() >= 6 && candidate.contains(secret));
    matches(value)
        || url::form_urlencoded::parse(value.as_bytes())
            .any(|(key, value)| matches(&key) || matches(&value))
}
fn invalid_input() -> TuneWeaveError {
    kuwo_invalid_request("Kuwo native password input is invalid or unsupported")
}
