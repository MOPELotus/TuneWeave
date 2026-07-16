mod response;

use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, RwLock},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
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
    AudioRecognition, AudioRecognitionRequest, AuthChallengeRequest, AuthChallengeValidation,
    AuthPrincipalStatus, AuthPrincipalStatusRequest, AuthState, Banner, BannerClient,
    BannerListRequest, Capability, ChallengeMethod, CloudImportRequest, CloudImportResult,
    CloudLyricsRequest, CloudMatchRequest, CloudMatchResult, CloudUploadCompleteRequest,
    CloudUploadRequest, CloudUploadResult, CloudUploadTicket, CloudUploadTicketRequest, Comment,
    CommentDeleteRequest, CommentListRequest, CommentListView, CommentMutationResult, CommentPage,
    CommentReaction, CommentReactionKind, CommentReactionListRequest, CommentReactionPage,
    CommentSort, CommentTarget, CommentTargetKind, CommentThreadStatsBatch,
    CommentThreadStatsRequest, CommentWriteRequest, DigitalAlbum, DigitalAlbumChartEntry,
    DigitalAlbumChartKind, DigitalAlbumChartPeriod, DigitalAlbumChartRequest,
    DigitalAlbumListRequest, DimensionChart, DimensionChartRequest, DimensionChartTrackSnapshot,
    Extensions, ImageUploadRequest, ImageUploadResult, Lyrics, MediaStream, PageRequest,
    PasswordFormat, PasswordLoginRequest, Platform, PlatformApiRequest, PlatformBatchRequest,
    PlaybackHistoryEntry, PlaybackHistoryPeriod, PlaybackHistoryRequest, Playlist, PrincipalType,
    ProviderRegistry, Quality, RadioStation, RadioStationCursor, RadioStationListRequest,
    RadioTaxonomy, RadioTaxonomyRequest, RecommendationRequest, ResolveRequest, ResourceRef,
    SearchItem, SearchKind, SearchQuery, StreamResolver, SubscriptionResult, Track,
    TrackAvailability, TrackAvailabilityRequest, TrackEntitlement, TuneWeaveError, User, Video,
    VideoKind,
};

pub use response::{ApiError, ApiResponse, ResponseMeta};

