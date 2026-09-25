use super::*;
use tuneweave_core::{ChartCatalog, ChartCatalogRequest, ChartTrackListRequest};

fn timeout() -> TuneWeaveError {
    TuneWeaveError::new(
        tuneweave_core::ErrorCode::UpstreamTimeout,
        "Migu charts exceeded the total time budget",
    )
    .with_platform(Platform::Migu)
}
impl MiguProvider {
    fn validate_chart_source(&self, account: Option<&str>) -> Result<()> {
        self.require_public_source()?;
        if account.is_some() {
            return Err(migu_invalid_request(
                "Migu public charts do not accept an account",
            ));
        }
        Ok(())
    }
    pub(super) async fn read_chart_catalogue(
        &self,
        request: &ChartCatalogRequest,
    ) -> Result<ChartCatalog> {
        self.validate_chart_source(request.account.as_deref())?;
        tokio::time::timeout(
            std::time::Duration::from_secs(45),
            self.client.chart_catalogue(request.view),
        )
        .await
        .map_err(|_| timeout())?
    }
    pub(super) async fn read_chart_tracks(
        &self,
        id: &str,
        request: &ChartTrackListRequest,
    ) -> Result<Page<Track>> {
        self.validate_chart_source(request.account.as_deref())?;
        let id = id.strip_prefix("chart:").unwrap_or(id);
        request
            .period
            .validate()
            .map_err(|e| e.with_platform(Platform::Migu))?;
        if matches!(request.period, tuneweave_core::ChartPeriod::Id { .. }) {
            return Err(TuneWeaveError::unsupported(
                Platform::Migu,
                Capability::ChartHistoricalTracks,
            ));
        }
        if !crate::client::charts::valid_id(id)
            || !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(migu_invalid_request(
                "Migu chart requires a canonical positive ID and valid pagination",
            ));
        }
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(45),
            self.client
                .complete_chart_tracks(id, request.include_tags, &request.period),
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
}

#[cfg(test)]
mod tests;
