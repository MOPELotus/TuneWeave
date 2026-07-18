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
        rejection::{BytesRejection, JsonRejection, QueryRejection},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use rand::{RngExt, distr::Alphanumeric};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tuneweave_core::{
    AccountProfile, Album, AlbumListRequest, AlbumStats, Artist, ArtistArea, ArtistCategory,
    ArtistChart, ArtistChartArea, ArtistChartRequest, ArtistListRequest, ArtistOverview,
    ArtistStats, ArtistTrackListRequest, ArtistTrackOrder, ArtistUpdatesRequest,
    ArtistVideoListRequest, ArtistWorkUpdate, ArtistWorksRequest, AudioRecognition,
    AudioRecognitionRequest, AuthChallengeRequest, AuthChallengeValidation, AuthPrincipalStatus,
    AuthPrincipalStatusRequest, AuthState, Banner, BannerClient, BannerListRequest, Capability,
    ChallengeMethod, ChartCatalog, ChartCatalogRequest, ChartCatalogView, CloudImportRequest,
    CloudImportResult, CloudLyricsRequest, CloudMatchRequest, CloudMatchResult, CloudTrack,
    CloudTrackDeleteRequest, CloudTrackDeleteResult, CloudTrackDetailRequest,
    CloudUploadCompleteRequest, CloudUploadRequest, CloudUploadResult, CloudUploadTicket,
    CloudUploadTicketRequest, Comment, CommentDeleteRequest, CommentListRequest, CommentListView,
    CommentMutationResult, CommentPage, CommentReaction, CommentReactionKind,
    CommentReactionListRequest, CommentReactionMutationRequest, CommentReactionMutationResult,
    CommentReactionPage, CommentReportRequest, CommentReportResult, CommentSort, CommentTarget,
    CommentTargetKind, CommentThreadStatsBatch, CommentThreadStatsRequest, CommentWriteRequest,
    CountryCallingCodeGroup, CountryCallingCodeListRequest, DigitalAlbum, DigitalAlbumChartEntry,
    DigitalAlbumChartKind, DigitalAlbumChartPeriod, DigitalAlbumChartRequest,
    DigitalAlbumListRequest, DimensionChart, DimensionChartRequest, DimensionChartTrackSnapshot,
    ErrorCode, Extensions, ImageUploadRequest, ImageUploadResult, LocalTrackMatchRequest,
    LocalTrackMatchResult, Lyrics, MediaDownload, MediaStream, MembershipSummary, PageRequest,
    PasswordFormat, PasswordLoginRequest, Platform, PlatformApiRequest, PlatformBatchRequest,
    PlaybackHistoryEntry, PlaybackHistoryPeriod, PlaybackHistoryRequest, Playlist,
    PlaylistCoverUpdateResult, PlaylistCreateRequest, PlaylistDeleteRequest, PlaylistDeleteResult,
    PlaylistItemKind, PlaylistItemMutationAction, PlaylistItemMutationRequest,
    PlaylistItemMutationResult, PlaylistKind, PlaylistMetadataUpdateVariant,
    PlaylistMutationResult, PlaylistOrderRequest, PlaylistOrderResult, PlaylistTrackOrderRequest,
    PlaylistTrackOrderResult, PlaylistUpdateRequest, PlaylistVisibility, Podcast, PodcastCatalog,
    PodcastEpisode, PodcastEpisodeListRequest, PodcastEpisodeLyrics, PodcastEpisodeStream,
    PodcastListRequest, PodcastTaxonomy, PrincipalType, ProviderRegistry, Quality, RadioStation,
    RadioStationCursor, RadioStationListRequest, RadioTaxonomy, RadioTaxonomyRequest,
    RecommendationRequest, ResolutionAttempt, ResolutionStatus, ResolveRequest, ResourceRef,
    SearchDefaultKeyword, SearchDefaultKeywordRequest, SearchItem, SearchKind, SearchMultiMatch,
    SearchMultiMatchRequest, SearchQuery, SearchSuggestionClient, SearchSuggestionList,
    SearchSuggestionRequest, SearchTrendingDetail, SearchTrendingList, SearchTrendingRequest,
    SearchVariant, StreamBatch, StreamOutcome, StreamRequest, StreamResolver, StreamVariant,
    SubscriptionResult, Track, TrackAvailability, TrackAvailabilityRequest, TrackEntitlement,
    TuneWeaveError, User, Video, VideoDetail, VideoDetailRequest, VideoKind, VideoResourceKind,
    VideoStats, VideoStream, VideoStreamRequest,
};

pub use response::{ApiError, ApiResponse, ResponseMeta};

