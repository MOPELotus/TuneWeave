use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::{Capability, Platform, ProviderCredential};

/// Stable machine-readable error codes exposed by the HTTP layer.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    InvalidRequest,
    AuthenticationRequired,
    PermissionDenied,
    ResourceNotFound,
    Conflict,
    CapabilityNotSupported,
    RateLimited,
    UpstreamError,
    PlatformUnavailable,
    UpstreamTimeout,
    MatchRejected,
    InternalError,
}

impl ErrorCode {
    /// Returns the stable wire-format name used by logs and HTTP responses.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::AuthenticationRequired => "authentication_required",
            Self::PermissionDenied => "permission_denied",
            Self::ResourceNotFound => "resource_not_found",
            Self::Conflict => "conflict",
            Self::CapabilityNotSupported => "capability_not_supported",
            Self::RateLimited => "rate_limited",
            Self::UpstreamError => "upstream_error",
            Self::PlatformUnavailable => "platform_unavailable",
            Self::UpstreamTimeout => "upstream_timeout",
            Self::MatchRejected => "match_rejected",
            Self::InternalError => "internal_error",
        }
    }
}

/// A platform-neutral TuneWeave failure.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct TuneWeaveError {
    pub code: ErrorCode,
    pub message: String,
    pub platform: Option<Platform>,
    pub retryable: bool,
    pub details: Value,
    caller_credential_update: Option<Box<ProviderCredential>>,
    auth_challenge_consumed: bool,
}

