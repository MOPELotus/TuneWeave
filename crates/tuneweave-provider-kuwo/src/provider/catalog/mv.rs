use super::*;

impl KuwoProvider {
    pub(super) async fn search_public_mvs(&self, query: &SearchQuery) -> Result<Page<SearchItem>> {
        validate_public_search_options(query)?;
        if query.offset.checked_add(query.limit).is_none() {
            return Err(kuwo_invalid_request(
                "Kuwo MV search window exceeds the unified offset range",
            ));
        }
        tokio::time::timeout(
            std::time::Duration::from_secs(45),
            self.search_mv_window(query),
        )
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                tuneweave_core::ErrorCode::UpstreamTimeout,
                "Kuwo MV search exceeded its total time budget",
            )
            .with_platform(Platform::Kuwo)
        })?
    }

    async fn search_mv_window(&self, query: &SearchQuery) -> Result<Page<SearchItem>> {
        let kind = CatalogKind::Mv;
        let size = kind.page_size();
        // An out-of-range upstream page reports total0. Establish total with
        // page1 and reuse it when the requested window includes that page.
        let mut page = self
            .client
            .search_catalog_page(kind, query.query.trim(), 1)
            .await?;
        let total = page.total;
        let first = query.offset / size + 1;
        let skip = query.offset % size;
        let budget = (skip + query.limit).div_ceil(size);
        let end = u64::from(first + budget).min(total.div_ceil(u64::from(size)) + 1) as u32;
        let mut remaining = first.max(2)..end;
        let mut number = 1;
        let mut fetched = 0;
        let mut seen = BTreeSet::new();
        let mut items = Vec::with_capacity(query.limit as usize);
        loop {
            if page.total != total {
                return Err(kuwo_upstream_error(
                    "Kuwo MV search total changed during pagination",
                ));
            }
            for item in &page.items {
                let SearchItem::Video(video) = item else {
                    unreachable!("typed MV catalogue parser")
                };
                if !seen.insert(video.id.clone()) {
                    return Err(kuwo_upstream_error(
                        "Kuwo MV search repeated a result identity",
                    ));
                }
            }
            fetched += 1;
            if number >= first {
                items.extend(
                    page.items
                        .into_iter()
                        .skip(if number == first { skip as usize } else { 0 })
                        .take(query.limit as usize - items.len()),
                );
            }
            let Some(next) = remaining.next() else {
                break;
            };
            number = next;
            page = self
                .client
                .search_catalog_page(kind, query.query.trim(), number)
                .await?;
        }
        let next = query.offset + items.len() as u32;
        let has_more = u64::from(next) < total;
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: query.limit,
                offset: query.offset,
                total: Some(total),
                has_more,
                next_offset: has_more.then_some(next),
                extensions: Extensions::from([
                    ("backend".into(), json!(kind.operation())),
                    ("catalogue_scope".into(), json!("public")),
                    (
                        "pagination_scope".into(),
                        json!("upstream_catalogue_positions"),
                    ),
                    ("upstream_page_size".into(), json!(size)),
                    ("upstream_pages_fetched".into(), json!(fetched)),
                ]),
            },
        })
    }
}

#[cfg(test)]
mod tests;