const AUTH_TRANSACTION_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_AVATAR_UPLOAD_BYTES: usize = 20 * 1024 * 1024;
const MAX_PLAYLIST_COVER_UPLOAD_BYTES: usize = 20 * 1024 * 1024;
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
        .route("/search/default", get(search_default))
        .route("/search/trending", get(search_trending))
        .route("/search/suggestions", get(search_suggestions))
        .route("/search/multimatch", get(search_multi_match))
        .route(
            "/search/match",
            get(search_local_track_match_get).post(search_local_track_match_post),
        )
        .route("/banners", get(banners))
        .route("/radio/taxonomy", get(radio_taxonomy))
        .route("/radio/stations", get(radio_stations))
        .route("/radio/stations/{reference}", get(radio_station))
        .route("/podcasts/categories", get(podcast_categories))
        .route("/podcasts", get(podcasts))
        .route("/podcasts/{reference}", get(podcast))
        .route("/podcasts/{reference}/episodes", get(podcast_episodes))
        .route("/episodes/{reference}", get(podcast_episode))
        .route("/episodes/{reference}/lyrics", get(podcast_episode_lyrics))
        .route("/episodes/{reference}/stream", get(podcast_episode_stream))
        .route(
            "/episodes/{reference}/stream/redirect",
            get(podcast_episode_stream_redirect),
        )
        .route("/audio/recognize", post(audio_recognize))
        .route(
            "/tracks/streams",
            get(track_streams_get).post(track_streams_post),
        )
        .route("/tracks/{reference}", get(track))
        .route("/tracks/{reference}/availability", get(track_availability))
        .route(
            "/tracks/{reference}/download/redirect",
            get(track_download_redirect),
        )
        .route("/tracks/{reference}/download", get(track_download))
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
        .route("/charts", get(chart_catalog))
        .route("/charts/artists", get(artist_chart))
        .route("/charts/digital-albums", get(digital_album_chart))
        .route("/charts/dimensions/{chart_code}", get(dimension_chart))
        .route(
            "/charts/dimensions/{chart_code}/tracks",
            get(dimension_chart_tracks),
        )
        .route("/charts/{reference}/tracks", get(playlist_tracks))
        .route("/artists", get(artists))
        .route("/artists/{reference}", get(artist))
        .route("/artists/{reference}/overview", get(artist_overview))
        .route("/artists/{reference}/stats", get(artist_stats))
        .route("/artists/{reference}/albums", get(artist_albums))
        .route("/artists/{reference}/fans", get(artist_fans))
        .route("/artists/{reference}/videos", get(artist_videos))
        .route("/artists/{reference}/tracks", get(artist_tracks))
        .route("/artists/{reference}/top-tracks", get(artist_top_tracks))
        .route("/videos/{reference}", get(video_detail))
        .route("/videos/{reference}/stats", get(video_stats))
        .route("/videos/{reference}/stream", get(video_stream))
        .route(
            "/videos/{reference}/stream/redirect",
            get(video_stream_redirect),
        )
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
        .route("/playlists", post(playlist_create).delete(playlists_delete))
        .route(
            "/playlists/{reference}",
            get(playlist).patch(playlist_update).delete(playlist_delete),
        )
        .route(
            "/playlists/{reference}/tracks",
            get(playlist_tracks)
                .post(playlist_tracks_add)
                .delete(playlist_tracks_remove),
        )
        .route(
            "/playlists/{reference}/videos",
            post(playlist_videos_add).delete(playlist_videos_remove),
        )
        .route(
            "/playlists/{reference}/items",
            post(playlist_items_add).delete(playlist_items_remove),
        )
        .route(
            "/playlists/{reference}/tracks/order",
            put(playlist_tracks_order),
        )
        .route(
            "/playlists/{reference}/cover",
            put(playlist_cover_update)
                .layer(DefaultBodyLimit::max(MAX_PLAYLIST_COVER_UPLOAD_BYTES)),
        )
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
            "/resources/{kind}/{reference}/comments/{comment_id}/reports",
            post(comment_report),
        )
        .route(
            "/resources/{kind}/{reference}/comments/{comment_id}/reactions/{reaction}",
            get(comment_reaction_list)
                .put(comment_reaction_enable)
                .delete(comment_reaction_disable),
        )
        .route(
            "/users/{reference}/favorites/tracks",
            get(user_favorite_tracks),
        )
        .route("/users/{reference}/membership", get(user_membership))
        .route("/users/{reference}/history", get(user_history))
        .route("/recommendations/tracks", get(recommended_tracks))
        .route("/recommendations/playlists", get(recommended_playlists))
        .route("/auth/country-codes", get(auth_country_calling_codes))
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
        .route("/account/membership", get(account_membership))
        .route(
            "/account/avatar",
            put(account_avatar).layer(DefaultBodyLimit::max(MAX_AVATAR_UPLOAD_BYTES)),
        )
        .route(
            "/account/cloud/tracks",
            get(cloud_tracks).delete(cloud_tracks_delete),
        )
        .route(
            "/account/cloud/tracks/details",
            get(cloud_track_details_get).post(cloud_track_details_post),
        )
        .route(
            "/account/cloud/tracks/{reference}/download/redirect",
            get(cloud_track_download_redirect),
        )
        .route(
            "/account/cloud/tracks/{reference}/download",
            get(cloud_track_download),
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
        .route("/account/playlists/order", put(account_playlists_order))
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
    #[serde(alias = "backend")]
    variant: Option<String>,
    platform: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
    account: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchDefaultParams {
    platform: Option<String>,
    account: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchTrendingParams {
    platform: Option<String>,
    account: Option<String>,
    #[serde(alias = "mode")]
    detail: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchSuggestionParams {
    #[serde(alias = "keywords", alias = "keyword")]
    q: Option<String>,
    #[serde(alias = "type")]
    client: Option<String>,
    platform: Option<String>,
    account: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchMultiMatchParams {
    #[serde(alias = "keywords", alias = "keyword")]
    q: Option<String>,
    #[serde(alias = "type")]
    kind: Option<String>,
    platform: Option<String>,
    account: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalTrackMatchParams {
    title: Option<String>,
    album: Option<String>,
    artist: Option<String>,
    duration_ms: Option<String>,
    #[serde(alias = "duration")]
    duration_seconds: Option<String>,
    md5: Option<String>,
    platform: Option<String>,
    account: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalTrackMatchBody {
    #[serde(default)]
    title: String,
    #[serde(default)]
    album: String,
    #[serde(default)]
    artist: String,
    #[serde(default)]
    duration_ms: Option<Value>,
    #[serde(default, alias = "duration")]
    duration_seconds: Option<Value>,
    md5: String,
    platform: Option<String>,
    account: Option<String>,
}

struct LocalTrackMatchInput {
    title: String,
    album: String,
    artist: String,
    duration_ms: u64,
    md5: String,
    platform: Option<String>,
    account: Option<String>,
}

async fn search_default(
    State(state): State<AppState>,
    params: Result<Query<SearchDefaultParams>, QueryRejection>,
) -> Result<Json<ApiResponse<SearchDefaultKeyword>>, ApiError> {
    let params = query_params(params)?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let prompt = provider
        .default_search_keyword(&SearchDefaultKeywordRequest {
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(prompt)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn search_trending(
    State(state): State<AppState>,
    params: Result<Query<SearchTrendingParams>, QueryRejection>,
) -> Result<Json<ApiResponse<SearchTrendingList>>, ApiError> {
    let params = query_params(params)?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let detail = parse_search_trending_detail(params.detail.as_deref())?;
    let provider = state.registry.require(platform)?;
    let list = provider
        .trending_searches(&SearchTrendingRequest {
            detail,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(list)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn search_suggestions(
    State(state): State<AppState>,
    params: Result<Query<SearchSuggestionParams>, QueryRejection>,
) -> Result<Json<ApiResponse<SearchSuggestionList>>, ApiError> {
    let params = query_params(params)?;
    let query = params
        .q
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| TuneWeaveError::invalid_request("q must not be empty"))?;
    let client = parse_search_suggestion_client(params.client.as_deref())?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let list = provider
        .search_suggestions(&SearchSuggestionRequest {
            query: query.to_owned(),
            client,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(list)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn search_multi_match(
    State(state): State<AppState>,
    params: Result<Query<SearchMultiMatchParams>, QueryRejection>,
) -> Result<Json<ApiResponse<SearchMultiMatch>>, ApiError> {
    let params = query_params(params)?;
    let query = params
        .q
        .as_deref()
        .map(str::trim)
        .filter(|query| !query.is_empty())
        .ok_or_else(|| TuneWeaveError::invalid_request("q must not be empty"))?;
    let kind = parse_search_kind(params.kind.as_deref())?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .search_multi_match(&SearchMultiMatchRequest {
            query: query.to_owned(),
            kind,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn search_local_track_match_get(
    State(state): State<AppState>,
    params: Result<Query<LocalTrackMatchParams>, QueryRejection>,
) -> Result<Json<ApiResponse<LocalTrackMatchResult>>, ApiError> {
    let params = query_params(params)?;
    let duration_ms = params.duration_ms.map(Value::String);
    let duration_seconds = params.duration_seconds.map(Value::String);
    execute_local_track_match(
        &state,
        LocalTrackMatchInput {
            title: params.title.unwrap_or_default(),
            album: params.album.unwrap_or_default(),
            artist: params.artist.unwrap_or_default(),
            duration_ms: parse_local_track_match_duration(
                duration_ms.as_ref(),
                duration_seconds.as_ref(),
            )?,
            md5: required_trimmed("md5", params.md5)?,
            platform: params.platform,
            account: params.account,
        },
    )
    .await
}

async fn search_local_track_match_post(
    State(state): State<AppState>,
    payload: Result<Json<LocalTrackMatchBody>, JsonRejection>,
) -> Result<Json<ApiResponse<LocalTrackMatchResult>>, ApiError> {
    let body = json_body(payload)?;
    execute_local_track_match(
        &state,
        LocalTrackMatchInput {
            title: body.title,
            album: body.album,
            artist: body.artist,
            duration_ms: parse_local_track_match_duration(
                body.duration_ms.as_ref(),
                body.duration_seconds.as_ref(),
            )?,
            md5: required_trimmed("md5", Some(body.md5))?,
            platform: body.platform,
            account: body.account,
        },
    )
    .await
}

async fn execute_local_track_match(
    state: &AppState,
    input: LocalTrackMatchInput,
) -> Result<Json<ApiResponse<LocalTrackMatchResult>>, ApiError> {
    let platform = account_platform(state, input.platform.as_deref())?;
    let account = account_alias(input.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .match_local_track(&LocalTrackMatchRequest {
            title: input.title,
            album: input.album,
            artist: input.artist,
            duration_ms: input.duration_ms,
            md5: input.md5,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
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
    let variant = parse_search_variant(params.variant.as_deref())?;
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
        variant,
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

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PodcastCategoriesParams {
    platform: Option<String>,
    account: Option<String>,
}

async fn podcast_categories(
    State(state): State<AppState>,
    params: Result<Query<PodcastCategoriesParams>, QueryRejection>,
) -> Result<Json<ApiResponse<PodcastTaxonomy>>, ApiError> {
    let params = query_params(params)?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = optional_trimmed(params.account);
    let provider = state.registry.require(platform)?;
    let taxonomy = provider.podcast_categories(account.as_deref()).await?;
    let mut response = ApiResponse::new(taxonomy).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PodcastListParams {
    platform: Option<String>,
    account: Option<String>,
    catalog: Option<String>,
    #[serde(alias = "categoryId")]
    category_id: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
}

async fn podcasts(
    State(state): State<AppState>,
    params: Result<Query<PodcastListParams>, QueryRejection>,
) -> Result<Json<ApiResponse<Vec<Podcast>>>, ApiError> {
    let params = query_params(params)?;
    let catalog = parse_podcast_catalog(params.catalog.as_deref())?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = optional_trimmed(params.account);
    let provider = state.registry.require(platform)?;
    let page = provider
        .podcasts(&PodcastListRequest {
            catalog,
            category_id: optional_trimmed(params.category_id),
            limit,
            offset,
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

async fn podcast(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<Podcast>>, ApiError> {
    let reference = parse_reference(reference)?;
    let account = optional_trimmed(params.account);
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let podcast = provider.podcast(reference.id(), account.as_deref()).await?;
    let mut response = ApiResponse::new(podcast).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

#[derive(Debug, Default, Deserialize)]
struct PodcastEpisodeListParams {
    account: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
    #[serde(alias = "asc")]
    ascending: Option<String>,
}

async fn podcast_episodes(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<PodcastEpisodeListParams>,
) -> Result<Json<ApiResponse<Vec<PodcastEpisode>>>, ApiError> {
    let reference = parse_reference(reference)?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let ascending = parse_bool_parameter("ascending", params.ascending.as_deref(), false)?;
    let account = optional_trimmed(params.account);
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let page = provider
        .podcast_episodes(
            reference.id(),
            &PodcastEpisodeListRequest {
                limit,
                offset,
                ascending,
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

async fn podcast_episode(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
) -> Result<Json<ApiResponse<PodcastEpisode>>, ApiError> {
    let reference = parse_reference(reference)?;
    let account = optional_trimmed(params.account);
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let episode = provider
        .podcast_episode(reference.id(), account.as_deref())
        .await?;
    let mut response = ApiResponse::new(episode).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PodcastEpisodeLyricsParams {
    account: Option<String>,
}

async fn podcast_episode_lyrics(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<PodcastEpisodeLyricsParams>, QueryRejection>,
) -> Result<Json<ApiResponse<PodcastEpisodeLyrics>>, ApiError> {
    let params = query_params(params)?;
    let reference = parse_reference(reference)?;
    let account = optional_trimmed(params.account);
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let lyrics = provider
        .podcast_episode_lyrics(reference.id(), account.as_deref())
        .await?;
    let mut response = ApiResponse::new(lyrics).with_platform(platform);
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
struct ChartCatalogParams {
    platform: Option<String>,
    account: Option<String>,
    #[serde(alias = "catalog")]
    view: Option<String>,
}

async fn chart_catalog(
    State(state): State<AppState>,
    Query(params): Query<ChartCatalogParams>,
) -> Result<Json<ApiResponse<ChartCatalog>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = optional_trimmed(params.account);
    let provider = state.registry.require(platform)?;
    let mut request = ChartCatalogRequest::new(parse_chart_catalog_view(params.view.as_deref())?);
    request.account.clone_from(&account);
    let catalog = provider.chart_catalog(&request).await?;
    let mut response = ApiResponse::new(catalog).with_platform(platform);
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

#[derive(Debug, Default, Deserialize)]
struct ArtistChartParams {
    platform: Option<String>,
    account: Option<String>,
    area: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

async fn artist_chart(
    State(state): State<AppState>,
    Query(params): Query<ArtistChartParams>,
) -> Result<Json<ApiResponse<ArtistChart>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = optional_trimmed(params.account);
    let area = resolve_artist_chart_area(params.area.as_deref(), params.kind.as_deref())?;
    let provider = state.registry.require(platform)?;
    let mut request = ArtistChartRequest::new(area);
    request.account.clone_from(&account);
    let chart = provider.artist_chart(&request).await?;
    let mut response = ApiResponse::new(chart).with_platform(platform);
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
#[serde(deny_unknown_fields)]
struct StreamParams {
    #[serde(alias = "level")]
    quality: Option<String>,
    #[serde(alias = "backend")]
    variant: Option<String>,
    #[serde(alias = "br")]
    bitrate: Option<String>,
    playback_platform: Option<String>,
    fallback: Option<String>,
    fallback_platforms: Option<String>,
    unblock: Option<String>,
    source: Option<String>,
    account: Option<String>,
}

#[derive(Clone, Copy, Debug, Default)]
struct StreamControlInput<'a> {
    quality: Option<&'a str>,
    variant: Option<&'a str>,
    bitrate: Option<&'a str>,
    playback_platform: Option<&'a str>,
    fallback: Option<&'a str>,
    fallback_platforms: Option<&'a str>,
    unblock: Option<&'a str>,
    source: Option<&'a str>,
    account: Option<&'a str>,
}

#[derive(Clone, Debug)]
struct StreamControls {
    quality: Quality,
    variant: StreamVariant,
    bitrate: Option<u64>,
    routing: StreamRouting,
    account: Option<String>,
}

#[derive(Clone, Debug)]
enum StreamRouting {
    Standard {
        preferred_platform: Option<Platform>,
        fallback: bool,
        fallback_platforms: Vec<Platform>,
    },
    Unblock {
        source: Option<Platform>,
    },
}

impl StreamControls {
    fn resolve_request(&self, origin_platform: Platform) -> ResolveRequest {
        let (playback_platforms, fallback, account_platform) = match &self.routing {
            StreamRouting::Standard {
                preferred_platform,
                fallback,
                fallback_platforms,
            } => {
                let mut platforms = Vec::new();
                if let Some(platform) = preferred_platform {
                    platforms.push(*platform);
                } else if !fallback_platforms.is_empty() {
                    platforms.push(origin_platform);
                }
                platforms.extend(fallback_platforms.iter().copied());
                (
                    platforms,
                    *fallback,
                    preferred_platform.unwrap_or(origin_platform),
                )
            }
            StreamRouting::Unblock { source } => {
                let mut platforms = source.map_or_else(
                    || {
                        vec![
                            Platform::Qq,
                            Platform::Kugou,
                            Platform::Kuwo,
                            Platform::Migu,
                        ]
                    },
                    |source| vec![source],
                );
                platforms.push(origin_platform);
                let account_platform = platforms[0];
                (platforms, true, account_platform)
            }
        };
        let mut request = ResolveRequest {
            quality: self.quality,
            variant: self.variant,
            bitrate: self.bitrate,
            playback_platforms,
            fallback,
            ..ResolveRequest::default()
        };
        if let Some(account) = self.account.clone() {
            request.accounts.insert(account_platform, account);
        }
        request
    }

    fn starts_with_origin(&self, origin_platform: Platform) -> bool {
        match &self.routing {
            StreamRouting::Standard {
                preferred_platform, ..
            } => preferred_platform.is_none_or(|platform| platform == origin_platform),
            StreamRouting::Unblock { source } => source.unwrap_or(Platform::Qq) == origin_platform,
        }
    }

    fn fallback_enabled(&self) -> bool {
        match self.routing {
            StreamRouting::Standard { fallback, .. } => fallback,
            StreamRouting::Unblock { .. } => true,
        }
    }

    fn provider_request(&self, platform: Platform) -> StreamRequest {
        let account_platform = match self.routing {
            StreamRouting::Standard {
                preferred_platform, ..
            } => preferred_platform.unwrap_or(platform),
            StreamRouting::Unblock { source } => source.unwrap_or(Platform::Qq),
        };
        StreamRequest {
            quality: self.quality,
            variant: self.variant,
            bitrate: self.bitrate,
            account: (account_platform == platform)
                .then(|| self.account.clone())
                .flatten(),
        }
    }
}

fn parse_stream_controls(input: StreamControlInput<'_>) -> Result<StreamControls, TuneWeaveError> {
    let quality = parse_quality(input.quality)?;
    let variant = parse_stream_variant(input.variant)?;
    let bitrate = parse_optional_u64_parameter("bitrate", input.bitrate)?;
    let unblock = parse_bool_parameter("unblock", input.unblock, false)?;
    let fallback = parse_bool_parameter("fallback", input.fallback, true)?;
    let routing = if unblock {
        if input.playback_platform.is_some() || input.fallback_platforms.is_some() {
            return Err(TuneWeaveError::invalid_request(
                "unblock cannot be combined with playback_platform or fallback_platforms",
            )
            .with_details(json!({
                "conflicts": ["playback_platform", "fallback_platforms"]
            })));
        }
        StreamRouting::Unblock {
            source: input.source.map(parse_platform_parameter).transpose()?,
        }
    } else {
        StreamRouting::Standard {
            preferred_platform: input
                .playback_platform
                .map(parse_platform_parameter)
                .transpose()?,
            fallback,
            fallback_platforms: parse_platform_list(input.fallback_platforms)?,
        }
    };
    Ok(StreamControls {
        quality,
        variant,
        bitrate,
        routing,
        account: input
            .account
            .map(str::trim)
            .filter(|account| !account.is_empty())
            .map(str::to_owned),
    })
}

async fn track_stream(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<StreamParams>, QueryRejection>,
) -> Result<Json<ApiResponse<MediaStream>>, ApiError> {
    let params = query_params(params)?;
    let reference = parse_reference(reference)?;
    let controls = parse_stream_controls(StreamControlInput {
        quality: params.quality.as_deref(),
        variant: params.variant.as_deref(),
        bitrate: params.bitrate.as_deref(),
        playback_platform: params.playback_platform.as_deref(),
        fallback: params.fallback.as_deref(),
        fallback_platforms: params.fallback_platforms.as_deref(),
        unblock: params.unblock.as_deref(),
        source: params.source.as_deref(),
        account: params.account.as_deref(),
    })?;
    let request = controls.resolve_request(reference.platform());

    let origin_provider = state.registry.require(reference.platform())?;
    let origin_request = controls.provider_request(reference.platform());
    let origin = origin_provider
        .track(reference.id(), origin_request.account.as_deref())
        .await?;
    let stream = state.resolver.resolve(&origin, &request).await?;
    let resolved_platform = stream.resolved_platform;
    let mut response = ApiResponse::new(stream).with_platform(resolved_platform);
    if let Some(account) = controls.account {
        response = response.with_account(account);
    }

    Ok(Json(response))
}

async fn resolve_podcast_episode_stream(
    state: &AppState,
    reference: &ResourceRef,
    controls: &StreamControls,
) -> Result<PodcastEpisodeStream, TuneWeaveError> {
    let origin_platform = reference.platform();
    let origin_provider = state.registry.require(origin_platform)?;
    let origin_request = controls.provider_request(origin_platform);
    let episode = origin_provider
        .podcast_episode(reference.id(), origin_request.account.as_deref())
        .await?;
    let audio = episode.audio.as_ref().ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "podcast episode did not expose a playable audio resource",
        )
        .with_platform(origin_platform)
        .with_details(json!({ "episode_ref": episode.resource_ref.to_string() }))
    })?;
    let episode_ref = episode.resource_ref.clone();
    let audio_ref = audio.resource_ref.clone();
    let stream = state
        .resolver
        .resolve(audio, &controls.resolve_request(origin_platform))
        .await?;
    Ok(PodcastEpisodeStream {
        episode_ref,
        audio_ref,
        stream,
        extensions: Extensions::from([("episode".to_owned(), json!(episode))]),
    })
}

async fn podcast_episode_stream(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<StreamParams>, QueryRejection>,
) -> Result<Json<ApiResponse<PodcastEpisodeStream>>, ApiError> {
    let params = query_params(params)?;
    let reference = parse_reference(reference)?;
    let controls = parse_stream_controls(StreamControlInput {
        quality: params.quality.as_deref(),
        variant: params.variant.as_deref(),
        bitrate: params.bitrate.as_deref(),
        playback_platform: params.playback_platform.as_deref(),
        fallback: params.fallback.as_deref(),
        fallback_platforms: params.fallback_platforms.as_deref(),
        unblock: params.unblock.as_deref(),
        source: params.source.as_deref(),
        account: params.account.as_deref(),
    })?;
    let result = resolve_podcast_episode_stream(&state, &reference, &controls).await?;
    let resolved_platform = result.stream.resolved_platform;
    let mut response = ApiResponse::new(result).with_platform(resolved_platform);
    if let Some(account) = controls.account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

async fn podcast_episode_stream_redirect(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<StreamParams>, QueryRejection>,
) -> Result<Response, ApiError> {
    let params = query_params(params)?;
    let reference = parse_reference(reference)?;
    let controls = parse_stream_controls(StreamControlInput {
        quality: params.quality.as_deref(),
        variant: params.variant.as_deref(),
        bitrate: params.bitrate.as_deref(),
        playback_platform: params.playback_platform.as_deref(),
        fallback: params.fallback.as_deref(),
        fallback_platforms: params.fallback_platforms.as_deref(),
        unblock: params.unblock.as_deref(),
        source: params.source.as_deref(),
        account: params.account.as_deref(),
    })?;
    let result = resolve_podcast_episode_stream(&state, &reference, &controls).await?;
    Ok(download_redirect_response(&result.stream.url))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct DownloadParams {
    #[serde(alias = "level")]
    quality: Option<String>,
    #[serde(alias = "backend")]
    variant: Option<String>,
    #[serde(alias = "br")]
    bitrate: Option<String>,
    account: Option<String>,
}

fn download_request(params: &DownloadParams) -> Result<StreamRequest, TuneWeaveError> {
    Ok(StreamRequest {
        quality: parse_quality(params.quality.as_deref())?,
        variant: parse_stream_variant(params.variant.as_deref())?,
        bitrate: parse_optional_u64_parameter("bitrate", params.bitrate.as_deref())?,
        account: params
            .account
            .as_deref()
            .map(str::trim)
            .filter(|account| !account.is_empty())
            .map(str::to_owned),
    })
}

async fn track_download(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<DownloadParams>, QueryRejection>,
) -> Result<Json<ApiResponse<MediaDownload>>, ApiError> {
    let params = query_params(params)?;
    let reference = parse_reference(reference)?;
    let request = download_request(&params)?;
    let provider = state.registry.require(reference.platform())?;
    let track = provider
        .track(reference.id(), request.account.as_deref())
        .await?;
    let download = provider.download(&track, &request).await?;
    let mut response = ApiResponse::new(download).with_platform(reference.platform());
    if let Some(account) = request.account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

async fn track_download_redirect(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<DownloadParams>, QueryRejection>,
) -> Result<Response, ApiError> {
    let params = query_params(params)?;
    let reference = parse_reference(reference)?;
    let request = download_request(&params)?;
    let provider = state.registry.require(reference.platform())?;
    let track = provider
        .track(reference.id(), request.account.as_deref())
        .await?;
    let download = provider.download(&track, &request).await?;
    if let Some(url) = download.url.as_deref() {
        return Ok(download_redirect_response(url));
    }
    match provider.stream(&track, &request).await {
        Ok(stream) => Ok(download_redirect_response(&stream.url)),
        Err(mut error) => {
            let stream_details = std::mem::take(&mut error.details);
            error.details = json!({
                "download": download,
                "stream": stream_details
            });
            Err(error.into())
        }
    }
}

fn download_redirect_response(url: &str) -> Response {
    (StatusCode::FOUND, [(header::LOCATION, url.to_owned())]).into_response()
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct StreamBatchParams {
    refs: Option<String>,
    #[serde(alias = "id")]
    ids: Option<String>,
    platform: Option<String>,
    #[serde(alias = "level")]
    quality: Option<String>,
    #[serde(alias = "backend")]
    variant: Option<String>,
    #[serde(alias = "br")]
    bitrate: Option<String>,
    playback_platform: Option<String>,
    fallback: Option<String>,
    fallback_platforms: Option<String>,
    unblock: Option<String>,
    source: Option<String>,
    account: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StreamReferenceInput {
    One(String),
    Many(Vec<String>),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StreamBooleanInput {
    Boolean(bool),
    String(String),
    Integer(i64),
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum StreamUnsignedInput {
    String(String),
    Integer(u64),
}

impl StreamUnsignedInput {
    fn into_parameter(self) -> String {
        match self {
            Self::String(value) => value,
            Self::Integer(value) => value.to_string(),
        }
    }
}

impl StreamBooleanInput {
    fn into_parameter(self) -> String {
        match self {
            Self::Boolean(value) => value.to_string(),
            Self::String(value) => value,
            Self::Integer(value) => value.to_string(),
        }
    }
}

impl StreamReferenceInput {
    fn into_values(self) -> Vec<String> {
        match self {
            Self::One(value) => vec![value],
            Self::Many(values) => values,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct StreamBatchBody {
    refs: Option<StreamReferenceInput>,
    #[serde(alias = "id")]
    ids: Option<StreamReferenceInput>,
    platform: Option<String>,
    #[serde(alias = "level")]
    quality: Option<String>,
    #[serde(alias = "backend")]
    variant: Option<String>,
    #[serde(alias = "br")]
    bitrate: Option<StreamUnsignedInput>,
    playback_platform: Option<String>,
    fallback: Option<StreamBooleanInput>,
    fallback_platforms: Option<String>,
    unblock: Option<StreamBooleanInput>,
    source: Option<String>,
    account: Option<String>,
}

async fn track_streams_get(
    State(state): State<AppState>,
    params: Result<Query<StreamBatchParams>, QueryRejection>,
) -> Result<Json<ApiResponse<StreamBatch>>, ApiError> {
    let params = query_params(params)?;
    let references = parse_stream_batch_references(
        params.refs.map(|value| vec![value]),
        params.ids.map(|value| vec![value]),
        params.platform.as_deref(),
        state.default_platform,
    )?;
    let controls = parse_stream_controls(StreamControlInput {
        quality: params.quality.as_deref(),
        variant: params.variant.as_deref(),
        bitrate: params.bitrate.as_deref(),
        playback_platform: params.playback_platform.as_deref(),
        fallback: params.fallback.as_deref(),
        fallback_platforms: params.fallback_platforms.as_deref(),
        unblock: params.unblock.as_deref(),
        source: params.source.as_deref(),
        account: params.account.as_deref(),
    })?;
    Ok(Json(
        stream_batch_response(&state, references, controls).await,
    ))
}

async fn track_streams_post(
    State(state): State<AppState>,
    body: Result<Json<StreamBatchBody>, JsonRejection>,
) -> Result<Json<ApiResponse<StreamBatch>>, ApiError> {
    let body = json_body(body)?;
    let references = parse_stream_batch_references(
        body.refs.map(StreamReferenceInput::into_values),
        body.ids.map(StreamReferenceInput::into_values),
        body.platform.as_deref(),
        state.default_platform,
    )?;
    let fallback = body.fallback.map(StreamBooleanInput::into_parameter);
    let unblock = body.unblock.map(StreamBooleanInput::into_parameter);
    let bitrate = body.bitrate.map(StreamUnsignedInput::into_parameter);
    let controls = parse_stream_controls(StreamControlInput {
        quality: body.quality.as_deref(),
        variant: body.variant.as_deref(),
        bitrate: bitrate.as_deref(),
        playback_platform: body.playback_platform.as_deref(),
        fallback: fallback.as_deref(),
        fallback_platforms: body.fallback_platforms.as_deref(),
        unblock: unblock.as_deref(),
        source: body.source.as_deref(),
        account: body.account.as_deref(),
    })?;
    Ok(Json(
        stream_batch_response(&state, references, controls).await,
    ))
}

fn parse_stream_batch_references(
    refs: Option<Vec<String>>,
    ids: Option<Vec<String>>,
    platform: Option<&str>,
    default_platform: Platform,
) -> Result<Vec<ResourceRef>, TuneWeaveError> {
    let (kind, values) = match (refs, ids) {
        (Some(_), Some(_)) => {
            return Err(TuneWeaveError::invalid_request(
                "refs and ids cannot be provided together",
            )
            .with_details(json!({ "conflicts": ["refs", "ids"] })));
        }
        (None, None) => {
            return Err(TuneWeaveError::invalid_request(
                "one of refs or ids must be provided",
            ));
        }
        (Some(values), None) => {
            if platform.is_some() {
                return Err(TuneWeaveError::invalid_request(
                    "platform can only be used with ids",
                ));
            }
            ("refs", split_stream_batch_values("refs", values)?)
        }
        (None, Some(values)) => ("ids", split_stream_batch_values("ids", values)?),
    };
    if kind == "refs" {
        values.into_iter().map(parse_reference).collect()
    } else {
        let platform = platform.map_or(Ok(default_platform), parse_platform_parameter)?;
        values
            .into_iter()
            .map(|id| {
                ResourceRef::new(platform, &id).map_err(|error| {
                    TuneWeaveError::invalid_request(format!("invalid stream id: {error}"))
                        .with_details(json!({ "id": id, "platform": platform }))
                })
            })
            .collect()
    }
}

fn split_stream_batch_values(
    name: &str,
    values: Vec<String>,
) -> Result<Vec<String>, TuneWeaveError> {
    let mut parsed = Vec::new();
    for value in values {
        for item in value.split(',') {
            let item = item.trim();
            if item.is_empty() {
                return Err(TuneWeaveError::invalid_request(format!(
                    "{name} must not contain empty items"
                ))
                .with_details(json!({ "parameter": name })));
            }
            parsed.push(item.to_owned());
        }
    }
    if parsed.is_empty() {
        return Err(TuneWeaveError::invalid_request(format!(
            "{name} must not be empty"
        )));
    }
    Ok(parsed)
}

async fn stream_batch_response(
    state: &AppState,
    references: Vec<ResourceRef>,
    controls: StreamControls,
) -> ApiResponse<StreamBatch> {
    let platform = references
        .first()
        .map(ResourceRef::platform)
        .filter(|platform| {
            references
                .iter()
                .all(|reference| reference.platform() == *platform)
        });
    let account = controls.account.clone();
    let batch = resolve_stream_batch(state, &references, &controls).await;
    let mut response = ApiResponse::new(batch);
    if let Some(platform) = platform {
        response = response.with_platform(platform);
    }
    if let Some(account) = account {
        response = response.with_account(account);
    }
    response
}

async fn resolve_stream_batch(
    state: &AppState,
    references: &[ResourceRef],
    controls: &StreamControls,
) -> StreamBatch {
    let mut outcomes = vec![None; references.len()];
    let mut direct_groups = BTreeMap::<Platform, Vec<(usize, Track)>>::new();
    for (index, reference) in references.iter().enumerate() {
        if controls.starts_with_origin(reference.platform()) {
            if state.registry.contains(reference.platform()) {
                direct_groups
                    .entry(reference.platform())
                    .or_default()
                    .push((
                        index,
                        Track::new(reference.clone(), reference.id().to_owned()),
                    ));
            } else {
                let error = TuneWeaveError::platform_unavailable(reference.platform());
                outcomes[index] = Some(stream_outcome_from_error(reference, &error));
            }
        } else {
            outcomes[index] = Some(resolve_stream_reference(state, reference, controls).await);
        }
    }

    let mut provider_batches = BTreeMap::<String, Value>::new();
    for (platform, entries) in direct_groups {
        let Some(provider) = state.registry.get(platform) else {
            continue;
        };
        let request = controls.provider_request(platform);
        let tracks = entries
            .iter()
            .map(|(_, track)| track.clone())
            .collect::<Vec<_>>();
        match provider.streams(&tracks, &request).await {
            Ok(batch) => {
                provider_batches.insert(platform.to_string(), json!(batch.extensions));
                for (position, (index, track)) in entries.into_iter().enumerate() {
                    let initial = if let Some(outcome) = batch.outcomes.get(position) {
                        normalize_direct_stream_outcome(track, &request, outcome.clone())
                    } else {
                        let error = TuneWeaveError::new(
                            ErrorCode::UpstreamError,
                            "provider omitted a requested stream outcome",
                        )
                        .with_platform(platform)
                        .with_details(json!({
                            "position": position,
                            "track_ref": track.resource_ref
                        }));
                        stream_outcome_from_error(&track.resource_ref, &error)
                    };
                    outcomes[index] =
                        Some(resolve_failed_stream_outcome(state, initial, controls).await);
                }
            }
            Err(error) => {
                provider_batches.insert(
                    platform.to_string(),
                    json!({
                        "error_code": error.code,
                        "error": error.message,
                        "details": error.details
                    }),
                );
                for (index, track) in entries {
                    let initial = stream_outcome_from_error(&track.resource_ref, &error);
                    outcomes[index] =
                        Some(resolve_failed_stream_outcome(state, initial, controls).await);
                }
            }
        }
    }

    let outcomes = outcomes
        .into_iter()
        .enumerate()
        .map(|(index, outcome)| {
            outcome.unwrap_or_else(|| {
                let error = TuneWeaveError::new(
                    ErrorCode::InternalError,
                    "stream batch outcome was not populated",
                )
                .with_details(json!({ "position": index }));
                stream_outcome_from_error(&references[index], &error)
            })
        })
        .collect();
    StreamBatch {
        outcomes,
        extensions: Extensions::from([
            ("requested_refs".to_owned(), json!(references)),
            ("provider_batches".to_owned(), json!(provider_batches)),
            ("quality".to_owned(), json!(controls.quality)),
            ("variant".to_owned(), json!(controls.variant)),
            ("bitrate".to_owned(), json!(controls.bitrate)),
            ("fallback".to_owned(), json!(controls.fallback_enabled())),
        ]),
    }
}

fn normalize_direct_stream_outcome(
    track: Track,
    request: &StreamRequest,
    mut outcome: StreamOutcome,
) -> StreamOutcome {
    if outcome.track_ref != track.resource_ref {
        let error = TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "provider returned a stream outcome for the wrong track",
        )
        .with_platform(track.platform)
        .with_details(json!({
            "expected": track.resource_ref,
            "actual": outcome.track_ref,
            "outcome": outcome
        }));
        return stream_outcome_from_error(&track.resource_ref, &error);
    }
    let Some(stream) = outcome.stream.as_mut() else {
        if outcome.status == ResolutionStatus::Success {
            let error = TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "provider marked a stream outcome successful without a stream",
            )
            .with_platform(track.platform)
            .with_details(json!({ "track_ref": track.resource_ref }));
            return stream_outcome_from_error(&track.resource_ref, &error);
        }
        return outcome;
    };
    if outcome.status != ResolutionStatus::Success {
        let error = TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "provider returned a stream together with a failed outcome",
        )
        .with_platform(track.platform)
        .with_details(json!({ "track_ref": track.resource_ref, "status": outcome.status }));
        return stream_outcome_from_error(&track.resource_ref, &error);
    }
    stream.origin_track = Some(track.resource_ref.clone());
    stream.resolved_track = track.resource_ref.clone();
    stream.resolved_platform = track.platform;
    stream.match_score = Some(1.0);
    if stream.attempts.is_empty() {
        stream.attempts.push(ResolutionAttempt {
            platform: track.platform,
            account: request.account.clone(),
            candidate: Some(track.resource_ref.clone()),
            match_score: Some(1.0),
            status: ResolutionStatus::Success,
            error: None,
        });
    }
    outcome.error_code = None;
    outcome.error = None;
    outcome
}

async fn resolve_failed_stream_outcome(
    state: &AppState,
    initial: StreamOutcome,
    controls: &StreamControls,
) -> StreamOutcome {
    if initial.status == ResolutionStatus::Success || !controls.fallback_enabled() {
        return initial;
    }
    let initial_value = serde_json::to_value(&initial)
        .unwrap_or_else(|error| json!({ "serialization_error": error.to_string() }));
    let mut resolved = resolve_stream_reference(state, &initial.track_ref, controls).await;
    resolved
        .extensions
        .insert("initial_outcome".to_owned(), initial_value);
    resolved
}

async fn resolve_stream_reference(
    state: &AppState,
    reference: &ResourceRef,
    controls: &StreamControls,
) -> StreamOutcome {
    let provider = match state.registry.require(reference.platform()) {
        Ok(provider) => provider,
        Err(error) => return stream_outcome_from_error(reference, &error),
    };
    let provider_request = controls.provider_request(reference.platform());
    let origin = match provider
        .track(reference.id(), provider_request.account.as_deref())
        .await
    {
        Ok(track) => track,
        Err(error) => return stream_outcome_from_error(reference, &error),
    };
    match state
        .resolver
        .resolve(&origin, &controls.resolve_request(reference.platform()))
        .await
    {
        Ok(stream) => StreamOutcome {
            track_ref: reference.clone(),
            status: ResolutionStatus::Success,
            stream: Some(stream),
            error_code: None,
            error: None,
            extensions: Extensions::new(),
        },
        Err(error) => stream_outcome_from_error(reference, &error),
    }
}

fn stream_outcome_from_error(reference: &ResourceRef, error: &TuneWeaveError) -> StreamOutcome {
    StreamOutcome {
        track_ref: reference.clone(),
        status: stream_error_status(error.code),
        stream: None,
        error_code: Some(error.code),
        error: Some(error.message.clone()),
        extensions: Extensions::from([("details".to_owned(), error.details.clone())]),
    }
}

const fn stream_error_status(code: ErrorCode) -> ResolutionStatus {
    match code {
        ErrorCode::AuthenticationRequired => ResolutionStatus::AuthenticationRequired,
        ErrorCode::PermissionDenied => ResolutionStatus::PermissionDenied,
        ErrorCode::MatchRejected => ResolutionStatus::NoMatch,
        ErrorCode::CapabilityNotSupported
        | ErrorCode::PlatformUnavailable
        | ErrorCode::ResourceNotFound => ResolutionStatus::Unavailable,
        ErrorCode::InvalidRequest
        | ErrorCode::Conflict
        | ErrorCode::RateLimited
        | ErrorCode::UpstreamError
        | ErrorCode::UpstreamTimeout
        | ErrorCode::InternalError => ResolutionStatus::UpstreamError,
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PlaylistReferenceValue {
    String(String),
    Integer(u64),
}

impl PlaylistReferenceValue {
    fn into_string(self) -> String {
        match self {
            Self::String(value) => value,
            Self::Integer(value) => value.to_string(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PlaylistReferenceInput {
    One(PlaylistReferenceValue),
    Many(Vec<PlaylistReferenceValue>),
}

impl PlaylistReferenceInput {
    fn into_values(self) -> Vec<PlaylistReferenceValue> {
        match self {
            Self::One(value) => vec![value],
            Self::Many(values) => values,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum PlaylistTagsInput {
    Joined(String),
    Many(Vec<String>),
}

impl PlaylistTagsInput {
    fn into_values(self) -> Vec<String> {
        match self {
            Self::Joined(value) if value.trim().is_empty() => Vec::new(),
            Self::Joined(value) => value.split(';').map(str::to_owned).collect(),
            Self::Many(values) => values,
        }
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaylistCreateBody {
    platform: Option<String>,
    account: Option<String>,
    name: Option<String>,
    visibility: Option<Value>,
    privacy: Option<Value>,
    kind: Option<Value>,
    #[serde(rename = "type")]
    playlist_type: Option<Value>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaylistUpdateBody {
    account: Option<String>,
    name: Option<String>,
    description: Option<String>,
    desc: Option<String>,
    tags: Option<PlaylistTagsInput>,
    variant: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaylistDeleteBody {
    #[serde(alias = "playlist_refs")]
    refs: Option<PlaylistReferenceInput>,
    #[serde(alias = "id")]
    ids: Option<PlaylistReferenceInput>,
    platform: Option<String>,
    account: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaylistItemMutationBody {
    #[serde(alias = "item_refs", alias = "itemRefs", alias = "tracks")]
    refs: Option<PlaylistReferenceInput>,
    #[serde(alias = "id", alias = "trackIds")]
    ids: Option<PlaylistReferenceInput>,
    kind: Option<Value>,
    #[serde(rename = "type")]
    item_type: Option<Value>,
    account: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaylistTrackOrderBody {
    #[serde(alias = "track_refs", alias = "trackRefs", alias = "tracks")]
    refs: Option<PlaylistReferenceInput>,
    #[serde(alias = "id", alias = "trackIds")]
    ids: Option<PlaylistReferenceInput>,
    account: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaylistOrderBody {
    #[serde(alias = "playlist_refs", alias = "playlistRefs", alias = "playlists")]
    refs: Option<PlaylistReferenceInput>,
    #[serde(alias = "id")]
    ids: Option<PlaylistReferenceInput>,
    platform: Option<String>,
    account: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct PlaylistAccountParams {
    account: Option<String>,
}

async fn playlist_create(
    State(state): State<AppState>,
    payload: Result<Json<PlaylistCreateBody>, JsonRejection>,
) -> Result<Json<ApiResponse<PlaylistMutationResult>>, ApiError> {
    let body = json_body(payload)?;
    let platform = account_platform(&state, body.platform.as_deref())?;
    let account = account_alias(body.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .create_playlist(&PlaylistCreateRequest {
            name: required_trimmed("name", body.name)?,
            visibility: parse_playlist_visibility(body.visibility.as_ref(), body.privacy.as_ref())?,
            kind: parse_playlist_kind(body.kind.as_ref(), body.playlist_type.as_ref())?,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn playlist_update(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    payload: Result<Json<PlaylistUpdateBody>, JsonRejection>,
) -> Result<Json<ApiResponse<PlaylistMutationResult>>, ApiError> {
    let reference = parse_reference(reference)?;
    let body = json_body(payload)?;
    let description = match (body.description, body.desc) {
        (Some(_), Some(_)) => {
            return Err(TuneWeaveError::invalid_request(
                "description and desc cannot be provided together",
            )
            .with_details(json!({ "conflicts": ["description", "desc"] }))
            .into());
        }
        (Some(value), None) | (None, Some(value)) => Some(value),
        (None, None) => None,
    };
    let account = account_alias(body.account.as_deref())?;
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let result = provider
        .update_playlist(
            reference.id(),
            &PlaylistUpdateRequest {
                name: body.name,
                description,
                tags: body.tags.map(PlaylistTagsInput::into_values),
                variant: parse_playlist_update_variant(body.variant.as_deref())?,
                account: Some(account.clone()),
            },
        )
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn playlist_delete(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<PlaylistAccountParams>, QueryRejection>,
) -> Result<Json<ApiResponse<PlaylistDeleteResult>>, ApiError> {
    let reference = parse_reference(reference)?;
    let params = query_params(params)?;
    let account = account_alias(params.account.as_deref())?;
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let result = provider
        .delete_playlists(&PlaylistDeleteRequest {
            playlist_refs: vec![reference],
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn playlists_delete(
    State(state): State<AppState>,
    payload: Result<Json<PlaylistDeleteBody>, JsonRejection>,
) -> Result<Json<ApiResponse<PlaylistDeleteResult>>, ApiError> {
    let body = json_body(payload)?;
    let playlist_refs = parse_playlist_reference_fields(
        body.refs,
        body.ids,
        body.platform.as_deref(),
        state.default_platform,
        "playlist",
    )?;
    let platform = single_reference_platform("playlist deletion", &playlist_refs)?;
    let account = account_alias(body.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .delete_playlists(&PlaylistDeleteRequest {
            playlist_refs,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn playlist_items_add(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    payload: Result<Json<PlaylistItemMutationBody>, JsonRejection>,
) -> Result<Json<ApiResponse<PlaylistItemMutationResult>>, ApiError> {
    playlist_items_mutation(
        state,
        reference,
        payload,
        PlaylistItemMutationAction::Add,
        None,
    )
    .await
}

async fn playlist_items_remove(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    payload: Result<Json<PlaylistItemMutationBody>, JsonRejection>,
) -> Result<Json<ApiResponse<PlaylistItemMutationResult>>, ApiError> {
    playlist_items_mutation(
        state,
        reference,
        payload,
        PlaylistItemMutationAction::Remove,
        None,
    )
    .await
}

async fn playlist_tracks_add(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    payload: Result<Json<PlaylistItemMutationBody>, JsonRejection>,
) -> Result<Json<ApiResponse<PlaylistItemMutationResult>>, ApiError> {
    playlist_items_mutation(
        state,
        reference,
        payload,
        PlaylistItemMutationAction::Add,
        Some(PlaylistItemKind::Track),
    )
    .await
}

async fn playlist_tracks_remove(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    payload: Result<Json<PlaylistItemMutationBody>, JsonRejection>,
) -> Result<Json<ApiResponse<PlaylistItemMutationResult>>, ApiError> {
    playlist_items_mutation(
        state,
        reference,
        payload,
        PlaylistItemMutationAction::Remove,
        Some(PlaylistItemKind::Track),
    )
    .await
}

async fn playlist_videos_add(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    payload: Result<Json<PlaylistItemMutationBody>, JsonRejection>,
) -> Result<Json<ApiResponse<PlaylistItemMutationResult>>, ApiError> {
    playlist_items_mutation(
        state,
        reference,
        payload,
        PlaylistItemMutationAction::Add,
        Some(PlaylistItemKind::Video),
    )
    .await
}

async fn playlist_videos_remove(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    payload: Result<Json<PlaylistItemMutationBody>, JsonRejection>,
) -> Result<Json<ApiResponse<PlaylistItemMutationResult>>, ApiError> {
    playlist_items_mutation(
        state,
        reference,
        payload,
        PlaylistItemMutationAction::Remove,
        Some(PlaylistItemKind::Video),
    )
    .await
}

async fn playlist_items_mutation(
    state: AppState,
    reference: String,
    payload: Result<Json<PlaylistItemMutationBody>, JsonRejection>,
    action: PlaylistItemMutationAction,
    forced_kind: Option<PlaylistItemKind>,
) -> Result<Json<ApiResponse<PlaylistItemMutationResult>>, ApiError> {
    let reference = parse_reference(reference)?;
    let body = json_body(payload)?;
    let kind = parse_playlist_item_kind(body.kind.as_ref(), body.item_type.as_ref(), forced_kind)?;
    let item_refs = parse_playlist_reference_fields(
        body.refs,
        body.ids,
        None,
        reference.platform(),
        "playlist item",
    )?;
    let account = account_alias(body.account.as_deref())?;
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let result = provider
        .mutate_playlist_items(
            reference.id(),
            action,
            &PlaylistItemMutationRequest {
                item_refs,
                kind,
                account: Some(account.clone()),
            },
        )
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn playlist_tracks_order(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    payload: Result<Json<PlaylistTrackOrderBody>, JsonRejection>,
) -> Result<Json<ApiResponse<PlaylistTrackOrderResult>>, ApiError> {
    let reference = parse_reference(reference)?;
    let body = json_body(payload)?;
    let track_refs = parse_playlist_reference_fields(
        body.refs,
        body.ids,
        None,
        reference.platform(),
        "playlist track",
    )?;
    let account = account_alias(body.account.as_deref())?;
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let result = provider
        .reorder_playlist_tracks(
            reference.id(),
            &PlaylistTrackOrderRequest {
                track_refs,
                account: Some(account.clone()),
            },
        )
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn account_playlists_order(
    State(state): State<AppState>,
    payload: Result<Json<PlaylistOrderBody>, JsonRejection>,
) -> Result<Json<ApiResponse<PlaylistOrderResult>>, ApiError> {
    let body = json_body(payload)?;
    let playlist_refs = parse_playlist_reference_fields(
        body.refs,
        body.ids,
        body.platform.as_deref(),
        state.default_platform,
        "playlist",
    )?;
    let platform = single_reference_platform("account playlist ordering", &playlist_refs)?;
    let account = account_alias(body.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .reorder_account_playlists(&PlaylistOrderRequest {
            playlist_refs,
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
struct PlaylistCoverParams {
    account: Option<String>,
    filename: Option<String>,
    #[serde(alias = "imgSize", alias = "img_size")]
    image_size: Option<String>,
    #[serde(alias = "imgX", alias = "img_x")]
    crop_x: Option<String>,
    #[serde(alias = "imgY", alias = "img_y")]
    crop_y: Option<String>,
}

async fn playlist_cover_update(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<PlaylistCoverParams>, QueryRejection>,
    headers: HeaderMap,
    payload: Result<Bytes, BytesRejection>,
) -> Result<Json<ApiResponse<PlaylistCoverUpdateResult>>, ApiError> {
    let reference = parse_reference(reference)?;
    let params = query_params(params)?;
    let account = account_alias(params.account.as_deref())?;
    let request = parse_image_upload_request(
        headers,
        payload,
        ImageUploadOptions {
            filename: params.filename.as_deref(),
            default_filename: "playlist-cover.jpg",
            image_size: params.image_size.as_deref(),
            crop_x: params.crop_x.as_deref(),
            crop_y: params.crop_y.as_deref(),
            account: account.clone(),
            max_bytes: MAX_PLAYLIST_COVER_UPLOAD_BYTES,
        },
    )?;
    let platform = reference.platform();
    let provider = state.registry.require(platform)?;
    let result = provider
        .update_playlist_cover(reference.id(), &request)
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

fn parse_playlist_visibility(
    visibility: Option<&Value>,
    privacy: Option<&Value>,
) -> Result<PlaylistVisibility, TuneWeaveError> {
    let (name, value) = match (visibility, privacy) {
        (Some(_), Some(_)) => {
            return Err(TuneWeaveError::invalid_request(
                "visibility and privacy cannot be provided together",
            )
            .with_details(json!({ "conflicts": ["visibility", "privacy"] })));
        }
        (Some(value), None) => ("visibility", value),
        (None, Some(value)) => ("privacy", value),
        (None, None) => return Ok(PlaylistVisibility::Public),
    };
    let value = required_string_or_number(name, value)?
        .trim()
        .to_ascii_lowercase();
    match value.as_str() {
        "public" | "0" => Ok(PlaylistVisibility::Public),
        "private" | "10" => Ok(PlaylistVisibility::Private),
        _ => Err(TuneWeaveError::invalid_request(format!(
            "{name} must be public, private, 0, or 10"
        ))
        .with_details(json!({ "parameter": name, "value": value }))),
    }
}

fn parse_playlist_kind(
    kind: Option<&Value>,
    playlist_type: Option<&Value>,
) -> Result<PlaylistKind, TuneWeaveError> {
    let (name, value) = match (kind, playlist_type) {
        (Some(_), Some(_)) => {
            return Err(TuneWeaveError::invalid_request(
                "kind and type cannot be provided together",
            )
            .with_details(json!({ "conflicts": ["kind", "type"] })));
        }
        (Some(value), None) => ("kind", value),
        (None, Some(value)) => ("type", value),
        (None, None) => return Ok(PlaylistKind::Normal),
    };
    let value = required_string_or_number(name, value)?
        .trim()
        .to_ascii_lowercase();
    match value.as_str() {
        "normal" | "music" | "track" => Ok(PlaylistKind::Normal),
        "video" | "mv" => Ok(PlaylistKind::Video),
        "shared" => Ok(PlaylistKind::Shared),
        _ => Err(TuneWeaveError::invalid_request(format!(
            "{name} must be normal, video, or shared"
        ))
        .with_details(json!({ "parameter": name, "value": value }))),
    }
}

fn parse_playlist_update_variant(
    value: Option<&str>,
) -> Result<PlaylistMetadataUpdateVariant, TuneWeaveError> {
    match value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_ascii_lowercase().replace('-', "_"))
        .as_deref()
    {
        None | Some("default") | Some("auto") => Ok(PlaylistMetadataUpdateVariant::Default),
        Some("batch") => Ok(PlaylistMetadataUpdateVariant::Batch),
        Some("individual") | Some("single") => Ok(PlaylistMetadataUpdateVariant::Individual),
        Some(value) => Err(TuneWeaveError::invalid_request(format!(
            "unsupported playlist update variant: {value}"
        ))
        .with_details(json!({ "allowed": ["default", "batch", "individual"] }))),
    }
}

fn parse_playlist_item_kind(
    kind: Option<&Value>,
    item_type: Option<&Value>,
    forced_kind: Option<PlaylistItemKind>,
) -> Result<PlaylistItemKind, TuneWeaveError> {
    let parsed = match (kind, item_type) {
        (Some(_), Some(_)) => {
            return Err(TuneWeaveError::invalid_request(
                "kind and type cannot be provided together",
            )
            .with_details(json!({ "conflicts": ["kind", "type"] })));
        }
        (None, None) => None,
        (Some(value), None) | (None, Some(value)) => {
            let value = required_string_or_number("kind", value)?
                .trim()
                .to_ascii_lowercase();
            Some(match value.as_str() {
                "track" | "song" | "music" | "0" => PlaylistItemKind::Track,
                "video" | "mv" | "3" => PlaylistItemKind::Video,
                _ => {
                    return Err(TuneWeaveError::invalid_request(
                        "playlist item kind must be track or video",
                    )
                    .with_details(json!({ "value": value, "allowed": ["track", "video"] })));
                }
            })
        }
    };
    if let (Some(forced), Some(parsed)) = (forced_kind, parsed)
        && forced != parsed
    {
        return Err(TuneWeaveError::invalid_request(
            "playlist item kind conflicts with the selected endpoint",
        )
        .with_details(json!({ "endpoint_kind": forced, "requested_kind": parsed })));
    }
    Ok(forced_kind.or(parsed).unwrap_or_default())
}

fn parse_playlist_reference_fields(
    refs: Option<PlaylistReferenceInput>,
    ids: Option<PlaylistReferenceInput>,
    platform: Option<&str>,
    default_platform: Platform,
    resource: &str,
) -> Result<Vec<ResourceRef>, TuneWeaveError> {
    match (refs, ids) {
        (Some(_), Some(_)) => Err(TuneWeaveError::invalid_request(
            "refs and ids cannot be provided together",
        )
        .with_details(json!({ "conflicts": ["refs", "ids"] }))),
        (None, None) => Err(TuneWeaveError::invalid_request(
            "one of refs or ids must be provided",
        )),
        (Some(refs), None) => {
            if platform.is_some() {
                return Err(TuneWeaveError::invalid_request(
                    "platform can only be used with ids",
                ));
            }
            split_playlist_reference_values("refs", refs.into_values())?
                .into_iter()
                .map(parse_reference)
                .collect()
        }
        (None, Some(ids)) => {
            let platform = platform.map_or(Ok(default_platform), parse_platform_parameter)?;
            split_playlist_reference_values("ids", ids.into_values())?
                .into_iter()
                .map(|id| {
                    ResourceRef::new(platform, &id).map_err(|error| {
                        TuneWeaveError::invalid_request(format!("invalid {resource} id: {error}"))
                            .with_details(json!({ "id": id, "platform": platform }))
                    })
                })
                .collect()
        }
    }
}

fn split_playlist_reference_values(
    name: &str,
    values: Vec<PlaylistReferenceValue>,
) -> Result<Vec<String>, TuneWeaveError> {
    let mut parsed = Vec::new();
    for value in values {
        let value = value.into_string();
        for item in value.split(',') {
            let item = item.trim();
            if item.is_empty() {
                return Err(TuneWeaveError::invalid_request(format!(
                    "{name} must not contain empty items"
                ))
                .with_details(json!({ "parameter": name })));
            }
            parsed.push(item.to_owned());
        }
    }
    if parsed.is_empty() {
        return Err(TuneWeaveError::invalid_request(format!(
            "{name} must not be empty"
        )));
    }
    Ok(parsed)
}

fn single_reference_platform(
    operation: &str,
    references: &[ResourceRef],
) -> Result<Platform, TuneWeaveError> {
    let Some(platform) = references.first().map(ResourceRef::platform) else {
        return Err(TuneWeaveError::invalid_request(format!(
            "{operation} requires at least one reference"
        )));
    };
    if references
        .iter()
        .any(|reference| reference.platform() != platform)
    {
        return Err(
            TuneWeaveError::invalid_request(format!("{operation} cannot mix platforms"))
                .with_details(json!({ "refs": references })),
        );
    }
    Ok(platform)
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
#[serde(deny_unknown_fields)]
struct VideoDetailParams {
    account: Option<String>,
    #[serde(alias = "type")]
    kind: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct VideoStreamParams {
    account: Option<String>,
    #[serde(alias = "type")]
    kind: Option<String>,
    #[serde(alias = "res")]
    resolution: Option<String>,
}

async fn video_detail(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<VideoDetailParams>, QueryRejection>,
) -> Result<Json<ApiResponse<VideoDetail>>, ApiError> {
    let params = query_params(params)?;
    let reference = parse_reference(reference)?;
    let account = optional_trimmed(params.account);
    let kind = parse_video_resource_kind(params.kind.as_deref(), reference.id())?;
    let provider = state.registry.require(reference.platform())?;
    let mut request = VideoDetailRequest::new(kind);
    request.account.clone_from(&account);
    let detail = provider.video(reference.id(), &request).await?;
    let mut response = ApiResponse::new(detail).with_platform(reference.platform());
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

async fn video_stats(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<VideoDetailParams>, QueryRejection>,
) -> Result<Json<ApiResponse<VideoStats>>, ApiError> {
    let params = query_params(params)?;
    let reference = parse_reference(reference)?;
    let account = optional_trimmed(params.account);
    let kind = parse_video_resource_kind(params.kind.as_deref(), reference.id())?;
    let provider = state.registry.require(reference.platform())?;
    let mut request = VideoDetailRequest::new(kind);
    request.account.clone_from(&account);
    let stats = provider.video_stats(reference.id(), &request).await?;
    let mut response = ApiResponse::new(stats).with_platform(reference.platform());
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

async fn video_stream(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<VideoStreamParams>, QueryRejection>,
) -> Result<Json<ApiResponse<VideoStream>>, ApiError> {
    let params = query_params(params)?;
    let reference = parse_reference(reference)?;
    let (account, request) = video_stream_request(&params, reference.id())?;
    let provider = state.registry.require(reference.platform())?;
    let stream = provider.video_stream(reference.id(), &request).await?;
    let mut response = ApiResponse::new(stream).with_platform(reference.platform());
    if let Some(account) = account {
        response = response.with_account(account);
    }
    Ok(Json(response))
}

async fn video_stream_redirect(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<VideoStreamParams>, QueryRejection>,
) -> Result<Response, ApiError> {
    let params = query_params(params)?;
    let reference = parse_reference(reference)?;
    let (_, request) = video_stream_request(&params, reference.id())?;
    let provider = state.registry.require(reference.platform())?;
    let stream = provider.video_stream(reference.id(), &request).await?;
    if let Some(url) = stream.url.as_deref() {
        return Ok(download_redirect_response(url));
    }
    Err(
        TuneWeaveError::new(ErrorCode::ResourceNotFound, "video stream is unavailable")
            .with_platform(reference.platform())
            .with_details(json!({ "stream": stream }))
            .into(),
    )
}

fn video_stream_request(
    params: &VideoStreamParams,
    id: &str,
) -> Result<(Option<String>, VideoStreamRequest), TuneWeaveError> {
    let account = optional_trimmed(params.account.clone());
    let kind = parse_video_resource_kind(params.kind.as_deref(), id)?;
    let resolution = parse_u32_parameter(
        "resolution",
        params.resolution.as_deref(),
        VideoStreamRequest::DEFAULT_RESOLUTION,
    )?;
    if !(1..=4_320).contains(&resolution) {
        return Err(TuneWeaveError::invalid_request(
            "resolution must be between 1 and 4320",
        ));
    }
    let mut request = VideoStreamRequest::new(kind, resolution);
    request.account.clone_from(&account);
    Ok((account, request))
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
#[serde(deny_unknown_fields)]
struct UserMembershipParams {
    account: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountMembershipParams {
    platform: Option<String>,
    account: Option<String>,
}

async fn user_membership(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<UserMembershipParams>, QueryRejection>,
) -> Result<Json<ApiResponse<MembershipSummary>>, ApiError> {
    let params = query_params(params)?;
    let reference = parse_reference(reference)?;
    let platform = reference.platform();
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let membership = provider
        .user_membership(Some(reference.id()), Some(&account))
        .await?;
    Ok(Json(
        ApiResponse::new(membership)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn account_membership(
    State(state): State<AppState>,
    params: Result<Query<AccountMembershipParams>, QueryRejection>,
) -> Result<Json<ApiResponse<MembershipSummary>>, ApiError> {
    let params = query_params(params)?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let membership = provider.user_membership(None, Some(&account)).await?;
    Ok(Json(
        ApiResponse::new(membership)
            .with_platform(platform)
            .with_account(account),
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

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CountryCallingCodesQuery {
    platform: Option<String>,
    account: Option<String>,
}

async fn auth_country_calling_codes(
    State(state): State<AppState>,
    params: Result<Query<CountryCallingCodesQuery>, QueryRejection>,
) -> Result<Json<ApiResponse<Vec<CountryCallingCodeGroup>>>, ApiError> {
    let params = query_params(params)?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let groups = provider
        .country_calling_codes(&CountryCallingCodeListRequest {
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(groups)
            .with_platform(platform)
            .with_account(account),
    ))
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
#[serde(deny_unknown_fields)]
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

struct ImageUploadOptions<'a> {
    filename: Option<&'a str>,
    default_filename: &'a str,
    image_size: Option<&'a str>,
    crop_x: Option<&'a str>,
    crop_y: Option<&'a str>,
    account: String,
    max_bytes: usize,
}

fn parse_image_upload_request(
    headers: HeaderMap,
    payload: Result<Bytes, BytesRejection>,
    options: ImageUploadOptions<'_>,
) -> Result<ImageUploadRequest, ApiError> {
    let max_mebibytes = options.max_bytes / (1024 * 1024);
    let data = payload.map_err(|_| {
        TuneWeaveError::invalid_request(format!(
            "image body is invalid or exceeds {max_mebibytes} MiB"
        ))
        .with_details(json!({ "max_bytes": options.max_bytes }))
    })?;
    if data.is_empty() {
        return Err(TuneWeaveError::invalid_request("image body must not be empty").into());
    }
    if data.len() > options.max_bytes {
        return Err(TuneWeaveError::invalid_request(format!(
            "image body exceeds {max_mebibytes} MiB"
        ))
        .with_details(json!({ "max_bytes": options.max_bytes }))
        .into());
    }
    let filename = options.filename.unwrap_or(options.default_filename).trim();
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
            .trim()
            .to_owned(),
        None => "image/jpeg".to_owned(),
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
    let image_size = parse_optional_u32_parameter("image_size", options.image_size)?;
    if image_size == Some(0) {
        return Err(TuneWeaveError::invalid_request("image_size must be greater than zero").into());
    }
    Ok(ImageUploadRequest {
        filename: filename.to_owned(),
        content_type,
        data: data.to_vec(),
        image_size,
        crop_x: parse_optional_u32_parameter("crop_x", options.crop_x)?,
        crop_y: parse_optional_u32_parameter("crop_y", options.crop_y)?,
        account: Some(options.account),
    })
}

async fn account_avatar(
    State(state): State<AppState>,
    params: Result<Query<AvatarUploadParams>, QueryRejection>,
    headers: HeaderMap,
    payload: Result<Bytes, BytesRejection>,
) -> Result<Json<ApiResponse<ImageUploadResult>>, ApiError> {
    let params = query_params(params)?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let request = parse_image_upload_request(
        headers,
        payload,
        ImageUploadOptions {
            filename: params.filename.as_deref(),
            default_filename: "avatar.jpg",
            image_size: params.image_size.as_deref(),
            crop_x: params.crop_x.as_deref(),
            crop_y: params.crop_y.as_deref(),
            account: account.clone(),
            max_bytes: MAX_AVATAR_UPLOAD_BYTES,
        },
    )?;
    let provider = state.registry.require(platform)?;
    let result = provider.upload_account_avatar(&request).await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloudTracksQuery {
    platform: Option<String>,
    account: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloudTrackDetailsQuery {
    platform: Option<String>,
    account: Option<String>,
    #[serde(alias = "track_refs", alias = "trackRefs")]
    refs: Option<String>,
    #[serde(alias = "id", alias = "songIds", alias = "trackIds")]
    ids: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloudTrackReferenceBody {
    #[serde(alias = "track_refs", alias = "trackRefs")]
    refs: Option<PlaylistReferenceInput>,
    #[serde(alias = "id", alias = "songIds", alias = "trackIds")]
    ids: Option<PlaylistReferenceInput>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloudTrackDeleteBody {
    #[serde(alias = "track_refs", alias = "trackRefs")]
    refs: Option<PlaylistReferenceInput>,
    #[serde(alias = "id", alias = "songIds", alias = "trackIds")]
    ids: Option<PlaylistReferenceInput>,
    platform: Option<String>,
    account: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CloudTrackDownloadQuery {
    account: Option<String>,
}

fn parse_cloud_track_reference_fields(
    refs: Option<PlaylistReferenceInput>,
    ids: Option<PlaylistReferenceInput>,
    platform: Option<&str>,
    default_platform: Platform,
) -> Result<(Platform, Vec<ResourceRef>), TuneWeaveError> {
    match (refs, ids) {
        (Some(_), Some(_)) => Err(TuneWeaveError::invalid_request(
            "refs and ids cannot be provided together",
        )
        .with_details(json!({ "conflicts": ["refs", "ids"] }))),
        (None, None) => Err(TuneWeaveError::invalid_request(
            "one of refs or ids must be provided",
        )),
        (Some(refs), None) => {
            let references = split_playlist_reference_values("refs", refs.into_values())?
                .into_iter()
                .map(parse_reference)
                .collect::<Result<Vec<_>, _>>()?;
            let reference_platform =
                single_reference_platform("cloud track operation", &references)?;
            if let Some(platform) = platform {
                let selected = parse_platform_parameter(platform)?;
                if selected != reference_platform {
                    return Err(TuneWeaveError::invalid_request(
                        "platform conflicts with the cloud track references",
                    )
                    .with_details(json!({
                        "platform": selected,
                        "reference_platform": reference_platform
                    })));
                }
            }
            Ok((reference_platform, references))
        }
        (None, Some(ids)) => {
            let platform = platform.map_or(Ok(default_platform), parse_platform_parameter)?;
            let references = split_playlist_reference_values("ids", ids.into_values())?
                .into_iter()
                .map(|id| {
                    ResourceRef::new(platform, &id).map_err(|error| {
                        TuneWeaveError::invalid_request(format!("invalid cloud track id: {error}"))
                            .with_details(json!({ "id": id, "platform": platform }))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok((platform, references))
        }
    }
}

async fn cloud_tracks(
    State(state): State<AppState>,
    params: Result<Query<CloudTracksQuery>, QueryRejection>,
) -> Result<Json<ApiResponse<Vec<CloudTrack>>>, ApiError> {
    let params = query_params(params)?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let account = account_alias(params.account.as_deref())?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    let provider = state.registry.require(platform)?;
    let page = provider
        .cloud_tracks(&PageRequest {
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

async fn cloud_track_details_response(
    state: &AppState,
    platform: Option<&str>,
    account: Option<&str>,
    refs: Option<PlaylistReferenceInput>,
    ids: Option<PlaylistReferenceInput>,
) -> Result<Json<ApiResponse<Vec<CloudTrack>>>, ApiError> {
    let (platform, track_refs) =
        parse_cloud_track_reference_fields(refs, ids, platform, state.default_platform)?;
    let account = account_alias(account)?;
    let provider = state.registry.require(platform)?;
    let tracks = provider
        .cloud_track_details(&CloudTrackDetailRequest {
            track_refs,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(tracks)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn cloud_track_details_get(
    State(state): State<AppState>,
    params: Result<Query<CloudTrackDetailsQuery>, QueryRejection>,
) -> Result<Json<ApiResponse<Vec<CloudTrack>>>, ApiError> {
    let params = query_params(params)?;
    cloud_track_details_response(
        &state,
        params.platform.as_deref(),
        params.account.as_deref(),
        params
            .refs
            .map(PlaylistReferenceValue::String)
            .map(PlaylistReferenceInput::One),
        params
            .ids
            .map(PlaylistReferenceValue::String)
            .map(PlaylistReferenceInput::One),
    )
    .await
}

async fn cloud_track_details_post(
    State(state): State<AppState>,
    params: Result<Query<CloudAccountQuery>, QueryRejection>,
    payload: Result<Json<CloudTrackReferenceBody>, JsonRejection>,
) -> Result<Json<ApiResponse<Vec<CloudTrack>>>, ApiError> {
    let params = query_params(params)?;
    let body = json_body(payload)?;
    cloud_track_details_response(
        &state,
        params.platform.as_deref(),
        params.account.as_deref(),
        body.refs,
        body.ids,
    )
    .await
}

async fn cloud_tracks_delete(
    State(state): State<AppState>,
    payload: Result<Json<CloudTrackDeleteBody>, JsonRejection>,
) -> Result<Json<ApiResponse<CloudTrackDeleteResult>>, ApiError> {
    let body = json_body(payload)?;
    let (platform, track_refs) = parse_cloud_track_reference_fields(
        body.refs,
        body.ids,
        body.platform.as_deref(),
        state.default_platform,
    )?;
    let account = account_alias(body.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .delete_cloud_tracks(&CloudTrackDeleteRequest {
            track_refs,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn cloud_track_download(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<CloudTrackDownloadQuery>, QueryRejection>,
) -> Result<Json<ApiResponse<MediaDownload>>, ApiError> {
    let params = query_params(params)?;
    let reference = parse_reference(reference)?;
    let platform = reference.platform();
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let download = provider
        .download_cloud_track(reference.id(), Some(&account))
        .await?;
    Ok(Json(
        ApiResponse::new(download)
            .with_platform(platform)
            .with_account(account),
    ))
}

async fn cloud_track_download_redirect(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<CloudTrackDownloadQuery>, QueryRejection>,
) -> Result<Response, ApiError> {
    let params = query_params(params)?;
    let reference = parse_reference(reference)?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(reference.platform())?;
    let download = provider
        .download_cloud_track(reference.id(), Some(&account))
        .await?;
    if let Some(url) = download
        .url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
    {
        return Ok(download_redirect_response(url));
    }
    let track = provider.track(reference.id(), Some(&account)).await?;
    let stream_request = StreamRequest {
        quality: Quality::Auto,
        variant: StreamVariant::Default,
        bitrate: None,
        account: Some(account),
    };
    match provider.stream(&track, &stream_request).await {
        Ok(stream) => Ok(download_redirect_response(&stream.url)),
        Err(mut error) => {
            let stream_details = std::mem::take(&mut error.details);
            error.details = json!({
                "download": download,
                "stream": stream_details
            });
            Err(error.into())
        }
    }
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

async fn comment_reaction_enable(
    State(state): State<AppState>,
    Path((kind, reference, comment_id, reaction)): Path<(String, String, String, String)>,
    params: Result<Query<CommentAccountQuery>, QueryRejection>,
) -> Result<Json<ApiResponse<CommentReactionMutationResult>>, ApiError> {
    execute_comment_reaction_mutation(
        &state,
        kind,
        reference,
        comment_id,
        reaction,
        query_params(params)?,
        true,
    )
    .await
}

async fn comment_reaction_disable(
    State(state): State<AppState>,
    Path((kind, reference, comment_id, reaction)): Path<(String, String, String, String)>,
    params: Result<Query<CommentAccountQuery>, QueryRejection>,
) -> Result<Json<ApiResponse<CommentReactionMutationResult>>, ApiError> {
    execute_comment_reaction_mutation(
        &state,
        kind,
        reference,
        comment_id,
        reaction,
        query_params(params)?,
        false,
    )
    .await
}

async fn execute_comment_reaction_mutation(
    state: &AppState,
    kind: String,
    reference: String,
    comment_id: String,
    reaction: String,
    params: CommentAccountQuery,
    active: bool,
) -> Result<Json<ApiResponse<CommentReactionMutationResult>>, ApiError> {
    let target = parse_comment_target(&kind, reference)?;
    let platform = target.resource_ref.platform();
    let comment_id = validate_comment_id("comment_id", &comment_id)?;
    let kind = parse_comment_reaction_kind(&reaction)?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .set_comment_reaction(&CommentReactionMutationRequest {
            target,
            comment_id,
            kind,
            active,
            target_user_ref: None,
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
struct CommentAccountQuery {
    account: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommentContentBody {
    content: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CommentReportBody {
    reason: String,
}

async fn comment_report(
    State(state): State<AppState>,
    Path((kind, reference, comment_id)): Path<(String, String, String)>,
    params: Result<Query<CommentAccountQuery>, QueryRejection>,
    payload: Result<Json<CommentReportBody>, JsonRejection>,
) -> Result<Json<ApiResponse<CommentReportResult>>, ApiError> {
    let body = json_body(payload)?;
    validate_comment_report_reason(&body.reason)?;
    let target = parse_comment_target(&kind, reference)?;
    let platform = target.resource_ref.platform();
    let comment_id = validate_comment_id("comment_id", &comment_id)?;
    let params = query_params(params)?;
    let account = account_alias(params.account.as_deref())?;
    let provider = state.registry.require(platform)?;
    let result = provider
        .report_comment(&CommentReportRequest {
            target,
            comment_id,
            reason: body.reason,
            account: Some(account.clone()),
        })
        .await?;
    Ok(Json(
        ApiResponse::new(result)
            .with_platform(platform)
            .with_account(account),
    ))
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

fn validate_comment_report_reason(reason: &str) -> Result<(), TuneWeaveError> {
    if reason.trim().is_empty() {
        return Err(TuneWeaveError::invalid_request(
            "comment report reason cannot be empty",
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

fn query_params<T>(params: Result<Query<T>, QueryRejection>) -> Result<T, ApiError> {
    params
        .map(|Query(params)| params)
        .map_err(|_| TuneWeaveError::invalid_request("query parameters are invalid").into())
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

fn parse_local_track_match_duration(
    duration_ms: Option<&Value>,
    duration_seconds: Option<&Value>,
) -> Result<u64, TuneWeaveError> {
    let milliseconds = duration_ms
        .map(|value| required_json_u64("duration_ms", value))
        .transpose()?;
    let seconds = duration_seconds
        .map(|value| {
            let raw = required_string_or_number("duration_seconds", value)?;
            let seconds = raw.parse::<f64>().map_err(|_| {
                TuneWeaveError::invalid_request("duration_seconds must be a non-negative number")
                    .with_details(json!({ "parameter": "duration_seconds", "value": raw }))
            })?;
            let milliseconds = seconds * 1_000.0;
            if !seconds.is_finite() || seconds.is_sign_negative() || milliseconds > u64::MAX as f64
            {
                return Err(TuneWeaveError::invalid_request(
                    "duration_seconds must be a finite non-negative number within range",
                )
                .with_details(json!({
                    "parameter": "duration_seconds",
                    "value": raw
                })));
            }
            Ok(milliseconds.round() as u64)
        })
        .transpose()?;
    match (milliseconds, seconds) {
        (Some(milliseconds), Some(seconds)) if milliseconds != seconds => {
            Err(TuneWeaveError::invalid_request(
                "duration_ms and duration_seconds must describe the same duration",
            )
            .with_details(json!({
                "duration_ms": milliseconds,
                "duration_seconds_ms": seconds
            })))
        }
        (Some(milliseconds), _) | (_, Some(milliseconds)) => Ok(milliseconds),
        (None, None) => Ok(0),
    }
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
        "higher" => Ok(Quality::Higher),
        "high" | "exhigh" => Ok(Quality::High),
        "lossless" => Ok(Quality::Lossless),
        "hires" | "hi_res" => Ok(Quality::Hires),
        "surround" | "jyeffect" => Ok(Quality::Surround),
        "spatial" | "sky" => Ok(Quality::Spatial),
        "dolby" | "atmos" => Ok(Quality::Dolby),
        "master" | "jymaster" => Ok(Quality::Master),
        value => Err(
            TuneWeaveError::invalid_request(format!("unsupported quality: {value}")).with_details(
                json!({
                    "allowed": [
                        "auto", "low", "standard", "higher", "high", "lossless", "hires",
                        "surround", "spatial", "dolby", "master"
                    ]
                }),
            ),
        ),
    }
}

fn parse_stream_variant(value: Option<&str>) -> Result<StreamVariant, TuneWeaveError> {
    match value
        .unwrap_or("default")
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "default" | "auto" => Ok(StreamVariant::Default),
        "legacy" | "old" | "v0" | "song_url" => Ok(StreamVariant::Legacy),
        "modern" | "new" | "v1" | "song_url_v1" => Ok(StreamVariant::Modern),
        value => Err(TuneWeaveError::invalid_request(format!(
            "unsupported stream variant: {value}"
        ))
        .with_details(json!({ "allowed": ["default", "legacy", "modern"] }))),
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

fn parse_video_resource_kind(
    value: Option<&str>,
    id: &str,
) -> Result<VideoResourceKind, TuneWeaveError> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(if id.chars().all(|character| character.is_ascii_digit()) {
            VideoResourceKind::Mv
        } else {
            VideoResourceKind::Video
        });
    };
    match value.to_ascii_lowercase().replace('-', "_").as_str() {
        "mv" | "music_video" | "0" => Ok(VideoResourceKind::Mv),
        "video" | "cloud_video" | "1" => Ok(VideoResourceKind::Video),
        value => Err(TuneWeaveError::invalid_request(format!(
            "unsupported video resource type: {value}"
        ))
        .with_details(json!({ "allowed": ["mv", "video", 0, 1] }))),
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

fn parse_chart_catalog_view(value: Option<&str>) -> Result<ChartCatalogView, TuneWeaveError> {
    match value
        .unwrap_or("summary")
        .trim()
        .to_ascii_lowercase()
        .replace('-', "_")
        .as_str()
    {
        "overview" | "introduction" | "toplist" => Ok(ChartCatalogView::Overview),
        "summary" | "detail" | "classic" | "toplist_detail" => Ok(ChartCatalogView::Summary),
        "modern" | "v2" | "detail_v2" | "toplist_detail_v2" => Ok(ChartCatalogView::Modern),
        value => Err(TuneWeaveError::invalid_request(format!(
            "unsupported chart catalog view: {value}"
        ))
        .with_details(json!({ "allowed": ["overview", "summary", "modern"] }))),
    }
}

fn resolve_artist_chart_area(
    area: Option<&str>,
    kind: Option<&str>,
) -> Result<ArtistChartArea, TuneWeaveError> {
    let area = area.map(parse_artist_chart_area).transpose()?;
    let kind = kind.map(parse_artist_chart_area).transpose()?;
    match (area, kind) {
        (Some(area), Some(kind)) if area != kind => Err(TuneWeaveError::invalid_request(
            "area and type select different artist chart regions",
        )
        .with_details(json!({ "conflicts": ["area", "type"] }))),
        (Some(area), _) | (_, Some(area)) => Ok(area),
        (None, None) => Ok(ArtistChartArea::Chinese),
    }
}

fn parse_artist_chart_area(value: &str) -> Result<ArtistChartArea, TuneWeaveError> {
    match value.trim().to_ascii_lowercase().replace('-', "_").as_str() {
        "chinese" | "china" | "zh" | "1" => Ok(ArtistChartArea::Chinese),
        "western" | "west" | "eu_america" | "europe_america" | "2" => Ok(ArtistChartArea::Western),
        "korean" | "korea" | "kr" | "3" => Ok(ArtistChartArea::Korean),
        "japanese" | "japan" | "jp" | "4" => Ok(ArtistChartArea::Japanese),
        value => Err(TuneWeaveError::invalid_request(format!(
            "unsupported artist chart area: {value}"
        ))
        .with_details(json!({
            "allowed": ["chinese", "western", "korean", "japanese", 1, 2, 3, 4]
        }))),
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

fn parse_search_variant(value: Option<&str>) -> Result<SearchVariant, TuneWeaveError> {
    match value
        .unwrap_or("default")
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "default" | "auto" => Ok(SearchVariant::Default),
        "legacy" | "search" => Ok(SearchVariant::Legacy),
        "cloud" | "cloudsearch" => Ok(SearchVariant::Cloud),
        value => Err(TuneWeaveError::invalid_request(format!(
            "unsupported search variant: {value}"
        ))
        .with_details(json!({
            "allowed": ["default", "legacy", "cloud"]
        }))),
    }
}

fn parse_search_trending_detail(
    value: Option<&str>,
) -> Result<SearchTrendingDetail, TuneWeaveError> {
    match value.unwrap_or("full").trim().to_ascii_lowercase().as_str() {
        "brief" | "simple" => Ok(SearchTrendingDetail::Brief),
        "full" | "detail" | "detailed" => Ok(SearchTrendingDetail::Full),
        value => Err(TuneWeaveError::invalid_request(format!(
            "unsupported trending search detail: {value}"
        ))
        .with_details(json!({ "allowed": ["brief", "full"] }))),
    }
}

fn parse_search_suggestion_client(
    value: Option<&str>,
) -> Result<SearchSuggestionClient, TuneWeaveError> {
    match value.unwrap_or("web").trim().to_ascii_lowercase().as_str() {
        "web" => Ok(SearchSuggestionClient::Web),
        "mobile" | "keyword" => Ok(SearchSuggestionClient::Mobile),
        "pc" => Ok(SearchSuggestionClient::Pc),
        value => Err(TuneWeaveError::invalid_request(format!(
            "unsupported search suggestion client: {value}"
        ))
        .with_details(json!({ "allowed": ["web", "mobile", "pc"] }))),
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

fn parse_podcast_catalog(value: Option<&str>) -> Result<PodcastCatalog, TuneWeaveError> {
    let value = value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| TuneWeaveError::invalid_request("catalog must not be empty"))?
        .to_ascii_lowercase();
    match value.as_str() {
        "featured" | "recommend" | "recommended" => Ok(PodcastCatalog::Featured),
        "hot" => Ok(PodcastCatalog::Hot),
        "category_featured" | "category_recommend" | "category_recommended" => {
            Ok(PodcastCatalog::CategoryFeatured)
        }
        "category_hot" => Ok(PodcastCatalog::CategoryHot),
        "personalized" | "personalize" => Ok(PodcastCatalog::Personalized),
        "today_preferred" | "today" => Ok(PodcastCatalog::TodayPreferred),
        "paid" | "paygift" => Ok(PodcastCatalog::Paid),
        _ => Err(
            TuneWeaveError::invalid_request(format!("unsupported podcast catalog: {value}"))
                .with_details(json!({
                    "allowed": [
                        "featured",
                        "hot",
                        "category_featured",
                        "category_hot",
                        "personalized",
                        "today_preferred",
                        "paid"
                    ]
                })),
        ),
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
    use std::collections::{BTreeMap, BTreeSet};

    use async_trait::async_trait;
    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode, header},
    };
    use serde_json::Value;
    use tower::ServiceExt;
    use tuneweave_core::{
        ArtistBiographySection, ArtistSummary, ArtistWorkKind, AudioRecognitionMatch,
        BannerTargetKind, Chart, ChartGroup, ChartTrackPreview, CommentMutationAction,
        CommentReplyReference, CommentThreadStats, CreatorSummary, DimensionChartTrackEntry,
        MusicProvider, Page, PageMeta, PodcastCategory, ProviderQrStart, RadioCatalogOption,
        Result, SearchQuery, StreamRequest, VideoResolution,
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
                Capability::SearchDefault,
                Capability::SearchTrending,
                Capability::SearchSuggestions,
                Capability::SearchMultiMatch,
                Capability::SearchLocalTrackMatch,
                Capability::UserMembership,
                Capability::AudioRecognition,
                Capability::Banners,
                Capability::RadioTaxonomy,
                Capability::RadioStationDetail,
                Capability::RadioStationList,
                Capability::RadioStationSubscriptionWrite,
                Capability::PodcastCategories,
                Capability::PodcastList,
                Capability::PodcastDetail,
                Capability::PodcastEpisodeList,
                Capability::PodcastEpisodeDetail,
                Capability::PodcastEpisodeStream,
                Capability::PodcastEpisodeLyrics,
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
                Capability::ChartCatalog,
                Capability::ArtistCharts,
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
                Capability::PlaylistWrite,
                Capability::Lyrics,
                Capability::AudioStream,
                Capability::AudioStreamBatch,
                Capability::AudioDownload,
                Capability::VideoDetail,
                Capability::VideoStats,
                Capability::VideoStream,
                Capability::QrLogin,
                Capability::PasswordLogin,
                Capability::PhoneLogin,
                Capability::CountryCallingCodes,
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
                Capability::AccountCloudRead,
                Capability::AccountCloudDelete,
                Capability::AccountCloudDownload,
                Capability::Favorites,
                Capability::ListeningHistory,
                Capability::Recommendations,
                Capability::CommentWrite,
                Capability::CommentsRead,
                Capability::CommentReactionsRead,
                Capability::CommentReactionsWrite,
                Capability::CommentReportsWrite,
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
            extensions.insert("variant".to_owned(), json!(query.variant));
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

        async fn default_search_keyword(
            &self,
            request: &SearchDefaultKeywordRequest,
        ) -> Result<SearchDefaultKeyword> {
            Ok(SearchDefaultKeyword {
                keyword: "周旋".to_owned(),
                display_text: "🔥周旋 最近很火哦".to_owned(),
                kind: Some(SearchKind::Track),
                image_url: None,
                extensions: Extensions::from([
                    ("account".to_owned(), json!(request.account)),
                    ("provider".to_owned(), json!("mock")),
                ]),
            })
        }

        async fn trending_searches(
            &self,
            request: &SearchTrendingRequest,
        ) -> Result<SearchTrendingList> {
            Ok(SearchTrendingList {
                detail: request.detail,
                entries: vec![tuneweave_core::SearchTrendingEntry {
                    rank: 1,
                    keyword: "薛之谦".to_owned(),
                    description: (request.detail == SearchTrendingDetail::Full)
                        .then(|| "热门搜索".to_owned()),
                    score: (request.detail == SearchTrendingDetail::Full).then_some(107_509),
                    icon_type: Some(4),
                    icon_url: None,
                    target_url: None,
                    extensions: Extensions::from([("account".to_owned(), json!(request.account))]),
                }],
                extensions: Extensions::from([("provider".to_owned(), json!("mock"))]),
            })
        }

        async fn search_suggestions(
            &self,
            request: &SearchSuggestionRequest,
        ) -> Result<SearchSuggestionList> {
            let suggestion = tuneweave_core::SearchSuggestion {
                keyword: request.query.clone(),
                kind: Some(SearchKind::Track),
                display_text: (request.client == SearchSuggestionClient::Pc)
                    .then(|| "歌曲".to_owned()),
                icon_url: None,
                resource: (request.client == SearchSuggestionClient::Web)
                    .then(|| SearchItem::Track(sample_track("1357375695"))),
                extensions: Extensions::from([("account".to_owned(), json!(request.account))]),
            };
            Ok(SearchSuggestionList {
                query: request.query.clone(),
                client: request.client,
                suggestions: vec![suggestion],
                recommendations: (request.client == SearchSuggestionClient::Pc)
                    .then(|| tuneweave_core::SearchSuggestion {
                        keyword: format!("{} Beyond", request.query),
                        kind: None,
                        display_text: None,
                        icon_url: None,
                        resource: None,
                        extensions: Extensions::new(),
                    })
                    .into_iter()
                    .collect(),
                extensions: Extensions::from([("provider".to_owned(), json!("mock"))]),
            })
        }

        async fn search_multi_match(
            &self,
            request: &SearchMultiMatchRequest,
        ) -> Result<SearchMultiMatch> {
            Ok(SearchMultiMatch {
                query: request.query.clone(),
                requested_kind: request.kind,
                sections: vec![tuneweave_core::SearchMultiMatchSection {
                    section: "artist".to_owned(),
                    kind: Some(SearchKind::Artist),
                    items: vec![SearchItem::Artist(sample_artist("11127"))],
                    extensions: Extensions::from([
                        ("account".to_owned(), json!(request.account)),
                        ("order_index".to_owned(), json!(0)),
                    ]),
                }],
                extensions: Extensions::from([("provider".to_owned(), json!("mock"))]),
            })
        }

        async fn match_local_track(
            &self,
            request: &LocalTrackMatchRequest,
        ) -> Result<LocalTrackMatchResult> {
            let mut track = sample_track("65766");
            track.name.clone_from(&request.title);
            track.duration_ms = Some(request.duration_ms);
            Ok(LocalTrackMatchResult {
                md5: request.md5.clone(),
                matches: vec![track],
                extensions: Extensions::from([
                    ("album".to_owned(), json!(request.album)),
                    ("artist".to_owned(), json!(request.artist)),
                    ("account".to_owned(), json!(request.account)),
                ]),
            })
        }

        async fn user_membership(
            &self,
            id: Option<&str>,
            account: Option<&str>,
        ) -> Result<MembershipSummary> {
            Ok(MembershipSummary {
                user_ref: id
                    .map(|id| ResourceRef::new(Platform::Netease, id))
                    .transpose()
                    .expect("valid mock membership user reference"),
                level: Some(7),
                active: id.is_none().then_some(true),
                annual_count: Some(1),
                expires_at: None,
                icon_url: None,
                extensions: Extensions::from([("account".to_owned(), json!(account))]),
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

        async fn podcast(&self, id: &str, account: Option<&str>) -> Result<Podcast> {
            let mut podcast = sample_podcast(id);
            podcast
                .extensions
                .insert("account".to_owned(), json!(account));
            Ok(podcast)
        }

        async fn podcast_categories(&self, account: Option<&str>) -> Result<PodcastTaxonomy> {
            Ok(PodcastTaxonomy {
                categories: vec![PodcastCategory {
                    id: "2".to_owned(),
                    name: "音乐播客".to_owned(),
                    icon_url: Some("https://example.test/podcast-category.png".to_owned()),
                    extensions: Extensions::from([(
                        "category".to_owned(),
                        json!({"id": 2, "futureField": true}),
                    )]),
                }],
                extensions: Extensions::from([
                    ("account".to_owned(), json!(account)),
                    ("response".to_owned(), json!({"code": 200})),
                ]),
            })
        }

        async fn podcasts(&self, request: &PodcastListRequest) -> Result<Page<Podcast>> {
            let featured = request.catalog == PodcastCatalog::Featured;
            let personalized = request.catalog == PodcastCatalog::Personalized;
            let category_featured = request.catalog == PodcastCatalog::CategoryFeatured;
            let category_hot = request.catalog == PodcastCatalog::CategoryHot;
            let hot = request.catalog == PodcastCatalog::Hot;
            let paged = hot || category_hot;
            let category_valid = if category_featured || category_hot {
                request.category_id.is_some()
            } else {
                request.category_id.is_none()
            };
            if !matches!(
                request.catalog,
                PodcastCatalog::Featured
                    | PodcastCatalog::Hot
                    | PodcastCatalog::CategoryFeatured
                    | PodcastCatalog::CategoryHot
                    | PodcastCatalog::Personalized
            ) || !category_valid
                || ((featured || category_featured || personalized) && request.offset != 0)
            {
                return Err(TuneWeaveError::invalid_request(
                    "test provider only supports featured, hot, category featured, category hot, and personalized podcast catalogs",
                ));
            }
            let mut podcast = sample_podcast("336355127");
            podcast
                .extensions
                .insert("request".to_owned(), json!(request));
            let mut pagination_extensions = Extensions::from([
                ("catalog".to_owned(), json!(request.catalog)),
                ("returned_count".to_owned(), json!(1)),
                (
                    "limit_applied".to_owned(),
                    json!(!featured && !category_featured && !category_hot),
                ),
                (
                    "response".to_owned(),
                    if featured {
                        json!({"code": 200, "name": "精选电台 - 测试"})
                    } else if personalized {
                        json!({"code": 200, "data": [{"id": 336355127}]})
                    } else if category_featured {
                        json!({"code": 200, "hasMore": true})
                    } else if category_hot {
                        json!({"code": 200, "count": 1000, "hasMore": true})
                    } else {
                        json!({"code": 200, "hasMore": true})
                    },
                ),
            ]);
            if let Some(category_id) = request.category_id.as_deref() {
                pagination_extensions.insert("category_id".to_owned(), json!(category_id));
            }
            if category_featured {
                pagination_extensions.insert("continuation_supported".to_owned(), json!(false));
            }
            Ok(Page {
                items: vec![podcast],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: if featured {
                        Some(1)
                    } else if category_hot {
                        Some(1000)
                    } else {
                        None
                    },
                    next_offset: if category_hot {
                        Some(request.offset.saturating_add(request.limit))
                    } else {
                        paged.then_some(request.offset.saturating_add(1))
                    },
                    has_more: paged || category_featured,
                    extensions: pagination_extensions,
                },
            })
        }

        async fn podcast_episodes(
            &self,
            id: &str,
            request: &PodcastEpisodeListRequest,
        ) -> Result<Page<PodcastEpisode>> {
            let mut episode = sample_podcast_episode("1367665101", id, "2603965162");
            episode
                .extensions
                .insert("account".to_owned(), json!(request.account));
            Ok(Page {
                items: vec![episode],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(12),
                    next_offset: Some(request.offset.saturating_add(1)),
                    has_more: true,
                    extensions: Extensions::from([("request".to_owned(), json!(request))]),
                },
            })
        }

        async fn podcast_episode(&self, id: &str, account: Option<&str>) -> Result<PodcastEpisode> {
            let mut episode = sample_podcast_episode(id, "336355127", "2603965162");
            episode
                .extensions
                .insert("account".to_owned(), json!(account));
            if let Some(audio) = episode.audio.as_mut() {
                audio
                    .extensions
                    .insert("account".to_owned(), json!(account));
            }
            Ok(episode)
        }

        async fn podcast_episode_lyrics(
            &self,
            id: &str,
            account: Option<&str>,
        ) -> Result<PodcastEpisodeLyrics> {
            let episode = sample_podcast_episode(id, "336355127", "2603965162");
            let episode_ref = episode.resource_ref.clone();
            let audio_ref = episode
                .audio
                .as_ref()
                .expect("sample episode audio")
                .resource_ref
                .clone();
            Ok(PodcastEpisodeLyrics {
                episode_ref,
                audio_ref: Some(audio_ref.clone()),
                lyrics: Lyrics {
                    track_ref: audio_ref,
                    plain: Some("[00:00.000]节目转写".to_owned()),
                    translated: None,
                    romanized: None,
                    word_synced: Some(
                        json!({
                            "duration": 258_000,
                            "sents": [{"beg": 0, "end": 1_000, "name": "节目转写"}]
                        })
                        .to_string(),
                    ),
                    format: "netease_voice_json".to_owned(),
                    contributors: Vec::new(),
                    extensions: Extensions::from([("available".to_owned(), json!(true))]),
                },
                extensions: Extensions::from([("account".to_owned(), json!(account))]),
            })
        }

        async fn track(&self, id: &str, account: Option<&str>) -> Result<Track> {
            let mut track = sample_track(id);
            track
                .extensions
                .insert("account".to_owned(), json!(account));
            Ok(track)
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

        async fn chart_catalog(&self, request: &ChartCatalogRequest) -> Result<ChartCatalog> {
            Ok(ChartCatalog {
                platform: Platform::Netease,
                view: request.view,
                groups: vec![ChartGroup {
                    code: Some("OFFICIAL".to_owned()),
                    name: "官方榜".to_owned(),
                    display_type: Some("HORIZONTAL".to_owned()),
                    target_url: None,
                    charts: vec![Chart {
                        resource_ref: Some(
                            ResourceRef::new(Platform::Netease, "19723756")
                                .expect("valid test chart reference"),
                        ),
                        platform: Platform::Netease,
                        id: Some("19723756".to_owned()),
                        name: "飙升榜".to_owned(),
                        description: "每天更新".to_owned(),
                        cover_url: Some("https://example.test/chart.jpg".to_owned()),
                        update_frequency: Some("每天更新".to_owned()),
                        updated_at_ms: Some(1_784_170_805_374),
                        track_count: Some(100),
                        play_count: Some(42_000),
                        subscribed: Some(false),
                        playable: Some(true),
                        target_kind: Some("playlist".to_owned()),
                        target_url: None,
                        previews: vec![ChartTrackPreview {
                            rank: Some(1),
                            previous_rank: Some(5),
                            rank_change: Some(4),
                            track_ref: Some(
                                ResourceRef::new(Platform::Netease, "3404238777")
                                    .expect("valid test chart track reference"),
                            ),
                            name: "周旋".to_owned(),
                            byline: Some("王以太/艾热 AIR".to_owned()),
                            cover_url: None,
                            extensions: Extensions::new(),
                        }],
                        extensions: Extensions::new(),
                    }],
                    extensions: Extensions::new(),
                }],
                extensions: Extensions::from([("account".to_owned(), json!(request.account))]),
            })
        }

        async fn artist_chart(&self, request: &ArtistChartRequest) -> Result<ArtistChart> {
            Ok(ArtistChart {
                platform: Platform::Netease,
                area: request.area,
                updated_at_ms: Some(1_784_170_805_374),
                entries: vec![tuneweave_core::ArtistChartEntry {
                    rank: 1,
                    previous_rank: Some(5),
                    rank_change: Some(4),
                    score: Some(63_562_038),
                    artist: sample_artist("3684"),
                    extensions: Extensions::from([("account".to_owned(), json!(request.account))]),
                }],
                extensions: Extensions::new(),
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

        async fn video(&self, id: &str, request: &VideoDetailRequest) -> Result<VideoDetail> {
            let mut video = sample_video(id);
            video
                .extensions
                .insert("account".to_owned(), json!(request.account));
            Ok(VideoDetail {
                kind: request.kind,
                video,
                resolutions: vec![VideoResolution {
                    resolution: 1080,
                    width: Some(1920),
                    height: Some(1080),
                    size: Some(177_950_120),
                    format: Some("mp4".to_owned()),
                    extensions: Extensions::new(),
                }],
                extensions: Extensions::new(),
            })
        }

        async fn video_stats(&self, id: &str, request: &VideoDetailRequest) -> Result<VideoStats> {
            Ok(VideoStats {
                video_ref: ResourceRef::new(Platform::Netease, id)
                    .expect("valid test video reference"),
                kind: request.kind,
                liked: Some(false),
                like_count: Some(4_662),
                comment_count: Some(675),
                share_count: Some(1_399),
                extensions: Extensions::from([("account".to_owned(), json!(request.account))]),
            })
        }

        async fn video_stream(
            &self,
            id: &str,
            request: &VideoStreamRequest,
        ) -> Result<VideoStream> {
            let available = id != "unavailable";
            Ok(VideoStream {
                video_ref: ResourceRef::new(Platform::Netease, id)
                    .expect("valid test video reference"),
                platform: Platform::Netease,
                available,
                url: available.then(|| format!("https://example.test/video/{id}.mp4")),
                backup_urls: Vec::new(),
                headers: BTreeMap::new(),
                expires_at: None,
                format: available.then(|| "mp4".to_owned()),
                codec: None,
                width: Some(1920),
                height: Some(request.resolution),
                size: Some(177_950_120),
                duration_ms: Some(266_000),
                requested_resolution: request.resolution,
                actual_resolution: available.then_some(request.resolution),
                platform_code: Some(if available { 200 } else { 404 }),
                fee: Some(0),
                message: (!available).then(|| "video unavailable".to_owned()),
                extensions: Extensions::from([("account".to_owned(), json!(request.account))]),
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

        async fn create_playlist(
            &self,
            request: &PlaylistCreateRequest,
        ) -> Result<PlaylistMutationResult> {
            let mut playlist = sample_playlist("9001");
            playlist.name.clone_from(&request.name);
            let mut extensions = Extensions::new();
            extensions.insert("visibility".to_owned(), json!(request.visibility));
            extensions.insert("kind".to_owned(), json!(request.kind));
            extensions.insert("account".to_owned(), json!(request.account));
            Ok(PlaylistMutationResult {
                playlist_ref: playlist.resource_ref.clone(),
                action: tuneweave_core::PlaylistMutationAction::Create,
                playlist: Some(playlist),
                extensions,
            })
        }

        async fn update_playlist(
            &self,
            id: &str,
            request: &PlaylistUpdateRequest,
        ) -> Result<PlaylistMutationResult> {
            let playlist_ref =
                ResourceRef::new(Platform::Netease, id).expect("valid playlist update reference");
            let mut extensions = Extensions::new();
            extensions.insert("name".to_owned(), json!(request.name));
            extensions.insert("description".to_owned(), json!(request.description));
            extensions.insert("tags".to_owned(), json!(request.tags));
            extensions.insert("variant".to_owned(), json!(request.variant));
            extensions.insert("account".to_owned(), json!(request.account));
            Ok(PlaylistMutationResult {
                playlist_ref,
                action: tuneweave_core::PlaylistMutationAction::Update,
                playlist: None,
                extensions,
            })
        }

        async fn delete_playlists(
            &self,
            request: &PlaylistDeleteRequest,
        ) -> Result<PlaylistDeleteResult> {
            Ok(PlaylistDeleteResult {
                playlist_refs: request.playlist_refs.clone(),
                extensions: Extensions::from([("account".to_owned(), json!(request.account))]),
            })
        }

        async fn mutate_playlist_items(
            &self,
            id: &str,
            action: PlaylistItemMutationAction,
            request: &PlaylistItemMutationRequest,
        ) -> Result<PlaylistItemMutationResult> {
            Ok(PlaylistItemMutationResult {
                playlist_ref: ResourceRef::new(Platform::Netease, id)
                    .expect("valid playlist item reference"),
                item_refs: request.item_refs.clone(),
                kind: request.kind,
                action,
                snapshot_id: Some("snapshot-items".to_owned()),
                cloud_track_count: Some(request.item_refs.len() as u64),
                extensions: Extensions::from([("account".to_owned(), json!(request.account))]),
            })
        }

        async fn reorder_playlist_tracks(
            &self,
            id: &str,
            request: &PlaylistTrackOrderRequest,
        ) -> Result<PlaylistTrackOrderResult> {
            Ok(PlaylistTrackOrderResult {
                playlist_ref: ResourceRef::new(Platform::Netease, id)
                    .expect("valid playlist track order reference"),
                track_refs: request.track_refs.clone(),
                snapshot_id: Some("snapshot-order".to_owned()),
                extensions: Extensions::from([("account".to_owned(), json!(request.account))]),
            })
        }

        async fn reorder_account_playlists(
            &self,
            request: &PlaylistOrderRequest,
        ) -> Result<PlaylistOrderResult> {
            Ok(PlaylistOrderResult {
                playlist_refs: request.playlist_refs.clone(),
                extensions: Extensions::from([("account".to_owned(), json!(request.account))]),
            })
        }

        async fn update_playlist_cover(
            &self,
            id: &str,
            request: &ImageUploadRequest,
        ) -> Result<PlaylistCoverUpdateResult> {
            let mut extensions = Extensions::new();
            extensions.insert("filename".to_owned(), json!(request.filename));
            extensions.insert("content_type".to_owned(), json!(request.content_type));
            extensions.insert("data_len".to_owned(), json!(request.data.len()));
            extensions.insert("image_size".to_owned(), json!(request.image_size));
            extensions.insert("crop_x".to_owned(), json!(request.crop_x));
            extensions.insert("crop_y".to_owned(), json!(request.crop_y));
            extensions.insert("account".to_owned(), json!(request.account));
            Ok(PlaylistCoverUpdateResult {
                playlist_ref: ResourceRef::new(Platform::Netease, id)
                    .expect("valid playlist cover reference"),
                image: ImageUploadResult {
                    url: Some("https://example.test/playlist-cover.jpg".to_owned()),
                    image_id: Some("109951168000000001".to_owned()),
                    extensions,
                },
                extensions: Extensions::new(),
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
                headers: BTreeMap::from([
                    (
                        "x-test-stream-variant".to_owned(),
                        match request.variant {
                            StreamVariant::Default => "default",
                            StreamVariant::Legacy => "legacy",
                            StreamVariant::Modern => "modern",
                        }
                        .to_owned(),
                    ),
                    (
                        "x-test-origin-account".to_owned(),
                        track
                            .extensions
                            .get("account")
                            .and_then(Value::as_str)
                            .unwrap_or("none")
                            .to_owned(),
                    ),
                ]),
                expires_at: None,
                format: Some("mp3".to_owned()),
                codec: Some("mp3".to_owned()),
                bitrate: request.bitrate.or(Some(320_000)),
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

        async fn download(&self, track: &Track, request: &StreamRequest) -> Result<MediaDownload> {
            let available = track.id != "fallback";
            Ok(MediaDownload {
                track_ref: track.resource_ref.clone(),
                platform: Platform::Netease,
                available,
                url: available.then(|| format!("https://example.test/download/{}.flac", track.id)),
                headers: BTreeMap::new(),
                expires_at: None,
                format: available.then(|| "flac".to_owned()),
                codec: available.then(|| "flac".to_owned()),
                bitrate: request.bitrate.or(Some(999_000)),
                size: available.then_some(2048),
                duration_ms: track.duration_ms,
                requested_quality: request.quality,
                actual_quality: if available {
                    Quality::Lossless
                } else {
                    Quality::Auto
                },
                platform_code: Some(if available { 200 } else { -110 }),
                fee: Some(0),
                message: (!available).then(|| "download unavailable".to_owned()),
                extensions: Extensions::from([
                    ("variant".to_owned(), json!(request.variant)),
                    ("account".to_owned(), json!(request.account)),
                    (
                        "origin_account".to_owned(),
                        track
                            .extensions
                            .get("account")
                            .cloned()
                            .unwrap_or(Value::Null),
                    ),
                ]),
            })
        }

        async fn start_qr_login(&self, _login_type: Option<&str>) -> Result<ProviderQrStart> {
            Ok(ProviderQrStart {
                provider_transaction_id: "provider-qr-key".to_owned(),
                url: "https://example.test/qr".to_owned(),
                image_data_url: Some(
                    "data:image/svg+xml;base64,PHN2ZyB4bWxucz0iaHR0cDovL3d3dy53My5vcmcvMjAwMC9zdmciLz4="
                        .to_owned(),
                ),
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

        async fn country_calling_codes(
            &self,
            request: &CountryCallingCodeListRequest,
        ) -> Result<Vec<CountryCallingCodeGroup>> {
            Ok(vec![CountryCallingCodeGroup {
                label: "常用".to_owned(),
                entries: vec![tuneweave_core::CountryCallingCode {
                    calling_code: "86".to_owned(),
                    region_code: "CN".to_owned(),
                    name: "中国".to_owned(),
                    english_name: "China".to_owned(),
                    extensions: Extensions::from([("account".to_owned(), json!(request.account))]),
                }],
                extensions: Extensions::from([("provider".to_owned(), json!("mock"))]),
            }])
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

        async fn cloud_tracks(&self, request: &PageRequest) -> Result<Page<CloudTrack>> {
            let mut track = sample_cloud_track("9001");
            track
                .extensions
                .insert("account".to_owned(), json!(request.account));
            Ok(Page {
                items: vec![track],
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(12),
                    next_offset: Some(request.offset.saturating_add(1)),
                    has_more: true,
                    extensions: Extensions::from([
                        ("storage_size".to_owned(), json!(50_412_168_u64)),
                        ("storage_max_size".to_owned(), json!(1_073_741_824_u64)),
                    ]),
                },
            })
        }

        async fn cloud_track_details(
            &self,
            request: &CloudTrackDetailRequest,
        ) -> Result<Vec<CloudTrack>> {
            Ok(request
                .track_refs
                .iter()
                .map(|track_ref| {
                    let mut track = sample_cloud_track(track_ref.id());
                    track
                        .extensions
                        .insert("account".to_owned(), json!(request.account));
                    track
                })
                .collect())
        }

        async fn delete_cloud_tracks(
            &self,
            request: &CloudTrackDeleteRequest,
        ) -> Result<CloudTrackDeleteResult> {
            Ok(CloudTrackDeleteResult {
                track_refs: request.track_refs.clone(),
                deleted: true,
                extensions: Extensions::from([("account".to_owned(), json!(request.account))]),
            })
        }

        async fn download_cloud_track(
            &self,
            id: &str,
            account: Option<&str>,
        ) -> Result<MediaDownload> {
            let available = id != "unavailable";
            Ok(MediaDownload {
                track_ref: ResourceRef::new(Platform::Netease, id)
                    .expect("valid cloud download reference"),
                platform: Platform::Netease,
                available,
                url: available.then(|| format!("https://example.test/cloud/{id}.flac")),
                headers: BTreeMap::new(),
                expires_at: None,
                format: Some("flac".to_owned()),
                codec: Some("flac".to_owned()),
                bitrate: Some(999_000),
                size: Some(50_412_168),
                duration_ms: Some(258_000),
                requested_quality: Quality::Auto,
                actual_quality: Quality::Lossless,
                platform_code: Some(200),
                fee: Some(0),
                message: (!available).then(|| "cloud download unavailable".to_owned()),
                extensions: Extensions::from([("account".to_owned(), json!(account))]),
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

        async fn set_comment_reaction(
            &self,
            request: &CommentReactionMutationRequest,
        ) -> Result<CommentReactionMutationResult> {
            Ok(CommentReactionMutationResult {
                target: request.target.clone(),
                comment_id: request.comment_id.clone(),
                kind: request.kind,
                active: request.active,
                target_user_ref: request.target_user_ref.clone(),
                extensions: Extensions::from([
                    ("account".to_owned(), json!(request.account)),
                    ("provider".to_owned(), json!("mock")),
                ]),
            })
        }

        async fn report_comment(
            &self,
            request: &CommentReportRequest,
        ) -> Result<CommentReportResult> {
            Ok(CommentReportResult {
                target: request.target.clone(),
                comment_id: request.comment_id.clone(),
                reason: request.reason.clone(),
                submitted: true,
                extensions: Extensions::from([
                    ("account".to_owned(), json!(request.account)),
                    ("provider".to_owned(), json!("mock")),
                ]),
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

    fn sample_cloud_track(id: &str) -> CloudTrack {
        let cloud_track_ref =
            ResourceRef::new(Platform::Netease, id).expect("valid cloud track reference");
        CloudTrack {
            cloud_track_ref: cloud_track_ref.clone(),
            track: sample_track(id),
            filename: Some("反方向的钟.flac".to_owned()),
            file_size: Some(50_412_168),
            file_type: Some("flac".to_owned()),
            bitrate: Some(999_000),
            md5: Some("d02b8ab79d91c01167ba31e349fe5275".to_owned()),
            added_at: Some("2024-01-01T00:00:00Z".to_owned()),
            matched_track_ref: Some(
                ResourceRef::new(Platform::Netease, "185809")
                    .expect("valid matched track reference"),
            ),
            extensions: Extensions::from([("provider".to_owned(), json!("mock"))]),
        }
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

    fn sample_podcast(id: &str) -> Podcast {
        let mut podcast = Podcast::new(
            ResourceRef::new(Platform::Netease, id).expect("valid test podcast reference"),
            "代码时间",
        );
        podcast.description = "面向开发者的播客".to_owned();
        podcast.cover_url = Some("https://example.test/podcast.jpg".to_owned());
        podcast.creator = Some(CreatorSummary {
            resource_ref: Some(
                ResourceRef::new(Platform::Netease, "32953014")
                    .expect("valid test podcast creator reference"),
            ),
            name: "主播".to_owned(),
            avatar_url: Some("https://example.test/avatar.jpg".to_owned()),
        });
        podcast.category = Some("科技".to_owned());
        podcast.secondary_category = Some("互联网".to_owned());
        podcast.episode_count = Some(120);
        podcast.subscriber_count = Some(4_567);
        podcast.play_count = Some(98_765);
        podcast.subscribed = Some(true);
        podcast.paid = Some(false);
        podcast.purchased = Some(false);
        podcast
    }

    fn sample_podcast_episode(id: &str, podcast_id: &str, audio_id: &str) -> PodcastEpisode {
        let mut episode = PodcastEpisode::new(
            ResourceRef::new(Platform::Netease, id).expect("valid test podcast episode reference"),
            "一期节目",
        );
        episode.podcast_ref = Some(
            ResourceRef::new(Platform::Netease, podcast_id)
                .expect("valid test episode podcast reference"),
        );
        episode.description = "节目介绍".to_owned();
        episode.cover_url = Some("https://example.test/episode.jpg".to_owned());
        episode.creator = Some(CreatorSummary {
            resource_ref: Some(
                ResourceRef::new(Platform::Netease, "32953014")
                    .expect("valid test episode creator reference"),
            ),
            name: "主播".to_owned(),
            avatar_url: Some("https://example.test/avatar.jpg".to_owned()),
        });
        let mut audio = sample_track(audio_id);
        audio.name = "一期节目音频".to_owned();
        episode.audio = Some(audio);
        episode.duration_ms = Some(258_000);
        episode.serial_number = Some(42);
        episode.listener_count = Some(1_234);
        episode.has_lyrics = Some(true);
        episode.paid = Some(false);
        episode.purchased = Some(false);
        episode
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
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["variant"],
            "default"
        );
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

    #[tokio::test]
    async fn search_exposes_legacy_and_cloud_variants_on_the_same_endpoint() {
        for (input, expected) in [
            ("legacy", "legacy"),
            ("search", "legacy"),
            ("cloud", "cloud"),
            ("cloudsearch", "cloud"),
            ("auto", "default"),
        ] {
            let path = format!("/v1/search?q=clock&variant={input}");
            let (status, response) = json_response_from(test_app_with_provider(), &path).await;
            assert_eq!(status, StatusCode::OK, "{input}");
            assert_eq!(
                response["meta"]["pagination"]["extensions"]["variant"], expected,
                "{input}"
            );
        }

        let (status, response) = json_response_from(
            test_app_with_provider(),
            "/v1/search?q=clock&backend=legacy",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            response["meta"]["pagination"]["extensions"]["variant"],
            "legacy"
        );

        let (status, response) = json_response_from(
            test_app_with_provider(),
            "/v1/search?q=clock&variant=unknown",
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(response["error"]["code"], "invalid_request");
    }

    #[tokio::test]
    async fn default_search_keyword_uses_selected_platform_account_and_stable_fields() {
        let (status, response) = json_response_from(
            test_app_with_provider(),
            "/v1/search/default?platform=netease&account=search-user",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["data"]["keyword"], "周旋");
        assert_eq!(response["data"]["display_text"], "🔥周旋 最近很火哦");
        assert_eq!(response["data"]["kind"], "track");
        assert!(response["data"]["image_url"].is_null());
        assert_eq!(response["data"]["extensions"]["account"], "search-user");
        assert_eq!(response["meta"]["platform"], "netease");
        assert_eq!(response["meta"]["account"], "search-user");

        let (status, defaulted) =
            json_response_from(test_app_with_provider(), "/v1/search/default").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(defaulted["meta"]["platform"], "netease");
        assert_eq!(defaulted["meta"]["account"], "default");
    }

    #[tokio::test]
    async fn default_search_keyword_rejects_unknown_platforms_and_query_fields() {
        for path in [
            "/v1/search/default?platform=unknown",
            "/v1/search/default?unknown=true",
        ] {
            let (status, response) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(response["error"]["code"], "invalid_request", "{path}");
        }
    }

    #[tokio::test]
    async fn trending_searches_share_one_endpoint_for_brief_and_full_catalogs() {
        for (input, expected, has_rich_fields) in [
            ("brief", "brief", false),
            ("simple", "brief", false),
            ("full", "full", true),
            ("detail", "full", true),
            ("detailed", "full", true),
        ] {
            let path =
                format!("/v1/search/trending?detail={input}&platform=netease&account=search-user");
            let (status, response) = json_response_from(test_app_with_provider(), &path).await;
            assert_eq!(status, StatusCode::OK, "{input}");
            assert_eq!(response["data"]["detail"], expected, "{input}");
            assert_eq!(response["data"]["entries"][0]["rank"], 1, "{input}");
            assert_eq!(
                response["data"]["entries"][0]["keyword"], "薛之谦",
                "{input}"
            );
            assert_eq!(
                response["data"]["entries"][0]["score"].is_number(),
                has_rich_fields,
                "{input}"
            );
            assert_eq!(
                response["data"]["entries"][0]["description"].is_string(),
                has_rich_fields,
                "{input}"
            );
            assert_eq!(response["meta"]["platform"], "netease");
            assert_eq!(response["meta"]["account"], "search-user");
        }

        let (status, defaulted) =
            json_response_from(test_app_with_provider(), "/v1/search/trending").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(defaulted["data"]["detail"], "full");
        assert_eq!(defaulted["meta"]["account"], "default");

        let (status, aliased) =
            json_response_from(test_app_with_provider(), "/v1/search/trending?mode=brief").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(aliased["data"]["detail"], "brief");
    }

    #[tokio::test]
    async fn trending_searches_reject_unknown_detail_platform_and_query_fields() {
        for path in [
            "/v1/search/trending?detail=unknown",
            "/v1/search/trending?platform=unknown",
            "/v1/search/trending?unknown=true",
        ] {
            let (status, response) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(response["error"]["code"], "invalid_request", "{path}");
        }
    }

    #[tokio::test]
    async fn search_suggestions_share_web_mobile_and_pc_clients_on_one_endpoint() {
        for (path, client, has_resource, recommendation_count) in [
            (
                "/v1/search/suggestions?q=%E6%B5%B7%E9%98%94%E5%A4%A9%E7%A9%BA&client=web&account=search-user",
                "web",
                true,
                0,
            ),
            (
                "/v1/search/suggestions?keywords=%E6%B5%B7%E9%98%94%E5%A4%A9%E7%A9%BA&type=mobile&account=search-user",
                "mobile",
                false,
                0,
            ),
            (
                "/v1/search/suggestions?keyword=%E6%B5%B7%E9%98%94%E5%A4%A9%E7%A9%BA&client=pc&account=search-user",
                "pc",
                false,
                1,
            ),
        ] {
            let (status, response) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::OK, "{client}");
            assert_eq!(response["data"]["query"], "海阔天空", "{client}");
            assert_eq!(response["data"]["client"], client, "{client}");
            assert_eq!(
                response["data"]["suggestions"][0]["keyword"], "海阔天空",
                "{client}"
            );
            assert_eq!(
                response["data"]["suggestions"][0]["resource"].is_object(),
                has_resource,
                "{client}"
            );
            assert_eq!(
                response["data"]["recommendations"].as_array().map(Vec::len),
                Some(recommendation_count),
                "{client}"
            );
            assert_eq!(response["meta"]["account"], "search-user");
        }

        let (status, defaulted) = json_response_from(
            test_app_with_provider(),
            "/v1/search/suggestions?q=%E6%B5%B7%E9%98%94%E5%A4%A9%E7%A9%BA",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(defaulted["data"]["client"], "web");
        assert_eq!(defaulted["meta"]["platform"], "netease");
        assert_eq!(defaulted["meta"]["account"], "default");
    }

    #[tokio::test]
    async fn search_suggestions_reject_missing_query_unknown_client_platform_and_fields() {
        for path in [
            "/v1/search/suggestions",
            "/v1/search/suggestions?q=%20%20",
            "/v1/search/suggestions?q=test&client=unknown",
            "/v1/search/suggestions?q=test&platform=unknown",
            "/v1/search/suggestions?q=test&unknown=true",
        ] {
            let (status, response) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(response["error"]["code"], "invalid_request", "{path}");
        }
    }

    #[tokio::test]
    async fn multi_match_search_accepts_unified_and_reference_query_names() {
        for (path, kind) in [
            (
                "/v1/search/multimatch?q=%E6%B5%B7%E9%98%94%E5%A4%A9%E7%A9%BA&kind=track&account=search-user",
                "track",
            ),
            (
                "/v1/search/multimatch?keywords=%E6%B5%B7%E9%98%94%E5%A4%A9%E7%A9%BA&type=100&platform=netease&account=search-user",
                "artist",
            ),
            (
                "/v1/search/multimatch?keyword=%E6%B5%B7%E9%98%94%E5%A4%A9%E7%A9%BA&type=1014&account=search-user",
                "video",
            ),
        ] {
            let (status, response) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert_eq!(response["data"]["query"], "海阔天空", "{path}");
            assert_eq!(response["data"]["requested_kind"], kind, "{path}");
            assert_eq!(response["data"]["sections"][0]["section"], "artist");
            assert_eq!(response["data"]["sections"][0]["kind"], "artist");
            assert_eq!(
                response["data"]["sections"][0]["items"][0]["type"],
                "artist"
            );
            assert_eq!(
                response["data"]["sections"][0]["items"][0]["data"]["ref"],
                "netease:11127"
            );
            assert_eq!(
                response["data"]["sections"][0]["extensions"]["account"],
                "search-user"
            );
            assert_eq!(response["meta"]["platform"], "netease");
            assert_eq!(response["meta"]["account"], "search-user");
        }

        let (status, defaulted) = json_response_from(
            test_app_with_provider(),
            "/v1/search/multimatch?q=%E6%B5%B7%E9%98%94%E5%A4%A9%E7%A9%BA",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(defaulted["data"]["requested_kind"], "track");
        assert_eq!(defaulted["meta"]["account"], "default");
    }

    #[tokio::test]
    async fn multi_match_search_rejects_missing_query_unknown_kind_platform_and_fields() {
        for path in [
            "/v1/search/multimatch",
            "/v1/search/multimatch?q=%20%20",
            "/v1/search/multimatch?q=test&kind=unknown",
            "/v1/search/multimatch?q=test&platform=unknown",
            "/v1/search/multimatch?q=test&unknown=true",
        ] {
            let (status, response) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(response["error"]["code"], "invalid_request", "{path}");
        }
    }

    #[tokio::test]
    async fn local_track_match_supports_reference_get_and_unified_post_inputs() {
        let md5 = "bd708d006912a09d827f02e754cf8e56";
        let reference_path = format!(
            "/v1/search/match?title=%E5%AF%8C%E5%A3%AB%E5%B1%B1%E4%B8%8B&album=&artist=%E9%99%88%E5%A5%95%E8%BF%85&duration=259.21&md5={md5}&account=local-user"
        );
        let (status, response) =
            json_response_from(test_app_with_provider(), &reference_path).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["data"]["md5"], md5);
        assert_eq!(response["data"]["matches"][0]["ref"], "netease:65766");
        assert_eq!(response["data"]["matches"][0]["name"], "富士山下");
        assert_eq!(response["data"]["matches"][0]["duration_ms"], 259_210);
        assert_eq!(response["data"]["extensions"]["album"], "");
        assert_eq!(response["data"]["extensions"]["artist"], "陈奕迅");
        assert_eq!(response["meta"]["platform"], "netease");
        assert_eq!(response["meta"]["account"], "local-user");

        for body in [
            json!({
                "title": "富士山下",
                "album": "",
                "artist": "陈奕迅",
                "duration_ms": 259210,
                "md5": md5,
                "platform": "netease",
                "account": "local-user"
            }),
            json!({
                "title": "富士山下",
                "artist": "陈奕迅",
                "duration": "259.21",
                "md5": md5,
                "account": "local-user"
            }),
            json!({
                "title": "富士山下",
                "artist": "陈奕迅",
                "duration_ms": 259210,
                "duration_seconds": 259.21,
                "md5": md5,
                "account": "local-user"
            }),
        ] {
            let (status, response) = json_request_from(
                test_app_with_provider(),
                Method::POST,
                "/v1/search/match",
                Some(body),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(response["data"]["matches"][0]["duration_ms"], 259_210);
            assert_eq!(response["meta"]["account"], "local-user");
        }

        let defaulted = format!("/v1/search/match?md5={md5}");
        let (status, response) = json_response_from(test_app_with_provider(), &defaulted).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["data"]["matches"][0]["name"], "");
        assert_eq!(response["data"]["matches"][0]["duration_ms"], 0);
        assert_eq!(response["meta"]["account"], "default");
    }

    #[tokio::test]
    async fn local_track_match_rejects_missing_md5_bad_durations_platform_and_fields() {
        for path in [
            "/v1/search/match",
            "/v1/search/match?md5=%20%20",
            "/v1/search/match?md5=00000000000000000000000000000000&duration=-1",
            "/v1/search/match?md5=00000000000000000000000000000000&duration=NaN",
            "/v1/search/match?md5=00000000000000000000000000000000&duration_ms=1000&duration=2",
            "/v1/search/match?md5=00000000000000000000000000000000&platform=unknown",
            "/v1/search/match?md5=00000000000000000000000000000000&unknown=true",
        ] {
            let (status, response) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(response["error"]["code"], "invalid_request", "{path}");
        }

        for body in [
            json!({"title": "missing checksum"}),
            json!({"md5": "00000000000000000000000000000000", "duration_ms": -1}),
            json!({
                "md5": "00000000000000000000000000000000",
                "duration_ms": 1000,
                "duration_seconds": 2
            }),
            json!({"md5": "00000000000000000000000000000000", "unknown": true}),
        ] {
            let (status, response) = json_request_from(
                test_app_with_provider(),
                Method::POST,
                "/v1/search/match",
                Some(body),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert_eq!(response["error"]["code"], "invalid_request");
        }
    }

    #[test]
    fn local_track_duration_parser_accepts_milliseconds_seconds_and_reference_defaults() {
        assert_eq!(
            parse_local_track_match_duration(Some(&json!(259_210)), None).expect("milliseconds"),
            259_210
        );
        assert_eq!(
            parse_local_track_match_duration(None, Some(&json!("259.21"))).expect("seconds"),
            259_210
        );
        assert_eq!(
            parse_local_track_match_duration(Some(&json!(259_210)), Some(&json!(259.21)))
                .expect("matching units"),
            259_210
        );
        assert_eq!(
            parse_local_track_match_duration(None, None).expect("reference default"),
            0
        );
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
    fn search_variant_parser_accepts_unified_and_reference_names() {
        for (value, expected) in [
            ("default", SearchVariant::Default),
            ("auto", SearchVariant::Default),
            ("legacy", SearchVariant::Legacy),
            ("search", SearchVariant::Legacy),
            ("cloud", SearchVariant::Cloud),
            ("cloudsearch", SearchVariant::Cloud),
        ] {
            assert_eq!(parse_search_variant(Some(value)).expect(value), expected);
        }
        assert_eq!(
            parse_search_variant(Some("unknown"))
                .expect_err("unsupported search variant")
                .code,
            tuneweave_core::ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn trending_search_detail_parser_accepts_unified_and_reference_names() {
        for (value, expected) in [
            ("brief", SearchTrendingDetail::Brief),
            ("simple", SearchTrendingDetail::Brief),
            ("full", SearchTrendingDetail::Full),
            ("detail", SearchTrendingDetail::Full),
            ("detailed", SearchTrendingDetail::Full),
        ] {
            assert_eq!(
                parse_search_trending_detail(Some(value)).expect(value),
                expected
            );
        }
        assert_eq!(
            parse_search_trending_detail(Some("unknown"))
                .expect_err("unsupported trending detail")
                .code,
            tuneweave_core::ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn search_suggestion_client_parser_accepts_unified_and_reference_names() {
        for (value, expected) in [
            ("web", SearchSuggestionClient::Web),
            ("mobile", SearchSuggestionClient::Mobile),
            ("keyword", SearchSuggestionClient::Mobile),
            ("pc", SearchSuggestionClient::Pc),
        ] {
            assert_eq!(
                parse_search_suggestion_client(Some(value)).expect(value),
                expected
            );
        }
        assert_eq!(
            parse_search_suggestion_client(Some("unknown"))
                .expect_err("unsupported suggestion client")
                .code,
            tuneweave_core::ErrorCode::InvalidRequest
        );
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
    async fn comment_reaction_put_and_delete_expose_like_and_unlike_actions() {
        let (status, liked) = json_request_from(
            test_app_with_provider(),
            Method::PUT,
            "/v1/resources/track/netease:29178366/comments/12840183/reactions/like?account=personal",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(liked["data"]["target"]["ref"], "netease:29178366");
        assert_eq!(liked["data"]["target"]["kind"], "track");
        assert_eq!(liked["data"]["comment_id"], "12840183");
        assert_eq!(liked["data"]["kind"], "like");
        assert_eq!(liked["data"]["active"], true);
        assert!(liked["data"]["target_user_ref"].is_null());
        assert_eq!(liked["data"]["extensions"]["account"], "personal");
        assert_eq!(liked["meta"]["platform"], "netease");
        assert_eq!(liked["meta"]["account"], "personal");

        let (status, unliked) = json_request_from(
            test_app_with_provider(),
            Method::DELETE,
            "/v1/resources/event/netease:A_EV_2_6559519868_32953014/comments/1419532712/reactions/like",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            unliked["data"]["target"]["ref"],
            "netease:A_EV_2_6559519868_32953014"
        );
        assert_eq!(unliked["data"]["target"]["kind"], "event");
        assert_eq!(unliked["data"]["active"], false);
        assert_eq!(unliked["meta"]["account"], "default");
    }

    #[tokio::test]
    async fn comment_reaction_mutations_reject_unknown_reactions_and_query_fields() {
        for path in [
            "/v1/resources/track/netease:29178366/comments/12840183/reactions/clap",
            "/v1/resources/article/netease:29178366/comments/12840183/reactions/like",
            "/v1/resources/track/netease:29178366/comments/12840183/reactions/like?unknown=true",
        ] {
            let (status, response) =
                json_request_from(test_app_with_provider(), Method::PUT, path, None).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(response["error"]["code"], "invalid_request", "{path}");
        }
    }

    #[tokio::test]
    async fn comment_report_exposes_reason_account_and_submission_state() {
        let (status, response) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/resources/track/netease:2058263032/comments/123456789/reports?account=personal",
            Some(json!({"reason": "  人身攻击  "})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["data"]["target"]["ref"], "netease:2058263032");
        assert_eq!(response["data"]["target"]["kind"], "track");
        assert_eq!(response["data"]["comment_id"], "123456789");
        assert_eq!(response["data"]["reason"], "  人身攻击  ");
        assert_eq!(response["data"]["submitted"], true);
        assert_eq!(response["data"]["extensions"]["account"], "personal");
        assert_eq!(response["meta"]["platform"], "netease");
        assert_eq!(response["meta"]["account"], "personal");
    }

    #[tokio::test]
    async fn comment_report_rejects_invalid_reason_body_target_and_query() {
        for (path, body) in [
            (
                "/v1/resources/track/netease:2058263032/comments/123456789/reports",
                json!({"reason": " \t"}),
            ),
            (
                "/v1/resources/track/netease:2058263032/comments/123456789/reports",
                json!({"reason": "人身攻击", "unknown": true}),
            ),
            (
                "/v1/resources/article/netease:2058263032/comments/123456789/reports",
                json!({"reason": "人身攻击"}),
            ),
            (
                "/v1/resources/track/netease:2058263032/comments/123456789/reports?unknown=true",
                json!({"reason": "人身攻击"}),
            ),
        ] {
            let (status, response) =
                json_request_from(test_app_with_provider(), Method::POST, path, Some(body)).await;
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
    async fn podcast_categories_use_selected_platform_account_and_preserve_extensions() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/podcasts/categories?platform=netease&account=podcast-user",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["categories"][0]["id"], "2");
        assert_eq!(json["data"]["categories"][0]["name"], "音乐播客");
        assert_eq!(
            json["data"]["categories"][0]["icon_url"],
            "https://example.test/podcast-category.png"
        );
        assert_eq!(
            json["data"]["categories"][0]["extensions"]["category"]["futureField"],
            true
        );
        assert_eq!(json["data"]["extensions"]["response"]["code"], 200);
        assert_eq!(json["data"]["extensions"]["account"], "podcast-user");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "podcast-user");
    }

    #[tokio::test]
    async fn podcast_categories_reject_unknown_platforms_and_query_fields() {
        for path in [
            "/v1/podcasts/categories?platform=unknown",
            "/v1/podcasts/categories?unknown=true",
        ] {
            let (status, json) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "path: {path}");
            assert_eq!(json["error"]["code"], "invalid_request", "path: {path}");
        }
    }

    #[tokio::test]
    async fn hot_podcast_catalog_uses_platform_account_and_real_pagination() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/podcasts?catalog=hot&limit=2&offset=4&platform=netease&account=podcast-user",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:336355127");
        assert_eq!(json["data"][0]["extensions"]["request"]["catalog"], "hot");
        assert_eq!(
            json["data"][0]["extensions"]["request"]["account"],
            "podcast-user"
        );
        assert_eq!(json["meta"]["pagination"]["limit"], 2);
        assert_eq!(json["meta"]["pagination"]["offset"], 4);
        assert_eq!(json["meta"]["pagination"]["total"], Value::Null);
        assert_eq!(json["meta"]["pagination"]["next_offset"], 5);
        assert_eq!(json["meta"]["pagination"]["has_more"], true);
        assert_eq!(json["meta"]["pagination"]["extensions"]["catalog"], "hot");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "podcast-user");
    }

    #[tokio::test]
    async fn featured_podcast_catalog_is_a_complete_fixed_snapshot() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/podcasts?catalog=recommend&limit=2&platform=netease&account=podcast-user",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:336355127");
        assert_eq!(
            json["data"][0]["extensions"]["request"]["catalog"],
            "featured"
        );
        assert_eq!(json["meta"]["pagination"]["limit"], 2);
        assert_eq!(json["meta"]["pagination"]["offset"], 0);
        assert_eq!(json["meta"]["pagination"]["total"], 1);
        assert_eq!(json["meta"]["pagination"]["next_offset"], Value::Null);
        assert_eq!(json["meta"]["pagination"]["has_more"], false);
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["catalog"],
            "featured"
        );
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["limit_applied"],
            false
        );
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["response"]["name"],
            "精选电台 - 测试"
        );
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "podcast-user");
    }

    #[tokio::test]
    async fn personalized_podcast_catalog_applies_limit_without_fake_continuation() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/podcasts?catalog=personalize&limit=3&platform=netease&account=podcast-user",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:336355127");
        assert_eq!(
            json["data"][0]["extensions"]["request"]["catalog"],
            "personalized"
        );
        assert_eq!(json["meta"]["pagination"]["limit"], 3);
        assert_eq!(json["meta"]["pagination"]["offset"], 0);
        assert_eq!(json["meta"]["pagination"]["total"], Value::Null);
        assert_eq!(json["meta"]["pagination"]["next_offset"], Value::Null);
        assert_eq!(json["meta"]["pagination"]["has_more"], false);
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["catalog"],
            "personalized"
        );
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["limit_applied"],
            true
        );
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "podcast-user");
    }

    #[tokio::test]
    async fn category_hot_podcast_catalog_preserves_insertions_and_request_window() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/podcasts?catalog=category_hot&categoryId=2&limit=3&offset=6&platform=netease&account=podcast-user",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:336355127");
        assert_eq!(
            json["data"][0]["extensions"]["request"]["catalog"],
            "category_hot"
        );
        assert_eq!(json["data"][0]["extensions"]["request"]["category_id"], "2");
        assert_eq!(json["meta"]["pagination"]["limit"], 3);
        assert_eq!(json["meta"]["pagination"]["offset"], 6);
        assert_eq!(json["meta"]["pagination"]["total"], 1000);
        assert_eq!(json["meta"]["pagination"]["next_offset"], 9);
        assert_eq!(json["meta"]["pagination"]["has_more"], true);
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["catalog"],
            "category_hot"
        );
        assert_eq!(json["meta"]["pagination"]["extensions"]["category_id"], "2");
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["limit_applied"],
            false
        );
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "podcast-user");
    }

    #[tokio::test]
    async fn category_featured_podcast_catalog_exposes_known_more_without_fake_cursor() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/podcasts?catalog=category_recommend&category_id=2&limit=2&platform=netease&account=podcast-user",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:336355127");
        assert_eq!(
            json["data"][0]["extensions"]["request"]["catalog"],
            "category_featured"
        );
        assert_eq!(json["data"][0]["extensions"]["request"]["category_id"], "2");
        assert_eq!(json["meta"]["pagination"]["limit"], 2);
        assert_eq!(json["meta"]["pagination"]["offset"], 0);
        assert_eq!(json["meta"]["pagination"]["total"], Value::Null);
        assert_eq!(json["meta"]["pagination"]["next_offset"], Value::Null);
        assert_eq!(json["meta"]["pagination"]["has_more"], true);
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["catalog"],
            "category_featured"
        );
        assert_eq!(json["meta"]["pagination"]["extensions"]["category_id"], "2");
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["limit_applied"],
            false
        );
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["continuation_supported"],
            false
        );
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "podcast-user");
    }

    #[tokio::test]
    async fn podcast_catalog_rejects_missing_unknown_and_invalid_controls() {
        for path in [
            "/v1/podcasts",
            "/v1/podcasts?catalog=unknown",
            "/v1/podcasts?catalog=featured&offset=1",
            "/v1/podcasts?catalog=featured&category_id=2",
            "/v1/podcasts?catalog=personalized&offset=1",
            "/v1/podcasts?catalog=personalized&category_id=2",
            "/v1/podcasts?catalog=category_hot",
            "/v1/podcasts?catalog=category_featured",
            "/v1/podcasts?catalog=category_featured&category_id=2&offset=1",
            "/v1/podcasts?catalog=hot&limit=0",
            "/v1/podcasts?catalog=hot&limit=101",
            "/v1/podcasts?catalog=hot&offset=invalid",
            "/v1/podcasts?catalog=hot&category_id=2",
            "/v1/podcasts?catalog=hot&unknown=true",
            "/v1/podcasts?catalog=hot&platform=unknown",
        ] {
            let (status, json) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "path: {path}");
            assert_eq!(json["error"]["code"], "invalid_request", "path: {path}");
        }
    }

    #[tokio::test]
    async fn podcast_and_episode_details_keep_show_program_and_audio_ids_separate() {
        let (status, podcast) = json_response_from(
            test_app_with_provider(),
            "/v1/podcasts/netease:336355127?account=podcast-user",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(podcast["data"]["ref"], "netease:336355127");
        assert_eq!(podcast["data"]["name"], "代码时间");
        assert_eq!(podcast["data"]["episode_count"], 120);
        assert_eq!(podcast["data"]["extensions"]["account"], "podcast-user");
        assert_eq!(podcast["meta"]["platform"], "netease");
        assert_eq!(podcast["meta"]["account"], "podcast-user");

        let (status, episode) = json_response_from(
            test_app_with_provider(),
            "/v1/episodes/netease:1367665101?account=podcast-user",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(episode["data"]["ref"], "netease:1367665101");
        assert_eq!(episode["data"]["podcast_ref"], "netease:336355127");
        assert_eq!(episode["data"]["audio"]["ref"], "netease:2603965162");
        assert_ne!(episode["data"]["ref"], episode["data"]["audio"]["ref"]);
        assert_eq!(episode["data"]["extensions"]["account"], "podcast-user");
        assert_eq!(episode["meta"]["platform"], "netease");
        assert_eq!(episode["meta"]["account"], "podcast-user");
    }

    #[tokio::test]
    async fn podcast_episode_lyrics_keep_episode_audio_and_transcript_formats_separate() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/episodes/netease:1367665101/lyrics?account=podcast-user",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["ref"], "netease:1367665101");
        assert_eq!(json["data"]["audio_ref"], "netease:2603965162");
        assert_eq!(json["data"]["lyrics"]["track_ref"], "netease:2603965162");
        assert_eq!(json["data"]["lyrics"]["plain"], "[00:00.000]节目转写");
        assert_eq!(json["data"]["lyrics"]["format"], "netease_voice_json");
        let transcript: Value = serde_json::from_str(
            json["data"]["lyrics"]["word_synced"]
                .as_str()
                .expect("word-synced transcript"),
        )
        .expect("valid transcript JSON");
        assert_eq!(transcript["duration"], 258_000);
        assert_eq!(json["data"]["extensions"]["account"], "podcast-user");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "podcast-user");
    }

    #[tokio::test]
    async fn podcast_episode_catalog_accepts_reference_aliases_account_and_pagination() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/podcasts/netease:336355127/episodes?limit=25&offset=50&asc=1&account=podcast-user",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:1367665101");
        assert_eq!(json["data"][0]["podcast_ref"], "netease:336355127");
        assert_eq!(json["data"][0]["audio"]["ref"], "netease:2603965162");
        assert_eq!(json["data"][0]["extensions"]["account"], "podcast-user");
        assert_eq!(json["meta"]["pagination"]["limit"], 25);
        assert_eq!(json["meta"]["pagination"]["offset"], 50);
        assert_eq!(json["meta"]["pagination"]["total"], 12);
        assert_eq!(json["meta"]["pagination"]["next_offset"], 51);
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["request"]["ascending"],
            true
        );
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["request"]["account"],
            "podcast-user"
        );
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "podcast-user");
    }

    #[tokio::test]
    async fn podcast_routes_reject_invalid_references_pagination_and_order() {
        for path in [
            "/v1/podcasts/336355127",
            "/v1/episodes/1367665101",
            "/v1/episodes/1367665101/lyrics",
            "/v1/episodes/netease:1367665101/lyrics?unknown=true",
            "/v1/podcasts/netease:336355127/episodes?limit=0",
            "/v1/podcasts/netease:336355127/episodes?limit=101",
            "/v1/podcasts/netease:336355127/episodes?offset=invalid",
            "/v1/podcasts/netease:336355127/episodes?ascending=newest",
        ] {
            let (status, json) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "path: {path}");
            assert_eq!(json["error"]["code"], "invalid_request", "path: {path}");
        }
    }

    #[tokio::test]
    async fn podcast_episode_stream_reuses_unified_quality_account_and_fallback_controls() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/episodes/netease:1367665101/stream?level=jyeffect&backend=v1&br=192123&account=podcast-user&fallback=false",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["ref"], "netease:1367665101");
        assert_eq!(json["data"]["audio_ref"], "netease:2603965162");
        assert_eq!(json["data"]["stream"]["origin_track"], "netease:2603965162");
        assert_eq!(
            json["data"]["stream"]["resolved_track"],
            "netease:2603965162"
        );
        assert_eq!(json["data"]["stream"]["requested_quality"], "surround");
        assert_eq!(json["data"]["stream"]["bitrate"], 192_123);
        assert_eq!(
            json["data"]["stream"]["headers"]["x-test-stream-variant"],
            "modern"
        );
        assert_eq!(
            json["data"]["stream"]["headers"]["x-test-origin-account"],
            "podcast-user"
        );
        assert_eq!(
            json["data"]["stream"]["attempts"].as_array().map(Vec::len),
            Some(1)
        );
        assert_eq!(
            json["data"]["extensions"]["episode"]["ref"],
            "netease:1367665101"
        );
        assert_eq!(
            json["data"]["extensions"]["episode"]["audio"]["ref"],
            "netease:2603965162"
        );
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "podcast-user");

        let (status, fallback) = json_response_from(
            test_app_with_provider(),
            "/v1/episodes/netease:1367665101/stream?unblock=true&source=qq&account=green-vip&level=sky&backend=modern",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(fallback["data"]["audio_ref"], "netease:2603965162");
        assert_eq!(
            fallback["data"]["stream"]["attempts"]
                .as_array()
                .map(Vec::len),
            Some(2)
        );
        assert_eq!(fallback["data"]["stream"]["attempts"][0]["platform"], "qq");
        assert_eq!(
            fallback["data"]["stream"]["attempts"][0]["status"],
            "unavailable"
        );
        assert_eq!(
            fallback["data"]["stream"]["attempts"][1]["platform"],
            "netease"
        );
        assert_eq!(
            fallback["data"]["stream"]["attempts"][1]["status"],
            "success"
        );
        assert_eq!(fallback["meta"]["account"], "green-vip");
    }

    #[tokio::test]
    async fn podcast_episode_stream_redirects_and_rejects_invalid_controls() {
        let response = test_app_with_provider()
            .oneshot(
                Request::builder()
                    .uri(
                        "/v1/episodes/netease:1367665101/stream/redirect?quality=high&fallback=false",
                    )
                    .body(Body::empty())
                    .expect("build episode stream redirect request"),
            )
            .await
            .expect("episode stream redirect request succeeds");
        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(
            response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok()),
            Some("https://example.test/audio.mp3")
        );

        for path in [
            "/v1/episodes/1367665101/stream",
            "/v1/episodes/netease:1367665101/stream?quality=future",
            "/v1/episodes/netease:1367665101/stream?br=invalid",
            "/v1/episodes/netease:1367665101/stream?unblock=true&playback_platform=qq",
            "/v1/episodes/netease:1367665101/stream?unknown=true",
            "/v1/episodes/netease:1367665101/stream/redirect?unknown=true",
        ] {
            let (status, json) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "path: {path}");
            assert_eq!(json["error"]["code"], "invalid_request", "path: {path}");
        }
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
    async fn video_detail_infers_mv_or_video_and_preserves_explicit_type_and_account() {
        for (path, expected_kind) in [
            ("/v1/videos/netease:22695250?account=collector", "mv"),
            (
                "/v1/videos/netease:D1C2B3A40987654321ABCDEF12345678?account=collector",
                "video",
            ),
            (
                "/v1/videos/netease:22695250?type=video&account=collector",
                "video",
            ),
        ] {
            let (status, detail) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::OK, "{path}");
            assert_eq!(detail["data"]["kind"], expected_kind, "{path}");
            assert_eq!(detail["data"]["video"]["platform"], "netease", "{path}");
            assert_eq!(
                detail["data"]["resolutions"][0]["resolution"], 1080,
                "{path}"
            );
            assert_eq!(
                detail["data"]["video"]["extensions"]["account"], "collector",
                "{path}"
            );
            assert_eq!(detail["meta"]["account"], "collector", "{path}");
        }
    }

    #[tokio::test]
    async fn video_stats_and_stream_use_unified_kind_resolution_and_metadata() {
        let (status, stats) = json_response_from(
            test_app_with_provider(),
            "/v1/videos/netease:22695250/stats?kind=mv&account=collector",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stats["data"]["video_ref"], "netease:22695250");
        assert_eq!(stats["data"]["kind"], "mv");
        assert_eq!(stats["data"]["like_count"], 4_662);
        assert_eq!(stats["data"]["comment_count"], 675);
        assert_eq!(stats["data"]["extensions"]["account"], "collector");

        let (status, stream) = json_response_from(
            test_app_with_provider(),
            "/v1/videos/netease:22695250/stream?type=mv&res=720&account=collector",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stream["data"]["video_ref"], "netease:22695250");
        assert_eq!(stream["data"]["available"], true);
        assert_eq!(stream["data"]["requested_resolution"], 720);
        assert_eq!(stream["data"]["actual_resolution"], 720);
        assert_eq!(stream["data"]["extensions"]["account"], "collector");
        assert_eq!(stream["meta"]["account"], "collector");
    }

    #[tokio::test]
    async fn video_stream_redirects_only_when_a_real_url_is_available() {
        let response = test_app_with_provider()
            .oneshot(
                Request::builder()
                    .uri("/v1/videos/netease:22695250/stream/redirect?kind=mv&resolution=1080")
                    .body(Body::empty())
                    .expect("build video redirect request"),
            )
            .await
            .expect("video redirect request succeeds");
        assert_eq!(response.status(), StatusCode::FOUND);
        assert_eq!(
            response
                .headers()
                .get(header::LOCATION)
                .and_then(|value| value.to_str().ok()),
            Some("https://example.test/video/22695250.mp4")
        );

        let (status, stream) = json_response_from(
            test_app_with_provider(),
            "/v1/videos/netease:unavailable/stream",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stream["data"]["available"], false);
        assert_eq!(stream["data"]["url"], Value::Null);

        let (status, unavailable) = json_response_from(
            test_app_with_provider(),
            "/v1/videos/netease:unavailable/stream/redirect",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(unavailable["error"]["code"], "resource_not_found");
        assert_eq!(
            unavailable["error"]["details"]["stream"]["available"],
            false
        );
    }

    #[tokio::test]
    async fn video_routes_reject_bad_types_resolutions_and_unknown_parameters() {
        for path in [
            "/v1/videos/netease:22695250?kind=future",
            "/v1/videos/netease:22695250?unknown=true",
            "/v1/videos/netease:22695250/stats?type=future",
            "/v1/videos/netease:22695250/stream?resolution=0",
            "/v1/videos/netease:22695250/stream?res=4321",
            "/v1/videos/netease:22695250/stream?unknown=true",
            "/v1/videos/netease:22695250/stream/redirect?unknown=true",
        ] {
            let (status, error) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(error["error"]["code"], "invalid_request", "{path}");
        }
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
    async fn general_chart_catalog_uses_unified_view_platform_and_account_parameters() {
        let (status, catalog) = json_response_from(
            test_app_with_provider(),
            "/v1/charts?platform=netease&account=vip",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(catalog["data"]["platform"], "netease");
        assert_eq!(catalog["data"]["view"], "summary");
        assert_eq!(catalog["data"]["groups"][0]["code"], "OFFICIAL");
        assert_eq!(
            catalog["data"]["groups"][0]["charts"][0]["ref"],
            "netease:19723756"
        );
        assert_eq!(
            catalog["data"]["groups"][0]["charts"][0]["previews"][0]["track_ref"],
            "netease:3404238777"
        );
        assert_eq!(catalog["data"]["extensions"]["account"], "vip");
        assert_eq!(catalog["meta"]["platform"], "netease");
        assert_eq!(catalog["meta"]["account"], "vip");
        assert!(catalog["meta"].get("pagination").is_none());
    }

    #[tokio::test]
    async fn general_chart_catalog_accepts_all_reference_view_aliases() {
        for (query, expected) in [
            ("view=overview", "overview"),
            ("catalog=toplist", "overview"),
            ("view=toplist_detail", "summary"),
            ("view=detail-v2", "modern"),
            ("view=toplist_detail_v2", "modern"),
        ] {
            let (status, catalog) =
                json_response_from(test_app_with_provider(), &format!("/v1/charts?{query}")).await;
            assert_eq!(status, StatusCode::OK, "{query}");
            assert_eq!(catalog["data"]["view"], expected, "{query}");
        }

        let (status, invalid) =
            json_response_from(test_app_with_provider(), "/v1/charts?view=future").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(invalid["error"]["code"], "invalid_request");
    }

    #[tokio::test]
    async fn artist_charts_accept_named_and_numeric_areas_and_reject_conflicts() {
        for (query, expected) in [
            ("area=western", "western"),
            ("type=3", "korean"),
            ("area=jp", "japanese"),
            ("type=1", "chinese"),
        ] {
            let (status, chart) = json_response_from(
                test_app_with_provider(),
                &format!("/v1/charts/artists?platform=netease&account=vip&{query}"),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{query}");
            assert_eq!(chart["data"]["area"], expected, "{query}");
            assert_eq!(chart["data"]["entries"][0]["rank"], 1, "{query}");
            assert_eq!(
                chart["data"]["entries"][0]["artist"]["ref"], "netease:3684",
                "{query}"
            );
            assert_eq!(
                chart["data"]["entries"][0]["extensions"]["account"], "vip",
                "{query}"
            );
            assert_eq!(chart["meta"]["account"], "vip", "{query}");
        }

        for query in ["area=western&type=3", "area=unknown"] {
            let (status, invalid) = json_response_from(
                test_app_with_provider(),
                &format!("/v1/charts/artists?{query}"),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{query}");
            assert_eq!(invalid["error"]["code"], "invalid_request", "{query}");
        }
    }

    #[tokio::test]
    async fn chart_tracks_reuse_the_platform_playlist_snapshot_with_unified_pagination() {
        let (status, tracks) = json_response_from(
            test_app_with_provider(),
            "/v1/charts/netease:19723756/tracks?account=vip&limit=2&offset=3",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(tracks["data"][0]["ref"], "netease:123");
        assert_eq!(tracks["meta"]["platform"], "netease");
        assert_eq!(tracks["meta"]["account"], "vip");
        assert_eq!(tracks["meta"]["pagination"]["limit"], 2);
        assert_eq!(tracks["meta"]["pagination"]["offset"], 3);
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
    async fn playlist_create_and_update_preserve_unified_and_reference_fields() {
        let (status, created) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/playlists",
            Some(json!({
                "platform": "netease",
                "account": "personal",
                "name": "跨平台收藏",
                "privacy": 10,
                "type": "VIDEO"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(created["data"]["playlist_ref"], "netease:9001");
        assert_eq!(created["data"]["action"], "create");
        assert_eq!(created["data"]["playlist"]["name"], "跨平台收藏");
        assert_eq!(created["data"]["extensions"]["visibility"], "private");
        assert_eq!(created["data"]["extensions"]["kind"], "video");
        assert_eq!(created["meta"]["platform"], "netease");
        assert_eq!(created["meta"]["account"], "personal");

        let (status, updated) = json_request_from(
            test_app_with_provider(),
            Method::PATCH,
            "/v1/playlists/netease:9001",
            Some(json!({
                "account": "personal",
                "name": "新的名字",
                "desc": "",
                "tags": "华语;流行",
                "variant": "batch"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(updated["data"]["playlist_ref"], "netease:9001");
        assert_eq!(updated["data"]["action"], "update");
        assert_eq!(updated["data"]["extensions"]["name"], "新的名字");
        assert_eq!(updated["data"]["extensions"]["description"], "");
        assert_eq!(
            updated["data"]["extensions"]["tags"],
            json!(["华语", "流行"])
        );
        assert_eq!(updated["data"]["extensions"]["variant"], "batch");
        assert_eq!(updated["meta"]["account"], "personal");
    }

    #[tokio::test]
    async fn playlist_delete_supports_single_and_ordered_batch_references() {
        let (status, deleted) = json_request_from(
            test_app_with_provider(),
            Method::DELETE,
            "/v1/playlists/netease:9001?account=personal",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(deleted["data"]["playlist_refs"], json!(["netease:9001"]));
        assert_eq!(deleted["meta"]["account"], "personal");

        let (status, deleted) = json_request_from(
            test_app_with_provider(),
            Method::DELETE,
            "/v1/playlists",
            Some(json!({
                "ids": [9001, "9002,9001"],
                "platform": "netease",
                "account": "personal"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            deleted["data"]["playlist_refs"],
            json!(["netease:9001", "netease:9002", "netease:9001"])
        );
        assert_eq!(deleted["data"]["extensions"]["account"], "personal");
    }

    #[tokio::test]
    async fn playlist_item_routes_keep_tracks_and_videos_distinct() {
        let (status, tracks) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/playlists/netease:9001/tracks",
            Some(json!({
                "trackIds": [185809, "5268328,185809"],
                "account": "personal"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(tracks["data"]["kind"], "track");
        assert_eq!(tracks["data"]["action"], "add");
        assert_eq!(
            tracks["data"]["item_refs"],
            json!(["netease:185809", "netease:5268328", "netease:185809"])
        );
        assert_eq!(tracks["data"]["snapshot_id"], "snapshot-items");

        let (status, videos) = json_request_from(
            test_app_with_provider(),
            Method::DELETE,
            "/v1/playlists/netease:9001/items",
            Some(json!({
                "refs": ["netease:89ADDE33C0AAE8EC14B99F32C116F479"],
                "type": 3,
                "account": "personal"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(videos["data"]["kind"], "video");
        assert_eq!(videos["data"]["action"], "remove");
        assert_eq!(
            videos["data"]["item_refs"],
            json!(["netease:89ADDE33C0AAE8EC14B99F32C116F479"])
        );

        let (status, videos) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/playlists/netease:9001/videos",
            Some(json!({ "ids": "opaque-video-id" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(videos["data"]["kind"], "video");
        assert_eq!(videos["meta"]["account"], "default");
    }

    #[tokio::test]
    async fn playlist_and_account_order_routes_preserve_exact_input_order() {
        let (status, tracks) = json_request_from(
            test_app_with_provider(),
            Method::PUT,
            "/v1/playlists/netease:9001/tracks/order",
            Some(json!({
                "ids": [5268328, 185809, 5268328],
                "account": "personal"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            tracks["data"]["track_refs"],
            json!(["netease:5268328", "netease:185809", "netease:5268328"])
        );
        assert_eq!(tracks["data"]["snapshot_id"], "snapshot-order");

        let (status, playlists) = json_request_from(
            test_app_with_provider(),
            Method::PUT,
            "/v1/account/playlists/order",
            Some(json!({
                "ids": "9003,9001,9002",
                "platform": "netease",
                "account": "personal"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            playlists["data"]["playlist_refs"],
            json!(["netease:9003", "netease:9001", "netease:9002"])
        );
        assert_eq!(playlists["meta"]["account"], "personal");
    }

    #[tokio::test]
    async fn playlist_cover_accepts_binary_images_and_reference_parameter_aliases() {
        let (status, cover) = binary_request_with_method(
            test_app_with_provider(),
            Method::PUT,
            "/v1/playlists/netease:9001/cover?account=personal&filename=cover.png&imgSize=600&imgX=2&imgY=3",
            Some("image/png"),
            vec![0x89, b'P', b'N', b'G'],
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(cover["data"]["playlist_ref"], "netease:9001");
        assert_eq!(
            cover["data"]["image"]["url"],
            "https://example.test/playlist-cover.jpg"
        );
        assert_eq!(
            cover["data"]["image"]["extensions"]["filename"],
            "cover.png"
        );
        assert_eq!(
            cover["data"]["image"]["extensions"]["content_type"],
            "image/png"
        );
        assert_eq!(cover["data"]["image"]["extensions"]["data_len"], 4);
        assert_eq!(cover["data"]["image"]["extensions"]["image_size"], 600);
        assert_eq!(cover["data"]["image"]["extensions"]["crop_x"], 2);
        assert_eq!(cover["data"]["image"]["extensions"]["crop_y"], 3);
        assert_eq!(cover["meta"]["platform"], "netease");
        assert_eq!(cover["meta"]["account"], "personal");
    }

    #[tokio::test]
    async fn playlist_write_routes_reject_conflicts_unknown_fields_and_mixed_platforms() {
        let invalid_requests = [
            (
                Method::POST,
                "/v1/playlists",
                json!({
                    "name": "冲突",
                    "visibility": "public",
                    "privacy": 10
                }),
            ),
            (
                Method::POST,
                "/v1/playlists",
                json!({ "name": "冲突", "kind": "normal", "type": "video" }),
            ),
            (
                Method::POST,
                "/v1/playlists",
                json!({ "name": "未知", "unexpected": true }),
            ),
            (
                Method::PATCH,
                "/v1/playlists/netease:9001",
                json!({ "description": "a", "desc": "b" }),
            ),
            (
                Method::PATCH,
                "/v1/playlists/netease:9001",
                json!({ "name": "a", "variant": "parallel" }),
            ),
            (
                Method::DELETE,
                "/v1/playlists",
                json!({
                    "refs": ["netease:1", "qq:2"]
                }),
            ),
            (
                Method::DELETE,
                "/v1/playlists",
                json!({ "refs": "netease:1", "ids": 2 }),
            ),
            (
                Method::POST,
                "/v1/playlists/netease:9001/tracks",
                json!({ "ids": 185809, "kind": "video" }),
            ),
            (
                Method::POST,
                "/v1/playlists/netease:9001/items",
                json!({ "kind": "track" }),
            ),
            (
                Method::PUT,
                "/v1/account/playlists/order",
                json!({ "refs": "netease:1", "platform": "netease" }),
            ),
        ];

        for (method, path, body) in invalid_requests {
            let (status, response) =
                json_request_from(test_app_with_provider(), method, path, Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(response["error"]["code"], "invalid_request", "{path}");
        }

        let (status, response) = binary_request_with_method(
            test_app_with_provider(),
            Method::PUT,
            "/v1/playlists/netease:9001/cover?unexpected=true",
            Some("image/jpeg"),
            vec![0xff, 0xd8, 0xff, 0xd9],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(response["error"]["code"], "invalid_request");
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

    #[test]
    fn stream_quality_parser_accepts_all_unified_and_netease_levels() {
        for (value, expected) in [
            ("auto", Quality::Auto),
            ("low", Quality::Low),
            ("standard", Quality::Standard),
            ("higher", Quality::Higher),
            ("high", Quality::High),
            ("exhigh", Quality::High),
            ("lossless", Quality::Lossless),
            ("hires", Quality::Hires),
            ("hi_res", Quality::Hires),
            ("surround", Quality::Surround),
            ("jyeffect", Quality::Surround),
            ("spatial", Quality::Spatial),
            ("sky", Quality::Spatial),
            ("dolby", Quality::Dolby),
            ("atmos", Quality::Dolby),
            ("master", Quality::Master),
            ("jymaster", Quality::Master),
        ] {
            assert_eq!(
                parse_quality(Some(value)).expect(value),
                expected,
                "{value}"
            );
        }
    }

    #[test]
    fn stream_variant_parser_accepts_unified_and_netease_aliases() {
        for (value, expected) in [
            ("default", StreamVariant::Default),
            ("auto", StreamVariant::Default),
            ("legacy", StreamVariant::Legacy),
            ("old", StreamVariant::Legacy),
            ("v0", StreamVariant::Legacy),
            ("song_url", StreamVariant::Legacy),
            ("modern", StreamVariant::Modern),
            ("new", StreamVariant::Modern),
            ("v1", StreamVariant::Modern),
            ("song-url-v1", StreamVariant::Modern),
        ] {
            assert_eq!(
                parse_stream_variant(Some(value)).expect(value),
                expected,
                "{value}"
            );
        }
    }

    #[tokio::test]
    async fn stream_and_download_forward_account_to_origin_metadata_lookup() {
        let (status, stream) = json_response_from(
            test_app_with_provider(),
            "/v1/tracks/netease:9001/stream?account=locker&fallback=false",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(stream["data"]["headers"]["x-test-origin-account"], "locker");
        assert_eq!(stream["meta"]["account"], "locker");

        let (status, download) = json_response_from(
            test_app_with_provider(),
            "/v1/tracks/netease:9001/download?account=locker",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(download["data"]["extensions"]["origin_account"], "locker");
        assert_eq!(download["meta"]["account"], "locker");
    }

    #[tokio::test]
    async fn track_stream_accepts_netease_level_and_backend_aliases() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/tracks/netease:2709812973/stream?level=jyeffect&backend=v1&br=192123&fallback=false",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["requested_quality"], "surround");
        assert_eq!(json["data"]["bitrate"], 192_123);
        assert_eq!(json["data"]["headers"]["x-test-stream-variant"], "modern");
        assert_eq!(json["data"]["attempts"].as_array().map(Vec::len), Some(1));
    }

    #[tokio::test]
    async fn track_stream_unblock_tries_selected_source_then_origin() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/tracks/netease:2709812973/stream?unblock=true&source=qq&account=green-vip&fallback=false&level=sky&backend=modern",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["requested_quality"], "spatial");
        assert_eq!(json["data"]["attempts"].as_array().map(Vec::len), Some(2));
        assert_eq!(json["data"]["attempts"][0]["platform"], "qq");
        assert_eq!(json["data"]["attempts"][0]["account"], "green-vip");
        assert_eq!(json["data"]["attempts"][0]["status"], "unavailable");
        assert_eq!(json["data"]["attempts"][1]["platform"], "netease");
        assert_eq!(json["data"]["attempts"][1]["status"], "success");
        assert_eq!(json["meta"]["account"], "green-vip");
    }

    #[tokio::test]
    async fn track_stream_batch_get_preserves_order_duplicates_and_netease_aliases() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/tracks/streams?id=2709812973,1969519579,2709812973&platform=netease&level=jyeffect&backend=v1&br=192123&fallback=false",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["outcomes"].as_array().map(Vec::len), Some(3));
        assert_eq!(
            json["data"]["outcomes"][0]["track_ref"],
            "netease:2709812973"
        );
        assert_eq!(
            json["data"]["outcomes"][1]["track_ref"],
            "netease:1969519579"
        );
        assert_eq!(
            json["data"]["outcomes"][2]["track_ref"],
            "netease:2709812973"
        );
        for outcome in json["data"]["outcomes"].as_array().expect("outcomes") {
            assert_eq!(outcome["status"], "success");
            assert_eq!(outcome["stream"]["requested_quality"], "surround");
            assert_eq!(outcome["stream"]["bitrate"], 192_123);
            assert_eq!(
                outcome["stream"]["headers"]["x-test-stream-variant"],
                "modern"
            );
            assert_eq!(
                outcome["stream"]["attempts"].as_array().map(Vec::len),
                Some(1)
            );
        }
        assert_eq!(json["data"]["extensions"]["quality"], "surround");
        assert_eq!(json["data"]["extensions"]["variant"], "modern");
        assert_eq!(json["data"]["extensions"]["bitrate"], 192_123);
        assert_eq!(json["meta"]["platform"], "netease");
    }

    #[tokio::test]
    async fn track_stream_batch_post_supports_mixed_refs_and_per_item_errors() {
        let body = json!({
            "refs": ["netease:1", "qq:2", "netease:1"],
                "quality": "high",
                "variant": "legacy",
                "bitrate": 128001,
                "fallback": false
        });
        serde_json::from_value::<StreamBatchBody>(body.clone()).expect("parse stream batch body");
        let (status, json) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/tracks/streams",
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{json}");
        assert_eq!(json["data"]["outcomes"].as_array().map(Vec::len), Some(3));
        assert_eq!(json["data"]["outcomes"][0]["status"], "success");
        assert_eq!(json["data"]["outcomes"][0]["stream"]["bitrate"], 128_001);
        assert_eq!(json["data"]["outcomes"][1]["track_ref"], "qq:2");
        assert_eq!(json["data"]["outcomes"][1]["status"], "unavailable");
        assert_eq!(
            json["data"]["outcomes"][1]["error_code"],
            "platform_unavailable"
        );
        assert_eq!(json["data"]["outcomes"][2]["track_ref"], "netease:1");
        assert_eq!(json["data"]["outcomes"][2]["status"], "success");
        assert_eq!(json["meta"]["platform"], Value::Null);
    }

    #[tokio::test]
    async fn track_stream_batch_unblock_uses_the_unified_resolver() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/tracks/streams?ids=2709812973&platform=netease&unblock=true&source=qq&account=green-vip&level=sky&backend=v1&fallback=false",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let outcome = &json["data"]["outcomes"][0];
        assert_eq!(outcome["status"], "success");
        assert_eq!(outcome["stream"]["requested_quality"], "spatial");
        assert_eq!(
            outcome["stream"]["attempts"].as_array().map(Vec::len),
            Some(2)
        );
        assert_eq!(outcome["stream"]["attempts"][0]["platform"], "qq");
        assert_eq!(outcome["stream"]["attempts"][0]["account"], "green-vip");
        assert_eq!(outcome["stream"]["attempts"][1]["platform"], "netease");
        assert_eq!(json["data"]["extensions"]["fallback"], true);
        assert_eq!(json["meta"]["account"], "green-vip");
    }

    #[tokio::test]
    async fn track_stream_batch_rejects_ambiguous_or_malformed_get_inputs() {
        for path in [
            "/v1/tracks/streams",
            "/v1/tracks/streams?refs=netease:1&ids=1",
            "/v1/tracks/streams?refs=netease:1&platform=netease",
            "/v1/tracks/streams?refs=netease:1,,netease:2",
            "/v1/tracks/streams?refs=invalid",
            "/v1/tracks/streams?ids=1&platform=unknown",
            "/v1/tracks/streams?ids=1&br=invalid",
            "/v1/tracks/streams?ids=1&unblock=true&playback_platform=qq",
            "/v1/tracks/streams?ids=1&unknown=true",
        ] {
            let (status, json) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(json["error"]["code"], "invalid_request", "{path}");
        }
    }

    #[tokio::test]
    async fn track_stream_batch_rejects_ambiguous_or_malformed_post_inputs() {
        for body in [
            json!({}),
            json!({ "refs": ["netease:1"], "ids": ["1"] }),
            json!({ "refs": [] }),
            json!({ "ids": ["1"], "platform": "unknown" }),
            json!({ "ids": ["1"], "bitrate": -1 }),
            json!({ "ids": ["1"], "unknown": true }),
        ] {
            let (status, json) = json_request_from(
                test_app_with_provider(),
                Method::POST,
                "/v1/tracks/streams",
                Some(body.clone()),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
            assert_eq!(json["error"]["code"], "invalid_request", "{body}");
        }
    }

    #[tokio::test]
    async fn track_download_accepts_modern_level_backend_and_bitrate_aliases() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/tracks/netease:2709812973/download?level=sky&backend=v1&br=192123&account=vip",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["ref"], "netease:2709812973");
        assert_eq!(json["data"]["available"], true);
        assert_eq!(json["data"]["requested_quality"], "spatial");
        assert_eq!(json["data"]["bitrate"], 192_123);
        assert_eq!(json["data"]["extensions"]["variant"], "modern");
        assert_eq!(json["data"]["extensions"]["account"], "vip");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "vip");
    }

    #[tokio::test]
    async fn track_download_keeps_an_unavailable_url_as_successful_data() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/tracks/netease:fallback/download?quality=spatial&variant=modern",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["available"], false);
        assert_eq!(json["data"]["url"], Value::Null);
        assert_eq!(json["data"]["platform_code"], -110);
        assert_eq!(json["data"]["actual_quality"], "auto");
    }

    #[tokio::test]
    async fn track_download_redirect_prefers_download_and_falls_back_to_stream() {
        for (reference, expected_location) in [
            (
                "netease:2709812973",
                "https://example.test/download/2709812973.flac",
            ),
            ("netease:fallback", "https://example.test/audio.mp3"),
        ] {
            let response = test_app_with_provider()
                .oneshot(
                    Request::builder()
                        .uri(format!(
                            "/v1/tracks/{reference}/download/redirect?level=exhigh"
                        ))
                        .body(Body::empty())
                        .expect("build download redirect request"),
                )
                .await
                .expect("download redirect request succeeds");
            assert_eq!(response.status(), StatusCode::FOUND, "{reference}");
            assert_eq!(
                response
                    .headers()
                    .get(header::LOCATION)
                    .and_then(|value| value.to_str().ok()),
                Some(expected_location),
                "{reference}"
            );
        }
    }

    #[tokio::test]
    async fn track_download_rejects_invalid_or_unknown_parameters() {
        for path in [
            "/v1/tracks/netease:1/download?quality=future",
            "/v1/tracks/netease:1/download?variant=future",
            "/v1/tracks/netease:1/download?br=invalid",
            "/v1/tracks/netease:1/download?unknown=true",
            "/v1/tracks/netease:1/download/redirect?unknown=true",
        ] {
            let (status, json) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(json["error"]["code"], "invalid_request", "{path}");
        }
    }

    #[tokio::test]
    async fn track_stream_rejects_invalid_or_ambiguous_modern_parameters() {
        for path in [
            "/v1/tracks/netease:2709812973/stream?variant=future",
            "/v1/tracks/netease:2709812973/stream?unblock=maybe",
            "/v1/tracks/netease:2709812973/stream?unblock=true&source=unknown",
            "/v1/tracks/netease:2709812973/stream?unblock=true&playback_platform=qq",
            "/v1/tracks/netease:2709812973/stream?unblock=true&fallback_platforms=qq",
            "/v1/tracks/netease:2709812973/stream?unknown=true",
        ] {
            let (status, json) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(json["error"]["code"], "invalid_request", "{path}");
        }
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
    async fn country_calling_codes_use_selected_platform_account_and_unified_fields() {
        let (status, response) = json_response_from(
            test_app_with_provider(),
            "/v1/auth/country-codes?platform=netease&account=personal",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(response["data"][0]["label"], "常用");
        assert_eq!(response["data"][0]["entries"][0]["calling_code"], "86");
        assert_eq!(response["data"][0]["entries"][0]["region_code"], "CN");
        assert_eq!(response["data"][0]["entries"][0]["name"], "中国");
        assert_eq!(response["data"][0]["entries"][0]["english_name"], "China");
        assert_eq!(
            response["data"][0]["entries"][0]["extensions"]["account"],
            "personal"
        );
        assert_eq!(response["meta"]["platform"], "netease");
        assert_eq!(response["meta"]["account"], "personal");

        let (status, defaulted) =
            json_response_from(test_app_with_provider(), "/v1/auth/country-codes").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(defaulted["meta"]["platform"], "netease");
        assert_eq!(defaulted["meta"]["account"], "default");
    }

    #[tokio::test]
    async fn country_calling_codes_reject_unknown_platforms_and_query_fields() {
        for path in [
            "/v1/auth/country-codes?platform=unknown",
            "/v1/auth/country-codes?unknown=true",
        ] {
            let (status, response) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(response["error"]["code"], "invalid_request", "{path}");
        }
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
        assert!(
            start["data"]["image_data_url"]
                .as_str()
                .is_some_and(|value| value.starts_with("data:image/svg+xml;base64,"))
        );

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
    async fn cloud_library_tracks_use_platform_account_and_unified_pagination() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/account/cloud/tracks?platform=netease&account=locker&limit=10&offset=5",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"][0]["ref"], "netease:9001");
        assert_eq!(json["data"][0]["track"]["ref"], "netease:9001");
        assert_eq!(json["data"][0]["filename"], "反方向的钟.flac");
        assert_eq!(json["data"][0]["file_size"], 50_412_168);
        assert_eq!(json["data"][0]["file_type"], "flac");
        assert_eq!(json["data"][0]["matched_track_ref"], "netease:185809");
        assert_eq!(json["data"][0]["extensions"]["account"], "locker");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "locker");
        assert_eq!(json["meta"]["pagination"]["limit"], 10);
        assert_eq!(json["meta"]["pagination"]["offset"], 5);
        assert_eq!(json["meta"]["pagination"]["total"], 12);
        assert_eq!(json["meta"]["pagination"]["next_offset"], 6);
        assert_eq!(
            json["meta"]["pagination"]["extensions"]["storage_max_size"],
            1_073_741_824_u64
        );
    }

    #[tokio::test]
    async fn cloud_track_details_accept_full_references_and_raw_id_batches() {
        let (status, get_details) = json_response_from(
            test_app_with_provider(),
            "/v1/account/cloud/tracks/details?account=locker&refs=netease:9002,netease:9001",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(get_details["data"][0]["ref"], "netease:9002");
        assert_eq!(get_details["data"][1]["ref"], "netease:9001");
        assert_eq!(get_details["meta"]["platform"], "netease");
        assert_eq!(get_details["meta"]["account"], "locker");

        let (status, post_details) = json_request_from(
            test_app_with_provider(),
            Method::POST,
            "/v1/account/cloud/tracks/details?platform=netease&account=archive",
            Some(json!({ "trackIds": [9002, "9001", 9002] })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(post_details["data"][0]["ref"], "netease:9002");
        assert_eq!(post_details["data"][1]["ref"], "netease:9001");
        assert_eq!(post_details["data"][2]["ref"], "netease:9002");
        assert_eq!(post_details["data"][0]["extensions"]["account"], "archive");
        assert_eq!(post_details["meta"]["account"], "archive");
    }

    #[tokio::test]
    async fn cloud_track_delete_preserves_reference_order_duplicates_and_account() {
        let (status, json) = json_request_from(
            test_app_with_provider(),
            Method::DELETE,
            "/v1/account/cloud/tracks",
            Some(json!({
                "track_refs": ["netease:9002", "netease:9001", "netease:9002"],
                "account": "locker"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["deleted"], true);
        assert_eq!(
            json["data"]["track_refs"],
            json!(["netease:9002", "netease:9001", "netease:9002"])
        );
        assert_eq!(json["data"]["extensions"]["account"], "locker");
        assert_eq!(json["meta"]["platform"], "netease");
        assert_eq!(json["meta"]["account"], "locker");
    }

    #[tokio::test]
    async fn cloud_track_download_exposes_data_and_redirects_with_stream_fallback() {
        let (status, json) = json_response_from(
            test_app_with_provider(),
            "/v1/account/cloud/tracks/netease:9001/download?account=locker",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(json["data"]["ref"], "netease:9001");
        assert_eq!(json["data"]["available"], true);
        assert_eq!(json["data"]["url"], "https://example.test/cloud/9001.flac");
        assert_eq!(json["data"]["requested_quality"], "auto");
        assert_eq!(json["data"]["actual_quality"], "lossless");
        assert_eq!(json["data"]["extensions"]["account"], "locker");
        assert_eq!(json["meta"]["account"], "locker");

        for (reference, expected_location) in [
            ("netease:9001", "https://example.test/cloud/9001.flac"),
            ("netease:unavailable", "https://example.test/audio.mp3"),
        ] {
            let response = test_app_with_provider()
                .oneshot(
                    Request::builder()
                        .uri(format!(
                            "/v1/account/cloud/tracks/{reference}/download/redirect?account=locker"
                        ))
                        .body(Body::empty())
                        .expect("build cloud download redirect request"),
                )
                .await
                .expect("cloud download redirect response");
            assert_eq!(response.status(), StatusCode::FOUND, "{reference}");
            assert_eq!(
                response
                    .headers()
                    .get(header::LOCATION)
                    .expect("redirect location"),
                expected_location,
                "{reference}"
            );
        }
    }

    #[tokio::test]
    async fn cloud_library_routes_reject_conflicts_mixed_platforms_and_unknown_fields() {
        for path in [
            "/v1/account/cloud/tracks?limit=0",
            "/v1/account/cloud/tracks?limit=101",
            "/v1/account/cloud/tracks?unknown=true",
            "/v1/account/cloud/tracks/details",
            "/v1/account/cloud/tracks/details?refs=netease:9001&ids=9001",
            "/v1/account/cloud/tracks/details?refs=netease:9001,qq:9002",
            "/v1/account/cloud/tracks/details?platform=qq&refs=netease:9001",
            "/v1/account/cloud/tracks/netease:9001/download?platform=netease",
        ] {
            let (status, json) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(json["error"]["code"], "invalid_request", "{path}");
        }

        for (method, path, body) in [
            (
                Method::POST,
                "/v1/account/cloud/tracks/details",
                json!({ "refs": ["netease:9001"], "ids": [9001] }),
            ),
            (
                Method::POST,
                "/v1/account/cloud/tracks/details",
                json!({ "refs": { "id": "netease:9001" } }),
            ),
            (
                Method::DELETE,
                "/v1/account/cloud/tracks",
                json!({ "platform": "netease", "account": "locker" }),
            ),
            (
                Method::DELETE,
                "/v1/account/cloud/tracks",
                json!({ "ids": [9001], "unexpected": true }),
            ),
        ] {
            let (status, json) =
                json_request_from(test_app_with_provider(), method, path, Some(body)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(json["error"]["code"], "invalid_request", "{path}");
        }
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
    async fn membership_endpoints_separate_public_user_and_current_account_status() {
        let (status, public) = json_response_from(
            test_app_with_provider(),
            "/v1/users/netease:32953014/membership?account=viewer",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(public["data"]["user_ref"], "netease:32953014");
        assert_eq!(public["data"]["level"], 7);
        assert!(public["data"]["active"].is_null());
        assert_eq!(public["data"]["annual_count"], 1);
        assert_eq!(public["data"]["extensions"]["account"], "viewer");
        assert_eq!(public["meta"]["platform"], "netease");
        assert_eq!(public["meta"]["account"], "viewer");

        let (status, current) = json_response_from(
            test_app_with_provider(),
            "/v1/account/membership?platform=netease&account=vip-user",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(current["data"]["user_ref"].is_null());
        assert_eq!(current["data"]["level"], 7);
        assert_eq!(current["data"]["active"], true);
        assert_eq!(current["data"]["extensions"]["account"], "vip-user");
        assert_eq!(current["meta"]["platform"], "netease");
        assert_eq!(current["meta"]["account"], "vip-user");

        let (status, defaulted) =
            json_response_from(test_app_with_provider(), "/v1/account/membership").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(defaulted["meta"]["platform"], "netease");
        assert_eq!(defaulted["meta"]["account"], "default");
    }

    #[tokio::test]
    async fn membership_endpoints_reject_bad_references_platforms_and_query_fields() {
        for path in [
            "/v1/users/invalid/membership",
            "/v1/users/netease:32953014/membership?platform=netease",
            "/v1/users/netease:32953014/membership?unknown=true",
            "/v1/account/membership?platform=unknown",
            "/v1/account/membership?unknown=true",
        ] {
            let (status, response) = json_response_from(test_app_with_provider(), path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
            assert_eq!(response["error"]["code"], "invalid_request", "{path}");
        }
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
