use super::*;
use tuneweave_core::{ArtistCatalogFilter, ArtistVideoTag};

#[derive(Debug, Default, Deserialize)]
pub(super) struct AlbumParams {
    pub limit: Option<String>,
    pub offset: Option<String>,
    pub account: Option<String>,
    pub order: Option<String>,
    pub album_type: Option<String>,
}
pub(super) fn parse_filter(
    order: Option<&str>,
    album_type: Option<&str>,
    tag_id: Option<&str>,
    categories: Option<&str>,
) -> Result<ArtistCatalogFilter, ApiError> {
    Ok(ArtistCatalogFilter {
        order: order
            .map(|v| parse_artist_track_order(Some(v)))
            .transpose()?,
        album_type: album_type
            .map(|v| parse_u32_parameter("album_type", Some(v), 0))
            .transpose()?,
        video_tag_id: parse_optional_u64_parameter("tag_id", tag_id)?,
        include_categories: parse_bool_parameter("include_categories", categories, false)?,
    })
}
pub(super) async fn video_tags(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<AccountParams>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<Vec<ArtistVideoTag>>>, ApiError> {
    let reference = parse_reference(reference)?;
    let account = optional_trimmed(params.account);
    let platform = reference.platform();
    let access = CallerCredentialSet::from_headers(&headers, &state)?.select_provider(
        &state,
        platform,
        account.as_deref(),
        AccountSelection::Optional,
    )?;
    let tags = access
        .provider
        .artist_video_tags(reference.id(), access.provider_account.as_deref())
        .await?;
    Ok(Json(access.list_response(tags, platform)))
}
