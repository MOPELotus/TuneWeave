use super::*;

impl MiguClient {
    pub(super) async fn passport_key(
        &self,
        cookies: &mut PassportCookies,
    ) -> Result<LoginPublicKey> {
        self.passport_request(
            Operation::Key,
            &[],
            CookieContext::Session(cookies),
            None,
            |body, _| {
                let value = body
                    .get("result")
                    .ok_or_else(|| error(ErrorCode::UpstreamError, "Migu public key is missing"))?;
                LoginPublicKey::parse(
                    value
                        .get("modulus")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| {
                            error(ErrorCode::UpstreamError, "Migu public modulus is missing")
                        })?,
                    value
                        .get("publicExponent")
                        .and_then(serde_json::Value::as_str)
                        .ok_or_else(|| {
                            error(ErrorCode::UpstreamError, "Migu public exponent is missing")
                        })?,
                )
            },
        )
        .await
    }

    pub(crate) async fn send_sms(
        &self,
        principal: &str,
        captcha: &str,
        cookies: &mut PassportCookies,
        check: &(impl Fn() -> Result<()> + Sync),
    ) -> Result<()> {
        check()?;
        let key = self.passport_key(cookies).await;
        check()?;
        let msisdn = key?.encrypt(principal)?;
        // The official send handler obtains a second key for fingerprint encryption.
        // Its absent-fingerprint branch still sends empty fingerprint fields.
        let fingerprint_key = self.passport_key(cookies).await;
        check()?;
        fingerprint_key?;
        let result = self
            .passport_request(
                Operation::SmsSend,
                &[
                    ("msisdn", msisdn),
                    ("sourceID", "220029".into()),
                    ("isAsync", "true".into()),
                    ("captcha", captcha.to_owned()),
                    // The global page initializer injects this hidden field for SMS forms.
                    ("imgcodeType", "2".into()),
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

    pub(crate) async fn authenticate_sms(
        &self,
        principal: &str,
        code: &str,
        captcha: &str,
        cookies: &mut PassportCookies,
        check: &(impl Fn() -> Result<()> + Sync),
    ) -> Result<String> {
        check()?;
        let key = self.passport_key(cookies).await;
        check()?;
        let key = key?;
        let result = self
            .passport_request(
                Operation::SmsVerify,
                &[
                    ("msisdn", key.encrypt(principal)?),
                    ("dynamicPassword", key.encrypt(code)?),
                    ("imgcodeType", "2".into()),
                    ("sourceID", "220029".into()),
                    ("appType", "0".into()),
                    ("relayState", String::new()),
                    ("securityCode", String::new()),
                    ("captcha", captcha.to_owned()),
                    ("isAsync", "true".into()),
                    ("fingerPrint", String::new()),
                    ("fingerPrintDetail", String::new()),
                ],
                CookieContext::Session(cookies),
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
                                "Migu SMS authentication omitted its exchange token",
                            )
                        })?;
                    validate_token(token).map_err(|_| {
                        error(
                            ErrorCode::UpstreamError,
                            "Migu SMS exchange token is invalid",
                        )
                    })?;
                    Ok(token.to_owned())
                },
            )
            .await;
        check()?;
        result
    }
}

#[cfg(test)]
mod tests;
