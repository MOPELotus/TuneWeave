use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::{Capability, Platform};

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

/// A platform-neutral TuneWeave failure.
#[derive(Debug, Error)]
#[error("{message}")]
pub struct TuneWeaveError {
    pub code: ErrorCode,
    pub message: String,
    pub platform: Option<Platform>,
    pub retryable: bool,
    pub details: Value,
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
