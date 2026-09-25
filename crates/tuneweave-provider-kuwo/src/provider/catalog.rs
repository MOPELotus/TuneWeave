use super::*;
use crate::client::catalog::CatalogKind;

mod mv;

impl KuwoProvider {
    pub(super) async fn search_public_catalog(
        &self,
        query: &SearchQuery,
    ) -> Result<Page<SearchItem>> {
        if query.kind == SearchKind::Mv {
            return self.search_public_mvs(query).await;
        }
        let kind = match query.kind {
            SearchKind::Album => CatalogKind::Album,
            SearchKind::Artist => CatalogKind::Artist,
            SearchKind::Playlist => CatalogKind::Playlist,
            _ => {
                return Err(TuneWeaveError::unsupported(
                    Platform::Kuwo,
                    capability_for_search(query.kind),
                ));
            }
        };
        validate_public_search_options(query)?;
        if query.offset.checked_add(query.limit).is_none() {
            return Err(kuwo_invalid_request(
                "Kuwo catalogue window exceeds the unified offset range",
            ));
        }
        let size = kind.page_size();
        let first = query.offset / size + 1;
        let skip = query.offset % size;
        let budget = (skip + query.limit).div_ceil(size);
        let mut items = Vec::with_capacity(query.limit as usize);
        let mut seen = BTreeSet::new();
        let mut total = None;
        let mut fetched = 0;
        for index in 0..budget {
            let page = self
                .client
                .search_catalog_page(kind, query.query.trim(), first + index)
                .await?;
            if total.is_some_and(|total| total != page.total) {
                return Err(kuwo_upstream_error(
                    "Kuwo catalogue total changed during pagination",
                ));
            }
            total = Some(page.total);
            fetched += 1;
            for item in &page.items {
                let id = match item {
                    SearchItem::Album(item) => &item.id,
                    SearchItem::Artist(item) => &item.id,
                    SearchItem::Playlist(item) => &item.id,
                    _ => unreachable!("typed catalogue parser"),
                };
                if !seen.insert(id.clone()) {
                    return Err(kuwo_upstream_error(
                        "Kuwo catalogue repeated a result identity",
                    ));
                }
            }
            let consumed = u64::from(first + index) * u64::from(size);
            let take = query.limit as usize - items.len();
            items.extend(
                page.items
                    .into_iter()
                    .skip(if index == 0 { skip as usize } else { 0 })
                    .take(take),
            );
            if items.len() == query.limit as usize || consumed >= page.total {
                break;
            }
        }
        let total = total.expect("positive validated catalogue window");
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
                    ("backend".to_owned(), json!(kind.operation())),
                    ("upstream_page_size".to_owned(), json!(size)),
                    ("upstream_pages_fetched".to_owned(), json!(fetched)),
                ]),
            },
        })
    }
}
