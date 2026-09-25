use super::*;

const COLLECTION_BUDGET: std::time::Duration = std::time::Duration::from_secs(45);

pub(super) struct AlbumCollectionSnapshot {
    pub(super) albums: Vec<Album>,
    raw_total: Option<u64>,
    pub(super) absence_proven: bool,
}

impl SodaProvider {
    pub(super) async fn saved_album_snapshot(
        &self,
        account: Option<&str>,
        source: &mut SodaCredential,
        stored: &mut Option<StoredAccountCredential>,
    ) -> Result<AlbumCollectionSnapshot> {
        let mut albums = Vec::new();
        let mut pagination = LibraryPagination::default();
        let mut cursor = "0".to_owned();
        loop {
            let page = self.client.album_library_page(&cursor, source).await;
            self.ensure_account_snapshot_current(account, source)?;
            let page = page?;
            let next = pagination.accept(&cursor, &page)?;
            self.advance_library_credential(source, stored, page.credential)?;
            albums.extend(page.items);
            let Some(next) = next else { break };
            cursor = next;
        }
        self.ensure_account_snapshot_current(account, source)?;
        Ok(AlbumCollectionSnapshot {
            albums,
            raw_total: pagination.raw_total(),
            absence_proven: pagination.absence_is_proven(),
        })
    }

    pub(super) async fn read_album_collections(
        &self,
        requested_user: Option<&str>,
        request: &PageRequest,
    ) -> Result<Page<Album>> {
        self.read_album_collections_with_budget(requested_user, request, COLLECTION_BUDGET)
            .await
    }

