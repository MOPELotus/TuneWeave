use std::{collections::BTreeSet, fmt};

use async_trait::async_trait;
use serde_json::json;
use tuneweave_core::{
    Capability, Extensions, MusicProvider, Page, PageMeta, Platform, Result, SearchKind,
    SearchQuery, SearchVariant, Track, TuneWeaveError,
};

use crate::client::{KuwoClient, KuwoConfig};

const UPSTREAM_PAGE_SIZE: u32 = 100;

#[derive(Clone)]
pub struct KuwoProvider {
    client: KuwoClient,
}

impl fmt::Debug for KuwoProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KuwoProvider")
            .finish_non_exhaustive()
    }
}

impl KuwoProvider {
    pub fn new(config: KuwoConfig) -> Result<Self> {
        Ok(Self {
            client: KuwoClient::new(&config)?,
        })
    }

    #[must_use]
    pub const fn from_client(client: KuwoClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl MusicProvider for KuwoProvider {
    fn platform(&self) -> Platform {
        Platform::Kuwo
    }

    fn name(&self) -> &'static str {
        "Kuwo Music"
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::SearchTracks])
    }

    async fn search(&self, query: &SearchQuery) -> Result<Page<Track>> {
        validate_search_query(query)?;
        let upstream_page = query.offset / UPSTREAM_PAGE_SIZE;
        let first_skip = usize::try_from(query.offset % UPSTREAM_PAGE_SIZE)
            .map_err(|_| kuwo_invalid_request("Kuwo search offset is too large"))?;
        let requested = usize::try_from(query.limit)
            .map_err(|_| kuwo_invalid_request("Kuwo search limit is too large"))?;

        let first = self
            .client
            .search_tracks_page(query.query.trim(), upstream_page, UPSTREAM_PAGE_SIZE)
            .await?;
        let total = first.total;
        let mut tracks = first
            .tracks
            .into_iter()
            .skip(first_skip)
            .collect::<Vec<_>>();
        let mut fetched_pages = 1_u32;
        let consumed_after_first =
            u64::from(query.offset).saturating_add(u64::try_from(tracks.len()).unwrap_or(u64::MAX));
        if tracks.len() < requested && consumed_after_first < total {
            let next_page = upstream_page.checked_add(1).ok_or_else(|| {
                kuwo_invalid_request("Kuwo search offset exceeds the upstream page range")
            })?;
            let second = self
                .client
                .search_tracks_page(query.query.trim(), next_page, UPSTREAM_PAGE_SIZE)
                .await?;
            if second.total != total {
                return Err(kuwo_upstream_error(
                    "Kuwo search total changed during pagination",
                ));
            }
            fetched_pages = fetched_pages.saturating_add(1);
            tracks.extend(second.tracks);
        }
        tracks.truncate(requested);

        let returned = u32::try_from(tracks.len()).unwrap_or(u32::MAX);
        let consumed = query.offset.saturating_add(returned);
        let has_more = u64::from(consumed) < total;
        let mut extensions = Extensions::new();
        extensions.insert(
            "backend".to_owned(),
            json!("current_web_search_music_by_keyword"),
        );
        extensions.insert("upstream_page_size".to_owned(), json!(UPSTREAM_PAGE_SIZE));
        extensions.insert("upstream_pages_fetched".to_owned(), json!(fetched_pages));
        Ok(Page {
            items: tracks,
            pagination: PageMeta {
                limit: query.limit,
                offset: query.offset,
                total: Some(total),
                next_offset: (has_more && returned > 0).then_some(consumed),
                has_more,
                extensions,
            },
        })
    }
}

fn validate_search_query(query: &SearchQuery) -> Result<()> {
    if query.kind != SearchKind::Track {
        return Err(TuneWeaveError::unsupported(
            Platform::Kuwo,
            capability_for_search(query.kind),
        ));
    }
    if query.variant != SearchVariant::Default {
        return Err(kuwo_invalid_request(
            "Kuwo public track search only supports the default backend",
        ));
    }
    if query.account.is_some() {
        return Err(kuwo_invalid_request(
            "Kuwo public track search does not accept an account",
        ));
    }
    if query.search_id.is_some() || query.highlight || !query.selectors.is_empty() {
        return Err(kuwo_invalid_request(
            "Kuwo public track search does not accept search_id, highlight, or selectors",
        ));
    }
    if query.video_filters.is_some() {
        return Err(kuwo_invalid_request(
            "Kuwo track search does not accept video filters",
        ));
    }
    let keyword = query.query.trim();
    if keyword.is_empty() || keyword.len() > 512 || keyword.chars().any(char::is_control) {
        return Err(kuwo_invalid_request(
            "Kuwo search query must contain 1 to 512 non-control UTF-8 bytes",
        ));
    }
    if !(1..=100).contains(&query.limit) {
        return Err(kuwo_invalid_request(
            "Kuwo search limit must be between 1 and 100",
        ));
    }
    Ok(())
}

fn capability_for_search(kind: SearchKind) -> Capability {
    match kind {
        SearchKind::Track => Capability::SearchTracks,
        SearchKind::Album => Capability::SearchAlbums,
        SearchKind::Artist => Capability::SearchArtists,
        SearchKind::Playlist => Capability::SearchPlaylists,
        SearchKind::User => Capability::SearchUsers,
        SearchKind::Mv => Capability::SearchMvs,
        SearchKind::Lyric => Capability::SearchLyrics,
        SearchKind::RadioStation => Capability::SearchRadioStations,
        SearchKind::Podcast => Capability::SearchPodcasts,
        SearchKind::Video => Capability::SearchVideos,
        SearchKind::Mixed => Capability::SearchMixed,
        SearchKind::Voice => Capability::SearchVoices,
        SearchKind::Ringtone => Capability::SearchRingtones,
    }
}

fn kuwo_invalid_request(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Kuwo)
}

fn kuwo_upstream_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(tuneweave_core::ErrorCode::UpstreamError, message)
        .with_platform(Platform::Kuwo)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search_query() -> SearchQuery {
        SearchQuery::tracks("反方向的钟", 30, 0)
    }

    #[test]
    fn provider_advertises_only_implemented_public_capabilities() {
        let provider = KuwoProvider::new(KuwoConfig::default()).expect("create Kuwo provider");
        assert_eq!(provider.platform(), Platform::Kuwo);
        assert_eq!(
            provider.capabilities(),
            BTreeSet::from([Capability::SearchTracks])
        );
    }

    #[test]
    fn search_validation_rejects_foreign_options_before_network_access() {
        let valid = search_query();
        assert!(validate_search_query(&valid).is_ok());

        let mut account = valid.clone();
        account.account = Some("default".to_owned());
        assert!(validate_search_query(&account).is_err());

        let mut legacy = valid.clone();
        legacy.variant = SearchVariant::Legacy;
        assert!(validate_search_query(&legacy).is_err());

        let mut album = valid.clone();
        album.kind = SearchKind::Album;
        assert!(validate_search_query(&album).is_err());

        let mut selector = valid.clone();
        selector.highlight = true;
        assert!(validate_search_query(&selector).is_err());

        let mut empty = valid.clone();
        empty.query = " \t".to_owned();
        assert!(validate_search_query(&empty).is_err());

        let mut too_many = valid;
        too_many.limit = 101;
        assert!(validate_search_query(&too_many).is_err());
    }
}
