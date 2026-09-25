use super::*;
use tuneweave_core::Album;

fn validate(id: &str, account: Option<&str>) -> Result<()> {
    if account.is_some()
        || id
            .parse::<u64>()
            .ok()
            .is_none_or(|n| n == 0 || n.to_string() != id)
    {
        return Err(kuwo_invalid_request(
            "Kuwo public album requires a canonical positive ID and no account",
        ));
    }
    Ok(())
}

impl KuwoProvider {
    pub(super) async fn read_album(&self, id: &str, account: Option<&str>) -> Result<Album> {
        validate(id, account)?;
        Ok(self.client.album_detail(id).await?.album)
    }

    pub(super) async fn read_album_tracks(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<Track>> {
        validate(id, request.account.as_deref())?;
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kuwo_invalid_request(
                "Kuwo album pagination is outside the supported range",
            ));
        }
        let detail = self.client.album_detail(id).await?;
        let total = detail.tracks.len() as u64;
        let items: Vec<_> = detail
            .tracks
            .into_iter()
            .skip(request.offset as usize)
            .take(request.limit as usize)
            .collect();
        let end = request.offset + items.len() as u32;
        let more = u64::from(end) < total;
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(total),
                has_more: more,
                next_offset: more.then_some(end),
                extensions: Extensions::from([
                    ("backend".into(), json!("current_web_albuminfo")),
                    ("source_album_id".into(), json!(id)),
                    ("complete_read".into(), json!(true)),
                    ("upstream_requests".into(), json!(1)),
                ]),
            },
        })
    }
}