impl TuneWeaveError {
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            platform: None,
            retryable: false,
            details: json!({}),
            caller_credential_update: None,
            auth_challenge_consumed: false,
        }
    }

    #[must_use]
    pub fn with_platform(mut self, platform: Platform) -> Self {
        self.platform = Some(platform);
        self
    }

    #[must_use]
    pub fn retryable(mut self, retryable: bool) -> Self {
        self.retryable = retryable;
        self
    }

    #[must_use]
    pub fn with_details(mut self, mut details: Value) -> Self {
        if let Some(object) = details.as_object_mut() {
            for key in [
                "source_chain",
                "json_line",
                "json_column",
                "json_category",
                "os_error_code",
                "upstream_code",
                "platform_code",
                "upstream_message",
                "upstream_http_status",
                "http_status",
            ] {
                if let Some(value) = self.details.get(key) {
                    object.entry(key).or_insert_with(|| value.clone());
                }
            }
        }
        self.details = details;
        self
    }

    /// Preserve the underlying cause chain without returning request URLs or payload values.
    /// The short chain is also added to message for clients that do not display details.
    #[must_use]
    pub fn with_source(mut self, error: &(dyn std::error::Error + 'static)) -> Self {
        let mut chain = Vec::new();
        let mut current = Some(error);
        let mut os_error_code = None;
        for _ in 0..8 {
            let Some(source) = current else { break };
            os_error_code = os_error_code.or_else(|| {
                source
                    .downcast_ref::<std::io::Error>()
                    .and_then(std::io::Error::raw_os_error)
            });
            let message = redact_upstream_message(&source.to_string());
            if !message.is_empty() && !chain.contains(&message) {
                chain.push(message);
            }
            current = source.source();
        }
        if !chain.is_empty() {
            let summary = chain.join(": ");
            self.message.push_str(": ");
            self.message.extend(summary.chars().take(1024));
            if !self.details.is_object() {
                self.details = json!({});
            }
            self.details["source_chain"] = json!(chain);
        }
        if let Some(error) = error.downcast_ref::<serde_json::Error>() {
            self.details["json_line"] = json!(error.line());
            self.details["json_column"] = json!(error.column());
            self.details["json_category"] =
                json!(format!("{:?}", error.classify()).to_ascii_lowercase());
        }
        if let Some(code) = os_error_code {
            self.details["os_error_code"] = json!(code);
        }
        self
    }

    /// Include a producer's human-readable business failure without echoing its payload.
    #[must_use]
    pub fn with_upstream_message(mut self, message: &str) -> Self {
        let message = redact_upstream_message(message);
        if !message.is_empty() {
            if !self.message.ends_with(&message) {
                self.message.push_str(": ");
                self.message.push_str(&message);
            }
            if !self.details.is_object() {
                self.details = json!({});
            }
            self.details["upstream_message"] = json!(message);
        }
        self
    }

    /// Normalize known provider diagnostic aliases for adapters. Legacy fields
    /// remain available; business codes are kept separate from HTTP status.
    pub fn normalize_upstream_diagnostics(&mut self) {
        if self.platform.is_none() {
            return;
        }
        let Some(details) = self.details.as_object_mut() else {
            return;
        };
        if let Some(code) = ["upstream_code", "platform_code"]
            .iter()
            .filter_map(|key| details.get(*key))
            .find(|value| is_upstream_code(value))
            .cloned()
        {
            let text = code
                .as_str()
                .map_or_else(|| code.to_string(), str::to_owned);
            if text.bytes().any(|byte| byte != b'0') {
                append_diagnostic(&mut self.message, "upstream code", &text);
            }
            details.insert("upstream_code".into(), code);
        } else {
            details.remove("upstream_code");
        }
        if let Some(status) = ["upstream_http_status", "http_status", "upstream_status"]
            .iter()
            .filter_map(|key| details.get(*key).and_then(Value::as_u64))
            .find(|status| (100..=599).contains(status))
        {
            if status >= 300 {
                append_diagnostic(&mut self.message, "upstream HTTP", &status.to_string());
            }
            details.insert("upstream_http_status".into(), json!(status));
        } else {
            details.remove("upstream_http_status");
        }
        // Some provider adapters predate the shared diagnostic field.
        if let Some(message) = details.get("platform_message").and_then(Value::as_str) {
            let message = redact_upstream_message(message);
            details.insert("platform_message".into(), json!(message));
            if !details
                .get("upstream_message")
                .is_some_and(Value::is_string)
                && !message.is_empty()
            {
                if !self.message.ends_with(&message) {
                    self.message.push_str(": ");
                    self.message.push_str(&message);
                }
                details.insert("upstream_message".into(), json!(message));
            }
        }
    }

    /// JSON diagnostics include location/category; actual payload values are redacted.
    #[must_use]
    pub fn with_json_source(self, error: &serde_json::Error) -> Self {
        self.with_source(error)
    }

    /// Marks a failed verification whose transaction must not be submitted again,
    /// including an uncertain upstream outcome. Adapters must remove their public receipt.
    #[must_use]
    pub fn with_consumed_auth_challenge(mut self) -> Self {
        self.auth_challenge_consumed = true;
        self.retryable = false;
        if !self.details.is_object() {
            self.details = json!({});
        }
        self.details["challenge_consumed"] = json!(true);
        self
    }

    #[must_use]
    pub const fn auth_challenge_consumed(&self) -> bool {
        self.auth_challenge_consumed
    }

    /// Preserves a provider-verified caller credential when a later step fails.
    /// Only use for an operation whose ownership explicitly permits caller delivery.
    /// This secret is not part of public error details; adapters must check the selected
    /// platform and ownership before delivering it. Invalidated sessions cannot export it.
    #[must_use]
    pub fn with_caller_credential_update(mut self, credential: ProviderCredential) -> Self {
        if !matches!(
            self.code,
            ErrorCode::AuthenticationRequired | ErrorCode::Conflict
        ) {
            self.caller_credential_update = Some(Box::new(credential));
        }
        self
    }

    /// Takes the latest accepted credential once. Callers must retain it even though the
    /// operation failed, after checking its platform against the requested credential source.
    pub fn take_caller_credential_update(&mut self) -> Option<ProviderCredential> {
        let credential = self.caller_credential_update.take();
        if matches!(
            self.code,
            ErrorCode::AuthenticationRequired | ErrorCode::Conflict
        ) {
            None
        } else {
            credential.map(|value| *value)
        }
    }

    #[must_use]
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InvalidRequest, message)
    }

    #[must_use]
    pub fn unsupported(platform: Platform, capability: Capability) -> Self {
        Self::new(
            ErrorCode::CapabilityNotSupported,
            format!("{platform} does not support {capability:?}"),
        )
        .with_platform(platform)
        .with_details(json!({ "capability": capability }))
    }

    #[must_use]
    pub fn platform_unavailable(platform: Platform) -> Self {
        Self::new(
            ErrorCode::PlatformUnavailable,
            format!("platform {platform} is not registered"),
        )
        .with_platform(platform)
        .retryable(true)
    }
}

