use std::{cell::RefCell, time::Instant};

use axum::{
    Json,
    extract::{MatchedPath, Request},
    http::{HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use rand::{RngExt, distr::Alphanumeric};
use serde::Serialize;
use serde_json::Value;
use tracing::Instrument;
use tuneweave_core::{ErrorCode, PageMeta, Platform, TuneWeaveError};

pub const REQUEST_ID_HEADER: &str = "x-request-id";
const MAX_REQUEST_ID_LENGTH: usize = 64;

tokio::task_local! {
    static CURRENT_REQUEST_ID: String;
    static CURRENT_REQUEST_SUMMARY: RefCell<RequestCompletionSummary>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RequestCredentialSource {
    Anonymous,
    Server,
    Caller,
}

pub(crate) trait ResponseResultCount {
    fn response_result_count(&self) -> usize;
}

impl<T> ResponseResultCount for Vec<T> {
    fn response_result_count(&self) -> usize {
        self.len()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum RequestPlatformSummary {
    #[default]
    None,
    One(Platform),
    Multiple,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum RequestCredentialSummary {
    #[default]
    None,
    One(RequestCredentialSource),
    Mixed,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct RequestCompletionSummary {
    platform: RequestPlatformSummary,
    credential_source: RequestCredentialSummary,
    pagination_has_more: Option<bool>,
    result_count: Option<u64>,
}

impl RequestCompletionSummary {
    fn observe_platform(&mut self, platform: Platform) {
        self.platform = match self.platform {
            RequestPlatformSummary::None => RequestPlatformSummary::One(platform),
            RequestPlatformSummary::One(existing) if existing == platform => self.platform,
            RequestPlatformSummary::One(_) | RequestPlatformSummary::Multiple => {
                RequestPlatformSummary::Multiple
            }
        };
    }

    fn observe_credential_source(&mut self, source: RequestCredentialSource) {
        self.credential_source = match self.credential_source {
            RequestCredentialSummary::None => RequestCredentialSummary::One(source),
            RequestCredentialSummary::One(existing) if existing == source => self.credential_source,
            RequestCredentialSummary::One(_) | RequestCredentialSummary::Mixed => {
                RequestCredentialSummary::Mixed
            }
        };
    }

    const fn platform_name(self) -> &'static str {
        match self.platform {
            RequestPlatformSummary::None => "none",
            RequestPlatformSummary::One(platform) => platform.as_str(),
            RequestPlatformSummary::Multiple => "multiple",
        }
    }

    const fn credential_source_name(self) -> &'static str {
        match self.credential_source {
            RequestCredentialSummary::None => "none",
            RequestCredentialSummary::One(RequestCredentialSource::Anonymous) => "anonymous",
            RequestCredentialSummary::One(RequestCredentialSource::Server) => "server",
            RequestCredentialSummary::One(RequestCredentialSource::Caller) => "caller",
            RequestCredentialSummary::Mixed => "mixed",
        }
    }
}

pub(crate) fn record_request_provider_access(
    platform: Platform,
    credential_source: RequestCredentialSource,
) {
    let _ = CURRENT_REQUEST_SUMMARY.try_with(|summary| {
        let mut summary = summary.borrow_mut();
        summary.observe_platform(platform);
        summary.observe_credential_source(credential_source);
    });
}

fn record_request_platform(platform: Platform) {
    let _ = CURRENT_REQUEST_SUMMARY.try_with(|summary| {
        summary.borrow_mut().observe_platform(platform);
    });
}

fn record_request_pagination(has_more: bool) {
    let _ = CURRENT_REQUEST_SUMMARY.try_with(|summary| {
        summary.borrow_mut().pagination_has_more = Some(has_more);
    });
}

fn record_request_result_count(result_count: usize) {
    let result_count = u64::try_from(result_count).unwrap_or(u64::MAX);
    let _ = CURRENT_REQUEST_SUMMARY.try_with(|summary| {
        summary.borrow_mut().result_count = Some(result_count);
    });
}

fn current_request_summary() -> RequestCompletionSummary {
    CURRENT_REQUEST_SUMMARY
        .try_with(|summary| *summary.borrow())
        .unwrap_or_default()
}

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
        record_request_platform(platform);
        self
    }

    #[must_use]
    pub fn with_account(mut self, account: impl Into<String>) -> Self {
        self.meta.account = Some(account.into());
        let _ = CURRENT_REQUEST_SUMMARY.try_with(|summary| {
            summary
                .borrow_mut()
                .observe_credential_source(RequestCredentialSource::Server);
        });
        self
    }

    #[must_use]
    pub(crate) fn with_pagination(mut self, pagination: PageMeta) -> Self
    where
        T: ResponseResultCount,
    {
        record_request_pagination(pagination.has_more);
        record_request_result_count(self.data.response_result_count());
        self.meta.pagination = Some(pagination);
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pagination: Option<PageMeta>,
    cached: bool,
}

impl ResponseMeta {
    #[must_use]
    pub fn new() -> Self {
        Self::with_request_id(current_request_id())
    }

    fn with_request_id(request_id: String) -> Self {
        Self {
            request_id,
            platform: None,
            account: None,
            pagination: None,
            cached: false,
        }
    }
}

pub(crate) async fn request_context_middleware(request: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = request.method().clone();
    let route = request
        .extensions()
        .get::<MatchedPath>()
        .map_or("unmatched", MatchedPath::as_str)
        .to_owned();
    let (request_id, caller_supplied, rejection) = match caller_request_id(request.headers()) {
        Ok(Some(request_id)) => (request_id, true, None),
        Ok(None) => (generate_request_id(), false, None),
        Err(message) => (generate_request_id(), false, Some(message)),
    };
    let span = tracing::info_span!(
        "http_request",
        request_id = %request_id,
        method = %method,
        route = %route,
        operation = %route,
    );
    let scoped_request_id = request_id.clone();
    CURRENT_REQUEST_SUMMARY
        .scope(
            RefCell::new(RequestCompletionSummary::default()),
            CURRENT_REQUEST_ID.scope(
                scoped_request_id,
                async move {
                    let mut response = if let Some(message) = rejection {
                        ApiError::from(TuneWeaveError::invalid_request(message)).into_response()
                    } else {
                        next.run(request).await
                    };
                    response.headers_mut().insert(
                        REQUEST_ID_HEADER,
                        HeaderValue::from_str(&request_id)
                            .expect("validated or generated request IDs are valid header values"),
                    );
                    log_request_completion(
                        &request_id,
                        &method,
                        &route,
                        response.status(),
                        caller_supplied,
                        started,
                        current_request_summary(),
                    );
                    response
                }
                .instrument(span),
            ),
        )
        .await
}

fn caller_request_id(headers: &HeaderMap) -> Result<Option<String>, &'static str> {
    let mut values = headers.get_all(REQUEST_ID_HEADER).iter();
    let Some(value) = values.next() else {
        return Ok(None);
    };
    if values.next().is_some() {
        return Err("x-request-id must be supplied at most once");
    }
    let value = value
        .to_str()
        .map_err(|_| "x-request-id must contain valid ASCII text")?;
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > MAX_REQUEST_ID_LENGTH
        || !bytes[0].is_ascii_alphanumeric()
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(byte))
    {
        return Err(
            "x-request-id must be 1 to 64 ASCII characters using letters, digits, '-', '_', '.', or ':'",
        );
    }
    Ok(Some(value.to_owned()))
}

fn current_request_id() -> String {
    CURRENT_REQUEST_ID
        .try_with(Clone::clone)
        .unwrap_or_else(|_| generate_request_id())
}

fn generate_request_id() -> String {
    let suffix = rand::rng()
        .sample_iter(&Alphanumeric)
        .take(24)
        .map(char::from)
        .collect::<String>();
    format!("tw-{suffix}")
}

fn log_request_completion(
    request_id: &str,
    method: &axum::http::Method,
    route: &str,
    status: StatusCode,
    caller_supplied: bool,
    started: Instant,
    summary: RequestCompletionSummary,
) {
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    let status_code = status.as_u16();
    let platform = summary.platform_name();
    let credential_source = summary.credential_source_name();
    let pagination_present = summary.pagination_has_more.is_some();
    let pagination_has_more = summary.pagination_has_more.unwrap_or(false);
    let result_count_present = summary.result_count.is_some();
    let result_count = summary.result_count.unwrap_or(0);
    if status.is_server_error() {
        tracing::error!(
            request_id,
            method = %method,
            route,
            operation = route,
            status = status_code,
            duration_ms,
            caller_supplied_request_id = caller_supplied,
            platform,
            credential_source,
            pagination_present,
            pagination_has_more,
            result_count_present,
            result_count,
            "HTTP request completed"
        );
    } else if status.is_client_error() {
        tracing::warn!(
            request_id,
            method = %method,
            route,
            operation = route,
            status = status_code,
            duration_ms,
            caller_supplied_request_id = caller_supplied,
            platform,
            credential_source,
            pagination_present,
            pagination_has_more,
            result_count_present,
            result_count,
            "HTTP request completed"
        );
    } else if is_quiet_route(route) {
        tracing::debug!(
            request_id,
            method = %method,
            route,
            operation = route,
            status = status_code,
            duration_ms,
            caller_supplied_request_id = caller_supplied,
            platform,
            credential_source,
            pagination_present,
            pagination_has_more,
            result_count_present,
            result_count,
            "HTTP request completed"
        );
    } else {
        tracing::info!(
            request_id,
            method = %method,
            route,
            operation = route,
            status = status_code,
            duration_ms,
            caller_supplied_request_id = caller_supplied,
            platform,
            credential_source,
            pagination_present,
            pagination_has_more,
            result_count_present,
            result_count,
            "HTTP request completed"
        );
    }
}

fn is_quiet_route(route: &str) -> bool {
    matches!(
        route,
        "/v1/auth/qr/{transaction_id}" | "/v1/tracks/{reference}/stream/content"
    )
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
        let request_id = current_request_id();
        if let Some(platform) = self.0.platform {
            record_request_platform(platform);
        }
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
        log_api_error(&self.0, status, &request_id);
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
                meta: ResponseMeta::with_request_id(request_id),
            }),
        )
            .into_response()
    }
}

