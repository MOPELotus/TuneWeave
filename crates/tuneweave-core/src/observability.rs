use std::time::Duration;

use crate::{ErrorCode, Platform};

/// Coarse, searchable classification of a provider's business response.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpstreamBusinessClass {
    NotInspected,
    Success,
    AllowedError,
    RejectedError,
    Unavailable,
}

impl UpstreamBusinessClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotInspected => "not_inspected",
            Self::Success => "success",
            Self::AllowedError => "business_error_allowed",
            Self::RejectedError => "business_error",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Stable final result of one upstream attempt.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpstreamOutcome {
    Success,
    Failure { code: ErrorCode, retryable: bool },
}

impl UpstreamOutcome {
    #[must_use]
    pub const fn final_class(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure { code, .. } => code.as_str(),
        }
    }

    #[must_use]
    pub const fn retryable(self) -> bool {
        match self {
            Self::Success => false,
            Self::Failure { retryable, .. } => retryable,
        }
    }
}

/// Secret-free fields shared by provider upstream request events.
///
/// Text fields are deliberately limited to static, implementation-owned values
/// so callers cannot accidentally log a dynamic URL, query, credential, or
/// platform response.
#[derive(Clone, Copy, Debug)]
pub struct UpstreamRequestSummary {
    pub provider: Platform,
    pub operation: &'static str,
    pub upstream_host: &'static str,
    pub endpoint: &'static str,
    pub http_status: Option<u16>,
    pub business_class: UpstreamBusinessClass,
    pub duration: Duration,
    pub batch_size: Option<usize>,
    pub retry_count: u8,
    pub proxy: bool,
    pub fallback: bool,
    pub outcome: UpstreamOutcome,
}

impl UpstreamRequestSummary {
    /// Emits one structured event into the current request span.
    pub fn emit(self) {
        let duration_ms = u64::try_from(self.duration.as_millis()).unwrap_or(u64::MAX);
        let http_status = self.http_status.unwrap_or_default();
        let http_completed = self.http_status.is_some();
        let business_class = self.business_class.as_str();
        let batch_size = self.batch_size.unwrap_or_default();
        let batched = self.batch_size.is_some();
        let final_class = self.outcome.final_class();
        let retryable = self.outcome.retryable();
        let provider = self.provider.as_str();

        match self.outcome {
            UpstreamOutcome::Failure { .. } => tracing::error!(
                target: "tuneweave::upstream",
                provider,
                operation = self.operation,
                upstream_host = self.upstream_host,
                endpoint = self.endpoint,
                http_status,
                http_completed,
                business_class,
                duration_ms,
                batch_size,
                batched,
                retry_count = self.retry_count,
                proxy = self.proxy,
                fallback = self.fallback,
                final_class,
                retryable,
                "Upstream request completed"
            ),
            UpstreamOutcome::Success
                if self.business_class == UpstreamBusinessClass::AllowedError =>
            {
                tracing::warn!(
                    target: "tuneweave::upstream",
                    provider,
                    operation = self.operation,
                    upstream_host = self.upstream_host,
                    endpoint = self.endpoint,
                    http_status,
                    http_completed,
                    business_class,
                    duration_ms,
                    batch_size,
                    batched,
                    retry_count = self.retry_count,
                    proxy = self.proxy,
                    fallback = self.fallback,
                    final_class,
                    retryable,
                    "Upstream request completed"
                );
            }
            UpstreamOutcome::Success => tracing::info!(
                target: "tuneweave::upstream",
                provider,
                operation = self.operation,
                upstream_host = self.upstream_host,
                endpoint = self.endpoint,
                http_status,
                http_completed,
                business_class,
                duration_ms,
                batch_size,
                batched,
                retry_count = self.retry_count,
                proxy = self.proxy,
                fallback = self.fallback,
                final_class,
                retryable,
                "Upstream request completed"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_outcomes_reuse_stable_error_names() {
        let failure = UpstreamOutcome::Failure {
            code: ErrorCode::RateLimited,
            retryable: true,
        };
        assert_eq!(failure.final_class(), "rate_limited");
        assert!(failure.retryable());
        assert_eq!(UpstreamOutcome::Success.final_class(), "success");
        assert!(!UpstreamOutcome::Success.retryable());
    }

    #[test]
    fn business_classes_have_stable_searchable_names() {
        assert_eq!(
            UpstreamBusinessClass::NotInspected.as_str(),
            "not_inspected"
        );
        assert_eq!(UpstreamBusinessClass::Success.as_str(), "success");
        assert_eq!(
            UpstreamBusinessClass::AllowedError.as_str(),
            "business_error_allowed"
        );
        assert_eq!(
            UpstreamBusinessClass::RejectedError.as_str(),
            "business_error"
        );
        assert_eq!(UpstreamBusinessClass::Unavailable.as_str(), "unavailable");
    }
}
