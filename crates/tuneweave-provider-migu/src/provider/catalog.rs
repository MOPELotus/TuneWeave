use super::*;
use crate::client::catalog::{CatalogKind, PAGE_SIZE};

impl MiguProvider {
    pub(super) async fn search_public_catalog(
        &self,
        query: &SearchQuery,
    ) -> Result<Page<SearchItem>> {
        let kind = match query.kind {
            SearchKind::Playlist => CatalogKind::Playlist,
            SearchKind::Artist => CatalogKind::Artist,
            SearchKind::Album => CatalogKind::Album,
            _ => {
                return Err(TuneWeaveError::unsupported(
                    Platform::Migu,
                    capability_for_search(query.kind),
                ));
            }
        };
        validate_public_search_options(query)?;
        if query.offset.checked_add(query.limit).is_none() {
            return Err(migu_invalid_request(
                "Migu catalogue offset exceeds the unified range",
            ));
        }
        let start_page = query.offset / PAGE_SIZE as u32 + 1;
        let first_skip = query.offset as usize % PAGE_SIZE;
        let requested = query.limit as usize;
        let budget = (first_skip + requested).div_ceil(PAGE_SIZE);
        let mut items = Vec::with_capacity(requested);
        let mut seen_pages = BTreeSet::new();
        let mut sequences = Vec::new();
        let mut conditions = Vec::new();
        let mut fetched = 0;
        let mut has_more = false;
        for page_index in 0..budget {
            let page = self
                .client
                .search_catalog_page(kind, query.query.trim(), start_page + page_index as u32)
                .await?;
            fetched += 1;
            let signature: Vec<_> = page
                .items
                .iter()
                .map(|item| match item {
                    SearchItem::Playlist(item) => ("playlist", item.id.clone()),
                    SearchItem::Artist(item) => ("artist", item.id.clone()),
                    SearchItem::Album(item) => ("album", item.id.clone()),
                    SearchItem::DigitalAlbum(item) => ("digital_album", item.id.clone()),
                    _ => unreachable!("typed catalogue parser"),
                })
                .collect();
            if !signature.is_empty() && !seen_pages.insert(signature) {
                return Err(migu_upstream_error("Migu catalogue repeated a result page"));
            }
            if let Some(sequence) = page.sequence {
                sequences.push(sequence);
            }
            if conditions.is_empty() {
                conditions = page.conditions;
            }
            let skip = if page_index == 0 { first_skip } else { 0 };
            let available = page.items.len().saturating_sub(skip);
            let take = available.min(requested - items.len());
            has_more = available > take || page.has_next;
            items.extend(page.items.into_iter().skip(skip).take(take));
            if items.len() == requested || !page.has_next {
                break;
            }
        }
        let returned = items.len() as u32;
        let mut extensions = Extensions::from([
            ("backend".to_owned(), json!(kind.backend())),
            ("upstream_page_size".to_owned(), json!(PAGE_SIZE)),
            ("upstream_pages_fetched".to_owned(), json!(fetched)),
        ]);
        if !sequences.is_empty() {
            extensions.insert("upstream_sequences".to_owned(), json!(sequences));
        }
        if !conditions.is_empty() {
            extensions.insert("conditions".to_owned(), json!(conditions));
        }
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: query.limit,
                offset: query.offset,
                total: None,
                has_more,
                next_offset: (has_more && returned > 0).then_some(query.offset + returned),
                extensions,
            },
        })
    }
}

#[cfg(test)]
pub(super) mod tests;
