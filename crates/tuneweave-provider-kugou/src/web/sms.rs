//! Official H5 SMS login. An SMS receipt and account choices are never credentials.
mod browser;

use super::*;
use crate::{credential::AccountCredential, device::KugouDevice};
use serde_json::{Value, json};
use std::{collections::BTreeSet, time::Duration};
use tokio::time::Instant as Deadline;
use tuneweave_core::{
    AuthAccountChoice, AuthBrowserChallenge, AuthChallengeAction, AuthChallengeProgress,
    AuthChallengeStatus,
};

pub struct KugouWebSmsRequest {
    pub phone: String,
    pub allow_account_creation: bool,
}
impl std::fmt::Debug for KugouWebSmsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KugouWebSmsRequest")
            .field("allow_account_creation", &self.allow_account_creation)
            .finish_non_exhaustive()
    }
}

/// Ephemeral, single-consumer SMS context. Never serialize or persist this value.
pub struct KugouWebSmsChallenge {
    request: KugouWebSmsRequest,
    device: KugouDeviceIdentity,
    deadline: Deadline,
    attempts: u8,
    // H5 secondary login sends the original username as a string, not a selected UID.
    password_principal: Option<String>,
    state: Stage,
}
#[derive(Clone)]
enum Stage {
    Waiting,
    Selecting(Vec<AuthAccountChoice>),
    Browser {
        verification: AuthBrowserChallenge,
        step: LoginStep,
    },
    Consumed,
}
#[derive(Clone, Default)]
struct LoginStep {
    // The offered list and selected identity survive the browser round trip.
    selection: Option<(Vec<AuthAccountChoice>, String)>,
    create: bool,
}
impl LoginStep {
    fn selected(&self) -> Option<&str> {
        self.selection.as_ref().map(|(_, uid)| uid.as_str())
    }
    fn retry_state(&self) -> Stage {
        self.selection
            .as_ref()
            .map_or(Stage::Waiting, |(choices, _)| {
                Stage::Selecting(choices.clone())
            })
    }
}
enum StepResult {
    Pending(Stage),
    Confirmed(ProviderAuthResult),
}
impl std::fmt::Debug for KugouWebSmsChallenge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("KugouWebSmsChallenge { fields: [redacted] }")
    }
}
impl KugouWebSmsChallenge {
    pub(crate) fn for_password(
        phone: String,
        device: KugouDeviceIdentity,
        deadline: std::time::Instant,
        principal: String,
    ) -> Result<Self> {
        validate_phone(&phone)?;
        Ok(Self {
            request: KugouWebSmsRequest {
                phone,
                allow_account_creation: false,
            },
            device,
            deadline: Deadline::from_std(deadline),
            attempts: 0,
            password_principal: Some(principal),
            state: Stage::Waiting,
        })
    }
    pub(crate) fn remaining_attempts(&self) -> u8 {
        5u8.saturating_sub(self.attempts)
    }

    pub fn status(&self) -> Result<AuthChallengeStatus> {
        if Deadline::now() >= self.deadline {
            return Err(expired());
        }
        match &self.state {
            Stage::Waiting => Ok(AuthChallengeStatus::Waiting),
            Stage::Selecting(accounts) => Ok(AuthChallengeStatus::AccountSelectionRequired {
                accounts: accounts.clone(),
            }),
            Stage::Browser { verification, .. } => {
                Ok(AuthChallengeStatus::BrowserVerificationRequired {
                    verification: verification.clone(),
                })
            }
            Stage::Consumed => Err(expired()),
        }
    }
    pub fn cancel(&mut self) {
        self.state = Stage::Consumed;
    }
}

#[derive(Clone, Copy)]
enum SmsEndpoint {
    Send,
    Verify,
    Choices,
}
impl SmsEndpoint {
    fn target(self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Send => ("gateway.kugou.com", "/v8/send_mobile_code", "web_sms_send"),
            Self::Verify => (
                "login-user.kugou.com",
                "/v2/loginbyverifycode/",
                "web_sms_verify",
            ),
            Self::Choices => (
                "userinfoservice.kugou.com",
                "/v3/check_mobile",
                "web_sms_accounts",
            ),
        }
    }
}
#[derive(Deserialize)]
struct Envelope {
    status: i64,
    error_code: i64,
    data: Option<Box<serde_json::value::RawValue>>,
}
impl Envelope {
    fn success(&self) -> bool {
        self.status == 1 && self.error_code == 0
    }
    fn rejected(&self, code: i64) -> bool {
        self.status == 0 && self.error_code == code
    }
    fn failure(&self) -> TuneWeaveError {
        let code = match (self.status, self.error_code) {
            (0, 20020 | 20021) => ErrorCode::AuthenticationRequired,
            (0, 20028 | 30703 | 30709 | 30701) => ErrorCode::PermissionDenied,
            _ => ErrorCode::UpstreamError,
        };
        error(code, "KuGou SMS login was not accepted")
            .with_details(json!({"upstream_code":self.error_code}))
    }
}

