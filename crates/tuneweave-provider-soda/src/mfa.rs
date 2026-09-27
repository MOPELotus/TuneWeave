use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

use serde_json::Value;
use tuneweave_core::{
    ErrorCode, Platform, QrVerification, QrVerificationAction, QrVerificationMethod, Result,
    TuneWeaveError, UpSmsInstructions,
};

use crate::client::soda_upstream_error;

const VERIFY_KEYS: [&str; 7] = [
    "passport_mfa_retry_tag",
    "std_verify_flow_id",
    "std_verify_scene",
    "std_verify_template",
    "std_verify_token",
    "std_verify_type",
    "std_verify_way",
];
const NEW_AUTHN_SDK_VERSION: &str = "1.0.0.428-web";
const CONTAINERS: [&str; 10] = [
    "data",
    "biz_params",
    "verify_data",
    "verify_methods",
    "verify_ways",
    "user_data",
    "mobile_sms_verify",
    "mobile_up_sms_verify",
    "extra",
    "verify_params",
];

#[derive(Clone)]
pub(crate) struct SodaMfa {
    encrypt_uid: String,
    pub params: BTreeMap<String, String>,
    methods: Vec<QrVerificationMethod>,
    masked_destination: Option<String>,
    up_sms: Option<UpSmsInstructions>,
    resend_at: Option<Instant>,
    attempt_at: Option<Instant>,
    attempts: u8,
    sent: bool,
    pub validated: bool,
}

impl SodaMfa {
    pub(crate) fn parse(bytes: &[u8]) -> Result<Self> {
        let raw: Value = serde_json::from_slice(bytes).map_err(|_| invalid_mfa())?;
        let mut fields = BTreeMap::new();
        let mut methods = Vec::new();
        let mut remaining = 128;
        collect(
            raw.get("data").ok_or_else(invalid_mfa)?,
            0,
            &mut remaining,
            &mut fields,
            &mut methods,
        )?;
        let encrypt_uid = fields
            .remove("encrypt_uid")
            .filter(|value| !value.is_empty())
            .ok_or_else(invalid_mfa)?;
        let mobile = fields
            .remove("mobile")
            .and_then(|value| mask_mobile(&value));
        let destination = fields.remove("channel_mobile");
        let message = fields.remove("sms_content");
        let up_sms = match (destination, message) {
            (Some(destination), Some(message))
                if destination.len() <= 32
                    && !destination.is_empty()
                    && destination.bytes().all(|b| b.is_ascii_digit() || b == b'+')
                    && !message.is_empty()
                    && message.len() <= 256 =>
            {
                Some(UpSmsInstructions {
                    destination,
                    message,
                })
            }
            (None, None) => None,
            _ => return Err(invalid_mfa()),
        };
        if up_sms.is_some() && !methods.contains(&QrVerificationMethod::UpSms) {
            methods.push(QrVerificationMethod::UpSms);
        }
        if methods.is_empty()
            || (methods.contains(&QrVerificationMethod::UpSms) && up_sms.is_none())
        {
            return Err(invalid_mfa());
        }
        fields.retain(|key, _| VERIFY_KEYS.contains(&key.as_str()));
        if !fields.contains_key("std_verify_flow_id") || !fields.contains_key("std_verify_token") {
            return Err(invalid_mfa());
        }
        Ok(Self {
            encrypt_uid,
            params: fields,
            methods,
            masked_destination: mobile,
            up_sms,
            resend_at: None,
            attempt_at: None,
            attempts: 0,
            sent: false,
            validated: false,
        })
    }

    pub(crate) fn same_challenge(&self, other: &Self) -> bool {
        self.encrypt_uid == other.encrypt_uid
            && self.params.get("std_verify_flow_id") == other.params.get("std_verify_flow_id")
            && self.params.get("std_verify_token") == other.params.get("std_verify_token")
    }

    pub(crate) fn preserve_limits(&mut self, previous: &Self) {
        self.resend_at = previous.resend_at;
        self.attempt_at = previous.attempt_at;
        self.attempts = previous.attempts;
    }

