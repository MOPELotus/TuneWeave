use super::*;
use crate::client::charts::PAGE_SIZE;
use tuneweave_core::{ChartCatalog, ChartCatalogRequest, ChartTrackListRequest};

fn account(account: Option<&str>) -> Result<()> {
    if account.is_some() {
        return Err(kuwo_invalid_request(
            "Kuwo public charts do not accept an account",
        ));
    }
    Ok(())
}
fn timeout() -> TuneWeaveError {
    TuneWeaveError::new(
        tuneweave_core::ErrorCode::UpstreamTimeout,
        "Kuwo charts exceeded the total time budget",
    )
    .with_platform(Platform::Kuwo)
}
impl KuwoProvider {
    pub(super) async fn read_chart_catalogue(
        &self,
        request: &ChartCatalogRequest,
    ) -> Result<ChartCatalog> {
        self.require_public_scope()?;
        account(request.account.as_deref())?;
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
        request.period.require_current(Platform::Kuwo)?;
        self.require_public_scope()?;
        account(request.account.as_deref())?;
        let id = id.strip_prefix("chart:").unwrap_or(id);
        if id
            .parse::<u64>()
            .ok()
            .is_none_or(|n| n == 0 || n.to_string() != id)
            || !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kuwo_invalid_request(
                "Kuwo chart requires a canonical positive ID and valid pagination",
            ));
        }
        tokio::time::timeout(
            std::time::Duration::from_secs(45),
            self.chart_window(id, request),
        )
        .await
        .map_err(|_| timeout())?
    }
    async fn chart_window(&self, id: &str, request: &ChartTrackListRequest) -> Result<Page<Track>> {
        let mut page = self
            .client
            .chart_tracks_page(id, 1, request.include_tags)
            .await?;
        let total = page.total;
        let publication = page.publication.clone();
        let first = request.offset / PAGE_SIZE + 1;
        let skip = request.offset % PAGE_SIZE;
        let count = (skip + request.limit).div_ceil(PAGE_SIZE);
        let end = u64::from(first + count).min(total.div_ceil(u64::from(PAGE_SIZE)) + 1) as u32;
        let mut remaining = first.max(2)..end;
        let mut number = 1;
        let mut fetched = 0;
        let mut seen = BTreeSet::new();
        let mut items = Vec::with_capacity(request.limit as usize);
        loop {
            if page.total != total || page.publication != publication {
                return Err(kuwo_upstream_error(
                    "Kuwo chart count or publication date changed during pagination",
                ));
            }
            for track in &page.items {
                if !seen.insert(track.id.clone()) {
                    return Err(kuwo_upstream_error("Kuwo chart repeated a track identity"));
                }
            }
            fetched += 1;
            if number >= first {
                items.extend(
                    page.items
                        .into_iter()
                        .skip(if number == first { skip as usize } else { 0 })
                        .take(request.limit as usize - items.len()),
                );
            }
            let Some(next) = remaining.next() else {
                break;
            };
            number = next;
            page = self
                .client
                .chart_tracks_page(id, number, request.include_tags)
                .await?;
        }
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
                extensions: Extensions::from([
                    ("backend".into(), json!("current_web_chart_tracks")),
                    ("chart_id".into(), json!(id)),
                    ("publication_date".into(), json!(publication)),
                    ("period_scope".into(), json!("current")),
                    (
                        "consistency_scope".into(),
                        json!("matching_count_and_publication_date"),
                    ),
                    (
                        "pagination_scope".into(),
                        json!("upstream_catalogue_positions"),
                    ),
                    ("upstream_page_size".into(), json!(PAGE_SIZE)),
                    ("upstream_pages_fetched".into(), json!(fetched)),
                    ("include_tags".into(), json!(request.include_tags)),
                ]),
            },
        })
    }
}

#[cfg(test)]
mod tests;
