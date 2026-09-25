use super::*;
use crate::{
    credential::{error, validate_token, validate_uid},
    passport::{LoginPublicKey, cookies::PassportCookies},
};
use reqwest::header::{CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue, ORIGIN, REFERER, SET_COOKIE};
use tuneweave_core::PasswordLoginRequest;

enum Operation {
    Key,
    Password,
    PasswordChallenge,
    SecondarySend,
    SecondaryVerify,
    VoiceSend,
    VoiceVerify,
    SmsSend,
    SmsVerify,
    ImageFetch,
    ImageCheck,
    Exchange,
}
impl Operation {
    fn path(&self) -> &'static str {
        match self {
            Self::Key => "/password/publickey",
            Self::Password | Self::PasswordChallenge => "/authn",
            Self::SecondarySend => "/login/second/password",
            Self::SecondaryVerify => "/authn/second/dynamicpassword",
            Self::VoiceSend => "/login/send/voice",
            Self::VoiceVerify => "/authn/voice/validate",
            Self::SmsSend => "/login/dynamicpassword",
            Self::SmsVerify => "/authn/dynamicpassword",
            Self::ImageFetch => "/captcha/graph/risk",
            Self::ImageCheck => "/captcha/graph/check",
            Self::Exchange => "/user/h5/token-validate/v3.0",
        }
    }
    fn host(&self) -> &'static str {
        if matches!(self, Self::Exchange) {
            "c.musicapp.migu.cn"
        } else {
            "passport.migu.cn"
        }
    }
    fn name(&self) -> &'static str {
        match self {
            Self::Key => "login_public_key",
            Self::Password | Self::PasswordChallenge => "password_authentication",
            Self::SecondarySend => "secondary_sms_delivery",
            Self::SecondaryVerify => "secondary_sms_verification",
            Self::VoiceSend => "voice_delivery",
            Self::VoiceVerify => "voice_verification",
            Self::SmsSend => "sms_delivery",
            Self::SmsVerify => "sms_verification",
            Self::ImageFetch => "image_challenge",
            Self::ImageCheck => "image_verification",
            Self::Exchange => "passport_token_exchange",
        }
    }
}

enum CookieContext<'a> {
    Once(Option<HeaderValue>),
    Session(&'a mut PassportCookies),
}

