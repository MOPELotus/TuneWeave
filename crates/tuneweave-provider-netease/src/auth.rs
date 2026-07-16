use std::fmt;

use md5::{Digest, Md5};
use serde_json::{Value, json};
use tuneweave_core::{ErrorCode, Platform, Result, TuneWeaveError};

use crate::{
    NeteaseClient, NeteaseResponse,
    client::{ensure_response_code, has_authenticated_cookie, merge_cookie_headers, response_code},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NeteaseAccountSummary {
    pub id: Option<String>,
    pub user_id: Option<String>,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Eq, PartialEq)]
pub struct NeteaseLoginResult {
    pub account: NeteaseAccountSummary,
    session_cookie: String,
}

impl fmt::Debug for NeteaseLoginResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NeteaseLoginResult")
            .field("account", &self.account)
            .field("has_session_cookie", &true)
            .finish()
    }
}

impl NeteaseLoginResult {
    #[must_use]
    pub fn session_cookie(&self) -> &str {
        &self.session_cookie
    }

    #[must_use]
    pub fn into_session_cookie(self) -> String {
        self.session_cookie
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NeteaseCaptchaVerification {
    pub code: i64,
    pub valid: bool,
    pub message: Option<String>,
    pub response: Value,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NeteaseSessionStatus {
    pub authenticated: bool,
    pub account: NeteaseAccountSummary,
}

#[derive(Clone, Eq, PartialEq)]
pub struct NeteaseSessionRefresh {
    session_cookie: String,
}

impl NeteaseSessionRefresh {
    #[must_use]
    pub fn session_cookie(&self) -> &str {
        &self.session_cookie
    }

    #[must_use]
    pub fn into_session_cookie(self) -> String {
        self.session_cookie
    }
}

impl fmt::Debug for NeteaseSessionRefresh {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NeteaseSessionRefresh")
            .field("has_session_cookie", &true)
            .finish()
    }
}

impl NeteaseClient {
    pub async fn logout(&self) -> Result<()> {
        let response = self.request_eapi("/api/logout", json!({})).await?;
        ensure_response_code(&response.body, 200, "logout")
    }

    pub async fn session_status(&self) -> Result<NeteaseSessionStatus> {
        let response = self
            .request_weapi("/api/w/nuser/account/get", json!({}))
            .await?;
        session_status_from_body(&response.body)
    }

    pub async fn refresh_session(&self) -> Result<NeteaseSessionRefresh> {
        if !self.is_authenticated() {
            return Err(TuneWeaveError::new(
                ErrorCode::AuthenticationRequired,
                "NetEase session refresh requires a logged-in account",
            )
            .with_platform(Platform::Netease));
        }
        let response = self
            .request_eapi("/api/login/token/refresh", json!({}))
            .await?;
        ensure_response_code(&response.body, 200, "refresh session")?;
        let session_cookie = merge_cookie_headers(self.configured_cookie(), &response.cookies)
            .filter(|cookie| has_authenticated_cookie(Some(cookie.as_str())))
            .ok_or_else(|| upstream_error("refresh session", "response did not contain MUSIC_U"))?;
        Ok(NeteaseSessionRefresh { session_cookie })
    }

    pub async fn send_phone_captcha(&self, phone: &str, country_code: &str) -> Result<()> {
        let phone = required_value("phone", phone)?;
        let country_code = normalized_country_code(country_code);
        let response = self
            .request_weapi(
                "/api/sms/captcha/sent",
                json!({
                    "ctcode": country_code,
                    "secrete": "music_middleuser_pclogin",
                    "cellphone": phone
                }),
            )
            .await?;
        ensure_response_code(&response.body, 200, "send phone captcha")
    }

    pub async fn verify_phone_captcha(
        &self,
        phone: &str,
        country_code: &str,
        captcha: &str,
    ) -> Result<NeteaseCaptchaVerification> {
        let phone = required_value("phone", phone)?;
        let captcha = required_value("captcha", captcha)?;
        let response = self
            .request_weapi(
                "/api/sms/captcha/verify",
                json!({
                    "ctcode": normalized_country_code(country_code),
                    "cellphone": phone,
                    "captcha": captcha
                }),
            )
            .await?;
        captcha_verification_from_body(response.body)
    }

    pub async fn login_with_email_password(
        &self,
        email: &str,
        password: &str,
    ) -> Result<NeteaseLoginResult> {
        let password_md5 = password_digest(required_value("password", password)?);
        self.login_with_email_md5(email, &password_md5).await
    }

    pub async fn login_with_email_md5(
        &self,
        email: &str,
        password_md5: &str,
    ) -> Result<NeteaseLoginResult> {
        let email = required_value("email", email)?;
        let password_md5 = validated_password_digest(password_md5)?;
        let response = self
            .request_eapi(
                "/api/w/login",
                json!({
                    "type": "0",
                    "https": "true",
                    "username": email,
                    "password": password_md5,
                    "rememberLogin": "true"
                }),
            )
            .await?;
        login_result(self, response, "email password login")
    }

    pub async fn login_with_phone_password(
        &self,
        phone: &str,
        country_code: &str,
        password: &str,
    ) -> Result<NeteaseLoginResult> {
        let password_md5 = password_digest(required_value("password", password)?);
        self.login_with_phone_password_md5(phone, country_code, &password_md5)
            .await
    }

    pub async fn login_with_phone_password_md5(
        &self,
        phone: &str,
        country_code: &str,
        password_md5: &str,
    ) -> Result<NeteaseLoginResult> {
        let phone = required_value("phone", phone)?;
        let password_md5 = validated_password_digest(password_md5)?;
        let response = self
            .request_weapi(
                "/api/w/login/cellphone",
                json!({
                    "type": "1",
                    "https": "true",
                    "phone": phone,
                    "countrycode": normalized_country_code(country_code),
                    "password": password_md5,
                    "remember": "true"
                }),
            )
            .await?;
        login_result(self, response, "phone password login")
    }

    pub async fn login_with_phone_captcha(
        &self,
        phone: &str,
        country_code: &str,
        captcha: &str,
    ) -> Result<NeteaseLoginResult> {
        let phone = required_value("phone", phone)?;
        let captcha = required_value("captcha", captcha)?;
        let response = self
            .request_weapi(
                "/api/w/login/cellphone",
                json!({
                    "type": "1",
                    "https": "true",
                    "phone": phone,
                    "countrycode": normalized_country_code(country_code),
                    "captcha": captcha,
                    "remember": "true"
                }),
            )
            .await?;
        login_result(self, response, "phone captcha login")
    }
}

fn captcha_verification_from_body(body: Value) -> Result<NeteaseCaptchaVerification> {
    let code = response_code(&body).ok_or_else(|| {
        upstream_error(
            "verify phone captcha",
            "response did not contain a status code",
        )
    })?;
    Ok(NeteaseCaptchaVerification {
        code,
        valid: code == 200,
        message: response_message(&body),
        response: body,
    })
}

fn login_result(
    client: &NeteaseClient,
    response: NeteaseResponse,
    operation: &str,
) -> Result<NeteaseLoginResult> {
    let code = response_code(&response.body)
        .ok_or_else(|| upstream_error(operation, "response did not contain a status code"))?;
    if code != 200 {
        return Err(TuneWeaveError::new(
            ErrorCode::AuthenticationRequired,
            response_message(&response.body)
                .unwrap_or_else(|| format!("NetEase {operation} failed")),
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "code": code })));
    }
    let session_cookie = merge_cookie_headers(client.configured_cookie(), &response.cookies)
        .filter(|cookie| has_authenticated_cookie(Some(cookie.as_str())))
        .ok_or_else(|| upstream_error(operation, "response did not contain MUSIC_U"))?;
    Ok(NeteaseLoginResult {
        account: map_account(&response.body),
        session_cookie,
    })
}

