use super::*;
use tuneweave_core::{PlaylistCatalogKind, PlaylistCatalogRequest, PlaylistCatalogTaxonomyRequest};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Params {
    platform: Option<String>,
    catalog: PlaylistCatalogKind,
    tag_id: Option<String>,
    limit: Option<String>,
    offset: Option<String>,
    account: Option<String>,
}

pub(super) async fn list(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<Params>, QueryRejection>,
) -> Result<Json<ApiResponse<Vec<Playlist>>>, ApiError> {
    let params = query_params(params)?;
    let limit = parse_u32_parameter("limit", params.limit.as_deref(), 20)?;
    let offset = parse_u32_parameter("offset", params.offset.as_deref(), 0)?;
    if !(1..=100).contains(&limit) || offset.checked_add(limit).is_none() {
        return Err(
            TuneWeaveError::invalid_request("playlist catalogue pagination is invalid").into(),
        );
    }
    match (params.catalog, params.tag_id.as_deref()) {
        (PlaylistCatalogKind::Tag, Some(id)) if !id.trim().is_empty() && id.len() <= 64 => {}
        (PlaylistCatalogKind::Tag, _) => {
            return Err(TuneWeaveError::invalid_request(
                "tag catalogues require a non-empty tag_id of at most 64 bytes",
            )
            .into());
        }
        (_, None) => {}
        (_, Some(_)) => {
            return Err(TuneWeaveError::invalid_request(
                "tag_id is only valid for the tag playlist catalogue",
            )
            .into());
        }
    }
    let platform = account_platform(&state, params.platform.as_deref())?;
    let credentials = CallerCredentialSet::from_headers(&headers, &state)?;
    let access = credentials.select_provider(
        &state,
        platform,
        params.account.as_deref(),
        AccountSelection::Optional,
    )?;
    let result = access
        .provider
        .playlist_catalog(&PlaylistCatalogRequest {
            catalog: params.catalog,
            tag_id: params.tag_id,
            limit,
            offset,
            account: access.provider_account.clone(),
        })
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TaxonomyParams {
    platform: Option<String>,
    account: Option<String>,
}

pub(super) async fn taxonomy(
    State(state): State<AppState>,
    headers: HeaderMap,
    params: Result<Query<TaxonomyParams>, QueryRejection>,
) -> Result<Json<ApiResponse<tuneweave_core::PlaylistCatalogTaxonomy>>, ApiError> {
    let params = query_params(params)?;
    let platform = account_platform(&state, params.platform.as_deref())?;
    let credentials = CallerCredentialSet::from_headers(&headers, &state)?;
    let access = credentials.select_provider(
        &state,
        platform,
        params.account.as_deref(),
        AccountSelection::Optional,
    )?;
    let result = access
        .provider
        .playlist_catalog_taxonomy(&PlaylistCatalogTaxonomyRequest {
            account: access.provider_account.clone(),
        })
        .await;
    let (taxonomy, update) = finish_account_operation(
        access.provider.as_ref(),
        platform,
        credentials.credentials.contains_key(&platform),
        result,
    )?;
    Ok(Json(
        access
            .response(taxonomy, platform)
            .with_caller_credential(update),
    ))
}