impl KugouClient {
    /// Explicitly sends one SMS. This SDK call does not manage server accounts or
    /// delivery cooldowns; KugouProvider owns those cross-request constraints.
    pub async fn send_web_login_sms(
        &self,
        request: KugouWebSmsRequest,
    ) -> Result<KugouWebSmsChallenge> {
        validate_phone(&request.phone)?;
        let mut receipt = KugouWebSmsChallenge {
            request,
            device: KugouDevice::default().identity().into_web(),
            deadline: Deadline::now() + Duration::from_secs(300),
            attempts: 0,
            password_principal: None,
            state: Stage::Waiting,
        };
        self.resend_web_sms(&mut receipt).await?;
        Ok(receipt)
    }

    // Delivery preserves the original device, deadline, username, and answer budget.
    pub(crate) async fn resend_web_sms(&self, receipt: &mut KugouWebSmsChallenge) -> Result<()> {
        receipt.status()?;
        if receipt.remaining_attempts() == 0 {
            return Err(expired());
        }
        receipt.state = Stage::Consumed;
        let cipher = crypto::WebCipher::random()?;
        let (query, body) = send_parameters(
            &receipt.request.phone,
            &receipt.device,
            crate::account::now_ms()?,
            &cipher,
        )?;
        let reply = tokio::time::timeout_at(
            receipt
                .deadline
                .min(Deadline::now() + Duration::from_secs(45)),
            self.sms_post(SmsEndpoint::Send, query, body, None),
        )
        .await
        .map_err(|_| total_timeout())??;
        if !reply.success() {
            return Err(reply.failure());
        }
        receipt.state = Stage::Waiting;
        receipt.status()?;
        Ok(())
    }

    /// Verifies a code or explicitly selects one of the accounts previously offered
    /// by this receipt. Codes are used for this call only and are never retained.
    pub async fn verify_web_login_sms(
        &self,
        receipt: &mut KugouWebSmsChallenge,
        code: &str,
        selected_user_id: Option<&str>,
    ) -> Result<AuthChallengeProgress> {
        let action = match selected_user_id {
            Some(user_id) => AuthChallengeAction::SelectAccount {
                user_id: user_id.into(),
                code: code.into(),
            },
            None => AuthChallengeAction::SubmitCode { code: code.into() },
        };
        self.advance_web_login_sms(receipt, &action).await
    }

    /// Advances the original SMS receipt, including a human-completed official H5
    /// callback. The code and callback are used for this call only, never retained.
    pub async fn advance_web_login_sms(
        &self,
        receipt: &mut KugouWebSmsChallenge,
        action: &AuthChallengeAction,
    ) -> Result<AuthChallengeProgress> {
        self.advance_web_sms_guarded(receipt, action, &|| Ok(()))
            .await
    }