fn log_api_error(error: &TuneWeaveError, status: StatusCode, request_id: &str) {
    let error_code = error.code.as_str();
    let platform = error.platform.map_or("none", Platform::as_str);
    let status = status.as_u16();
    if matches!(
        error.code,
        ErrorCode::UpstreamError
            | ErrorCode::PlatformUnavailable
            | ErrorCode::UpstreamTimeout
            | ErrorCode::InternalError
    ) {
        tracing::error!(
            request_id,
            stage = "api_response",
            error_code,
            platform,
            status,
            retryable = error.retryable,
            "TuneWeave request failed"
        );
    } else {
        tracing::warn!(
            request_id,
            stage = "api_response",
            error_code,
            platform,
            status,
            retryable = error.retryable,
            "TuneWeave request failed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn response_summary_merges_platform_credential_and_pagination_without_identifiers() {
        CURRENT_REQUEST_SUMMARY
            .scope(RefCell::new(RequestCompletionSummary::default()), async {
                record_request_provider_access(Platform::Netease, RequestCredentialSource::Caller);
                let _response = ApiResponse::new(Vec::<String>::new())
                    .with_platform(Platform::Netease)
                    .with_pagination(PageMeta {
                        limit: 20,
                        offset: 0,
                        total: None,
                        next_offset: Some(20),
                        has_more: true,
                        extensions: Default::default(),
                    });
                let summary = current_request_summary();
                assert_eq!(summary.platform_name(), "netease");
                assert_eq!(summary.credential_source_name(), "caller");
                assert_eq!(summary.pagination_has_more, Some(true));
                assert_eq!(summary.result_count, Some(0));

                record_request_provider_access(Platform::Qq, RequestCredentialSource::Server);
                let summary = current_request_summary();
                assert_eq!(summary.platform_name(), "multiple");
                assert_eq!(summary.credential_source_name(), "mixed");
            })
            .await;
    }
}
