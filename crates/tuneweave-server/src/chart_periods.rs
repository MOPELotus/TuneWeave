use super::*;
use tuneweave_core::ChartPeriodSummary;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Params {
    #[serde(alias = "num")]
    limit: Option<String>,
    offset: Option<String>,
    page: Option<String>,
    account: Option<String>,
}

pub(super) async fn list(
    State(state): State<AppState>,
    Path(reference): Path<String>,
    Query(params): Query<Params>,
    headers: HeaderMap,
) -> Result<Json<ApiResponse<Vec<ChartPeriodSummary>>>, ApiError> {
    let reference = parse_reference(reference)?;
    let limit = parse_u32_parameter("limit/num", params.limit.as_deref(), 10)?;
    if !(1..=100).contains(&limit) || (params.offset.is_some() && params.page.is_some()) {
        return Err(TuneWeaveError::invalid_request(
            "chart periods require limit 1–100 and either offset or page",
        )
        .into());
    }
    let offset = if let Some(page) = params.page.as_deref() {
        let page = parse_u32_parameter("page", Some(page), 1)?;
        page.checked_sub(1)
            .and_then(|p| p.checked_mul(limit))
            .ok_or_else(|| TuneWeaveError::invalid_request("chart period page is out of range"))?
    } else {
        parse_u32_parameter("offset", params.offset.as_deref(), 0)?
    };
    if offset.checked_add(limit).is_none() {
        return Err(TuneWeaveError::invalid_request("chart period offset is out of range").into());
    }
    let account = optional_trimmed(params.account);
    let platform = reference.platform();
    let access = CallerCredentialSet::from_headers(&headers, &state)?.select_provider(
        &state,
        platform,
        account.as_deref(),
        AccountSelection::Optional,
    )?;
    let page = access
        .provider
        .chart_periods(
            reference.id(),
            &PageRequest {
                limit,
                offset,
                account: access.provider_account.clone(),
            },
        )
        .await?;
    Ok(Json(
        access
            .response(page.items, platform)
            .with_pagination(page.pagination),
    ))
}