impl MiguClient {
    async fn passport_request<T>(
        &self,
        operation: Operation,
        fields: &[(&str, String)],
        mut cookies: CookieContext<'_>,
        device_id: Option<HeaderValue>,
        parse: impl FnOnce(serde_json::Value, &HeaderMap) -> Result<T>,
    ) -> Result<T> {
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let url = Url::parse(&format!("https://{}{}", operation.host(), operation.path()))
                .map_err(|_| {
                    error(
                        ErrorCode::InternalError,
                        "Invalid Migu authentication endpoint",
                    )
                })?;
            #[cfg(test)]
            let url = self
                .catalog_test_origin
                .as_ref()
                .map_or(Ok(url), |origin| origin.join(operation.path()))
                .map_err(|_| {
                    error(
                        ErrorCode::InternalError,
                        "Invalid Migu authentication test endpoint",
                    )
                })?;
            let mut request = if matches!(
                operation,
                Operation::Exchange
                    | Operation::SmsSend
                    | Operation::SecondarySend
                    | Operation::VoiceSend
            ) {
                self.http.get(url).query(fields)
            } else if matches!(operation, Operation::ImageFetch) {
                self.http.post(url).query(fields)
            } else {
                self.http.post(url).form(fields)
            };
            request = request.header(ACCEPT, "application/json");
            if matches!(operation, Operation::Exchange) {
                request = request
                    .header(ORIGIN, "https://music.migu.cn")
                    .header(REFERER, "https://music.migu.cn/");
            } else {
                request = request.header(ORIGIN, "https://passport.migu.cn").header(
                    REFERER,
                    "https://passport.migu.cn/login?sourceid=220029&appType=0",
                );
            }
            let cookie = match &cookies {
                CookieContext::Once(value) => value.clone(),
                CookieContext::Session(jar) => jar.header(operation.path())?,
            };
            if let Some(cookie) = cookie {
                request = request.header(COOKIE, cookie);
            }
            if let Some(device) = device_id {
                request = request.header("deviceId", device);
            }
            let response = request
                .send()
                .await
                .map_err(|e| transport_error(e.is_timeout()))?;
            let http_status = response.status();
            status = Some(http_status);
            if !http_status.is_success() {
                return Err(error(
                    match http_status {
                        StatusCode::TOO_MANY_REQUESTS => ErrorCode::RateLimited,
                        StatusCode::UNAUTHORIZED => ErrorCode::AuthenticationRequired,
                        StatusCode::FORBIDDEN => ErrorCode::PermissionDenied,
                        _ => ErrorCode::UpstreamError,
                    },
                    "Migu authentication HTTP request failed",
                ));
            }
            if let CookieContext::Session(jar) = &mut cookies {
                jar.update(response.headers(), operation.path())?;
            }
            if !response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
            {
                return Err(error(
                    ErrorCode::UpstreamError,
                    "Migu authentication response is not JSON",
                ));
            }
            let headers = response.headers().clone();
            let body = super::account::read_session_body(response).await?;
            let value: serde_json::Value = serde_json::from_slice(&body).map_err(|_| {
                error(
                    ErrorCode::UpstreamError,
                    "Migu authentication response is invalid",
                )
            })?;
            if matches!(operation, Operation::Exchange) {
                if value.get("code").and_then(serde_json::Value::as_str) != Some("000000") {
                    return Err(error(
                        ErrorCode::UpstreamError,
                        "Migu passport token exchange was not confirmed",
                    ));
                }
            } else {
                let code = value
                    .get("status")
                    .and_then(|v| {
                        v.as_u64().or_else(|| {
                            v.as_str()
                                .filter(|s| s.len() == 4 && s.bytes().all(|b| b.is_ascii_digit()))
                                .and_then(|s| s.parse().ok())
                        })
                    })
                    .filter(|v| (1000..=9999).contains(v))
                    .ok_or_else(|| {
                        error(
                            ErrorCode::UpstreamError,
                            "Migu authentication status is invalid",
                        )
                    })?;
                if matches!(operation, Operation::PasswordChallenge)
                    && matches!(code, 4002 | 4044 | 4045 | 6103 | 6118)
                {
                    return parse(value, &headers);
                }
                if code != 2000 {
                    let verification =
                        matches!(
                            operation,
                            Operation::Password
                                | Operation::PasswordChallenge
                                | Operation::SecondarySend
                                | Operation::SecondaryVerify
                                | Operation::VoiceSend
                                | Operation::VoiceVerify
                                | Operation::SmsSend
                                | Operation::SmsVerify
                                | Operation::ImageCheck
                        ) && matches!(code, 4002 | 4044 | 4045 | 6103 | 6119 | 6118 | 6123 | 4049);
                    let mut e = error(
                        if verification
                            || (matches!(
                                operation,
                                Operation::SmsVerify | Operation::SecondaryVerify
                            ) && code == 4005)
                        {
                            ErrorCode::AuthenticationRequired
                        } else if code == 4016 {
                            ErrorCode::RateLimited
                        } else {
                            ErrorCode::UpstreamError
                        },
                        if verification {
                            "Migu login requires additional verification"
                        } else {
                            "Migu authentication request was not confirmed"
                        },
                    )
                    .with_details(json!({"platform_code":code.to_string()}));
                    if verification {
                        e.details["verification_required"] = json!(true);
                    }
                    return Err(e);
                }
            }
            parse(value, &headers)
        }
        .await;
        self.log_upstream_request(
            operation.name(),
            operation.host(),
            operation.path(),
            status,
            started,
            &result,
        );
        result
    }

    /// Returns identity-bound music credentials only after the passport token exchange.
    /// Final profile verification and persistence remain the provider's responsibility.
    pub(crate) async fn password_music_session(
        &self,
        request: &PasswordLoginRequest,
    ) -> Result<(String, String)> {
        let (key, cookies) = self
            .passport_request(
                Operation::Key,
                &[],
                CookieContext::Once(None),
                None,
                |body, headers| {
                    let result = body.get("result").ok_or_else(|| {
                        error(
                            ErrorCode::UpstreamError,
                            "Migu public key response omitted its key",
                        )
                    })?;
                    let key = LoginPublicKey::parse(
                        result
                            .get("modulus")
                            .and_then(serde_json::Value::as_str)
                            .ok_or_else(|| {
                                error(
                                    ErrorCode::UpstreamError,
                                    "Migu public key response omitted its modulus",
                                )
                            })?,
                        result
                            .get("publicExponent")
                            .and_then(serde_json::Value::as_str)
                            .ok_or_else(|| {
                                error(
                                    ErrorCode::UpstreamError,
                                    "Migu public key response omitted its exponent",
                                )
                            })?,
                    )?;
                    Ok((key, passport_cookie(headers)?))
                },
            )
            .await?;
        let fields = [
            ("loginID", key.encrypt(&request.principal)?),
            ("enpassword", key.encrypt(&request.password)?),
            ("sourceID", "220029".into()),
            ("appType", "0".into()),
            ("relayState", String::new()),
            ("captcha", String::new()),
            ("imgcodeType", "1".into()),
            ("isAsync", "true".into()),
            ("fingerPrint", String::new()),
            ("fingerPrintDetail", String::new()),
        ];
        // The official page uses empty fields when browser fingerprint collection is absent.
        // Do not fabricate browser/plugin details for a server process.
        let token = self
            .passport_request(
                Operation::Password,
                &fields,
                CookieContext::Once(cookies),
                None,
                |body, _| {
                    let token = body
                        .get("result")
                        .and_then(|v| v.get("token"))
                        .and_then(serde_json::Value::as_str)
                        .filter(|v| v.len() <= 4096)
                        .ok_or_else(|| {
                            error(
                                ErrorCode::UpstreamError,
                                "Migu authentication omitted its exchange token",
                            )
                        })?;
                    validate_token(token).map_err(|_| {
                        error(
                            ErrorCode::UpstreamError,
                            "Migu authentication returned an invalid exchange token",
                        )
                    })?;
                    Ok(token.to_owned())
                },
            )
            .await?;
        self.exchange_passport_token(token).await
    }

    pub(crate) async fn exchange_passport_token(&self, token: String) -> Result<(String, String)> {
        let device = sensitive(&self.music_device.identity()?)?;
        self.passport_request(
            Operation::Exchange,
            &[
                ("token", token),
                ("type", "2".into()),
                ("sourceId", "220029".into()),
                ("activityId", "MUSIC-WWW".into()),
            ],
            CookieContext::Once(None),
            Some(device),
            |body, headers| {
                let uid = body
                    .get("data")
                    .and_then(|v| v.get("userId"))
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        error(
                            ErrorCode::UpstreamError,
                            "Migu token exchange omitted its user identity",
                        )
                    })?;
                validate_uid(uid).map_err(|_| {
                    error(
                        ErrorCode::UpstreamError,
                        "Migu token exchange returned an invalid identity",
                    )
                })?;
                let token = super::account::exchanged_token(headers)?;
                Ok((uid.into(), token))
            },
        )
        .await
    }
}

