use super::*;
use tuneweave_core::{
    ChartCatalog, ChartCatalogRequest, ChartPeriod, ChartPeriodSummary, ChartTrackListRequest,
};

fn timeout() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamTimeout,
        "KuGou chart read exceeded its total time budget",
    )
    .with_platform(Platform::Kugou)
}

fn validate_account(account: Option<&str>) -> Result<()> {
    if account.is_some() {
        return Err(kugou_invalid_request(
            "KuGou public charts do not accept an account",
        ));
    }
    Ok(())
}
fn chart_id(id: &str) -> Result<u64> {
    let id = id.strip_prefix("chart:").unwrap_or(id);
    id.parse::<u64>()
        .ok()
        .filter(|n| *n > 0 && n.to_string() == id)
        .ok_or_else(|| {
            kugou_invalid_request(
                "KuGou chart ID must be a canonical positive decimal or chart:<id>",
            )
        })
}
impl KugouProvider {
    pub(super) async fn read_chart_periods(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<ChartPeriodSummary>> {
        self.require_public_source()?;
        validate_account(request.account.as_deref())?;
        let id = chart_id(id)?;
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kugou_invalid_request(
                "Invalid KuGou chart period pagination",
            ));
        }
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(45),
            self.client.complete_chart_periods(id),
        )
        .await
        .map_err(|_| timeout())??;
        let total = result.items.len() as u64;
        let items: Vec<_> = result
            .items
            .into_iter()
            .skip(request.offset as usize)
            .take(request.limit as usize)
            .collect();
        let next = request.offset + items.len() as u32;
        let has_more = u64::from(next) < total;
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(total),
                has_more,
                next_offset: has_more.then_some(next),
                extensions: result.extensions,
            },
        })
    }
    pub(super) async fn read_chart_catalogue(
        &self,
        request: &ChartCatalogRequest,
    ) -> Result<ChartCatalog> {
        self.require_public_source()?;
        validate_account(request.account.as_deref())?;
        self.client.public_charts(request.view).await
    }
    pub(super) async fn read_chart_tracks(
        &self,
        id: &str,
        request: &ChartTrackListRequest,
    ) -> Result<Page<Track>> {
        request
            .period
            .validate()
            .map_err(|e| e.with_platform(Platform::Kugou))?;
        match &request.period {
            ChartPeriod::Current => {}
            ChartPeriod::Id { id } => {
                crate::client::charts::periods::period_id(id)?;
            }
            _ => {
                return Err(TuneWeaveError::unsupported(
                    Platform::Kugou,
                    Capability::ChartHistoricalTracks,
                ));
            }
        }
        self.require_public_source()?;
        validate_account(request.account.as_deref())?;
        let id = chart_id(id)?;
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kugou_invalid_request("Invalid KuGou chart pagination"));
        }
        let c = tokio::time::timeout(
            std::time::Duration::from_secs(45),
            self.client
                .complete_chart_tracks(id, request.include_tags, &request.period),
        )
        .await
        .map_err(|_| timeout())??;
        let items: Vec<_> = c
            .items
            .into_iter()
            .skip(request.offset as usize)
            .take(request.limit as usize)
            .collect();
        let end = u64::from(request.offset) + items.len() as u64;
        let has_more = end < c.total;
        let mut extensions = Extensions::from([
            ("backend".into(), json!("official_complete_chart_period")),
            ("chart_id".into(), json!(id.to_string())),
            ("chart_name".into(), json!(c.name)),
            ("rank_cid".into(), json!(c.period.to_string())),
            ("complete_snapshot".into(), json!(true)),
            ("upstream_pages_fetched".into(), json!(c.pages)),
            ("include_tags".into(), json!(request.include_tags)),
            ("requested_period".into(), json!(request.period)),
            ("consistency_scope".into(), json!("period_pinned_pages")),
        ]);
        if let Some(selected) = c.selected_period {
            extensions.insert("selected_period".into(), json!(selected));
        }
        if request.include_tags && !c.tags.is_empty() {
            extensions.insert("chart_tags".into(), json!(c.tags));
        }
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(c.total),
                has_more,
                next_offset: has_more.then_some(end as u32),
                extensions,
            },
        })
    }
}

#[cfg(test)]
mod tests;
