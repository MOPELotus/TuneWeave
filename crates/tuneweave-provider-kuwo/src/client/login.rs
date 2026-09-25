//! Current official login challenges, with a separate anonymous session per login attempt.
use super::*;
use rand::{TryRng, rngs::SysRng};
use reqwest::header::{CONTENT_TYPE, HeaderMap, RETRY_AFTER};
use std::time::{SystemTime, UNIX_EPOCH};
use tuneweave_core::{AuthImageAnswerKind, AuthImageChallenge};

const CAPTCHA_PATH: &str = "/api/common/captcha/getcode";
const WEB_FORM_CAPTCHA_PATH: &str = "/api/captcha/getCommonCodeWithKaptcha";
const WEB_PASSWORD_PATH: &str = "/api/www/kuwoLogin";
const WEB_CHECK_LOGIN_PATH: &str = "/api/user/checkLogin";
const WEB_LOGIN_REFERER: &str = "https://www.kuwo.cn/www/user/register";
const WEB_SMS_PAGE: &str = "/vip/added/webView/kwOutLogin/index.html";
const WEB_SMS_HANGER_PATH: &str = "/vip/manage/hanger";
const WEB_SMS_SEND_URL: &str = "https://i.kuwo.cn/US_NEW/kuwo/send_sms_mp";
const WEB_SMS_LOGIN_URL: &str = "https://i.kuwo.cn/US_NEW/kuwo/login_sms";
const WEB_SMS_KEY: &str = "kw@#d09b";
const WEB_SMS_DEVICE_ID: &str = "loginPlugin";
const WEB_SMS_VERSION: &str = "MUSIC_9.0.9.0_BCS5";
const WEB_SMS_SX: &str = "15604173";
const WEB_SMS_TIMEOUT: Duration = Duration::from_secs(20);
const LOCAL_LIFETIME: Duration = Duration::from_secs(300);
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
const MAX_REFRESHES: u8 = 5;
const JSON_LIMIT: u64 = 256 * 1024;
const IMAGE_LIMIT: usize = 128 * 1024;

/// A current official Web form challenge. Cookies and the image key stay private.
pub(crate) struct KuwoWebFormChallenge {
    session: LoginSession,
    captcha: Option<WebFormCaptcha>,
    deadline: Instant,
    refresh_at: Instant,
    refreshes: u8,
}
impl fmt::Debug for KuwoWebFormChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KuwoWebFormChallenge")
            .field("ready", &self.captcha.is_some())
            .field("expired", &(Instant::now() >= self.deadline))
            .field("refreshes", &self.refreshes)
            .finish_non_exhaustive()
    }
}
struct WebFormCaptcha {
    key: String,
    image: String,
}
impl KuwoWebFormChallenge {
    pub(crate) fn image(&self) -> Result<AuthImageChallenge> {
        self.check_live()?;
        let captcha = self.captcha.as_ref().ok_or_else(missing)?;
        Ok(AuthImageChallenge {
            image_data_url: captcha.image.clone(),
            answer_kind: AuthImageAnswerKind::Alphanumeric,
            remaining_attempts: 1,
            refresh_after_secs: self.refresh_after_secs(),
        })
    }
    pub(crate) fn validate_answer(&self, answer: &str) -> Result<()> {
        self.check_live()?;
        if self.captcha.is_none()
            || !(1..=6).contains(&answer.len())
            || !answer.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return Err(kuwo_invalid_request(
                "Kuwo Web image answers must contain 1–6 ASCII letters or digits",
            ));
        }
        Ok(())
    }
    fn check_live(&self) -> Result<()> {
        if Instant::now() >= self.deadline {
            Err(missing())
        } else {
            Ok(())
        }
    }
    fn refresh_after_secs(&self) -> u64 {
        self.refresh_at
            .saturating_duration_since(Instant::now())
            .as_millis()
            .div_ceil(1000) as u64
    }
}

pub(crate) struct KuwoWebFormSession {
    pub(crate) user_id: String,
    pub(crate) session_id: String,
}
impl fmt::Debug for KuwoWebFormSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KuwoWebFormSession { fields: [redacted] }")
    }
}

struct LoginSession {
    cookie: String,
}

/// Anonymous receipt for the official PC Web SMS flow. It is single-use and
/// keeps the Web cookie and platform timestamp private until identity checks finish.
pub(crate) struct KuwoWebSmsChallenge {
    session: LoginSession,
    phone: String,
    server_tm: String,
    deadline: Instant,
}
impl fmt::Debug for KuwoWebSmsChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("KuwoWebSmsChallenge { fields: [redacted], authenticated: false }")
    }
}
impl KuwoWebSmsChallenge {
    fn check(&self) -> Result<()> {
        if Instant::now() >= self.deadline {
            return Err(TuneWeaveError::new(
                ErrorCode::Conflict,
                "Kuwo Web SMS receipt has expired",
            )
            .with_platform(Platform::Kuwo));
        }
        Ok(())
    }
}
struct Captcha {
    token: String,
    image: String,
}

