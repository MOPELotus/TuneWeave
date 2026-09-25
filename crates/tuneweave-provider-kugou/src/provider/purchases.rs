use super::*;
use crate::account::purchases::{Item, Kind, MAX_PAGES};
use md5::{Digest, Md5};
use tuneweave_core::{PurchasedAlbum, PurchasedTrack};

fn changed() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::Conflict,
        "KuGou purchase library changed during its complete read",
    )
    .with_platform(Platform::Kugou)
}
fn budget_exceeded() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        "KuGou purchases exceeded the complete-read budget",
    )
    .with_platform(Platform::Kugou)
}
fn charge_bytes(remaining: &mut usize, bytes: usize) -> Result<()> {
    *remaining = remaining.checked_sub(bytes).ok_or_else(budget_exceeded)?;
    Ok(())
}
impl KugouProvider {
    pub(super) async fn native_purchased_tracks(
        &self,
        request: &PageRequest,
    ) -> Result<Page<PurchasedTrack>> {
        let page = self.native_purchases(Kind::Tracks, request).await?;
        Ok(Page {
            items: page
                .items
                .into_iter()
                .map(|i| match i {
                    Item::Track(t) => Ok(t),
                    _ => Err(changed()),
                })
                .collect::<Result<_>>()?,
            pagination: page.pagination,
        })
    }
    pub(super) async fn native_purchased_albums(
        &self,
        request: &PageRequest,
    ) -> Result<Page<PurchasedAlbum>> {
        let page = self.native_purchases(Kind::Albums, request).await?;
        Ok(Page {
            items: page
                .items
                .into_iter()
                .map(|i| match i {
                    Item::Album(a) => Ok(a),
                    _ => Err(changed()),
                })
                .collect::<Result<_>>()?,
            pagination: page.pagination,
        })
    }
    async fn native_purchases(&self, kind: Kind, request: &PageRequest) -> Result<Page<Item>> {
        self.native_purchases_for_owner(kind, request, None).await
    }

    async fn native_purchases_for_owner(
        &self,
        kind: Kind,
        request: &PageRequest,
        expected_uid: Option<&str>,
    ) -> Result<Page<Item>> {
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kugou_invalid_request(
                "KuGou purchase pagination is invalid",
            ));
        }
        let mut read = self
            .begin_native_read(
                request.account.as_deref().unwrap_or("default"),
                expected_uid,
            )
            .await?;
        let result = async {
            let (items, pages) = self.scan_purchases(&mut read, kind).await?;
            // There is no proven version token on these endpoints. Compare two complete
            // observations; a first-page-only check cannot detect later-page edits.
            let (confirmation, confirm_pages) = self.scan_purchases(&mut read, kind).await?;
            if items != confirmation || pages != confirm_pages {
                return Err(changed());
            }
            let total = items.len() as u64;
            let unresolved = items.iter().filter(|i| i.catalogue_id().is_none()).count();
            let bytes = serde_json::to_vec(&(kind.name(), &read.session()?.user_id, &items))
                .map_err(|_| changed())?;
            let snapshot_id = format!("kugou-purchases-{:x}", Md5::digest(bytes));
            let selected = items
                .into_iter()
                .skip(request.offset as usize)
                .take(request.limit as usize)
                .collect::<Vec<_>>();
            let end = request.offset + selected.len() as u32;
            let more = u64::from(end) < total;
            Ok(Page {
                items: selected,
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(total),
                    next_offset: more.then_some(end),
                    has_more: more,
                    extensions: Extensions::from([
                        ("backend".into(), json!("native_purchase_library")),
                        ("library_owner_id".into(), json!(read.session()?.user_id)),
                        ("purchase_kind".into(), json!(kind.name())),
                        ("source_snapshot_id".into(), json!(snapshot_id)),
                        ("complete_read".into(), json!(true)),
                        ("consistency".into(), json!("two_complete_reads")),
                        ("upstream_page_size".into(), json!(kind.page_size())),
                        (
                            "upstream_pages_fetched".into(),
                            json!(pages + confirm_pages),
                        ),
                        ("unresolved_entries".into(), json!(unresolved)),
                    ]),
                },
            })
        }
        .await;
        self.finish_account_read(read, result)
    }
    async fn scan_purchases(
        &self,
        read: &mut session::AccountRead,
        kind: Kind,
    ) -> Result<(Vec<Item>, u32)> {
        let mut remaining = usize::MAX;
        self.scan_purchases_bounded(read, kind, usize::MAX, &mut remaining)
            .await
    }
    async fn scan_purchases_bounded(
        &self,
        read: &mut session::AccountRead,
        kind: Kind,
        max_items: usize,
        remaining_bytes: &mut usize,
    ) -> Result<(Vec<Item>, u32)> {
        let mut items = Vec::new();
        let mut total = None;
        let mut seen = BTreeSet::new();
        for page in 1..=MAX_PAGES {
            self.check_account_read(read)?;
            let response = self
                .client
                .native_purchases_page(read.session()?, kind, page)
                .await;
            self.check_account_read(read)?;
            let response = response?;
            charge_bytes(remaining_bytes, response.response_bytes)?;
            if response.total > max_items as u64 {
                return Err(budget_exceeded());
            }
            if total.is_some_and(|n| n != response.total) {
                return Err(changed());
            }
            total = Some(response.total);
            for item in response.items {
                if !seen.insert(item.identity()?) {
                    return Err(changed());
                }
                items.push(item);
            }
            if items.len() as u64 == response.total {
                return Ok((items, page));
            }
        }
        Err(budget_exceeded())
    }
}
mod album_source;
mod source;
#[cfg(test)]
mod tests;
