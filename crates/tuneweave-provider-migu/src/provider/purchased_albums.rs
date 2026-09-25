use super::account_read::Read;
use super::*;
use crate::client::purchased_albums::{BACKEND, MAX_PAGES};
use crate::credential::error;
use sha1::{Digest, Sha1};
use std::time::Duration;
use tuneweave_core::{ErrorCode, PurchasedAlbum};

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const DEADLINE: Duration = Duration::from_secs(120);

impl MiguProvider {
    pub(super) async fn read_purchased_albums(
        &self,
        request: &PageRequest,
    ) -> Result<Page<PurchasedAlbum>> {
        self.read_purchased_albums_bounded(request, MAX_BYTES, DEADLINE)
            .await
    }
    async fn read_purchased_albums_bounded(
        &self,
        request: &PageRequest,
        maximum: u64,
        deadline: Duration,
    ) -> Result<Page<PurchasedAlbum>> {
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(migu_invalid_request(
                "Migu purchased album pagination is invalid",
            ));
        }
        let mut read = Read::new(self, request.account.as_deref())?;
        let result = tokio::time::timeout(deadline, async {
            read.start().await?;
            let mut remaining = maximum;
            let (items, page_sizes) = self
                .scan_purchased_albums(&mut read, &mut remaining)
                .await?;
            let (confirmed, confirmed_sizes) = self
                .scan_purchased_albums(&mut read, &mut remaining)
                .await?;
            if items != confirmed || page_sizes != confirmed_sizes {
                return Err(error(
                    ErrorCode::Conflict,
                    "Migu purchased albums changed during the complete read",
                ));
            }
            let bytes =
                serde_json::to_vec(&(BACKEND, read.current.user_id(), &items)).map_err(|_| {
                    error(
                        ErrorCode::InternalError,
                        "Migu purchase snapshot could not be serialized",
                    )
                })?;
            // Check all records again against tokens learned during the confirmation scan.
            let exported = String::from_utf8_lossy(&bytes);
            for token in read.secrets.iter().collect::<BTreeSet<_>>() {
                read.check()?;
                if exported.contains(token) {
                    return Err(migu_upstream_error(
                        "Migu purchased albums reflected a credential",
                    ));
                }
                for item in &items {
                    if let Some(url) = &item.cover_url {
                        crate::client::account_media::reject_url_secret(url, token)?;
                    }
                }
            }
            let total = items.len() as u64;
            let selected = items
                .into_iter()
                .enumerate()
                .skip(request.offset as usize)
                .take(request.limit as usize)
                .map(|(index, mut item)| {
                    item.extensions
                        .insert("source_position".into(), json!(index));
                    item
                })
                .collect::<Vec<_>>();
            let end = request.offset + selected.len() as u32;
            let more = u64::from(end) < total;
            Ok(Page {
                items: selected,
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(total),
                    has_more: more,
                    next_offset: more.then_some(end),
                    extensions: Extensions::from([
                        ("backend".into(), json!(BACKEND)),
                        ("source_user_id".into(), json!(read.current.user_id())),
                        ("purchase_kind".into(), json!("album_subscriptions")),
                        ("complete_read".into(), json!(true)),
                        ("consistency".into(), json!("two_complete_reads")),
                        (
                            "upstream_pages_fetched".into(),
                            json!(page_sizes.len() + confirmed_sizes.len()),
                        ),
                        (
                            "source_snapshot_id".into(),
                            json!(format!(
                                "migu-purchased-albums-{}",
                                hex::encode(Sha1::digest(bytes))
                            )),
                        ),
                    ]),
                },
            })
        })
        .await
        .map_err(|_| {
            error(
                ErrorCode::UpstreamTimeout,
                "Migu purchased albums exceeded the total deadline",
            )
        })
        .and_then(|r| r);
        read.finish(result)
    }
    pub(super) async fn scan_purchased_albums(
        &self,
        read: &mut Read<'_>,
        remaining: &mut u64,
    ) -> Result<(Vec<PurchasedAlbum>, Vec<usize>)> {
        let mut items = Vec::new();
        let mut pages = BTreeSet::new();
        let mut sizes = Vec::new();
        for page in 1..=MAX_PAGES {
            read.check()?;
            if *remaining == 0 {
                return Err(migu_upstream_error(
                    "Migu purchased albums exceeded the response budget",
                ));
            }
            let response = self
                .client
                .account_purchased_albums_page(
                    read.current.token(),
                    read.current.user_id(),
                    page,
                    *remaining,
                )
                .await;
            let response = read.accept(response).await??;
            *remaining = remaining
                .checked_sub(response.bytes as u64)
                .ok_or_else(|| {
                    migu_upstream_error("Migu purchased albums exceeded the response budget")
                })?;
            let ids = response
                .items
                .iter()
                .map(|item| {
                    (
                        item.extensions["resource_type"].clone(),
                        item.extensions["content_id"].clone(),
                    )
                })
                .collect::<Vec<_>>();
            let ids = serde_json::to_string(&ids).map_err(|_| {
                migu_upstream_error("Migu purchase identities could not be serialized")
            })?;
            if !response.items.is_empty() && !pages.insert(ids) {
                return Err(migu_upstream_error(
                    "Migu purchased albums repeated an entire page",
                ));
            }
            sizes.push(response.items.len());
            items.extend(response.items);
            if !response.has_next {
                return Ok((items, sizes));
            }
        }
        Err(migu_upstream_error(
            "Migu purchased albums exceeded the complete-read page budget",
        ))
    }
}

#[cfg(test)]
mod tests;