/// An unverified login challenge. This is neither an account nor a credential.
///
/// The browser session and captcha token remain private and are never serialized.
/// The five-minute lifetime and two-second refresh interval are local limits, not
/// assertions about the platform's expiry. No SMS or login request is sent here.
pub struct KuwoLoginChallenge {
    session: LoginSession,
    captcha: Option<Captcha>,
    deadline: Instant,
    refresh_at: Instant,
    refreshes: u8,
}
impl fmt::Debug for KuwoLoginChallenge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KuwoLoginChallenge")
            .field(
                "ready",
                &self.captcha.as_ref().is_some_and(|c| !c.token.is_empty()),
            )
            .field("expired", &(Instant::now() >= self.deadline))
            .field("refreshes", &self.refreshes)
            .finish_non_exhaustive()
    }
}
impl KuwoLoginChallenge {
    /// Returns only the image and local interaction limits, never its upstream token.
    pub fn image(&self) -> Result<AuthImageChallenge> {
        self.check_live()?;
        let captcha = self.captcha.as_ref().ok_or_else(missing)?;
        Ok(AuthImageChallenge {
            image_data_url: captcha.image.clone(),
            answer_kind: AuthImageAnswerKind::Alphanumeric,
            remaining_attempts: 1,
            refresh_after_secs: self.refresh_after_secs(),
        })
    }

