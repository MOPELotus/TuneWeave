mod response;

use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use axum::{
    Json, Router,
    extract::{Path, Query, State, rejection::JsonRejection},
    routing::{get, post},
};
use rand::{RngExt, distr::Alphanumeric};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tuneweave_core::{
    AccountProfile, AuthChallengeRequest, AuthState, Capability, ChallengeMethod, Lyrics,
    MediaStream, PageRequest, PasswordFormat, PasswordLoginRequest, Platform, Playlist,
    PrincipalType, ProviderRegistry, Quality, ResolveRequest, ResourceRef, SearchKind, SearchQuery,
    StreamResolver, Track, TuneWeaveError,
};

pub use response::{ApiError, ApiResponse, ResponseMeta};

const AUTH_TRANSACTION_TTL: Duration = Duration::from_secs(10 * 60);

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
        .route("/tracks/{reference}", get(track))
        .route("/tracks/{reference}/lyrics", get(track_lyrics))
        .route("/tracks/{reference}/stream", get(track_stream))
        .route("/playlists/{reference}", get(playlist))
        .route("/playlists/{reference}/tracks", get(playlist_tracks))
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
        .route("/account/playlists", get(account_playlists));

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
    limit: Option<String>,
    offset: Option<String>,
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
        http::{Method, Request, StatusCode, header},
    };
    use serde_json::Value;
    use tower::ServiceExt;
    use tuneweave_core::{
        ArtistSummary, MusicProvider, Page, PageMeta, ProviderQrStart, Result, SearchQuery,
        StreamRequest,
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
                Capability::TrackDetail,
                Capability::PlaylistRead,
                Capability::Lyrics,
                Capability::AudioStream,
                Capability::QrLogin,
                Capability::PasswordLogin,
                Capability::PhoneLogin,
                Capability::SessionManagement,
                Capability::AccountProfile,
                Capability::AccountPlaylists,
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

        async fn account_playlists(&self, request: &PageRequest) -> Result<Page<Playlist>> {
            Ok(Page {
                items: vec![sample_playlist("3778678")],
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
