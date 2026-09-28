use super::*;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PurchaseParams {
    platform: Option<String>,
    account: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
}
fn page_request(params: &PurchaseParams, account: String) -> Result<PageRequest, TuneWeaveError> {
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    if !(1..=100).contains(&limit) || offset.checked_add(limit).is_none() {
        return Err(TuneWeaveError::invalid_request(
            "purchase library pagination is invalid",
        ));
    }
    Ok(PageRequest {
        limit,
        offset,
        account: Some(account),
    })
}
pub(super) async fn tracks(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<PurchaseParams>, QueryRejection>,
) -> Result<Json<ApiResponse<Vec<PurchasedTrack>>>, ApiError> {
    let params = query_params(params)?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let credentials = CallerCredentialSet::from_headers(&headers, &state)?;
    let access = credentials.select_provider(
        &state,
        platform,
        params.account.as_deref(),
        AccountSelection::Default,
    )?;
    if !access.provider.supports(Capability::AccountPurchasedTracks) {
        return Err(
            TuneWeaveError::unsupported(platform, Capability::AccountPurchasedTracks).into(),
        );
    }
    let request = page_request(&params, access.required_account().to_owned())?;
    let result = access.provider.account_purchased_tracks(&request).await;
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
pub(super) async fn albums(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<PurchaseParams>, QueryRejection>,
) -> Result<Json<ApiResponse<Vec<PurchasedAlbum>>>, ApiError> {
    let params = query_params(params)?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let credentials = CallerCredentialSet::from_headers(&headers, &state)?;
    let access = credentials.select_provider(
        &state,
        platform,
        params.account.as_deref(),
        AccountSelection::Default,
    )?;
    if !access.provider.supports(Capability::AccountPurchasedAlbums) {
        return Err(
            TuneWeaveError::unsupported(platform, Capability::AccountPurchasedAlbums).into(),
        );
    }
    let request = page_request(&params, access.required_account().to_owned())?;
    let result = access.provider.account_purchased_albums(&request).await;
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
