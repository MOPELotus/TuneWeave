//! Normalized direct-upload steps; audio bytes remain between the caller and storage.
use super::*;
use tuneweave_core::{
    CloudUploadPublishMetadata, CloudUploadStepResponse, CloudUploadStrategy, CloudUploadTransfer,
    CloudUploadTransferRequest,
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StartBody {
    file: CloudUploadTicketBody,
    #[serde(default)]
    strategy: CloudUploadStrategy,
}

pub(super) async fn start(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<CloudAccountQuery>,
    payload: Result<Json<StartBody>, JsonRejection>,
) -> Result<Json<ApiResponse<CloudUploadTransfer>>, ApiError> {
    let body = json_body(payload)?;
    if body.file.file_size == 0 || body.file.bitrate == Some(0) {
        return Err(TuneWeaveError::invalid_request(
            "file_size and bitrate must be greater than zero",
        )
        .into());
    }
    let platform = account_platform(&state, params.platform.as_deref())?;
    let access = CallerCredentialSet::from_headers(&headers, &state)?.select_provider(
        &state,
        platform,
        params.account.as_deref(),
        AccountSelection::Default,
    )?;
    let result = access
        .provider
        .begin_cloud_upload_transfer(&CloudUploadTransferRequest {
            file: CloudUploadTicketRequest {
                md5: body.file.md5,
                file_size: body.file.file_size,
                filename: body.file.filename,
                bitrate: body
                    .file
                    .bitrate
                    .unwrap_or(CloudUploadRequest::DEFAULT_BITRATE),
                content_type: optional_trimmed(body.file.content_type),
                account: Some(access.required_account().into()),
            },
            strategy: body.strategy,
        })
        .await?;
    Ok(Json(access.response(result, platform)))
}

pub(super) async fn read(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<CloudAccountQuery>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<CloudUploadTransfer>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let access = CallerCredentialSet::from_headers(&headers, &state)?.select_provider(
        &state,
        platform,
        params.account.as_deref(),
        AccountSelection::Default,
    )?;
    let result = access
        .provider
        .cloud_upload_transfer(&id, Some(access.required_account()))
        .await?;
    Ok(Json(access.response(result, platform)))
}

pub(super) async fn advance(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<CloudAccountQuery>,
    Path(id): Path<String>,
    payload: Result<Json<CloudUploadStepResponse>, JsonRejection>,
) -> Result<Json<ApiResponse<CloudUploadTransfer>>, ApiError> {
    let body = json_body(payload)?;
    body.validate()?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let access = CallerCredentialSet::from_headers(&headers, &state)?.select_provider(
        &state,
        platform,
        params.account.as_deref(),
        AccountSelection::Default,
    )?;
    let result = access
        .provider
        .advance_cloud_upload_transfer(&id, &body, Some(access.required_account()))
        .await?;
    Ok(Json(access.response(result, platform)))
}

pub(super) async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<CloudAccountQuery>,
    Path(id): Path<String>,
) -> Result<Json<ApiResponse<Value>>, ApiError> {
    let platform = account_platform(&state, params.platform.as_deref())?;
    let access = CallerCredentialSet::from_headers(&headers, &state)?.select_provider(
        &state,
        platform,
        params.account.as_deref(),
        AccountSelection::Default,
    )?;
    let cancelled = access
        .provider
        .cancel_cloud_upload_transfer(&id, Some(access.required_account()))
        .await?;
    Ok(Json(
        access.response(json!({"cancelled":cancelled}), platform),
    ))
}

pub(super) async fn publish(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<CloudAccountQuery>,
    Path(id): Path<String>,
    payload: Result<Json<CloudUploadPublishMetadata>, JsonRejection>,
) -> Result<Json<ApiResponse<CloudUploadResult>>, ApiError> {
    let body = json_body(payload)?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let access = CallerCredentialSet::from_headers(&headers, &state)?.select_provider(
        &state,
        platform,
        params.account.as_deref(),
        AccountSelection::Default,
    )?;
    let result = access
        .provider
        .publish_cloud_upload_transfer(&id, &body, Some(access.required_account()))
        .await?;
    Ok(Json(access.response(result, platform)))
}
