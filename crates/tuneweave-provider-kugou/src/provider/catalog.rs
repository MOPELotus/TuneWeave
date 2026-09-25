use super::*;
use crate::client::catalog::{CatalogKind, PAGE_SIZE, item_id};
use tuneweave_core::SearchItem;

impl KugouProvider {
    pub(super) async fn search_public_catalog(
        &self,
        query: &SearchQuery,
    ) -> Result<Page<SearchItem>> {
        self.require_public_source()?;
        let kind = match query.kind {
            SearchKind::Album => CatalogKind::Album,
            SearchKind::Artist => CatalogKind::Artist,
            SearchKind::Playlist => CatalogKind::Playlist,
            SearchKind::Mv => CatalogKind::Mv,
            _ => {
                return Err(TuneWeaveError::unsupported(
                    Platform::Kugou,
                    capability_for_search(query.kind),
                ));
            }
        };
        validate_search_options(query)?;
        if query.offset.checked_add(query.limit).is_none() {
            return Err(kugou_invalid_request(
                "KuGou catalogue offset exceeds the unified range",
            ));
        }
        let first_page = query.offset / PAGE_SIZE + 1;
        let skip = query.offset % PAGE_SIZE;
        let budget = (skip + query.limit).div_ceil(PAGE_SIZE);
        let mut total = None;
        let mut seen = BTreeSet::new();
        let mut items = Vec::with_capacity(query.limit as usize);
        let mut extensions = Extensions::new();
        let mut fetched = 0;
        for index in 0..budget {
            let page = self
                .client
                .search_catalog_page(kind, query.query.trim(), first_page + index)
                .await?;
            if total.is_some_and(|v| v != page.total)
                || page
                    .items
                    .iter()
                    .any(|item| !seen.insert(item_id(item).to_owned()))
            {
                return Err(TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "KuGou catalogue changed or repeated results while paging",
                )
                .with_platform(Platform::Kugou));
            }
            total = Some(page.total);
            fetched += 1;
            if index == 0 {
                extensions = page.extensions;
            }
            let to_skip = if index == 0 { skip as usize } else { 0 };
            let take = query.limit as usize - items.len();
            items.extend(page.items.into_iter().skip(to_skip).take(take));
            if items.len() == query.limit as usize
                || u64::from(first_page + index) * u64::from(PAGE_SIZE) >= page.total
            {
                break;
            }
        }
        let consumed = query.offset + items.len() as u32;
        let has_more = u64::from(consumed) < total.unwrap_or(0);
        extensions.insert("backend".into(), json!(kind.backend()));
        extensions.insert("upstream_page_size".into(), json!(PAGE_SIZE));
        extensions.insert("upstream_pages_fetched".into(), json!(fetched));
        Ok(Page {
            items,
            pagination: PageMeta {
                offset: query.offset,
                limit: query.limit,
                total,
                next_offset: has_more.then_some(consumed),
                has_more,
                extensions,
            },
        })
    }
}

#[cfg(test)]
mod tests;
