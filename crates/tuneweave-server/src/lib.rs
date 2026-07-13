mod response;

use std::time::Instant;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tuneweave_core::{
    Capability, Lyrics, PageRequest, Platform, Playlist, ProviderRegistry, ResourceRef, SearchKind,
    SearchQuery, Track, TuneWeaveError,
};

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
        .route("/capabilities", get(capabilities))
        .route("/search", get(search))
        .route("/tracks/{reference}", get(track))
        .route("/tracks/{reference}/lyrics", get(track_lyrics))
        .route("/playlists/{reference}", get(playlist))
        .route("/playlists/{reference}/tracks", get(playlist_tracks));

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

#[derive(Debug, Default, Deserialize)]
struct SearchParams {
    q: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    platform: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
    account: Option<String>,
}

async fn search(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<Json<ApiResponse<Vec<Track>>>, ApiError> {
    let query_text = params
        .q
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| TuneWeaveError::invalid_request("q must not be empty"))?;
    let kind = parse_search_kind(params.kind.as_deref())?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let platform = search_platform(&state, params.platform.as_deref())?;
    let provider = state.registry.require(platform)?;
    let account = params
        .account
        .as_deref()
        .map(str::trim)
        .filter(|account| !account.is_empty())
        .map(str::to_owned);
    let query = SearchQuery {
        query: query_text.to_owned(),
        kind,
        limit,
        offset,
        account: account.clone(),
    };
    let page = provider.search(&query).await?;
    let mut response = ApiResponse::new(page.items)
        .with_platform(platform)
        .with_pagination(page.pagination);
    if let Some(account) = account {
        response = response.with_account(account);
    }

    Ok(Json(response))
}

#[derive(Debug, Default, Deserialize)]
struct AccountParams {
    account: Option<String>,
}

async fn track(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<Track>>, ApiError> {
    let reference = parse_reference(reference)?;
    let account = params
        .account
        .as_deref()
        .map(str::trim)
        .filter(|account| !account.is_empty());
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let track = provider.track(reference.id(), account).await?;
    let mut response = ApiResponse::new(track).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }

    Ok(Json(response))
}

async fn track_lyrics(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<Lyrics>>, ApiError> {
    let reference = parse_reference(reference)?;
    let account = params
        .account
        .as_deref()
        .map(str::trim)
        .filter(|account| !account.is_empty());
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let lyrics = provider.lyrics(reference.id(), account).await?;
    let mut response = ApiResponse::new(lyrics).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }

    Ok(Json(response))
}

async fn playlist(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<Playlist>>, ApiError> {
    let reference = parse_reference(reference)?;
    let account = params
        .account
        .as_deref()
        .map(str::trim)
        .filter(|account| !account.is_empty());
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let playlist = provider.playlist(reference.id(), account).await?;
    let mut response = ApiResponse::new(playlist).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }

    Ok(Json(response))
}

#[derive(Debug, Default, Deserialize)]
struct PlaylistTracksParams {
    limit: Option<String>,
    offset: Option<String>,
    account: Option<String>,
}

async fn playlist_tracks(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<PlaylistTracksParams>,
) -> Result<Json<ApiResponse<Vec<Track>>>, ApiError> {
    let reference = parse_reference(reference)?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let account = params
        .account
        .as_deref()
        .map(str::trim)
        .filter(|account| !account.is_empty())
        .map(str::to_owned);
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let page = provider
        .playlist_tracks(
            reference.id(),
            &PageRequest {
                limit,
                offset,
                account: account.clone(),
            },
        )
        .await?;
    let mut response = ApiResponse::new(page.items)
        .with_platform(platform)
        .with_pagination(page.pagination);
    if let Some(account) = account {
        response = response.with_account(account);
    }

    Ok(Json(response))
}

fn parse_reference(reference: String) -> Result<ResourceRef, TuneWeaveError> {
    reference.parse().map_err(|error| {
        TuneWeaveError::invalid_request(format!("{error}"))
            .with_details(json!({ "reference": reference }))
    })
}

fn search_platform(state: &AppState, value: Option<&str>) -> Result<Platform, TuneWeaveError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(state.default_platform);
    };
    if value.eq_ignore_ascii_case("all") {
        let registered = state.registry.descriptors();
        return match registered.as_slice() {
            [provider] => Ok(provider.platform),
            [] => Err(TuneWeaveError::platform_unavailable(state.default_platform)),
            _ => Err(TuneWeaveError::invalid_request(
                "platform=all is not available until aggregate ranking is enabled",
            )),
        };
    }
    value
        .parse()
        .map_err(|_| TuneWeaveError::invalid_request(format!("unsupported platform: {value}")))
}

fn parse_search_kind(value: Option<&str>) -> Result<SearchKind, TuneWeaveError> {
    match value
        .unwrap_or("track")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "track" => Ok(SearchKind::Track),
        "album" => Ok(SearchKind::Album),
        "artist" => Ok(SearchKind::Artist),
        "playlist" => Ok(SearchKind::Playlist),
        "video" => Ok(SearchKind::Video),
        value => Err(TuneWeaveError::invalid_request(format!(
            "unsupported search type: {value}"
        ))),
    }
}

