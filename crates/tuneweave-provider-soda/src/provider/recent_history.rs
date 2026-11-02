use super::*;
use tuneweave_core::RecentTrackHistoryEntry;

impl SodaProvider {
    pub(super) async fn read_recent_history(
        &self,
        request: &PageRequest,
    ) -> Result<Page<RecentTrackHistoryEntry>> {
        let end = request
            .offset
            .checked_add(request.limit)
            .filter(|end| *end <= 6_400)
            .filter(|_| (1..=100).contains(&request.limit))
            .ok_or_else(|| {
                soda_invalid_request(
                    "Soda recent history requires limit=1..100 and offset+limit<=6400",
                )
            })?;
        let alias = request.account.as_deref().unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let result = tokio::time::timeout(std::time::Duration::from_secs(45), async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            self.advance_library_credential(&mut source, &mut stored, verified?.credential)?;
            let mut cursor = String::new();
            let mut seen = BTreeSet::new();
            let mut rows = Vec::new();
            let mut total = None;
            for _ in 0..64 {
                if !seen.insert(cursor.clone()) {
                    return Err(soda_upstream_error(
                        "Soda recent history repeated its cursor",
                    ));
                }
                let page = self.client.recent_history_page(&cursor, 100, &source).await;
                self.ensure_account_snapshot_current(Some(alias), &source)?;
                let page = page?;
                if total.is_some_and(|total| total != page.total) {
                    return Err(soda_upstream_error(
                        "Soda recent history changed while paging",
                    ));
                }
                total = Some(page.total);
                self.advance_library_credential(&mut source, &mut stored, page.credential)?;
                rows.extend(page.items);
                if (page.next_cursor.is_some() && rows.len() as u64 >= page.total)
                    || rows.len() as u64 > page.total
                    || (page.next_cursor.is_none() && rows.len() as u64 != page.total)
                {
                    return Err(soda_upstream_error(
                        "Soda recent history returned an incomplete or inconsistent total",
                    ));
                }
                let finished = page.next_cursor.is_none();
                if rows.len() >= end as usize || finished {
                    let items: Vec<_> = rows
                        .into_iter()
                        .skip(request.offset as usize)
                        .take(request.limit as usize)
                        .collect();
                    let next = request.offset + items.len() as u32;
                    let has_more = u64::from(next) < page.total;
                    self.ensure_account_snapshot_current(Some(alias), &source)?;
                    return Ok(Page {
                        items,
                        pagination: PageMeta {
                            limit: request.limit,
                            offset: request.offset,
                            total,
                            has_more,
                            next_offset: has_more.then_some(next),
                            extensions: Extensions::from([
                                ("backend".into(), json!("official_pc_recently_played")),
                                ("pagination_source".into(), json!("native_cursor_traversal")),
                            ]),
                        },
                    });
                }
                cursor = page
                    .next_cursor
                    .ok_or_else(|| soda_upstream_error("Soda recent history cursor missing"))?;
            }
            Err(soda_upstream_error(
                "Soda recent history exceeded its page budget",
            ))
        })
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Soda recent history exceeded its time budget",
            )
            .with_platform(Platform::Soda)
        })?;
        let page = result?;
        pending.complete();
        Ok(page)
    }
}
