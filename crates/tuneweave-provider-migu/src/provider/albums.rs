use super::account_read::Read;
use super::*;
use crate::client::albums::{AlbumKind, MAX_ALBUM_ITEMS};

pub(super) fn validate_source(id: &str, account: Option<&str>) -> Result<()> {
    if account.is_some() {
        return Err(migu_invalid_request(
            "Migu public albums do not accept an account",
        ));
    }
    if id.is_empty()
        || id.len() > 64
        || id.starts_with('0')
        || !id.bytes().all(|v| v.is_ascii_digit())
    {
        return Err(migu_invalid_request(
            "Migu album ID must be a canonical positive decimal",
        ));
    }
    Ok(())
}

impl MiguProvider {
    pub(super) async fn read_album_tracks(
        &self,
        id: &str,
        request: &PageRequest,
        digital: bool,
    ) -> Result<Page<Track>> {
        validate_source(id, request.account.as_deref())?;
        if !(1..=100).contains(&request.limit) {
            return Err(migu_invalid_request(
                "Migu album track limit must be between 1 and 100",
            ));
        }
        let (tracks, pages) = self.complete_album_tracks(id, digital, None).await?;
        let total = tracks.len() as u64;
        let items: Vec<_> = tracks
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
                extensions: Extensions::from([
                    (
                        "backend".to_owned(),
                        json!("official_album_complete_tracks"),
                    ),
                    (
                        "collection_type".to_owned(),
                        json!(if digital { "digital_album" } else { "album" }),
                    ),
                    ("collection_id".to_owned(), json!(id)),
                    ("complete_snapshot".to_owned(), json!(true)),
                    ("upstream_pages_fetched".to_owned(), json!(pages)),
                ]),
            },
        })
    }

    pub(super) async fn complete_album_tracks(
        &self,
        id: &str,
        digital: bool,
        read: Option<&Read<'_>>,
    ) -> Result<(Vec<Track>, u32)> {
        validate_source(id, None)?;
        let check = || read.map_or(Ok(()), Read::check);
        let kind = if digital {
            AlbumKind::Digital
        } else {
            AlbumKind::Ordinary
        };
        check()?;
        let metadata = if digital {
            self.client
                .digital_album_metadata(id)
                .await
                .map(|album| album.track_count)
        } else {
            self.client
                .album_metadata(id)
                .await
                .map(|album| album.track_count)
        };
        check()?;
        let mut total = metadata?;
        if total.is_some_and(|n| n > MAX_ALBUM_ITEMS as u64) {
            return Err(migu_upstream_error(
                "Migu album exceeded the bounded collection size",
            ));
        }
        let mut tracks = Vec::new();
        let mut signatures = BTreeSet::new();
        let mut pages = 0;
        loop {
            pages += 1;
            check()?;
            let response = self.client.album_track_page(kind, id, pages).await;
            check()?;
            let page = response?;
            if let Some(count) = page.total {
                if total.is_some_and(|previous| previous != count) || count > MAX_ALBUM_ITEMS as u64
                {
                    return Err(migu_upstream_error(
                        "Migu album count changed during traversal",
                    ));
                }
                total = Some(count);
            }
            let signature: Vec<_> = page.tracks.iter().map(|t| t.id.clone()).collect();
            if !signature.is_empty() && !signatures.insert(signature) {
                return Err(migu_upstream_error("Migu album repeated a track page"));
            }
            tracks.extend(page.tracks);
            if tracks.len() > MAX_ALBUM_ITEMS
                || total.is_some_and(|count| count < tracks.len() as u64)
            {
                return Err(migu_upstream_error(
                    "Migu album track count exceeded its reported bounds",
                ));
            }
            if !page.has_more {
                if total != Some(tracks.len() as u64) {
                    return Err(migu_upstream_error(
                        "Migu album ended without its complete counted track list",
                    ));
                }
                break;
            }
            if pages >= 64 || total.is_some_and(|count| count <= tracks.len() as u64) {
                return Err(migu_upstream_error(
                    "Migu album cannot continue within its request bounds",
                ));
            }
        }
        Ok((tracks, pages))
    }
}

#[cfg(test)]
mod tests;
