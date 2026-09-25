use super::*;
use crate::client::account_search::{mark, reject_secrets, search_id};

const MAX_ACCOUNT_SEARCH_PAGES: u32 = 128;

impl SodaProvider {
    pub(super) async fn read_account_search(
        &self,
        query: &SearchQuery,
        deadline: std::time::Duration,
    ) -> Result<Page<SearchItem>> {
        let mut plain = query.clone();
        plain.account = None;
        validate_search_options(&plain)?;
        if query.offset + query.limit > MAX_ACCOUNT_SEARCH_PAGES * 20 {
            return Err(soda_invalid_request(
                "Soda account search offset plus limit must not exceed 2560",
            ));
        }
        if query.query.trim().chars().any(char::is_control) {
            return Err(soda_invalid_request(
                "Soda search query cannot contain control characters",
            ));
        }
        if !matches!(
            query.kind,
            SearchKind::Track | SearchKind::Album | SearchKind::Artist | SearchKind::Playlist
        ) {
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "Soda does not support this account search kind",
            )
            .with_platform(Platform::Soda));
        }
        let alias = query.account.as_deref().unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        source.user_id().ok_or_else(soda_authentication_required)?;
        let mut sources = vec![source.clone()];
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let result = tokio::time::timeout(deadline, async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            let verified = verified?;
            self.advance_library_credential(&mut source, &mut stored, verified.credential)?;
            sources.push(source.clone());
            let id = search_id();
            // PC searches may return fewer rows than the cursor increment.
            // Follow cursors from zero and count actual results for unified offset.
            let start = 0;
            let skip = query.offset as usize;
            let needed = skip + query.limit as usize;
            let mut cursor = start;
            let mut buffer = Vec::with_capacity(needed);
            let mut pages = 0;
            let mut more = false;
            let mut next = None;
            let mut budget = 16 * 1024 * 1024;
            while buffer.len() < needed {
                if pages >= MAX_ACCOUNT_SEARCH_PAGES {
                    return Err(soda_upstream_error(
                        "Soda account search exceeded its page budget",
                    ));
                }
                let page = self
                    .client
                    .account_search_page(
                        query.kind,
                        query.query.trim(),
                        cursor,
                        &id,
                        &source,
                        &mut budget,
                    )
                    .await;
                self.ensure_account_snapshot_current(Some(alias), &source)?;
                let (page, updated) = page?;
                self.advance_library_credential(&mut source, &mut stored, updated)?;
                sources.push(source.clone());
                pages += 1;
                more = page.has_more;
                next = page.next_cursor;
                buffer.extend(page.items);
                if !more {
                    break;
                }
                cursor =
                    next.ok_or_else(|| soda_upstream_error("Soda account search lost its cursor"))?;
            }
            let buffered = buffer.len().saturating_sub(skip);
            // Validate even unused tail entries before exposing the requested window.
            reject_secrets(
                &serde_json::to_value(&buffer).map_err(|_| {
                    soda_upstream_error("Soda search metadata serialization failed")
                })?,
                &sources,
            )?;
            let items = buffer
                .into_iter()
                .skip(skip)
                .take(query.limit as usize)
                .collect::<Vec<_>>();
            more = more || buffered > items.len();
            let next_offset =
                (more && !items.is_empty()).then_some(query.offset + items.len() as u32);
            let mut extensions = Extensions::from([
                ("upstream_page_size_limit".into(), json!(20)),
                ("upstream_pages_fetched".into(), json!(pages)),
                ("upstream_cursor_start".into(), json!(start)),
            ]);
            if let Some(next) = next {
                extensions.insert("upstream_next_cursor".into(), json!(next));
            }
            mark(
                &mut extensions,
                source.user_id().ok_or_else(soda_authentication_required)?,
            );
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            Ok(Page {
                items,
                pagination: PageMeta {
                    limit: query.limit,
                    offset: query.offset,
                    total: None,
                    has_more: more,
                    next_offset,
                    extensions,
                },
            })
        })
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Soda account search exceeded its total time budget",
            )
            .with_platform(Platform::Soda)
            .retryable(true)
        })?;
        let page = result?;
        pending.complete();
        Ok(page)
    }
}