    pub(super) async fn read_album_collections_with_budget(
        &self,
        requested_user: Option<&str>,
        request: &PageRequest,
        budget: std::time::Duration,
    ) -> Result<Page<Album>> {
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        if !(1..=100).contains(&request.limit) {
            return Err(soda_invalid_request(
                "Soda album collection limit must be between 1 and 100",
            ));
        }
        let account = request.account.as_deref();
        let (mut source, mut stored) = self.collection_source(requested_user, account)?;
        let outcome = tokio::time::timeout(budget, async {
            self.verify_collection_source(requested_user, account, &mut source, &mut stored)
                .await?;
            self.saved_album_snapshot(account, &mut source, &mut stored)
                .await
        })
        .await;
        self.ensure_account_snapshot_current(account, &source)?;
        let result = outcome.map_err(|_| library_operations::library_timeout())?;
        if let Err(error) = &result {
            self.discard_response_credential_after_error(error.code)?;
        }
        pending.complete();
        let snapshot = result?;
        let total = snapshot.albums.len() as u64;
        let items: Vec<_> = snapshot
            .albums
            .into_iter()
            .skip(request.offset as usize)
            .take(request.limit as usize)
            .collect();
        let end = u64::from(request.offset) + items.len() as u64;
        let has_more = end < total;
        let mut extensions = Extensions::from([
            (
                "backend".to_owned(),
                json!("official_pc_mixed_album_collection"),
            ),
            ("complete_snapshot".to_owned(), json!(true)),
            ("source_user_id".to_owned(), json!(source.user_id())),
        ]);
        if let Some(raw_total) = snapshot.raw_total {
            extensions.insert("upstream_raw_collection_count".to_owned(), json!(raw_total));
        }
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(total),
                has_more,
                next_offset: has_more.then_some(end as u32),
                extensions,
            },
        })
    }

    pub(super) async fn change_album_collection(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        self.change_album_collection_with_budget(id, subscribed, account, COLLECTION_BUDGET)
            .await
    }

    pub(super) async fn change_album_collection_with_budget(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
        budget: std::time::Duration,
    ) -> Result<SubscriptionResult> {
        self.change_album_collections_with_budget(&[id.to_owned()], subscribed, account, budget)
            .await
            .and_then(|results| {
                results.into_iter().next().ok_or_else(|| {
                    soda_upstream_error("Soda album collection omitted its confirmed result")
                })
            })
            .map_err(|error| {
                if error
                    .details
                    .get("operation")
                    .and_then(serde_json::Value::as_str)
                    == Some("album_subscription_batch")
                {
                    error.with_details(
                        json!({"operation":"album_subscription", "write_outcome":"unconfirmed"}),
                    )
                } else {
                    error
                }
            })
    }

    pub(super) async fn change_album_collections(
        &self,
        ids: &[String],
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<Vec<SubscriptionResult>> {
        self.change_album_collections_with_budget(ids, subscribed, account, COLLECTION_BUDGET)
            .await
    }

    pub(super) async fn change_album_collections_with_budget(
        &self,
        ids: &[String],
        subscribed: bool,
        account: Option<&str>,
        budget: std::time::Duration,
    ) -> Result<Vec<SubscriptionResult>> {
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        if ids.is_empty() || ids.len() > 100 {
            return Err(soda_invalid_request(
                "Soda album collection batches must contain 1 to 100 IDs",
            ));
        }
        for id in ids {
            parse_album_id(id)?;
        }
        let (mut source, mut stored) = self.collection_source(None, account)?;
        let mut results: Vec<SubscriptionResult> = Vec::with_capacity(ids.len());
        let mut dispatched = false;
        let outcome = tokio::time::timeout(budget, async {
            self.verify_collection_source(None, account, &mut source, &mut stored)
                .await?;
            for id in ids {
                self.ensure_account_snapshot_current(account, &source)?;
                dispatched = true;
                let result = self
                    .write_album_and_verify(id, subscribed, account, &mut source, &mut stored)
                    .await?;
                results.push(result);
                dispatched = false;
            }
            Ok(())
        })
        .await;
        let timed_out = outcome.is_err();
        let result = self
            .ensure_account_snapshot_current(account, &source)
            .and_then(|()| outcome.unwrap_or_else(|_| Err(library_operations::library_timeout())));
        if let Err(error) = &result {
            self.discard_response_credential_after_error(error.code)?;
        }
        if !timed_out {
            pending.complete();
        }
        result.map_err(|error| {
            if dispatched || !results.is_empty() {
                error.retryable(false).with_details(json!({
                    "operation":"album_subscription_batch", "atomic":false, "write_outcome":"unconfirmed",
                    "completed_refs":results.iter().map(|result| &result.resource_ref).collect::<Vec<_>>(),
                    "failed_ref":ids.get(results.len()).map(|id| format!("soda:{id}")),
                    "remaining_refs":ids.iter().skip(results.len()+1).map(|id| format!("soda:{id}")).collect::<Vec<_>>(),
                }))
            } else { error }
        })?;
        Ok(results)
    }

    async fn write_album_and_verify(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
        source: &mut SodaCredential,
        stored: &mut Option<StoredAccountCredential>,
    ) -> Result<SubscriptionResult> {
        let updated = self
            .client
            .write_album_collection(id, subscribed, source)
            .await;
        self.ensure_account_snapshot_current(account, source)?;
        self.advance_library_credential(source, stored, updated?)?;
        let snapshot = self.saved_album_snapshot(account, source, stored).await?;
        if snapshot.albums.iter().any(|album| album.id == id) != subscribed {
            return Err(soda_upstream_error(
                "Soda album collection readback did not match the requested state",
            ));
        }
        if !subscribed && !snapshot.absence_proven {
            return Err(soda_upstream_error(
                "Soda album removal lacks a complete counted collection readback",
            ));
        }
        Ok(SubscriptionResult {
            resource_ref: tuneweave_core::ResourceRef::new(Platform::Soda, id)
                .map_err(|_| soda_invalid_request("Soda album ID is invalid"))?,
            subscribed,
            extensions: Extensions::from([
                ("backend".to_owned(), json!("official_pc_album_collection")),
                (
                    "verified_by".to_owned(),
                    json!("complete_mixed_collection_readback"),
                ),
                ("source_user_id".to_owned(), json!(source.user_id())),
            ]),
        })
    }
}
