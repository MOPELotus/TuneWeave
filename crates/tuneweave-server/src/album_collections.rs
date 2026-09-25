use super::*;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DigitalAlbumLibraryParams {
    platform: Option<String>,
    account: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
}

pub(super) async fn account_digital_albums(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<DigitalAlbumLibraryParams>, QueryRejection>,
) -> Result<Json<ApiResponse<Vec<DigitalAlbum>>>, ApiError> {
    let params = query_params(params)?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let credentials = CallerCredentialSet::from_headers(&headers, &state)?;
    let access = credentials.select_provider(
        &state,
        platform,
        params.account.as_deref(),
        AccountSelection::Default,
    )?;
    let request = page_request(
        params.limit.as_deref(),
        params.offset.as_deref(),
        Some(access.required_account().to_owned()),
    )?;
    let result = access.provider.account_digital_albums(&request).await;
    let (page, credential) = finish_account_operation(
        access.provider.as_ref(),
        platform,
        credentials.credentials.contains_key(&platform),
        result,
    )?;
    Ok(Json(
        access
            .response(page.items, platform)
            .with_pagination(page.pagination)
            .with_caller_credential(credential),
    ))
}

pub(super) async fn user_favorite_digital_albums(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<UserFavoriteAlbumParams>, QueryRejection>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<Vec<DigitalAlbum>>>, ApiError> {
    let params = query_params(params)?;
    let reference = parse_reference(reference)?;
    let platform = reference.platform();
    let account = optional_trimmed(params.account);
    let credentials = CallerCredentialSet::from_headers(&headers, &state)?;
    let access = credentials.select_provider(
        &state,
        platform,
        account.as_deref(),
        AccountSelection::Optional,
    )?;
    let request = page_request(
        params.limit.as_deref(),
        params.offset.as_deref(),
        access.provider_account.clone(),
    )?;
    let result = access
        .provider
        .user_favorite_digital_albums(reference.id(), &request)
        .await;
    let (page, credential) = finish_account_operation(
        access.provider.as_ref(),
        platform,
        credentials.credentials.contains_key(&platform),
        result,
    )?;
    Ok(Json(
        access
            .response(page.items, platform)
            .with_pagination(page.pagination)
            .with_caller_credential(credential),
    ))
}

fn page_request(
    limit: Option<&str>,
    offset: Option<&str>,
    account: Option<String>,
) -> Result<PageRequest, TuneWeaveError> {
    let limit = parse_u32_parameter("limit", limit, 25)?;
    let offset = parse_u32_parameter("offset", offset, 0)?;
    if !(1..=100).contains(&limit) || offset.checked_add(limit).is_none() {
        return Err(TuneWeaveError::invalid_request(
            "album library pagination is invalid",
        ));
    }
    Ok(PageRequest {
        limit,
        offset,
        account,
    })
}

pub(super) async fn digital_albums_subscribe(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<AlbumSubscriptionBatchBody>, JsonRejection>,
) -> Result<Json<ApiResponse<Vec<SubscriptionResult>>>, ApiError> {
    set_album_subscriptions(state, headers, payload, true, true).await
}
pub(super) async fn digital_albums_unsubscribe(
    State(state): State<AppState>,
    headers: HeaderMap,
    payload: Result<Json<AlbumSubscriptionBatchBody>, JsonRejection>,
) -> Result<Json<ApiResponse<Vec<SubscriptionResult>>>, ApiError> {
    set_album_subscriptions(state, headers, payload, false, true).await
}
pub(super) async fn digital_album_subscribe(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(reference): Path<String>,
    params: Result<Query<AlbumSubscriptionParams>, QueryRejection>,
) -> Result<Json<ApiResponse<SubscriptionResult>>, ApiError> {
    set_album_subscription(state, headers, reference, query_params(params)?, true, true).await
}
pub(super) async fn digital_album_unsubscribe(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(reference): Path<String>,
    params: Result<Query<AlbumSubscriptionParams>, QueryRejection>,
) -> Result<Json<ApiResponse<SubscriptionResult>>, ApiError> {
    set_album_subscription(
        state,
        headers,
        reference,
        query_params(params)?,
        false,
        true,
    )
    .await
}
