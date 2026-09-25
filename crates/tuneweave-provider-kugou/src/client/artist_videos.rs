//! The artist's native MV catalogue, including the uploaded clips shown by the official PC MV tab.
use super::dto::{Number, required_text};
use super::openapi::{Endpoint, check_status};
use super::*;

pub(crate) const PAGE_SIZE: u32 = 30;

#[derive(Deserialize)]
struct Envelope {
    data: Vec<Entry>,
    total: Number,
    extra: Option<Extra>,
}
#[derive(Deserialize)]
struct Extra {
    page_total: Option<Number>,
}
#[derive(Deserialize)]
struct Entry {
    video_id: Number,
    video_name: String,
    timelength: Option<Number>,
    audio_id: Option<Number>,
    album_audio_id: Option<Number>,
    user_id: Option<Number>,
}
pub(crate) struct VideoEntry {
    pub id: String,
    pub duration_ms: Option<u64>,
    pub audio_id: Option<String>,
    pub album_audio_id: Option<String>,
    pub uploader_id: Option<String>,
}
pub(crate) struct VideoPage {
    pub items: Vec<VideoEntry>,
    pub total: u64,
}

impl KugouClient {
    pub(crate) async fn artist_video_page(&self, id: u64, page: u32) -> Result<VideoPage> {
        if id == 0 || page == 0 {
            return Err(kugou_invalid_media_request(
                "Invalid KuGou artist video page",
            ));
        }
        let device = self.device_identity()?;
        let query = BTreeMap::from([
            ("author_id", id.to_string()),
            ("is_fanmade", String::new()),
            ("tag_idx", String::new()),
            ("pagesize", PAGE_SIZE.to_string()),
            ("page", page.to_string()),
        ]);
        let bytes = self
            .public_catalogue_get(Endpoint::ArtistVideos, query, &device)
            .await?;
        parse(&bytes, page)
    }
}
fn optional_id(value: Option<Number>) -> Option<String> {
    value.map(|n| n.0).filter(|n| *n > 0).map(|n| n.to_string())
}
fn invalid() -> TuneWeaveError {
    kugou_upstream_error("KuGou artist video catalogue returned inconsistent metadata")
}
fn parse(bytes: &[u8], page: u32) -> Result<VideoPage> {
    check_status(bytes)?;
    let e: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let start = u64::from(page.checked_sub(1).ok_or_else(invalid)?) * u64::from(PAGE_SIZE);
    if e.data.len() as u64 != e.total.0.saturating_sub(start).min(u64::from(PAGE_SIZE))
        || e.extra
            .and_then(|v| v.page_total)
            .is_some_and(|n| n.0 != e.total.0)
    {
        return Err(invalid());
    }
    let mut ids = BTreeSet::new();
    let mut items = Vec::with_capacity(e.data.len());
    for entry in e.data {
        let id = entry.video_id.id()?;
        if !ids.insert(id.clone()) {
            return Err(invalid());
        }
        // Validate the catalogue title, but detail metadata supplies the returned video.
        required_text(entry.video_name)?;
        items.push(VideoEntry {
            id,
            duration_ms: entry.timelength.map(|n| n.0).filter(|n| *n > 0),
            audio_id: optional_id(entry.audio_id),
            album_audio_id: optional_id(entry.album_audio_id),
            uploader_id: optional_id(entry.user_id),
        });
    }
    Ok(VideoPage {
        items,
        total: e.total.0,
    })
}

#[cfg(test)]
mod tests;