    pub(crate) async fn advance_web_sms_guarded<F: Fn() -> Result<()> + Sync>(
        &self,
        receipt: &mut KugouWebSmsChallenge,
        action: &AuthChallengeAction,
        check: &F,
    ) -> Result<AuthChallengeProgress> {
        receipt.status()?;
        let (code, step, proof) = match (&receipt.state, action) {
            (Stage::Waiting, AuthChallengeAction::SubmitCode { code }) => {
                (code, LoginStep::default(), None)
            }
            (Stage::Selecting(choices), AuthChallengeAction::SelectAccount { user_id, code })
                if choices.iter().any(|v| v.user_id == *user_id) =>
            {
                (
                    code,
                    LoginStep {
                        selection: Some((choices.clone(), user_id.clone())),
                        create: false,
                    },
                    None,
                )
            }
            (
                Stage::Browser { verification, step },
                AuthChallengeAction::SubmitBrowser {
                    verification_id,
                    code,
                    response,
                },
            ) if verification_id == &verification.verification_id => {
                (code, step.clone(), Some(browser::Proof::parse(response)?))
            }
            _ => return Err(invalid()),
        };
        validate_code(code)?;
        if receipt.attempts >= 5 {
            receipt.cancel();
            return Err(expired());
        }
        check()?;
        let retry_state = step.retry_state();
        receipt.state = Stage::Consumed;
        receipt.attempts += 1;
        let deadline = receipt
            .deadline
            .min(Deadline::now() + Duration::from_secs(45));
        let result = tokio::time::timeout_at(
            deadline,
            self.verify_sms_steps(receipt, code, step, proof.as_ref(), check),
        )
        .await;
        // Cancellation, ambiguous failures and source changes consume the receipt.
        // A wrong SMS code discards the browser challenge; no proof is retained/reused.
        check().map_err(TuneWeaveError::with_consumed_auth_challenge)?;
        let result = result.map_err(|_| total_timeout()).and_then(|v| v);
        match result {
            Ok(StepResult::Pending(stage)) => {
                if receipt.attempts >= 5 || Deadline::now() >= receipt.deadline {
                    return Err(expired());
                }
                receipt.state = stage;
                Ok(AuthChallengeProgress::Pending(receipt.status()?))
            }
            Ok(StepResult::Confirmed(result)) => Ok(AuthChallengeProgress::Confirmed(result)),
            Err(e)
                if e.code == ErrorCode::AuthenticationRequired
                    && e.details["upstream_code"] == 20021
                    && receipt.attempts < 5
                    && Deadline::now() < receipt.deadline =>
            {
                receipt.state = retry_state;
                Err(e)
            }
            Err(e) => Err(e.with_consumed_auth_challenge()),
        }
    }

    async fn verify_sms_steps<F: Fn() -> Result<()> + Sync>(
        &self,
        receipt: &KugouWebSmsChallenge,
        code: &str,
        mut step: LoginStep,
        proof: Option<&browser::Proof>,
        check: &F,
    ) -> Result<StepResult> {
        let (query, body) = verify_parameters(
            &receipt.device,
            &receipt.request.phone,
            code,
            step.selected(),
            receipt.password_principal.as_deref(),
            step.create,
            crate::account::now_ms()?,
        )?;
        let result = self.sms_post(SmsEndpoint::Verify, query, body, proof).await;
        check()?;
        let mut reply = result?;
        if reply.rejected(30703)
            && receipt.request.allow_account_creation
            && step.selected().is_none()
            && !step.create
        {
            step.create = true;
            let (query, body) = verify_parameters(
                &receipt.device,
                &receipt.request.phone,
                code,
                None,
                None,
                true,
                crate::account::now_ms()?,
            )?;
            // Human proof is submitted only once. A subsequent challenge has its own callback.
            let result = self.sms_post(SmsEndpoint::Verify, query, body, None).await;
            check()?;
            reply = result?;
        }
        if reply.rejected(20028) {
            let verification = browser::challenge(&reply, receipt)?;
            if let Some(proof) = proof {
                proof.reject_reflection(&encode(&verification)?)?;
            }
            return Ok(StepResult::Pending(Stage::Browser { verification, step }));
        }
        if reply.rejected(34175) && step.selected().is_none() {
            let (query, body) = choices_parameters(
                &receipt.device,
                &receipt.request.phone,
                code,
                crate::account::now_ms()?,
            )?;
            let result = self.sms_post(SmsEndpoint::Choices, query, body, None).await;
            check()?;
            let accounts = parse_choices(result?)?;
            if let Some(proof) = proof {
                proof.reject_reflection(&encode(&accounts)?)?;
            }
            return Ok(StepResult::Pending(Stage::Selecting(accounts)));
        }
        if !reply.success() {
            return Err(reply.failure());
        }
        let candidate = parse_session(&reply.data.ok_or_else(malformed)?, &receipt.device)?;
        if step.selected().is_some_and(|id| id != candidate.user_id) {
            return Err(conflict());
        }
        let result = self.refresh_web_session(&candidate).await;
        check()?;
        let session = result?;
        let profile = session.profile()?;
        if let Some(proof) = proof {
            proof.reject_reflection(&encode(&profile)?)?;
        }
        let credential = AccountCredential::verified_web(session)?.caller()?;
        Ok(StepResult::Confirmed(ProviderAuthResult {
            profile,
            credential: Some(credential),
        }))
    }

