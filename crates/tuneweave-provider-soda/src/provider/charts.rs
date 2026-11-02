use super::*;
use tuneweave_core::{ChartCatalog, ChartCatalogRequest, ChartGroup, ChartTrackListRequest};

impl SodaProvider {
    fn require_public_chart_source(&self, account: Option<&str>) -> Result<()> {
        if account.is_some() || self.caller_credential.is_some() {
            return Err(soda_invalid_request(
                "Soda public charts do not accept account credentials",
            ));
        }
        Ok(())
    }
    pub(super) async fn read_chart_catalog(
        &self,
        request: &ChartCatalogRequest,
    ) -> Result<ChartCatalog> {
        self.require_public_chart_source(request.account.as_deref())?;
        tokio::time::timeout(std::time::Duration::from_secs(60), async {
            let mut charts = Vec::new();
            for id in crate::client::charts::CHART_IDS {
                charts.push(self.client.chart_snapshot(id).await?.0);
            }
            Ok(ChartCatalog {
                platform: Platform::Soda,
                view: request.view,
                groups: vec![ChartGroup {
                    code: Some("curated".into()),
                    name: "汽水榜单".into(),
                    display_type: None,
                    target_url: None,
                    charts,
                    extensions: Extensions::new(),
                }],
                extensions: Extensions::from([
                    ("catalog_scope".into(), json!("curated_chart_ids")),
                    (
                        "metadata_source".into(),
                        json!("live_official_chart_details"),
                    ),
                ]),
            })
        })
        .await
        .map_err(|_| chart_timeout())?
    }
    pub(super) async fn read_chart_tracks(
        &self,
        id: &str,
        request: &ChartTrackListRequest,
    ) -> Result<Page<Track>> {
        self.require_public_chart_source(request.account.as_deref())?;
        request.period.require_current(Platform::Soda)?;
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(soda_invalid_request(
                "Soda chart requires bounded pagination",
            ));
        }
        let id = id.strip_prefix("chart:").unwrap_or(id);
        let (_, tracks) = tokio::time::timeout(
            std::time::Duration::from_secs(45),
            self.client.chart_snapshot(id),
        )
        .await
        .map_err(|_| chart_timeout())??;
        let total = tracks.len() as u64;
        let items: Vec<_> = tracks
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
                next_offset: has_more.then_some(next),
                has_more,
                extensions: Extensions::from([("complete_snapshot".into(), json!(true))]),
            },
        })
    }
}
fn chart_timeout() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamTimeout,
        "Soda chart exceeded its time budget",
    )
    .with_platform(Platform::Soda)
}