fn is_upstream_code(value: &Value) -> bool {
    value.is_i64()
        || value.is_u64()
        || value.as_str().is_some_and(|code| {
            !code.is_empty()
                && code.len() <= 24
                && code
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        })
}

fn append_diagnostic(message: &mut String, label: &str, value: &str) {
    if !message
        .split(|character: char| {
            !character.is_ascii_alphanumeric() && !matches!(character, '-' | '_')
        })
        .any(|part| part == value)
    {
        message.push_str(&format!(" ({label} {value})"));
    }
}

/// Preserve a bounded diagnostic while redacting URLs, credentials and quoted
/// payload values. Pass only a known upstream message field, never a raw body.
#[must_use]
pub fn redact_upstream_message(value: &str) -> String {
    let bounded: String = value.chars().take(4096).collect();
    let mut input = bounded.as_str();
    let mut result = String::new();
    while !input.is_empty() {
        if input
            .get(..7)
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case("http://"))
            || input
                .get(..8)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
        {
            let end = input.find(char::is_whitespace).unwrap_or(input.len());
            result.push_str("[URL redacted]");
            input = &input[end..];
            continue;
        }
        let character = input.chars().next().unwrap();
        input = &input[character.len_utf8()..];
        let apostrophe = character == '\''
            && result.ends_with(|character: char| character.is_ascii_alphanumeric())
            && input.starts_with(|character: char| character.is_ascii_alphanumeric());
        if matches!(character, '\"' | '\'' | '`') && !apostrophe {
            let mut escaped = false;
            while let Some(next) = input.chars().next() {
                input = &input[next.len_utf8()..];
                if next == character && !escaped {
                    break;
                }
                escaped = next == '\\' && !escaped;
            }
            result.push_str("[value-redacted]");
        } else {
            result.push(if character.is_control() {
                ' '
            } else {
                character
            });
        }
    }
    let mut words = result.split_whitespace().peekable();
    let mut output = Vec::new();
    while let Some(word) = words.next() {
        let inline_assignment = word.contains(['=', ':']);
        let detached_assignment = !inline_assignment
            && words
                .peek()
                .is_some_and(|next| next.starts_with(['=', ':']));
        let key = word
            .split(['=', ':'])
            .next()
            .unwrap_or_default()
            .trim_start_matches(|character: char| {
                !character.is_ascii_alphanumeric() && character != '_'
            })
            .to_ascii_lowercase();
        if matches!(key.as_str(), "cookie" | "set-cookie" | "authorization")
            && (inline_assignment || detached_assignment)
        {
            // These fields can contain space-separated values. The rest of the
            // upstream message is not a safe boundary for exposing them.
            output.push("[credential redacted]".to_owned());
            break;
        } else if word
            .trim_matches(|c: char| !c.is_ascii_alphabetic())
            .eq_ignore_ascii_case("bearer")
        {
            words.next();
            output.push("[credential redacted]".to_owned());
        } else if (inline_assignment || detached_assignment)
            && (matches!(
                key.as_str(),
                "sid" | "skey" | "p_skey" | "buvid3" | "buvid4"
            ) || [
                "token",
                "password",
                "cookie",
                "credential",
                "secret",
                "signature",
                "authorization",
                "musickey",
                "music_u",
                "music_a",
                "nmtid",
                "__csrf",
                "sessionid",
                "session_id",
                "vkey",
                "accesskey",
                "access_key",
                "a_bogus",
                "x-helios",
                "x-medusa",
                "pacm",
                "passport_ticket",
                "sessdata",
                "bili_jct",
                "qrcode_key",
                "v_voucher",
                "qm_keyst",
                "qqmusic_key",
            ]
            .iter()
            .any(|name| key.contains(name)))
        {
            let separate_value = if detached_assignment {
                matches!(words.next(), Some("=" | ":"))
            } else {
                word.ends_with(['=', ':'])
            };
            if separate_value
                && let Some(next) = words.next()
                && next.eq_ignore_ascii_case("bearer")
            {
                words.next();
            }
            output.push("[credential redacted]".to_owned());
        } else if (word.len() >= 32
            && word
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-')))
            || (word.len() >= 4 && word.bytes().all(|c| c.is_ascii_digit()))
        {
            output.push("[opaque value redacted]".to_owned());
        } else {
            output.push(word.to_owned());
        }
    }
    output.join(" ").chars().take(512).collect()
}

