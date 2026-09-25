use super::*;
use crate::client::videos::{self, MvDetail};
use tuneweave_core::{
    ArtistVideoListRequest, ErrorCode, Video, VideoDetailRequest, VideoKind, VideoResourceKind,
    VideoSearchDuration, VideoSearchOrder,
};

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const DEADLINE: std::time::Duration = std::time::Duration::from_secs(45);
async fn bounded<T>(work: impl std::future::Future<Output = Result<T>>) -> Result<T> {
    tokio::time::timeout(DEADLINE, work).await.map_err(|_| {
        TuneWeaveError::new(
            ErrorCode::UpstreamTimeout,
            "Migu MV request exceeded its total deadline",
        )
        .with_platform(Platform::Migu)
    })?
}
impl MiguProvider {
    fn validate_mv_source(&self, id: &str, account: Option<&str>) -> Result<()> {
        self.require_public_source()?;
        if account.is_some() || !videos::valid_id(id) {
            return Err(migu_invalid_request(
                "Migu public MV requires a canonical positive ID and no account",
            ));
        }
        Ok(())
    }
    pub(super) async fn read_mv(&self, id: &str, r: &VideoDetailRequest) -> Result<MvDetail> {
        if r.account.is_some() || self.caller_credential.is_some() {
            if !videos::valid_id(id) || r.kind != VideoResourceKind::Mv {
                return Err(migu_invalid_request(
                    "Migu MV detail requires an MV identity",
                ));
            }
            let mut read = super::account_read::Read::new(self, r.account.as_deref())?;
            let result = bounded(async {
                read.start().await?;
                let mut budget = MAX_BYTES;
                let result = self.client.mv_detail(id, &mut budget).await;
                read.check()?;
                let mut value = result?;
                let serialized = serde_json::to_string(&value.detail)
                    .map_err(|_| migu_upstream_error("Migu MV detail serialization failed"))?;
                if read.secrets.iter().any(|secret| {
                    serialized.contains(secret)
                        || url::form_urlencoded::parse(serialized.as_bytes())
                            .any(|(key, value)| key.contains(secret) || value.contains(secret))
                }) {
                    return Err(migu_upstream_error(
                        "Migu MV metadata reflected credential material",
                    ));
                }
                read.mark(&mut value.detail.video.extensions);
                read.mark(&mut value.stats.extensions);
                Ok(value)
            })
            .await;
            return read.finish(result);
        }
        self.validate_mv_source(id, r.account.as_deref())?;
        if r.kind != VideoResourceKind::Mv {
            return Err(migu_invalid_request(
                "Migu video detail currently requires kind=mv",
            ));
        }
        let mut budget = MAX_BYTES;
        bounded(self.client.mv_detail(id, &mut budget)).await
    }
    pub(super) async fn search_mvs(&self, q: &SearchQuery) -> Result<Page<SearchItem>> {
        self.require_public_source()?;
        let mut plain = q.clone();
        plain.video_filters = None;
        validate_public_search_options(&plain)?;
        let order = if let Some(f) = &q.video_filters {
            if f.duration != VideoSearchDuration::Any || f.category_id.is_some() {
                return Err(migu_invalid_request(
                    "Migu MV search does not support duration or category filters",
                ));
            }
            match f.order {
                VideoSearchOrder::Relevance => 0,
                VideoSearchOrder::Newest => 1,
                VideoSearchOrder::MostPlayed => 2,
                _ => return Err(migu_invalid_request("Unsupported Migu MV search order")),
            }
        } else {
            0
        };
        let result =
            bounded(self.mv_window(q.limit, q.offset, None, Some((q.query.trim(), order)))).await?;
        Ok(Page {
            items: result.items.into_iter().map(SearchItem::Video).collect(),
            pagination: result.pagination,
        })
    }
    pub(super) async fn read_artist_mvs(
        &self,
        id: &str,
        r: &ArtistVideoListRequest,
    ) -> Result<Page<Video>> {
        self.validate_mv_source(id, r.account.as_deref())?;
        if r.kind != VideoKind::Mv
            || r.cursor.is_some()
            || r.order.as_deref().is_some_and(|v| v != "platform_default")
        {
            return Err(migu_invalid_request(
                "Migu artist videos require kind=mv, platform_default order and offset pagination",
            ));
        }
        bounded(self.mv_window(r.limit, r.offset, Some(id), None)).await
    }
    async fn mv_window(
        &self,
        limit: u32,
        offset: u32,
        artist: Option<&str>,
        search: Option<(&str, u32)>,
    ) -> Result<Page<Video>> {
        if !(1..=100).contains(&limit) || offset.checked_add(limit).is_none() {
            return Err(migu_invalid_request("Migu MV pagination is invalid"));
        }
        let width = if artist.is_some() { 10 } else { 20 };
        let skip = (offset % width) as usize;
        let first = offset / width + 1;
        let pages = (skip + limit as usize).div_ceil(width as usize);
        let mut budget = MAX_BYTES;
        if let Some(id) = artist {
            self.client.mv_artist_info(id, &mut budget).await?;
        }
        let mut items = Vec::new();
        let mut seen = BTreeSet::new();
        let mut more = false;
        let mut fetched = 0;
        for index in 0..pages {
            let number = first
                .checked_add(index as u32)
                .ok_or_else(|| migu_invalid_request("Migu MV page overflow"))?;
            let page = if let Some(id) = artist {
                self.client.mv_artist_page(id, number, &mut budget).await?
            } else {
                let (keyword, order) = search.expect("private MV window requires artist or search");
                self.client
                    .mv_search(keyword, order, number, &mut budget)
                    .await?
            };
            fetched += 1;
            for video in &page.items {
                if !seen.insert(video.id.clone()) {
                    return Err(migu_upstream_error(
                        "Migu MV pages repeated a resource identity",
                    ));
                }
            }
            let local_skip = if index == 0 { skip } else { 0 };
            let available = page.items.len().saturating_sub(local_skip);
            let take = available.min(limit as usize - items.len());
            more = available > take || page.more;
            items.extend(page.items.into_iter().skip(local_skip).take(take));
            if items.len() == limit as usize || !page.more {
                break;
            }
        }
        let end = offset + items.len() as u32;
        let mut extensions = Extensions::from([
            (
                "backend".into(),
                json!(if artist.is_some() {
                    "official_artist_mv_v1"
                } else {
                    "bmw_mv_search_v1"
                }),
            ),
            ("upstream_page_size".into(), json!(width)),
            ("upstream_pages_fetched".into(), json!(fetched)),
            ("catalogue_scope".into(), json!("public")),
        ]);
        if let Some(id) = artist {
            extensions.insert("source_artist_id".into(), json!(id));
            extensions.insert("kind".into(), json!("mv"));
        }
        if let Some((_, order)) = search {
            extensions.insert("upstream_order".into(), json!(order));
        }
        Ok(Page {
            items,
            pagination: PageMeta {
                limit,
                offset,
                total: None,
                has_more: more,
                next_offset: more.then_some(end),
                extensions,
            },
        })
    }
}

#[cfg(test)]
mod tests;
