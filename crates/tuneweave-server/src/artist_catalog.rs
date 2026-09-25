use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Params {
    limit: Option<String>,
    offset: Option<String>,
    account: Option<String>,
}

pub(super) async fn digital_albums(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    headers: HeaderMap,
    params: Result<Query<Params>, QueryRejection>,
) -> Result<Json<ApiResponse<Vec<DigitalAlbum>>>, ApiError> {
    let params = query_params(params)?;
    let reference = parse_reference(reference)?;
    let platform = reference.platform();
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    if !(1..=100).contains(&limit) || offset.checked_add(limit).is_none() {
        return Err(TuneWeaveError::invalid_request("artist album pagination is invalid").into());
    }
    let credentials = CallerCredentialSet::from_headers(&headers, &state)?;
    let access = credentials.select_provider(
        &state,
        platform,
        params.account.as_deref(),
        AccountSelection::Optional,
    )?;
    let result = access
        .provider
        .artist_digital_albums(
            reference.id(),
            &PageRequest {
                limit,
                offset,
                account: access.provider_account.clone(),
            },
        )
        .await;
    let (page, update) = finish_account_operation(
        access.provider.as_ref(),
        platform,
        credentials.credentials.contains_key(&platform),
        result,
    )?;
    Ok(Json(
        access
            .response(page.items, platform)
            .with_pagination(page.pagination)
            .with_caller_credential(update),
    ))
}
