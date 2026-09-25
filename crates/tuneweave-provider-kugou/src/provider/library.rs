use super::*;
use crate::account::library::{LibraryVersion, MAX_PAGES, PAGE_SIZE};

mod collection_plan;
mod favorites;
mod management;
mod sorting;
mod tracks;
mod web;
mod write;

pub(super) struct LibrarySnapshot {
    version: LibraryVersion,
    items: Vec<(u8, Playlist)>,
    pages: u32,
    deleted: usize,
}

impl KugouProvider {
    async fn read_native_library(
        &self,
        read: &mut session::AccountRead,
    ) -> Result<LibrarySnapshot> {
        let mut version = None;
        let mut seen = BTreeSet::new();
        let mut items = Vec::new();
        let mut deleted = 0;
        for page in 1..=MAX_PAGES {
            let response = self
                .client
                .native_library_page(read.session()?, page)
                .await?;
            self.check_account_read(read)?;
            if version.as_ref().is_some_and(|v| v != &response.version) {
                return Err(library_changed());
            }
            version = Some(response.version);
            let count = response.rows.len();
            for row in response.rows {
                if !seen.insert((row.list_id, row.kind)) {
                    return Err(library_changed());
                }
                if let Some(playlist) = row.playlist {
                    items.push((row.kind, playlist));
                } else {
                    deleted += 1;
                }
            }
            // list_count/collect_count describe upstream state, not the visible row total.
            // A full-sized last page requires a subsequent empty page.
            if count < PAGE_SIZE {
                return Ok(LibrarySnapshot {
                    version: version.ok_or_else(library_changed)?,
                    items,
                    pages: page,
                    deleted,
                });
            }
        }
        Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "KuGou library exceeded its complete-read page budget",
        )
        .with_platform(Platform::Kugou))
    }

    pub(super) async fn native_account_playlists(
        &self,
        uid: Option<&str>,
        section: Option<u8>,
        request: &PageRequest,
    ) -> Result<Page<Playlist>> {
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kugou_invalid_request(
                "KuGou account library pagination is invalid",
            ));
        }
        let mut read = self
            .begin_native_read(request.account.as_deref().unwrap_or("default"), uid)
            .await?;
        let result = async {
            let snapshot = self.read_native_library(&mut read).await?;
            let all = snapshot
                .items
                .into_iter()
                .filter(|(kind, _)| section.is_none_or(|v| *kind == v))
                .map(|(_, playlist)| playlist)
                .collect::<Vec<_>>();
            let total = all.len() as u64;
            let items: Vec<_> = all
                .into_iter()
                .skip(request.offset as usize)
                .take(request.limit as usize)
                .collect();
            let end = request.offset + items.len() as u32;
            let has_more = u64::from(end) < total;
            let mut extensions = snapshot.version.extensions();
            extensions.extend([
                ("backend".into(), json!("native_cloudlist_v8")),
                ("library_owner_id".into(), json!(read.session()?.user_id)),
                (
                    "library_section".into(),
                    json!(match section {
                        Some(0) => "created",
                        Some(1) => "collected",
                        _ => "all",
                    }),
                ),
                ("upstream_page_size".into(), json!(PAGE_SIZE)),
                ("upstream_pages_fetched".into(), json!(snapshot.pages)),
                ("deleted_rows".into(), json!(snapshot.deleted)),
                ("complete_read".into(), json!(true)),
            ]);
            Ok(Page {
                items,
                pagination: PageMeta {
                    limit: request.limit,
                    offset: request.offset,
                    total: Some(total),
                    next_offset: has_more.then_some(end),
                    has_more,
                    extensions,
                },
            })
        }
        .await;
        self.finish_account_read(read, result)
    }

    pub(super) async fn native_playlist_metadata(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        Ok(self.native_playlist_snapshot(id, account).await?.playlist)
    }
}

pub(super) fn parse_reference(id: &str) -> Result<(&str, u8, u64)> {
    let invalid = || kugou_invalid_request("KuGou account playlist reference is invalid");
    let mut parts = id.split(':');
    if parts.next() != Some("cloudlist") {
        return Err(invalid());
    }
    let uid = parts
        .next()
        .filter(|v| crate::credential::valid_uid(v))
        .ok_or_else(invalid)?;
    let kind = match parts.next() {
        Some("0") => 0,
        Some("1") => 1,
        _ => return Err(invalid()),
    };
    let list = parts
        .next()
        .filter(|v| crate::credential::valid_uid(v))
        .ok_or_else(invalid)?;
    if parts.next().is_some() {
        return Err(invalid());
    }
    Ok((uid, kind, list.parse().map_err(|_| invalid())?))
}

fn library_changed() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::Conflict,
        "KuGou library version, counters or playlist identity changed while paging",
    )
    .with_platform(Platform::Kugou)
}

#[cfg(test)]
pub(super) mod tests;
