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
    pub fn with_details(mut self, details: Value) -> Self {
        self.details = details;
        self
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

pub type Result<T> = std::result::Result<T, TuneWeaveError>;

#[cfg(test)]
mod tests {
    use super::*;

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
