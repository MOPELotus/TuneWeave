use super::*;
use crate::client::artists::{self, ArtistAlbum, ArtistOperation};
use tuneweave_core::{Artist, ArtistOverview, ArtistTrackListRequest, ArtistTrackOrder};

const MAX_PAGES: u32 = 128;
fn page<T>(
    items: Vec<T>,
    r: &PageRequest,
    uid: &str,
    raw: usize,
    pages: u32,
    kind: &str,
) -> Page<T> {
    let total = items.len() as u64;
    let items: Vec<_> = items
        .into_iter()
        .skip(r.offset as usize)
        .take(r.limit as usize)
        .collect();
    let end = u64::from(r.offset) + items.len() as u64;
    Page {
        items,
        pagination: PageMeta {
            limit: r.limit,
            offset: r.offset,
            total: Some(total),
            has_more: end < total,
            next_offset: (end < total).then_some(end as u32),
            extensions: Extensions::from([
                (
                    "backend".into(),
                    json!("official_pc_complete_artist_catalog"),
                ),
                ("source_artist_id".into(), json!(uid)),
                ("source_type".into(), json!(kind)),
                ("complete_read".into(), json!(true)),
                ("upstream_raw_count".into(), json!(raw)),
                ("upstream_pages_fetched".into(), json!(pages)),
            ]),
        },
    }
}
fn complete(count: usize, total: Option<u64>, more: bool, page: u32, width: usize) -> Result<bool> {
    if total.is_some_and(|n| {
        n < count as u64 || (!more && n != count as u64) || (more && n == count as u64)
    }) {
        return Err(migu_upstream_error(
            "Migu artist catalogue disagrees with its reported count",
        ));
    }
    if count > MAX_PAGES as usize * width || more && page == MAX_PAGES {
        return Err(migu_upstream_error(
            "Migu artist catalogue exceeded its complete-read budget",
        ));
    }
    Ok(!more)
}
impl MiguProvider {
    fn validate_artist_source(&self, uid: &str, account: Option<&str>) -> Result<()> {
        self.require_public_source()?;
        if account.is_some()
            || uid.is_empty()
            || uid.len() > 64
            || uid.starts_with('0')
            || !uid.bytes().all(|v| v.is_ascii_digit())
        {
            return Err(migu_invalid_request(
                "Migu public artist requires a canonical positive ID and no account",
            ));
        }
        Ok(())
    }
    fn validate_artist_page(&self, uid: &str, r: &PageRequest) -> Result<()> {
        self.validate_artist_source(uid, r.account.as_deref())?;
        if !(1..=100).contains(&r.limit) || r.offset.checked_add(r.limit).is_none() {
            return Err(migu_invalid_request("Migu artist pagination is invalid"));
        }
        Ok(())
    }
    async fn append_artist_biography(&self, artist: &mut Artist) -> Result<()> {
        let sections = self.client.artist_biography(&artist.id).await?;
        artist.description = sections
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join("\n\n");
        artist.biography_sections = sections;
        Ok(())
    }
    pub(super) async fn read_artist(&self, uid: &str, account: Option<&str>) -> Result<Artist> {
        self.validate_artist_source(uid, account)?;
        let mut artist = self.client.artist_info(uid).await?;
        self.append_artist_biography(&mut artist).await?;
        Ok(artist)
    }
    async fn complete_artist_tracks(&self, uid: &str) -> Result<(Artist, Vec<Track>, u32)> {
        let artist = self.client.artist_info(uid).await?;
        let total = artist.track_count;
        if total.is_some_and(|n| n > u64::from(MAX_PAGES) * 50) {
            return Err(migu_upstream_error(
                "Migu artist track count exceeds the complete-read budget",
            ));
        }
        let mut items = Vec::new();
        let mut seen = BTreeSet::new();
        for number in 1..=MAX_PAGES {
            let data = self
                .client
                .artist_response(ArtistOperation::Songs, uid, number)
                .await?;
            let response = artists::songs(data, uid, number)?;
            for item in &response.items {
                if !seen.insert(item.id.clone()) {
                    return Err(migu_upstream_error(
                        "Migu artist catalogue repeated a track identity",
                    ));
                }
            }
            items.extend(response.items);
            if complete(items.len(), total, response.has_more, number, 50)? {
                return Ok((artist, items, number));
            }
        }
        Err(migu_upstream_error("Migu artist tracks did not terminate"))
    }
    pub(super) async fn read_artist_tracks(
        &self,
        uid: &str,
        r: &ArtistTrackListRequest,
    ) -> Result<Page<Track>> {
        let request = PageRequest {
            limit: r.limit,
            offset: r.offset,
            account: r.account.clone(),
        };
        self.validate_artist_page(uid, &request)?;
        if r.order != ArtistTrackOrder::PlatformDefault {
            return Err(migu_invalid_request(
                "Migu artist tracks support only explicit platform_default order",
            ));
        }
        let (_, items, pages) = self.complete_artist_tracks(uid).await?;
        let count = items.len();
        let mut result = page(items, &request, uid, count, pages, "tracks");
        result
            .pagination
            .extensions
            .insert("order".into(), json!("platform_default"));
        Ok(result)
    }
    pub(super) async fn read_artist_overview(
        &self,
        uid: &str,
        account: Option<&str>,
    ) -> Result<ArtistOverview> {
        self.validate_artist_source(uid, account)?;
        let (mut artist, items, pages) = self.complete_artist_tracks(uid).await?;
        let has_more_tracks = items.len() > 10;
        self.append_artist_biography(&mut artist).await?;
        Ok(ArtistOverview {
            artist,
            featured_tracks: items.into_iter().take(10).collect(),
            has_more_tracks,
            extensions: Extensions::from([
                ("order".into(), json!("platform_default")),
                ("upstream_pages_fetched".into(), json!(pages)),
                ("complete_track_read".into(), json!(true)),
            ]),
        })
    }
    async fn complete_artist_albums(&self, uid: &str) -> Result<(Vec<ArtistAlbum>, u32)> {
        let artist = self.client.artist_info(uid).await?;
        let total = artist.album_count;
        if total.is_some_and(|n| n > u64::from(MAX_PAGES) * 10) {
            return Err(migu_upstream_error(
                "Migu artist album count exceeds the complete-read budget",
            ));
        }
        let mut items = Vec::new();
        let mut seen = BTreeSet::new();
        for number in 1..=MAX_PAGES {
            let data = self
                .client
                .artist_response(ArtistOperation::Albums, uid, number)
                .await?;
            let response = artists::albums(data, uid, number)?;
            for item in &response.items {
                let (kind, id) = item.identity();
                if !seen.insert((kind, id.to_owned())) {
                    return Err(migu_upstream_error(
                        "Migu artist catalogue repeated a typed album identity",
                    ));
                }
            }
            items.extend(response.items);
            if complete(items.len(), total, response.has_more, number, 10)? {
                return Ok((items, number));
            }
        }
        Err(migu_upstream_error("Migu artist albums did not terminate"))
    }
    pub(super) async fn read_artist_albums(
        &self,
        uid: &str,
        r: &PageRequest,
    ) -> Result<Page<Album>> {
        self.validate_artist_page(uid, r)?;
        let (items, pages) = self.complete_artist_albums(uid).await?;
        let count = items.len();
        Ok(page(
            items
                .into_iter()
                .filter_map(|v| match v {
                    ArtistAlbum::Ordinary(v) => Some(v),
                    _ => None,
                })
                .collect(),
            r,
            uid,
            count,
            pages,
            "albums",
        ))
    }
    pub(super) async fn read_artist_digital_albums(
        &self,
        uid: &str,
        r: &PageRequest,
    ) -> Result<Page<DigitalAlbum>> {
        self.validate_artist_page(uid, r)?;
        let (items, pages) = self.complete_artist_albums(uid).await?;
        let count = items.len();
        Ok(page(
            items
                .into_iter()
                .filter_map(|v| match v {
                    ArtistAlbum::Digital(v) => Some(v),
                    _ => None,
                })
                .collect(),
            r,
            uid,
            count,
            pages,
            "digital_albums",
        ))
    }
}

#[cfg(test)]
mod tests;