fn map_account(body: &Value) -> NeteaseAccountSummary {
    NeteaseAccountSummary {
        id: value_as_id(body.pointer("/account/id")),
        user_id: value_as_id(body.pointer("/profile/userId")),
        nickname: body
            .pointer("/profile/nickname")
            .and_then(Value::as_str)
            .map(str::to_owned),
        avatar_url: body
            .pointer("/profile/avatarUrl")
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

fn session_status_from_body(body: &Value) -> Result<NeteaseSessionStatus> {
    let code = response_code(body).ok_or_else(|| {
        upstream_error("session status", "response did not contain a status code")
    })?;
    if code != 200 && code != 301 {
        return Err(upstream_error(
            "session status",
            &format!("failed with code {code}"),
        ));
    }
    let account = map_account(body);
    let authenticated = code == 200
        && (account.id.is_some() || account.user_id.is_some() || account.nickname.is_some());
    Ok(NeteaseSessionStatus {
        authenticated,
        account,
    })
}

fn value_as_id(value: Option<&Value>) -> Option<String> {
    value.and_then(|value| match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    })
}

fn response_message(body: &Value) -> Option<String> {
    body.get("message")
        .or_else(|| body.get("msg"))
        .and_then(Value::as_str)
        .filter(|message| !message.is_empty())
        .map(str::to_owned)
}

fn required_value<'a>(name: &str, value: &'a str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        return Err(
            TuneWeaveError::invalid_request(format!("NetEase {name} cannot be empty"))
                .with_platform(Platform::Netease),
        );
    }
    Ok(value)
}

fn normalized_country_code(value: &str) -> &str {
    let value = value.trim();
    if value.is_empty() { "86" } else { value }
}

fn password_digest(password: &str) -> String {
    hex::encode(Md5::digest(password.as_bytes()))
}

