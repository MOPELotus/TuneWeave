//! The official Web token exchange establishes identity through a fresh KuGoo cookie.

mod cookies;
mod crypto;
pub(crate) mod library;
pub(crate) mod membership;
pub(crate) mod password;
pub(crate) mod sms;

use std::{collections::BTreeMap, time::Instant};

use reqwest::header::{CONTENT_TYPE, COOKIE, ORIGIN, REFERER};
use serde::{Deserialize, Serialize, de::IgnoredAny};
use tuneweave_core::{
    AccountProfile, ErrorCode, Platform, ProviderAuthResult, ProviderCredential, Result,
    TuneWeaveError,
};

use crate::{
    KugouClient, KugouLoginClient, KugouQrAuthorization,
    credential::{valid_secret, valid_uid},
    device::KugouDeviceIdentity,
    signing::web_signature,
};
pub(crate) use cookies::WebCookie;

pub(crate) const KIND: &str = "kugou_web_v1";
const HOST: &str = "loginservice.kugou.com";
const PATH: &str = "/v1/login_by_token_get";

#[derive(Clone, Copy)]
enum Endpoint {
    Token,
    Password,
}
impl Endpoint {
    fn path(self) -> &'static str {
        match self {
            Self::Token => PATH,
            Self::Password => "/v1/login_by_pwd_get",
        }
    }
    fn operation(self) -> &'static str {
        match self {
            Self::Token => "web_token_exchange",
            Self::Password => "web_password_login",
        }
    }
    fn origin(self) -> &'static str {
        match self {
            Self::Token => "https://login-user.kugou.com",
            Self::Password => "https://m.kugou.com",
        }
    }
    fn referer(self) -> &'static str {
        match self {
            Self::Token => "https://login-user.kugou.com/",
            Self::Password => {
                "https://m.kugou.com/loginReg.php?act=login&appid=1014&logintype=account"
            }
        }
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct WebSession {
    pub device: KugouDeviceIdentity,
    pub user_id: String,
    pub cookie: WebCookie,
}

impl std::fmt::Debug for WebSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("WebSession { fields: [redacted] }")
    }
}

impl WebSession {
    pub(crate) fn media_token(&self) -> Result<String> {
        if !self.valid() {
            return Err(invalid());
        }
        self.cookie.media_token(crate::account::now_ms()? / 1000)
    }

    #[cfg(test)]
    pub(crate) fn test_session(user_id: &str, token: &str) -> Self {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert(reqwest::header::SET_COOKIE, format!("KuGoo=KugooID={user_id}&t={token}&a_id=1014&NickName=Listener; Domain=.kugou.com; Path=/").parse().unwrap());
        Self {
            device: crate::device::KugouDevice::default().identity().into_web(),
            user_id: user_id.to_owned(),
            cookie: WebCookie::received(&headers, 1700000000).unwrap(),
        }
    }
    pub(crate) fn valid(&self) -> bool {
        self.device.valid_web()
            && valid_uid(&self.user_id)
            && self
                .cookie
                .identity()
                .is_ok_and(|identity| identity.user_id == self.user_id)
    }

    pub(crate) fn profile(&self) -> Result<AccountProfile> {
        let identity = self.cookie.identity()?;
        if identity.user_id != self.user_id {
            return Err(conflict());
        }
        Ok(AccountProfile {
            platform: Platform::Kugou,
            account: "default".to_owned(),
            user_id: Some(identity.user_id),
            nickname: identity.nickname,
            avatar_url: identity.avatar_url,
            authenticated: true,
            extensions: BTreeMap::new(),
        })
    }
}

impl KugouClient {
    /// Refreshes an existing caller-owned Web login using the selected cookie/token only.
    /// Returns the replacement credential after a fresh same-user cookie is received.
    pub async fn refresh_web_login(
        &self,
        source: &ProviderCredential,
    ) -> Result<ProviderAuthResult> {
        let previous = crate::credential::AccountCredential::parse_caller(source)?;
        let crate::credential::AccountCredential::Web(web) = &previous else {
            return Err(invalid());
        };
        let session = self.refresh_web_session(&web.session).await?;
        let profile = session.profile()?;
        let credential = previous.rotate_web(session)?.caller()?;
        Ok(ProviderAuthResult {
            profile,
            credential: Some(credential),
        })
    }
    /// Completes a Web QR through the official HTTPS exchange and its fresh identity cookie.
    /// No browser, account store or anonymous device state is modified. Real account
    /// acceptance is separate from the synthetic protocol tests.
    pub async fn complete_web_qr_login(
        &self,
        authorization: KugouQrAuthorization,
    ) -> Result<ProviderAuthResult> {
        let session = self.exchange_web_qr(authorization).await?;
        let profile = session.profile()?;
        let credential = crate::credential::AccountCredential::verified_web(session)?.caller()?;
        Ok(ProviderAuthResult {
            profile,
            credential: Some(credential),
        })
    }

