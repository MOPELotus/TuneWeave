use super::*;
use tuneweave_core::{RecommendationStyleCategory, RecommendationStyleSelection};
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Params {
    platform: Option<String>,
    account: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
}
fn access(
    state: &AppState,
    headers: &HeaderMap,
    params: &Params,
    default: Option<Platform>,
) -> Result<ProviderAccess, ApiError> {
    let platform = match params.platform.as_deref() {
        Some(value) => parse_platform_parameter(value)?,
        None => default.unwrap_or(Platform::Netease),
    };
    Ok(
        CallerCredentialSet::from_headers(headers, state)?.select_provider(
            state,
            platform,
            params.account.as_deref(),
            AccountSelection::Optional,
        )?,
    )
}
pub(super) async fn catalog(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<Params>, QueryRejection>,
) -> Result<Json<ApiResponse<Vec<RecommendationStyleCategory>>>, ApiError> {
    caller_scope::mark_sensitive();
    let params = query_params(params)?;
    let access = access(&state, &headers, &params, None)?;
    let value = access
        .provider
        .recommendation_styles(access.provider_account.as_deref())
        .await?;
    Ok(Json(
        access.list_response(value, access.provider.platform()),
    ))
}
pub(super) async fn selection(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<Params>, QueryRejection>,
) -> Result<Json<ApiResponse<RecommendationStyleSelection>>, ApiError> {
    caller_scope::mark_sensitive();
    let params = query_params(params)?;
    let access = access(&state, &headers, &params, None)?;
    let value = access
        .provider
        .recommendation_style_selection(access.provider_account.as_deref())
        .await?;
    Ok(Json(access.response(value, access.provider.platform())))
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Update {
    platform: Option<String>,
    account: Option<String>,
    category_id: u64,
    tag_ids: Vec<u64>,
}
pub(super) async fn update(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<Update>, JsonRejection>,
) -> Result<Json<ApiResponse<RecommendationStyleSelection>>, ApiError> {
    caller_scope::mark_sensitive();
    let body = json_body(body)?;
    let params = Params {
        platform: body.platform,
        account: body.account,
        ..Default::default()
    };
    let access = access(&state, &headers, &params, None)?;
    let value = access
        .provider
        .update_recommendation_style(
            &RecommendationStyleSelection {
                category_id: body.category_id,
                tag_ids: body.tag_ids,
            },
            access.provider_account.as_deref(),
        )
        .await?;
    Ok(Json(access.response(value, access.provider.platform())))
}
pub(super) async fn tracks(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<Params>, QueryRejection>,
) -> Result<Json<ApiResponse<Vec<Track>>>, ApiError> {
    caller_scope::mark_sensitive();
    read_tracks(&state, &headers, query_params(params)?, false).await
}
pub(super) async fn daily(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<Params>, QueryRejection>,
) -> Result<Json<ApiResponse<Vec<Track>>>, ApiError> {
    caller_scope::mark_sensitive();
    read_tracks(&state, &headers, query_params(params)?, true).await
}
async fn read_tracks(
    state: &AppState,
    headers: &HeaderMap,
    params: Params,
    daily: bool,
) -> Result<Json<ApiResponse<Vec<Track>>>, ApiError> {
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be 1 to 100").into());
    }
    let access = access(state, headers, &params, None)?;
    if daily && access.provider.platform() != Platform::Netease {
        return Err(TuneWeaveError::invalid_request("this daily endpoint requires NetEase").into());
    }
    let request = PageRequest {
        limit,
        offset,
        account: access.provider_account.clone(),
    };
    let page = if daily {
        access
            .provider
            .daily_recommended_tracks_v1(&request)
            .await?
    } else {
        access.provider.style_recommended_tracks(&request).await?
    };
    Ok(Json(
        access
            .response(page.items, access.provider.platform())
            .with_pagination(page.pagination),
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
    async fn recommendation_style_http_rejects_bad_inputs_and_anonymous_writes_without_network() {
        let mut registry = ProviderRegistry::new();
        registry
            .register(
                tuneweave_provider_netease::NeteaseProvider::new(
                    tuneweave_provider_netease::NeteaseConfig::default(),
                )
                .unwrap(),
            )
            .unwrap();
        let app = build_router(AppState::new(registry, Platform::Netease));
        for (method, path, body, status) in [
            (
                "GET",
                "/v1/recommendations/styles/tracks?limit=0",
                "",
                StatusCode::BAD_REQUEST,
            ),
            (
                "GET",
                "/v1/recommendations/styles",
                "",
                StatusCode::UNAUTHORIZED,
            ),
            (
                "GET",
                "/v1/recommendations/styles/selection",
                "",
                StatusCode::UNAUTHORIZED,
            ),
            (
                "PUT",
                "/v1/recommendations/styles/selection",
                r#"{"category_id":1,"tag_ids":[2,2]}"#,
                StatusCode::BAD_REQUEST,
            ),
            (
                "PUT",
                "/v1/recommendations/styles/selection",
                r#"{"category_id":1,"tag_ids":[2]}"#,
                StatusCode::UNAUTHORIZED,
            ),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status, "{path}");
            assert!(
                response.headers()["cache-control"]
                    .to_str()
                    .unwrap()
                    .contains("no-store")
            );
        }
    }
}