fn validated_password_digest(value: &str) -> Result<String> {
    let value = required_value("MD5 password", value)?;
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(TuneWeaveError::invalid_request(
            "NetEase MD5 password must contain 32 hexadecimal characters",
        )
        .with_platform(Platform::Netease));
    }
    Ok(value.to_ascii_lowercase())
}

fn upstream_error(operation: &str, reason: &str) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("NetEase {operation} {reason}"),
    )
    .with_platform(Platform::Netease)
}

#[cfg(test)]
mod tests {
    use reqwest::StatusCode;

    use super::*;

    #[test]
    fn password_digest_matches_netease_lowercase_md5() {
        assert_eq!(
            password_digest("TuneWeave-password"),
            "bf7d03ccdbea30d8b131e636b9815e58"
        );
    }

    #[test]
    fn captcha_verification_preserves_valid_invalid_and_raw_responses() {
        let valid = captcha_verification_from_body(json!({
            "code": 200,
            "data": true
        }))
        .expect("valid captcha response");
        assert!(valid.valid);
        assert_eq!(valid.code, 200);
        assert_eq!(valid.response["data"], true);

        let invalid = captcha_verification_from_body(json!({
            "code": 503,
            "message": "验证码错误"
        }))
        .expect("invalid captcha response is still a result");
        assert!(!invalid.valid);
        assert_eq!(invalid.code, 503);
        assert_eq!(invalid.message.as_deref(), Some("验证码错误"));

        let error = captcha_verification_from_body(json!({ "data": false }))
            .expect_err("missing business code");
        assert_eq!(error.code, ErrorCode::UpstreamError);
    }

    #[test]
    fn maps_successful_login_without_exposing_cookie_in_account_data() {
        let client = NeteaseClient::new(crate::NeteaseConfig::default()).expect("client");
        let result = login_result(
            &client,
            NeteaseResponse {
                status: StatusCode::OK,
                body: json!({
                    "code": 200,
                    "account": { "id": 123 },
                    "profile": {
                        "userId": 456,
                        "nickname": "TuneWeave",
                        "avatarUrl": "https://example.test/avatar.jpg"
                    }
                }),
                cookies: vec![
                    "MUSIC_U=secret-session; Path=/; HttpOnly".to_owned(),
                    "__csrf=csrf-token; Path=/".to_owned(),
                ],
            },
            "test login",
        )
        .expect("login result");
        assert_eq!(result.account.id.as_deref(), Some("123"));
        assert_eq!(result.account.user_id.as_deref(), Some("456"));
        assert_eq!(result.account.nickname.as_deref(), Some("TuneWeave"));
        assert!(result.session_cookie().contains("MUSIC_U=secret-session"));
    }

    #[test]
    fn rejects_invalid_prehashed_password_before_network_access() {
        let error = validated_password_digest("not-md5").expect_err("invalid digest");
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn reports_wrong_credentials_as_authentication_failure() {
        let client = NeteaseClient::new(crate::NeteaseConfig::default()).expect("client");
        let error = login_result(
            &client,
            NeteaseResponse {
                status: StatusCode::OK,
                body: json!({ "code": 502, "message": "账号或密码错误" }),
                cookies: Vec::new(),
            },
            "test login",
        )
        .expect_err("wrong credentials");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
        assert_eq!(error.details["code"], 502);
    }

    #[test]
    fn login_debug_output_never_contains_the_session_cookie() {
        let result = NeteaseLoginResult {
            account: NeteaseAccountSummary {
                id: Some("123".to_owned()),
                user_id: Some("456".to_owned()),
                nickname: None,
                avatar_url: None,
            },
            session_cookie: "MUSIC_U=must-not-appear".to_owned(),
        };
        let output = format!("{result:?}");
        assert!(!output.contains("must-not-appear"));
        assert!(output.contains("has_session_cookie: true"));
    }

    #[test]
    fn maps_authenticated_and_anonymous_session_statuses() {
        let authenticated = session_status_from_body(&json!({
            "code": 200,
            "account": { "id": 123 },
            "profile": { "userId": 456, "nickname": "TuneWeave" }
        }))
        .expect("authenticated status");
        assert!(authenticated.authenticated);
        assert_eq!(authenticated.account.user_id.as_deref(), Some("456"));

        let anonymous = session_status_from_body(&json!({
            "code": 200,
            "account": null,
            "profile": null
        }))
        .expect("anonymous status");
        assert!(!anonymous.authenticated);
    }

    #[test]
    fn session_refresh_debug_output_redacts_cookie() {
        let refresh = NeteaseSessionRefresh {
            session_cookie: "MUSIC_U=must-not-appear".to_owned(),
        };
        let output = format!("{refresh:?}");
        assert!(!output.contains("must-not-appear"));
        assert!(output.contains("has_session_cookie: true"));
    }
}