    pub(crate) async fn exchange_web_qr(
        &self,
        authorization: KugouQrAuthorization,
    ) -> Result<WebSession> {
        let (client, device, user_id, token) = authorization.into_parts();
        if client != KugouLoginClient::Web {
            return Err(invalid());
        }
        self.web_exchange(&device, &user_id, &token, None).await
    }

    pub(crate) async fn refresh_web_session(&self, session: &WebSession) -> Result<WebSession> {
        if !session.valid() {
            return Err(invalid());
        }
        let identity = session.cookie.identity()?;
        self.web_exchange(
            &session.device,
            &session.user_id,
            &identity.token,
            Some(&session.cookie),
        )
        .await
    }

    async fn web_exchange(
        &self,
        device: &KugouDeviceIdentity,
        user_id: &str,
        token: &str,
        previous: Option<&WebCookie>,
    ) -> Result<WebSession> {
        if !device.valid_web() || !valid_uid(user_id) || !valid_secret(token) {
            return Err(invalid());
        }
        let milliseconds = crate::account::now_ms()?;
        let cipher = crypto::WebCipher::random()?;
        let mut params = BTreeMap::from([
            ("appid", "1014".to_owned()),
            ("srcappid", "2919".to_owned()),
            ("clientver", "1000".to_owned()),
            ("clienttime", (milliseconds / 1000).to_string()),
            ("clienttime_ms", milliseconds.to_string()),
            ("mid", device.mid.clone()),
            ("uuid", device.mid.clone()),
            ("dfid", device.dfid().to_owned()),
            ("dev", "web".to_owned()),
            ("userid", user_id.to_owned()),
            ("plat", "4".to_owned()),
            // The official login page requests one day by default. Actual cookie expiry
            // still comes only from Set-Cookie and is never inferred from this preference.
            ("expire_day", "1".to_owned()),
            ("pk", cipher.pk(milliseconds)?),
            ("params", cipher.token(token)?),
        ]);
        params.insert("signature", web_signature(&params, b""));
        self.web_post(Endpoint::Token, params, previous, |headers, bytes| {
            #[derive(Deserialize)]
            struct Envelope {
                status: i64,
                error_code: i64,
                data: Option<IgnoredAny>,
            }
            let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| malformed())?;
            let _ = envelope.data;
            if envelope.status != 1 || envelope.error_code != 0 {
                return Err(error(
                    ErrorCode::UpstreamError,
                    "KuGou Web token exchange was rejected",
                )
                .with_details(serde_json::json!({"platform_code":envelope.error_code})));
            }
            let cookie = WebCookie::received(headers, crate::account::now_ms()? / 1000)?;
            let identity = cookie.identity()?;
            if identity.user_id != user_id {
                return Err(conflict());
            }
            Ok(WebSession {
                device: device.clone(),
                user_id: user_id.to_owned(),
                cookie,
            })
        })
        .await
    }

    async fn web_post<T>(
        &self,
        endpoint: Endpoint,
        params: BTreeMap<&str, String>,
        previous: Option<&WebCookie>,
        decode: impl FnOnce(&reqwest::header::HeaderMap, &[u8]) -> Result<T>,
    ) -> Result<T> {
        let path = endpoint.path();
        let url = format!("https://{HOST}{path}");
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(path).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let mut request = self
                .http
                .post(url)
                .query(&params)
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header("accept", "application/json")
                .header(ORIGIN, endpoint.origin())
                .header(REFERER, endpoint.referer())
                .body(Vec::new());
            if let Some(cookie) = previous {
                request = request.header(COOKIE, cookie.header(crate::account::now_ms()? / 1000)?);
            }
            let response = request
                .send()
                .await
                .map_err(crate::account::network_error)?;
            status = Some(response.status());
            let headers = response.headers().clone();
            let bytes = crate::account::read_response(response).await?;
            decode(&headers, &bytes)
        }
        .await;
        self.log_upstream_request(
            endpoint.operation(),
            HOST,
            path,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

fn error(code: ErrorCode, message: &'static str) -> TuneWeaveError {
    TuneWeaveError::new(code, message).with_platform(Platform::Kugou)
}
fn malformed() -> TuneWeaveError {
    error(
        ErrorCode::UpstreamError,
        "KuGou Web login response or cookie is invalid",
    )
}
fn invalid() -> TuneWeaveError {
    error(
        ErrorCode::InvalidRequest,
        "KuGou Web login input is invalid",
    )
}
fn conflict() -> TuneWeaveError {
    error(
        ErrorCode::Conflict,
        "KuGou Web login returned another identity",
    )
}
