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
            for key in ["source_chain", "json_line", "json_column", "json_category"] {
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
        for _ in 0..8 {
            let Some(source) = current else { break };
            let message = safe_error_cause(&source.to_string());
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
        self
    }

    /// Include a producer's human-readable business failure without echoing its payload.
    #[must_use]
    pub fn with_upstream_message(mut self, message: &str) -> Self {
        let message = safe_error_cause(message);
        if !message.is_empty() {
            self.message.push_str(": ");
            self.message.push_str(&message);
            if !self.details.is_object() {
                self.details = json!({});
            }
            self.details["upstream_message"] = json!(message);
        }
        self
    }

    /// JSON diagnostics include location/category; actual payload values are redacted.
    #[must_use]
    pub fn with_json_source(self, error: &serde_json::Error) -> Self {
        let mut error_result = self.with_source(error);
        error_result.details["json_line"] = json!(error.line());
        error_result.details["json_column"] = json!(error.column());
        error_result.details["json_category"] =
            json!(format!("{:?}", error.classify()).to_ascii_lowercase());
        error_result
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

// Third-party errors sometimes embed complete URLs or the rejected JSON value.
// Preserve transport/parser wording, but remove those untrusted data fragments.
fn safe_error_cause(value: &str) -> String {
    let bounded: String = value.chars().take(4096).collect();
    let mut input = bounded.as_str();
    let mut result = String::new();
    while !input.is_empty() {
        if input.starts_with("http://") || input.starts_with("https://") {
            let end = input.find(char::is_whitespace).unwrap_or(input.len());
            result.push_str("[URL redacted]");
            input = &input[end..];
            continue;
        }
        let character = input.chars().next().unwrap();
        input = &input[character.len_utf8()..];
        if matches!(character, '\"' | '\'' | '`') {
            let mut escaped = false;
            while let Some(next) = input.chars().next() {
                input = &input[next.len_utf8()..];
                if next == character && !escaped {
                    break;
                }
                escaped = next == '\\' && !escaped;
            }
            result.push_str("[value redacted]");
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
        let key = word
            .split(['=', ':'])
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        if word.contains(['=', ':'])
            && [
                "token",
                "password",
                "cookie",
                "credential",
                "secret",
                "signature",
                "authorization",
                "musickey",
            ]
            .iter()
            .any(|name| key.contains(name))
        {
            if word.ends_with(['=', ':'])
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