fn parse_u32_parameter(
    name: &str,
    value: Option<&str>,
    default: u32,
) -> Result<u32, TuneWeaveError> {
    value.map_or(Ok(default), |value| {
        value.parse().map_err(|_| {
            TuneWeaveError::invalid_request(format!("{name} must be an unsigned integer"))
                .with_details(json!({ "parameter": name, "value": value }))
        })
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use async_trait::async_trait;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use serde_json::Value;
    use tower::ServiceExt;
    use tuneweave_core::{ArtistSummary, MusicProvider, Page, PageMeta, Result, SearchQuery};

    use super::*;

    fn test_app() -> Router {
        build_router(AppState::new(ProviderRegistry::new(), Platform::Netease))
    }

    struct TestProvider;

    #[async_trait]
    impl MusicProvider for TestProvider {
        fn platform(&self) -> Platform {
            Platform::Netease
        }

        fn name(&self) -> &'static str {
            "Test NetEase"
        }

        fn capabilities(&self) -> BTreeSet<Capability> {
            BTreeSet::from([
                Capability::SearchTracks,
                Capability::TrackDetail,
                Capability::PlaylistRead,
                Capability::Lyrics,
            ])
        }

        async fn search(&self, query: &SearchQuery) -> Result<Page<Track>> {
            Ok(Page {
                items: vec![sample_track("123")],
                pagination: PageMeta {
                    limit: query.limit,
                    offset: query.offset,
                    total: Some(1),
                    next_offset: None,
                    has_more: false,
                },
            })
        }

        async fn track(&self, id: &str, _account: Option<&str>) -> Result<Track> {
            Ok(sample_track(id))
        }

        async fn playlist(&self, id: &str, _account: Option<&str>) -> Result<Playlist> {
            Ok(sample_playlist(id))
        }

        async fn playlist_tracks(&self, _id: &str, request: &PageRequest) -> Result<Page<Track>> {
            Ok(Page {
                items: vec![sample_track("123")],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(1),
                    next_offset: None,
                    has_more: false,
                },
            })
        }

        async fn lyrics(&self, id: &str, _account: Option<&str>) -> Result<Lyrics> {
            Ok(Lyrics {
                track_ref: ResourceRef::new(Platform::Netease, id).expect("valid test reference"),
                plain: Some("[00:01.00]素胚勾勒出青花".to_owned()),
                translated: None,
                romanized: None,
                word_synced: None,
                format: "lrc".to_owned(),
                contributors: Vec::new(),
                extensions: Default::default(),
            })
        }
    }

    fn sample_track(id: &str) -> Track {
        let mut track = Track::new(
            ResourceRef::new(Platform::Netease, id).expect("valid test reference"),
            "反方向的钟",
        );
        track.artists.push(ArtistSummary {
            resource_ref: None,
            name: "周杰伦".to_owned(),
        });
        track
    }

    fn sample_playlist(id: &str) -> Playlist {
        Playlist {
            resource_ref: ResourceRef::new(Platform::Netease, id).expect("valid test reference"),
            platform: Platform::Netease,
            id: id.to_owned(),
            name: "云音乐热歌榜".to_owned(),
            description: "热门歌曲".to_owned(),
            cover_url: None,
            creator: None,
            track_count: Some(1),
            tags: vec!["流行".to_owned()],
            subscribed: Some(false),
            created_at: None,
            updated_at: None,
            extensions: Default::default(),
        }
    }

    fn test_app_with_provider() -> Router {
        let mut registry = ProviderRegistry::new();
        registry.register(TestProvider).expect("register provider");
        build_router(AppState::new(registry, Platform::Netease))
    }

    async fn json_response_from(app: Router, path: &str) -> (StatusCode, Value) {
        let response = app
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

    async fn json_response(path: &str) -> (StatusCode, Value) {
        json_response_from(test_app(), path).await
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

    #[tokio::test]
    async fn search_uses_default_provider_and_pagination_metadata() {
        let (status, json) =
            json_response_from(test_app_with_provider(), "/v1/search?q=clock&limit=10").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:123");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["pagination"]["limit"], 10);
        assert_eq!(json["meta"]["pagination"]["total"], 1);
    }

    #[tokio::test]
    async fn track_reference_selects_its_provider() {
        let (status, json) =
            json_response_from(test_app_with_provider(), "/v1/tracks/netease:185809").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["ref"], "netease:185809");
        assert_eq!(json["data"]["artists"][0]["name"], "周杰伦");
    }

    #[tokio::test]
    async fn invalid_search_parameters_use_the_error_envelope() {
        let (status, json) =
            json_response_from(test_app_with_provider(), "/v1/search?q=clock&limit=101").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], "invalid_request");
    }

    #[tokio::test]
    async fn invalid_track_reference_uses_the_error_envelope() {
        let (status, json) =
            json_response_from(test_app_with_provider(), "/v1/tracks/missing-separator").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], "invalid_request");
    }

    #[tokio::test]
    async fn playlist_reference_selects_its_provider() {
        let (status, json) =
            json_response_from(test_app_with_provider(), "/v1/playlists/netease:3778678").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["ref"], "netease:3778678");
        assert_eq!(json["data"]["name"], "云音乐热歌榜");
        assert_eq!(json["meta"]["platform"], "netease");
    }

    #[tokio::test]
    async fn playlist_tracks_use_unified_pagination() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/playlists/netease:3778678/tracks?limit=10&offset=0",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:123");
        assert_eq!(json["meta"]["pagination"]["limit"], 10);
        assert_eq!(json["meta"]["pagination"]["total"], 1);
    }

    #[tokio::test]
    async fn track_lyrics_use_reference_platform() {
        let (status, json) =
            json_response_from(test_app_with_provider(), "/v1/tracks/netease:185809/lyrics").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["track_ref"], "netease:185809");
        assert_eq!(json["data"]["format"], "lrc");
        assert_eq!(json["meta"]["platform"], "netease");
    }
}