    async fn sms_post(
        &self,
        endpoint: SmsEndpoint,
        mut query: BTreeMap<&str, String>,
        body: Vec<u8>,
        proof: Option<&browser::Proof>,
    ) -> Result<Envelope> {
        let (host, path, operation) = endpoint.target();
        query.insert("signature", web_signature(&query, &body));
        let url = format!("https://{host}{path}");
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|o| o.join(path).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let mut request = self
                .http
                .post(url)
                .query(&query)
                .header(CONTENT_TYPE, "text/plain;charset=UTF-8")
                .header("accept", "*/*")
                .header(ORIGIN, "https://m.kugou.com")
                .header(
                    REFERER,
                    "https://m.kugou.com/loginReg.php?act=login&appid=1014&logintype=account",
                )
                .body(body);
            if matches!(endpoint, SmsEndpoint::Send) {
                request = request.header("x-router", "loginservice.kugou.com");
            }
            if let Some(value) = proof.and_then(|p| p.header.as_ref()) {
                if !matches!(endpoint, SmsEndpoint::Verify) {
                    return Err(invalid());
                }
                request = request.header("VerifyData", value.clone());
            }
            let response = request
                .send()
                .await
                .map_err(crate::account::network_error)?;
            status = Some(response.status());
            let bytes = crate::account::read_response_with_types(
                response,
                1024 * 1024,
                &["application/json", "text/plain", "text/html"],
            )
            .await?;
            if let Some(proof) = proof {
                proof.reject_reflection(&bytes)?;
            }
            serde_json::from_slice::<Envelope>(&bytes).map_err(|_| malformed())
        }
        .await;
        self.log_upstream_request(operation, host, path, status, started, 0, false, &result);
        result
    }
}

