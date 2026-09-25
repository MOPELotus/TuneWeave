use super::*;
use tuneweave_core::PlaylistSubmission;

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SubmitParams {}

pub(super) async fn submit(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<SubmitParams>, QueryRejection>,
    headers: HeaderMap,
    payload: Result<Json<tuneweave_core::PlaylistSubmissionRequest>, JsonRejection>,
) -> Result<Json<ApiResponse<tuneweave_core::PlaylistSubmissionResult>>, ApiError> {
    let reference = parse_reference(reference)?;
    query_params(params)?;
    let mut request = json_body(payload)?;
    let platform = reference.platform();
    let credentials = CallerCredentialSet::from_headers(&headers, &state)?;
    let access = credentials.select_provider(
        &state,
        platform,
        request.account.as_deref(),
        AccountSelection::Default,
    )?;
    request.account = Some(access.required_account().to_owned());
    let result = access
        .provider
        .submit_playlist(reference.id(), &request)
        .await;
    let (result, credential) = finish_account_operation(
        access.provider.as_ref(),
        platform,
        credentials.credentials.contains_key(&platform),
        result,
    )?;
    Ok(Json(
        access
            .response(result, platform)
            .with_caller_credential(credential),
    ))
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Params {
    platform: Option<String>,
    account: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
}

pub(super) async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<Params>, QueryRejection>,
) -> Result<Json<ApiResponse<Vec<PlaylistSubmission>>>, ApiError> {
    let params = query_params(params)?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 30)?;
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    if !(1..=100).contains(&limit) || offset.checked_add(limit).is_none() {
        return Err(TuneWeaveError::invalid_request("submission pagination is invalid").into());
    }
    let platform = account_platform(&state, params.platform.as_deref())?;
    let credentials = CallerCredentialSet::from_headers(&headers, &state)?;
    let access = credentials.select_provider(
        &state,
        platform,
        params.account.as_deref(),
        AccountSelection::Default,
    )?;
    let request = PageRequest {
        limit,
        offset,
        account: Some(access.required_account().to_owned()),
    };
    let result = access.provider.account_playlist_submissions(&request).await;
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

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DeleteParams {
    account: Option<String>,
}

pub(super) async fn delete_records(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    params: Result<Query<DeleteParams>, QueryRejection>,
    headers: HeaderMap,
    body: Result<axum::body::Bytes, axum::extract::rejection::BytesRejection>,
) -> Result<Json<ApiResponse<tuneweave_core::PlaylistSubmissionRecordDeleteResult>>, ApiError> {
    if !body
        .map_err(|_| {
            TuneWeaveError::invalid_request(
                "Submission record deletion does not accept a request body",
            )
        })?
        .is_empty()
    {
        return Err(TuneWeaveError::invalid_request(
            "Submission record deletion does not accept a request body",
        )
        .into());
    }
    let reference = parse_reference(reference)?;
    let params = query_params(params)?;
    let platform = reference.platform();
    let credentials = CallerCredentialSet::from_headers(&headers, &state)?;
    let access = credentials.select_provider(
        &state,
        platform,
        params.account.as_deref(),
        AccountSelection::Default,
    )?;
    let request = tuneweave_core::PlaylistSubmissionRecordDeleteRequest {
        account: Some(access.required_account().to_owned()),
    };
    let result = access
        .provider
        .delete_playlist_submission_records(reference.id(), &request)
        .await;
    let (result, credential) = finish_account_operation(
        access.provider.as_ref(),
        platform,
        credentials.credentials.contains_key(&platform),
        result,
    )?;
    Ok(Json(
        access
            .response(result, platform)
            .with_caller_credential(credential),
    ))
}
