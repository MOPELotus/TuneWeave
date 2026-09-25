use super::*;

pub(crate) enum PasswordOutcome {
    Token(String),
    Image { chinese: Option<bool> },
    Secondary(SecondaryIdentity),
    Voice(SecondaryIdentity),
}
pub(crate) struct SecondaryIdentity {
    encrypted: String,
    pub(crate) masked: String,
}
impl SecondaryIdentity {
    fn parse(value: &serde_json::Value) -> Result<Self> {
        let invalid = || {
            error(
                ErrorCode::UpstreamError,
                "Invalid Migu secondary verification identity",
            )
        };
        let encrypted = value
            .get("msisdnRSA")
            .and_then(serde_json::Value::as_str)
            .filter(|v| !v.is_empty() && v.len() <= 4096 && v.bytes().all(|b| b.is_ascii_graphic()))
            .ok_or_else(invalid)?;
        // Only show a masked mainland destination, never expose an unexpected full number.
        let masked = value
            .get("msisdnHide")
            .and_then(serde_json::Value::as_str)
            .filter(|v| {
                v.len() <= 64
                    && v.bytes()
                        .all(|b| b.is_ascii_digit() || b"*+()- ".contains(&b))
                    && v.bytes().filter(|b| *b == b'*').count() >= 4
                    && v.bytes().filter(u8::is_ascii_digit).count() <= 7
            })
            .ok_or_else(invalid)?;
        Ok(Self {
            encrypted: encrypted.into(),
            masked: masked.into(),
        })
    }
}

fn token(body: &serde_json::Value) -> Result<String> {
    let invalid = || {
        error(
            ErrorCode::UpstreamError,
            "Migu authentication omitted a valid exchange token",
        )
    };
    let value = body
        .get("result")
        .and_then(|v| v.get("token"))
        .and_then(serde_json::Value::as_str)
        .filter(|v| v.len() <= 4096)
        .ok_or_else(invalid)?;
    validate_token(value).map_err(|_| invalid())?;
    Ok(value.into())
}

impl MiguClient {
    pub(crate) async fn password_outcome(
        &self,
        principal: &str,
        password: &str,
        captcha: &str,
        cookies: &mut PassportCookies,
        check: &(impl Fn() -> Result<()> + Sync),
    ) -> Result<PasswordOutcome> {
        check()?;
        let key = self.passport_key(cookies).await;
        check()?;
        let key = key?;
        let result = self
            .passport_request(
                Operation::PasswordChallenge,
                &[
                    ("loginID", key.encrypt(principal)?),
                    ("enpassword", key.encrypt(password)?),
                    ("sourceID", "220029".into()),
                    ("appType", "0".into()),
                    ("relayState", String::new()),
                    ("captcha", captcha.into()),
                    ("imgcodeType", "1".into()),
                    ("isAsync", "true".into()),
                    ("fingerPrint", String::new()),
                    ("fingerPrintDetail", String::new()),
                ],
                CookieContext::Session(cookies),
                None,
                |body, _| {
                    let status = body
                        .get("status")
                        .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()));
                    match status {
                        Some(2000) => token(&body).map(PasswordOutcome::Token),
                        Some(4002) => Ok(PasswordOutcome::Image { chinese: None }),
                        Some(4044) => Ok(PasswordOutcome::Image {
                            chinese: Some(false),
                        }),
                        Some(4045) => Ok(PasswordOutcome::Image {
                            chinese: Some(true),
                        }),
                        Some(6103) => SecondaryIdentity::parse(
                            body.get("result").unwrap_or(&serde_json::Value::Null),
                        )
                        .map(PasswordOutcome::Secondary),
                        Some(6118) => SecondaryIdentity::parse(
                            body.get("result").unwrap_or(&serde_json::Value::Null),
                        )
                        .map(PasswordOutcome::Voice),
                        _ => Err(error(
                            ErrorCode::UpstreamError,
                            "Unknown Migu password verification status",
                        )),
                    }
                },
            )
            .await;
        check()?;
        result
    }

    pub(crate) async fn send_secondary_sms(
        &self,
        identity: &SecondaryIdentity,
        cookies: &mut PassportCookies,
        check: &(impl Fn() -> Result<()> + Sync),
    ) -> Result<()> {
        check()?;
        // msisdn is already RSA-encrypted by passport. The new key belongs to the
        // optional browser fingerprint fields, whose absent branch stays empty.
        let key = self.passport_key(cookies).await;
        check()?;
        key?;
        let result = self
            .passport_request(
                Operation::SecondarySend,
                &[
                    ("msisdn", identity.encrypted.clone()),
                    ("sourceID", "220029".into()),
                    ("isAsync", "true".into()),
                    ("fingerPrint", String::new()),
                    ("fingerPrintDetail", String::new()),
                ],
                CookieContext::Session(cookies),
                None,
                |_, _| Ok(()),
            )
            .await;
        check()?;
        result
    }

    pub(crate) async fn verify_secondary_sms(
        &self,
        identity: &SecondaryIdentity,
        code: &str,
        cookies: &mut PassportCookies,
        check: &(impl Fn() -> Result<()> + Sync),
    ) -> Result<String> {
        check()?;
        let key = self.passport_key(cookies).await;
        check()?;
        let key = key?;
        let result = self
            .passport_request(
                Operation::SecondaryVerify,
                &[
                    ("msisdn", identity.encrypted.clone()),
                    ("secondPassword", key.encrypt(code)?),
                    ("sourceID", "220029".into()),
                    ("appType", "0".into()),
                    ("relayState", String::new()),
                    ("isAsync", "true".into()),
                ],
                CookieContext::Session(cookies),
                None,
                |body, _| token(&body),
            )
            .await;
        check()?;
        result
    }

    pub(crate) async fn send_secondary_voice(
        &self,
        identity: &SecondaryIdentity,
        cookies: &mut PassportCookies,
        check: &(impl Fn() -> Result<()> + Sync),
    ) -> Result<()> {
        check()?;
        // The voice button passes the server-encrypted phone unchanged. Unlike
        // secondary SMS, its handler never requests a key or adds fingerprints.
        let result = self
            .passport_request(
                Operation::VoiceSend,
                &[
                    ("isAsync", "true".into()),
                    ("msisdn", identity.encrypted.clone()),
                    ("sourceID", "220029".into()),
                ],
                CookieContext::Session(cookies),
                None,
                |_, _| Ok(()),
            )
            .await;
        check()?;
        result
    }

    pub(crate) async fn verify_secondary_voice(
        &self,
        identity: &SecondaryIdentity,
        code: &str,
        cookies: &mut PassportCookies,
        check: &(impl Fn() -> Result<()> + Sync),
    ) -> Result<String> {
        check()?;
        // The official voice form serializes voiceCode directly over HTTPS.
        // Its hidden captchaId has no name and is not part of the wire form.
        let result = self
            .passport_request(
                Operation::VoiceVerify,
                &[
                    ("sourceID", "220029".into()),
                    ("appType", "0".into()),
                    ("relayState", String::new()),
                    ("msisdn", identity.encrypted.clone()),
                    ("voiceCode", code.into()),
                    ("isAsync", "true".into()),
                ],
                CookieContext::Session(cookies),
                None,
                |body, _| token(&body),
            )
            .await;
        check()?;
        result
    }
}