fn transport_error(timeout: bool) -> TuneWeaveError {
    error(
        if timeout {
            ErrorCode::UpstreamTimeout
        } else {
            ErrorCode::UpstreamError
        },
        "Migu authentication transport failed",
    )
}
fn sensitive(value: &str) -> Result<HeaderValue> {
    let mut header = HeaderValue::from_str(value).map_err(|_| {
        error(
            ErrorCode::UpstreamError,
            "Migu authentication header is invalid",
        )
    })?;
    header.set_sensitive(true);
    Ok(header)
}

fn passport_cookie(headers: &HeaderMap) -> Result<Option<HeaderValue>> {
    let mut values = BTreeMap::new();
    let mut names = std::collections::BTreeSet::new();
    for header in headers.get_all(SET_COOKIE) {
        let raw = header
            .to_str()
            .map_err(|_| error(ErrorCode::UpstreamError, "Migu passport cookie is invalid"))?;
        let mut parts = raw.split(';');
        let Some((name, value)) = parts.next().and_then(|v| v.trim().split_once('=')) else {
            continue;
        };
        if !matches!(
            name,
            "mgnd_session_id" | "mgnd_session_create" | "mgnd_session_last_access"
        ) {
            continue;
        }
        let mut applies = true;
        let mut attributes = std::collections::BTreeSet::new();
        let mut max_age = None;
        let mut expires = None;
        for part in parts {
            let (name, value) = part.trim().split_once('=').unwrap_or((part.trim(), ""));
            let name = name.to_ascii_lowercase();
            if matches!(name.as_str(), "domain" | "path" | "max-age" | "expires")
                && !attributes.insert(name.clone())
            {
                return Err(error(
                    ErrorCode::UpstreamError,
                    "Migu passport cookie has ambiguous attributes",
                ));
            }
            match name.as_str() {
                "domain" => {
                    applies &= matches!(
                        value
                            .strip_prefix('.')
                            .unwrap_or(value)
                            .to_ascii_lowercase()
                            .as_str(),
                        "migu.cn" | "passport.migu.cn"
                    )
                }
                "path" => applies &= matches!(value, "/" | "/authn"),
                "max-age" => {
                    max_age = Some(value.parse::<i64>().map_err(|_| {
                        error(
                            ErrorCode::UpstreamError,
                            "Migu passport cookie lifetime is invalid",
                        )
                    })?)
                }
                "expires" => {
                    expires = Some(httpdate::parse_http_date(value).map_err(|_| {
                        error(
                            ErrorCode::UpstreamError,
                            "Migu passport cookie expiry is invalid",
                        )
                    })?)
                }
                _ => {}
            }
        }
        // A missing Path defaults to /password, which is not in scope for /authn.
        applies &= attributes.contains("path");
        if !applies {
            continue;
        }
        if !names.insert(name) {
            return Err(error(
                ErrorCode::UpstreamError,
                "Migu passport returned duplicate session cookies",
            ));
        }
        if max_age.is_some_and(|v| v <= 0)
            || (max_age.is_none() && expires.is_some_and(|v| v <= std::time::SystemTime::now()))
        {
            continue;
        }
        if value.len() > 4096 {
            return Err(error(
                ErrorCode::UpstreamError,
                "Migu passport cookie exceeds the limit",
            ));
        }
        validate_token(value)
            .map_err(|_| error(ErrorCode::UpstreamError, "Migu passport cookie is invalid"))?;
        values.insert(name, value);
    }
    if values.is_empty() {
        Ok(None)
    } else {
        sensitive(
            &values
                .iter()
                .map(|(k, v)| format!("{k}={v}"))
                .collect::<Vec<_>>()
                .join("; "),
        )
        .map(Some)
    }
}

#[cfg(test)]
pub(crate) mod tests;

mod sms;

pub(crate) mod image;

pub(crate) mod password;
