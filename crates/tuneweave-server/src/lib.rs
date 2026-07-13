mod response;

use std::time::Instant;

use axum::{
    Json, Router,
    extract::{Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};
use tuneweave_core::{Capability, Platform, ProviderRegistry, TuneWeaveError};

pub use response::{ApiError, ApiResponse, ResponseMeta};

#[derive(Clone)]
pub struct AppState {
    registry: ProviderRegistry,
    default_platform: Platform,
    started_at: Instant,
}

impl AppState {
    #[must_use]
    pub fn new(registry: ProviderRegistry, default_platform: Platform) -> Self {
        Self {
            registry,
            default_platform,
            started_at: Instant::now(),
        }
    }
}

pub fn build_router(state: AppState) -> Router {
    let versioned = Router::new()
        .route("/platforms", get(platforms))
        .route("/capabilities", get(capabilities));

    Router::new()
        .route("/healthz", get(health))
        .nest("/v1", versioned)
        .with_state(state)
}

#[derive(Debug, Serialize)]
struct Health {
    status: &'static str,
    version: &'static str,
    uptime_ms: u128,
}

async fn health(State(state): State<AppState>) -> Json<ApiResponse<Health>> {
    Json(ApiResponse::new(Health {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        uptime_ms: state.started_at.elapsed().as_millis(),
    }))
}

#[derive(Debug, Serialize)]
struct PlatformStatus {
    platform: Platform,
    registered: bool,
    default: bool,
    capabilities: Vec<Capability>,
}

fn platform_status(state: &AppState, platform: Platform) -> PlatformStatus {
    let provider = state.registry.get(platform);
    PlatformStatus {
        platform,
        registered: provider.is_some(),
        default: platform == state.default_platform,
        capabilities: provider
            .map(|provider| provider.capabilities().into_iter().collect())
            .unwrap_or_default(),
    }
}

async fn platforms(State(state): State<AppState>) -> Json<ApiResponse<Vec<PlatformStatus>>> {
    let data = Platform::ALL
        .into_iter()
        .map(|platform| platform_status(&state, platform))
        .collect();
    Json(ApiResponse::new(data))
}

#[derive(Debug, Deserialize)]
struct CapabilitiesQuery {
    platform: Option<String>,
}

async fn capabilities(
    State(state): State<AppState>,
    Query(query): Query<CapabilitiesQuery>,
) -> Result<Json<ApiResponse<Vec<PlatformStatus>>>, ApiError> {
    let data = if let Some(value) = query.platform {
        let platform = value.parse().map_err(|_| {
            TuneWeaveError::invalid_request(format!("unsupported platform: {value}"))
        })?;
        vec![platform_status(&state, platform)]
    } else {
        Platform::ALL
            .into_iter()
            .map(|platform| platform_status(&state, platform))
            .collect()
    };

    Ok(Json(ApiResponse::new(data)))
}

#[cfg(test)]
mod tests {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use serde_json::Value;
    use tower::ServiceExt;

    use super::*;

    fn test_app() -> Router {
        build_router(AppState::new(ProviderRegistry::new(), Platform::Netease))
    }

    async fn json_response(path: &str) -> (StatusCode, Value) {
        let response = test_app()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .body(Body::empty())
                    .expect("build request"),
            )
            .await
            .expect("request succeeds");
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let json = serde_json::from_slice(&body).expect("valid JSON");
        (status, json)
    }

    #[tokio::test]
    async fn health_uses_the_success_envelope() {
        let (status, json) = json_response("/healthz").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["ok"], true);
        assert_eq!(json["data"]["status"], "ok");
        assert!(json["meta"]["request_id"].is_string());
    }

    #[tokio::test]
    async fn platform_discovery_does_not_claim_unregistered_capabilities() {
        let (status, json) = json_response("/v1/platforms").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"].as_array().map(Vec::len), Some(6));
        assert_eq!(json["data"][0]["platform"], "netease");
        assert_eq!(json["data"][0]["registered"], false);
        assert_eq!(json["data"][0]["default"], true);
    }

    #[tokio::test]
    async fn invalid_platform_uses_the_error_envelope() {
        let (status, json) = json_response("/v1/capabilities?platform=unknown").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["ok"], false);
        assert_eq!(json["error"]["code"], "invalid_request");
    }
}
