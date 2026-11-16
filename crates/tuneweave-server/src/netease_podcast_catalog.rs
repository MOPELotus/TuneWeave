use super::*;
use tuneweave_core::{
    CollectedPodcastFeedPage, CollectedPodcastFeedRequest, PodcastCategoryTree,
    RecommendationDislikeList,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AccountQuery {
    platform: Option<String>,
    account: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FeedQuery {
    platform: Option<String>,
    account: Option<String>,
    cursor: Option<String>,
    limit: Option<u32>,
}

fn select_access(
    state: &AppState,
    headers: &HeaderMap,
    platform: Option<&str>,
    account: Option<&str>,
    selection: AccountSelection,
) -> Result<(Platform, ProviderAccess, bool), ApiError> {
    let platform = platform.map_or(Ok(Platform::Netease), parse_platform_parameter)?;
    let credentials = CallerCredentialSet::from_headers(headers, state)?;
    let access = credentials.select_provider(state, platform, account, selection)?;
    Ok((
        platform,
        access,
        credentials.credentials.contains_key(&platform),
    ))
}

fn account_response<T>(
    access: &ProviderAccess,
    platform: Platform,
    caller_managed: bool,
    result: tuneweave_core::Result<T>,
) -> Result<Json<ApiResponse<T>>, ApiError> {
    let (value, update) =
        finish_account_operation(access.provider.as_ref(), platform, caller_managed, result)?;
    Ok(Json(
        access
            .response(value, platform)
            .with_caller_credential(update),
    ))
}

pub(super) async fn category_tree(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<AccountQuery>, QueryRejection>,
) -> Result<Json<ApiResponse<PodcastCategoryTree>>, ApiError> {
    let params = query_params(params)?;
    let (platform, access, caller_managed) = select_access(
        &state,
        &headers,
        params.platform.as_deref(),
        params.account.as_deref(),
        AccountSelection::Optional,
    )?;
    if access.provider_account.is_some() {
        caller_scope::mark_sensitive();
    }
    let result = access
        .provider
        .podcast_category_tree(access.provider_account.as_deref())
        .await;
    account_response(&access, platform, caller_managed, result)
}

pub(super) async fn recommendation_dislikes(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<AccountQuery>, QueryRejection>,
) -> Result<Json<ApiResponse<RecommendationDislikeList>>, ApiError> {
    caller_scope::mark_sensitive();
    let params = query_params(params)?;
    let (platform, access, caller_managed) = select_access(
        &state,
        &headers,
        params.platform.as_deref(),
        params.account.as_deref(),
        AccountSelection::Default,
    )?;
    let result = access
        .provider
        .recommendation_dislikes(Some(access.required_account()))
        .await;
    account_response(&access, platform, caller_managed, result)
}

pub(super) async fn collected_feed(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<FeedQuery>, QueryRejection>,
) -> Result<Json<ApiResponse<CollectedPodcastFeedPage>>, ApiError> {
    caller_scope::mark_sensitive();
    let params = query_params(params)?;
    let limit = params.limit.unwrap_or(30);
    if !(1..=100).contains(&limit) {
        return Err(TuneWeaveError::invalid_request("limit must be between 1 and 100").into());
    }
    if params.cursor.as_deref().is_some_and(|cursor| {
        cursor.starts_with('0')
            || !cursor.bytes().all(|byte| byte.is_ascii_digit())
            || cursor.parse::<i64>().is_err()
    }) {
        return Err(TuneWeaveError::invalid_request(
            "cursor must be a positive decimal value returned by the previous page",
        )
        .into());
    }
    let (platform, access, caller_managed) = select_access(
        &state,
        &headers,
        params.platform.as_deref(),
        params.account.as_deref(),
        AccountSelection::Default,
    )?;
    let result = access
        .provider
        .collected_podcast_feed(&CollectedPodcastFeedRequest {
            account: Some(access.required_account().to_owned()),
            cursor: params.cursor,
            limit,
        })
        .await;
    account_response(&access, platform, caller_managed, result)
}
