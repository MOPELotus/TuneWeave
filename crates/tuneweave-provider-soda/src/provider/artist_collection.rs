use super::*;

const ARTIST_COLLECTION_BUDGET: std::time::Duration = std::time::Duration::from_secs(45);
const ARTIST_COLLECTION_MAX_PAGES: usize = 128;
const ARTIST_COLLECTION_MAX_ITEMS: usize = 10_000;

#[derive(Debug)]
pub(super) struct ArtistCollectionSnapshot {
    pub(super) ids: BTreeSet<String>,
    pub(super) total: u64,
    pub(super) pages: usize,
}

impl SodaProvider {
    pub(super) async fn change_artist_collection(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        self.change_artist_collection_with_budget(id, subscribed, account, ARTIST_COLLECTION_BUDGET)
            .await
    }

    pub(super) async fn change_artist_collection_with_budget(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
        budget: std::time::Duration,
    ) -> Result<SubscriptionResult> {
        super::artist_catalog::validate_identity(id)?;
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        let alias = account.unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let source_user_id = source
            .user_id()
            .ok_or_else(soda_authentication_required)?
            .to_owned();
        let mut dispatched = false;

        let outcome = tokio::time::timeout(budget, async {
            self.verify_collection_source(
                Some(&source_user_id),
                Some(alias),
                &mut source,
                &mut stored,
            )
            .await?;
            let before = self
                .saved_artist_collection_snapshot(Some(alias), &mut source, &mut stored)
                .await?;
            let already_subscribed = before.ids.contains(id);
            let mut confirmed_total = before.total;
            let mut confirmed_pages = before.pages;
            if already_subscribed != subscribed {
                self.ensure_account_snapshot_current(Some(alias), &source)?;
                dispatched = true;
                let updated = self
                    .client
                    .write_account_artist_collection(id, subscribed, &source)
                    .await;
                self.ensure_account_snapshot_current(Some(alias), &source)?;
                self.advance_library_credential(&mut source, &mut stored, updated?)?;

                let after = self
                    .saved_artist_collection_snapshot(Some(alias), &mut source, &mut stored)
                    .await?;
                if after.ids.contains(id) != subscribed {
                    return Err(soda_upstream_error(
                        "Soda complete artist collection readback did not confirm the requested state",
                    ));
                }
                confirmed_total = after.total;
                confirmed_pages = after.pages;
                dispatched = false;
            }
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            if source.user_id() != Some(source_user_id.as_str()) {
                return Err(soda_session_changed());
            }
            Ok(SubscriptionResult {
                resource_ref: tuneweave_core::ResourceRef::new(Platform::Soda, id)
                    .map_err(|_| soda_invalid_request("Soda artist ID is invalid"))?,
                subscribed,
                extensions: Extensions::from([
                    (
                        "backend".to_owned(),
                        json!("official_android_artist_collection"),
                    ),
                    (
                        "verified_by".to_owned(),
                        json!("complete_selected_account_collection_readback"),
                    ),
                    ("source_user_id".to_owned(), json!(source_user_id)),
                    (
                        "write_performed".to_owned(),
                        json!(already_subscribed != subscribed),
                    ),
                    ("complete_collection_count".to_owned(), json!(confirmed_total)),
                    ("upstream_pages_read".to_owned(), json!(confirmed_pages)),
                ]),
            })
        })
        .await;

        let timed_out = outcome.is_err();
        let result = self
            .ensure_account_snapshot_current(Some(alias), &source)
            .and_then(|()| {
                outcome.unwrap_or_else(|_| {
                    Err(TuneWeaveError::new(
                        ErrorCode::UpstreamTimeout,
                        "Soda artist collection exceeded its total time budget",
                    )
                    .with_platform(Platform::Soda)
                    .retryable(true))
                })
            });
        if let Err(error) = &result {
            self.discard_response_credential_after_error(error.code)?;
        }
        if !timed_out {
            pending.complete();
        }
        result.map_err(|error| {
            if dispatched {
                error.retryable(false).with_details(json!({
                    "operation":"artist_subscription",
                    "write_outcome":"unconfirmed",
                    "retry_safe":false,
                    "artist_ref":format!("soda:{id}"),
                }))
            } else {
                error
            }
        })
    }

    pub(super) async fn saved_artist_collection_snapshot(
        &self,
        account: Option<&str>,
        source: &mut SodaCredential,
        stored: &mut Option<StoredAccountCredential>,
    ) -> Result<ArtistCollectionSnapshot> {
        let mut cursor: Option<String> = None;
        let mut requested_cursors = BTreeSet::new();
        let mut artist_ids = BTreeSet::new();
        let mut total_num = None;

        for page_index in 0..ARTIST_COLLECTION_MAX_PAGES {
            self.ensure_account_snapshot_current(account, source)?;
            let page = self
                .client
                .account_artist_collection_page(cursor.as_deref(), source)
                .await;
            self.ensure_account_snapshot_current(account, source)?;
            let page = page?;
            if total_num.is_some_and(|total| total != page.total_num)
                || page.total_num > ARTIST_COLLECTION_MAX_ITEMS as u64
            {
                return Err(soda_upstream_error(
                    "Soda artist collection total changed or exceeded its bounded snapshot",
                ));
            }
            total_num = Some(page.total_num);
            self.advance_library_credential(source, stored, page.credential)?;

            for artist_id in page.artist_ids {
                if !artist_ids.insert(artist_id) {
                    return Err(soda_upstream_error(
                        "Soda artist collection repeated an artist across pages",
                    ));
                }
            }
            if artist_ids.len() > ARTIST_COLLECTION_MAX_ITEMS
                || artist_ids.len() as u64 > page.total_num
            {
                return Err(soda_upstream_error(
                    "Soda artist collection exceeded or contradicted its reported total",
                ));
            }
            if !page.has_more {
                if artist_ids.len() as u64 != page.total_num {
                    return Err(soda_upstream_error(
                        "Soda artist collection ended before its complete reported total",
                    ));
                }
                self.ensure_account_snapshot_current(account, source)?;
                return Ok(ArtistCollectionSnapshot {
                    ids: artist_ids,
                    total: page.total_num,
                    pages: page_index + 1,
                });
            }

            let next = page.next_cursor.ok_or_else(|| {
                soda_upstream_error("Soda artist collection omitted its next cursor")
            })?;
            if !requested_cursors.insert(next.clone()) || cursor.as_deref() == Some(next.as_str()) {
                return Err(soda_upstream_error(
                    "Soda artist collection repeated a continuation cursor",
                ));
            }
            cursor = Some(next);
        }

        Err(soda_upstream_error(
            "Soda artist collection exceeded its page budget",
        ))
    }
}
