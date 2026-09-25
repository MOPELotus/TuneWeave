use super::*;
use crate::client::artists::Catalogue;
use tuneweave_core::{Album, Artist, ArtistOverview, ArtistTrackListRequest, ArtistTrackOrder};

fn validate(id: &str, account: Option<&str>) -> Result<u64> {
    if account.is_some() {
        return Err(kugou_invalid_request(
            "KuGou public artists do not accept an account",
        ));
    }
    id.parse::<u64>()
        .ok()
        .filter(|n| *n > 0 && n.to_string() == id)
        .ok_or_else(|| {
            kugou_invalid_request("KuGou artist ID must be a canonical positive decimal")
        })
}
fn pagination(limit: u32, offset: u32) -> Result<()> {
    if !(1..=100).contains(&limit) || offset.checked_add(limit).is_none() {
        return Err(kugou_invalid_request("Invalid KuGou artist pagination"));
    }
    Ok(())
}
impl KugouProvider {
    pub(super) async fn read_artists(
        &self,
        request: &tuneweave_core::ArtistListRequest,
    ) -> Result<Page<Artist>> {
        self.require_public_source()?;
        pagination(request.limit, request.offset)?;
        let initial = request.initial.as_deref();
        if initial.is_some_and(|value| {
            value != "#" && !(value.len() == 1 && value.as_bytes()[0].is_ascii_uppercase())
        }) {
            return Err(kugou_invalid_request(
                "KuGou artist initial must be A-Z or #",
            ));
        }
        let directory = self
            .client
            .public_artist_catalog(&tuneweave_core::ArtistCatalogRequest {
                account: request.account.clone(),
                area: request.area,
                category: request.category,
                genre: request.genre,
            })
            .await?;
        // The hot view is separate and may repeat artists in these initial groups.
        let artists: Vec<_> = directory
            .artists
            .into_iter()
            .filter(|artist| match initial {
                None => true,
                Some(value) => {
                    artist
                        .extensions
                        .get("directory_group")
                        .and_then(serde_json::Value::as_str)
                        == Some(value)
                }
            })
            .collect();
        let total = artists.len() as u64;
        let items: Vec<_> = artists
            .into_iter()
            .skip(request.offset as usize)
            .take(request.limit as usize)
            .collect();
        let next = request.offset + items.len() as u32;
        let has_more = u64::from(next) < total;
        let mut extensions = directory.extensions;
        extensions.insert("initial".into(), json!(initial));
        extensions.insert("pagination_source".into(), json!("local_directory_slice"));
        extensions.insert("total_scope".into(), json!("selected_directory_groups"));
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(total),
                has_more,
                next_offset: has_more.then_some(next),
                extensions,
            },
        })
    }

    pub(super) async fn read_artist_catalog(
        &self,
        request: &tuneweave_core::ArtistCatalogRequest,
    ) -> Result<tuneweave_core::ArtistCatalog> {
        self.require_public_source()?;
        self.client.public_artist_catalog(request).await
    }

    pub(super) async fn read_artist(&self, id: &str, account: Option<&str>) -> Result<Artist> {
        self.require_public_source()?;
        self.client.artist_metadata(validate(id, account)?).await
    }
    pub(super) async fn read_artist_stats(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<tuneweave_core::ArtistStats> {
        let artist = self.read_artist(id, account).await?;
        let follower_count = artist
            .extensions
            .get("fans_count")
            .and_then(serde_json::Value::as_u64);
        let video_counts = artist
            .mv_count
            .into_iter()
            .map(|count| tuneweave_core::ArtistContentCount {
                category: Some("mv".into()),
                count,
                extensions: Extensions::from([(
                    "catalog_scope".into(),
                    json!("official_artist_mv_catalogue"),
                )]),
            })
            .collect();
        Ok(tuneweave_core::ArtistStats {
            artist_ref: artist.resource_ref,
            followed: None,
            follower_count,
            video_counts,
            online_concert_count: None,
            extensions: Extensions::from([(
                "backend".into(),
                json!("official_public_artist_metadata"),
            )]),
        })
    }
    pub(super) async fn read_artist_overview(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<ArtistOverview> {
        self.require_public_source()?;
        let catalogue = self
            .client
            .complete_artist_tracks(validate(id, account)?, 1)
            .await?;
        let featured_tracks: Vec<_> = catalogue.items.into_iter().take(10).collect();
        Ok(ArtistOverview {
            artist: catalogue.artist,
            has_more_tracks: catalogue.total > featured_tracks.len() as u64,
            featured_tracks,
            extensions: Extensions::from([
                ("order".into(), json!("hot")),
                ("complete_snapshot".into(), json!(true)),
                ("upstream_pages_fetched".into(), json!(catalogue.pages)),
            ]),
        })
    }
    pub(super) async fn read_artist_tracks(
        &self,
        id: &str,
        request: &ArtistTrackListRequest,
    ) -> Result<Page<Track>> {
        self.require_public_source()?;
        let id = validate(id, request.account.as_deref())?;
        pagination(request.limit, request.offset)?;
        let (sort, order) = match request.order {
            ArtistTrackOrder::Hot => (1, "hot"),
            ArtistTrackOrder::Time | ArtistTrackOrder::PlatformDefault => (2, "time"),
        };
        let c = self.client.complete_artist_tracks(id, sort).await?;
        Ok(page(c, request.limit, request.offset, "tracks", order))
    }
    pub(super) async fn read_artist_top_tracks(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<Page<Track>> {
        // Match the existing overview's first ten hot-order tracks. This is a
        // TuneWeave projection of the validated catalogue, not a separate chart.
        let mut result = self
            .read_artist_tracks(
                id,
                &ArtistTrackListRequest {
                    account: account.map(str::to_owned),
                    limit: 10,
                    offset: 0,
                    order: ArtistTrackOrder::Hot,
                },
            )
            .await?;
        let source_total = result.pagination.total;
        result.pagination.total = Some(result.items.len() as u64);
        result.pagination.has_more = false;
        result.pagination.next_offset = None;
        result.pagination.extensions.extend([
            ("result_scope".into(), json!("hot_catalogue_first_10")),
            ("source_catalogue_total".into(), json!(source_total)),
        ]);
        Ok(result)
    }

    pub(super) async fn read_artist_albums(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<Album>> {
        self.require_public_source()?;
        let id = validate(id, request.account.as_deref())?;
        pagination(request.limit, request.offset)?;
        let c = self.client.complete_artist_albums(id).await?;
        Ok(page(c, request.limit, request.offset, "albums", "time"))
    }
}
fn page<T>(c: Catalogue<T>, limit: u32, offset: u32, kind: &str, order: &str) -> Page<T> {
    let items: Vec<_> = c
        .items
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect();
    let end = u64::from(offset) + items.len() as u64;
    let has_more = end < c.total;
    Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total: Some(c.total),
            has_more,
            next_offset: has_more.then_some(end as u32),
            extensions: Extensions::from([
                (
                    "backend".into(),
                    json!("official_artist_complete_catalogue"),
                ),
                ("artist_id".into(), json!(c.artist.id)),
                ("kind".into(), json!(kind)),
                ("order".into(), json!(order)),
                ("complete_snapshot".into(), json!(true)),
                ("upstream_pages_fetched".into(), json!(c.pages)),
            ]),
        },
    }
}

#[cfg(test)]
mod tests;