    pub(crate) fn describe(&self) -> QrVerification {
        let remaining = self
            .resend_at
            .and_then(|until| until.checked_duration_since(Instant::now()));
        QrVerification {
            methods: self.methods.clone(),
            masked_destination: self.masked_destination.clone(),
            resend_after_secs: remaining.map(|d| d.as_secs() + u64::from(d.subsec_nanos() > 0)),
            up_sms: self.up_sms.clone(),
        }
    }

    pub(crate) fn prepare(
        &mut self,
        action: &QrVerificationAction,
    ) -> Result<(&'static str, String)> {
        if self.validated {
            return Err(invalid_action(
                "Soda verification already succeeded; poll the QR transaction",
            ));
        }
        let method = match action {
            QrVerificationAction::VerifyUpSms => QrVerificationMethod::UpSms,
            _ => QrVerificationMethod::Sms,
        };
        if !self.methods.contains(&method) {
            return Err(invalid_action(
                "the selected Soda verification method is unavailable",
            ));
        }
        let now = Instant::now();
        match action {
            QrVerificationAction::SendSms => {
                if self.resend_at.is_some_and(|until| now < until) {
                    return Err(rate_limited());
                }
                // A timed-out delivery can still have sent a message. Never blindly send again.
                self.resend_at = Some(now + Duration::from_secs(60));
            }
            QrVerificationAction::SubmitSms { code } => {
                if !self.sent {
                    return Err(invalid_action(
                        "request the Soda SMS code before submitting it",
                    ));
                }
                if code.len() != 6 || !code.bytes().all(|byte| byte.is_ascii_digit()) {
                    return Err(invalid_action(
                        "Soda verification requires a six-digit code",
                    ));
                }
                self.begin_attempt(now)?;
            }
            QrVerificationAction::VerifyUpSms => self.begin_attempt(now)?,
        }
        let mut params = self.params.clone();
        params.extend([
            ("encrypt_uid".to_owned(), self.encrypt_uid.clone()),
            ("aid".to_owned(), "386088".to_owned()),
            ("verify_ticket".to_owned(), String::new()),
            ("copywriting_key".to_owned(), "qr_connect".to_owned()),
            ("ies_safety_diversion_tag".to_owned(), "mfa".to_owned()),
            ("new_verify_flow".to_owned(), String::new()),
            (
                "new_authn_sdk_version".to_owned(),
                NEW_AUTHN_SDK_VERSION.to_owned(),
            ),
        ]);
        let endpoint = match action {
            QrVerificationAction::SendSms => {
                params.insert("is6Digits".to_owned(), "1".to_owned());
                "https://api.qishui.com/passport/web/send_code/"
            }
            QrVerificationAction::SubmitSms { code } => {
                params.insert("code".to_owned(), hex::encode(code.as_bytes()));
                "https://api.qishui.com/passport/web/validate_code/"
            }
            QrVerificationAction::VerifyUpSms => "https://api.qishui.com/passport/upsms/verify/",
        };
        if method == QrVerificationMethod::Sms {
            params.insert("mix_mode".to_owned(), "1".to_owned());
            params.insert("type".to_owned(), "3737".to_owned());
        }
        params.insert("std_verify_way".to_owned(), method_name(method).to_owned());
        let form = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(params)
            .finish();
        Ok((endpoint, form))
    }

    fn begin_attempt(&mut self, now: Instant) -> Result<()> {
        if self.attempts >= 8 {
            return Err(TuneWeaveError::new(
                ErrorCode::RateLimited,
                "Soda verification attempt limit reached; create a new QR transaction",
            )
            .with_platform(Platform::Soda));
        }
        if self.attempt_at.is_some_and(|until| now < until) {
            return Err(rate_limited());
        }
        self.attempts += 1;
        self.attempt_at = Some(now + Duration::from_secs(2));
        Ok(())
    }

    pub(crate) fn cool_down(&mut self) {
        let until = Instant::now() + Duration::from_secs(60);
        self.attempt_at = Some(self.attempt_at.map_or(until, |current| current.max(until)));
        self.resend_at = Some(self.resend_at.map_or(until, |current| current.max(until)));
    }

