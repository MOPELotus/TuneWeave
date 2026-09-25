use super::*;
use crate::client::artist_videos::{PAGE_SIZE, VideoEntry};
use crate::client::videos::DETAIL_PAGE_SIZE;
use tuneweave_core::{ArtistVideoListRequest, Video, VideoResourceKind};

fn invalid() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        "KuGou artist video catalogue changed or returned inconsistent identities",
    )
    .with_platform(Platform::Kugou)
}
fn crosscheck(entry: &VideoEntry, video: &Video) -> Result<()> {
    if entry.id != video.id {
        return Err(invalid());
    }
    if let (Some(a), Some(b)) = (entry.duration_ms, video.duration_ms) {
        if a != b {
            return Err(invalid());
        }
    }
    for (key, expected) in [
        ("audio_id", &entry.audio_id),
        ("album_audio_id", &entry.album_audio_id),
    ] {
        if let (Some(expected), Some(actual)) = (expected, video.extensions.get(key)) {
            if actual.as_str() != Some(expected) {
                return Err(invalid());
            }
        }
    }
    if let (Some(expected), Some(actual)) = (
        &entry.uploader_id,
        video.extensions.get("uploader").and_then(|u| u.get("id")),
    ) {
        if actual.as_str() != Some(expected) {
            return Err(invalid());
        }
    }
    Ok(())
}

impl KugouProvider {
    pub(super) async fn read_artist_videos(
        &self,
        id: &str,
        request: &ArtistVideoListRequest,
    ) -> Result<Page<Video>> {
        self.require_public_source()?;
        if request.account.is_some() || request.cursor.is_some() || request.order.is_some() {
            return Err(kugou_invalid_request(
                "KuGou public artist videos accept only anonymous offset pagination and the platform default order",
            ));
        }
        if !(1..=100).contains(&request.limit)
            || request.offset.checked_add(request.limit).is_none()
        {
            return Err(kugou_invalid_request(
                "Invalid KuGou artist video pagination",
            ));
        }
        let id = id
            .parse::<u64>()
            .ok()
            .filter(|n| *n > 0 && n.to_string() == id)
            .ok_or_else(|| {
                kugou_invalid_request("KuGou artist ID must be a canonical positive decimal")
            })?;
        let artist = self.client.artist_metadata(id).await?;
        let first_page = request.offset / PAGE_SIZE + 1;
        let skip = request.offset % PAGE_SIZE;
        let budget = (skip + request.limit).div_ceil(PAGE_SIZE);
        let mut total = artist.mv_count;
        let mut selected = Vec::new();
        let mut seen = BTreeSet::new();
        let mut pages = 0;
        for index in 0..budget {
            let page = self
                .client
                .artist_video_page(id, first_page + index)
                .await?;
            if total.is_some_and(|total| total != page.total) {
                return Err(invalid());
            }
            total = Some(page.total);
            for item in &page.items {
                if !seen.insert(item.id.clone()) {
                    return Err(invalid());
                }
            }
            let to_skip = if index == 0 { skip as usize } else { 0 };
            let take = request.limit as usize - selected.len();
            selected.extend(page.items.into_iter().skip(to_skip).take(take));
            pages += 1;
            if selected.len() == request.limit as usize
                || u64::from(first_page + index) * u64::from(PAGE_SIZE) >= page.total
            {
                break;
            }
        }
        let mut items = Vec::with_capacity(selected.len());
        let mut detail_batches = 0;
        for batch in selected.chunks(DETAIL_PAGE_SIZE) {
            let ids = batch.iter().map(|e| e.id.clone()).collect::<Vec<_>>();
            let details = self
                .client
                .public_video_details(&ids, VideoResourceKind::Mv)
                .await?;
            detail_batches += 1;
            for (entry, detail) in batch.iter().zip(details) {
                let mut video = detail.video;
                crosscheck(entry, &video)?;
                // This is the source catalogue artist, never a substitute for an actual creator.
                video
                    .extensions
                    .insert("catalogue_artist_id".into(), json!(id.to_string()));
                video.extensions.insert(
                    "artist_video_position".into(),
                    json!(u64::from(request.offset) + items.len() as u64),
                );
                items.push(video);
            }
        }
        let end = request.offset + items.len() as u32;
        let has_more = u64::from(end) < total.unwrap_or(0);
        Ok(Page {
            items,
            pagination: PageMeta {
                offset: request.offset,
                limit: request.limit,
                total,
                has_more,
                next_offset: has_more.then_some(end),
                extensions: Extensions::from([
                    ("backend".into(), json!("official_artist_mv_catalogue")),
                    ("artist_id".into(), json!(id.to_string())),
                    ("kind".into(), json!(request.kind)),
                    ("order".into(), json!("platform_default")),
                    ("upstream_page_size".into(), json!(PAGE_SIZE)),
                    ("upstream_pages_fetched".into(), json!(pages)),
                    ("detail_batches_fetched".into(), json!(detail_batches)),
                    ("complete_window".into(), json!(true)),
                ]),
            },
        })
    }
}

#[cfg(test)]
mod tests;
