use super::*;

pub(super) struct LibraryPlaylistSnapshot {
    pub(super) items: Vec<Playlist>,
    pub(super) absence_proven: bool,
}

impl SodaProvider {
    pub(super) async fn read_library_playlists(
        &self,
        request: &PageRequest,
        budget: std::time::Duration,
    ) -> Result<Page<Playlist>> {
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        if !(1..=100).contains(&request.limit) {
            return Err(soda_invalid_request(
                "Soda account playlist limit must be between 1 and 100",
            ));
        }
        let alias = request.account.as_deref().unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let outcome = tokio::time::timeout(budget, async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            self.advance_library_credential(&mut source, &mut stored, verified?.credential)?;
            let mut items = Vec::new();
            for section in [LibrarySection::Created, LibrarySection::Saved] {
                items.extend(
                    self.read_library_section(section, &mut source, &mut stored)
                        .await?,
                );
            }
            let total = items.len() as u64;
            let items: Vec<_> = items
                .into_iter()
                .skip(request.offset as usize)
                .take(request.limit as usize)
                .collect();
            let end = u64::from(request.offset) + items.len() as u64;
            let has_more = end < total;
            Ok(Page {
                items,
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(total),
                    has_more,
                    next_offset: has_more.then_some(end as u32),
                    extensions: Extensions::new(),
                },
            })
        })
        .await;
        self.ensure_account_snapshot_current(Some(alias), &source)?;
        let result: Result<_> = outcome.map_err(|_| library_timeout())?;
        // Completed business errors may still deliver an earlier, verified rotation.
        // Session invalidation suppresses it; cancellation and total timeout drop the guard.
        if let Err(error) = &result {
            self.discard_response_credential_after_error(error.code)?;
        }
        pending.complete();
        result
    }

    pub(super) async fn change_playlist_collection(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
        budget: std::time::Duration,
    ) -> Result<SubscriptionResult> {
        let mut pending = PendingCredentialUpdate::new(self.response_credential.clone());
        if id.len() > 64
            || id.starts_with('0')
            || id.is_empty()
            || !id.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(soda_invalid_request(
                "Soda playlist ID must be a canonical positive decimal",
            ));
        }
        let alias = account.unwrap_or("default");
        let (mut source, mut stored) = self
            .selected_credential(alias)?
            .ok_or_else(soda_authentication_required)?;
        let mut dispatched = false;
        let outcome = tokio::time::timeout(budget, async {
            let verified = self.client.account(alias, &source).await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            self.advance_library_credential(&mut source, &mut stored, verified?.credential)?;
            // Once dispatched, neither a missing ACK nor failed readback can prove
            // whether the upstream changed the collection. Never retry the write.
            dispatched = true;
            let refreshed = self
                .client
                .write_playlist_collection(id, subscribed, &source)
                .await;
            self.ensure_account_snapshot_current(Some(alias), &source)?;
            self.advance_library_credential(&mut source, &mut stored, refreshed?)?;
            let saved = self
                .read_library_section_snapshot(LibrarySection::Saved, &mut source, &mut stored)
                .await?;
            if saved.items.iter().any(|playlist| playlist.id == id) != subscribed {
                return Err(soda_upstream_error(
                    "Soda collection readback did not match the requested state",
                ));
            }
            if !subscribed && !saved.absence_proven {
                return Err(soda_upstream_error(
                    "Soda playlist removal lacks a complete counted collection readback",
                ));
            }
            Ok(SubscriptionResult {
                resource_ref: tuneweave_core::ResourceRef::new(Platform::Soda, id)
                    .map_err(|_| soda_invalid_request("Soda playlist ID is invalid"))?,
                subscribed,
                extensions: Extensions::from([
                    (
                        "backend".to_owned(),
                        json!("official_pc_playlist_collection"),
                    ),
                    (
                        "verified_by".to_owned(),
                        json!("complete_saved_library_readback"),
                    ),
                    ("source_user_id".to_owned(), json!(source.user_id())),
                ]),
            })
        })
        .await;
        let timed_out = outcome.is_err();
        let result = self
            .ensure_account_snapshot_current(Some(alias), &source)
            .and_then(|()| outcome.unwrap_or_else(|_| Err(library_timeout())));
        if let Err(error) = &result {
            self.discard_response_credential_after_error(error.code)?;
        }
        if !timed_out {
            pending.complete();
        }
        result.map_err(|error| {
            if dispatched {
                error.retryable(false).with_details(json!({
                    "operation": "playlist_subscription", "write_outcome": "unconfirmed",
                }))
            } else {
                error
            }
        })
    }
}

pub(super) fn library_timeout() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamTimeout,
        "Soda library operation exceeded its total time budget",
    )
    .with_platform(Platform::Soda)
    .retryable(true)
}