    /// Checks the official browser's alphanumeric answer format without submitting it.
    pub fn validate_answer(&self, answer: &str) -> Result<()> {
        self.check_live()?;
        if self.captcha.is_none() {
            return Err(missing());
        }
        if !(1..=6).contains(&answer.len()) || !answer.bytes().all(|b| b.is_ascii_alphanumeric()) {
            return Err(kuwo_invalid_request(
                "Kuwo image answers must contain 1–6 ASCII letters or digits",
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn expires_in_secs(&self) -> u64 {
        self.deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .div_ceil(1000) as u64
    }
    fn refresh_after_secs(&self) -> u64 {
        self.refresh_at
            .saturating_duration_since(Instant::now())
            .as_millis()
            .div_ceil(1000) as u64
    }
    fn check_live(&self) -> Result<()> {
        if Instant::now() >= self.deadline {
            Err(missing())
        } else {
            Ok(())
        }
    }
}

impl KuwoClient {
    /// Starts the current official anonymous PC Web SMS flow. The platform page
    /// says an unregistered number creates an account; callers must separately
    /// collect account-creation and platform-policy consent before calling.
    pub(crate) async fn send_web_login_sms(&self, phone: &str) -> Result<KuwoWebSmsChallenge> {
        let native_request = KuwoNativeSmsRequest {
            phone: phone.to_owned(),
            allow_account_creation: true,
        };
        native::sms::validate_request(&native_request)?;
        let deadline = Instant::now() + LOCAL_LIFETIME;
        let request_tm = epoch_millis()?;
        let session = self.web_sms_session().await?;
        let param = format!(
            "devType=pc&sx={WEB_SMS_SX}&from=pc&dev_name=pc&devType=pc&devResolution=240&version={WEB_SMS_VERSION}&src=pc&type=4&dev_id={WEB_SMS_DEVICE_ID}&tm={request_tm}&mobile={phone}"
        );
        let started = Instant::now();
        let mut status = None;
        let result = tokio::time::timeout(WEB_SMS_TIMEOUT, async {
            let response = self
                .http
                .get(self.vip1_login_target(WEB_SMS_HANGER_PATH))
                .query(&[
                    ("op", "connectToLoginSys"),
                    ("url", WEB_SMS_SEND_URL),
                    ("param", param.as_str()),
                    ("f", "pc"),
                    ("key", WEB_SMS_KEY),
                    ("sx", WEB_SMS_SX),
                ])
                .header(ACCEPT, "application/json,text/plain,*/*")
                .header(ORIGIN, "https://vip1.kuwo.cn")
                .header(REFERER, format!("https://vip1.kuwo.cn{WEB_SMS_PAGE}"))
                .send()
                .await
                .map_err(kuwo_network_error)?;
            status = Some(response.status());
            let bytes = read_web_json_response(response, JSON_LIMIT).await?;
            parse_pc_sms_send_ack(&bytes, &epoch_millis()?)
        })
        .await
        .map_err(|_| web_timeout())?;
        self.log_upstream_request(
            "web_sms_send",
            "vip1.kuwo.cn",
            WEB_SMS_HANGER_PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        let server_tm = result?;
        let challenge = KuwoWebSmsChallenge {
            session,
            phone: phone.to_owned(),
            server_tm,
            deadline,
        };
        challenge.check()?;
        Ok(challenge)
    }

    /// Completes one PC Web SMS receipt. The returned UID/SID must still pass
    /// both Web checkLogin and the independent native-session validation.
    pub(crate) async fn complete_web_login_sms(
        &self,
        challenge: &KuwoWebSmsChallenge,
        code: &str,
    ) -> Result<KuwoWebFormSession> {
        native::sms::validate_code(code)?;
        challenge.check()?;
        let param = format!(
            "devType=pc&sx={WEB_SMS_SX}\n  &from=pc&dev_name=pc&devType=pc&devResolution=240\n  &version={WEB_SMS_VERSION}&src=pc&type=4&dev_id={WEB_SMS_DEVICE_ID}&tm={}&mobile={}&code={code}",
            challenge.server_tm, challenge.phone
        );
        let started = Instant::now();
        let mut status = None;
        let result = tokio::time::timeout(WEB_SMS_TIMEOUT, async {
            let response = self
                .http
                .get(self.vip1_login_target(WEB_SMS_HANGER_PATH))
                .query(&[
                    ("op", "connectToLoginSys"),
                    ("url", WEB_SMS_LOGIN_URL),
                    ("param", param.as_str()),
                    ("f", "pc"),
                    ("key", WEB_SMS_KEY),
                    ("sx", WEB_SMS_SX),
                ])
                .header(ACCEPT, "application/json,text/plain,*/*")
                .header(ORIGIN, "https://vip1.kuwo.cn")
                .header(REFERER, format!("https://vip1.kuwo.cn{WEB_SMS_PAGE}"))
                .send()
                .await
                .map_err(kuwo_network_error)?;
            status = Some(response.status());
            let bytes = read_web_json_response(response, JSON_LIMIT).await?;
            let mut session = parse_pc_sms_login_ack(&bytes)?;
            if !valid_web_uid(&session.user_id) || !valid_web_sid(&session.session_id) {
                return Err(invalid());
            }
            let session_id = session
                .session_id
                .split('_')
                .next()
                .filter(|sid| !sid.is_empty())
                .ok_or_else(invalid)?
                .to_owned();
            if !valid_web_sid(&session_id) {
                return Err(invalid());
            }
            // The PC H5 client writes userid/websid itself and validates them
            // against the anonymous www.kuwo.cn cookie initialized before the flow.
            // Do not forward vip1's host-scoped cookies across subdomains.
            self.validate_web_form_session(
                &challenge.session.cookie,
                &session.user_id,
                &session.session_id,
                &session_id,
            )
            .await?;
            session.session_id = session_id;
            Ok(session)
        })
        .await
        .map_err(|_| web_timeout())?;
        self.log_upstream_request(
            "web_sms_login",
            "vip1.kuwo.cn",
            WEB_SMS_HANGER_PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        challenge.check()?;
        result
    }

    async fn web_sms_session(&self) -> Result<LoginSession> {
        // The landing page itself does not initialize the www host-only cookie
        // used by checkLogin, so establish that anonymous session independently.
        let session = self.login_session().await?;
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .get(self.vip1_login_target(WEB_SMS_PAGE))
                .header(ACCEPT, "text/html,application/xhtml+xml")
                .send()
                .await
                .map_err(kuwo_network_error)?;
            status = Some(response.status());
            read_login_response(response, "text/html", 2 * 1024 * 1024).await?;
            Ok(())
        }
        .await;
        self.log_upstream_request(
            "web_sms_session",
            "vip1.kuwo.cn",
            WEB_SMS_PAGE,
            status,
            started,
            0,
            false,
            &result,
        );
        result.map(|()| session)
    }

    fn vip1_login_target(&self, path: &str) -> String {
        #[cfg(test)]
        if let Some(origin) = &self.login_test_origin {
            return origin.join(path).unwrap().to_string();
        }
        format!("https://vip1.kuwo.cn{path}")
    }

    /// Starts the official Web password form challenge in its own anonymous
    /// browser session. It does not send the principal or password.
    pub(crate) async fn create_web_form_password_challenge(&self) -> Result<KuwoWebFormChallenge> {
        let deadline = Instant::now() + LOCAL_LIFETIME;
        let session = self.login_session_for_page("/www/user/register").await?;
        let captcha = self.web_form_captcha(&session).await?;
        Ok(KuwoWebFormChallenge {
            session,
            captcha: Some(captcha),
            deadline,
            refresh_at: Instant::now() + REFRESH_INTERVAL,
            refreshes: 0,
        })
    }

    /// Refreshes the official form image in the same session. Once the request
    /// begins, the prior key and image are retired even when the request fails.
    pub(crate) async fn refresh_web_form_password_challenge(
        &self,
        challenge: &mut KuwoWebFormChallenge,
    ) -> Result<()> {
        challenge.check_live()?;
        let delay = challenge.refresh_after_secs();
        if delay != 0 || challenge.refreshes >= MAX_REFRESHES {
            return Err(TuneWeaveError::new(
                ErrorCode::RateLimited,
                "Kuwo Web image refresh is limited",
            )
            .with_platform(Platform::Kuwo)
            .with_details(json!({"retry_after_secs":if challenge.refreshes>=MAX_REFRESHES {LOCAL_LIFETIME.as_secs()} else {delay}})));
        }
        challenge.captcha = None;
        challenge.refreshes += 1;
        challenge.refresh_at = Instant::now() + REFRESH_INTERVAL;
        let result = self.web_form_captcha(&challenge.session).await;
        challenge.check_live()?;
        challenge.captcha = Some(result?);
        challenge.refresh_at = Instant::now() + REFRESH_INTERVAL;
        Ok(())
    }

    /// Submits the official Web form, confirms its documented success receipt,
    /// then independently checks the returned UID/SID against the Web session.
    pub(crate) async fn submit_web_form_password(
        &self,
        challenge: &KuwoWebFormChallenge,
        principal: &str,
        password: &str,
        answer: &str,
    ) -> Result<KuwoWebFormSession> {
        challenge.validate_answer(answer)?;
        if principal.is_empty()
            || principal.len() > 256
            || principal.trim() != principal
            || principal.chars().any(char::is_control)
            || password.is_empty()
            || password.len() > 1024
            || password.chars().any(char::is_control)
        {
            return Err(kuwo_invalid_request("Kuwo Web password input is invalid"));
        }
        let captcha_key = challenge.captcha.as_ref().ok_or_else(missing)?.key.clone();
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .post(self.login_target(WEB_PASSWORD_PATH))
                .header(ACCEPT, "application/json,text/plain,*/*")
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(ORIGIN, "https://www.kuwo.cn")
                .header(REFERER, WEB_LOGIN_REFERER)
                .header(
                    COOKIE,
                    format!("{WEB_SESSION_COOKIE}={}", challenge.session.cookie),
                )
                .form(&[
                    ("uname", principal),
                    ("password", password),
                    ("verifyCode", answer),
                    ("verifyCodeKey", captcha_key.as_str()),
                    ("retUrl", WEB_LOGIN_REFERER),
                    ("keepLogin", "1"),
                ])
                .send()
                .await
                .map_err(kuwo_network_error)?;
            status = Some(response.status());
            let headers = response.headers().clone();
            let bytes = read_web_json_response(response, JSON_LIMIT).await?;
            parse_web_login_ack(&bytes)?;
            let user_id = response_cookie(&headers, "userid")?.ok_or_else(invalid)?;
            let raw_sid = response_cookie(&headers, "sid")?.ok_or_else(invalid)?;
            if !valid_web_uid(&user_id) {
                return Err(invalid());
            }
            // The official Web client discards the suffix after the first
            // underscore before calling checkLogin.
            let session_id = raw_sid
                .split('_')
                .next()
                .filter(|sid| !sid.is_empty())
                .ok_or_else(invalid)?
                .to_owned();
            if !valid_web_sid(&session_id) {
                return Err(invalid());
            }
            let cookie = response_cookie(&headers, WEB_SESSION_COOKIE)?
                .unwrap_or_else(|| challenge.session.cookie.clone());
            self.validate_web_form_session(&cookie, &user_id, &raw_sid, &session_id)
                .await?;
            Ok(KuwoWebFormSession {
                user_id,
                session_id,
            })
        }
        .await;
        self.log_upstream_request(
            "web_password_login",
            "www.kuwo.cn",
            WEB_PASSWORD_PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }

    async fn validate_web_form_session(
        &self,
        web_cookie: &str,
        user_id: &str,
        raw_session_id: &str,
        session_id: &str,
    ) -> Result<()> {
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .post(self.login_target(WEB_CHECK_LOGIN_PATH))
                .header(ACCEPT, "application/json")
                .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
                .header(ORIGIN, "https://www.kuwo.cn")
                .header(REFERER, WEB_LOGIN_REFERER)
                .header(
                    COOKIE,
                    format!(
                        "{WEB_SESSION_COOKIE}={web_cookie}; userid={user_id}; sid={raw_session_id}"
                    ),
                )
                .form(&[("uid", user_id), ("sid", session_id)])
                .send()
                .await
                .map_err(kuwo_network_error)?;
            status = Some(response.status());
            let bytes = read_web_json_response(response, JSON_LIMIT).await?;
            parse_web_session_ack(&bytes)
        }
        .await;
        self.log_upstream_request(
            "web_session_validate",
            "www.kuwo.cn",
            WEB_CHECK_LOGIN_PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }

    async fn web_form_captcha(&self, session: &LoginSession) -> Result<WebFormCaptcha> {
        let mut random = SysRng;
        let key = random.try_next_u32().map_err(|_| state_error())? % 999_999;
        let key = key.to_string();
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .get(self.login_target(WEB_FORM_CAPTCHA_PATH))
                .query(&[("key", key.as_str()), ("type", "login")])
                .header(ACCEPT, "image/png,*/*")
                .header(REFERER, WEB_LOGIN_REFERER)
                .header(COOKIE, format!("{WEB_SESSION_COOKIE}={}", session.cookie))
                .send()
                .await
                .map_err(kuwo_network_error)?;
            status = Some(response.status());
            let image = read_web_captcha_response(response).await?;
            Ok(WebFormCaptcha {
                key,
                image: format!("data:image/png;base64,{}", BASE64_STANDARD.encode(image)),
            })
        }
        .await;
        self.log_upstream_request(
            "web_login_captcha",
            "www.kuwo.cn",
            WEB_FORM_CAPTCHA_PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }

    /// Creates a current official captcha in a fresh, isolated browser session.
    /// Does not read or replace the public music session, establish login, or send SMS.
    pub async fn create_login_challenge(&self) -> Result<KuwoLoginChallenge> {
        let deadline = Instant::now() + LOCAL_LIFETIME;
        let session = self.login_session().await?;
        let captcha = self.login_captcha(&session).await?;
        let result = KuwoLoginChallenge {
            session,
            captcha: Some(captcha),
            deadline,
            refresh_at: Instant::now() + REFRESH_INTERVAL,
            refreshes: 0,
        };
        result.check_live()?;
        Ok(result)
    }

    /// Refreshes the image in the same login session. Once a request starts the old
    /// captcha is retired, including on error; a failed refresh cannot revive it.
    pub async fn refresh_login_challenge(&self, challenge: &mut KuwoLoginChallenge) -> Result<()> {
        challenge.check_live()?;
        let delay = challenge.refresh_after_secs();
        if delay != 0 || challenge.refreshes >= MAX_REFRESHES {
            return Err(TuneWeaveError::new(ErrorCode::RateLimited,"Kuwo login image refresh is limited")
                .with_platform(Platform::Kuwo)
                .with_details(json!({"retry_after_secs":if challenge.refreshes>=MAX_REFRESHES {challenge.expires_in_secs()} else {delay}})));
        }
        challenge.captcha = None;
        challenge.refreshes += 1;
        challenge.refresh_at = Instant::now() + REFRESH_INTERVAL;
        let result = self.login_captcha(&challenge.session).await;
        challenge.check_live()?;
        if let Err(error) = &result
            && error.code == ErrorCode::RateLimited
        {
            let delay = error
                .details
                .get("retry_after_secs")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(2)
                .clamp(2, 300);
            challenge.refresh_at = Instant::now() + Duration::from_secs(delay);
        }
        challenge.captcha = Some(result?);
        challenge.refresh_at = Instant::now() + REFRESH_INTERVAL;
        Ok(())
    }

    async fn login_session(&self) -> Result<LoginSession> {
        self.login_session_for_page("/").await
    }

    async fn login_session_for_page(&self, path: &'static str) -> Result<LoginSession> {
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .get(self.login_target(path))
                .header(ACCEPT, "text/html,application/xhtml+xml")
                .send()
                .await
                .map_err(kuwo_network_error)?;
            status = Some(response.status());
            let headers = response.headers().clone();
            read_login_response(response, "text/html", 2 * 1024 * 1024).await?;
            Ok(LoginSession {
                cookie: login_cookie(&headers)?,
            })
        }
        .await;
        self.log_upstream_request(
            "login_session",
            "www.kuwo.cn",
            path,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }

    async fn login_captcha(&self, session: &LoginSession) -> Result<Captcha> {
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .get(self.login_target(CAPTCHA_PATH))
                .header(ACCEPT, "application/json")
                .header(REFERER, HOME_ENDPOINT)
                .header(COOKIE, format!("{WEB_SESSION_COOKIE}={}", session.cookie))
                // The login component uses request export `a`, which signs only
                // relative /api/www and /api/v1 URLs. This captcha path is unsigned.
                // The official caller misspells `methods`; the wrapper appends only
                // these two query parameters before Axios uses its default GET.
                .query(&[("reqId", new_request_id()), ("httpsStatus", "1".into())])
                .send()
                .await
                .map_err(kuwo_network_error)?;
            status = Some(response.status());
            let bytes = read_login_response(response, "application/json", JSON_LIMIT).await?;
            parse_captcha(&bytes)
        }
        .await;
        self.log_upstream_request(
            "login_captcha",
            "www.kuwo.cn",
            CAPTCHA_PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }

    fn login_target(&self, path: &str) -> String {
        #[cfg(test)]
        if let Some(origin) = &self.login_test_origin {
            return origin.join(path).unwrap().to_string();
        }
        format!("https://www.kuwo.cn{path}")
    }
}

fn login_cookie(headers: &HeaderMap) -> Result<String> {
    let mut selected = None;
    for value in headers.get_all(SET_COOKIE) {
        let Ok(value) = value.to_str() else {
            return Err(invalid());
        };
        let mut parts = value.split(';');
        let pair = parts.next().ok_or_else(invalid)?;
        let Some((name, value)) = pair.split_once('=') else {
            continue;
        };
        if name.trim() != WEB_SESSION_COOKIE {
            continue;
        }
        if selected.is_some()
            || !(16..=128).contains(&value.len())
            || !value.bytes().all(|b| b.is_ascii_alphanumeric())
        {
            return Err(invalid());
        }
        for attribute in parts {
            let (key, value) = attribute
                .trim()
                .split_once('=')
                .unwrap_or((attribute.trim(), ""));
            if key.eq_ignore_ascii_case("domain")
                && !matches!(
                    value.trim_start_matches('.').to_ascii_lowercase().as_str(),
                    "kuwo.cn" | "www.kuwo.cn" | "vip1.kuwo.cn"
                )
                || key.eq_ignore_ascii_case("path") && value != "/"
                || key.eq_ignore_ascii_case("max-age")
                    && value.parse::<i64>().map_or(true, |n| n <= 0)
            {
                return Err(invalid());
            }
        }
        selected = Some(value.to_owned());
    }
    selected.ok_or_else(invalid)
}

async fn read_login_response(
    mut response: reqwest::Response,
    mime: &str,
    limit: u64,
) -> Result<Vec<u8>> {
    let status = response.status();
    if !status.is_success() {
        let code = match status {
            StatusCode::TOO_MANY_REQUESTS => ErrorCode::RateLimited,
            StatusCode::UNAUTHORIZED => ErrorCode::AuthenticationRequired,
            StatusCode::FORBIDDEN => ErrorCode::PermissionDenied,
            _ => ErrorCode::UpstreamError,
        };
        let mut error = TuneWeaveError::new(code, "Kuwo login HTTP request failed")
            .with_platform(Platform::Kuwo)
            .with_details(json!({"http_status":status.as_u16()}));
        if code == ErrorCode::RateLimited {
            error.details["retry_after_secs"] = json!(
                response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|s| s.to_str().ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(2)
                    .clamp(2, 300)
            );
        }
        return Err(error);
    }
    let actual = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|s| s.to_str().ok())
        .and_then(|s| s.split(';').next());
    if !actual.is_some_and(|s| s.trim().eq_ignore_ascii_case(mime))
        || response.content_length().is_some_and(|n| n > limit)
        || response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|s| s.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .is_some_and(|n| n > limit)
    {
        return Err(invalid());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(kuwo_network_error)? {
        if bytes.len().saturating_add(chunk.len()) > limit as usize {
            return Err(invalid());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

async fn read_web_json_response(mut response: reqwest::Response, limit: u64) -> Result<Vec<u8>> {
    let status = response.status();
    if !status.is_success() {
        return Err(web_http_error(status));
    }
    let mime = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if !mime.is_some_and(|value| {
        ["application/json", "text/plain", "text/html"]
            .iter()
            .any(|allowed| value.eq_ignore_ascii_case(allowed))
    }) || response.content_length().is_some_and(|size| size > limit)
        || response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|size| size > limit)
    {
        return Err(invalid());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(kuwo_network_error)? {
        if body.len().saturating_add(chunk.len()) > limit as usize {
            return Err(invalid());
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

async fn read_web_captcha_response(mut response: reqwest::Response) -> Result<Vec<u8>> {
    let status = response.status();
    if !status.is_success() {
        return Err(web_http_error(status));
    }
    let mime = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    // The official Kaptcha endpoint currently labels its PNG body as text/html.
    if !mime.is_some_and(|value| {
        value.eq_ignore_ascii_case("image/png") || value.eq_ignore_ascii_case("text/html")
    }) || response
        .content_length()
        .is_some_and(|size| size > IMAGE_LIMIT as u64)
    {
        return Err(invalid());
    }
    let mut image = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(kuwo_network_error)? {
        if image.len().saturating_add(chunk.len()) > IMAGE_LIMIT {
            return Err(invalid());
        }
        image.extend_from_slice(&chunk);
    }
    if !png_envelope(&image) {
        return Err(invalid());
    }
    Ok(image)
}

fn web_http_error(status: StatusCode) -> TuneWeaveError {
    let code = match status {
        StatusCode::TOO_MANY_REQUESTS => ErrorCode::RateLimited,
        StatusCode::UNAUTHORIZED => ErrorCode::AuthenticationRequired,
        StatusCode::FORBIDDEN => ErrorCode::PermissionDenied,
        _ => ErrorCode::UpstreamError,
    };
    TuneWeaveError::new(code, "Kuwo Web login request failed").with_platform(Platform::Kuwo)
}

fn parse_web_login_ack(bytes: &[u8]) -> Result<()> {
    #[derive(Deserialize)]
    struct Ack {
        status: i64,
        msg: String,
    }
    let ack: Ack = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if ack.status == 200 && ack.msg == "成功" {
        Ok(())
    } else {
        Err(TuneWeaveError::new(
            ErrorCode::AuthenticationRequired,
            "Kuwo Web password login was not accepted",
        )
        .with_platform(Platform::Kuwo))
    }
}

fn parse_web_session_ack(bytes: &[u8]) -> Result<()> {
    #[derive(Deserialize)]
    struct Ack {
        status: i64,
    }
    let ack: Ack = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if ack.status == 200 {
        Ok(())
    } else {
        Err(TuneWeaveError::new(
            ErrorCode::AuthenticationRequired,
            "Kuwo Web session did not pass independent validation",
        )
        .with_platform(Platform::Kuwo))
    }
}

fn parse_pc_sms_send_ack(bytes: &[u8], fallback_tm: &str) -> Result<String> {
    #[derive(Deserialize)]
    struct Envelope {
        meta: Meta,
        data: Data,
    }
    #[derive(Deserialize)]
    struct Meta {
        code: i64,
    }
    #[derive(Deserialize)]
    struct Data {
        status: i64,
        #[serde(default)]
        tm: Option<serde_json::Value>,
    }
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if envelope.meta.code != 200 || envelope.data.status != 200 {
        return Err(TuneWeaveError::new(
            ErrorCode::AuthenticationRequired,
            "Kuwo Web SMS was not accepted",
        )
        .with_platform(Platform::Kuwo));
    }
    let Some(value) = envelope.data.tm else {
        return valid_pc_sms_tm(fallback_tm);
    };
    let tm = match value {
        serde_json::Value::String(value) if !value.is_empty() => value,
        serde_json::Value::String(_) => return valid_pc_sms_tm(fallback_tm),
        serde_json::Value::Number(value) => value.to_string(),
        serde_json::Value::Null | serde_json::Value::Bool(false) => {
            return valid_pc_sms_tm(fallback_tm);
        }
        _ => return Err(invalid()),
    };
    if tm == "0" {
        return valid_pc_sms_tm(fallback_tm);
    }
    valid_pc_sms_tm(&tm)
}

fn parse_pc_sms_login_ack(bytes: &[u8]) -> Result<KuwoWebFormSession> {
    #[derive(Deserialize)]
    struct Envelope {
        meta: Meta,
        data: Data,
    }
    #[derive(Deserialize)]
    struct Meta {
        code: i64,
    }
    #[derive(Deserialize)]
    struct Data {
        result: String,
        #[serde(default)]
        uid: Option<serde_json::Value>,
        #[serde(default)]
        sid: Option<String>,
    }
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if envelope.meta.code != 200 || envelope.data.result != "succ" {
        return Err(TuneWeaveError::new(
            ErrorCode::AuthenticationRequired,
            "Kuwo Web SMS login was not accepted",
        )
        .with_platform(Platform::Kuwo));
    }
    let user_id = match envelope.data.uid.ok_or_else(invalid)? {
        serde_json::Value::String(value) => value,
        serde_json::Value::Number(value) => value.to_string(),
        _ => return Err(invalid()),
    };
    Ok(KuwoWebFormSession {
        user_id,
        session_id: envelope.data.sid.ok_or_else(invalid)?,
    })
}

fn valid_pc_sms_tm(value: &str) -> Result<String> {
    if (1..=20).contains(&value.len())
        && value != "0"
        && value.bytes().all(|byte| byte.is_ascii_digit())
    {
        Ok(value.to_owned())
    } else {
        Err(invalid())
    }
}

fn epoch_millis() -> Result<String> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| invalid())?
        .as_millis()
        .to_string())
}

fn web_timeout() -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamTimeout, "Kuwo Web SMS request timed out")
        .with_platform(Platform::Kuwo)
        .retryable(false)
}

fn response_cookie(headers: &HeaderMap, name: &str) -> Result<Option<String>> {
    let mut selected = None;
    for value in headers.get_all(SET_COOKIE) {
        let value = value.to_str().map_err(|_| invalid())?;
        let mut parts = value.split(';');
        let pair = parts.next().ok_or_else(invalid)?;
        let Some((cookie_name, cookie_value)) = pair.split_once('=') else {
            continue;
        };
        if cookie_name.trim() != name {
            continue;
        }
        if selected.is_some()
            || cookie_value.is_empty()
            || cookie_value.len() > 4096
            || !cookie_value
                .bytes()
                .all(|byte| (0x21..=0x7e).contains(&byte) && !matches!(byte, b';' | b',' | b'\\'))
        {
            return Err(invalid());
        }
        for attribute in parts {
            let (key, value) = attribute
                .trim()
                .split_once('=')
                .unwrap_or((attribute.trim(), ""));
            if key.eq_ignore_ascii_case("domain")
                && !matches!(
                    value.trim_start_matches('.').to_ascii_lowercase().as_str(),
                    "kuwo.cn" | "www.kuwo.cn" | "vip1.kuwo.cn"
                )
                || key.eq_ignore_ascii_case("path") && value != "/"
                || key.eq_ignore_ascii_case("max-age")
                    && value.parse::<i64>().map_or(true, |age| age <= 0)
            {
                return Err(invalid());
            }
        }
        selected = Some(cookie_value.to_owned());
    }
    Ok(selected)
}

fn valid_web_uid(value: &str) -> bool {
    !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<i32>().is_ok_and(|id| id > 0)
}

fn valid_web_sid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 4096
        && value != "0"
        && value != "null"
        && value != "undefined"
        && value
            .bytes()
            .all(|byte| (0x21..=0x7e).contains(&byte) && !matches!(byte, b';' | b',' | b'\\'))
}

fn parse_captcha(bytes: &[u8]) -> Result<Captcha> {
    #[derive(Deserialize)]
    struct Envelope {
        code: i64,
        data: Option<Data>,
    }
    #[derive(Deserialize)]
    struct Data {
        img: String,
        token: String,
    }
    let value: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if value.code != 200 {
        return Err(invalid().with_details(json!({"platform_code":value.code})));
    }
    let value = value.data.ok_or_else(invalid)?;
    if value.token.len() != 32 || !value.token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    let encoded = value
        .img
        .strip_prefix("data:image/png;base64,")
        .filter(|s| s.len() <= IMAGE_LIMIT * 4 / 3 + 4)
        .ok_or_else(invalid)?;
    let image = BASE64_STANDARD.decode(encoded).map_err(|_| invalid())?;
    if !png_envelope(&image) {
        return Err(invalid());
    }
    Ok(Captcha {
        token: value.token,
        image: value.img,
    })
}

// Only bounded PNG envelopes may be presented; no URL fetch, SVG, HTML or decoding.
fn png_envelope(bytes: &[u8]) -> bool {
    if bytes.len() < 45 || bytes.len() > IMAGE_LIMIT || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return false;
    }
    let mut offset = 8;
    let mut first = true;
    let mut pixels = false;
    while offset + 12 <= bytes.len() {
        let length = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let Some(end) = offset
            .checked_add(length)
            .and_then(|n| n.checked_add(12))
            .filter(|n| *n <= bytes.len())
        else {
            return false;
        };
        let kind = &bytes[offset + 4..offset + 8];
        if first {
            if kind != b"IHDR" || length != 13 {
                return false;
            }
            let width = u32::from_be_bytes(bytes[offset + 8..offset + 12].try_into().unwrap());
            let height = u32::from_be_bytes(bytes[offset + 12..offset + 16].try_into().unwrap());
            if !(1..=2048).contains(&width) || !(1..=2048).contains(&height) {
                return false;
            }
            first = false;
        } else if kind == b"IHDR" {
            return false;
        }
        if kind == b"IDAT" {
            pixels |= length > 0;
        }
        if kind == b"IEND" {
            return pixels && length == 0 && end == bytes.len();
        }
        offset = end;
    }
    false
}
fn missing() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::ResourceNotFound,
        "Kuwo login image has expired or was retired",
    )
    .with_platform(Platform::Kuwo)
}
fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo login challenge response is invalid")
}
fn state_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        "Kuwo Web login randomness is unavailable",
    )
    .with_platform(Platform::Kuwo)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod web_password_tests;
#[cfg(test)]
mod web_sms_tests;
