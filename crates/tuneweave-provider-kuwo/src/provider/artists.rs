use super::*;
use crate::client::artists::{ArtistPage, PAGE_SIZE};
use tuneweave_core::{
    Album, Artist, ArtistArea, ArtistCatalog, ArtistCatalogFilterOption, ArtistCatalogFilters,
    ArtistCatalogRequest, ArtistCategory, ArtistGenre, ArtistOverview, ArtistTrackListRequest,
    ArtistTrackOrder, ArtistVideoListRequest, Video, VideoKind,
};

fn validate(id: &str, account: Option<&str>) -> Result<()> {
    if account.is_some()
        || id
            .parse::<u64>()
            .ok()
            .is_none_or(|number| number == 0 || number.to_string() != id)
    {
        return Err(kuwo_invalid_request(
            "Kuwo public artist requires a canonical positive ID and no account",
        ));
    }
    Ok(())
}
fn validate_page(id: &str, request: &PageRequest) -> Result<()> {
    validate(id, request.account.as_deref())?;
    if !(1..=100).contains(&request.limit) || request.offset.checked_add(request.limit).is_none() {
        return Err(kuwo_invalid_request(
            "Kuwo artist pagination is outside the supported range",
        ));
    }
    Ok(())
}

fn validate_catalog_request(request: &ArtistCatalogRequest) -> Result<()> {
    if request.account.is_some()
        || request.area != ArtistArea::All
        || request.category != ArtistCategory::All
        || request.genre != ArtistGenre::All
    {
        return Err(kuwo_invalid_request(
            "Kuwo artist catalogue currently exposes only the anonymous Baicheng anchor directory with default filters",
        ));
    }
    Ok(())
}

trait Identity {
    fn id(&self) -> &str;
}
impl Identity for Track {
    fn id(&self) -> &str {
        &self.id
    }
}
impl Identity for Album {
    fn id(&self) -> &str {
        &self.id
    }
}
impl Identity for Video {
    fn id(&self) -> &str {
        &self.id
    }
}

struct Window<T> {
    first: u32,
    skip: usize,
    wanted: usize,
    total: u64,
    items: Vec<T>,
    seen: BTreeSet<String>,
    fetched: u32,
}
impl<T: Identity> Window<T> {
    fn new(request: &PageRequest, expected: Option<u64>, seed: ArtistPage<T>) -> Result<Self> {
        if expected.is_some_and(|n| n != seed.total) {
            return Err(kuwo_upstream_error(
                "Kuwo artist detail and catalogue counts disagree",
            ));
        }
        let mut value = Self {
            first: request.offset / PAGE_SIZE + 1,
            skip: (request.offset % PAGE_SIZE) as usize,
            wanted: request.limit as usize,
            total: seed.total,
            items: vec![],
            seen: BTreeSet::new(),
            fetched: 0,
        };
        value.append(1, seed)?;
        Ok(value)
    }
    fn append(&mut self, page: u32, result: ArtistPage<T>) -> Result<()> {
        if result.total != self.total {
            return Err(kuwo_upstream_error(
                "Kuwo artist catalogue total changed during pagination",
            ));
        }
        for item in &result.items {
            if !self.seen.insert(item.id().to_owned()) {
                return Err(kuwo_upstream_error(
                    "Kuwo artist catalogue repeated a result identity",
                ));
            }
        }
        self.fetched += 1;
        if page >= self.first {
            self.items.extend(
                result
                    .items
                    .into_iter()
                    .skip(if page == self.first { self.skip } else { 0 })
                    .take(self.wanted - self.items.len()),
            );
        }
        Ok(())
    }
    fn remaining_pages(&self) -> std::ops::Range<u32> {
        let pages = (self.skip as u32 + self.wanted as u32).div_ceil(PAGE_SIZE);
        let last = self
            .total
            .div_ceil(u64::from(PAGE_SIZE))
            .min(u64::from(u32::MAX)) as u32;
        // Page one was always read to establish the real total. In particular,
        // the upstream album endpoint reports zero total for out-of-range pages.
        self.first.max(2)..(self.first + pages).min(last.saturating_add(1))
    }
    fn finish(self, request: &PageRequest, id: &str, kind: &str) -> Page<T> {
        let end = request.offset + self.items.len() as u32;
        let more = u64::from(end) < self.total;
        Page {
            items: self.items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(self.total),
                has_more: more,
                next_offset: more.then_some(end),
                extensions: Extensions::from([
                    ("backend".into(), json!("current_web_artist_catalogue")),
                    ("source_artist_id".into(), json!(id)),
                    ("source_type".into(), json!(kind)),
                    ("order".into(), json!("platform_default")),
                    ("upstream_page_size".into(), json!(PAGE_SIZE)),
                    ("upstream_pages_fetched".into(), json!(self.fetched)),
                ]),
            },
        }
    }
}

