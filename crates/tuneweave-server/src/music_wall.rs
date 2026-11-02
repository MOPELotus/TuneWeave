use super::*;
use crate::caller_scope::mark_sensitive;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Params {
    account: Option<String>,
    user_id: Option<String>,
}
pub(super) async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<Params>, QueryRejection>,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    mark_sensitive();
    let params = query_params(params)?;
    let credentials = CallerCredentialSet::from_headers(&headers, &state)?;
    let access = credentials.select_provider(
        &state,
        Platform::Soda,
        params.account.as_deref(),
        AccountSelection::Default,
    )?;
    let result = access
        .provider
        .music_wall(params.user_id.as_deref(), Some(access.required_account()))
        .await;
    let (wall, update) = finish_account_operation(
        access.provider.as_ref(),
        Platform::Soda,
        credentials.credentials.contains_key(&Platform::Soda),
        result,
    )?;
    Ok(Json(
        access
            .response(wall, Platform::Soda)
            .with_caller_credential(update),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    #[tokio::test]
    async fn music_wall_route_rejects_invalid_input_without_caching_errors() {
        let mut registry = ProviderRegistry::new();
        registry
            .register(
                tuneweave_provider_soda::SodaProvider::new(
                    tuneweave_provider_soda::SodaConfig::default(),
                )
                .unwrap(),
            )
            .unwrap();
        let app = build_router(AppState::new(registry, Platform::Soda));
        for query in ["?unexpected=1", "?user_id=1%2Fbad"] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/v1/extensions/soda/music-wall{query}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert_eq!(response.headers().get("cache-control").unwrap(), "no-store");
        }
    }
}
