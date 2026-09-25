use super::*;
use tuneweave_core::Album;

fn validate(id: &str, account: Option<&str>) -> Result<u64> {
    if account.is_some() {
        return Err(kugou_invalid_request(
            "KuGou public albums do not accept an account",
        ));
    }
    id.parse::<u64>()
        .ok()
        .filter(|n| *n > 0 && n.to_string() == id)
        .ok_or_else(|| kugou_invalid_request("KuGou album ID must be a canonical positive decimal"))
}
impl KugouProvider {
    pub(super) async fn read_album(&self, id: &str, account: Option<&str>) -> Result<Album> {
        self.require_public_source()?;
        self.client.album_metadata(validate(id, account)?).await
    }
    pub(super) async fn read_album_tracks(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<Track>> {
        self.require_public_source()?;
        let id = validate(id, request.account.as_deref())?;
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kugou_invalid_request("Invalid KuGou album pagination"));
        }
        let album = self.client.complete_album_tracks(id).await?;
        let items: Vec<_> = album
            .tracks
            .into_iter()
            .skip(request.offset as usize)
            .take(request.limit as usize)
            .collect();
        let end = u64::from(request.offset) + items.len() as u64;
        let has_more = end < album.total;
        let mut extensions = Extensions::from([
            ("backend".into(), json!("official_album_complete_tracks")),
            ("album_id".into(), json!(id.to_string())),
            ("complete_snapshot".into(), json!(true)),
            ("upstream_pages_fetched".into(), json!(album.pages)),
        ]);
        if let Some(count) = album.discs {
            extensions.insert("disc_count".into(), json!(count));
        }
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(album.total),
                has_more,
                next_offset: has_more.then_some(end as u32),
                extensions,
            },
        })
    }
}

#[cfg(test)]
mod tests;
