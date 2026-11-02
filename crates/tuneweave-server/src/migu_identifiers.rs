use super::*;
use tuneweave_core::MiguSongIdentifiers;
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct QueryParams {
    song_id: String,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Batch {
    song_ids: Vec<String>,
}
fn access(state: &AppState, headers: &HeaderMap) -> Result<ProviderAccess, ApiError> {
    Ok(
        CallerCredentialSet::from_headers(headers, state)?.select_provider(
            state,
            Platform::Migu,
            None,
            AccountSelection::Optional,
        )?,
    )
}
pub(super) async fn identifiers(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<QueryParams>, QueryRejection>,
) -> Result<Json<ApiResponse<MiguSongIdentifiers>>, ApiError> {
    let params = query_params(params)?;
    let access = access(&state, &headers)?;
    let result = access
        .provider
        .migu_song_identifiers(&params.song_id)
        .await?;
    Ok(Json(access.response(result, Platform::Migu)))
}
pub(super) async fn tracks(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Result<Json<Batch>, JsonRejection>,
) -> Result<Json<ApiResponse<Vec<Track>>>, ApiError> {
    let body = json_body(body)?;
    let access = access(&state, &headers)?;
    let result = access
        .provider
        .migu_tracks_by_song_ids(&body.song_ids)
        .await?;
    Ok(Json(access.list_response(result, Platform::Migu)))
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
    async fn migu_identifier_routes_reject_bad_ids_and_batches_before_network() {
        let mut registry = ProviderRegistry::new();
        registry
            .register(
                tuneweave_provider_migu::MiguProvider::new(
                    tuneweave_provider_migu::MiguConfig::default(),
                )
                .unwrap(),
            )
            .unwrap();
        let app = build_router(AppState::new(registry, Platform::Migu));
        for (method, path, body) in [
            (
                "GET",
                "/v1/extensions/migu/song-identifiers?song_id=1%7C2",
                "",
            ),
            (
                "POST",
                "/v1/extensions/migu/songs/by-songid",
                r#"{"song_ids":[]}"#,
            ),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .method(method)
                        .header("content-type", "application/json")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
    }
}
