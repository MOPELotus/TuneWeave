use super::*;
use crate::client::{
    album_collections::{CollectedAlbum, CollectionPage, MAX_PAGES, PAGE_SIZE},
    albums::AlbumKind,
};
use crate::credential::{MiguCredential, authentication_required, error, validate_uid};
use tuneweave_core::{ErrorCode, StoredAccountCredential, SubscriptionResult};

mod source;

#[derive(Default)]
struct CollectionScan {
    items: Vec<CollectedAlbum>,
    seen: BTreeSet<(&'static str, String)>,
    expected_total: Option<Option<u64>>,
}
impl CollectionScan {
    fn push(&mut self, response: CollectionPage) -> Result<bool> {
        if let Some(expected) = self.expected_total {
            if expected != response.total {
                return Err(migu_upstream_error(
                    "Migu album collection total changed between pages",
                ));
            }
        } else {
            self.expected_total = Some(response.total);
        }
        let count = response.items.len();
        for item in &response.items {
            let (kind, id) = item.identity();
            if !self.seen.insert((kind, id.to_owned())) {
                return Err(migu_upstream_error(
                    "Migu album collections repeated an identity within the same resource type",
                ));
            }
        }
        self.items.extend(response.items);
        let consumed = self.items.len() as u64;
        if response
            .total
            .is_some_and(|total| consumed > total || consumed < total && count != PAGE_SIZE)
        {
            return Err(migu_upstream_error(
                "Migu album collection page disagrees with its total",
            ));
        }
        let finished = response
            .total
            .map_or(count < PAGE_SIZE, |total| total == consumed);
        if let Some(next) = response.has_next {
            if next && count != PAGE_SIZE || response.total.is_some() && next == finished {
                return Err(migu_upstream_error(
                    "Migu album collection continuation is inconsistent",
                ));
            }
            if !next {
                return Ok(true);
            }
        }
        Ok(finished)
    }
}

struct CollectionSession {
    alias: String,
    original: MiguCredential,
    current: MiguCredential,
    stored: Option<StoredAccountCredential>,
    any_write: bool,
}
struct Snapshot {
    items: Vec<CollectedAlbum>,
}
impl Snapshot {
    fn contains(&self, kind: AlbumKind, id: &str) -> bool {
        self.items
            .iter()
            .any(|item| item.identity() == (kind.resource_type(), id))
    }
    fn page(self, kind: AlbumKind, request: &PageRequest, uid: &str) -> Page<CollectedAlbum> {
        let raw_count = self.items.len();
        let typed: Vec<_> = self
            .items
            .into_iter()
            .filter(|item| item.identity().0 == kind.resource_type())
            .collect();
        let total = typed.len() as u64;
        let items: Vec<_> = typed
            .into_iter()
            .skip(request.offset as usize)
            .take(request.limit as usize)
            .collect();
        let end = u64::from(request.offset) + items.len() as u64;
        Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(total),
                has_more: end < total,
                next_offset: (end < total).then_some(end as u32),
                extensions: Extensions::from([
                    ("backend".into(), json!("official_pc_album_collections")),
                    ("complete_read".into(), json!(true)),
                    ("source_user_id".into(), json!(uid)),
                    ("resource_type".into(), json!(kind.resource_type())),
                    ("upstream_raw_collection_count".into(), json!(raw_count)),
                ]),
            },
        }
    }
}
impl MiguProvider {
    fn collection_session(
        &self,
        account: Option<&str>,
        uid: Option<&str>,
    ) -> Result<CollectionSession> {
        if let Some(uid) = uid {
            validate_uid(uid)?;
        }
        let alias = account.unwrap_or("default");
        let (current, stored) = self.selected(alias)?.ok_or_else(authentication_required)?;
        if uid.is_some_and(|uid| uid != current.user_id()) {
            return Err(error(
                ErrorCode::PermissionDenied,
                "Migu album collections are available only for the selected account",
            ));
        }
        Ok(CollectionSession {
            alias: alias.into(),
            original: current.clone(),
            current,
            stored,
            any_write: false,
        })
    }
    fn finish_album_collection<T>(&self, s: &CollectionSession, result: Result<T>) -> Result<T> {
        self.finish_account_read(&s.original, &s.current, s.stored.as_ref(), result)
            .map_err(|error| {
                if !s.any_write {
                    return error;
                }
                let mut details = error.details.as_object().cloned().unwrap_or_default();
                details.insert("operation".into(), json!("album_collection"));
                details.insert("write_outcome".into(), json!("unconfirmed"));
                error
                    .retryable(false)
                    .with_details(serde_json::Value::Object(details))
            })
    }
    async fn album_collection_snapshot(&self, s: &mut CollectionSession) -> Result<Snapshot> {
        let mut scan = CollectionScan::default();
        for page in 1..=MAX_PAGES {
            let response = self
                .client
                .account_album_collections(page, s.current.token(), s.current.user_id())
                .await?;
            let response = self
                .accept_playlist_read(&s.alias, &mut s.current, &mut s.stored, response)
                .await?;
            if scan.push(response)? {
                self.accept_read(&s.current, s.stored.as_ref(), &s.current)?;
                return Ok(Snapshot { items: scan.items });
            }
        }
        Err(migu_upstream_error(
            "Migu album collections exceeded the complete-read page budget",
        ))
    }
    async fn collected_album_page(
        &self,
        kind: AlbumKind,
        uid: Option<&str>,
        request: &PageRequest,
    ) -> Result<Page<CollectedAlbum>> {
        super::account_playlists::validate_page(request)?;
        let mut s = self.collection_session(request.account.as_deref(), uid)?;
        let result = async {
            self.verify_account_step(&s.alias, &mut s.current, &mut s.stored, s.original.token())
                .await?;
            let snapshot = self.album_collection_snapshot(&mut s).await?;
            Ok(snapshot.page(kind, request, s.current.user_id()))
        }
        .await;
        self.finish_album_collection(&s, result)
    }
    pub(super) async fn ordinary_album_collections(
        &self,
        uid: Option<&str>,
        request: &PageRequest,
    ) -> Result<Page<Album>> {
        let page = self
            .collected_album_page(AlbumKind::Ordinary, uid, request)
            .await?;
        let items = page
            .items
            .into_iter()
            .map(|item| match item {
                CollectedAlbum::Ordinary(album) => Ok(album),
                CollectedAlbum::Digital(_) => Err(migu_upstream_error(
                    "Migu ordinary album page contained a digital album",
                )),
            })
            .collect::<Result<_>>()?;
        Ok(Page {
            items,
            pagination: page.pagination,
        })
    }
    pub(super) async fn digital_album_collections(
        &self,
        uid: Option<&str>,
        request: &PageRequest,
    ) -> Result<Page<DigitalAlbum>> {
        let page = self
            .collected_album_page(AlbumKind::Digital, uid, request)
            .await?;
        let items = page
            .items
            .into_iter()
            .map(|item| match item {
                CollectedAlbum::Digital(album) => Ok(album),
                CollectedAlbum::Ordinary(_) => Err(migu_upstream_error(
                    "Migu digital album page contained an ordinary album",
                )),
            })
            .collect::<Result<_>>()?;
        Ok(Page {
            items,
            pagination: page.pagination,
        })
    }
    async fn change_one_album(
        &self,
        kind: AlbumKind,
        id: &str,
        subscribed: bool,
        s: &mut CollectionSession,
        dispatched: &mut bool,
    ) -> Result<SubscriptionResult> {
        let title = if subscribed {
            let response = self
                .client
                .account_collection_album_title(kind, id, s.current.token(), s.current.user_id())
                .await?;
            self.accept_playlist_read(&s.alias, &mut s.current, &mut s.stored, response)
                .await?
        } else {
            String::new()
        };
        self.accept_read(&s.current, s.stored.as_ref(), &s.current)?;
        s.any_write = true;
        *dispatched = true;
        let response = self
            .client
            .set_account_album_collection(
                kind,
                id,
                &title,
                subscribed,
                s.current.token(),
                s.current.user_id(),
            )
            .await?;
        self.accept_playlist_read(&s.alias, &mut s.current, &mut s.stored, response)
            .await?;
        let snapshot = self.album_collection_snapshot(s).await?;
        if snapshot.contains(kind, id) != subscribed {
            return Err(migu_upstream_error(
                "Migu complete album collection readback disagrees with the requested state",
            ));
        }
        let response = self
            .client
            .account_album_collection_state(kind, id, s.current.token(), s.current.user_id())
            .await?;
        let state = self
            .accept_playlist_read(&s.alias, &mut s.current, &mut s.stored, response)
            .await?;
        if state != subscribed {
            return Err(migu_upstream_error(
                "Migu explicit album collection state disagrees with the full library",
            ));
        }
        self.accept_read(&s.current, s.stored.as_ref(), &s.current)?;
        Ok(SubscriptionResult {
            resource_ref: tuneweave_core::ResourceRef::new(Platform::Migu, id)
                .map_err(|_| migu_invalid_request("Migu album ID is invalid"))?,
            subscribed,
            extensions: Extensions::from([
                ("backend".into(), json!("official_pc_album_collection")),
                ("resource_type".into(), json!(kind.resource_type())),
                ("source_type".into(), json!(kind.source_type())),
                ("source_user_id".into(), json!(s.current.user_id())),
                (
                    "verified_by".into(),
                    json!("complete_mixed_album_library_and_explicit_state"),
                ),
            ]),
        })
    }
    pub(super) async fn change_album_collections(
        &self,
        kind: AlbumKind,
        ids: &[String],
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<Vec<SubscriptionResult>> {
        if ids.is_empty() || ids.len() > 100 {
            return Err(migu_invalid_request(
                "Migu album collection batches require 1 to 100 distinct IDs",
            ));
        }
        let mut seen = BTreeSet::new();
        for id in ids {
            super::albums::validate_source(id, None)?;
            if !seen.insert(id) {
                return Err(migu_invalid_request(
                    "Migu album collection batches require distinct IDs",
                ));
            }
        }
        let mut s = self.collection_session(account, None)?;
        let mut completed: Vec<SubscriptionResult> = Vec::new();
        let mut dispatched = false;
        let result = async {
            self.verify_account_step(&s.alias, &mut s.current, &mut s.stored, s.original.token())
                .await?;
            for id in ids {
                dispatched = false;
                completed.push(
                    self.change_one_album(kind, id, subscribed, &mut s, &mut dispatched)
                        .await?,
                );
            }
            Ok(())
        }
        .await;
        self.finish_album_collection(&s, result).map_err(|error| {
            if ids.len() == 1 {
                return error;
            }
            let mut details = error.details.as_object().cloned().unwrap_or_default();
            details.insert("atomic".into(), json!(false));
            details.insert("resource_type".into(), json!(kind.resource_type()));
            details.insert(
                "completed_refs".into(),
                json!(
                    completed
                        .iter()
                        .map(|v| &v.resource_ref)
                        .collect::<Vec<_>>()
                ),
            );
            details.insert(
                "failed_ref".into(),
                json!(ids.get(completed.len()).map(|id| format!("migu:{id}"))),
            );
            details.insert("failed_write_dispatched".into(), json!(dispatched));
            details.insert(
                "remaining_refs".into(),
                json!(
                    ids.iter()
                        .skip(completed.len() + 1)
                        .map(|id| format!("migu:{id}"))
                        .collect::<Vec<_>>()
                ),
            );
            error.with_details(serde_json::Value::Object(details))
        })?;
        Ok(completed)
    }
    pub(super) async fn change_album_collection(
        &self,
        kind: AlbumKind,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        self.change_album_collections(kind, &[id.to_owned()], subscribed, account)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| {
                migu_upstream_error("Migu album collection operation returned no result")
            })
    }
}

#[cfg(test)]
mod tests;
