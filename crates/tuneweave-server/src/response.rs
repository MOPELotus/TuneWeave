use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use serde_json::Value;
use tuneweave_core::{ErrorCode, Platform, TuneWeaveError};

static REQUEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Serialize)]
pub struct ApiResponse<T> {
    ok: bool,
    data: T,
    meta: ResponseMeta,
}

impl<T> ApiResponse<T> {
    #[must_use]
    pub fn new(data: T) -> Self {
        Self {
            ok: true,
            data,
            meta: ResponseMeta::new(),
        }
    }

    #[must_use]
    pub fn with_platform(mut self, platform: Platform) -> Self {
        self.meta.platform = Some(platform);
        self
    }
}

#[derive(Debug, Serialize)]
pub struct ResponseMeta {
    request_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<Platform>,
    #[serde(skip_serializing_if = "Option::is_none")]
    account: Option<String>,
    cached: bool,
}

impl ResponseMeta {
    #[must_use]
    pub fn new() -> Self {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis());
        let sequence = REQUEST_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        Self {
            request_id: format!("tw-{timestamp:x}-{sequence:x}"),
            platform: None,
            account: None,
            cached: false,
        }
    }
}

impl Default for ResponseMeta {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    ok: bool,
    error: ErrorBody,
    meta: ResponseMeta,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: ErrorCode,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform: Option<Platform>,
    retryable: bool,
    details: Value,
}

#[derive(Debug)]
pub struct ApiError(TuneWeaveError);

impl From<TuneWeaveError> for ApiError {
    fn from(error: TuneWeaveError) -> Self {
        Self(error)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = match self.0.code {
            ErrorCode::InvalidRequest => StatusCode::BAD_REQUEST,
            ErrorCode::AuthenticationRequired => StatusCode::UNAUTHORIZED,
            ErrorCode::PermissionDenied => StatusCode::FORBIDDEN,
            ErrorCode::ResourceNotFound => StatusCode::NOT_FOUND,
            ErrorCode::Conflict => StatusCode::CONFLICT,
            ErrorCode::CapabilityNotSupported | ErrorCode::MatchRejected => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            ErrorCode::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            ErrorCode::UpstreamError => StatusCode::BAD_GATEWAY,
            ErrorCode::PlatformUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            ErrorCode::UpstreamTimeout => StatusCode::GATEWAY_TIMEOUT,
            ErrorCode::InternalError => StatusCode::INTERNAL_SERVER_ERROR,
        };
        let error = ErrorBody {
            code: self.0.code,
            message: self.0.message,
            platform: self.0.platform,
            retryable: self.0.retryable,
            details: self.0.details,
        };
        (
            status,
            Json(ErrorEnvelope {
                ok: false,
                error,
                meta: ResponseMeta::new(),
            }),
        )
            .into_response()
    }
}