pub type Result<T> = std::result::Result<T, TuneWeaveError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_causes_are_readable_without_urls_or_payload_values() {
        let source = std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "Connection refused (os error 111) https://user:private@example.test/?token=secret token=private-value",
        );
        let error = TuneWeaveError::new(ErrorCode::UpstreamError, "QQ API request failed")
            .with_source(&source);
        assert!(error.message.contains("Connection refused (os error 111)"));
        assert!(error.details["source_chain"].is_array());
        let public = format!("{error:?}");
        for secret in ["private", "example.test", "secret"] {
            assert!(!public.contains(secret));
        }
    }

    #[test]
    fn upstream_business_messages_redact_bearer_and_separate_header_values() {
        let error = TuneWeaveError::new(ErrorCode::UpstreamError, "Request rejected")
            .with_upstream_message("account restricted; cookie: private-cookie authorization: Bearer private-token token=private-value");
        assert!(error.message.contains("account restricted"));
        for secret in ["private-cookie", "private-token", "private-value"] {
            assert!(!format!("{error:?}").contains(secret));
        }
    }

    #[test]
    fn provider_diagnostics_keep_business_code_and_http_status_without_repeating_the_reason() {
        let mut error = TuneWeaveError::new(ErrorCode::UpstreamError, "Request rejected")
            .with_platform(Platform::Migu)
            .with_details(json!({
                "platform_code": "0091",
                "platform_message": "Session expired HTTPS://private.test?q=secret",
                "http_status": 503
            }));
        error.normalize_upstream_diagnostics();
        let message = error.message.clone();
        error.normalize_upstream_diagnostics();
        assert_eq!(error.message, message);
        assert_eq!(error.details["upstream_code"], "0091");
        assert_eq!(error.details["upstream_http_status"], 503);
        assert_eq!(
            error.details["upstream_message"],
            "Session expired [URL redacted]"
        );
        assert!(message.contains("upstream code 0091"));
        assert!(message.contains("upstream HTTP 503"));
        assert!(!message.contains("private.test"));
        assert!(!error.details.to_string().contains("private.test"));
    }

    #[test]
    fn provider_messages_do_not_return_named_secrets_or_multivalue_headers() {
        let error = TuneWeaveError::new(ErrorCode::UpstreamError, "Request rejected")
            .with_upstream_message(
                "Session expired; MUSIC_U=short-a PACM: short-b (__csrf=short-c) Bearer short-d session_id =short-g Set-Cookie : a=short-e; b=short-f",
            )
            .with_details(json!({"operation": "login"}));
        let diagnostic = error.details["upstream_message"].as_str().unwrap();
        assert!(diagnostic.contains("Session expired"));
        for secret in [
            "short-a", "short-b", "short-c", "short-d", "short-e", "short-f",
        ] {
            assert!(!diagnostic.contains(secret), "{secret}");
            assert!(!error.message.contains(secret), "{secret}");
        }
    }

    #[test]
    fn diagnostic_preserves_contractions_and_exposes_an_os_code_without_guessing() {
        assert_eq!(
            redact_upstream_message("Can't connect to HTTPS://private.test"),
            "Can't connect to [URL redacted]"
        );
        let error = TuneWeaveError::new(ErrorCode::InternalError, "File operation failed")
            .with_source(&std::io::Error::from_raw_os_error(13))
            .with_details(json!({"operation": "read"}));
        assert_eq!(error.details["os_error_code"], 13);
        assert_eq!(error.details["operation"], "read");
    }

    #[test]
    fn json_errors_keep_the_underlying_reason_and_location_but_not_rejected_values() {
        let source = serde_json::from_str::<Vec<u64>>(r#"["private-cookie"]"#).unwrap_err();
        let error = TuneWeaveError::new(ErrorCode::UpstreamError, "Invalid search JSON")
            .with_json_source(&source);
        assert!(error.message.contains("invalid type"));
        assert!(!format!("{error:?}").contains("private-cookie"));
        assert_eq!(error.details["json_category"], "data");
        assert_eq!(error.details["json_line"], 1);
    }

    #[test]
    fn challenge_consumption_is_typed_and_prevents_retry_without_changing_the_failure_code() {
        let public_only = TuneWeaveError::new(ErrorCode::UpstreamTimeout, "uncertain")
            .with_details(json!({"challenge_consumed":true}));
        assert!(!public_only.auth_challenge_consumed());
        let consumed = TuneWeaveError::new(ErrorCode::UpstreamTimeout, "uncertain")
            .retryable(true)
            .with_consumed_auth_challenge();
        assert!(consumed.auth_challenge_consumed());
        assert!(!consumed.retryable);
        assert_eq!(consumed.code, ErrorCode::UpstreamTimeout);
        assert_eq!(consumed.details["challenge_consumed"], true);
        assert!(
            consumed
                .with_details(json!({"platform_code":"4005"}))
                .auth_challenge_consumed()
        );
    }

    #[test]
    fn failed_operation_updates_are_redacted_latest_only_and_suppressed_on_invalidation() {
        let credential =
            |secret| ProviderCredential::new(Platform::Migu, "session", secret, None).unwrap();
        let mut error = TuneWeaveError::new(ErrorCode::UpstreamTimeout, "request failed")
            .with_platform(Platform::Migu)
            .with_caller_credential_update(credential("earlier-secret"))
            .with_caller_credential_update(credential("latest-secret"));
        for rendered in [
            format!("{error:?}"),
            error.to_string(),
            error.details.to_string(),
        ] {
            assert!(!rendered.contains("earlier-secret"));
            assert!(!rendered.contains("latest-secret"));
        }
        assert_eq!(
            error.take_caller_credential_update().unwrap().secret(),
            "latest-secret"
        );
        assert!(error.take_caller_credential_update().is_none());
        for code in [ErrorCode::AuthenticationRequired, ErrorCode::Conflict] {
            let mut error = TuneWeaveError::new(code, "invalidated")
                .with_caller_credential_update(credential("discard-secret"));
            assert!(error.take_caller_credential_update().is_none());
            let mut error = TuneWeaveError::new(ErrorCode::UpstreamError, "failed")
                .with_caller_credential_update(credential("discard-secret"));
            error.code = code;
            assert!(error.take_caller_credential_update().is_none());
        }
    }

    #[test]
    fn error_code_names_match_the_wire_format() {
        for code in [
            ErrorCode::InvalidRequest,
            ErrorCode::AuthenticationRequired,
            ErrorCode::PermissionDenied,
            ErrorCode::ResourceNotFound,
            ErrorCode::Conflict,
            ErrorCode::CapabilityNotSupported,
            ErrorCode::RateLimited,
            ErrorCode::UpstreamError,
            ErrorCode::PlatformUnavailable,
            ErrorCode::UpstreamTimeout,
            ErrorCode::MatchRejected,
            ErrorCode::InternalError,
        ] {
            assert_eq!(
                serde_json::to_value(code).expect("serialize error code"),
                Value::String(code.as_str().to_owned())
            );
        }
    }
}
