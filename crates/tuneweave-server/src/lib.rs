mod response;

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    body::Bytes,
    extract::{
        DefaultBodyLimit, Path, Query, State,
        rejection::{BytesRejection, JsonRejection},
    },
    http::{HeaderMap, header},
    routing::{get, post, put},
};
use rand::{RngExt, distr::Alphanumeric};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tuneweave_core::{
    AccountProfile, Album, AlbumListRequest, AlbumStats, Artist, ArtistArea, ArtistCategory,
    ArtistListRequest, ArtistOverview, ArtistStats, ArtistTrackListRequest, ArtistTrackOrder,
    ArtistUpdatesRequest, ArtistVideoListRequest, ArtistWorkUpdate, ArtistWorksRequest,
    AudioRecognition, AudioRecognitionRequest, AuthChallengeRequest, AuthState, Banner,
    BannerClient, BannerListRequest, Capability, ChallengeMethod, DigitalAlbum,
    DigitalAlbumChartEntry, DigitalAlbumChartKind, DigitalAlbumChartPeriod,
    DigitalAlbumChartRequest, DigitalAlbumListRequest, ImageUploadRequest, ImageUploadResult,
    Lyrics, MediaStream, PageRequest, PasswordFormat, PasswordLoginRequest, Platform,
    PlatformApiRequest, PlatformBatchRequest, PlaybackHistoryEntry, PlaybackHistoryPeriod,
    PlaybackHistoryRequest, Playlist, PrincipalType, ProviderRegistry, Quality, RadioStation,
    RadioTaxonomy, RadioTaxonomyRequest, RecommendationRequest, ResolveRequest, ResourceRef,
    SearchKind, SearchQuery, StreamResolver, SubscriptionResult, Track, TrackEntitlement,
    TuneWeaveError, User, Video, VideoKind,
};

pub use response::{ApiError, ApiResponse, ResponseMeta};

const AUTH_TRANSACTION_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_AVATAR_UPLOAD_BYTES: usize = 20 * 1024 * 1024;

#[derive(Clone, Default)]
struct AuthTransactions {
    entries: Arc<RwLock<HashMap<String, StoredAuthTransaction>>>,
}

#[derive(Clone)]
struct StoredAuthTransaction {
    expires_at: Instant,
    kind: StoredAuthKind,
}

#[derive(Clone)]
enum StoredAuthKind {
    Qr {
        platform: Platform,
        account: String,
        provider_transaction_id: String,
    },
    Challenge {
        platform: Platform,
        request: AuthChallengeRequest,
    },
}

impl AuthTransactions {
    fn insert(&self, kind: StoredAuthKind) -> Result<String, TuneWeaveError> {
        let mut entries = self.entries.write().map_err(|_| auth_store_error())?;
        let now = Instant::now();
        entries.retain(|_, transaction| transaction.expires_at > now);
        for _ in 0..8 {
            let suffix = rand::rng()
                .sample_iter(Alphanumeric)
                .take(24)
                .map(char::from)
                .collect::<String>();
            let transaction_id = format!("tw-auth-{suffix}");
            if !entries.contains_key(&transaction_id) {
                entries.insert(
                    transaction_id.clone(),
                    StoredAuthTransaction {
                        expires_at: now + AUTH_TRANSACTION_TTL,
                        kind,
                    },
                );
                return Ok(transaction_id);
            }
        }
        Err(TuneWeaveError::new(
            tuneweave_core::ErrorCode::InternalError,
            "failed to allocate a unique authentication transaction",
        ))
    }

    fn get(&self, transaction_id: &str) -> Result<StoredAuthKind, TuneWeaveError> {
        let mut entries = self.entries.write().map_err(|_| auth_store_error())?;
        let now = Instant::now();
        entries.retain(|_, transaction| transaction.expires_at > now);
        entries
            .get(transaction_id)
            .map(|transaction| transaction.kind.clone())
            .ok_or_else(|| {
                TuneWeaveError::new(
                    tuneweave_core::ErrorCode::ResourceNotFound,
                    "authentication transaction was not found or has expired",
                )
            })
    }

    fn remove(&self, transaction_id: &str) -> Result<(), TuneWeaveError> {
        self.entries
            .write()
            .map_err(|_| auth_store_error())?
            .remove(transaction_id);
        Ok(())
    }
}

fn auth_store_error() -> TuneWeaveError {
    TuneWeaveError::new(
        tuneweave_core::ErrorCode::InternalError,
        "authentication transaction store lock is poisoned",
    )
}

#[derive(Clone)]
pub struct AppState {
    registry: ProviderRegistry,
    resolver: StreamResolver,
    auth_transactions: AuthTransactions,
    default_platform: Platform,
    started_at: Instant,
}

impl AppState {
    #[must_use]
    pub fn new(registry: ProviderRegistry, default_platform: Platform) -> Self {
        Self::with_fallbacks(
            registry,
            default_platform,
            vec![
                Platform::Netease,
                Platform::Qq,
                Platform::Kugou,
                Platform::Migu,
            ],
        )
    }