const AUTH_TRANSACTION_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_AVATAR_UPLOAD_BYTES: usize = 20 * 1024 * 1024;
const MAX_CLOUD_PROXY_UPLOAD_BYTES: usize = 500 * 1024 * 1024;

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
        .route("/radio/stations", get(radio_stations))
        .route("/radio/stations/{reference}", get(radio_station))
        .route("/audio/recognize", post(audio_recognize))
        .route("/tracks/{reference}", get(track))
        .route("/tracks/{reference}/availability", get(track_availability))
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
        .route("/charts/dimensions/{chart_code}", get(dimension_chart))
        .route(
            "/charts/dimensions/{chart_code}/tracks",
            get(dimension_chart_tracks),
        )
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
        .route(
            "/account/library/radio-stations/{reference}",
            put(radio_station_subscribe).delete(radio_station_unsubscribe),
        )
        .route("/tracks/{reference}/lyrics", get(track_lyrics))
        .route("/tracks/{reference}/stream", get(track_stream))
        .route("/playlists/{reference}", get(playlist))
        .route("/playlists/{reference}/tracks", get(playlist_tracks))
        .route(
            "/resources/{kind}/comments/stats",
            get(comment_thread_stats),
        )
        .route(
            "/resources/{kind}/{reference}/comments",
            get(comment_list).post(comment_create),
        )
        .route(
            "/resources/{kind}/{reference}/comments/{comment_id}",
            axum::routing::delete(comment_delete),
        )
        .route(
            "/resources/{kind}/{reference}/comments/{comment_id}/replies",
            post(comment_reply),
        )
        .route(
            "/resources/{kind}/{reference}/comments/{comment_id}/reactions/{reaction}",
            get(comment_reaction_list),
        )
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
        .route("/auth/challenges/validate", post(auth_challenge_validate))
        .route("/auth/principals/status", post(auth_principal_status))
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
        .route(
            "/account/cloud/uploads",
            post(cloud_upload).layer(DefaultBodyLimit::max(MAX_CLOUD_PROXY_UPLOAD_BYTES)),
        )
        .route("/account/cloud/uploads/ticket", post(cloud_upload_ticket))
        .route(
            "/account/cloud/uploads/complete",
            post(cloud_upload_complete),
        )
        .route("/account/cloud/imports", post(cloud_import))
        .route("/account/cloud/lyrics", get(cloud_lyrics))
        .route("/account/cloud/matches", post(cloud_match))
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
        .route("/extensions/netease/calendar", get(netease_calendar))
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
    #[serde(alias = "keywords")]
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
) -> Result<Json<ApiResponse<Vec<SearchItem>>>, ApiError> {
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
    let page = provider.search_catalog(&query).await?;
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

#[derive(Debug, Default, Deserialize)]
struct RadioStationListParams {
    platform: Option<String>,
    account: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
    #[serde(alias = "categoryId")]
    category_id: Option<String>,
    #[serde(alias = "regionId")]
    region_id: Option<String>,
    #[serde(alias = "lastId")]
    last_id: Option<String>,
    score: Option<String>,
}

async fn radio_stations(
    State(state): State<AppState>,
    Query(params): Query<RadioStationListParams>,
) -> Result<Json<ApiResponse<Vec<RadioStation>>>, ApiError> {
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 20)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let category_id = optional_trimmed(params.category_id);
    let region_id = optional_trimmed(params.region_id);
    let last_id = optional_trimmed(params.last_id);
    let score = parse_optional_i64_parameter("score", params.score.as_deref())?;
    let cursor = if last_id.is_some() || score.is_some() {
        Some(RadioStationCursor {
            id: last_id.unwrap_or_else(|| "0".to_owned()),
            score: score.unwrap_or(-1),
        })
    } else {
        None
    };
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = optional_trimmed(params.account);
    let provider = state.registry.require(platform)?;
    let page = provider
        .radio_stations(&RadioStationListRequest {
            limit,
            offset,
            category_id,
            region_id,
            cursor,
            account: account.clone(),
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
struct TrackAvailabilityParams {
    account: Option<String>,
    #[serde(alias = "br")]
    bitrate: Option<String>,
}

async fn track_availability(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<TrackAvailabilityParams>,
) -> Result<Json<ApiResponse<TrackAvailability>>, ApiError> {
    let reference = parse_reference(reference)?;
    let bitrate = parse_optional_u64_parameter("bitrate", params.bitrate.as_deref())?
        .unwrap_or(TrackAvailabilityRequest::DEFAULT_BITRATE);
    let account = optional_trimmed(params.account);
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let availability = provider
        .track_availability(
            reference.id(),
            &TrackAvailabilityRequest {
                bitrate,
                account: account.clone(),
            },
        )
        .await?;
    let mut response = ApiResponse::new(availability).with_platform(platform);
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

#[derive(Debug, Default, Deserialize)]
struct DimensionChartParams {
    platform: Option<String>,
    account: Option<String>,
    #[serde(alias = "targetId")]
    target_id: Option<String>,
    #[serde(alias = "targetType")]
    target_type: Option<String>,
}

async fn dimension_chart(
    State(state): State<AppState>,
    Path(chart_code): Path<String>,
    Query(params): Query<DimensionChartParams>,
) -> Result<Json<ApiResponse<DimensionChart>>, ApiError> {
    let (platform, account, request) = dimension_chart_request(&state, chart_code, params)?;
    let provider = state.registry.require(platform)?;
    let chart = provider.dimension_chart(&request).await?;
    let mut response = ApiResponse::new(chart).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

async fn dimension_chart_tracks(
    State(state): State<AppState>,
    Path(chart_code): Path<String>,
    Query(params): Query<DimensionChartParams>,
) -> Result<Json<ApiResponse<DimensionChartTrackSnapshot>>, ApiError> {
    let (platform, account, request) = dimension_chart_request(&state, chart_code, params)?;
    let provider = state.registry.require(platform)?;
    let snapshot = provider.dimension_chart_tracks(&request).await?;
    let mut response = ApiResponse::new(snapshot).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

fn dimension_chart_request(
    state: &AppState,
    chart_code: String,
    params: DimensionChartParams,
) -> Result<(Platform, Option<String>, DimensionChartRequest), TuneWeaveError> {
    let platform = account_platform(state, params.platform.as_deref())?;
    let account = optional_trimmed(params.account);
    let mut request = DimensionChartRequest::new(
        required_trimmed("chart_code", Some(chart_code))?,
        required_trimmed("target_id", params.target_id)?,
        required_trimmed("target_type", params.target_type)?,
    );
    request.account.clone_from(&account);
    Ok((platform, account, request))
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

async fn radio_station_subscribe(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<SubscriptionResult>>, ApiError> {
    set_radio_station_subscription(state, reference, params, true).await
}

async fn radio_station_unsubscribe(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<SubscriptionResult>>, ApiError> {
    set_radio_station_subscription(state, reference, params, false).await
}

async fn set_radio_station_subscription(
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
        .set_radio_station_subscription(reference.id(), subscribed, Some(&account))
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthChallengeValidationBody {
    platform: String,
    account: Option<String>,
    method: Option<ChallengeMethod>,
    #[serde(alias = "phone")]
    principal: Value,
    #[serde(alias = "captcha")]
    code: String,
    #[serde(default, alias = "ctcode", alias = "countrycode")]
    country_code: Option<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AuthPrincipalStatusBody {
    platform: String,
    account: Option<String>,
    principal_type: Option<PrincipalType>,
    #[serde(alias = "phone")]
    principal: Value,
    #[serde(default, alias = "countrycode", alias = "countryCode")]
    country_code: Option<Value>,
}

async fn auth_principal_status(
    State(state): State<AppState>,
    payload: Result<Json<AuthPrincipalStatusBody>, JsonRejection>,
) -> Result<Json<ApiResponse<AuthPrincipalStatus>>, ApiError> {
    let body = json_body(payload)?;
    let platform = parse_platform_parameter(&body.platform)?;
    let account = account_alias(body.account.as_deref())?;
    let principal = required_string_or_number("principal", &body.principal)?;
    let country_code = match body.country_code.as_ref() {
        None => "86".to_owned(),
        Some(Value::String(value)) if value.trim().is_empty() => "86".to_owned(),
        Some(value) => required_string_or_number("country_code", value)?,
    };
    let provider = state.registry.require(platform)?;
    let status = provider
        .auth_principal_status(&AuthPrincipalStatusRequest {
            account: account.clone(),
            principal_type: body.principal_type.unwrap_or(PrincipalType::Phone),
            principal,
            country_code: Some(country_code),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(status)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn auth_challenge_validate(
    State(state): State<AppState>,
    payload: Result<Json<AuthChallengeValidationBody>, JsonRejection>,
) -> Result<Json<ApiResponse<AuthChallengeValidation>>, ApiError> {
    let body = json_body(payload)?;
    let platform = parse_platform_parameter(&body.platform)?;
    let account = account_alias(body.account.as_deref())?;
    let principal = required_string_or_number("principal", &body.principal)?;
    let code = body.code.trim();
    if code.is_empty() {
        return Err(TuneWeaveError::invalid_request("code must not be empty").into());
    }
    let country_code = match body.country_code.as_ref() {
        None => "86".to_owned(),
        Some(Value::String(value)) if value.trim().is_empty() => "86".to_owned(),
        Some(value) => required_string_or_number("country_code", value)?,
    };
    let provider = state.registry.require(platform)?;
    let validation = provider
        .validate_auth_challenge(
            &AuthChallengeRequest {
                account: account.clone(),
                method: body.method.unwrap_or(ChallengeMethod::Sms),
                principal,
                country_code: Some(country_code),
            },
            code,
        )
        .await?;
    Ok(Json(
        ApiResponse::new(validation)
            .with_platform(platform)
            .with_account(account),
    ))
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

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloudAccountQuery {
    platform: Option<String>,
    account: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct CloudUploadParams {
    platform: Option<String>,
    account: Option<String>,
    #[serde(alias = "fileName")]
    filename: Option<String>,
    bitrate: Option<String>,
    #[serde(alias = "song", alias = "songName")]
    song_name: Option<String>,
    artist: Option<String>,
    album: Option<String>,
}

fn validate_cloud_proxy_upload_size(size: usize) -> Result<(), TuneWeaveError> {
    if size == 0 {
        return Err(TuneWeaveError::invalid_request(
            "audio body must not be empty",
        ));
    }
    if size > MAX_CLOUD_PROXY_UPLOAD_BYTES {
        return Err(
            TuneWeaveError::invalid_request("audio body exceeds 500 MiB")
                .with_details(json!({ "max_bytes": MAX_CLOUD_PROXY_UPLOAD_BYTES })),
        );
    }
    Ok(())
}

async fn cloud_upload(
    State(state): State<AppState>,
    Query(params): Query<CloudUploadParams>,
    headers: HeaderMap,
    payload: Result<Bytes, BytesRejection>,
) -> Result<Json<ApiResponse<CloudUploadResult>>, ApiError> {
    let data = payload.map_err(|_| {
        TuneWeaveError::invalid_request("audio body is invalid or exceeds 500 MiB")
            .with_details(json!({ "max_bytes": MAX_CLOUD_PROXY_UPLOAD_BYTES }))
    })?;
    validate_cloud_proxy_upload_size(data.len())?;
    let filename = required_trimmed("filename", params.filename)?;
    let bitrate = parse_optional_u64_parameter("bitrate", params.bitrate.as_deref())?
        .unwrap_or(CloudUploadRequest::DEFAULT_BITRATE);
    if bitrate == 0 {
        return Err(TuneWeaveError::invalid_request("bitrate must be greater than zero").into());
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .map(|value| {
            value
                .to_str()
                .map(str::trim)
                .map(str::to_owned)
                .map_err(|_| TuneWeaveError::invalid_request("Content-Type is not valid text"))
        })
        .transpose()?
        .unwrap_or_default();
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .upload_cloud_track(&CloudUploadRequest {
            filename,
            content_type,
            data: data.into(),
            bitrate,
            song_name: optional_trimmed(params.song_name),
            artist: optional_trimmed(params.artist),
            album: optional_trimmed(params.album),
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloudUploadTicketBody {
    md5: String,
    #[serde(alias = "fileSize")]
    file_size: u64,
    filename: String,
    bitrate: Option<u64>,
    #[serde(alias = "contentType")]
    content_type: Option<String>,
}

async fn cloud_upload_ticket(
    State(state): State<AppState>,
    Query(params): Query<CloudAccountQuery>,
    payload: Result<Json<CloudUploadTicketBody>, JsonRejection>,
) -> Result<Json<ApiResponse<CloudUploadTicket>>, ApiError> {
    let body = json_body(payload)?;
    let bitrate = body.bitrate.unwrap_or(CloudUploadRequest::DEFAULT_BITRATE);
    if bitrate == 0 {
        return Err(TuneWeaveError::invalid_request("bitrate must be greater than zero").into());
    }
    if body.file_size == 0 {
        return Err(TuneWeaveError::invalid_request("file_size must be greater than zero").into());
    }
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let ticket = provider
        .cloud_upload_ticket(&CloudUploadTicketRequest {
            md5: body.md5,
            file_size: body.file_size,
            filename: body.filename,
            bitrate,
            content_type: optional_trimmed(body.content_type),
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(ticket)
            .with_platform(platform)
            .with_account(account),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloudUploadCompleteBody {
    #[serde(alias = "songId")]
    provisional_track_id: String,
    #[serde(alias = "resourceId")]
    resource_id: String,
    md5: String,
    filename: String,
    #[serde(alias = "song")]
    song_name: Option<String>,
    artist: Option<String>,
    album: Option<String>,
    bitrate: Option<u64>,
}

async fn cloud_upload_complete(
    State(state): State<AppState>,
    Query(params): Query<CloudAccountQuery>,
    payload: Result<Json<CloudUploadCompleteBody>, JsonRejection>,
) -> Result<Json<ApiResponse<CloudUploadResult>>, ApiError> {
    let body = json_body(payload)?;
    let bitrate = body.bitrate.unwrap_or(CloudUploadRequest::DEFAULT_BITRATE);
    if bitrate == 0 {
        return Err(TuneWeaveError::invalid_request("bitrate must be greater than zero").into());
    }
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .complete_cloud_upload(&CloudUploadCompleteRequest {
            provisional_track_id: body.provisional_track_id,
            resource_id: body.resource_id,
            md5: body.md5,
            filename: body.filename,
            song_name: optional_trimmed(body.song_name),
            artist: optional_trimmed(body.artist),
            album: optional_trimmed(body.album),
            bitrate,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloudImportBody {
    md5: String,
    #[serde(alias = "id")]
    source_track_id: Option<Value>,
    bitrate: Value,
    #[serde(alias = "fileSize")]
    file_size: Value,
    #[serde(alias = "fileType")]
    file_type: String,
    #[serde(alias = "song")]
    song_name: String,
    artist: Option<String>,
    album: Option<String>,
}

async fn cloud_import(
    State(state): State<AppState>,
    Query(params): Query<CloudAccountQuery>,
    payload: Result<Json<CloudImportBody>, JsonRejection>,
) -> Result<Json<ApiResponse<CloudImportResult>>, ApiError> {
    let body = json_body(payload)?;
    let source_track_id = body
        .source_track_id
        .as_ref()
        .map(|value| required_string_or_number("source_track_id", value))
        .transpose()?;
    let bitrate = required_json_u64("bitrate", &body.bitrate)?;
    let file_size = required_json_u64("file_size", &body.file_size)?;
    if bitrate < 1_000 {
        return Err(TuneWeaveError::invalid_request("bitrate must be at least 1000 bit/s").into());
    }
    if file_size == 0 {
        return Err(TuneWeaveError::invalid_request("file_size must be greater than zero").into());
    }
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .import_cloud_track(&CloudImportRequest {
            md5: body.md5,
            source_track_id,
            bitrate,
            file_size,
            file_type: body.file_type,
            song_name: body.song_name,
            artist: body.artist.unwrap_or_default(),
            album: body.album.unwrap_or_default(),
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloudLyricsQuery {
    platform: Option<String>,
    account: Option<String>,
    #[serde(alias = "uid")]
    user_id: Option<String>,
    #[serde(alias = "sid")]
    track_id: Option<String>,
}

async fn cloud_lyrics(
    State(state): State<AppState>,
    Query(params): Query<CloudLyricsQuery>,
) -> Result<Json<ApiResponse<Lyrics>>, ApiError> {
    let user_id = required_trimmed("user_id", params.user_id)?;
    let track_id = required_trimmed("track_id", params.track_id)?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let lyrics = provider
        .cloud_lyrics(&CloudLyricsRequest {
            user_id,
            track_id,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(lyrics)
            .with_platform(platform)
            .with_account(account),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloudMatchBody {
    #[serde(alias = "uid")]
    user_id: Value,
    #[serde(alias = "sid")]
    cloud_track_id: Value,
    #[serde(alias = "asid")]
    target_track_id: Option<Value>,
}

async fn cloud_match(
    State(state): State<AppState>,
    Query(params): Query<CloudAccountQuery>,
    payload: Result<Json<CloudMatchBody>, JsonRejection>,
) -> Result<Json<ApiResponse<CloudMatchResult>>, ApiError> {
    let body = json_body(payload)?;
    let user_id = required_string_or_number("user_id", &body.user_id)?;
    let cloud_track_id = required_string_or_number("cloud_track_id", &body.cloud_track_id)?;
    let target_track_id = body
        .target_track_id
        .as_ref()
        .map(|value| required_string_or_number("target_track_id", value))
        .transpose()?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .match_cloud_track(&CloudMatchRequest {
            user_id,
            cloud_track_id,
            target_track_id,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

#[derive(Debug, Default, Deserialize)]
struct CommentThreadStatsQuery {
    platform: Option<String>,
    account: Option<String>,
    ids: Option<String>,
    id: Option<String>,
}

async fn comment_thread_stats(
    State(state): State<AppState>,
    Path(kind): Path<String>,
    Query(params): Query<CommentThreadStatsQuery>,
) -> Result<Json<ApiResponse<CommentThreadStatsBatch>>, ApiError> {
    let platform = search_platform(&state, params.platform.as_deref())?;
    let kind = parse_comment_target_kind(&kind)?;
    let ids = optional_trimmed(params.ids)
        .or_else(|| optional_trimmed(params.id))
        .unwrap_or_default();
    let resource_refs = ids
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(|id| {
            ResourceRef::new(platform, id).map_err(|error| {
                TuneWeaveError::invalid_request(format!("invalid comment stats id: {error}"))
                    .with_details(json!({ "id": id, "platform": platform }))
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let account = optional_trimmed(params.account);
    let provider = state.registry.require(platform)?;
    let batch = provider
        .comment_thread_stats(&CommentThreadStatsRequest {
            kind,
            resource_refs,
            account: account.clone(),
        })
        .await?;
    let mut response = ApiResponse::new(batch).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

#[derive(Debug, Default, Deserialize)]
struct CommentListQuery {
    account: Option<String>,
    view: Option<String>,
    #[serde(alias = "sortType")]
    sort: Option<String>,
    #[serde(alias = "pageSize")]
    limit: Option<String>,
    offset: Option<String>,
    #[serde(alias = "pageNo")]
    page: Option<String>,
    cursor: Option<String>,
    #[serde(alias = "before", alias = "beforeTime", alias = "time")]
    before_time_ms: Option<String>,
    #[serde(alias = "parentCommentId")]
    parent_comment_id: Option<String>,
    #[serde(alias = "showInner")]
    include_replies: Option<String>,
}

#[derive(Debug, Serialize)]
struct CommentPageData {
    target: CommentTarget,
    comments: Vec<Comment>,
    hot_comments: Vec<Comment>,
    top_comments: Vec<Comment>,
    current_comment: Option<Comment>,
    extensions: Extensions,
}

async fn comment_list(
    State(state): State<AppState>,
    Path((kind, reference)): Path<(String, String)>,
    Query(params): Query<CommentListQuery>,
) -> Result<Json<ApiResponse<CommentPageData>>, ApiError> {
    let target = parse_comment_target(&kind, reference)?;
    let platform = target.resource_ref.platform();
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 20)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let page = parse_optional_u32_parameter("page", params.page.as_deref())?;
    if page == Some(0) {
        return Err(TuneWeaveError::invalid_request("page must be greater than zero").into());
    }
    let before_time_ms =
        parse_optional_u64_parameter("before_time_ms", params.before_time_ms.as_deref())?;
    let parent_comment_id = optional_trimmed(params.parent_comment_id);
    let view = parse_comment_list_view(params.view.as_deref(), parent_comment_id.is_some())?;
    let sort = parse_comment_sort(params.sort.as_deref())?;
    let cursor = optional_trimmed(params.cursor);
    validate_comment_list_options(
        view,
        sort,
        page,
        cursor.as_deref(),
        before_time_ms,
        parent_comment_id.as_deref(),
    )?;
    let include_replies =
        parse_bool_parameter("include_replies", params.include_replies.as_deref(), true)?;
    let account = optional_trimmed(params.account);
    let provider = state.registry.require(platform)?;
    let page_result = provider
        .comments(&CommentListRequest {
            target,
            view,
            sort,
            limit,
            offset,
            page,
            cursor,
            before_time_ms,
            parent_comment_id,
            include_replies,
            account: account.clone(),
        })
        .await?;
    let CommentPage {
        target,
        comments,
        hot_comments,
        top_comments,
        current_comment,
        pagination,
        extensions,
    } = page_result;
    let mut response = ApiResponse::new(CommentPageData {
        target,
        comments,
        hot_comments,
        top_comments,
        current_comment,
        extensions,
    })
    .with_platform(platform)
    .with_pagination(pagination);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

#[derive(Debug, Default, Deserialize)]
struct CommentReactionListQuery {
    account: Option<String>,
    #[serde(alias = "targetUserRef")]
    target_user_ref: Option<String>,
    #[serde(alias = "targetUserId", alias = "uid")]
    target_user_id: Option<String>,
    #[serde(alias = "pageSize")]
    limit: Option<String>,
    #[serde(alias = "pageNo")]
    page: Option<String>,
    cursor: Option<String>,
    #[serde(alias = "idCursor")]
    id_cursor: Option<String>,
}

#[derive(Debug, Serialize)]
struct CommentReactionPageData {
    target: CommentTarget,
    comment_id: String,
    target_user_ref: ResourceRef,
    kind: CommentReactionKind,
    reactions: Vec<CommentReaction>,
    current_comment: Option<Comment>,
    extensions: Extensions,
}

async fn comment_reaction_list(
    State(state): State<AppState>,
    Path((kind, reference, comment_id, reaction)): Path<(String, String, String, String)>,
    Query(params): Query<CommentReactionListQuery>,
) -> Result<Json<ApiResponse<CommentReactionPageData>>, ApiError> {
    let target = parse_comment_target(&kind, reference)?;
    let platform = target.resource_ref.platform();
    let comment_id = validate_comment_id("comment_id", &comment_id)?;
    let reaction = parse_comment_reaction_kind(&reaction)?;
    let target_user_ref = parse_comment_reaction_target_user(
        platform,
        optional_trimmed(params.target_user_ref),
        optional_trimmed(params.target_user_id),
    )?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 100)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let page = parse_u32_parameter("page", params.page.as_deref(), 1)?;
    if page == 0 {
        return Err(TuneWeaveError::invalid_request("page must be greater than zero").into());
    }
    let cursor = optional_trimmed(params.cursor);
    let id_cursor = optional_trimmed(params.id_cursor);
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let page_result = provider
        .comment_reactions(&CommentReactionListRequest {
            target,
            comment_id,
            target_user_ref,
            kind: reaction,
            limit,
            page,
            cursor,
            id_cursor,
            account: Some(account.clone()),
        })
        .await?;
    let CommentReactionPage {
        target,
        comment_id,
        target_user_ref,
        kind,
        reactions,
        current_comment,
        pagination,
        extensions,
    } = page_result;
    Ok(Json(
        ApiResponse::new(CommentReactionPageData {
            target,
            comment_id,
            target_user_ref,
            kind,
            reactions,
            current_comment,
            extensions,
        })
        .with_platform(platform)
        .with_account(account)
        .with_pagination(pagination),
    ))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommentAccountQuery {
    account: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommentContentBody {
    content: String,
}

async fn comment_create(
    State(state): State<AppState>,
    Path((kind, reference)): Path<(String, String)>,
    Query(params): Query<CommentAccountQuery>,
    payload: Result<Json<CommentContentBody>, JsonRejection>,
) -> Result<Json<ApiResponse<CommentMutationResult>>, ApiError> {
    let body = json_body(payload)?;
    validate_comment_content(&body.content)?;
    let target = parse_comment_target(&kind, reference)?;
    let platform = target.resource_ref.platform();
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .post_comment(&CommentWriteRequest {
            target,
            content: body.content,
            reply_to: None,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn comment_reply(
    State(state): State<AppState>,
    Path((kind, reference, comment_id)): Path<(String, String, String)>,
    Query(params): Query<CommentAccountQuery>,
    payload: Result<Json<CommentContentBody>, JsonRejection>,
) -> Result<Json<ApiResponse<CommentMutationResult>>, ApiError> {
    let body = json_body(payload)?;
    validate_comment_content(&body.content)?;
    let comment_id = validate_comment_id("comment_id", &comment_id)?;
    let target = parse_comment_target(&kind, reference)?;
    let platform = target.resource_ref.platform();
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .post_comment(&CommentWriteRequest {
            target,
            content: body.content,
            reply_to: Some(comment_id),
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn comment_delete(
    State(state): State<AppState>,
    Path((kind, reference, comment_id)): Path<(String, String, String)>,
    Query(params): Query<CommentAccountQuery>,
) -> Result<Json<ApiResponse<CommentMutationResult>>, ApiError> {
    let comment_id = validate_comment_id("comment_id", &comment_id)?;
    let target = parse_comment_target(&kind, reference)?;
    let platform = target.resource_ref.platform();
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .delete_comment(&CommentDeleteRequest {
            target,
            comment_id,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

fn validate_comment_content(content: &str) -> Result<(), TuneWeaveError> {
    if content.trim().is_empty() {
        return Err(TuneWeaveError::invalid_request(
            "comment content cannot be empty",
        ));
    }
    Ok(())
}

fn validate_comment_id(field: &str, value: &str) -> Result<String, TuneWeaveError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(TuneWeaveError::invalid_request(format!(
            "{field} cannot be empty"
        )));
    }
    Ok(value.to_owned())
}

fn validate_comment_list_options(
    view: CommentListView,
    sort: Option<CommentSort>,
    page: Option<u32>,
    cursor: Option<&str>,
    before_time_ms: Option<u64>,
    parent_comment_id: Option<&str>,
) -> Result<(), TuneWeaveError> {
    match view {
        CommentListView::All if sort.is_none() => {
            if page.is_some() || cursor.is_some() || parent_comment_id.is_some() {
                return Err(TuneWeaveError::invalid_request(
                    "page and cursor require sort; parent_comment_id requires view=replies",
                ));
            }
        }
        CommentListView::All => {
            if before_time_ms.is_some() || parent_comment_id.is_some() {
                return Err(TuneWeaveError::invalid_request(
                    "sorted comments do not accept before_time_ms or parent_comment_id",
                ));
            }
            if sort != Some(CommentSort::Time) && cursor.is_some() {
                return Err(TuneWeaveError::invalid_request(
                    "cursor is only accepted with sort=time",
                ));
            }
        }
        CommentListView::Hot => {
            if sort.is_some() || page.is_some() || cursor.is_some() || parent_comment_id.is_some() {
                return Err(TuneWeaveError::invalid_request(
                    "view=hot does not accept sort, page, cursor, or parent_comment_id",
                ));
            }
        }
        CommentListView::Replies => {
            if parent_comment_id.is_none() {
                return Err(TuneWeaveError::invalid_request(
                    "parent_comment_id is required for view=replies",
                ));
            }
            if sort.is_some() || page.is_some() || cursor.is_some() {
                return Err(TuneWeaveError::invalid_request(
                    "view=replies does not accept sort, page, or cursor",
                ));
            }
        }
    }
    Ok(())
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

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct NeteaseCalendarQuery {
    #[serde(alias = "startTime")]
    start_time: Option<String>,
    #[serde(alias = "endTime")]
    end_time: Option<String>,
    account: Option<String>,
}

async fn netease_calendar(
    State(state): State<AppState>,
    Query(params): Query<NeteaseCalendarQuery>,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    let now = unix_time_millis()?;
    let start_time =
        parse_optional_u64_parameter("start_time", params.start_time.as_deref())?.unwrap_or(now);
    let end_time =
        parse_optional_u64_parameter("end_time", params.end_time.as_deref())?.unwrap_or(now);
    let account = optional_trimmed(params.account);
    let provider = state.registry.require(Platform::Netease)?;
    let data = provider
        .platform_api(&PlatformApiRequest {
            uri: "/api/mcalendar/detail".to_owned(),
            data: json!({
                "startTime": start_time,
                "endTime": end_time,
            }),
            protocol: Some("weapi".to_owned()),
            account: account.clone(),
        })
        .await?;
    let mut response = ApiResponse::new(data).with_platform(Platform::Netease);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
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

fn required_trimmed(name: &str, value: Option<String>) -> Result<String, TuneWeaveError> {
    optional_trimmed(value)
        .ok_or_else(|| TuneWeaveError::invalid_request(format!("{name} must not be empty")))
}

fn required_string_or_number(name: &str, value: &Value) -> Result<String, TuneWeaveError> {
    let value = match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => {
            return Err(TuneWeaveError::invalid_request(format!(
                "{name} must be a string or number"
            )));
        }
    };
    let value = value.trim();
    if value.is_empty() {
        return Err(TuneWeaveError::invalid_request(format!(
            "{name} must not be empty"
        )));
    }
    Ok(value.to_owned())
}

fn required_json_u64(name: &str, value: &Value) -> Result<u64, TuneWeaveError> {
    let value = required_string_or_number(name, value)?;
    value.parse().map_err(|_| {
        TuneWeaveError::invalid_request(format!("{name} must be an unsigned integer"))
            .with_details(json!({ "parameter": name, "value": value }))
    })
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

fn parse_comment_target(kind: &str, reference: String) -> Result<CommentTarget, TuneWeaveError> {
    Ok(CommentTarget::new(
        parse_reference(reference)?,
        parse_comment_target_kind(kind)?,
    ))
}

fn parse_comment_target_kind(value: &str) -> Result<CommentTargetKind, TuneWeaveError> {
    let value = value.trim().to_ascii_lowercase().replace('-', "_");
    match value.as_str() {
        "track" | "song" | "music" | "0" => Ok(CommentTargetKind::Track),
        "mv" | "1" => Ok(CommentTargetKind::Mv),
        "playlist" | "2" => Ok(CommentTargetKind::Playlist),
        "album" | "3" => Ok(CommentTargetKind::Album),
        "radio_episode" | "episode" | "program" | "dj" | "4" => Ok(CommentTargetKind::RadioEpisode),
        "video" | "5" => Ok(CommentTargetKind::Video),
        "event" | "6" => Ok(CommentTargetKind::Event),
        "radio_station" | "station" | "7" => Ok(CommentTargetKind::RadioStation),
        value => Err(TuneWeaveError::invalid_request(format!(
            "unsupported comment target type: {value}"
        ))
        .with_details(json!({
            "allowed": [
                "track", "mv", "playlist", "album", "radio_episode", "video",
                "event", "radio_station", 0, 1, 2, 3, 4, 5, 6, 7
            ]
        }))),
    }
}

fn parse_comment_reaction_kind(value: &str) -> Result<CommentReactionKind, TuneWeaveError> {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "hug" => Ok(CommentReactionKind::Hug),
        "like" => Ok(CommentReactionKind::Like),
        value => Err(TuneWeaveError::invalid_request(format!(
            "unsupported comment reaction: {value}"
        ))
        .with_details(json!({ "allowed": ["hug", "like"] }))),
    }
}

fn parse_comment_reaction_target_user(
    platform: Platform,
    target_user_ref: Option<String>,
    target_user_id: Option<String>,
) -> Result<ResourceRef, TuneWeaveError> {
    let qualified = target_user_ref.map(parse_reference).transpose()?;
    if let Some(reference) = qualified.as_ref()
        && reference.platform() != platform
    {
        return Err(TuneWeaveError::invalid_request(
            "target_user_ref must use the comment resource platform",
        )
        .with_details(json!({
            "comment_platform": platform,
            "target_user_ref": reference
        })));
    }
    let unqualified = target_user_id
        .map(|id| {
            ResourceRef::new(platform, id.clone()).map_err(|error| {
                TuneWeaveError::invalid_request(format!("invalid target_user_id: {error}"))
                    .with_details(json!({ "target_user_id": id }))
            })
        })
        .transpose()?;
    match (qualified, unqualified) {
        (Some(qualified), Some(unqualified)) if qualified != unqualified => Err(
            TuneWeaveError::invalid_request("target_user_ref and target_user_id must match")
                .with_details(json!({
                    "target_user_ref": qualified,
                    "target_user_id": unqualified.id()
                })),
        ),
        (Some(reference), _) | (_, Some(reference)) => Ok(reference),
        (None, None) => Err(TuneWeaveError::invalid_request(
            "target_user_ref or target_user_id is required",
        )),
    }
}

fn parse_comment_list_view(
    value: Option<&str>,
    has_parent_comment: bool,
) -> Result<CommentListView, TuneWeaveError> {
    let default = if has_parent_comment { "replies" } else { "all" };
    match value
        .unwrap_or(default)
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "all" | "default" | "legacy" => Ok(CommentListView::All),
        "hot" => Ok(CommentListView::Hot),
        "replies" | "reply" | "floor" => Ok(CommentListView::Replies),
        value => Err(
            TuneWeaveError::invalid_request(format!("unsupported comment view: {value}"))
                .with_details(json!({ "allowed": ["all", "hot", "replies"] })),
        ),
    }
}

fn parse_comment_sort(value: Option<&str>) -> Result<Option<CommentSort>, TuneWeaveError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    match value.to_ascii_lowercase().as_str() {
        "recommended" | "recommend" | "1" | "99" => Ok(Some(CommentSort::Recommended)),
        "hot" | "2" => Ok(Some(CommentSort::Hot)),
        "time" | "newest" | "3" => Ok(Some(CommentSort::Time)),
        value => Err(
            TuneWeaveError::invalid_request(format!("unsupported comment sort: {value}"))
                .with_details(json!({ "allowed": ["recommended", "hot", "time", 1, 2, 3, 99] })),
        ),
    }
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
        "track" | "song" | "1" => Ok(SearchKind::Track),
        "album" | "10" => Ok(SearchKind::Album),
        "artist" | "100" => Ok(SearchKind::Artist),
        "playlist" | "1000" => Ok(SearchKind::Playlist),
        "user" | "1002" => Ok(SearchKind::User),
        "mv" | "1004" => Ok(SearchKind::Mv),
        "lyric" | "lyrics" | "1006" => Ok(SearchKind::Lyric),
        "radio_station" | "radio" | "dj" | "1009" => Ok(SearchKind::RadioStation),
        "video" | "1014" => Ok(SearchKind::Video),
        "mixed" | "complex" | "1018" => Ok(SearchKind::Mixed),
        "voice" | "2000" => Ok(SearchKind::Voice),
        value => Err(
            TuneWeaveError::invalid_request(format!("unsupported search type: {value}"))
                .with_details(json!({
                    "allowed": [
                        "track", "album", "artist", "playlist", "user", "mv", "lyric",
                        "radio_station", "video", "mixed", "voice", 1, 10, 100, 1000, 1002,
                        1004, 1006, 1009, 1014, 1018, 2000
                    ]
                })),
        ),
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

fn unix_time_millis() -> Result<u64, TuneWeaveError> {
    let duration = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
        TuneWeaveError::new(
            tuneweave_core::ErrorCode::InternalError,
            "system clock is before the Unix epoch",
        )
    })?;
    u64::try_from(duration.as_millis()).map_err(|_| {
        TuneWeaveError::new(
            tuneweave_core::ErrorCode::InternalError,
            "system time exceeds the supported millisecond range",
        )
    })
}

fn parse_optional_i64_parameter(
    name: &str,
    value: Option<&str>,
) -> Result<Option<i64>, TuneWeaveError> {
    value
        .map(|value| {
            value.parse().map_err(|_| {
                TuneWeaveError::invalid_request(format!("{name} must be a signed integer"))
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
        BannerTargetKind, CommentMutationAction, CommentReplyReference, CommentThreadStats,
        CreatorSummary, DimensionChartTrackEntry, MusicProvider, Page, PageMeta, ProviderQrStart,
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
                Capability::SearchAlbums,
                Capability::SearchArtists,
                Capability::SearchPlaylists,
                Capability::SearchUsers,
                Capability::SearchMvs,
                Capability::SearchLyrics,
                Capability::SearchRadioStations,
                Capability::SearchVideos,
                Capability::SearchMixed,
                Capability::SearchVoices,
                Capability::AudioRecognition,
                Capability::Banners,
                Capability::RadioTaxonomy,
                Capability::RadioStationDetail,
                Capability::RadioStationList,
                Capability::RadioStationSubscriptionWrite,
                Capability::TrackDetail,
                Capability::TrackAvailability,
                Capability::AlbumDetail,
                Capability::AlbumList,
                Capability::AlbumStats,
                Capability::AlbumTrackEntitlements,
                Capability::AlbumSubscriptionWrite,
                Capability::DigitalAlbumDetail,
                Capability::DigitalAlbumList,
                Capability::DigitalAlbumCharts,
                Capability::DimensionCharts,
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
                Capability::ChallengeValidation,
                Capability::PrincipalStatus,
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
                Capability::AccountCloudUpload,
                Capability::AccountCloudDirectUpload,
                Capability::AccountCloudImport,
                Capability::AccountCloudLyrics,
                Capability::AccountCloudMatch,
                Capability::Favorites,
                Capability::ListeningHistory,
                Capability::Recommendations,
                Capability::CommentWrite,
                Capability::CommentsRead,
                Capability::CommentReactionsRead,
                Capability::CommentThreadStats,
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

        async fn search_catalog(&self, query: &SearchQuery) -> Result<Page<SearchItem>> {
            let item = match query.kind {
                SearchKind::Track | SearchKind::Lyric => SearchItem::Track(sample_track("123")),
                SearchKind::Album => SearchItem::Album(sample_album("18915")),
                SearchKind::Artist => SearchItem::Artist(sample_artist("6452")),
                SearchKind::Playlist => SearchItem::Playlist(sample_playlist("3778678")),
                SearchKind::User => SearchItem::User(sample_user("6298206519")),
                SearchKind::Mv | SearchKind::Video => SearchItem::Video(sample_video("22695250")),
                SearchKind::RadioStation => SearchItem::RadioStation(sample_radio_station("362")),
                SearchKind::Mixed | SearchKind::Voice => {
                    SearchItem::Opaque(tuneweave_core::SearchOpaqueItem {
                        platform: Platform::Netease,
                        kind: serde_json::to_value(query.kind)
                            .expect("serialize search kind")
                            .as_str()
                            .expect("string search kind")
                            .to_owned(),
                        id: Some("opaque-1".to_owned()),
                        title: Some("opaque search result".to_owned()),
                        extensions: tuneweave_core::Extensions::new(),
                    })
                }
            };
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("kind".to_owned(), json!(query.kind));
            extensions.insert("query".to_owned(), json!(query.query));
            extensions.insert("account".to_owned(), json!(query.account));
            Ok(Page {
                items: vec![item],
                pagination: PageMeta {
                    limit: query.limit,
                    offset: query.offset,
                    total: Some(1),
                    next_offset: None,
                    has_more: false,
                    extensions,
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

        async fn radio_stations(
            &self,
            request: &RadioStationListRequest,
        ) -> Result<Page<RadioStation>> {
            let mut station = sample_radio_station("175");
            station.name = "河北音乐广播".to_owned();
            station.region = Some("河北".to_owned());
            station.extensions.insert(
                "broadcast_station".to_owned(),
                json!({ "score": 1492, "source": "QT" }),
            );
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("request".to_owned(), json!(request));
            extensions.insert(
                "next_cursor".to_owned(),
                json!({ "id": "14", "score": 1472 }),
            );
            extensions.insert("requested_offset".to_owned(), json!(request.offset));
            extensions.insert("offset_applied".to_owned(), json!(false));
            Ok(Page {
                items: vec![station],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: 0,
                    total: Some(843),
                    next_offset: None,
                    has_more: true,
                    extensions,
                },
            })
        }

        async fn set_radio_station_subscription(
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

        async fn track(&self, id: &str, _account: Option<&str>) -> Result<Track> {
            Ok(sample_track(id))
        }

        async fn track_availability(
            &self,
            id: &str,
            request: &TrackAvailabilityRequest,
        ) -> Result<TrackAvailability> {
            let playable = id != "1";
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("account".to_owned(), json!(request.account));
            Ok(TrackAvailability {
                track_ref: ResourceRef::new(Platform::Netease, id)
                    .expect("valid test track reference"),
                playable,
                requested_bitrate: request.bitrate,
                actual_bitrate: playable.then_some(request.bitrate.min(320_000)),
                platform_code: Some(if playable { 200 } else { 404 }),
                message: if playable {
                    "ok"
                } else {
                    "亲爱的,暂无版权"
                }
                .to_owned(),
                extensions,
            })
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

        async fn dimension_chart(&self, request: &DimensionChartRequest) -> Result<DimensionChart> {
            let id = format!(
                "{}#{}@{}#",
                request.chart_code, request.target_id, request.target_type
            );
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("account".to_owned(), json!(request.account));
            Ok(DimensionChart {
                resource_ref: ResourceRef::new(Platform::Netease, &id)
                    .expect("valid test chart reference"),
                platform: Platform::Netease,
                id,
                chart_code: request.chart_code.clone(),
                target_id: request.target_id.clone(),
                target_type: request.target_type.clone(),
                name: "北京榜".to_owned(),
                description: "当前城市用户一周内收听的歌曲。".to_owned(),
                cover_url: Some("https://example.test/city.png".to_owned()),
                updated_at_ms: Some(1_784_181_600_000),
                play_count: Some(120),
                share_count: Some(3),
                comment_count: Some(9),
                supports_comments: Some(true),
                extensions,
            })
        }

        async fn dimension_chart_tracks(
            &self,
            request: &DimensionChartRequest,
        ) -> Result<DimensionChartTrackSnapshot> {
            let id = format!(
                "{}#{}@{}#",
                request.chart_code, request.target_id, request.target_type
            );
            let mut entry_extensions = tuneweave_core::Extensions::new();
            entry_extensions.insert(
                "target_url".to_owned(),
                json!("https://example.test/reason"),
            );
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("account".to_owned(), json!(request.account));
            Ok(DimensionChartTrackSnapshot {
                chart_ref: ResourceRef::new(Platform::Netease, id)
                    .expect("valid test chart reference"),
                chart_code: request.chart_code.clone(),
                target_id: request.target_id.clone(),
                target_type: request.target_type.clone(),
                entries: vec![DimensionChartTrackEntry {
                    rank: 1,
                    previous_rank: Some(4),
                    rank_change: Some(3),
                    track: sample_track("185809"),
                    reason: Some("城市流行热度上升".to_owned()),
                    reason_id: Some("17".to_owned()),
                    score: Some(98.5),
                    ratio: Some(0.98),
                    collected: Some(false),
                    extensions: entry_extensions,
                }],
                period_label: Some("每周更新".to_owned()),
                groups: BTreeMap::from([
                    ("CITY".to_owned(), "城市".to_owned()),
                    ("1020".to_owned(), "流行".to_owned()),
                ]),
                extensions,
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

        async fn validate_auth_challenge(
            &self,
            request: &AuthChallengeRequest,
            code: &str,
        ) -> Result<AuthChallengeValidation> {
            let valid = request.method == ChallengeMethod::Sms
                && request.principal == "13800138000"
                && request.country_code.as_deref() == Some("86")
                && code == "1234";
            let platform_code = if valid { "200" } else { "503" };
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert(
                "response".to_owned(),
                json!({ "code": platform_code, "data": valid }),
            );
            Ok(AuthChallengeValidation {
                method: request.method,
                valid,
                platform_code: Some(platform_code.to_owned()),
                message: (!valid).then(|| "invalid challenge code".to_owned()),
                extensions,
            })
        }

        async fn auth_principal_status(
            &self,
            request: &AuthPrincipalStatusRequest,
        ) -> Result<AuthPrincipalStatus> {
            if request.principal_type != PrincipalType::Phone {
                return Err(TuneWeaveError::invalid_request(
                    "test principal status only supports phone numbers",
                )
                .with_platform(Platform::Netease));
            }
            let valid_country_code = request.country_code.as_deref() == Some("86");
            let exists = valid_country_code && request.principal == "13800138000";
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert(
                "response".to_owned(),
                json!({
                    "code": 200,
                    "exist": if exists { 1 } else { -1 },
                    "cellphone": if exists { "138****8000" } else { "1" }
                }),
            );
            Ok(AuthPrincipalStatus {
                principal_type: request.principal_type,
                exists,
                has_password: Some(exists),
                display_name: exists.then(|| "masked-user".to_owned()),
                avatar_url: exists.then(|| "https://example.test/avatar.jpg".to_owned()),
                platform_code: Some("200".to_owned()),
                extensions,
            })
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

        async fn upload_cloud_track(
            &self,
            request: &CloudUploadRequest,
        ) -> Result<CloudUploadResult> {
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("filename".to_owned(), json!(request.filename));
            extensions.insert("content_type".to_owned(), json!(request.content_type));
            extensions.insert("data_len".to_owned(), json!(request.data.len()));
            extensions.insert("bitrate".to_owned(), json!(request.bitrate));
            extensions.insert("song_name".to_owned(), json!(request.song_name));
            extensions.insert("artist".to_owned(), json!(request.artist));
            extensions.insert("album".to_owned(), json!(request.album));
            extensions.insert("account".to_owned(), json!(request.account));
            Ok(CloudUploadResult {
                track_ref: Some(
                    ResourceRef::new(Platform::Netease, "cloud-uploaded")
                        .expect("valid uploaded cloud track reference"),
                ),
                upload_required: Some(true),
                uploaded: Some(true),
                published: true,
                extensions,
            })
        }

        async fn cloud_upload_ticket(
            &self,
            request: &CloudUploadTicketRequest,
        ) -> Result<CloudUploadTicket> {
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("md5".to_owned(), json!(request.md5));
            extensions.insert("file_size".to_owned(), json!(request.file_size));
            extensions.insert("filename".to_owned(), json!(request.filename));
            extensions.insert("bitrate".to_owned(), json!(request.bitrate));
            extensions.insert("content_type".to_owned(), json!(request.content_type));
            extensions.insert("account".to_owned(), json!(request.account));
            Ok(CloudUploadTicket {
                upload_required: true,
                provisional_track_id: Some("123".to_owned()),
                resource_id: "resource-456".to_owned(),
                upload_method: "POST".to_owned(),
                upload_url:
                    "https://nosup-jd1.127.net/bucket/object?offset=0&complete=true&version=1.0"
                        .to_owned(),
                upload_headers: BTreeMap::from([
                    ("Content-Length".to_owned(), request.file_size.to_string()),
                    ("Content-MD5".to_owned(), request.md5.to_ascii_lowercase()),
                    ("x-nos-token".to_owned(), "upload-secret".to_owned()),
                ]),
                extensions,
            })
        }

        async fn complete_cloud_upload(
            &self,
            request: &CloudUploadCompleteRequest,
        ) -> Result<CloudUploadResult> {
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert(
                "provisional_track_id".to_owned(),
                json!(request.provisional_track_id),
            );
            extensions.insert("resource_id".to_owned(), json!(request.resource_id));
            extensions.insert("md5".to_owned(), json!(request.md5));
            extensions.insert("filename".to_owned(), json!(request.filename));
            extensions.insert("song_name".to_owned(), json!(request.song_name));
            extensions.insert("artist".to_owned(), json!(request.artist));
            extensions.insert("album".to_owned(), json!(request.album));
            extensions.insert("bitrate".to_owned(), json!(request.bitrate));
            extensions.insert("account".to_owned(), json!(request.account));
            Ok(CloudUploadResult {
                track_ref: Some(
                    ResourceRef::new(Platform::Netease, &request.provisional_track_id)
                        .expect("valid cloud track reference"),
                ),
                upload_required: None,
                uploaded: None,
                published: true,
                extensions,
            })
        }

        async fn import_cloud_track(
            &self,
            request: &CloudImportRequest,
        ) -> Result<CloudImportResult> {
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("md5".to_owned(), json!(request.md5));
            extensions.insert("source_track_id".to_owned(), json!(request.source_track_id));
            extensions.insert("bitrate".to_owned(), json!(request.bitrate));
            extensions.insert("file_size".to_owned(), json!(request.file_size));
            extensions.insert("file_type".to_owned(), json!(request.file_type));
            extensions.insert("song_name".to_owned(), json!(request.song_name));
            extensions.insert("artist".to_owned(), json!(request.artist));
            extensions.insert("album".to_owned(), json!(request.album));
            extensions.insert("account".to_owned(), json!(request.account));
            Ok(CloudImportResult {
                track_ref: Some(
                    ResourceRef::new(Platform::Netease, "cloud-imported")
                        .expect("valid imported cloud track reference"),
                ),
                imported: true,
                already_present: Some(false),
                extensions,
            })
        }

        async fn cloud_lyrics(&self, request: &CloudLyricsRequest) -> Result<Lyrics> {
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("user_id".to_owned(), json!(request.user_id));
            extensions.insert("account".to_owned(), json!(request.account));
            Ok(Lyrics {
                track_ref: ResourceRef::new(Platform::Netease, &request.track_id)
                    .expect("valid cloud lyrics track reference"),
                plain: Some("[00:01.00]云盘歌词".to_owned()),
                translated: None,
                romanized: None,
                word_synced: None,
                format: "lrc".to_owned(),
                contributors: Vec::new(),
                extensions,
            })
        }

        async fn match_cloud_track(&self, request: &CloudMatchRequest) -> Result<CloudMatchResult> {
            let target_track_ref = request
                .target_track_id
                .as_deref()
                .filter(|id| *id != "0")
                .map(|id| ResourceRef::new(Platform::Netease, id))
                .transpose()
                .expect("valid cloud match target reference");
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("user_id".to_owned(), json!(request.user_id));
            extensions.insert("account".to_owned(), json!(request.account));
            Ok(CloudMatchResult {
                cloud_track_ref: ResourceRef::new(Platform::Netease, &request.cloud_track_id)
                    .expect("valid cloud track reference"),
                matched: target_track_ref.is_some(),
                target_track_ref,
                extensions,
            })
        }

        async fn post_comment(
            &self,
            request: &CommentWriteRequest,
        ) -> Result<CommentMutationResult> {
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("content".to_owned(), json!(request.content));
            extensions.insert("reply_to".to_owned(), json!(request.reply_to));
            extensions.insert("account".to_owned(), json!(request.account));
            Ok(CommentMutationResult {
                target: request.target.clone(),
                comment_id: Some("mock-comment-1".to_owned()),
                action: if request.reply_to.is_some() {
                    CommentMutationAction::Reply
                } else {
                    CommentMutationAction::Create
                },
                extensions,
            })
        }

        async fn delete_comment(
            &self,
            request: &CommentDeleteRequest,
        ) -> Result<CommentMutationResult> {
            let mut extensions = tuneweave_core::Extensions::new();
            extensions.insert("account".to_owned(), json!(request.account));
            Ok(CommentMutationResult {
                target: request.target.clone(),
                comment_id: Some(request.comment_id.clone()),
                action: CommentMutationAction::Delete,
                extensions,
            })
        }

        async fn comments(&self, request: &CommentListRequest) -> Result<CommentPage> {
            let mut pagination_extensions = Extensions::new();
            pagination_extensions.insert("request".to_owned(), json!(request));
            let comments = if request.view != CommentListView::Hot {
                vec![sample_comment("3160990055", "普通评论")]
            } else {
                Vec::new()
            };
            let hot_comments = if request.view == CommentListView::Hot
                || (request.view == CommentListView::All && request.sort.is_none())
            {
                vec![sample_comment("200", "热门评论")]
            } else {
                Vec::new()
            };
            let current_comment = (request.view == CommentListView::Replies
                || request.sort.is_some())
            .then(|| sample_comment("300", "当前评论"));
            Ok(CommentPage {
                target: request.target.clone(),
                comments,
                hot_comments,
                top_comments: vec![sample_comment("400", "置顶评论")],
                current_comment,
                pagination: tuneweave_core::PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(68_334),
                    next_offset: Some(request.offset.saturating_add(request.limit)),
                    has_more: true,
                    extensions: pagination_extensions,
                },
                extensions: Extensions::from([("provider".to_owned(), json!("mock"))]),
            })
        }

        async fn comment_reactions(
            &self,
            request: &CommentReactionListRequest,
        ) -> Result<CommentReactionPage> {
            let offset = request.page.saturating_sub(1).saturating_mul(request.limit);
            let mut pagination_extensions = Extensions::new();
            pagination_extensions.insert("request".to_owned(), json!(request));
            Ok(CommentReactionPage {
                target: request.target.clone(),
                comment_id: request.comment_id.clone(),
                target_user_ref: request.target_user_ref.clone(),
                kind: request.kind,
                reactions: vec![CommentReaction {
                    kind: request.kind,
                    user: sample_user("2121989064"),
                    content: Some("给了评论作者一个抱抱".to_owned()),
                    extensions: Extensions::from([("provider".to_owned(), json!("mock"))]),
                }],
                current_comment: Some(sample_comment(&request.comment_id, "当前评论")),
                pagination: tuneweave_core::PageMeta {
                    limit: request.limit,
                    offset,
                    total: Some(150),
                    next_offset: Some(offset.saturating_add(request.limit)),
                    has_more: true,
                    extensions: pagination_extensions,
                },
                extensions: Extensions::from([("provider".to_owned(), json!("mock"))]),
            })
        }

        async fn comment_thread_stats(
            &self,
            request: &CommentThreadStatsRequest,
        ) -> Result<CommentThreadStatsBatch> {
            let stats = request
                .resource_refs
                .iter()
                .cloned()
                .enumerate()
                .map(|(index, resource_ref)| CommentThreadStats {
                    target: CommentTarget::new(resource_ref.clone(), request.kind),
                    requested_ref: Some(resource_ref),
                    liked: Some(false),
                    like_count: Some(36 + index as u64),
                    comment_count: Some(68_334 + index as u64),
                    comment_count_text: Some("6万+".to_owned()),
                    share_count: Some(27_153 + index as u64),
                    comment_upgraded: Some(false),
                    musician_comment_count: Some(0),
                    latest_liked_users: vec![sample_user("2121989064")],
                    comments: vec![sample_comment("3160990055", "最近评论")],
                    extensions: Extensions::from([("provider".to_owned(), json!("mock"))]),
                })
                .collect();
            Ok(CommentThreadStatsBatch {
                kind: request.kind,
                requested_refs: request.resource_refs.clone(),
                stats,
                extensions: Extensions::from([
                    ("provider".to_owned(), json!("mock")),
                    ("request".to_owned(), json!(request)),
                ]),
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

    fn sample_comment(id: &str, content: &str) -> Comment {
        Comment {
            platform: Platform::Netease,
            id: id.to_owned(),
            content: content.to_owned(),
            author: Some(sample_user("278612322")),
            created_at_ms: Some(1_582_035_919_432),
            created_at_text: Some("2020-02-18".to_owned()),
            liked: Some(false),
            like_count: Some(5_646),
            parent_comment_id: None,
            reply_count: Some(2),
            replied_to: vec![CommentReplyReference {
                comment_id: Some("100".to_owned()),
                content: "原评论".to_owned(),
                author: Some(sample_user("200")),
                extensions: Extensions::new(),
            }],
            ip_location: Some("上海".to_owned()),
            extensions: Extensions::new(),
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
        binary_request_with_method(app, Method::PUT, path, content_type, body).await
    }

    async fn binary_request_with_method(
        app: Router,
        method: Method,
        path: &str,
        content_type: Option<&str>,
        body: Vec<u8>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder().method(method).uri(path);
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
        assert_eq!(json["data"][0]["type"], "track");
        assert_eq!(json["data"][0]["data"]["ref"], "netease:123");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["pagination"]["limit"], 10);
        assert_eq!(json["meta"]["pagination"]["total"], 1);
        assert_eq!(json["meta"]["pagination"]["extensions"]["kind"], "track");
        assert_eq!(json["meta"]["pagination"]["extensions"]["query"], "clock");
    }

    #[tokio::test]
    async fn search_accepts_every_reference_cloudsearch_type_and_keywords_alias() {
        for (platform_type, kind, item_type, reference) in [
            ("1", "track", "track", Some("netease:123")),
            ("10", "album", "album", Some("netease:18915")),
            ("100", "artist", "artist", Some("netease:6452")),
            ("1000", "playlist", "playlist", Some("netease:3778678")),
            ("1002", "user", "user", Some("netease:6298206519")),
            ("1004", "mv", "video", Some("netease:22695250")),
            ("1006", "lyric", "track", Some("netease:123")),
            (
                "1009",
                "radio_station",
                "radio_station",
                Some("netease:362"),
            ),
            ("1014", "video", "video", Some("netease:22695250")),
            ("1018", "mixed", "opaque", None),
            ("2000", "voice", "opaque", None),
        ] {
            let path = format!(
                "/v1/search?keywords=clock&type={platform_type}&platform=netease&account=search-user&limit=5&offset=2"
            );
            let (status, json) = json_response_from(test_app_with_provider(), &path).await;
            assert_eq!(status, StatusCode::OK, "type={platform_type}");
            assert_eq!(json["data"][0]["type"], item_type, "type={platform_type}");
            if let Some(reference) = reference {
                assert_eq!(
                    json["data"][0]["data"]["ref"], reference,
                    "type={platform_type}"
                );
            } else {
                assert_eq!(
                    json["data"][0]["data"]["kind"], kind,
                    "type={platform_type}"
                );
            }
            assert_eq!(json["meta"]["account"], "search-user");
            assert_eq!(json["meta"]["pagination"]["limit"], 5);
            assert_eq!(json["meta"]["pagination"]["offset"], 2);
            assert_eq!(
                json["meta"]["pagination"]["extensions"]["kind"], kind,
                "type={platform_type}"
            );
        }
    }

    #[test]
    fn search_kind_parser_accepts_unified_and_reference_names() {
        for (value, expected) in [
            ("track", SearchKind::Track),
            ("song", SearchKind::Track),
            ("album", SearchKind::Album),
            ("artist", SearchKind::Artist),
            ("playlist", SearchKind::Playlist),
            ("user", SearchKind::User),
            ("mv", SearchKind::Mv),
            ("lyrics", SearchKind::Lyric),
            ("dj", SearchKind::RadioStation),
            ("video", SearchKind::Video),
            ("complex", SearchKind::Mixed),
            ("voice", SearchKind::Voice),
        ] {
            assert_eq!(parse_search_kind(Some(value)).expect(value), expected);
        }
    }

    #[test]
    fn comment_target_parser_accepts_unified_names_and_all_reference_types() {
        for (value, expected) in [
            ("track", CommentTargetKind::Track),
            ("music", CommentTargetKind::Track),
            ("0", CommentTargetKind::Track),
            ("mv", CommentTargetKind::Mv),
            ("1", CommentTargetKind::Mv),
            ("playlist", CommentTargetKind::Playlist),
            ("2", CommentTargetKind::Playlist),
            ("album", CommentTargetKind::Album),
            ("3", CommentTargetKind::Album),
            ("radio-episode", CommentTargetKind::RadioEpisode),
            ("dj", CommentTargetKind::RadioEpisode),
            ("4", CommentTargetKind::RadioEpisode),
            ("video", CommentTargetKind::Video),
            ("5", CommentTargetKind::Video),
            ("event", CommentTargetKind::Event),
            ("6", CommentTargetKind::Event),
            ("radio-station", CommentTargetKind::RadioStation),
            ("7", CommentTargetKind::RadioStation),
        ] {
            assert_eq!(parse_comment_target_kind(value).expect(value), expected);
        }
        assert_eq!(
            parse_comment_target_kind("article")
                .expect_err("unsupported target")
                .code,
            tuneweave_core::ErrorCode::InvalidRequest
        );
    }

    #[tokio::test]
    async fn comment_thread_stats_accept_reference_batches_and_preserve_requested_refs() {
        let (status, response) = json_response_from(
            test_app_with_provider(),
            "/v1/resources/track/comments/stats?platform=netease&ids=185809,%20347230&account=personal",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["data"]["kind"], "track");
        assert_eq!(response["data"]["requested_refs"][0], "netease:185809");
        assert_eq!(response["data"]["requested_refs"][1], "netease:347230");
        assert_eq!(response["data"]["stats"].as_array().map(Vec::len), Some(2));
        assert_eq!(
            response["data"]["stats"][0]["target"]["ref"],
            "netease:185809"
        );
        assert_eq!(
            response["data"]["stats"][0]["requested_ref"],
            "netease:185809"
        );
        assert_eq!(response["data"]["stats"][0]["comment_count"], 68_334);
        assert_eq!(
            response["data"]["stats"][0]["latest_liked_users"][0]["ref"],
            "netease:2121989064"
        );
        assert_eq!(
            response["data"]["stats"][0]["comments"][0]["id"],
            "3160990055"
        );
        assert_eq!(response["data"]["extensions"]["provider"], "mock");
        assert_eq!(
            response["data"]["extensions"]["request"]["resource_refs"][1],
            "netease:347230"
        );
        assert_eq!(response["meta"]["platform"], "netease");
        assert_eq!(response["meta"]["account"], "personal");
    }

    #[tokio::test]
    async fn comment_thread_stats_support_single_id_fallback_numeric_types_and_empty_lists() {
        let (status, single) = json_response_from(
            test_app_with_provider(),
            "/v1/resources/0/comments/stats?platform=netease&ids=&id=185809",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(single["data"]["kind"], "track");
        assert_eq!(single["data"]["stats"].as_array().map(Vec::len), Some(1));
        assert_eq!(single["data"]["requested_refs"][0], "netease:185809");
        assert!(single["meta"].get("account").is_none());

        let (status, empty) = json_response_from(
            test_app_with_provider(),
            "/v1/resources/track/comments/stats?platform=netease",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            empty["data"]["requested_refs"].as_array().map(Vec::len),
            Some(0)
        );
        assert_eq!(empty["data"]["stats"].as_array().map(Vec::len), Some(0));
    }

    #[tokio::test]
    async fn comment_thread_stats_reject_unknown_platforms_and_resource_types() {
        for path in [
            "/v1/resources/article/comments/stats?platform=netease&ids=185809",
            "/v1/resources/track/comments/stats?platform=unknown&ids=185809",
        ] {
            let (status, response) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(response["error"]["code"], "invalid_request", "{path}");
        }
    }

    #[test]
    fn comment_view_and_sort_parsers_cover_unified_and_reference_values() {
        for (value, expected) in [
            ("all", CommentListView::All),
            ("legacy", CommentListView::All),
            ("hot", CommentListView::Hot),
            ("floor", CommentListView::Replies),
        ] {
            assert_eq!(
                parse_comment_list_view(Some(value), false).expect(value),
                expected
            );
        }
        assert_eq!(
            parse_comment_list_view(None, true).expect("parent default"),
            CommentListView::Replies
        );
        for (value, expected) in [
            ("recommended", CommentSort::Recommended),
            ("1", CommentSort::Recommended),
            ("99", CommentSort::Recommended),
            ("hot", CommentSort::Hot),
            ("2", CommentSort::Hot),
            ("time", CommentSort::Time),
            ("3", CommentSort::Time),
        ] {
            assert_eq!(
                parse_comment_sort(Some(value)).expect(value),
                Some(expected)
            );
        }
        assert_eq!(parse_comment_sort(None).expect("no sort"), None);
    }

    #[tokio::test]
    async fn comment_lists_expose_legacy_modern_hot_and_floor_views() {
        let (status, legacy) = json_response_from(
            test_app_with_provider(),
            "/v1/resources/track/netease:185809/comments?account=reader&limit=2&offset=3&before=1582035919432",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(legacy["data"]["target"]["ref"], "netease:185809");
        assert_eq!(legacy["data"]["target"]["kind"], "track");
        assert_eq!(legacy["data"]["comments"][0]["id"], "3160990055");
        assert_eq!(
            legacy["data"]["comments"][0]["author"]["ref"],
            "netease:278612322"
        );
        assert_eq!(
            legacy["data"]["comments"][0]["replied_to"][0]["comment_id"],
            "100"
        );
        assert_eq!(legacy["data"]["hot_comments"][0]["id"], "200");
        assert_eq!(legacy["data"]["top_comments"][0]["id"], "400");
        assert!(legacy["data"]["current_comment"].is_null());
        assert_eq!(legacy["data"]["extensions"]["provider"], "mock");
        assert!(legacy["data"].get("pagination").is_none());
        assert_eq!(legacy["meta"]["platform"], "netease");
        assert_eq!(legacy["meta"]["account"], "reader");
        assert_eq!(legacy["meta"]["pagination"]["limit"], 2);
        assert_eq!(legacy["meta"]["pagination"]["offset"], 3);
        assert_eq!(
            legacy["meta"]["pagination"]["extensions"]["request"]["before_time_ms"],
            1_582_035_919_432_u64
        );

        let (status, modern) = json_response_from(
            test_app_with_provider(),
            "/v1/resources/0/netease:185809/comments?sortType=3&pageSize=2&pageNo=2&cursor=1581222127578&showInner=false",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(modern["data"]["current_comment"]["id"], "300");
        let request = &modern["meta"]["pagination"]["extensions"]["request"];
        assert_eq!(request["sort"], "time");
        assert_eq!(request["limit"], 2);
        assert_eq!(request["page"], 2);
        assert_eq!(request["cursor"], "1581222127578");
        assert_eq!(request["include_replies"], false);

        let (status, hot) = json_response_from(
            test_app_with_provider(),
            "/v1/resources/track/netease:185809/comments?view=hot&limit=1",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(hot["data"]["comments"].as_array().map(Vec::len), Some(0));
        assert_eq!(hot["data"]["hot_comments"][0]["content"], "热门评论");

        let (status, floor) = json_response_from(
            test_app_with_provider(),
            "/v1/resources/track/netease:185809/comments?parentCommentId=3160990055&time=1580000000000",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(floor["data"]["current_comment"]["content"], "当前评论");
        let request = &floor["meta"]["pagination"]["extensions"]["request"];
        assert_eq!(request["view"], "replies");
        assert_eq!(request["parent_comment_id"], "3160990055");
        assert_eq!(request["before_time_ms"], 1_580_000_000_000_u64);
    }

    #[tokio::test]
    async fn comment_lists_reject_invalid_pagination_view_sort_and_conflicts() {
        for path in [
            "/v1/resources/track/netease:185809/comments?limit=0",
            "/v1/resources/track/netease:185809/comments?limit=101",
            "/v1/resources/track/netease:185809/comments?page=0&sort=time",
            "/v1/resources/track/netease:185809/comments?view=unknown",
            "/v1/resources/track/netease:185809/comments?sortType=4",
            "/v1/resources/track/netease:185809/comments?view=hot&sort=hot",
            "/v1/resources/track/netease:185809/comments?view=replies",
            "/v1/resources/track/netease:185809/comments?cursor=100",
            "/v1/resources/track/netease:185809/comments?sort=hot&cursor=100",
        ] {
            let (status, response) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(response["error"]["code"], "invalid_request", "{path}");
        }
    }

    #[tokio::test]
    async fn comment_reaction_lists_expose_unified_users_and_reference_cursor_aliases() {
        let (status, response) = json_response_from(
            test_app_with_provider(),
            "/v1/resources/track/netease:863481066/comments/1167145843/reactions/hug?uid=285516405&pageSize=2&pageNo=3&cursor=cursor-1&idCursor=100&account=personal",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["data"]["target"]["ref"], "netease:863481066");
        assert_eq!(response["data"]["target"]["kind"], "track");
        assert_eq!(response["data"]["comment_id"], "1167145843");
        assert_eq!(response["data"]["target_user_ref"], "netease:285516405");
        assert_eq!(response["data"]["kind"], "hug");
        assert_eq!(
            response["data"]["reactions"][0]["user"]["ref"],
            "netease:2121989064"
        );
        assert_eq!(
            response["data"]["reactions"][0]["content"],
            "给了评论作者一个抱抱"
        );
        assert_eq!(response["data"]["current_comment"]["id"], "1167145843");
        assert_eq!(response["data"]["extensions"]["provider"], "mock");
        assert!(response["data"].get("pagination").is_none());
        assert_eq!(response["meta"]["platform"], "netease");
        assert_eq!(response["meta"]["account"], "personal");
        assert_eq!(response["meta"]["pagination"]["limit"], 2);
        assert_eq!(response["meta"]["pagination"]["offset"], 4);
        let request = &response["meta"]["pagination"]["extensions"]["request"];
        assert_eq!(request["target_user_ref"], "netease:285516405");
        assert_eq!(request["page"], 3);
        assert_eq!(request["cursor"], "cursor-1");
        assert_eq!(request["id_cursor"], "100");

        let (status, response) = json_response_from(
            test_app_with_provider(),
            "/v1/resources/0/netease:863481066/comments/1167145843/reactions/like?target_user_ref=netease%3A285516405&target_user_id=285516405",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["data"]["kind"], "like");
        assert_eq!(response["meta"]["account"], "default");
        assert_eq!(response["meta"]["pagination"]["limit"], 100);
    }

    #[tokio::test]
    async fn comment_reaction_lists_reject_missing_cross_platform_and_invalid_inputs() {
        for path in [
            "/v1/resources/track/netease:863481066/comments/1167145843/reactions/hug",
            "/v1/resources/track/netease:863481066/comments/1167145843/reactions/hug?target_user_ref=qq%3A285516405",
            "/v1/resources/track/netease:863481066/comments/1167145843/reactions/hug?target_user_ref=netease%3A1&uid=2",
            "/v1/resources/track/netease:863481066/comments/1167145843/reactions/hug?uid=285516405&limit=0",
            "/v1/resources/track/netease:863481066/comments/1167145843/reactions/hug?uid=285516405&limit=101",
            "/v1/resources/track/netease:863481066/comments/1167145843/reactions/hug?uid=285516405&page=0",
            "/v1/resources/track/netease:863481066/comments/1167145843/reactions/clap?uid=285516405",
        ] {
            let (status, response) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(response["error"]["code"], "invalid_request", "{path}");
        }
    }

    #[tokio::test]
    async fn comment_create_reply_and_delete_share_the_unified_resource_endpoint() {
        let (status, created) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/resources/music/netease:185809/comments?account=personal",
            Some(json!({"content": "  新评论  "})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(created["data"]["target"]["ref"], "netease:185809");
        assert_eq!(created["data"]["target"]["kind"], "track");
        assert_eq!(created["data"]["action"], "create");
        assert_eq!(created["data"]["comment_id"], "mock-comment-1");
        assert_eq!(created["data"]["extensions"]["content"], "  新评论  ");
        assert!(created["data"]["extensions"]["reply_to"].is_null());
        assert_eq!(created["data"]["extensions"]["account"], "personal");
        assert_eq!(created["meta"]["platform"], "netease");
        assert_eq!(created["meta"]["account"], "personal");

        let (status, replied) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/resources/0/netease:185809/comments/1438569889/replies?account=personal",
            Some(json!({"content": "回复内容"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(replied["data"]["action"], "reply");
        assert_eq!(replied["data"]["extensions"]["reply_to"], "1438569889");

        let (status, deleted) = json_request_from(
            test_app_with_provider(),
            Method::DELETE,
            "/v1/resources/track/netease:185809/comments/1535550516319?account=personal",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(deleted["data"]["action"], "delete");
        assert_eq!(deleted["data"]["comment_id"], "1535550516319");
        assert_eq!(deleted["data"]["target"]["ref"], "netease:185809");
        assert_eq!(deleted["meta"]["account"], "personal");
    }

    #[tokio::test]
    async fn comment_endpoints_reject_invalid_targets_content_and_json() {
        for (path, body) in [
            (
                "/v1/resources/article/netease:185809/comments",
                Some(json!({"content": "test"})),
            ),
            (
                "/v1/resources/track/missing-platform/comments",
                Some(json!({"content": "test"})),
            ),
            (
                "/v1/resources/track/netease:185809/comments",
                Some(json!({"content": "  "})),
            ),
            (
                "/v1/resources/track/netease:185809/comments",
                Some(json!({"content": "test", "unknown": true})),
            ),
        ] {
            let (status, response) =
                json_request_from(test_app_with_provider(), Method::POST, path, body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(response["error"]["code"], "invalid_request", "{path}");
        }
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
    async fn radio_station_catalog_preserves_filters_cursor_and_ignored_offset() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/radio/stations?platform=netease&account=radio-user&categoryId=1&region_id=407&lastId=172&score=1542&limit=20&offset=100",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:175");
        assert_eq!(json["data"][0]["name"], "河北音乐广播");
        assert_eq!(
            json["data"][0]["extensions"]["broadcast_station"]["score"],
            1492
        );
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["request"]["category_id"],
            "1"
        );
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["request"]["region_id"],
            "407"
        );
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["request"]["cursor"]["id"],
            "172"
        );
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["request"]["cursor"]["score"],
            1542
        );
        assert_eq!(json["meta"]["pagination"]["offset"], 0);
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["requested_offset"],
            100
        );
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["offset_applied"],
            false
        );
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["next_cursor"]["id"],
            "14"
        );
        assert_eq!(json["meta"]["account"], "radio-user");
    }

    #[tokio::test]
    async fn radio_station_catalog_accepts_independent_reference_cursor_fields() {
        for (query, expected_id, expected_score) in
            [("lastId=172", "172", -1), ("score=1542", "0", 1542)]
        {
            let path = format!("/v1/radio/stations?{query}");
            let (status, json) = json_response_from(test_app_with_provider(), &path).await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(
                json["meta"]["pagination"]["extensions"]["request"]["cursor"]["id"],
                expected_id
            );
            assert_eq!(
                json["meta"]["pagination"]["extensions"]["request"]["cursor"]["score"],
                expected_score
            );
        }

        let (status, json) =
            json_response_from(test_app_with_provider(), "/v1/radio/stations?score=invalid").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], "invalid_request");
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
    async fn track_availability_supports_default_unified_and_reference_bitrates() {
        let (status, default) = json_response_from(
            test_app_with_provider(),
            "/v1/tracks/netease:1969519579/availability?account=vip",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(default["data"]["track_ref"], "netease:1969519579");
        assert_eq!(default["data"]["playable"], true);
        assert_eq!(default["data"]["requested_bitrate"], 999_000);
        assert_eq!(default["data"]["actual_bitrate"], 320_000);
        assert_eq!(default["data"]["platform_code"], 200);
        assert_eq!(default["data"]["extensions"]["account"], "vip");
        assert_eq!(default["meta"]["platform"], "netease");
        assert_eq!(default["meta"]["account"], "vip");

        let (status, reference) = json_response_from(
            test_app_with_provider(),
            "/v1/tracks/netease:1969519579/availability?br=128000",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(reference["data"]["requested_bitrate"], 128_000);
        assert_eq!(reference["data"]["actual_bitrate"], 128_000);

        let (status, unified) = json_response_from(
            test_app_with_provider(),
            "/v1/tracks/netease:1/availability?bitrate=64000",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(unified["data"]["playable"], false);
        assert_eq!(unified["data"]["requested_bitrate"], 64_000);
        assert_eq!(unified["data"]["actual_bitrate"], Value::Null);
        assert_eq!(unified["data"]["platform_code"], 404);
    }

    #[tokio::test]
    async fn track_availability_rejects_non_numeric_bitrates() {
        let (status, response) = json_response_from(
            test_app_with_provider(),
            "/v1/tracks/netease:1969519579/availability?br=lossless",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(response["error"]["code"], "invalid_request");
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
    async fn radio_station_library_put_and_delete_share_the_subscription_result() {
        let (status, subscribed) = json_request_from(
            test_app_with_provider(),
            Method::PUT,
            "/v1/account/library/radio-stations/netease:362?account=collector",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(subscribed["data"]["resource_ref"], "netease:362");
        assert_eq!(subscribed["data"]["subscribed"], true);
        assert_eq!(subscribed["data"]["extensions"]["account"], "collector");
        assert_eq!(subscribed["meta"]["platform"], "netease");
        assert_eq!(subscribed["meta"]["account"], "collector");

        let (status, unsubscribed) = json_request_from(
            test_app_with_provider(),
            Method::DELETE,
            "/v1/account/library/radio-stations/netease:362?account=collector",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(unsubscribed["data"]["resource_ref"], "netease:362");
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
    async fn dimension_chart_detail_accepts_reference_parameter_aliases() {
        let (status, chart) = json_response_from(
            test_app_with_provider(),
            "/v1/charts/dimensions/CITY_SONG_CHART?platform=netease&account=vip&targetId=110000&targetType=CITY",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(chart["data"]["ref"], "netease:CITY_SONG_CHART#110000@CITY#");
        assert_eq!(chart["data"]["chart_code"], "CITY_SONG_CHART");
        assert_eq!(chart["data"]["target_id"], "110000");
        assert_eq!(chart["data"]["target_type"], "CITY");
        assert_eq!(chart["data"]["name"], "北京榜");
        assert_eq!(chart["data"]["extensions"]["account"], "vip");
        assert_eq!(chart["meta"]["platform"], "netease");
        assert_eq!(chart["meta"]["account"], "vip");
        assert!(chart["meta"].get("pagination").is_none());
    }

    #[tokio::test]
    async fn dimension_chart_tracks_return_a_complete_unpaginated_snapshot() {
        let (status, chart) = json_response_from(
            test_app_with_provider(),
            "/v1/charts/dimensions/CITY_STYLE_SONG_CHART/tracks?platform=netease&account=vip&target_id=110000_1020&target_type=CITY_STYLE",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            chart["data"]["chart_ref"],
            "netease:CITY_STYLE_SONG_CHART#110000_1020@CITY_STYLE#"
        );
        assert_eq!(chart["data"]["entries"][0]["rank"], 1);
        assert_eq!(chart["data"]["entries"][0]["previous_rank"], 4);
        assert_eq!(chart["data"]["entries"][0]["rank_change"], 3);
        assert_eq!(
            chart["data"]["entries"][0]["track"]["ref"],
            "netease:185809"
        );
        assert_eq!(chart["data"]["entries"][0]["reason_id"], "17");
        assert_eq!(chart["data"]["groups"]["1020"], "流行");
        assert_eq!(chart["data"]["period_label"], "每周更新");
        assert_eq!(chart["data"]["extensions"]["account"], "vip");
        assert!(chart["meta"].get("pagination").is_none());
    }

    #[tokio::test]
    async fn dimension_chart_rejects_missing_dimension_parameters() {
        for path in [
            "/v1/charts/dimensions/CITY_SONG_CHART?target_type=CITY",
            "/v1/charts/dimensions/CITY_SONG_CHART?target_id=110000",
        ] {
            let (status, response) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(response["error"]["code"], "invalid_request");
        }
    }

    #[tokio::test]
    async fn invalid_search_parameters_use_the_error_envelope() {
        for path in [
            "/v1/search?q=clock&limit=101",
            "/v1/search?q=clock&type=podcast",
            "/v1/search?type=1",
        ] {
            let (status, json) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(json["error"]["code"], "invalid_request", "{path}");
        }
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
    async fn principal_status_accepts_reference_and_unified_phone_fields() {
        let (status, registered) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/auth/principals/status",
            Some(json!({
                "platform": "netease",
                "account": "lookup-account",
                "phone": 13800138000_u64,
                "countrycode": 86
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(registered["data"]["principal_type"], "phone");
        assert_eq!(registered["data"]["exists"], true);
        assert_eq!(registered["data"]["has_password"], true);
        assert_eq!(registered["data"]["display_name"], "masked-user");
        assert_eq!(registered["data"]["platform_code"], "200");
        assert_eq!(
            registered["data"]["extensions"]["response"]["cellphone"],
            "138****8000"
        );
        assert_eq!(registered["meta"]["platform"], "netease");
        assert_eq!(registered["meta"]["account"], "lookup-account");
        let serialized = serde_json::to_string(&registered).expect("serialize status response");
        assert!(!serialized.contains("13800138000"));

        let (status, unregistered) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/auth/principals/status",
            Some(json!({
                "platform": "netease",
                "principal_type": "phone",
                "principal": "1",
                "country_code": ""
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(unregistered["data"]["exists"], false);
        assert_eq!(unregistered["data"]["has_password"], false);
        assert!(unregistered["data"]["display_name"].is_null());
        assert_eq!(unregistered["meta"]["account"], "default");

        let (status, camel_country) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/auth/principals/status",
            Some(json!({
                "platform": "netease",
                "phone": "1",
                "countryCode": "86"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(camel_country["data"]["exists"], false);
    }

    #[tokio::test]
    async fn principal_status_rejects_unsupported_types_and_non_scalar_values() {
        let (status, json) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/auth/principals/status",
            Some(json!({
                "platform": "netease",
                "principal_type": "email",
                "principal": "private@example.test"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], "invalid_request");
        let serialized = serde_json::to_string(&json).expect("serialize unsupported response");
        assert!(!serialized.contains("private@example.test"));

        let (status, json) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/auth/principals/status",
            Some(json!({
                "platform": "netease",
                "principal": ["private"]
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], "invalid_request");
        let serialized = serde_json::to_string(&json).expect("serialize invalid response");
        assert!(!serialized.contains("private"));
    }

    #[tokio::test]
    async fn challenge_validation_accepts_reference_and_unified_sms_fields_without_login() {
        let (status, reference) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/auth/challenges/validate",
            Some(json!({
                "platform": "netease",
                "account": "validation-account",
                "phone": 13800138000_u64,
                "captcha": "1234",
                "ctcode": 86
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(reference["data"]["method"], "sms");
        assert_eq!(reference["data"]["valid"], true);
        assert_eq!(reference["data"]["platform_code"], "200");
        assert_eq!(reference["data"]["extensions"]["response"]["data"], true);
        assert_eq!(reference["meta"]["platform"], "netease");
        assert_eq!(reference["meta"]["account"], "validation-account");
        let serialized = serde_json::to_string(&reference).expect("serialize validation response");
        assert!(!serialized.contains("13800138000"));
        assert!(!serialized.contains("1234"));

        let (status, unified) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/auth/challenges/validate",
            Some(json!({
                "platform": "netease",
                "method": "sms",
                "principal": "13800138000",
                "code": "1234",
                "country_code": "86"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(unified["data"]["valid"], true);
        assert_eq!(unified["meta"]["account"], "default");

        let (status, empty_country) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/auth/challenges/validate",
            Some(json!({
                "platform": "netease",
                "phone": "13800138000",
                "captcha": "1234",
                "ctcode": ""
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(empty_country["data"]["valid"], true);
    }

    #[tokio::test]
    async fn challenge_validation_rejects_non_scalar_principals_and_empty_codes() {
        let (status, json) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/auth/challenges/validate",
            Some(json!({
                "platform": "netease",
                "principal": { "phone": "private" },
                "code": "1234"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], "invalid_request");
        let serialized = serde_json::to_string(&json).expect("serialize invalid response");
        assert!(!serialized.contains("private"));

        let (status, json) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/auth/challenges/validate",
            Some(json!({
                "platform": "netease",
                "principal": "13800138000",
                "code": "   "
            })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], "invalid_request");
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
    async fn cloud_proxy_upload_accepts_binary_audio_and_reference_metadata_aliases() {
        let (status, json) = binary_request_with_method(
            test_app_with_provider(),
            Method::POST,
            "/v1/account/cloud/uploads?platform=netease&account=locker&filename=clock.flac&bitrate=1411200&song=Clock&artist=Jay&album=Jay",
            Some("audio/flac"),
            b"fLaC-audio-content".to_vec(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["track_ref"], "netease:cloud-uploaded");
        assert_eq!(json["data"]["upload_required"], true);
        assert_eq!(json["data"]["uploaded"], true);
        assert_eq!(json["data"]["published"], true);
        assert_eq!(json["data"]["extensions"]["filename"], "clock.flac");
        assert_eq!(json["data"]["extensions"]["content_type"], "audio/flac");
        assert_eq!(json["data"]["extensions"]["data_len"], 18);
        assert_eq!(json["data"]["extensions"]["bitrate"], 1_411_200);
        assert_eq!(json["data"]["extensions"]["song_name"], "Clock");
        assert_eq!(json["data"]["extensions"]["artist"], "Jay");
        assert_eq!(json["data"]["extensions"]["album"], "Jay");
        assert_eq!(json["data"]["extensions"]["account"], "locker");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "locker");
    }

    #[tokio::test]
    async fn cloud_proxy_upload_uses_default_bitrate_and_optional_content_type() {
        let (status, json) = binary_request_with_method(
            test_app_with_provider(),
            Method::POST,
            "/v1/account/cloud/uploads?filename=song.mp3",
            None,
            b"audio".to_vec(),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["extensions"]["content_type"], "");
        assert_eq!(json["data"]["extensions"]["bitrate"], 999_000);
        assert_eq!(json["meta"]["account"], "default");
    }

    #[tokio::test]
    async fn cloud_proxy_upload_rejects_missing_audio_fields_and_invalid_bitrate() {
        for (path, body) in [
            ("/v1/account/cloud/uploads?filename=song.mp3", Vec::new()),
            ("/v1/account/cloud/uploads", vec![1]),
            (
                "/v1/account/cloud/uploads?filename=song.mp3&bitrate=0",
                vec![1],
            ),
            (
                "/v1/account/cloud/uploads?filename=song.mp3&bitrate=fast",
                vec![1],
            ),
        ] {
            let (status, json) = binary_request_with_method(
                test_app_with_provider(),
                Method::POST,
                path,
                Some("audio/mpeg"),
                body,
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(json["error"]["code"], "invalid_request", "{path}");
        }
    }

    #[test]
    fn cloud_proxy_upload_validates_the_reference_size_boundary_without_allocating_it() {
        validate_cloud_proxy_upload_size(MAX_CLOUD_PROXY_UPLOAD_BYTES)
            .expect("reference maximum is accepted");
        let error = validate_cloud_proxy_upload_size(MAX_CLOUD_PROXY_UPLOAD_BYTES + 1)
            .expect_err("oversized proxy upload");
        assert_eq!(error.code, tuneweave_core::ErrorCode::InvalidRequest);
        assert_eq!(error.details["max_bytes"], MAX_CLOUD_PROXY_UPLOAD_BYTES);
    }

    #[tokio::test]
    async fn cloud_upload_ticket_uses_platform_account_and_reference_aliases() {
        let (status, json) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/account/cloud/uploads/ticket?platform=netease&account=locker",
            Some(json!({
                "md5": "0123456789ABCDEF0123456789ABCDEF",
                "fileSize": 42,
                "filename": "反方向的钟.flac",
                "bitrate": 999000,
                "contentType": "audio/flac"
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["upload_required"], true);
        assert_eq!(json["data"]["provisional_track_id"], "123");
        assert_eq!(json["data"]["resource_id"], "resource-456");
        assert_eq!(json["data"]["upload_method"], "POST");
        assert_eq!(json["data"]["upload_headers"]["Content-Length"], "42");
        assert_eq!(
            json["data"]["upload_headers"]["Content-MD5"],
            "0123456789abcdef0123456789abcdef"
        );
        assert_eq!(
            json["data"]["upload_headers"]["x-nos-token"],
            "upload-secret"
        );
        assert_eq!(json["data"]["extensions"]["filename"], "反方向的钟.flac");
        assert_eq!(json["data"]["extensions"]["bitrate"], 999_000);
        assert_eq!(json["data"]["extensions"]["content_type"], "audio/flac");
        assert_eq!(json["data"]["extensions"]["account"], "locker");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "locker");
    }

    #[tokio::test]
    async fn cloud_upload_completion_uses_reference_aliases_and_default_bitrate() {
        let (status, json) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/account/cloud/uploads/complete?platform=netease&account=locker",
            Some(json!({
                "songId": "123",
                "resourceId": "resource-456",
                "md5": "0123456789abcdef0123456789abcdef",
                "filename": "反方向的钟.flac",
                "song": "反方向的钟",
                "artist": "周杰伦",
                "album": "Jay"
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["track_ref"], "netease:123");
        assert_eq!(json["data"]["published"], true);
        assert_eq!(json["data"]["extensions"]["resource_id"], "resource-456");
        assert_eq!(json["data"]["extensions"]["song_name"], "反方向的钟");
        assert_eq!(json["data"]["extensions"]["artist"], "周杰伦");
        assert_eq!(json["data"]["extensions"]["album"], "Jay");
        assert_eq!(json["data"]["extensions"]["bitrate"], 999_000);
        assert_eq!(json["data"]["extensions"]["account"], "locker");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "locker");
    }

    #[tokio::test]
    async fn cloud_upload_routes_reject_invalid_json_and_zero_numeric_fields() {
        for (path, body) in [
            (
                "/v1/account/cloud/uploads/ticket",
                json!({
                    "md5": "0123456789abcdef0123456789abcdef",
                    "file_size": 0,
                    "filename": "song.mp3"
                }),
            ),
            (
                "/v1/account/cloud/uploads/ticket",
                json!({
                    "md5": "0123456789abcdef0123456789abcdef",
                    "file_size": 1,
                    "filename": "song.mp3",
                    "bitrate": 0
                }),
            ),
            (
                "/v1/account/cloud/uploads/complete",
                json!({
                    "provisional_track_id": "123",
                    "resource_id": "resource-456",
                    "md5": "0123456789abcdef0123456789abcdef",
                    "filename": "song.mp3",
                    "bitrate": 0
                }),
            ),
            (
                "/v1/account/cloud/uploads/ticket",
                json!({
                    "md5": "0123456789abcdef0123456789abcdef",
                    "file_size": 1,
                    "filename": "song.mp3",
                    "unexpected": true
                }),
            ),
        ] {
            let (status, json) =
                json_request_from(test_app_with_provider(), Method::POST, path, Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(json["error"]["code"], "invalid_request", "{path}");
        }
    }

    #[tokio::test]
    async fn cloud_import_accepts_reference_aliases_and_stringified_numbers() {
        let (status, json) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/account/cloud/imports?platform=netease&account=locker",
            Some(json!({
                "md5": "d02b8ab79d91c01167ba31e349fe5275",
                "id": 185809,
                "bitrate": "1652999",
                "fileSize": "50412168",
                "fileType": "flac",
                "song": "最伟大的作品",
                "artist": "周杰伦",
                "album": "最伟大的作品"
            })),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["track_ref"], "netease:cloud-imported");
        assert_eq!(json["data"]["imported"], true);
        assert_eq!(json["data"]["already_present"], false);
        assert_eq!(json["data"]["extensions"]["source_track_id"], "185809");
        assert_eq!(json["data"]["extensions"]["bitrate"], 1_652_999);
        assert_eq!(json["data"]["extensions"]["file_size"], 50_412_168);
        assert_eq!(json["data"]["extensions"]["file_type"], "flac");
        assert_eq!(json["data"]["extensions"]["song_name"], "最伟大的作品");
        assert_eq!(json["data"]["extensions"]["account"], "locker");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "locker");
    }

    #[tokio::test]
    async fn cloud_lyrics_accept_reference_query_names_and_opaque_track_ids() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/account/cloud/lyrics?platform=netease&account=locker&uid=32953014&sid=cloud-song",
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["track_ref"], "netease:cloud-song");
        assert_eq!(json["data"]["plain"], "[00:01.00]云盘歌词");
        assert_eq!(json["data"]["format"], "lrc");
        assert_eq!(json["data"]["extensions"]["user_id"], "32953014");
        assert_eq!(json["data"]["extensions"]["account"], "locker");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "locker");
    }

    #[tokio::test]
    async fn cloud_match_supports_reference_matching_and_cancellation_branches() {
        let (status, matched) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/account/cloud/matches?platform=netease&account=locker",
            Some(json!({
                "uid": 32953014,
                "sid": "cloud-song",
                "asid": 185809
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(matched["data"]["cloud_track_ref"], "netease:cloud-song");
        assert_eq!(matched["data"]["target_track_ref"], "netease:185809");
        assert_eq!(matched["data"]["matched"], true);
        assert_eq!(matched["data"]["extensions"]["user_id"], "32953014");
        assert_eq!(matched["meta"]["account"], "locker");

        let (status, canceled) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/account/cloud/matches?platform=netease&account=locker",
            Some(json!({
                "user_id": "32953014",
                "cloud_track_id": "cloud-song",
                "target_track_id": 0
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(canceled["data"]["matched"], false);
        assert_eq!(canceled["data"]["target_track_ref"], Value::Null);
    }

    #[tokio::test]
    async fn cloud_account_routes_reject_missing_non_scalar_and_out_of_range_inputs() {
        for (method, path, body) in [
            (
                Method::POST,
                "/v1/account/cloud/imports",
                Some(json!({
                    "md5": "d02b8ab79d91c01167ba31e349fe5275",
                    "bitrate": 999,
                    "file_size": 1,
                    "file_type": "flac",
                    "song_name": "song"
                })),
            ),
            (
                Method::POST,
                "/v1/account/cloud/imports",
                Some(json!({
                    "md5": "d02b8ab79d91c01167ba31e349fe5275",
                    "bitrate": 128000,
                    "file_size": 0,
                    "file_type": "flac",
                    "song_name": "song"
                })),
            ),
            (
                Method::POST,
                "/v1/account/cloud/matches",
                Some(json!({
                    "user_id": { "id": 32953014 },
                    "cloud_track_id": "cloud-song"
                })),
            ),
            (
                Method::POST,
                "/v1/account/cloud/matches",
                Some(json!({
                    "user_id": "32953014",
                    "cloud_track_id": "cloud-song",
                    "unexpected": true
                })),
            ),
        ] {
            let (status, json) =
                json_request_from(test_app_with_provider(), method, path, body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(json["error"]["code"], "invalid_request", "{path}");
        }

        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/account/cloud/lyrics?uid=32953014",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], "invalid_request");
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
    async fn netease_calendar_accepts_reference_and_unified_parameter_names() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/extensions/netease/calendar?startTime=1606752000000&endTime=1609430399999&account=calendar-user",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["ok"], true);
        assert_eq!(json["data"]["uri"], "/api/mcalendar/detail");
        assert_eq!(json["data"]["data"]["startTime"], 1_606_752_000_000_u64);
        assert_eq!(json["data"]["data"]["endTime"], 1_609_430_399_999_u64);
        assert_eq!(json["data"]["crypto"], "weapi");
        assert_eq!(json["data"]["account"], "calendar-user");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "calendar-user");

        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/extensions/netease/calendar?start_time=1606752000001&end_time=1609430399998",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["data"]["startTime"], 1_606_752_000_001_u64);
        assert_eq!(json["data"]["data"]["endTime"], 1_609_430_399_998_u64);
        assert!(json["data"]["account"].is_null());
        assert!(json["meta"].get("account").is_none());
    }

    #[tokio::test]
    async fn netease_calendar_defaults_both_runtime_timestamps_to_the_same_current_time() {
        let before = unix_time_millis().expect("current time before request");
        let (status, json) =
            json_response_from(test_app_with_provider(), "/v1/extensions/netease/calendar").await;
        let after = unix_time_millis().expect("current time after request");
        assert_eq!(status, StatusCode::OK);
        let start_time = json["data"]["data"]["startTime"]
            .as_u64()
            .expect("numeric startTime");
        let end_time = json["data"]["data"]["endTime"]
            .as_u64()
            .expect("numeric endTime");
        assert_eq!(start_time, end_time);
        assert!((before..=after).contains(&start_time));
    }

    #[tokio::test]
    async fn netease_calendar_rejects_invalid_timestamps_before_provider_dispatch() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/extensions/netease/calendar?startTime=tomorrow&endTime=-1",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], "invalid_request");
        assert_eq!(json["error"]["details"]["parameter"], "start_time");
        assert_eq!(json["error"]["details"]["value"], "tomorrow");
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