impl KuwoProvider {
    pub(super) async fn read_artist_catalog(
        &self,
        request: &ArtistCatalogRequest,
    ) -> Result<ArtistCatalog> {
        validate_catalog_request(request)?;
        let artists = self.client.bcsy_artist_catalog().await?;
        Ok(ArtistCatalog {
            platform: Platform::Kuwo,
            area: ArtistArea::All,
            category: ArtistCategory::All,
            genre: ArtistGenre::All,
            featured_artists: Vec::new(),
            artists,
            filters: ArtistCatalogFilters {
                areas: Vec::<ArtistCatalogFilterOption>::new(),
                categories: Vec::new(),
                genres: Vec::new(),
                initials: Vec::new(),
                extensions: Extensions::from([("available_filters".into(), json!(false))]),
            },
            extensions: Extensions::from([
                ("backend".into(), json!("native_bcsy")),
                ("catalog_scope".into(), json!("baicheng_sound_anchors")),
                ("pagination".into(), json!("single_upstream_array")),
            ]),
        })
    }

    pub(super) async fn read_artist_videos(
        &self,
        id: &str,
        request: &ArtistVideoListRequest,
    ) -> Result<Page<Video>> {
        let page = PageRequest {
            limit: request.limit,
            offset: request.offset,
            account: request.account.clone(),
        };
        validate_page(id, &page)?;
        if request.kind != VideoKind::Mv || request.cursor.is_some() || request.order.is_some() {
            return Err(kuwo_invalid_request(
                "Kuwo artist videos require explicit kind=mv, offset pagination and the default order",
            ));
        }
        let artist = self.client.artist_info(id).await?;
        let seed = self.client.artist_mvs_page(&artist, 1).await?;
        let mut window = Window::new(&page, artist.mv_count, seed)?;
        for number in window.remaining_pages() {
            window.append(number, self.client.artist_mvs_page(&artist, number).await?)?;
        }
        let mut result = window.finish(&page, id, "mvs");
        result
            .pagination
            .extensions
            .insert("kind".into(), json!("mv"));
        Ok(result)
    }
    pub(super) async fn read_artist(&self, id: &str, account: Option<&str>) -> Result<Artist> {
        validate(id, account)?;
        self.client.artist_info(id).await
    }
    pub(super) async fn read_artist_overview(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<ArtistOverview> {
        validate(id, account)?;
        let artist = self.client.artist_info(id).await?;
        let seed = self.client.artist_tracks_page(&artist, 1).await?;
        let window = Window::new(&PageRequest::new(10, 0), artist.track_count, seed)?;
        Ok(ArtistOverview {
            artist,
            has_more_tracks: window.total > window.items.len() as u64,
            featured_tracks: window.items,
            extensions: Extensions::from([
                ("backend".into(), json!("current_web_artist_overview")),
                ("preview_limit".into(), json!(10)),
            ]),
        })
    }
    pub(super) async fn read_artist_tracks(
        &self,
        id: &str,
        request: &ArtistTrackListRequest,
    ) -> Result<Page<Track>> {
        let page = PageRequest {
            limit: request.limit,
            offset: request.offset,
            account: request.account.clone(),
        };
        validate_page(id, &page)?;
        if request.order != ArtistTrackOrder::PlatformDefault {
            return Err(kuwo_invalid_request(
                "Kuwo artist tracks require explicit platform_default order",
            ));
        }
        let artist = self.client.artist_info(id).await?;
        let seed = self.client.artist_tracks_page(&artist, 1).await?;
        let mut window = Window::new(&page, artist.track_count, seed)?;
        for number in window.remaining_pages() {
            window.append(
                number,
                self.client.artist_tracks_page(&artist, number).await?,
            )?;
        }
        Ok(window.finish(&page, id, "tracks"))
    }
    pub(super) async fn read_artist_albums(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<Album>> {
        validate_page(id, request)?;
        let artist = self.client.artist_info(id).await?;
        let seed = self.client.artist_albums_page(&artist, 1).await?;
        let mut window = Window::new(request, artist.album_count, seed)?;
        for number in window.remaining_pages() {
            window.append(
                number,
                self.client.artist_albums_page(&artist, number).await?,
            )?;
        }
        Ok(window.finish(request, id, "albums"))
    }
}

#[cfg(test)]
#[path = "artists/bcsy_tests.rs"]
mod bcsy_tests;