fn query(
    device: &KugouDeviceIdentity,
    time: u64,
    version: &str,
    uuid: bool,
) -> BTreeMap<&'static str, String> {
    let mut p = BTreeMap::from([
        ("appid", "1014".into()),
        ("srcappid", "2919".into()),
        ("clientver", version.into()),
        ("clienttime", time.to_string()),
        ("mid", device.mid.clone()),
        ("dfid", device.dfid().into()),
    ]);
    if uuid {
        p.insert("uuid", device.mid.clone());
    }
    p
}
fn send_parameters(
    phone: &str,
    device: &KugouDeviceIdentity,
    now: u64,
    cipher: &crypto::WebCipher,
) -> Result<(BTreeMap<&'static str, String>, Vec<u8>)> {
    #[derive(Serialize)]
    struct Secret<'a> {
        mobile: &'a str,
    }
    #[derive(Serialize)]
    struct Body {
        plat: u8,
        clienttime_ms: u64,
        businessid: u8,
        pk: String,
        params: String,
        mobile: String,
    }
    let body = Body {
        plat: 4,
        clienttime_ms: now,
        businessid: 5,
        pk: cipher.pk(now)?,
        params: cipher.encrypt(&Secret { mobile: phone })?,
        mobile: format!("{}********{}", &phone[..2], &phone[10..]),
    };
    Ok((query(device, now, "1000", true), encode(&body)?))
}
fn verify_parameters(
    device: &KugouDeviceIdentity,
    phone: &str,
    code: &str,
    selected: Option<&str>,
    initial_principal: Option<&str>,
    create: bool,
    now: u64,
) -> Result<(BTreeMap<&'static str, String>, Vec<u8>)> {
    #[derive(Serialize)]
    struct Body<'a> {
        plat: u8,
        mobile: &'a str,
        code: &'a str,
        expire_day: u8,
        support_multi: u8,
        userid: Value,
        force_login: u8,
    }
    // LoginByVerifycodeV2 omits uuid; the official signing helper supplies
    // Date.now(), while preserving this endpoint's seconds-based clienttime.
    let mut parameters = query(device, now / 1000, "10", false);
    parameters.insert("uuid", now.to_string());
    // H5 loginById receives a numeric UID. Secondary password login starts with
    // its original username string; ordinary SMS starts with an empty string.
    let userid = match selected {
        None => Value::String(initial_principal.unwrap_or_default().to_owned()),
        Some(uid) => {
            let number = uid.parse::<u64>().map_err(|_| invalid())?;
            if number == 0 || number.to_string() != uid {
                return Err(invalid());
            }
            Value::from(number)
        }
    };
    Ok((
        parameters,
        encode(&Body {
            plat: 4,
            mobile: phone,
            code,
            expire_day: 1,
            support_multi: 1,
            userid,
            force_login: u8::from(create),
        })?,
    ))
}
fn choices_parameters(
    device: &KugouDeviceIdentity,
    phone: &str,
    code: &str,
    now: u64,
) -> Result<(BTreeMap<&'static str, String>, Vec<u8>)> {
    #[derive(Serialize)]
    struct Fields {
        duration: u8,
        p_grade: u8,
    }
    #[derive(Serialize)]
    struct Body<'a> {
        plat: u8,
        mobile: &'a str,
        code: &'a str,
        businessid: u8,
        query: Fields,
    }
    Ok((
        query(device, now / 1000, "1000", true),
        encode(&Body {
            plat: 4,
            mobile: phone,
            code,
            businessid: 5,
            query: Fields {
                duration: 1,
                p_grade: 1,
            },
        })?,
    ))
}
fn encode(value: &impl Serialize) -> Result<Vec<u8>> {
    serde_json::to_vec(value).map_err(|_| invalid())
}
fn parse_session(
    data: &serde_json::value::RawValue,
    device: &KugouDeviceIdentity,
) -> Result<WebSession> {
    #[derive(Deserialize)]
    struct Cookie {
        name: String,
        domain: String,
        path: String,
        value: String,
    }
    let data: Cookie = serde_json::from_str(data.get()).map_err(|_| malformed())?;
    let cookie = WebCookie::from_sms_fields(
        &data.name,
        &data.domain,
        &data.path,
        &data.value,
        crate::account::now_ms()? / 1000,
    )?;
    let user_id = cookie.identity()?.user_id;
    Ok(WebSession {
        device: device.clone(),
        user_id,
        cookie,
    })
}
fn parse_choices(reply: Envelope) -> Result<Vec<AuthAccountChoice>> {
    if !reply.success() {
        return Err(reply.failure());
    }
    #[derive(Deserialize)]
    struct Choices {
        info_list: Vec<Choice>,
    }
    #[derive(Deserialize)]
    struct Choice {
        userid: Value,
        nickname: Option<String>,
        pic: Option<String>,
    }
    let rows: Choices =
        serde_json::from_str(reply.data.ok_or_else(malformed)?.get()).map_err(|_| malformed())?;
    if rows.info_list.is_empty() || rows.info_list.len() > 128 {
        return Err(malformed());
    }
    let mut seen = BTreeSet::new();
    let mut choices = Vec::with_capacity(rows.info_list.len());
    for row in rows.info_list {
        let id = match row.userid {
            Value::String(s) => s,
            Value::Number(n) if n.as_u64().is_some() => n.to_string(),
            _ => return Err(malformed()),
        };
        if !valid_uid(&id) || !seen.insert(id.clone()) {
            return Err(malformed());
        }
        let nickname = row
            .nickname
            .map(|s| {
                if s.len() > 512 || s.chars().any(char::is_control) {
                    Err(malformed())
                } else {
                    Ok(s)
                }
            })
            .transpose()?;
        choices.push(AuthAccountChoice {
            user_id: id,
            nickname,
            avatar_url: row
                .pic
                .as_deref()
                .and_then(crate::client::normalize_image_url),
        });
    }
    Ok(choices)
}
pub(crate) fn validate_phone(phone: &str) -> Result<()> {
    if phone.len() != 11
        || !phone.starts_with('1')
        || phone == "10000000000"
        || !phone.bytes().all(|c| c.is_ascii_digit())
    {
        return Err(invalid());
    }
    Ok(())
}
pub(crate) fn validate_code(code: &str) -> Result<()> {
    if !(4..=8).contains(&code.len()) || !code.bytes().all(|c| c.is_ascii_digit()) {
        return Err(invalid());
    }
    Ok(())
}
fn expired() -> TuneWeaveError {
    error(
        ErrorCode::AuthenticationRequired,
        "KuGou SMS transaction is expired or consumed",
    )
    .with_consumed_auth_challenge()
}
fn total_timeout() -> TuneWeaveError {
    error(
        ErrorCode::UpstreamTimeout,
        "KuGou SMS exceeded its total time budget",
    )
}

#[cfg(test)]
mod tests;