    #[must_use]
    pub fn with_fallbacks(
        registry: ProviderRegistry,
        default_platform: Platform,
        fallback_platforms: Vec<Platform>,
    ) -> Self {
        Self {
            resolver: StreamResolver::new(registry.clone(), fallback_platforms),
            auth_transactions: AuthTransactions::default(),
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
        .route("/banners", get(banners))
        .route("/radio/taxonomy", get(radio_taxonomy))
        .route("/radio/stations/{reference}", get(radio_station))
        .route("/audio/recognize", post(audio_recognize))
        .route("/tracks/{reference}", get(track))
        .route("/albums", get(albums))
        .route("/albums/{reference}", get(album))
        .route("/albums/{reference}/tracks", get(album_tracks))
        .route("/albums/{reference}/stats", get(album_stats))
        .route(
            "/albums/{reference}/track-entitlements",
            get(album_track_entitlements),
        )
        .route("/digital-albums", get(digital_albums))
        .route("/digital-albums/{reference}", get(digital_album))
        .route("/charts/digital-albums", get(digital_album_chart))
        .route("/artists", get(artists))
        .route("/artists/{reference}", get(artist))
        .route("/artists/{reference}/overview", get(artist_overview))
        .route("/artists/{reference}/stats", get(artist_stats))
        .route("/artists/{reference}/albums", get(artist_albums))
        .route("/artists/{reference}/fans", get(artist_fans))
        .route("/artists/{reference}/videos", get(artist_videos))
        .route("/artists/{reference}/tracks", get(artist_tracks))
        .route("/artists/{reference}/top-tracks", get(artist_top_tracks))
        .route(
            "/account/library/albums/{reference}",
            put(album_subscribe).delete(album_unsubscribe),
        )
        .route("/tracks/{reference}/lyrics", get(track_lyrics))
        .route("/tracks/{reference}/stream", get(track_stream))
        .route("/playlists/{reference}", get(playlist))
        .route("/playlists/{reference}/tracks", get(playlist_tracks))
        .route(
            "/users/{reference}/favorites/tracks",
            get(user_favorite_tracks),
        )
        .route("/users/{reference}/history", get(user_history))
        .route("/recommendations/tracks", get(recommended_tracks))
        .route("/recommendations/playlists", get(recommended_playlists))
        .route("/auth/qr", post(auth_qr_start))
        .route("/auth/qr/{transaction_id}", get(auth_qr_poll))
        .route("/auth/password", post(auth_password))
        .route("/auth/challenges", post(auth_challenge_start))
        .route(
            "/auth/challenges/{transaction_id}/verify",
            post(auth_challenge_verify),
        )
        .route(
            "/auth/session",
            get(auth_session_get).delete(auth_session_delete),
        )
        .route("/auth/session/refresh", post(auth_session_refresh))
        .route("/account", get(account_profile))
        .route(
            "/account/avatar",
            put(account_avatar).layer(DefaultBodyLimit::max(MAX_AVATAR_UPLOAD_BYTES)),
        )
        .route("/account/playlists", get(account_playlists))
        .route("/account/library/albums", get(account_albums))
        .route(
            "/account/library/radio-stations",
            get(account_radio_stations),
        )
        .route("/account/following/artists", get(account_following_artists))
        .route(
            "/account/following/artists/{reference}",
            put(artist_subscribe).delete(artist_unsubscribe),
        )
        .route(
            "/account/following/artists/new-videos",
            get(account_artist_new_videos),
        )
        .route(
            "/account/following/artists/new-tracks",
            get(account_artist_new_tracks),
        )
        .route(
            "/account/following/artists/new-works",
            get(account_artist_new_works),
        )
        .route(
            "/account/following/artists/new-tracks/play-all",
            get(account_artist_new_tracks_play_all),
        )
        .route("/account/favorites/tracks", get(account_favorite_tracks))
        .route("/account/history", get(account_history))
        .route("/extensions/netease/api", post(netease_extension_api))
        .route(
            "/extensions/netease/batch",
            get(netease_extension_batch_get).post(netease_extension_batch_post),
        );

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
struct BannerParams {
    platform: Option<String>,
    account: Option<String>,
    #[serde(alias = "type")]
    client: Option<String>,
}

async fn banners(
    State(state): State<AppState>,
    Query(params): Query<BannerParams>,
) -> Result<Json<ApiResponse<Vec<Banner>>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = optional_trimmed(params.account);
    let provider = state.registry.require(platform)?;
    let mut request = BannerListRequest::new(parse_banner_client(params.client.as_deref())?);
    request.account.clone_from(&account);
    let banners = provider.banners(&request).await?;
    let mut response = ApiResponse::new(banners).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

#[derive(Debug, Default, Deserialize)]
struct RadioTaxonomyParams {
    platform: Option<String>,
    account: Option<String>,
}

async fn radio_taxonomy(
    State(state): State<AppState>,
    Query(params): Query<RadioTaxonomyParams>,
) -> Result<Json<ApiResponse<RadioTaxonomy>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = optional_trimmed(params.account);
    let provider = state.registry.require(platform)?;
    let taxonomy = provider
        .radio_taxonomy(&RadioTaxonomyRequest {
            account: account.clone(),
        })
        .await?;
    let mut response = ApiResponse::new(taxonomy).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

async fn radio_station(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<RadioStation>>, ApiError> {
    let reference = parse_reference(reference)?;
    let account = params
        .account
        .as_deref()
        .map(str::trim)
        .filter(|account| !account.is_empty());
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let station = provider.radio_station(reference.id(), account).await?;
    let mut response = ApiResponse::new(station).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct AudioRecognizeBody {
    platform: Option<String>,
    account: Option<String>,
    #[serde(alias = "audio_fp", alias = "audioFP")]
    fingerprint: String,
    #[serde(alias = "duration")]
    duration_seconds: u32,
}

async fn audio_recognize(
    State(state): State<AppState>,
    payload: Result<Json<AudioRecognizeBody>, JsonRejection>,
) -> Result<Json<ApiResponse<AudioRecognition>>, ApiError> {
    let body = json_body(payload)?;
    let fingerprint = body.fingerprint.trim();
    if fingerprint.is_empty() {
        return Err(TuneWeaveError::invalid_request("fingerprint must not be empty").into());
    }
    if fingerprint.len() > 131_072 {
        return Err(
            TuneWeaveError::invalid_request("fingerprint cannot exceed 131072 bytes").into(),
        );
    }
    if !(1..=300).contains(&body.duration_seconds) {
        return Err(
            TuneWeaveError::invalid_request("duration_seconds must be between 1 and 300").into(),
        );
    }
    let platform = account_platform(&state, body.platform.as_deref())?;
    let account = optional_trimmed(body.account);
    let provider = state.registry.require(platform)?;
    let recognition = provider
        .recognize_audio(&AudioRecognitionRequest {
            fingerprint: fingerprint.to_owned(),
            duration_seconds: body.duration_seconds,
            account: account.clone(),
        })
        .await?;
    let mut response = ApiResponse::new(recognition).with_platform(platform);
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

#[derive(Debug, Default, Deserialize)]
struct AlbumListParams {
    platform: Option<String>,
    account: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
    area: Option<String>,
    catalog: Option<String>,
}

async fn albums(
    State(state): State<AppState>,
    Query(params): Query<AlbumListParams>,
) -> Result<Json<ApiResponse<Vec<Album>>>, ApiError> {
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = optional_trimmed(params.account);
    let provider = state.registry.require(platform)?;
    let mut request = AlbumListRequest::new(limit, offset);
    request.account.clone_from(&account);
    request.area = optional_trimmed(params.area);
    request.catalog = optional_trimmed(params.catalog);
    let page = provider.albums(&request).await?;
    let mut response = ApiResponse::new(page.items)
        .with_platform(platform)
        .with_pagination(page.pagination);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

async fn album(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<Album>>, ApiError> {
    let reference = parse_reference(reference)?;
    let account = params
        .account
        .as_deref()
        .map(str::trim)
        .filter(|account| !account.is_empty());
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let album = provider.album(reference.id(), account).await?;
    let mut response = ApiResponse::new(album).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

async fn album_tracks(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<PageParams>,
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
        .album_tracks(
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

async fn album_stats(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<AlbumStats>>, ApiError> {
    let reference = parse_reference(reference)?;
    let account = params
        .account
        .as_deref()
        .map(str::trim)
        .filter(|account| !account.is_empty());
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let stats = provider.album_stats(reference.id(), account).await?;
    let mut response = ApiResponse::new(stats).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

async fn album_track_entitlements(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<PageParams>,
) -> Result<Json<ApiResponse<Vec<TrackEntitlement>>>, ApiError> {
    let reference = parse_reference(reference)?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let account = optional_trimmed(params.account);
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let page = provider
        .album_track_entitlements(
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

async fn digital_album(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<DigitalAlbum>>, ApiError> {
    let reference = parse_reference(reference)?;
    let account = params
        .account
        .as_deref()
        .map(str::trim)
        .filter(|account| !account.is_empty());
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let album = provider.digital_album(reference.id(), account).await?;
    let mut response = ApiResponse::new(album).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

#[derive(Debug, Default, Deserialize)]
struct DigitalAlbumListParams {
    platform: Option<String>,
    account: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
    area: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    catalog: Option<String>,
}

async fn digital_albums(
    State(state): State<AppState>,
    Query(params): Query<DigitalAlbumListParams>,
) -> Result<Json<ApiResponse<Vec<DigitalAlbum>>>, ApiError> {
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = optional_trimmed(params.account);
    let provider = state.registry.require(platform)?;
    let mut request = DigitalAlbumListRequest::new(limit, offset);
    request.account.clone_from(&account);
    request.area = optional_trimmed(params.area);
    request.kind = optional_trimmed(params.kind);
    request.catalog = optional_trimmed(params.catalog);
    let page = provider.digital_albums(&request).await?;
    let mut response = ApiResponse::new(page.items)
        .with_platform(platform)
        .with_pagination(page.pagination);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

#[derive(Debug, Default, Deserialize)]
struct DigitalAlbumChartParams {
    platform: Option<String>,
    account: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
    period: Option<String>,
    #[serde(alias = "type")]
    kind: Option<String>,
    year: Option<String>,
}

async fn digital_album_chart(
    State(state): State<AppState>,
    Query(params): Query<DigitalAlbumChartParams>,
) -> Result<Json<ApiResponse<Vec<DigitalAlbumChartEntry>>>, ApiError> {
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 20)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let period = parse_digital_album_chart_period(params.period.as_deref())?;
    let kind = parse_digital_album_chart_kind(params.kind.as_deref())?;
    let year = parse_optional_u16_parameter("year", params.year.as_deref())?;
    if year.is_some() && period != DigitalAlbumChartPeriod::Year {
        return Err(
            TuneWeaveError::invalid_request("year is only supported when period=year").into(),
        );
    }
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = optional_trimmed(params.account);
    let provider = state.registry.require(platform)?;
    let page = provider
        .digital_album_chart(&DigitalAlbumChartRequest {
            limit,
            offset,
            account: account.clone(),
            period,
            kind,
            year,
        })
        .await?;
    let mut response = ApiResponse::new(page.items)
        .with_platform(platform)
        .with_pagination(page.pagination);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

async fn album_subscribe(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<SubscriptionResult>>, ApiError> {
    set_album_subscription(state, reference, params, true).await
}

async fn album_unsubscribe(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<SubscriptionResult>>, ApiError> {
    set_album_subscription(state, reference, params, false).await
}

async fn set_album_subscription(
    state: AppState,
    reference: String,
    params: AccountParams,
    subscribed: bool,
) -> Result<Json<ApiResponse<SubscriptionResult>>, ApiError> {
    let reference = parse_reference(reference)?;
    let platform = reference.platform();
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .set_album_subscription(reference.id(), subscribed, Some(&account))
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn artist_subscribe(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<SubscriptionResult>>, ApiError> {
    set_artist_subscription(state, reference, params, true).await
}

async fn artist_unsubscribe(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<SubscriptionResult>>, ApiError> {
    set_artist_subscription(state, reference, params, false).await
}

async fn set_artist_subscription(
    state: AppState,
    reference: String,
    params: AccountParams,
    subscribed: bool,
) -> Result<Json<ApiResponse<SubscriptionResult>>, ApiError> {
    let reference = parse_reference(reference)?;
    let platform = reference.platform();
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .set_artist_subscription(reference.id(), subscribed, Some(&account))
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
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

#[derive(Debug, Default, Deserialize)]
struct StreamParams {
    quality: Option<String>,
    playback_platform: Option<String>,
    fallback: Option<String>,
    fallback_platforms: Option<String>,
    account: Option<String>,
}

async fn track_stream(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<StreamParams>,
) -> Result<Json<ApiResponse<MediaStream>>, ApiError> {
    let reference = parse_reference(reference)?;
    let quality = parse_quality(params.quality.as_deref())?;
    let fallback = parse_bool_parameter("fallback", params.fallback.as_deref(), true)?;
    let preferred_platform = params
        .playback_platform
        .as_deref()
        .map(parse_platform_parameter)
        .transpose()?;
    let fallback_platforms = parse_platform_list(params.fallback_platforms.as_deref())?;
    let mut playback_platforms = Vec::new();
    if let Some(platform) = preferred_platform {
        playback_platforms.push(platform);
    } else if !fallback_platforms.is_empty() {
        playback_platforms.push(reference.platform());
    }
    playback_platforms.extend(fallback_platforms);
    let account = params
        .account
        .as_deref()
        .map(str::trim)
        .filter(|account| !account.is_empty())
        .map(str::to_owned);
    let account_platform = preferred_platform.unwrap_or(reference.platform());
    let mut request = ResolveRequest {
        quality,
        playback_platforms,
        fallback,
        ..ResolveRequest::default()
    };
    if let Some(account) = account.clone() {
        request.accounts.insert(account_platform, account);
    }

    let origin_provider = state.registry.require(reference.platform())?;
    let origin = origin_provider.track(reference.id(), None).await?;
    let stream = state.resolver.resolve(&origin, &request).await?;
    let resolved_platform = stream.resolved_platform;
    let mut response = ApiResponse::new(stream).with_platform(resolved_platform);
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
struct PageParams {
    limit: Option<String>,
    offset: Option<String>,
    account: Option<String>,
}

async fn playlist_tracks(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<PageParams>,
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

#[derive(Debug, Default, Deserialize)]
struct ArtistListParams {
    platform: Option<String>,
    account: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
    #[serde(rename = "type", alias = "category")]
    category: Option<String>,
    area: Option<String>,
    initial: Option<String>,
}

async fn artists(
    State(state): State<AppState>,
    Query(params): Query<ArtistListParams>,
) -> Result<Json<ApiResponse<Vec<Artist>>>, ApiError> {
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = optional_trimmed(params.account);
    let provider = state.registry.require(platform)?;
    let mut request = ArtistListRequest::new(limit, offset);
    request.account.clone_from(&account);
    request.category = parse_artist_category(params.category.as_deref())?;
    request.area = parse_artist_area(params.area.as_deref())?;
    request.initial = optional_trimmed(params.initial);
    let page = provider.artists(&request).await?;
    let mut response = ApiResponse::new(page.items)
        .with_platform(platform)
        .with_pagination(page.pagination);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

async fn artist(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<Artist>>, ApiError> {
    let reference = parse_reference(reference)?;
    let account = params
        .account
        .as_deref()
        .map(str::trim)
        .filter(|account| !account.is_empty());
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let artist = provider.artist(reference.id(), account).await?;
    let mut response = ApiResponse::new(artist).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

async fn artist_overview(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<ArtistOverview>>, ApiError> {
    let reference = parse_reference(reference)?;
    let account = optional_trimmed(params.account);
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let overview = provider
        .artist_overview(reference.id(), account.as_deref())
        .await?;
    let mut response = ApiResponse::new(overview).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

async fn artist_stats(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<ArtistStats>>, ApiError> {
    let reference = parse_reference(reference)?;
    let account = params
        .account
        .as_deref()
        .map(str::trim)
        .filter(|account| !account.is_empty());
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let stats = provider.artist_stats(reference.id(), account).await?;
    let mut response = ApiResponse::new(stats).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

async fn artist_albums(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<PageParams>,
) -> Result<Json<ApiResponse<Vec<Album>>>, ApiError> {
    let reference = parse_reference(reference)?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let account = optional_trimmed(params.account);
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let page = provider
        .artist_albums(
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

async fn artist_fans(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<PageParams>,
) -> Result<Json<ApiResponse<Vec<User>>>, ApiError> {
    let reference = parse_reference(reference)?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 20)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let account = optional_trimmed(params.account);
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let page = provider
        .artist_fans(
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

#[derive(Debug, Default, Deserialize)]
struct ArtistVideoListParams {
    limit: Option<String>,
    offset: Option<String>,
    cursor: Option<String>,
    account: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    order: Option<String>,
}

async fn artist_videos(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<ArtistVideoListParams>,
) -> Result<Json<ApiResponse<Vec<Video>>>, ApiError> {
    let reference = parse_reference(reference)?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let account = optional_trimmed(params.account);
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let mut request = ArtistVideoListRequest::new(limit, offset);
    request.account.clone_from(&account);
    request.cursor = optional_trimmed(params.cursor);
    request.kind = parse_video_kind(params.kind.as_deref())?;
    request.order = optional_trimmed(params.order);
    let page = provider.artist_videos(reference.id(), &request).await?;
    let mut response = ApiResponse::new(page.items)
        .with_platform(platform)
        .with_pagination(page.pagination);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

#[derive(Debug, Default, Deserialize)]
struct ArtistTrackListParams {
    limit: Option<String>,
    offset: Option<String>,
    account: Option<String>,
    order: Option<String>,
}

async fn artist_tracks(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<ArtistTrackListParams>,
) -> Result<Json<ApiResponse<Vec<Track>>>, ApiError> {
    let reference = parse_reference(reference)?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 100)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let account = optional_trimmed(params.account);
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let mut request = ArtistTrackListRequest::new(limit, offset);
    request.account.clone_from(&account);
    request.order = parse_artist_track_order(params.order.as_deref())?;
    let page = provider.artist_tracks(reference.id(), &request).await?;
    let mut response = ApiResponse::new(page.items)
        .with_platform(platform)
        .with_pagination(page.pagination);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

async fn artist_top_tracks(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<Vec<Track>>>, ApiError> {
    let reference = parse_reference(reference)?;
    let account = optional_trimmed(params.account);
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let page = provider
        .artist_top_tracks(reference.id(), account.as_deref())
        .await?;
    let mut response = ApiResponse::new(page.items)
        .with_platform(platform)
        .with_pagination(page.pagination);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

async fn user_favorite_tracks(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<PageParams>,
) -> Result<Json<ApiResponse<Vec<Track>>>, ApiError> {
    let reference = parse_reference(reference)?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let account = account_alias(params.account.as_deref())?;
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let page = provider
        .user_favorite_tracks(
            reference.id(),
            &PageRequest {
                limit,
                offset,
                account: Some(account.clone()),
            },
        )
        .await?;
    Ok(Json(
        ApiResponse::new(page.items)
            .with_platform(platform)
            .with_account(account)
            .with_pagination(page.pagination),
    ))
}

#[derive(Debug, Default, Deserialize)]
struct HistoryParams {
    period: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
    account: Option<String>,
}

async fn user_history(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<HistoryParams>,
) -> Result<Json<ApiResponse<Vec<PlaybackHistoryEntry>>>, ApiError> {
    let reference = parse_reference(reference)?;
    let period = parse_history_period(params.period.as_deref())?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let account = account_alias(params.account.as_deref())?;
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let page = provider
        .user_history(
            reference.id(),
            &PlaybackHistoryRequest {
                period,
                limit,
                offset,
                account: Some(account.clone()),
            },
        )
        .await?;
    Ok(Json(
        ApiResponse::new(page.items)
            .with_platform(platform)
            .with_account(account)
            .with_pagination(page.pagination),
    ))
}

#[derive(Debug, Default, Deserialize)]
struct RecommendationParams {
    platform: Option<String>,
    account: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
    refresh: Option<String>,
}

async fn recommended_tracks(
    State(state): State<AppState>,
    Query(params): Query<RecommendationParams>,
) -> Result<Json<ApiResponse<Vec<Track>>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let request = recommendation_request(&params, account.clone())?;
    let provider = state.registry.require(platform)?;
    let page = provider.recommended_tracks(&request).await?;
    Ok(Json(
        ApiResponse::new(page.items)
            .with_platform(platform)
            .with_account(account)
            .with_pagination(page.pagination),
    ))
}

async fn recommended_playlists(
    State(state): State<AppState>,
    Query(params): Query<RecommendationParams>,
) -> Result<Json<ApiResponse<Vec<Playlist>>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let request = recommendation_request(&params, account.clone())?;
    let provider = state.registry.require(platform)?;
    let page = provider.recommended_playlists(&request).await?;
    Ok(Json(
        ApiResponse::new(page.items)
            .with_platform(platform)
            .with_account(account)
            .with_pagination(page.pagination),
    ))
}

fn recommendation_request(
    params: &RecommendationParams,
    account: String,
) -> Result<RecommendationRequest, TuneWeaveError> {
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request(
            "limit must be between 1 and 100",
        ));
    }
    Ok(RecommendationRequest {
        limit,
        offset: parse_u32_parameter("offset", params.offset.as_deref(), 0)?,
        account: Some(account),
        refresh: parse_bool_parameter("refresh", params.refresh.as_deref(), false)?,
    })
}

#[derive(Deserialize)]
struct AuthQrStartBody {
    platform: String,
    account: Option<String>,
    login_type: Option<String>,
}

#[derive(Serialize)]
struct AuthQrStartData {
    transaction_id: String,
    url: String,
    image_data_url: Option<String>,
    expires_at: Option<String>,
}

async fn auth_qr_start(
    State(state): State<AppState>,
    payload: Result<Json<AuthQrStartBody>, JsonRejection>,
) -> Result<Json<ApiResponse<AuthQrStartData>>, ApiError> {
    let body = json_body(payload)?;
    let platform = parse_platform_parameter(&body.platform)?;
    let account = account_alias(body.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let start = provider.start_qr_login(body.login_type.as_deref()).await?;
    let transaction_id = state.auth_transactions.insert(StoredAuthKind::Qr {
        platform,
        account: account.clone(),
        provider_transaction_id: start.provider_transaction_id,
    })?;
    let data = AuthQrStartData {
        transaction_id,
        url: start.url,
        image_data_url: start.image_data_url,
        expires_at: start.expires_at,
    };
    Ok(Json(
        ApiResponse::new(data)
            .with_platform(platform)
            .with_account(account),
    ))
}

#[derive(Serialize)]
struct AuthQrPollData {
    transaction_id: String,
    state: AuthState,
    message: Option<String>,
    profile: Option<AccountProfile>,
}

async fn auth_qr_poll(
    State(state): State<AppState>,
    Path(transaction_id): Path<String>,
) -> Result<Json<ApiResponse<AuthQrPollData>>, ApiError> {
    let stored = state.auth_transactions.get(&transaction_id)?;
    let StoredAuthKind::Qr {
        platform,
        account,
        provider_transaction_id,
    } = stored
    else {
        return Err(auth_transaction_not_found().into());
    };
    let provider = state.registry.require(platform)?;
    let poll = provider
        .poll_qr_login(&provider_transaction_id, &account)
        .await?;
    if poll.state.is_terminal() {
        state.auth_transactions.remove(&transaction_id)?;
    }
    let data = AuthQrPollData {
        transaction_id,
        state: poll.state,
        message: poll.message,
        profile: poll.profile,
    };
    Ok(Json(
        ApiResponse::new(data)
            .with_platform(platform)
            .with_account(account),
    ))
}

#[derive(Deserialize)]
struct AuthPasswordBody {
    platform: String,
    account: Option<String>,
    principal_type: PrincipalType,
    principal: String,
    password: String,
    #[serde(default)]
    password_format: PasswordFormat,
    country_code: Option<String>,
}

async fn auth_password(
    State(state): State<AppState>,
    payload: Result<Json<AuthPasswordBody>, JsonRejection>,
) -> Result<Json<ApiResponse<AccountProfile>>, ApiError> {
    let body = json_body(payload)?;
    let platform = parse_platform_parameter(&body.platform)?;
    let account = account_alias(body.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let profile = provider
        .password_login(&PasswordLoginRequest {
            account: account.clone(),
            principal_type: body.principal_type,
            principal: body.principal,
            password: body.password,
            password_format: body.password_format,
            country_code: optional_trimmed(body.country_code),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(profile)
            .with_platform(platform)
            .with_account(account),
    ))
}

#[derive(Deserialize)]
struct AuthChallengeStartBody {
    platform: String,
    account: Option<String>,
    method: ChallengeMethod,
    principal: String,
    country_code: Option<String>,
}

#[derive(Serialize)]
struct AuthChallengeStartData {
    transaction_id: String,
    method: ChallengeMethod,
}

async fn auth_challenge_start(
    State(state): State<AppState>,
    payload: Result<Json<AuthChallengeStartBody>, JsonRejection>,
) -> Result<Json<ApiResponse<AuthChallengeStartData>>, ApiError> {
    let body = json_body(payload)?;
    let platform = parse_platform_parameter(&body.platform)?;
    let account = account_alias(body.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let request = AuthChallengeRequest {
        account: account.clone(),
        method: body.method,
        principal: body.principal,
        country_code: optional_trimmed(body.country_code),
    };
    provider.start_auth_challenge(&request).await?;
    let transaction_id = state
        .auth_transactions
        .insert(StoredAuthKind::Challenge { platform, request })?;
    let data = AuthChallengeStartData {
        transaction_id,
        method: body.method,
    };
    Ok(Json(
        ApiResponse::new(data)
            .with_platform(platform)
            .with_account(account),
    ))
}

#[derive(Deserialize)]
struct AuthChallengeVerifyBody {
    code: String,
}

#[derive(Serialize)]
struct AuthChallengeVerifyData {
    state: AuthState,
    profile: AccountProfile,
}

async fn auth_challenge_verify(
    State(state): State<AppState>,
    Path(transaction_id): Path<String>,
    payload: Result<Json<AuthChallengeVerifyBody>, JsonRejection>,
) -> Result<Json<ApiResponse<AuthChallengeVerifyData>>, ApiError> {
    let body = json_body(payload)?;
    if body.code.trim().is_empty() {
        return Err(TuneWeaveError::invalid_request("code must not be empty").into());
    }
    let stored = state.auth_transactions.get(&transaction_id)?;
    let StoredAuthKind::Challenge { platform, request } = stored else {
        return Err(auth_transaction_not_found().into());
    };
    let provider = state.registry.require(platform)?;
    let profile = provider
        .verify_auth_challenge(&request, body.code.trim())
        .await?;
    state.auth_transactions.remove(&transaction_id)?;
    let account = request.account;
    Ok(Json(
        ApiResponse::new(AuthChallengeVerifyData {
            state: AuthState::Confirmed,
            profile,
        })
        .with_platform(platform)
        .with_account(account),
    ))
}

#[derive(Deserialize)]
struct AuthSessionParams {
    platform: String,
    account: Option<String>,
}

async fn auth_session_get(
    State(state): State<AppState>,
    Query(params): Query<AuthSessionParams>,
) -> Result<Json<ApiResponse<AccountProfile>>, ApiError> {
    let platform = parse_platform_parameter(&params.platform)?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let profile = provider.session_profile(&account).await?;
    Ok(Json(
        ApiResponse::new(profile)
            .with_platform(platform)
            .with_account(account),
    ))
}

#[derive(Deserialize)]
struct AuthSessionBody {
    platform: String,
    account: Option<String>,
}

async fn auth_session_refresh(
    State(state): State<AppState>,
    payload: Result<Json<AuthSessionBody>, JsonRejection>,
) -> Result<Json<ApiResponse<AccountProfile>>, ApiError> {
    let body = json_body(payload)?;
    let platform = parse_platform_parameter(&body.platform)?;
    let account = account_alias(body.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let profile = provider.refresh_session(&account).await?;
    Ok(Json(
        ApiResponse::new(profile)
            .with_platform(platform)
            .with_account(account),
    ))
}

#[derive(Serialize)]
struct AuthSessionDeleteData {
    removed: bool,
}

async fn auth_session_delete(
    State(state): State<AppState>,
    Query(params): Query<AuthSessionParams>,
) -> Result<Json<ApiResponse<AuthSessionDeleteData>>, ApiError> {
    let platform = parse_platform_parameter(&params.platform)?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let removed = provider.logout(&account).await?;
    Ok(Json(
        ApiResponse::new(AuthSessionDeleteData { removed })
            .with_platform(platform)
            .with_account(account),
    ))
}

#[derive(Default, Deserialize)]
struct AccountQuery {
    platform: Option<String>,
    account: Option<String>,
    period: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
}

#[derive(Default, Deserialize)]
struct AvatarUploadParams {
    platform: Option<String>,
    account: Option<String>,
    filename: Option<String>,
    #[serde(alias = "imgSize", alias = "img_size")]
    image_size: Option<String>,
    #[serde(alias = "imgX", alias = "img_x")]
    crop_x: Option<String>,
    #[serde(alias = "imgY", alias = "img_y")]
    crop_y: Option<String>,
}

async fn account_avatar(
    State(state): State<AppState>,
    Query(params): Query<AvatarUploadParams>,
    headers: HeaderMap,
    payload: Result<Bytes, BytesRejection>,
) -> Result<Json<ApiResponse<ImageUploadResult>>, ApiError> {
    let data = payload.map_err(|_| {
        TuneWeaveError::invalid_request("image body is invalid or exceeds 20 MiB")
            .with_details(json!({ "max_bytes": MAX_AVATAR_UPLOAD_BYTES }))
    })?;
    if data.is_empty() {
        return Err(TuneWeaveError::invalid_request("image body must not be empty").into());
    }
    if data.len() > MAX_AVATAR_UPLOAD_BYTES {
        return Err(TuneWeaveError::invalid_request("image body exceeds 20 MiB")
            .with_details(json!({ "max_bytes": MAX_AVATAR_UPLOAD_BYTES }))
            .into());
    }
    let filename = params.filename.as_deref().unwrap_or("avatar.jpg").trim();
    if filename.is_empty() || filename.len() > 255 || filename.chars().any(char::is_control) {
        return Err(TuneWeaveError::invalid_request(
            "filename must be 1 to 255 bytes and contain no control characters",
        )
        .into());
    }
    let content_type = match headers.get(header::CONTENT_TYPE) {
        Some(value) => value
            .to_str()
            .map_err(|_| TuneWeaveError::invalid_request("Content-Type is not valid text"))?
            .trim(),
        None => "image/jpeg",
    };
    let media_type = content_type
        .split(';')
        .next()
        .map(str::trim)
        .unwrap_or_default();
    if !media_type.to_ascii_lowercase().starts_with("image/") || media_type.len() <= 6 {
        return Err(
            TuneWeaveError::invalid_request("Content-Type must use the image media type").into(),
        );
    }
    let image_size = parse_optional_u32_parameter("image_size", params.image_size.as_deref())?;
    if image_size == Some(0) {
        return Err(TuneWeaveError::invalid_request("image_size must be greater than zero").into());
    }
    let crop_x = parse_optional_u32_parameter("crop_x", params.crop_x.as_deref())?;
    let crop_y = parse_optional_u32_parameter("crop_y", params.crop_y.as_deref())?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .upload_account_avatar(&ImageUploadRequest {
            filename: filename.to_owned(),
            content_type: content_type.to_owned(),
            data: data.to_vec(),
            image_size,
            crop_x,
            crop_y,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn account_profile(
    State(state): State<AppState>,
    Query(params): Query<AccountQuery>,
) -> Result<Json<ApiResponse<AccountProfile>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let profile = provider.session_profile(&account).await?;
    if !profile.authenticated {
        return Err(TuneWeaveError::new(
            tuneweave_core::ErrorCode::AuthenticationRequired,
            format!("{platform} account alias {account} is not logged in"),
        )
        .with_platform(platform)
        .with_details(json!({ "account": account }))
        .into());
    }
    Ok(Json(
        ApiResponse::new(profile)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn account_playlists(
    State(state): State<AppState>,
    Query(params): Query<AccountQuery>,
) -> Result<Json<ApiResponse<Vec<Playlist>>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let provider = state.registry.require(platform)?;
    let page = provider
        .account_playlists(&PageRequest {
            limit,
            offset,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(page.items)
            .with_platform(platform)
            .with_account(account)
            .with_pagination(page.pagination),
    ))
}

async fn account_albums(
    State(state): State<AppState>,
    Query(params): Query<AccountQuery>,
) -> Result<Json<ApiResponse<Vec<Album>>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 25)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let provider = state.registry.require(platform)?;
    let page = provider
        .account_albums(&PageRequest {
            limit,
            offset,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(page.items)
            .with_platform(platform)
            .with_account(account)
            .with_pagination(page.pagination),
    ))
}

async fn account_radio_stations(
    State(state): State<AppState>,
    Query(params): Query<AccountQuery>,
) -> Result<Json<ApiResponse<Vec<RadioStation>>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 25)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let provider = state.registry.require(platform)?;
    let page = provider
        .account_radio_stations(&PageRequest {
            limit,
            offset,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(page.items)
            .with_platform(platform)
            .with_account(account)
            .with_pagination(page.pagination),
    ))
}

async fn account_following_artists(
    State(state): State<AppState>,
    Query(params): Query<AccountQuery>,
) -> Result<Json<ApiResponse<Vec<Artist>>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 25)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let provider = state.registry.require(platform)?;
    let page = provider
        .account_following_artists(&PageRequest {
            limit,
            offset,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(page.items)
            .with_platform(platform)
            .with_account(account)
            .with_pagination(page.pagination),
    ))
}

#[derive(Default, Deserialize)]
struct ArtistUpdatesParams {
    platform: Option<String>,
    account: Option<String>,
    limit: Option<String>,
    before: Option<String>,
    source_type: Option<String>,
    first_request: Option<String>,
}

async fn account_artist_new_videos(
    State(state): State<AppState>,
    Query(params): Query<ArtistUpdatesParams>,
) -> Result<Json<ApiResponse<Vec<Video>>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 20)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let provider = state.registry.require(platform)?;
    let page = provider
        .account_artist_new_videos(&ArtistUpdatesRequest {
            limit,
            before_ms: parse_optional_u64_parameter("before", params.before.as_deref())?,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(page.items)
            .with_platform(platform)
            .with_account(account)
            .with_pagination(page.pagination),
    ))
}

async fn account_artist_new_tracks(
    State(state): State<AppState>,
    Query(params): Query<ArtistUpdatesParams>,
) -> Result<Json<ApiResponse<Vec<Track>>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 20)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let provider = state.registry.require(platform)?;
    let page = provider
        .account_artist_new_tracks(&ArtistUpdatesRequest {
            limit,
            before_ms: parse_optional_u64_parameter("before", params.before.as_deref())?,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(page.items)
            .with_platform(platform)
            .with_account(account)
            .with_pagination(page.pagination),
    ))
}

async fn account_artist_new_works(
    State(state): State<AppState>,
    Query(params): Query<ArtistUpdatesParams>,
) -> Result<Json<ApiResponse<Vec<ArtistWorkUpdate>>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 10)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let provider = state.registry.require(platform)?;
    let page = provider
        .account_artist_new_works(&ArtistWorksRequest {
            limit,
            before_ms: parse_optional_u64_parameter("before", params.before.as_deref())?,
            source_type: parse_u32_parameter("source_type", params.source_type.as_deref(), 1)?,
            first_request: parse_bool_parameter(
                "first_request",
                params.first_request.as_deref(),
                true,
            )?,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(page.items)
            .with_platform(platform)
            .with_account(account)
            .with_pagination(page.pagination),
    ))
}

async fn account_artist_new_tracks_play_all(
    State(state): State<AppState>,
    Query(params): Query<AccountQuery>,
) -> Result<Json<ApiResponse<Vec<Track>>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let page = provider
        .account_artist_new_tracks_play_all(Some(&account))
        .await?;
    Ok(Json(
        ApiResponse::new(page.items)
            .with_platform(platform)
            .with_account(account)
            .with_pagination(page.pagination),
    ))
}

async fn account_favorite_tracks(
    State(state): State<AppState>,
    Query(params): Query<AccountQuery>,
) -> Result<Json<ApiResponse<Vec<Track>>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let provider = state.registry.require(platform)?;
    let page = provider
        .favorite_tracks(&PageRequest {
            limit,
            offset,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(page.items)
            .with_platform(platform)
            .with_account(account)
            .with_pagination(page.pagination),
    ))
}

async fn account_history(
    State(state): State<AppState>,
    Query(params): Query<AccountQuery>,
) -> Result<Json<ApiResponse<Vec<PlaybackHistoryEntry>>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let period = parse_history_period(params.period.as_deref())?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let provider = state.registry.require(platform)?;
    let page = provider
        .account_history(&PlaybackHistoryRequest {
            period,
            limit,
            offset,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(page.items)
            .with_platform(platform)
            .with_account(account)
            .with_pagination(page.pagination),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct NeteaseExtensionApiBody {
    uri: String,
    #[serde(default = "empty_json_object")]
    data: Value,
    #[serde(default, alias = "protocol")]
    crypto: Option<String>,
    account: Option<String>,
}

async fn netease_extension_api(
    State(state): State<AppState>,
    payload: Result<Json<NeteaseExtensionApiBody>, JsonRejection>,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    let body = json_body(payload)?;
    let account = optional_trimmed(body.account);
    let protocol = optional_trimmed(body.crypto);
    let provider = state.registry.require(Platform::Netease)?;
    let data = provider
        .platform_api(&PlatformApiRequest {
            uri: body.uri,
            data: body.data,
            protocol,
            account: account.clone(),
        })
        .await?;
    let mut response = ApiResponse::new(data).with_platform(Platform::Netease);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

#[derive(Debug, Default, Deserialize)]
struct NeteaseExtensionBatchBody {
    #[serde(default)]
    requests: BTreeMap<String, Value>,
    #[serde(default, alias = "protocol")]
    crypto: Option<String>,
    #[serde(default, alias = "e_r")]
    encrypted_response: Option<Value>,
    account: Option<String>,
    #[serde(flatten)]
    direct_requests: BTreeMap<String, Value>,
}

struct NeteaseBatchInput {
    requests: BTreeMap<String, Value>,
    protocol: Option<String>,
    encrypted_response: Option<Value>,
    account: Option<String>,
}

async fn netease_extension_batch_post(
    State(state): State<AppState>,
    payload: Result<Json<NeteaseExtensionBatchBody>, JsonRejection>,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    let body = json_body(payload)?;
    let requests = merge_netease_batch_requests(body.requests, body.direct_requests)?;
    execute_netease_batch(
        &state,
        NeteaseBatchInput {
            requests,
            protocol: body.crypto,
            encrypted_response: body.encrypted_response,
            account: body.account,
        },
    )
    .await
}

async fn netease_extension_batch_get(
    State(state): State<AppState>,
    Query(mut query): Query<BTreeMap<String, String>>,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    let protocol = take_netease_batch_alias(&mut query, "crypto", "protocol")?;
    let encrypted_response =
        take_netease_batch_alias(&mut query, "e_r", "encrypted_response")?.map(Value::String);
    let account = query.remove("account");
    let requests = query
        .remove("requests")
        .map(|requests| parse_netease_batch_requests_json(&requests))
        .transpose()?
        .unwrap_or_default();
    let direct_requests = query
        .into_iter()
        .map(|(uri, data)| (uri, Value::String(data)))
        .collect();
    let requests = merge_netease_batch_requests(requests, direct_requests)?;
    execute_netease_batch(
        &state,
        NeteaseBatchInput {
            requests,
            protocol,
            encrypted_response,
            account,
        },
    )
    .await
}

async fn execute_netease_batch(
    state: &AppState,
    input: NeteaseBatchInput,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    let account = optional_trimmed(input.account);
    let protocol = optional_trimmed(input.protocol);
    let encrypted_response = parse_json_bool("encrypted_response", input.encrypted_response)?;
    let provider = state.registry.require(Platform::Netease)?;
    let data = provider
        .platform_batch(&PlatformBatchRequest {
            requests: input.requests,
            protocol,
            encrypted_response,
            account: account.clone(),
        })
        .await?;
    let mut response = ApiResponse::new(data).with_platform(Platform::Netease);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

fn merge_netease_batch_requests(
    mut requests: BTreeMap<String, Value>,
    direct_requests: BTreeMap<String, Value>,
) -> Result<BTreeMap<String, Value>, TuneWeaveError> {
    for uri in requests.keys() {
        if !uri.starts_with("/api/") {
            return Err(TuneWeaveError::invalid_request(format!(
                "unsupported NetEase batch request: {uri}"
            ))
            .with_details(json!({ "uri": uri })));
        }
    }
    for (uri, data) in direct_requests {
        if !uri.starts_with("/api/") {
            return Err(TuneWeaveError::invalid_request(format!(
                "unsupported NetEase batch field: {uri}"
            ))
            .with_details(json!({ "field": uri })));
        }
        if requests.insert(uri.clone(), data).is_some() {
            return Err(TuneWeaveError::invalid_request(format!(
                "duplicate NetEase batch request: {uri}"
            ))
            .with_details(json!({ "uri": uri })));
        }
    }
    if requests.is_empty() {
        return Err(TuneWeaveError::invalid_request(
            "NetEase batch requires at least one /api/... request",
        ));
    }
    Ok(requests)
}

fn take_netease_batch_alias(
    query: &mut BTreeMap<String, String>,
    primary: &str,
    alias: &str,
) -> Result<Option<String>, TuneWeaveError> {
    let primary_value = query.remove(primary);
    let alias_value = query.remove(alias);
    if primary_value.is_some() && alias_value.is_some() {
        return Err(TuneWeaveError::invalid_request(format!(
            "{primary} and {alias} cannot be used together"
        )));
    }
    Ok(primary_value.or(alias_value))
}

fn parse_netease_batch_requests_json(
    requests: &str,
) -> Result<BTreeMap<String, Value>, TuneWeaveError> {
    serde_json::from_str::<BTreeMap<String, Value>>(requests).map_err(|_| {
        TuneWeaveError::invalid_request("requests must be a JSON object of /api/... calls")
    })
}

fn parse_json_bool(name: &str, value: Option<Value>) -> Result<bool, TuneWeaveError> {
    match value {
        None | Some(Value::Null) => Ok(false),
        Some(Value::Bool(value)) => Ok(value),
        Some(Value::Number(value)) if value.as_i64() == Some(1) => Ok(true),
        Some(Value::Number(value)) if value.as_i64() == Some(0) => Ok(false),
        Some(Value::String(value)) if value.trim().is_empty() => Ok(false),
        Some(Value::String(value)) => parse_bool_parameter(name, Some(&value), false),
        Some(value) => Err(TuneWeaveError::invalid_request(format!(
            "{name} must be true or false"
        ))
        .with_details(json!({ "parameter": name, "value": value }))),
    }
}

fn empty_json_object() -> Value {
    json!({})
}

fn json_body<T>(payload: Result<Json<T>, JsonRejection>) -> Result<T, ApiError> {
    payload
        .map(|Json(body)| body)
        .map_err(|_| TuneWeaveError::invalid_request("request body must be valid JSON").into())
}

fn account_alias(value: Option<&str>) -> Result<String, TuneWeaveError> {
    let account = value.unwrap_or("default").trim();
    let account = if account.is_empty() {
        "default"
    } else {
        account
    };
    if account.len() > 64 {
        return Err(TuneWeaveError::invalid_request(
            "account alias cannot exceed 64 bytes",
        ));
    }
    Ok(account.to_owned())
}

fn account_platform(state: &AppState, value: Option<&str>) -> Result<Platform, TuneWeaveError> {
    value.map_or(Ok(state.default_platform), parse_platform_parameter)
}

fn optional_trimmed(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn auth_transaction_not_found() -> TuneWeaveError {
    TuneWeaveError::new(
        tuneweave_core::ErrorCode::ResourceNotFound,
        "authentication transaction was not found or has expired",
    )
}

fn parse_reference(reference: String) -> Result<ResourceRef, TuneWeaveError> {
    reference.parse().map_err(|error| {
        TuneWeaveError::invalid_request(format!("{error}"))
            .with_details(json!({ "reference": reference }))
    })
}

fn parse_quality(value: Option<&str>) -> Result<Quality, TuneWeaveError> {
    match value.unwrap_or("auto").trim().to_ascii_lowercase().as_str() {
        "auto" => Ok(Quality::Auto),
        "low" => Ok(Quality::Low),
        "standard" => Ok(Quality::Standard),
        "high" => Ok(Quality::High),
        "lossless" => Ok(Quality::Lossless),
        "hires" => Ok(Quality::Hires),
        "spatial" => Ok(Quality::Spatial),
        "master" => Ok(Quality::Master),
        value => Err(TuneWeaveError::invalid_request(format!(
            "unsupported quality: {value}"
        ))),
    }
}

fn parse_history_period(value: Option<&str>) -> Result<PlaybackHistoryPeriod, TuneWeaveError> {
    match value
        .unwrap_or("all_time")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "all" | "all_time" => Ok(PlaybackHistoryPeriod::AllTime),
        "week" => Ok(PlaybackHistoryPeriod::Week),
        value => Err(TuneWeaveError::invalid_request(format!(
            "unsupported history period: {value}"
        ))),
    }
}

fn parse_artist_category(value: Option<&str>) -> Result<ArtistCategory, TuneWeaveError> {
    match value.unwrap_or("all").trim().to_ascii_lowercase().as_str() {
        "all" => Ok(ArtistCategory::All),
        "male" => Ok(ArtistCategory::Male),
        "female" => Ok(ArtistCategory::Female),
        "group" | "band" => Ok(ArtistCategory::Group),
        value => Err(
            TuneWeaveError::invalid_request(format!("unsupported artist type: {value}"))
                .with_details(json!({ "allowed": ["all", "male", "female", "group"] })),
        ),
    }
}

fn parse_artist_area(value: Option<&str>) -> Result<ArtistArea, TuneWeaveError> {
    match value.unwrap_or("all").trim().to_ascii_lowercase().as_str() {
        "all" => Ok(ArtistArea::All),
        "chinese" => Ok(ArtistArea::Chinese),
        "western" => Ok(ArtistArea::Western),
        "japanese" => Ok(ArtistArea::Japanese),
        "korean" => Ok(ArtistArea::Korean),
        "other" => Ok(ArtistArea::Other),
        value => Err(
            TuneWeaveError::invalid_request(format!("unsupported artist area: {value}"))
                .with_details(json!({
                    "allowed": ["all", "chinese", "western", "japanese", "korean", "other"]
                })),
        ),
    }
}

fn parse_video_kind(value: Option<&str>) -> Result<VideoKind, TuneWeaveError> {
    match value.unwrap_or("mv").trim().to_ascii_lowercase().as_str() {
        "all" | "video" => Ok(VideoKind::All),
        "mv" => Ok(VideoKind::Mv),
        value => Err(
            TuneWeaveError::invalid_request(format!("unsupported video type: {value}"))
                .with_details(json!({ "allowed": ["all", "mv"] })),
        ),
    }
}

fn parse_artist_track_order(value: Option<&str>) -> Result<ArtistTrackOrder, TuneWeaveError> {
    match value.unwrap_or("hot").trim().to_ascii_lowercase().as_str() {
        "hot" => Ok(ArtistTrackOrder::Hot),
        "time" => Ok(ArtistTrackOrder::Time),
        value => Err(TuneWeaveError::invalid_request(format!(
            "unsupported artist track order: {value}"
        ))
        .with_details(json!({ "allowed": ["hot", "time"] }))),
    }
}

fn parse_digital_album_chart_period(
    value: Option<&str>,
) -> Result<DigitalAlbumChartPeriod, TuneWeaveError> {
    match value
        .unwrap_or("daily")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "daily" => Ok(DigitalAlbumChartPeriod::Daily),
        "week" => Ok(DigitalAlbumChartPeriod::Week),
        "year" => Ok(DigitalAlbumChartPeriod::Year),
        "total" => Ok(DigitalAlbumChartPeriod::Total),
        value => Err(TuneWeaveError::invalid_request(format!(
            "unsupported digital album chart period: {value}"
        ))
        .with_details(json!({ "allowed": ["daily", "week", "year", "total"] }))),
    }
}

fn parse_digital_album_chart_kind(
    value: Option<&str>,
) -> Result<DigitalAlbumChartKind, TuneWeaveError> {
    match value
        .unwrap_or("album")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "album" => Ok(DigitalAlbumChartKind::Album),
        "single" => Ok(DigitalAlbumChartKind::Single),
        value => Err(TuneWeaveError::invalid_request(format!(
            "unsupported digital album chart type: {value}"
        ))
        .with_details(json!({ "allowed": ["album", "single"] }))),
    }
}

fn parse_bool_parameter(
    name: &str,
    value: Option<&str>,
    default: bool,
) -> Result<bool, TuneWeaveError> {
    match value.map(str::trim).map(str::to_ascii_lowercase) {
        None => Ok(default),
        Some(value) if value == "true" || value == "1" => Ok(true),
        Some(value) if value == "false" || value == "0" => Ok(false),
        Some(value) => Err(TuneWeaveError::invalid_request(format!(
            "{name} must be true or false"
        ))
        .with_details(json!({ "parameter": name, "value": value }))),
    }
}

fn parse_platform_parameter(value: &str) -> Result<Platform, TuneWeaveError> {
    let value = value.trim();
    value
        .parse()
        .map_err(|_| TuneWeaveError::invalid_request(format!("unsupported platform: {value}")))
}

fn parse_platform_list(value: Option<&str>) -> Result<Vec<Platform>, TuneWeaveError> {
    value.map_or_else(
        || Ok(Vec::new()),
        |value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|platform| !platform.is_empty())
                .map(parse_platform_parameter)
                .collect()
        },
    )
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

fn parse_banner_client(value: Option<&str>) -> Result<BannerClient, TuneWeaveError> {
    match value.unwrap_or("pc").trim().to_ascii_lowercase().as_str() {
        "pc" | "0" => Ok(BannerClient::Pc),
        "android" | "1" => Ok(BannerClient::Android),
        "iphone" | "ios" | "2" => Ok(BannerClient::Iphone),
        "ipad" | "3" => Ok(BannerClient::Ipad),
        value => Err(TuneWeaveError::invalid_request(format!(
            "unsupported banner client: {value}"
        ))
        .with_details(json!({
            "allowed": ["pc", "android", "iphone", "ipad", "0", "1", "2", "3"]
        }))),
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

fn parse_optional_u32_parameter(
    name: &str,
    value: Option<&str>,
) -> Result<Option<u32>, TuneWeaveError> {
    value
        .map(|value| {
            value.parse().map_err(|_| {
                TuneWeaveError::invalid_request(format!("{name} must be an unsigned integer"))
                    .with_details(json!({ "parameter": name, "value": value }))
            })
        })
        .transpose()
}

fn parse_optional_u64_parameter(
    name: &str,
    value: Option<&str>,
) -> Result<Option<u64>, TuneWeaveError> {
    value
        .map(|value| {
            value.parse().map_err(|_| {
                TuneWeaveError::invalid_request(format!("{name} must be an unsigned integer"))
                    .with_details(json!({ "parameter": name, "value": value }))
            })
        })
        .transpose()
}

fn parse_optional_u16_parameter(
    name: &str,
    value: Option<&str>,
) -> Result<Option<u16>, TuneWeaveError> {
    value
        .map(|value| {
            value.parse().map_err(|_| {
                TuneWeaveError::invalid_request(format!(
                    "{name} must be an unsigned 16-bit integer"
                ))
                .with_details(json!({ "parameter": name, "value": value }))
            })
        })
        .transpose()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use async_trait::async_trait;
    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode, header},
    };
    use serde_json::Value;
    use tower::ServiceExt;
    use tuneweave_core::{
        ArtistBiographySection, ArtistSummary, ArtistWorkKind, AudioRecognitionMatch,
        BannerTargetKind, CreatorSummary, MusicProvider, Page, PageMeta, ProviderQrStart,
        RadioCatalogOption, Result, SearchQuery, StreamRequest,
    };

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
                Capability::AudioRecognition,
                Capability::Banners,
                Capability::RadioTaxonomy,
                Capability::RadioStationDetail,
                Capability::TrackDetail,
                Capability::AlbumDetail,
                Capability::AlbumList,
                Capability::AlbumStats,
                Capability::AlbumTrackEntitlements,
                Capability::AlbumSubscriptionWrite,
                Capability::DigitalAlbumDetail,
                Capability::DigitalAlbumList,
                Capability::DigitalAlbumCharts,
                Capability::ArtistDetail,
                Capability::ArtistOverview,
                Capability::ArtistStats,
                Capability::ArtistList,
                Capability::ArtistAlbums,
                Capability::ArtistFans,
                Capability::ArtistVideos,
                Capability::ArtistTracks,
                Capability::ArtistTopTracks,
                Capability::ArtistSubscriptionWrite,
                Capability::PlaylistRead,
                Capability::Lyrics,
                Capability::AudioStream,
                Capability::QrLogin,
                Capability::PasswordLogin,
                Capability::PhoneLogin,
                Capability::SessionManagement,
                Capability::AccountProfile,
                Capability::AccountPlaylists,
                Capability::AccountAlbums,
                Capability::AccountRadioStations,
                Capability::AccountFollowingArtists,
                Capability::AccountArtistNewVideos,
                Capability::AccountArtistNewTracks,
                Capability::AccountArtistNewWorks,
                Capability::AccountArtistNewTracksPlayAll,
                Capability::AccountAvatarWrite,
                Capability::Favorites,
                Capability::ListeningHistory,
                Capability::Recommendations,
                Capability::PlatformApi,
                Capability::PlatformBatch,
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
                    extensions: Default::default(),
                },
            })
        }

        async fn recognize_audio(
            &self,
            request: &AudioRecognitionRequest,
        ) -> Result<AudioRecognition> {
            let mut track = sample_track("185809");
            track
                .extensions
                .insert("fingerprint".to_owned(), json!(request.fingerprint));
            let mut match_extensions = tuneweave_core::Extensions::new();
            match_extensions.insert("score".to_owned(), json!(0.97));
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert(
                "duration_seconds".to_owned(),
                json!(request.duration_seconds),
            );
            Ok(AudioRecognition {
                matches: vec![AudioRecognitionMatch {
                    track,
                    start_time_ms: Some(1_500),
                    extensions: match_extensions,
                }],
                query_id: Some("query-1".to_owned()),
                no_match_reason: None,
                extensions,
            })
        }

        async fn banners(&self, request: &BannerListRequest) -> Result<Vec<Banner>> {
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("client".to_owned(), json!(request.client));
            extensions.insert("account".to_owned(), json!(request.account));
            Ok(vec![Banner {
                id: Some("banner-1".to_owned()),
                title: Some("新歌首发".to_owned()),
                image_url: "https://example.test/banner.jpg".to_owned(),
                target_ref: Some(
                    ResourceRef::new(Platform::Netease, "185809").expect("valid banner target"),
                ),
                target_kind: BannerTargetKind::Track,
                url: Some("https://music.163.com/song?id=185809".to_owned()),
                exclusive: Some(false),
                extensions,
            }])
        }

        async fn radio_taxonomy(&self, request: &RadioTaxonomyRequest) -> Result<RadioTaxonomy> {
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("account".to_owned(), json!(request.account));
            Ok(RadioTaxonomy {
                categories: vec![RadioCatalogOption {
                    id: "1".to_owned(),
                    name: "音乐台".to_owned(),
                    extensions: Default::default(),
                }],
                regions: vec![RadioCatalogOption {
                    id: "407".to_owned(),
                    name: "网络台".to_owned(),
                    extensions: Default::default(),
                }],
                extensions,
            })
        }

        async fn radio_station(&self, id: &str, account: Option<&str>) -> Result<RadioStation> {
            let mut station = sample_radio_station(id);
            station.stream_url = Some("https://example.test/radio-live.mp3".to_owned());
            station.current_program = Some("晚安金山".to_owned());
            station.extensions.insert(
                "current_info".to_owned(),
                json!({ "thirdChannelId": "4022", "account": account }),
            );
            Ok(station)
        }

        async fn track(&self, id: &str, _account: Option<&str>) -> Result<Track> {
            Ok(sample_track(id))
        }

        async fn album(&self, id: &str, _account: Option<&str>) -> Result<Album> {
            Ok(sample_album(id))
        }

        async fn album_tracks(&self, _id: &str, request: &PageRequest) -> Result<Page<Track>> {
            Ok(Page {
                items: vec![sample_track("185809")],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(1),
                    next_offset: None,
                    has_more: false,
                    extensions: Default::default(),
                },
            })
        }

        async fn albums(&self, request: &AlbumListRequest) -> Result<Page<Album>> {
            let mut album = sample_album("387169747");
            if let Some(area) = &request.area {
                album.extensions.insert("area".to_owned(), json!(area));
            }
            if let Some(catalog) = &request.catalog {
                album
                    .extensions
                    .insert("catalog".to_owned(), json!(catalog));
            }
            Ok(Page {
                items: vec![album],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(500),
                    next_offset: Some(request.offset.saturating_add(1)),
                    has_more: true,
                    extensions: Default::default(),
                },
            })
        }

        async fn album_stats(&self, id: &str, _account: Option<&str>) -> Result<AlbumStats> {
            Ok(sample_album_stats(id))
        }

        async fn album_track_entitlements(
            &self,
            _id: &str,
            request: &PageRequest,
        ) -> Result<Page<TrackEntitlement>> {
            Ok(Page {
                items: vec![sample_track_entitlement("2058263030")],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(10),
                    next_offset: Some(request.offset.saturating_add(1)),
                    has_more: true,
                    extensions: Default::default(),
                },
            })
        }

        async fn set_album_subscription(
            &self,
            id: &str,
            subscribed: bool,
            account: Option<&str>,
        ) -> Result<SubscriptionResult> {
            let mut extensions = tuneweave_core::Extensions::new();
            if let Some(account) = account {
                extensions.insert("account".to_owned(), json!(account));
            }
            Ok(SubscriptionResult {
                resource_ref: ResourceRef::new(Platform::Netease, id)
                    .expect("valid test reference"),
                subscribed,
                extensions,
            })
        }

        async fn digital_album(&self, id: &str, _account: Option<&str>) -> Result<DigitalAlbum> {
            Ok(sample_digital_album(id))
        }

        async fn digital_albums(
            &self,
            request: &DigitalAlbumListRequest,
        ) -> Result<Page<DigitalAlbum>> {
            let mut album = sample_digital_album("120605500");
            if let Some(area) = &request.area {
                album.extensions.insert("area".to_owned(), json!(area));
            }
            if let Some(kind) = &request.kind {
                album.extensions.insert("type".to_owned(), json!(kind));
            }
            if let Some(catalog) = &request.catalog {
                album
                    .extensions
                    .insert("catalog".to_owned(), json!(catalog));
            }
            Ok(Page {
                items: vec![album],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: None,
                    next_offset: None,
                    has_more: false,
                    extensions: Default::default(),
                },
            })
        }

        async fn digital_album_chart(
            &self,
            request: &DigitalAlbumChartRequest,
        ) -> Result<Page<DigitalAlbumChartEntry>> {
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("period".to_owned(), json!(request.period));
            extensions.insert("kind".to_owned(), json!(request.kind));
            if let Some(year) = request.year {
                extensions.insert("year".to_owned(), json!(year));
            }
            Ok(Page {
                items: vec![DigitalAlbumChartEntry {
                    rank: request.offset.saturating_add(1),
                    rank_change: Some(5),
                    product: sample_digital_album("156507145"),
                    extensions,
                }],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(20),
                    next_offset: Some(request.offset.saturating_add(1)),
                    has_more: true,
                    extensions: Default::default(),
                },
            })
        }

        async fn artist(&self, id: &str, _account: Option<&str>) -> Result<Artist> {
            Ok(sample_artist(id))
        }

        async fn artist_overview(
            &self,
            id: &str,
            _account: Option<&str>,
        ) -> Result<ArtistOverview> {
            let mut track = sample_track("210049");
            track
                .extensions
                .insert("overview_track".to_owned(), json!({ "copyright": 2 }));
            Ok(ArtistOverview {
                artist: sample_artist(id),
                featured_tracks: vec![track],
                has_more_tracks: true,
                extensions: Default::default(),
            })
        }

        async fn artist_stats(&self, id: &str, _account: Option<&str>) -> Result<ArtistStats> {
            Ok(sample_artist_stats(id))
        }

        async fn artists(&self, request: &ArtistListRequest) -> Result<Page<Artist>> {
            let mut artist = sample_artist("178059");
            artist
                .extensions
                .insert("category".to_owned(), json!(request.category));
            artist
                .extensions
                .insert("area".to_owned(), json!(request.area));
            if let Some(initial) = &request.initial {
                artist
                    .extensions
                    .insert("initial".to_owned(), json!(initial));
            }
            Ok(Page {
                items: vec![artist],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: None,
                    next_offset: Some(request.offset.saturating_add(1)),
                    has_more: true,
                    extensions: Default::default(),
                },
            })
        }

        async fn artist_albums(&self, id: &str, request: &PageRequest) -> Result<Page<Album>> {
            let mut album = sample_album("18915");
            album.extensions.insert("artist_id".to_owned(), json!(id));
            Ok(Page {
                items: vec![album],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: None,
                    next_offset: Some(request.offset.saturating_add(1)),
                    has_more: true,
                    extensions: Default::default(),
                },
            })
        }

        async fn artist_fans(&self, id: &str, request: &PageRequest) -> Result<Page<User>> {
            let mut user = sample_user("6298206519");
            user.extensions.insert("artist_id".to_owned(), json!(id));
            Ok(Page {
                items: vec![user],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: None,
                    next_offset: Some(request.offset.saturating_add(1)),
                    has_more: true,
                    extensions: Default::default(),
                },
            })
        }

        async fn artist_videos(
            &self,
            id: &str,
            request: &ArtistVideoListRequest,
        ) -> Result<Page<Video>> {
            let mut video = sample_video("22695250");
            video.extensions.insert("artist_id".to_owned(), json!(id));
            video
                .extensions
                .insert("type".to_owned(), json!(request.kind));
            Ok(Page {
                items: vec![video],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: None,
                    next_offset: Some(request.offset.saturating_add(1)),
                    has_more: true,
                    extensions: Default::default(),
                },
            })
        }

        async fn artist_tracks(
            &self,
            id: &str,
            request: &ArtistTrackListRequest,
        ) -> Result<Page<Track>> {
            let mut track = sample_track("298317");
            track.extensions.insert("artist_id".to_owned(), json!(id));
            track
                .extensions
                .insert("order".to_owned(), json!(request.order));
            Ok(Page {
                items: vec![track],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(566),
                    next_offset: Some(request.offset.saturating_add(1)),
                    has_more: true,
                    extensions: Default::default(),
                },
            })
        }

        async fn artist_top_tracks(&self, id: &str, _account: Option<&str>) -> Result<Page<Track>> {
            let mut track = sample_track("185809");
            track.extensions.insert("artist_id".to_owned(), json!(id));
            Ok(Page {
                items: vec![track],
                pagination: PageMeta {
                    limit: 50,
                    offset: 0,
                    total: Some(1),
                    next_offset: None,
                    has_more: false,
                    extensions: Default::default(),
                },
            })
        }

        async fn set_artist_subscription(
            &self,
            id: &str,
            subscribed: bool,
            account: Option<&str>,
        ) -> Result<SubscriptionResult> {
            let mut extensions = tuneweave_core::Extensions::new();
            if let Some(account) = account {
                extensions.insert("account".to_owned(), json!(account));
            }
            Ok(SubscriptionResult {
                resource_ref: ResourceRef::new(Platform::Netease, id)
                    .expect("valid test reference"),
                subscribed,
                extensions,
            })
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
                    extensions: Default::default(),
                },
            })
        }

        async fn account_playlists(&self, request: &PageRequest) -> Result<Page<Playlist>> {
            Ok(Page {
                items: vec![sample_playlist("3778678")],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(1),
                    next_offset: None,
                    has_more: false,
                    extensions: Default::default(),
                },
            })
        }

        async fn favorite_tracks(&self, request: &PageRequest) -> Result<Page<Track>> {
            Ok(Page {
                items: vec![sample_track("185809")],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(1),
                    next_offset: None,
                    has_more: false,
                    extensions: Default::default(),
                },
            })
        }

        async fn user_favorite_tracks(
            &self,
            user_id: &str,
            request: &PageRequest,
        ) -> Result<Page<Track>> {
            Ok(Page {
                items: vec![sample_track(user_id)],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(1),
                    next_offset: None,
                    has_more: false,
                    extensions: Default::default(),
                },
            })
        }

        async fn account_albums(&self, request: &PageRequest) -> Result<Page<Album>> {
            let mut album = sample_album("32311");
            album.extensions.insert(
                "subscription_item".to_owned(),
                json!({ "subTime": 1704067200000_u64 }),
            );
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert(
                "response".to_owned(),
                json!({ "code": 200, "paidCount": 1 }),
            );
            Ok(Page {
                items: vec![album],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(1),
                    next_offset: None,
                    has_more: false,
                    extensions,
                },
            })
        }

        async fn account_radio_stations(
            &self,
            request: &PageRequest,
        ) -> Result<Page<RadioStation>> {
            let mut station = sample_radio_station("362");
            station.extensions.insert(
                "collection_item".to_owned(),
                json!({ "collectTime": 1_700_000_000_000_u64 }),
            );
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert(
                "response".to_owned(),
                json!({ "code": 200, "source": "QT" }),
            );
            Ok(Page {
                items: vec![station],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(53),
                    next_offset: Some(request.offset.saturating_add(1)),
                    has_more: true,
                    extensions,
                },
            })
        }

        async fn account_following_artists(&self, request: &PageRequest) -> Result<Page<Artist>> {
            let mut artist = sample_artist("6452");
            artist.extensions.insert(
                "following_item".to_owned(),
                json!({ "subTime": 1_720_000_000_000_u64 }),
            );
            Ok(Page {
                items: vec![artist],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(8),
                    next_offset: Some(request.offset.saturating_add(1)),
                    has_more: true,
                    extensions: Default::default(),
                },
            })
        }

        async fn account_artist_new_videos(
            &self,
            request: &ArtistUpdatesRequest,
        ) -> Result<Page<Video>> {
            let mut video = sample_video("1099001");
            video.title = "新 MV".to_owned();
            video.extensions.insert(
                "account".to_owned(),
                json!(request.account.as_deref().unwrap_or("default")),
            );
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("before_ms".to_owned(), json!(request.before_ms));
            extensions.insert("next_before_ms".to_owned(), json!(1_720_000_000_000_u64));
            Ok(Page {
                items: vec![video],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: 0,
                    total: None,
                    next_offset: None,
                    has_more: true,
                    extensions,
                },
            })
        }

        async fn account_artist_new_tracks(
            &self,
            request: &ArtistUpdatesRequest,
        ) -> Result<Page<Track>> {
            let mut track = sample_track("2099001");
            track.name = "新歌".to_owned();
            track.extensions.insert(
                "account".to_owned(),
                json!(request.account.as_deref().unwrap_or("default")),
            );
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("before_ms".to_owned(), json!(request.before_ms));
            extensions.insert("next_before_ms".to_owned(), json!(1_720_000_000_000_u64));
            Ok(Page {
                items: vec![track],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: 0,
                    total: Some(3),
                    next_offset: None,
                    has_more: true,
                    extensions,
                },
            })
        }

        async fn account_artist_new_works(
            &self,
            request: &ArtistWorksRequest,
        ) -> Result<Page<ArtistWorkUpdate>> {
            let mut item_extensions = tuneweave_core::Extensions::new();
            item_extensions.insert(
                "account".to_owned(),
                json!(request.account.as_deref().unwrap_or("default")),
            );
            let item = ArtistWorkUpdate {
                source_type: request.source_type,
                kind: ArtistWorkKind::Track,
                published_at: Some("2024-07-03".to_owned()),
                artist: None,
                title: Some("新专辑".to_owned()),
                cover_url: Some("https://example.test/new-album.jpg".to_owned()),
                tracks: vec![sample_track("2099001")],
                videos: Vec::new(),
                extensions: item_extensions,
            };
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("before_ms".to_owned(), json!(request.before_ms));
            extensions.insert("next_before_ms".to_owned(), json!(1_720_000_000_000_u64));
            extensions.insert("source_type".to_owned(), json!(request.source_type));
            extensions.insert("first_request".to_owned(), json!(request.first_request));
            Ok(Page {
                items: vec![item],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: 0,
                    total: None,
                    next_offset: None,
                    has_more: true,
                    extensions,
                },
            })
        }

        async fn account_artist_new_tracks_play_all(
            &self,
            account: Option<&str>,
        ) -> Result<Page<Track>> {
            let mut track = sample_track("2099001");
            track.name = "新歌".to_owned();
            track
                .extensions
                .insert("account".to_owned(), json!(account.unwrap_or("default")));
            Ok(Page {
                items: vec![track],
                pagination: PageMeta {
                    limit: 50,
                    offset: 0,
                    total: Some(1),
                    next_offset: None,
                    has_more: false,
                    extensions: Default::default(),
                },
            })
        }

        async fn account_history(
            &self,
            request: &PlaybackHistoryRequest,
        ) -> Result<Page<PlaybackHistoryEntry>> {
            Ok(Page {
                items: vec![sample_history("185809")],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(1),
                    next_offset: None,
                    has_more: false,
                    extensions: Default::default(),
                },
            })
        }

        async fn user_history(
            &self,
            user_id: &str,
            request: &PlaybackHistoryRequest,
        ) -> Result<Page<PlaybackHistoryEntry>> {
            Ok(Page {
                items: vec![sample_history(user_id)],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(1),
                    next_offset: None,
                    has_more: false,
                    extensions: Default::default(),
                },
            })
        }

        async fn recommended_tracks(&self, request: &RecommendationRequest) -> Result<Page<Track>> {
            let mut track = sample_track("185809");
            track
                .extensions
                .insert("refresh".to_owned(), json!(request.refresh));
            Ok(Page {
                items: vec![track],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(1),
                    next_offset: None,
                    has_more: false,
                    extensions: Default::default(),
                },
            })
        }

        async fn recommended_playlists(
            &self,
            request: &RecommendationRequest,
        ) -> Result<Page<Playlist>> {
            Ok(Page {
                items: vec![sample_playlist("99")],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(1),
                    next_offset: None,
                    has_more: false,
                    extensions: Default::default(),
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

        async fn stream(&self, track: &Track, request: &StreamRequest) -> Result<MediaStream> {
            Ok(MediaStream {
                url: "https://example.test/audio.mp3".to_owned(),
                backup_urls: Vec::new(),
                headers: Default::default(),
                expires_at: None,
                format: Some("mp3".to_owned()),
                codec: Some("mp3".to_owned()),
                bitrate: Some(320_000),
                size: Some(1024),
                duration_ms: track.duration_ms,
                requested_quality: request.quality,
                actual_quality: Quality::High,
                trial: None,
                origin_track: Some(track.resource_ref.clone()),
                resolved_track: track.resource_ref.clone(),
                resolved_platform: Platform::Netease,
                match_score: Some(1.0),
                attempts: Vec::new(),
            })
        }

        async fn start_qr_login(&self, _login_type: Option<&str>) -> Result<ProviderQrStart> {
            Ok(ProviderQrStart {
                provider_transaction_id: "provider-qr-key".to_owned(),
                url: "https://example.test/qr".to_owned(),
                image_data_url: None,
                expires_at: None,
            })
        }

        async fn poll_qr_login(
            &self,
            provider_transaction_id: &str,
            account: &str,
        ) -> Result<tuneweave_core::ProviderQrPoll> {
            assert_eq!(provider_transaction_id, "provider-qr-key");
            Ok(tuneweave_core::ProviderQrPoll {
                state: AuthState::Confirmed,
                message: None,
                profile: Some(AccountProfile::authenticated(Platform::Netease, account)),
            })
        }

        async fn password_login(&self, request: &PasswordLoginRequest) -> Result<AccountProfile> {
            Ok(AccountProfile::authenticated(
                Platform::Netease,
                &request.account,
            ))
        }

        async fn start_auth_challenge(&self, _request: &AuthChallengeRequest) -> Result<()> {
            Ok(())
        }

        async fn verify_auth_challenge(
            &self,
            request: &AuthChallengeRequest,
            _code: &str,
        ) -> Result<AccountProfile> {
            Ok(AccountProfile::authenticated(
                Platform::Netease,
                &request.account,
            ))
        }

        async fn logout(&self, _account: &str) -> Result<bool> {
            Ok(true)
        }

        async fn session_profile(&self, account: &str) -> Result<AccountProfile> {
            Ok(AccountProfile::authenticated(Platform::Netease, account))
        }

        async fn refresh_session(&self, account: &str) -> Result<AccountProfile> {
            let mut profile = AccountProfile::authenticated(Platform::Netease, account);
            profile
                .extensions
                .insert("refreshed".to_owned(), json!(true));
            Ok(profile)
        }

        async fn upload_account_avatar(
            &self,
            request: &ImageUploadRequest,
        ) -> Result<ImageUploadResult> {
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("filename".to_owned(), json!(request.filename));
            extensions.insert("content_type".to_owned(), json!(request.content_type));
            extensions.insert("data_len".to_owned(), json!(request.data.len()));
            extensions.insert("image_size".to_owned(), json!(request.image_size));
            extensions.insert("crop_x".to_owned(), json!(request.crop_x));
            extensions.insert("crop_y".to_owned(), json!(request.crop_y));
            extensions.insert("account".to_owned(), json!(request.account));
            Ok(ImageUploadResult {
                url: Some("https://example.test/avatar.png".to_owned()),
                image_id: Some("109951168000000000".to_owned()),
                extensions,
            })
        }

        async fn platform_api(&self, request: &PlatformApiRequest) -> Result<Value> {
            Ok(json!({
                "code": 200,
                "uri": request.uri,
                "data": request.data,
                "crypto": request.protocol,
                "account": request.account
            }))
        }

        async fn platform_batch(&self, request: &PlatformBatchRequest) -> Result<Value> {
            Ok(json!({
                "code": 200,
                "requests": request.requests,
                "crypto": request.protocol,
                "encrypted_response": request.encrypted_response,
                "account": request.account
            }))
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
        track.duration_ms = Some(258_000);
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

    fn sample_album(id: &str) -> Album {
        Album {
            resource_ref: ResourceRef::new(Platform::Netease, id).expect("valid test reference"),
            platform: Platform::Netease,
            id: id.to_owned(),
            name: "Jay".to_owned(),
            aliases: Vec::new(),
            artists: vec![ArtistSummary {
                resource_ref: None,
                name: "周杰伦".to_owned(),
            }],
            description: "周杰伦首张专辑".to_owned(),
            cover_url: None,
            published_at: None,
            track_count: Some(10),
            company: None,
            kind: Some("album".to_owned()),
            extensions: Default::default(),
        }
    }

    fn sample_radio_station(id: &str) -> RadioStation {
        let mut station = RadioStation::new(
            ResourceRef::new(Platform::Netease, id).expect("valid test reference"),
            "金山区广播电视台综合广播",
        );
        station.cover_url = Some("https://example.test/radio-362.jpg".to_owned());
        station.region = Some("上海".to_owned());
        station.subscribed = Some(true);
        station
    }

    fn sample_artist(id: &str) -> Artist {
        Artist {
            resource_ref: ResourceRef::new(Platform::Netease, id).expect("valid test reference"),
            platform: Platform::Netease,
            id: id.to_owned(),
            name: "周杰伦".to_owned(),
            aliases: vec!["Jay Chou".to_owned(), "周董".to_owned()],
            description: "歌手、词曲作者与制作人。".to_owned(),
            biography_sections: vec![ArtistBiographySection {
                title: "代表作品".to_owned(),
                text: "范特西".to_owned(),
            }],
            avatar_url: Some("https://example.test/avatar.jpg".to_owned()),
            cover_url: Some("https://example.test/cover.jpg".to_owned()),
            album_count: Some(44),
            track_count: Some(568),
            mv_count: Some(9),
            video_count: Some(8),
            identities: vec!["作曲".to_owned()],
            extensions: Default::default(),
        }
    }

    fn sample_artist_stats(id: &str) -> ArtistStats {
        ArtistStats {
            artist_ref: ResourceRef::new(Platform::Netease, id).expect("valid test reference"),
            followed: Some(false),
            follower_count: Some(13_704_928),
            video_counts: vec![tuneweave_core::ArtistContentCount {
                category: Some("0".to_owned()),
                count: 9,
                extensions: Default::default(),
            }],
            online_concert_count: Some(0),
            extensions: Default::default(),
        }
    }

    fn sample_user(id: &str) -> User {
        User {
            resource_ref: ResourceRef::new(Platform::Netease, id).expect("valid test reference"),
            platform: Platform::Netease,
            id: id.to_owned(),
            name: "轻手揍人丸".to_owned(),
            avatar_url: Some("https://example.test/avatar.jpg".to_owned()),
            signature: Some("111".to_owned()),
            followed: Some(false),
            mutual: Some(false),
            extensions: Default::default(),
        }
    }

    fn sample_video(id: &str) -> Video {
        Video {
            resource_ref: ResourceRef::new(Platform::Netease, id).expect("valid test reference"),
            platform: Platform::Netease,
            id: id.to_owned(),
            title: "任性 (5525 Live版)".to_owned(),
            creators: vec![CreatorSummary {
                resource_ref: Some(
                    ResourceRef::new(Platform::Netease, "6452").expect("valid creator reference"),
                ),
                name: "周杰伦".to_owned(),
                avatar_url: None,
            }],
            description: String::new(),
            cover_url: Some("https://example.test/cover.jpg".to_owned()),
            duration_ms: Some(266_000),
            published_at: Some("2025-02-23".to_owned()),
            play_count: Some(100_726),
            subscribed: Some(false),
            extensions: Default::default(),
        }
    }

    fn sample_album_stats(id: &str) -> AlbumStats {
        AlbumStats {
            album_ref: ResourceRef::new(Platform::Netease, id).expect("valid test reference"),
            subscribed: Some(false),
            subscriber_count: Some(71_671),
            comment_count: Some(1_989),
            share_count: Some(9_306),
            like_count: Some(0),
            on_sale: Some(false),
            subscribed_at: None,
            extensions: Default::default(),
        }
    }

    fn sample_track_entitlement(id: &str) -> TrackEntitlement {
        TrackEntitlement {
            track_ref: ResourceRef::new(Platform::Netease, id).expect("valid test reference"),
            playable: Some(true),
            downloadable: Some(false),
            play_bitrate: Some(320_000),
            download_bitrate: Some(0),
            max_play_bitrate: Some(999_000),
            max_download_bitrate: Some(999_000),
            play_quality: Some(Quality::High),
            download_quality: None,
            available_qualities: vec![
                Quality::Standard,
                Quality::High,
                Quality::Lossless,
                Quality::Hires,
            ],
            fee: Some(8),
            paid: Some(false),
            extensions: Default::default(),
        }
    }

    fn sample_digital_album(id: &str) -> DigitalAlbum {
        DigitalAlbum {
            resource_ref: ResourceRef::new(Platform::Netease, id).expect("valid test reference"),
            platform: Platform::Netease,
            id: id.to_owned(),
            name: "冀西南林路行".to_owned(),
            artists: vec![ArtistSummary {
                resource_ref: None,
                name: "万能青年旅店".to_owned(),
            }],
            description: "西郊有密林 助君出重围".to_owned(),
            cover_url: None,
            published_at: Some("2020-12-21T16:00:01Z".to_owned()),
            price: Some(tuneweave_core::Money {
                amount: 22.0,
                currency: "CNY".to_owned(),
            }),
            is_free: Some(false),
            purchasable: Some(true),
            purchased: Some(false),
            sale_count: Some(42),
            track_count: None,
            tags: vec!["独家".to_owned()],
            extensions: Default::default(),
        }
    }

    fn sample_history(id: &str) -> PlaybackHistoryEntry {
        PlaybackHistoryEntry {
            track: sample_track(id),
            play_count: Some(42),
            score: Some(99),
            last_played_at: None,
            extensions: Default::default(),
        }
    }

    fn test_app_with_provider() -> Router {
        let mut registry = ProviderRegistry::new();
        registry.register(TestProvider).expect("register provider");
        build_router(AppState::new(registry, Platform::Netease))
    }

    async fn json_response_from(app: Router, path: &str) -> (StatusCode, Value) {
        json_request_from(app, Method::GET, path, None).await
    }

    async fn json_request_from(
        app: Router,
        method: Method,
        path: &str,
        json_body: Option<Value>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(path);
        let body = if let Some(json_body) = json_body {
            request = request.header(header::CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(&json_body).expect("serialize request JSON"))
        } else {
            Body::empty()
        };
        let response = app
            .oneshot(request.body(body).expect("build request"))
            .await
            .expect("request succeeds");
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let json = serde_json::from_slice(&body).expect("valid JSON");
        (status, json)
    }

    async fn binary_request_from(
        app: Router,
        path: &str,
        content_type: Option<&str>,
        body: Vec<u8>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder().method(Method::PUT).uri(path);
        if let Some(content_type) = content_type {
            request = request.header(header::CONTENT_TYPE, content_type);
        }
        let response = app
            .oneshot(request.body(Body::from(body)).expect("build request"))
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
    async fn banners_use_unified_client_platform_and_account_parameters() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/banners?platform=netease&client=iphone&account=personal",
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["id"], "banner-1");
        assert_eq!(json["data"][0]["target_ref"], "netease:185809");
        assert_eq!(json["data"][0]["target_kind"], "track");
        assert_eq!(json["data"][0]["extensions"]["client"], "iphone");
        assert_eq!(json["data"][0]["extensions"]["account"], "personal");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "personal");
    }

    #[tokio::test]
    async fn banners_accept_every_reference_numeric_client_type() {
        for (kind, expected) in [
            ("0", "pc"),
            ("1", "android"),
            ("2", "iphone"),
            ("3", "ipad"),
        ] {
            let path = format!("/v1/banners?type={kind}");
            let (status, json) = json_response_from(test_app_with_provider(), &path).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(json["data"][0]["extensions"]["client"], expected);
        }
    }

    #[tokio::test]
    async fn banners_reject_unknown_clients() {
        let (status, json) =
            json_response_from(test_app_with_provider(), "/v1/banners?client=windows-phone").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], "invalid_request");
    }

    #[tokio::test]
    async fn radio_taxonomy_uses_platform_account_and_stable_string_ids() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/radio/taxonomy?platform=netease&account=radio-user",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["categories"][0]["id"], "1");
        assert_eq!(json["data"]["categories"][0]["name"], "音乐台");
        assert_eq!(json["data"]["regions"][0]["id"], "407");
        assert_eq!(json["data"]["regions"][0]["name"], "网络台");
        assert_eq!(json["data"]["extensions"]["account"], "radio-user");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "radio-user");
    }

    #[tokio::test]
    async fn radio_station_detail_uses_reference_platform_and_account() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/radio/stations/netease:362?account=radio-listener",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["ref"], "netease:362");
        assert_eq!(json["data"]["name"], "金山区广播电视台综合广播");
        assert_eq!(json["data"]["region"], "上海");
        assert_eq!(
            json["data"]["stream_url"],
            "https://example.test/radio-live.mp3"
        );
        assert_eq!(json["data"]["current_program"], "晚安金山");
        assert_eq!(
            json["data"]["extensions"]["current_info"]["thirdChannelId"],
            "4022"
        );
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "radio-listener");
    }

    #[tokio::test]
    async fn audio_recognition_uses_unified_input_and_response_metadata() {
        let (status, json) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/audio/recognize",
            Some(json!({
                "platform": "netease",
                "account": "green-vip",
                "fingerprint": "  shazam-v2-fingerprint  ",
                "duration_seconds": 6
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["matches"][0]["track"]["ref"], "netease:185809");
        assert_eq!(
            json["data"]["matches"][0]["track"]["extensions"]["fingerprint"],
            "shazam-v2-fingerprint"
        );
        assert_eq!(json["data"]["matches"][0]["start_time_ms"], 1_500);
        assert_eq!(json["data"]["matches"][0]["extensions"]["score"], 0.97);
        assert_eq!(json["data"]["query_id"], "query-1");
        assert_eq!(json["data"]["extensions"]["duration_seconds"], 6);
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "green-vip");
    }

    #[tokio::test]
    async fn audio_recognition_accepts_source_compatible_field_aliases() {
        let (status, json) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/audio/recognize",
            Some(json!({
                "audioFP": "shazam-v2-fingerprint",
                "duration": 6
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["extensions"]["duration_seconds"], 6);
        assert_eq!(json["meta"]["platform"], "netease");
        assert!(json["meta"].get("account").is_none());
    }

    #[tokio::test]
    async fn audio_recognition_rejects_invalid_fingerprint_boundaries() {
        for body in [
            json!({"fingerprint": "   ", "duration_seconds": 6}),
            json!({"fingerprint": "fingerprint", "duration_seconds": 0}),
            json!({"fingerprint": "x".repeat(131_073), "duration_seconds": 6}),
        ] {
            let (status, json) = json_request_from(
                test_app_with_provider(),
                Method::POST,
                "/v1/audio/recognize",
                Some(body),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(json["error"]["code"], "invalid_request");
        }
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
    async fn album_list_uses_unified_catalog_filters_and_pagination() {
        let (status, albums) = json_response_from(
            test_app_with_provider(),
            "/v1/albums?platform=netease&account=default&catalog=new&area=KR&limit=5&offset=10",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(albums["data"][0]["ref"], "netease:387169747");
        assert_eq!(albums["data"][0]["extensions"]["area"], "KR");
        assert_eq!(albums["data"][0]["extensions"]["catalog"], "new");
        assert_eq!(albums["meta"]["pagination"]["limit"], 5);
        assert_eq!(albums["meta"]["pagination"]["offset"], 10);
        assert_eq!(albums["meta"]["pagination"]["total"], 500);
        assert_eq!(albums["meta"]["account"], "default");
    }

    #[tokio::test]
    async fn album_detail_and_tracks_use_reference_platform_and_pagination() {
        let (status, album) =
            json_response_from(test_app_with_provider(), "/v1/albums/netease:18915").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(album["data"]["ref"], "netease:18915");
        assert_eq!(album["data"]["artists"][0]["name"], "周杰伦");
        assert_eq!(album["meta"]["platform"], "netease");

        let (status, tracks) = json_response_from(
            test_app_with_provider(),
            "/v1/albums/netease:18915/tracks?limit=5&offset=0",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(tracks["data"][0]["ref"], "netease:185809");
        assert_eq!(tracks["meta"]["pagination"]["limit"], 5);
        assert_eq!(tracks["meta"]["pagination"]["total"], 1);
    }

    #[tokio::test]
    async fn artist_albums_use_reference_platform_account_and_pagination() {
        let (status, albums) = json_response_from(
            test_app_with_provider(),
            "/v1/artists/netease:6452/albums?limit=5&offset=10&account=collector",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(albums["data"][0]["ref"], "netease:18915");
        assert_eq!(albums["data"][0]["extensions"]["artist_id"], "6452");
        assert_eq!(albums["meta"]["platform"], "netease");
        assert_eq!(albums["meta"]["account"], "collector");
        assert_eq!(albums["meta"]["pagination"]["limit"], 5);
        assert_eq!(albums["meta"]["pagination"]["offset"], 10);
        assert_eq!(albums["meta"]["pagination"]["next_offset"], 11);
        assert_eq!(albums["meta"]["pagination"]["has_more"], true);
    }

    #[tokio::test]
    async fn artist_fans_use_reference_platform_account_and_pagination() {
        let (status, fans) = json_response_from(
            test_app_with_provider(),
            "/v1/artists/netease:2116/fans?limit=2&offset=10&account=collector",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(fans["data"][0]["ref"], "netease:6298206519");
        assert_eq!(fans["data"][0]["name"], "轻手揍人丸");
        assert_eq!(fans["data"][0]["extensions"]["artist_id"], "2116");
        assert_eq!(fans["meta"]["platform"], "netease");
        assert_eq!(fans["meta"]["account"], "collector");
        assert_eq!(fans["meta"]["pagination"]["limit"], 2);
        assert_eq!(fans["meta"]["pagination"]["offset"], 10);
        assert_eq!(fans["meta"]["pagination"]["next_offset"], 11);
        assert_eq!(fans["meta"]["pagination"]["has_more"], true);
    }

    #[tokio::test]
    async fn artist_videos_use_reference_platform_type_account_and_pagination() {
        let (status, videos) = json_response_from(
            test_app_with_provider(),
            "/v1/artists/netease:6452/videos?type=mv&limit=2&offset=10&account=collector",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(videos["data"][0]["ref"], "netease:22695250");
        assert_eq!(videos["data"][0]["title"], "任性 (5525 Live版)");
        assert_eq!(videos["data"][0]["creators"][0]["ref"], "netease:6452");
        assert_eq!(videos["data"][0]["extensions"]["artist_id"], "6452");
        assert_eq!(videos["data"][0]["extensions"]["type"], "mv");
        assert_eq!(videos["meta"]["platform"], "netease");
        assert_eq!(videos["meta"]["account"], "collector");
        assert_eq!(videos["meta"]["pagination"]["limit"], 2);
        assert_eq!(videos["meta"]["pagination"]["offset"], 10);
        assert_eq!(videos["meta"]["pagination"]["next_offset"], 11);
        assert_eq!(videos["meta"]["pagination"]["has_more"], true);
    }

    #[tokio::test]
    async fn artist_tracks_use_reference_platform_order_account_and_pagination() {
        let (status, tracks) = json_response_from(
            test_app_with_provider(),
            "/v1/artists/netease:6452/tracks?order=time&limit=2&offset=10&account=collector",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(tracks["data"][0]["ref"], "netease:298317");
        assert_eq!(tracks["data"][0]["extensions"]["artist_id"], "6452");
        assert_eq!(tracks["data"][0]["extensions"]["order"], "time");
        assert_eq!(tracks["meta"]["platform"], "netease");
        assert_eq!(tracks["meta"]["account"], "collector");
        assert_eq!(tracks["meta"]["pagination"]["limit"], 2);
        assert_eq!(tracks["meta"]["pagination"]["offset"], 10);
        assert_eq!(tracks["meta"]["pagination"]["total"], 566);
        assert_eq!(tracks["meta"]["pagination"]["next_offset"], 11);
        assert_eq!(tracks["meta"]["pagination"]["has_more"], true);
    }

    #[tokio::test]
    async fn artist_tracks_reject_unknown_order() {
        let (status, response) = json_response_from(
            test_app_with_provider(),
            "/v1/artists/netease:6452/tracks?order=random",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(response["error"]["code"], "invalid_request");
        assert_eq!(response["error"]["details"]["allowed"][0], "hot");
        assert_eq!(response["error"]["details"]["allowed"][1], "time");
    }

    #[tokio::test]
    async fn artist_top_tracks_are_exposed_as_a_fixed_snapshot() {
        let (status, tracks) = json_response_from(
            test_app_with_provider(),
            "/v1/artists/netease:6452/top-tracks?account=collector",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(tracks["data"][0]["ref"], "netease:185809");
        assert_eq!(tracks["data"][0]["extensions"]["artist_id"], "6452");
        assert_eq!(tracks["meta"]["platform"], "netease");
        assert_eq!(tracks["meta"]["account"], "collector");
        assert_eq!(tracks["meta"]["pagination"]["limit"], 50);
        assert_eq!(tracks["meta"]["pagination"]["offset"], 0);
        assert_eq!(tracks["meta"]["pagination"]["total"], 1);
        assert_eq!(tracks["meta"]["pagination"]["next_offset"], Value::Null);
        assert_eq!(tracks["meta"]["pagination"]["has_more"], false);
    }

    #[tokio::test]
    async fn artist_detail_uses_reference_platform_and_account() {
        let (status, artist) = json_response_from(
            test_app_with_provider(),
            "/v1/artists/netease:6452?account=collector",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(artist["data"]["ref"], "netease:6452");
        assert_eq!(artist["data"]["name"], "周杰伦");
        assert_eq!(artist["data"]["aliases"][0], "Jay Chou");
        assert_eq!(artist["data"]["biography_sections"][0]["title"], "代表作品");
        assert_eq!(artist["data"]["track_count"], 568);
        assert_eq!(artist["meta"]["platform"], "netease");
        assert_eq!(artist["meta"]["account"], "collector");
    }

    #[tokio::test]
    async fn artist_overview_keeps_featured_tracks_distinct_from_the_artist() {
        let (status, overview) = json_response_from(
            test_app_with_provider(),
            "/v1/artists/netease:6452/overview?account=collector",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(overview["data"]["artist"]["ref"], "netease:6452");
        assert_eq!(overview["data"]["artist"]["name"], "周杰伦");
        assert_eq!(
            overview["data"]["featured_tracks"][0]["ref"],
            "netease:210049"
        );
        assert_eq!(
            overview["data"]["featured_tracks"][0]["extensions"]["overview_track"]["copyright"],
            2
        );
        assert_eq!(overview["data"]["has_more_tracks"], true);
        assert_eq!(overview["meta"]["platform"], "netease");
        assert_eq!(overview["meta"]["account"], "collector");
    }

    #[tokio::test]
    async fn artist_catalog_uses_unified_filters_and_pagination() {
        let (status, artists) = json_response_from(
            test_app_with_provider(),
            "/v1/artists?platform=netease&account=collector&type=male&area=western&initial=b&limit=2&offset=10",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(artists["data"][0]["ref"], "netease:178059");
        assert_eq!(artists["data"][0]["extensions"]["category"], "male");
        assert_eq!(artists["data"][0]["extensions"]["area"], "western");
        assert_eq!(artists["data"][0]["extensions"]["initial"], "b");
        assert_eq!(artists["meta"]["platform"], "netease");
        assert_eq!(artists["meta"]["account"], "collector");
        assert_eq!(artists["meta"]["pagination"]["limit"], 2);
        assert_eq!(artists["meta"]["pagination"]["offset"], 10);
        assert_eq!(artists["meta"]["pagination"]["next_offset"], 11);
        assert_eq!(artists["meta"]["pagination"]["has_more"], true);
    }

    #[tokio::test]
    async fn artist_catalog_rejects_unknown_cross_platform_filters() {
        let (status, response) =
            json_response_from(test_app_with_provider(), "/v1/artists?type=solo&area=mars").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(response["error"]["code"], "invalid_request");
    }

    #[tokio::test]
    async fn artist_stats_use_reference_platform_and_account() {
        let (status, stats) = json_response_from(
            test_app_with_provider(),
            "/v1/artists/netease:6452/stats?account=collector",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stats["data"]["artist_ref"], "netease:6452");
        assert_eq!(stats["data"]["followed"], false);
        assert_eq!(stats["data"]["follower_count"], 13_704_928);
        assert_eq!(stats["data"]["video_counts"][0]["category"], "0");
        assert_eq!(stats["data"]["video_counts"][0]["count"], 9);
        assert_eq!(stats["meta"]["platform"], "netease");
        assert_eq!(stats["meta"]["account"], "collector");
    }

    #[tokio::test]
    async fn album_stats_use_reference_platform_and_account() {
        let (status, stats) = json_response_from(
            test_app_with_provider(),
            "/v1/albums/netease:32311/stats?account=collector",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stats["data"]["album_ref"], "netease:32311");
        assert_eq!(stats["data"]["subscriber_count"], 71_671);
        assert_eq!(stats["data"]["comment_count"], 1_989);
        assert_eq!(stats["meta"]["platform"], "netease");
        assert_eq!(stats["meta"]["account"], "collector");
    }

    #[tokio::test]
    async fn album_track_entitlements_use_reference_platform_and_pagination() {
        let (status, entitlements) = json_response_from(
            test_app_with_provider(),
            "/v1/albums/netease:168223858/track-entitlements?account=vip&limit=2&offset=0",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(entitlements["data"][0]["track_ref"], "netease:2058263030");
        assert_eq!(entitlements["data"][0]["playable"], true);
        assert_eq!(entitlements["data"][0]["play_quality"], "high");
        assert_eq!(entitlements["data"][0]["available_qualities"][3], "hires");
        assert_eq!(entitlements["meta"]["pagination"]["total"], 10);
        assert_eq!(entitlements["meta"]["account"], "vip");
    }

    #[tokio::test]
    async fn album_library_put_and_delete_share_the_subscription_result() {
        let (status, subscribed) = json_request_from(
            test_app_with_provider(),
            Method::PUT,
            "/v1/account/library/albums/netease:32311?account=collector",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(subscribed["data"]["resource_ref"], "netease:32311");
        assert_eq!(subscribed["data"]["subscribed"], true);
        assert_eq!(subscribed["data"]["extensions"]["account"], "collector");
        assert_eq!(subscribed["meta"]["platform"], "netease");
        assert_eq!(subscribed["meta"]["account"], "collector");

        let (status, unsubscribed) = json_request_from(
            test_app_with_provider(),
            Method::DELETE,
            "/v1/account/library/albums/netease:32311?account=collector",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(unsubscribed["data"]["resource_ref"], "netease:32311");
        assert_eq!(unsubscribed["data"]["subscribed"], false);
        assert_eq!(unsubscribed["meta"]["account"], "collector");
    }

    #[tokio::test]
    async fn artist_following_put_and_delete_share_the_subscription_result() {
        let (status, subscribed) = json_request_from(
            test_app_with_provider(),
            Method::PUT,
            "/v1/account/following/artists/netease:6452?account=collector",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(subscribed["data"]["resource_ref"], "netease:6452");
        assert_eq!(subscribed["data"]["subscribed"], true);
        assert_eq!(subscribed["data"]["extensions"]["account"], "collector");
        assert_eq!(subscribed["meta"]["platform"], "netease");
        assert_eq!(subscribed["meta"]["account"], "collector");

        let (status, unsubscribed) = json_request_from(
            test_app_with_provider(),
            Method::DELETE,
            "/v1/account/following/artists/netease:6452?account=collector",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(unsubscribed["data"]["resource_ref"], "netease:6452");
        assert_eq!(unsubscribed["data"]["subscribed"], false);
        assert_eq!(unsubscribed["meta"]["account"], "collector");
    }

    #[tokio::test]
    async fn digital_album_detail_uses_reference_platform_and_account() {
        let (status, album) = json_response_from(
            test_app_with_provider(),
            "/v1/digital-albums/netease:120605500?account=vip",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(album["data"]["ref"], "netease:120605500");
        assert_eq!(album["data"]["name"], "冀西南林路行");
        assert_eq!(album["data"]["price"]["amount"], 22.0);
        assert_eq!(album["data"]["price"]["currency"], "CNY");
        assert_eq!(album["meta"]["platform"], "netease");
        assert_eq!(album["meta"]["account"], "vip");
    }

    #[tokio::test]
    async fn digital_album_list_uses_unified_filters_and_pagination() {
        let (status, albums) = json_response_from(
            test_app_with_provider(),
            "/v1/digital-albums?platform=netease&account=vip&catalog=latest&area=KR&type=album&limit=5&offset=10",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(albums["data"][0]["ref"], "netease:120605500");
        assert_eq!(albums["data"][0]["extensions"]["area"], "KR");
        assert_eq!(albums["data"][0]["extensions"]["type"], "album");
        assert_eq!(albums["data"][0]["extensions"]["catalog"], "latest");
        assert_eq!(albums["meta"]["pagination"]["limit"], 5);
        assert_eq!(albums["meta"]["pagination"]["offset"], 10);
        assert_eq!(albums["meta"]["pagination"]["total"], Value::Null);
        assert_eq!(albums["meta"]["account"], "vip");
    }

    #[tokio::test]
    async fn digital_album_chart_uses_typed_filters_and_pagination() {
        let (status, chart) = json_response_from(
            test_app_with_provider(),
            "/v1/charts/digital-albums?platform=netease&account=vip&period=year&type=single&year=2025&limit=2&offset=3",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(chart["data"][0]["rank"], 4);
        assert_eq!(chart["data"][0]["rank_change"], 5);
        assert_eq!(chart["data"][0]["product"]["ref"], "netease:156507145");
        assert_eq!(chart["data"][0]["extensions"]["period"], "year");
        assert_eq!(chart["data"][0]["extensions"]["kind"], "single");
        assert_eq!(chart["data"][0]["extensions"]["year"], 2025);
        assert_eq!(chart["meta"]["pagination"]["limit"], 2);
        assert_eq!(chart["meta"]["pagination"]["offset"], 3);
        assert_eq!(chart["meta"]["pagination"]["total"], 20);
        assert_eq!(chart["meta"]["account"], "vip");
    }

    #[tokio::test]
    async fn digital_album_chart_rejects_invalid_period_and_year_combinations() {
        let (status, invalid_period) = json_response_from(
            test_app_with_provider(),
            "/v1/charts/digital-albums?period=month",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(invalid_period["error"]["code"], "invalid_request");

        let (status, invalid_year) = json_response_from(
            test_app_with_provider(),
            "/v1/charts/digital-albums?period=daily&year=2025",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(invalid_year["error"]["code"], "invalid_request");
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

    #[tokio::test]
    async fn track_stream_reports_resolution_attempts() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/tracks/netease:2709812973/stream?quality=high&fallback=false",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["requested_quality"], "high");
        assert_eq!(json["data"]["actual_quality"], "high");
        assert_eq!(json["data"]["origin_track"], "netease:2709812973");
        assert_eq!(json["data"]["resolved_track"], "netease:2709812973");
        assert_eq!(json["data"]["attempts"].as_array().map(Vec::len), Some(1));
        assert_eq!(json["data"]["attempts"][0]["status"], "success");
        assert_eq!(json["meta"]["platform"], "netease");
    }

    #[tokio::test]
    async fn invalid_stream_quality_uses_the_error_envelope() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/tracks/netease:2709812973/stream?quality=studio",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["ok"], false);
        assert_eq!(json["error"]["code"], "invalid_request");
    }

    #[tokio::test]
    async fn invalid_stream_platform_uses_the_error_envelope() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/tracks/netease:2709812973/stream?playback_platform=unknown",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], "invalid_request");
    }

    #[tokio::test]
    async fn qr_auth_uses_an_opaque_server_transaction_and_saves_the_account() {
        let app = test_app_with_provider();
        let (status, start) = json_request_from(
            app.clone(),
            Method::POST,
            "/v1/auth/qr",
            Some(json!({
                "platform": "netease",
                "account": "personal",
                "login_type": "pc"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let transaction_id = start["data"]["transaction_id"]
            .as_str()
            .expect("transaction id");
        assert!(transaction_id.starts_with("tw-auth-"));
        assert!(!transaction_id.contains("provider-qr-key"));
        assert_eq!(start["data"]["url"], "https://example.test/qr");

        let path = format!("/v1/auth/qr/{transaction_id}");
        let (status, poll) = json_response_from(app, &path).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(poll["data"]["state"], "confirmed");
        assert_eq!(poll["data"]["profile"]["account"], "personal");
        assert_eq!(poll["meta"]["platform"], "netease");
        assert_eq!(poll["meta"]["account"], "personal");
    }

    #[tokio::test]
    async fn password_auth_never_echoes_credentials() {
        let (status, json) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/auth/password",
            Some(json!({
                "platform": "netease",
                "account": "personal",
                "principal_type": "email",
                "principal": "private@example.test",
                "password": "must-never-appear"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["authenticated"], true);
        assert_eq!(json["data"]["account"], "personal");
        let output = serde_json::to_string(&json).expect("serialize response");
        assert!(!output.contains("private@example.test"));
        assert!(!output.contains("must-never-appear"));
    }

    #[tokio::test]
    async fn sms_challenge_verification_returns_an_authenticated_profile() {
        let app = test_app_with_provider();
        let (status, start) = json_request_from(
            app.clone(),
            Method::POST,
            "/v1/auth/challenges",
            Some(json!({
                "platform": "netease",
                "account": "sms-account",
                "method": "sms",
                "principal": "13800138000",
                "country_code": "86"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let transaction_id = start["data"]["transaction_id"]
            .as_str()
            .expect("transaction id");
        let path = format!("/v1/auth/challenges/{transaction_id}/verify");
        let (status, verified) =
            json_request_from(app, Method::POST, &path, Some(json!({ "code": "1234" }))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(verified["data"]["state"], "confirmed");
        assert_eq!(verified["data"]["profile"]["account"], "sms-account");
    }

    #[tokio::test]
    async fn auth_logout_uses_the_selected_platform_and_account() {
        let (status, json) = json_request_from(
            test_app_with_provider(),
            Method::DELETE,
            "/v1/auth/session?platform=netease&account=personal",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["removed"], true);
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "personal");
    }

    #[tokio::test]
    async fn auth_session_status_returns_only_the_selected_account_profile() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/auth/session?platform=netease&account=personal",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["authenticated"], true);
        assert_eq!(json["data"]["account"], "personal");
        assert_eq!(json["meta"]["platform"], "netease");
    }

    #[tokio::test]
    async fn auth_session_refresh_uses_a_json_body_and_returns_fresh_status() {
        let (status, json) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/auth/session/refresh",
            Some(json!({
                "platform": "netease",
                "account": "personal"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["authenticated"], true);
        assert_eq!(json["data"]["extensions"]["refreshed"], true);
        assert_eq!(json["meta"]["account"], "personal");
    }

    #[tokio::test]
    async fn account_profile_selects_platform_and_account_alias() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/account?platform=netease&account=personal",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["authenticated"], true);
        assert_eq!(json["data"]["account"], "personal");
        assert_eq!(json["meta"]["platform"], "netease");
    }

    #[tokio::test]
    async fn account_avatar_accepts_binary_images_and_reference_parameter_aliases() {
        let (status, json) = binary_request_from(
            test_app_with_provider(),
            "/v1/account/avatar?platform=netease&account=personal&filename=avatar.png&imgSize=300&imgX=1&imgY=2",
            Some("image/png"),
            vec![0x89, b'P', b'N', b'G'],
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["url"], "https://example.test/avatar.png");
        assert_eq!(json["data"]["image_id"], "109951168000000000");
        assert_eq!(json["data"]["extensions"]["filename"], "avatar.png");
        assert_eq!(json["data"]["extensions"]["content_type"], "image/png");
        assert_eq!(json["data"]["extensions"]["data_len"], 4);
        assert_eq!(json["data"]["extensions"]["image_size"], 300);
        assert_eq!(json["data"]["extensions"]["crop_x"], 1);
        assert_eq!(json["data"]["extensions"]["crop_y"], 2);
        assert_eq!(json["data"]["extensions"]["account"], "personal");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "personal");
    }

    #[tokio::test]
    async fn account_avatar_uses_safe_jpeg_defaults() {
        let (status, json) = binary_request_from(
            test_app_with_provider(),
            "/v1/account/avatar",
            None,
            vec![0xff, 0xd8, 0xff, 0xd9],
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["extensions"]["filename"], "avatar.jpg");
        assert_eq!(json["data"]["extensions"]["content_type"], "image/jpeg");
        assert_eq!(json["meta"]["account"], "default");
    }

    #[tokio::test]
    async fn account_avatar_rejects_invalid_binary_inputs() {
        for (path, content_type, body) in [
            ("/v1/account/avatar", Some("image/jpeg"), Vec::new()),
            ("/v1/account/avatar", Some("text/plain"), vec![1, 2, 3]),
            (
                "/v1/account/avatar?image_size=0",
                Some("image/jpeg"),
                vec![1, 2, 3],
            ),
        ] {
            let (status, json) =
                binary_request_from(test_app_with_provider(), path, content_type, body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(json["error"]["code"], "invalid_request");
        }
    }

    #[tokio::test]
    async fn account_avatar_size_limit_uses_the_error_envelope() {
        let (status, json) = binary_request_from(
            test_app_with_provider(),
            "/v1/account/avatar",
            Some("image/jpeg"),
            vec![0; MAX_AVATAR_UPLOAD_BYTES + 1],
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], "invalid_request");
        assert_eq!(
            json["error"]["details"]["max_bytes"],
            MAX_AVATAR_UPLOAD_BYTES
        );
    }

    #[tokio::test]
    async fn account_playlists_use_unified_pagination() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/account/playlists?platform=netease&account=personal&limit=10&offset=0",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:3778678");
        assert_eq!(json["meta"]["account"], "personal");
        assert_eq!(json["meta"]["pagination"]["limit"], 10);
        assert_eq!(json["meta"]["pagination"]["total"], 1);
    }

    #[tokio::test]
    async fn account_albums_preserve_subscription_and_page_metadata() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/account/library/albums?platform=netease&account=personal&limit=25&offset=0",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:32311");
        assert_eq!(
            json["data"][0]["extensions"]["subscription_item"]["subTime"],
            1704067200000_u64
        );
        assert_eq!(json["meta"]["account"], "personal");
        assert_eq!(json["meta"]["pagination"]["limit"], 25);
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["response"]["paidCount"],
            1
        );
    }

    #[tokio::test]
    async fn account_radio_stations_preserve_collection_and_page_metadata() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/account/library/radio-stations?platform=netease&account=personal&limit=25&offset=50",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:362");
        assert_eq!(json["data"][0]["name"], "金山区广播电视台综合广播");
        assert_eq!(json["data"][0]["region"], "上海");
        assert_eq!(json["data"][0]["subscribed"], true);
        assert_eq!(
            json["data"][0]["extensions"]["collection_item"]["collectTime"],
            1_700_000_000_000_u64
        );
        assert_eq!(json["meta"]["account"], "personal");
        assert_eq!(json["meta"]["pagination"]["limit"], 25);
        assert_eq!(json["meta"]["pagination"]["offset"], 50);
        assert_eq!(json["meta"]["pagination"]["total"], 53);
        assert_eq!(json["meta"]["pagination"]["next_offset"], 51);
        assert_eq!(json["meta"]["pagination"]["has_more"], true);
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["response"]["source"],
            "QT"
        );
    }

    #[tokio::test]
    async fn account_following_artists_use_unified_pagination() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/account/following/artists?platform=netease&account=personal&limit=2&offset=4",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:6452");
        assert_eq!(json["data"][0]["name"], "周杰伦");
        assert_eq!(
            json["data"][0]["extensions"]["following_item"]["subTime"],
            1_720_000_000_000_u64
        );
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "personal");
        assert_eq!(json["meta"]["pagination"]["limit"], 2);
        assert_eq!(json["meta"]["pagination"]["offset"], 4);
        assert_eq!(json["meta"]["pagination"]["total"], 8);
        assert_eq!(json["meta"]["pagination"]["next_offset"], 5);
        assert_eq!(json["meta"]["pagination"]["has_more"], true);
    }

    #[tokio::test]
    async fn account_artist_new_videos_use_platform_account_and_timestamp_cursor() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/account/following/artists/new-videos?platform=netease&account=personal&limit=2&before=1730000000000",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:1099001");
        assert_eq!(json["data"][0]["title"], "新 MV");
        assert_eq!(json["data"][0]["extensions"]["account"], "personal");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "personal");
        assert_eq!(json["meta"]["pagination"]["limit"], 2);
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["before_ms"],
            1_730_000_000_000_u64
        );
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["next_before_ms"],
            1_720_000_000_000_u64
        );
    }

    #[tokio::test]
    async fn account_artist_new_tracks_use_platform_account_and_timestamp_cursor() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/account/following/artists/new-tracks?platform=netease&account=personal&limit=2&before=1730000000000",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:2099001");
        assert_eq!(json["data"][0]["name"], "新歌");
        assert_eq!(json["data"][0]["extensions"]["account"], "personal");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "personal");
        assert_eq!(json["meta"]["pagination"]["limit"], 2);
        assert_eq!(json["meta"]["pagination"]["total"], 3);
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["before_ms"],
            1_730_000_000_000_u64
        );
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["next_before_ms"],
            1_720_000_000_000_u64
        );
    }

    #[tokio::test]
    async fn account_artist_new_works_keep_type_and_first_request_controls() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/account/following/artists/new-works?platform=netease&account=personal&limit=2&before=1730000000000&source_type=1&first_request=false",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["kind"], "track");
        assert_eq!(json["data"][0]["source_type"], 1);
        assert_eq!(json["data"][0]["tracks"][0]["ref"], "netease:2099001");
        assert_eq!(json["data"][0]["extensions"]["account"], "personal");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "personal");
        assert_eq!(json["meta"]["pagination"]["limit"], 2);
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["before_ms"],
            1_730_000_000_000_u64
        );
        assert_eq!(json["meta"]["pagination"]["extensions"]["source_type"], 1);
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["first_request"],
            false
        );
    }

    #[tokio::test]
    async fn account_artist_new_tracks_play_all_returns_the_fixed_snapshot() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/account/following/artists/new-tracks/play-all?platform=netease&account=personal",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:2099001");
        assert_eq!(json["data"][0]["name"], "新歌");
        assert_eq!(json["data"][0]["extensions"]["account"], "personal");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "personal");
        assert_eq!(json["meta"]["pagination"]["limit"], 50);
        assert_eq!(json["meta"]["pagination"]["total"], 1);
        assert_eq!(json["meta"]["pagination"]["has_more"], false);
    }

    #[tokio::test]
    async fn account_favorite_tracks_use_unified_pagination() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/account/favorites/tracks?platform=netease&account=personal&limit=10&offset=0",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:185809");
        assert_eq!(json["meta"]["account"], "personal");
        assert_eq!(json["meta"]["pagination"]["limit"], 10);
        assert_eq!(json["meta"]["pagination"]["total"], 1);
    }

    #[tokio::test]
    async fn user_favorite_tracks_select_reference_platform_and_account() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/users/netease:32953014/favorites/tracks?account=personal&limit=10&offset=0",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:32953014");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "personal");
        assert_eq!(json["meta"]["pagination"]["limit"], 10);
    }

    #[tokio::test]
    async fn account_history_maps_period_and_pagination() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/account/history?platform=netease&account=personal&period=week&limit=10&offset=0",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["track"]["ref"], "netease:185809");
        assert_eq!(json["data"][0]["play_count"], 42);
        assert_eq!(json["data"][0]["score"], 99);
        assert_eq!(json["meta"]["account"], "personal");
        assert_eq!(json["meta"]["pagination"]["limit"], 10);
    }

    #[tokio::test]
    async fn user_history_selects_reference_platform_and_rejects_bad_periods() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/users/netease:32953014/history?account=personal&period=all_time",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["track"]["ref"], "netease:32953014");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "personal");

        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/users/netease:32953014/history?period=month",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], "invalid_request");
    }

    #[tokio::test]
    async fn recommendation_endpoints_share_platform_account_and_pagination() {
        let (status, tracks) = json_response_from(
            test_app_with_provider(),
            "/v1/recommendations/tracks?platform=netease&account=personal&refresh=true&limit=10&offset=0",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(tracks["data"][0]["ref"], "netease:185809");
        assert_eq!(tracks["data"][0]["extensions"]["refresh"], true);
        assert_eq!(tracks["meta"]["platform"], "netease");
        assert_eq!(tracks["meta"]["account"], "personal");
        assert_eq!(tracks["meta"]["pagination"]["limit"], 10);

        let (status, playlists) = json_response_from(
            test_app_with_provider(),
            "/v1/recommendations/playlists?platform=netease&account=personal&limit=5",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(playlists["data"][0]["ref"], "netease:99");
        assert_eq!(playlists["meta"]["pagination"]["limit"], 5);
    }

    #[tokio::test]
    async fn recommendation_refresh_rejects_invalid_booleans() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/recommendations/tracks?refresh=sometimes",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], "invalid_request");
    }

    #[tokio::test]
    async fn netease_extension_api_uses_the_standard_envelope_and_account_alias() {
        let (status, json) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/extensions/netease/api",
            Some(json!({
                "uri": "/api/search/get",
                "data": { "s": "TuneWeave", "type": 1 },
                "protocol": "linuxapi",
                "account": "green-diamond"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["ok"], true);
        assert_eq!(json["data"]["code"], 200);
        assert_eq!(json["data"]["uri"], "/api/search/get");
        assert_eq!(json["data"]["data"]["s"], "TuneWeave");
        assert_eq!(json["data"]["crypto"], "linuxapi");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "green-diamond");
    }

    #[tokio::test]
    async fn netease_extension_api_defaults_to_an_empty_data_object() {
        let (status, json) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/extensions/netease/api",
            Some(json!({ "uri": "/api/logout" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["data"], json!({}));
        assert!(json["data"]["crypto"].is_null());
        assert!(json["meta"].get("account").is_none());
    }

    #[tokio::test]
    async fn netease_extension_api_rejects_transport_and_credential_overrides() {
        for field in ["cookie", "domain", "headers", "proxy", "ua"] {
            let mut body = serde_json::Map::from_iter([(
                "uri".to_owned(),
                Value::String("/api/search/get".to_owned()),
            )]);
            body.insert(field.to_owned(), Value::String("forbidden".to_owned()));
            let (status, json) = json_request_from(
                test_app_with_provider(),
                Method::POST,
                "/v1/extensions/netease/api",
                Some(Value::Object(body)),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{field}");
            assert_eq!(json["error"]["code"], "invalid_request", "{field}");
        }
    }

    #[tokio::test]
    async fn netease_batch_post_accepts_container_and_reference_dynamic_fields() {
        let (status, json) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/extensions/netease/batch",
            Some(json!({
                "requests": {
                    "/api/v2/banner/get": { "clientType": "pc" }
                },
                "/api/search/get": "{\"s\":\"TuneWeave\",\"type\":1}",
                "protocol": "weapi",
                "e_r": "1",
                "account": "batch-user"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["ok"], true);
        assert_eq!(
            json["data"]["requests"]["/api/v2/banner/get"]["clientType"],
            "pc"
        );
        assert_eq!(
            json["data"]["requests"]["/api/search/get"],
            "{\"s\":\"TuneWeave\",\"type\":1}"
        );
        assert_eq!(json["data"]["crypto"], "weapi");
        assert_eq!(json["data"]["encrypted_response"], true);
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "batch-user");
    }

    #[tokio::test]
    async fn netease_batch_get_accepts_reference_query_shape() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/extensions/netease/batch?%2Fapi%2Fv2%2Fbanner%2Fget=%7B%22clientType%22%3A%22pc%22%7D&crypto=eapi&e_r=true&account=query-user",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            json["data"]["requests"]["/api/v2/banner/get"],
            "{\"clientType\":\"pc\"}"
        );
        assert_eq!(json["data"]["crypto"], "eapi");
        assert_eq!(json["data"]["encrypted_response"], true);
        assert_eq!(json["meta"]["account"], "query-user");
    }

    #[tokio::test]
    async fn netease_batch_rejects_empty_duplicate_and_transport_fields() {
        for body in [
            json!({}),
            json!({
                "requests": { "/api/search/get": {} },
                "/api/search/get": {}
            }),
            json!({ "cookie": "MUSIC_U=raw-secret" }),
            json!({ "domain": "https://example.com" }),
            json!({ "proxy": "http://example.com" }),
            json!({ "headers": { "Cookie": "raw-secret" } }),
            json!({ "realIP": "127.0.0.1" }),
            json!({
                "requests": { "/api/search/get": {} },
                "encrypted_response": 2
            }),
        ] {
            let (status, json) = json_request_from(
                test_app_with_provider(),
                Method::POST,
                "/v1/extensions/netease/batch",
                Some(body),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(json["error"]["code"], "invalid_request");
        }
    }

    #[tokio::test]
    async fn malformed_auth_json_uses_the_error_envelope() {
        let response = test_app_with_provider()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/auth/password")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from("{"))
                    .expect("build malformed request"),
            )
            .await
            .expect("request succeeds");
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("read body");
        let json: Value = serde_json::from_slice(&body).expect("valid error JSON");
        assert_eq!(json["ok"], false);
        assert_eq!(json["error"]["code"], "invalid_request");
    }

    #[tokio::test]
    async fn unknown_auth_transaction_uses_the_error_envelope() {
        let (status, json) =
            json_response_from(test_app_with_provider(), "/v1/auth/qr/tw-auth-missing").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(json["error"]["code"], "resource_not_found");
    }
}