    pub(crate) fn accept(&mut self, action: &QrVerificationAction, bytes: &[u8]) -> Result<()> {
        let raw: Value = serde_json::from_slice(bytes).map_err(|_| invalid_mfa())?;
        let data = raw
            .get("data")
            .and_then(Value::as_object)
            .ok_or_else(invalid_mfa)?;
        let code = match data.get("error_code") {
            Some(value) => value.as_i64().ok_or_else(invalid_mfa)?,
            None => 0,
        };
        if code == 7 {
            return Err(rate_limited());
        }
        if code != 0 || raw.get("message").and_then(Value::as_str) != Some("success") {
            return Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "Soda did not accept the verification action",
            )
            .with_platform(Platform::Soda));
        }
        match action {
            QrVerificationAction::SendSms => {
                let retry = match data.get("retry_time") {
                    Some(value) => value.as_u64().ok_or_else(invalid_mfa)?,
                    None => 60,
                }
                .max(60);
                if retry > 86400 {
                    return Err(invalid_mfa());
                }
                self.resend_at = Some(Instant::now() + Duration::from_secs(retry));
                self.sent = true;
            }
            QrVerificationAction::SubmitSms { .. } | QrVerificationAction::VerifyUpSms => {
                let ticket = data
                    .get("ticket")
                    .and_then(Value::as_str)
                    .is_some_and(|ticket| !ticket.is_empty());
                let registered = data.get("registered").and_then(Value::as_bool) == Some(true);
                if !ticket
                    && (!registered || matches!(action, QrVerificationAction::SubmitSms { .. }))
                {
                    return Err(invalid_mfa());
                }
                self.validated = true;
            }
        }
        Ok(())
    }
}

fn collect(
    value: &Value,
    depth: usize,
    remaining: &mut usize,
    fields: &mut BTreeMap<String, String>,
    methods: &mut Vec<QrVerificationMethod>,
) -> Result<()> {
    if depth > 6 || *remaining == 0 {
        return Err(invalid_mfa());
    }
    *remaining -= 1;
    match value {
        Value::Object(object) => {
            for (key, value) in object {
                if (key == "verify_way" || key == "std_verify_way")
                    && let Some(method) = value.as_str().and_then(|way| match way {
                        "mobile_sms_verify" => Some(QrVerificationMethod::Sms),
                        "mobile_up_sms_verify" => Some(QrVerificationMethod::UpSms),
                        _ => None,
                    })
                    && !methods.contains(&method)
                {
                    methods.push(method);
                }
                if VERIFY_KEYS.contains(&key.as_str())
                    || ["encrypt_uid", "mobile", "channel_mobile", "sms_content"]
                        .contains(&key.as_str())
                {
                    let text = match value {
                        Value::String(text) => text.clone(),
                        Value::Number(number) if number.is_u64() || number.is_i64() => {
                            number.to_string()
                        }
                        _ => return Err(invalid_mfa()),
                    };
                    if text.len() > 4096 || text.chars().any(char::is_control) {
                        return Err(invalid_mfa());
                    }
                    if !text.is_empty() || key == "std_verify_way" {
                        if fields.get(key).is_some_and(|existing| existing != &text) {
                            return Err(invalid_mfa());
                        }
                        fields.insert(key.clone(), text);
                    }
                } else if CONTAINERS.contains(&key.as_str()) {
                    collect(value, depth + 1, remaining, fields, methods)?;
                }
            }
        }
        Value::Array(array) => {
            for value in array {
                collect(value, depth + 1, remaining, fields, methods)?;
            }
        }
        Value::String(text) if text.len() <= 16384 => {
            if text.trim_start().starts_with('{') || text.trim_start().starts_with('[') {
                let nested: Value = serde_json::from_str(text).map_err(|_| invalid_mfa())?;
                collect(&nested, depth + 1, remaining, fields, methods)?;
            } else {
                let query = text
                    .split_once('?')
                    .map_or(text.as_str(), |(_, query)| query);
                let mut object = serde_json::Map::new();
                for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
                    if VERIFY_KEYS.contains(&key.as_ref())
                        && object
                            .insert(key.into_owned(), Value::String(value.into_owned()))
                            .is_some()
                    {
                        return Err(invalid_mfa());
                    }
                }
                collect(
                    &Value::Object(object),
                    depth + 1,
                    remaining,
                    fields,
                    methods,
                )?;
            }
        }
        Value::Null => {}
        _ => return Err(invalid_mfa()),
    }
    Ok(())
}

fn mask_mobile(value: &str) -> Option<String> {
    if value.len() > 32
        || !value
            .bytes()
            .all(|b| b.is_ascii_digit() || matches!(b, b'*' | b'+' | b' ' | b'-'))
    {
        return None;
    }
    if value.contains('*') {
        return Some(value.to_owned());
    }
    let digits: String = value.chars().filter(char::is_ascii_digit).collect();
    (digits.len() >= 7).then(|| format!("{}****{}", &digits[..3], &digits[digits.len() - 2..]))
}

fn method_name(method: QrVerificationMethod) -> &'static str {
    match method {
        QrVerificationMethod::Sms => "mobile_sms_verify",
        QrVerificationMethod::UpSms => "mobile_up_sms_verify",
    }
}
fn invalid_mfa() -> TuneWeaveError {
    soda_upstream_error(
        "Soda returned incomplete or inconsistent verification data; create a new QR transaction",
    )
}
fn invalid_action(message: &str) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Soda)
}
fn rate_limited() -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::RateLimited, "Soda verification is cooling down")
        .with_platform(Platform::Soda)
        .retryable(true)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sms_fixture() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"data": {
            "account_flow":"verify", "error_code":2046, "encrypt_uid":"encrypted-user-secret",
            "mobile":"13800138000", "biz_params": {"std_verify_flow_id":"flow-secret",
            "std_verify_token":"verify-token-secret", "std_verify_type":1, "std_verify_way":"",
            "verify_way":"mobile_sms_verify"}
        }}))
        .unwrap()
    }

    #[test]
    fn public_instructions_hide_private_parameters_and_forms_use_fixed_targets() {
        let mut state = SodaMfa::parse(&sms_fixture()).unwrap();
        let description = state.describe();
        assert_eq!(description.methods, vec![QrVerificationMethod::Sms]);
        let public = serde_json::to_string(&description).unwrap();
        for secret in [
            "encrypted-user-secret",
            "flow-secret",
            "verify-token-secret",
            "13800138000",
        ] {
            assert!(!public.contains(secret));
            assert!(!format!("{description:?}").contains(secret));
        }
        let (url, form) = state.prepare(&QrVerificationAction::SendSms).unwrap();
        assert_eq!(url, "https://api.qishui.com/passport/web/send_code/");
        let params: BTreeMap<_, _> = url::form_urlencoded::parse(form.as_bytes())
            .into_owned()
            .collect();
        assert_eq!(params["aid"], "386088");
        assert_eq!(params["encrypt_uid"], "encrypted-user-secret");
        assert_eq!(params["std_verify_way"], "mobile_sms_verify");
        assert_eq!(params["new_authn_sdk_version"], NEW_AUTHN_SDK_VERSION);
        assert_eq!(
            state
                .prepare(&QrVerificationAction::SendSms)
                .unwrap_err()
                .code,
            ErrorCode::RateLimited
        );
        assert_eq!(
            state
                .prepare(&QrVerificationAction::SubmitSms {
                    code: "864209".to_owned()
                })
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn validation_requires_sent_sms_and_proven_success_and_is_bounded() {
        let mut state = SodaMfa::parse(&sms_fixture()).unwrap();
        state.prepare(&QrVerificationAction::SendSms).unwrap();
        state
            .accept(
                &QrVerificationAction::SendSms,
                br#"{"message":"success","data":{"retry_time":120}}"#,
            )
            .unwrap();
        let action = QrVerificationAction::SubmitSms {
            code: "864209".to_owned(),
        };
        let (_, form) = state.prepare(&action).unwrap();
        let params: BTreeMap<_, _> = url::form_urlencoded::parse(form.as_bytes())
            .into_owned()
            .collect();
        assert_eq!(params["code"], "383634323039");
        assert_eq!(params["new_authn_sdk_version"], NEW_AUTHN_SDK_VERSION);
        assert!(
            state
                .accept(
                    &action,
                    br#"{"message":"success","data":{"registered":true}}"#
                )
                .is_err()
        );
        assert!(!state.validated);
        for _ in 1..8 {
            state.attempt_at = None;
            state.prepare(&action).unwrap();
            assert!(
                state
                    .accept(&action, br#"{"message":"error","data":{"error_code":123}}"#)
                    .is_err()
            );
        }
        state.attempt_at = None;
        assert_eq!(
            state.prepare(&action).unwrap_err().code,
            ErrorCode::RateLimited
        );
        let mut replacement = SodaMfa::parse(&sms_fixture()).unwrap();
        replacement.preserve_limits(&state);
        assert_eq!(replacement.attempts, 8);
    }

    #[test]
    fn successful_sms_keeps_the_original_empty_poll_way_for_qr_continuation() {
        let mut state = SodaMfa::parse(&sms_fixture()).unwrap();
        state.prepare(&QrVerificationAction::SendSms).unwrap();
        state
            .accept(
                &QrVerificationAction::SendSms,
                br#"{"message":"success","data":{"retry_time":60}}"#,
            )
            .unwrap();
        let action = QrVerificationAction::SubmitSms {
            code: "864209".to_owned(),
        };
        state.prepare(&action).unwrap();
        state
            .accept(
                &action,
                br#"{"message":"success","data":{"ticket":"safe-test-ticket"}}"#,
            )
            .unwrap();
        assert!(state.validated);
        assert_eq!(
            state.params.get("std_verify_way").map(String::as_str),
            Some("")
        );
    }

    #[test]
    fn parser_rejects_conflicting_identity_unbounded_nesting_and_unusable_methods() {
        let mut raw: Value = serde_json::from_slice(&sms_fixture()).unwrap();
        raw["data"]["biz_params"]["encrypt_uid"] = Value::String("different-user".to_owned());
        assert!(SodaMfa::parse(&serde_json::to_vec(&raw).unwrap()).is_err());
        let mut raw: Value = serde_json::from_slice(&sms_fixture()).unwrap();
        raw["data"]["biz_params"]["verify_way"] = Value::String("unknown-way".to_owned());
        assert!(SodaMfa::parse(&serde_json::to_vec(&raw).unwrap()).is_err());
        let mut nested = serde_json::json!({"encrypt_uid":"hidden"});
        for _ in 0..10 {
            nested = serde_json::json!({"biz_params":nested});
        }
        assert!(
            SodaMfa::parse(&serde_json::to_vec(&serde_json::json!({"data":nested})).unwrap())
                .is_err()
        );
    }

    #[test]
    fn upstream_sms_requires_instructions_and_cannot_be_used_as_normal_sms() {
        let mut raw: Value = serde_json::from_slice(&sms_fixture()).unwrap();
        raw["data"]["biz_params"]["verify_way"] = serde_json::json!("mobile_up_sms_verify");
        assert!(SodaMfa::parse(&serde_json::to_vec(&raw).unwrap()).is_err());
        raw["data"]["channel_mobile"] = serde_json::json!("10690000");
        raw["data"]["sms_content"] = serde_json::json!("example-up-sms-content");
        let mut state = SodaMfa::parse(&serde_json::to_vec(&raw).unwrap()).unwrap();
        assert!(state.prepare(&QrVerificationAction::SendSms).is_err());
        let (url, _) = state.prepare(&QrVerificationAction::VerifyUpSms).unwrap();
        assert_eq!(url, "https://api.qishui.com/passport/upsms/verify/");
        state
            .accept(
                &QrVerificationAction::VerifyUpSms,
                br#"{"message":"success","data":{"registered":true}}"#,
            )
            .unwrap();
        assert!(state.validated);
    }
}
