use std::{collections::BTreeSet, fmt};

use async_trait::async_trait;
use serde_json::json;
use tuneweave_core::{
    Capability, Extensions, Lyrics, LyricsRequest, MediaDownload, MediaStream, MusicProvider, Page,
    PageMeta, Platform, Result, SearchKind, SearchQuery, SearchVariant, StreamRequest, Track,
    TrackAvailability, TrackAvailabilityRequest, TuneWeaveError,
};

use crate::client::{MiguClient, MiguConfig, MiguSearchCondition};

const UPSTREAM_PAGE_SIZE: u32 = 20;
const MAX_UPSTREAM_PAGES: u32 = 6;

#[derive(Clone)]
pub struct MiguProvider {
    client: MiguClient,
}

impl fmt::Debug for MiguProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MiguProvider")
            .finish_non_exhaustive()
    }
}

impl MiguProvider {
    pub fn new(config: MiguConfig) -> Result<Self> {
        Ok(Self {
            client: MiguClient::new(&config)?,
        })
    }

    #[must_use]
    pub const fn from_client(client: MiguClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl MusicProvider for MiguProvider {
    fn platform(&self) -> Platform {
        Platform::Migu
    }

    fn name(&self) -> &'static str {
        "Migu Music"
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::AudioDownload,
            Capability::AudioStream,
            Capability::Lyrics,
            Capability::SearchTracks,
            Capability::TrackAvailability,
            Capability::TrackDetail,
        ])
    }

    async fn search(&self, query: &SearchQuery) -> Result<Page<Track>> {
        validate_search_query(query)?;
        let start_page = query.offset / UPSTREAM_PAGE_SIZE + 1;
        let first_skip = usize::try_from(query.offset % UPSTREAM_PAGE_SIZE)
            .map_err(|_| migu_invalid_request("Migu search offset is too large"))?;
        let requested = usize::try_from(query.limit)
            .map_err(|_| migu_invalid_request("Migu search limit is too large"))?;
        let required = u32::try_from(first_skip.saturating_add(requested)).unwrap_or(u32::MAX);
        let page_budget = required
            .saturating_add(UPSTREAM_PAGE_SIZE - 1)
            .checked_div(UPSTREAM_PAGE_SIZE)
            .unwrap_or(MAX_UPSTREAM_PAGES)
            .clamp(1, MAX_UPSTREAM_PAGES);

        let mut tracks = Vec::with_capacity(requested);
        let mut sequences = Vec::new();
        let mut conditions: Vec<MiguSearchCondition> = Vec::new();
        let mut fetched_pages = 0_u32;
        let mut has_more = false;
        for page_index in 0..page_budget {
            let page_number = start_page.checked_add(page_index).ok_or_else(|| {
                migu_invalid_request("Migu search offset exceeds the upstream page range")
            })?;
            let page = self
                .client
                .search_tracks_page(query.query.trim(), page_number)
                .await?;
            fetched_pages = fetched_pages.saturating_add(1);
            if page.has_next && page.tracks.is_empty() {
                return Err(migu_upstream_error(
                    "Migu search reported another page without returning any tracks",
                ));
            }
            if let Some(sequence) = page.sequence {
                sequences.push(sequence);
            }
            if conditions.is_empty() {
                conditions = page.conditions;
            }
            let skip = if page_index == 0 { first_skip } else { 0 };
            let mut unconsumed = false;
            for track in page.tracks.into_iter().skip(skip) {
                if tracks.len() == requested {
                    unconsumed = true;
                    break;
                }
                tracks.push(track);
            }
            has_more = unconsumed || page.has_next;
            if tracks.len() == requested || !page.has_next {
                break;
            }
        }

        let returned = u32::try_from(tracks.len()).unwrap_or(u32::MAX);
        let consumed = query.offset.saturating_add(returned);
        let mut extensions = Extensions::new();
        extensions.insert("backend".to_owned(), json!("bmw_song_search_v1"));
        extensions.insert("upstream_page_size".to_owned(), json!(UPSTREAM_PAGE_SIZE));
        extensions.insert("upstream_pages_fetched".to_owned(), json!(fetched_pages));
        if !sequences.is_empty() {
            extensions.insert("upstream_sequences".to_owned(), json!(sequences));
        }
        if !conditions.is_empty() {
            extensions.insert("conditions".to_owned(), json!(conditions));
        }
        Ok(Page {
            items: tracks,
            pagination: PageMeta {
                limit: query.limit,
                offset: query.offset,
                total: None,
                next_offset: (has_more && returned > 0).then_some(consumed),
                has_more,
                extensions,
            },
        })
    }

    async fn track(&self, id: &str, account: Option<&str>) -> Result<Track> {
        if account.is_some() {
            return Err(migu_invalid_request(
                "Migu public track detail does not accept an account",
            ));
        }
        let content_id = parse_content_id(id)?;
        self.client.track_detail(content_id).await
    }

    async fn track_availability(
        &self,
        id: &str,
        request: &TrackAvailabilityRequest,
    ) -> Result<TrackAvailability> {
        validate_availability_request(request)?;
        let content_id = parse_content_id(id)?;
        self.client.track_availability(content_id, request).await
    }

    async fn lyrics(&self, id: &str, account: Option<&str>) -> Result<Lyrics> {
        if account.is_some() {
            return Err(migu_invalid_request(
                "Migu public lyrics do not accept an account",
            ));
        }
        let content_id = parse_content_id(id)?;
        self.client.lyrics(content_id).await
    }

    async fn lyrics_with_options(&self, id: &str, request: &LyricsRequest) -> Result<Lyrics> {
        validate_lyrics_request(request)?;
        let content_id = parse_content_id(id)?;
        self.client.lyrics(content_id).await
    }

    async fn stream(&self, track: &Track, request: &StreamRequest) -> Result<MediaStream> {
        self.client.stream(track, request).await
    }

    async fn download(&self, track: &Track, request: &StreamRequest) -> Result<MediaDownload> {
        self.client.download(track, request).await
    }
}

fn validate_availability_request(request: &TrackAvailabilityRequest) -> Result<()> {
    if request.account.is_some() {
        return Err(migu_invalid_request(
            "Migu public listening rights do not accept an account",
        ));
    }
    if request.bitrate == 0 || request.bitrate > 10_000_000 {
        return Err(migu_invalid_request(
            "Migu availability bitrate must be between 1 and 10000000",
        ));
    }
    Ok(())
}

fn validate_lyrics_request(request: &LyricsRequest) -> Result<()> {
    if request.account.is_some() {
        return Err(migu_invalid_request(
            "Migu public lyrics do not accept an account",
        ));
    }
    if request.song_type.is_some() || request.singing_annotations {
        return Err(migu_invalid_request(
            "Migu lyrics do not accept song_type or singing annotations",
        ));
    }
    Ok(())
}

fn parse_content_id(id: &str) -> Result<&str> {
    if id.is_empty() || id.len() > 64 || !id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(migu_invalid_request(
            "Migu track ID must be a canonical alphanumeric contentId",
        ));
    }
    Ok(id)
}

fn validate_search_query(query: &SearchQuery) -> Result<()> {
    if query.kind != SearchKind::Track {
        return Err(TuneWeaveError::unsupported(
            Platform::Migu,
            capability_for_search(query.kind),
        ));
    }
    if query.variant != SearchVariant::Default {
        return Err(migu_invalid_request(
            "Migu public track search only supports the default backend",
        ));
    }
    if query.account.is_some() {
        return Err(migu_invalid_request(
            "Migu public track search does not accept an account",
        ));
    }
    if query.search_id.is_some() || query.highlight || !query.selectors.is_empty() {
        return Err(migu_invalid_request(
            "Migu public track search does not accept search_id, highlight, or selectors",
        ));
    }
    if query.video_filters.is_some() {
        return Err(migu_invalid_request(
            "Migu track search does not accept video filters",
        ));
    }
    let keyword = query.query.trim();
    if keyword.is_empty() || keyword.len() > 512 || keyword.chars().any(char::is_control) {
        return Err(migu_invalid_request(
            "Migu search query must contain 1 to 512 non-control UTF-8 bytes",
        ));
    }
    if !(1..=100).contains(&query.limit) {
        return Err(migu_invalid_request(
            "Migu search limit must be between 1 and 100",
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

fn migu_invalid_request(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Migu)
}

fn migu_upstream_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(tuneweave_core::ErrorCode::UpstreamError, message)
        .with_platform(Platform::Migu)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search_query() -> SearchQuery {
        SearchQuery::tracks("反方向的钟", 30, 0)
    }

    #[test]
    fn provider_advertises_only_implemented_public_capabilities() {
        let provider = MiguProvider::new(MiguConfig::default()).expect("create Migu provider");
        assert_eq!(provider.platform(), Platform::Migu);
        assert_eq!(
            provider.capabilities(),
            BTreeSet::from([
                Capability::AudioDownload,
                Capability::AudioStream,
                Capability::Lyrics,
                Capability::SearchTracks,
                Capability::TrackAvailability,
                Capability::TrackDetail
            ])
        );
    }

    #[test]
    fn availability_rejects_accounts_and_invalid_bitrate_bounds() {
        assert!(validate_availability_request(&TrackAvailabilityRequest::default()).is_ok());
        assert!(validate_availability_request(&TrackAvailabilityRequest::new(1)).is_ok());
        assert!(validate_availability_request(&TrackAvailabilityRequest::new(10_000_000)).is_ok());
        assert!(validate_availability_request(&TrackAvailabilityRequest::new(0)).is_err());
        assert!(validate_availability_request(&TrackAvailabilityRequest::new(10_000_001)).is_err());
        let account = TrackAvailabilityRequest {
            account: Some("default".to_owned()),
            ..TrackAvailabilityRequest::default()
        };
        assert!(validate_availability_request(&account).is_err());
    }

    #[test]
    fn lyrics_accept_display_preferences_but_reject_foreign_protocol_options() {
        let rich = LyricsRequest {
            word_synced: true,
            translated: true,
            romanized: true,
            ..LyricsRequest::default()
        };
        assert!(validate_lyrics_request(&rich).is_ok());

        let mut account = rich.clone();
        account.account = Some("default".to_owned());
        assert!(validate_lyrics_request(&account).is_err());

        let mut song_type = rich.clone();
        song_type.song_type = Some(1);
        assert!(validate_lyrics_request(&song_type).is_err());

        let mut annotations = rich;
        annotations.singing_annotations = true;
        assert!(validate_lyrics_request(&annotations).is_err());
    }

    #[test]
    fn track_detail_requires_a_canonical_content_identity() {
        assert_eq!(
            parse_content_id("600908000007288315").expect("valid content ID"),
            "600908000007288315"
        );
        for id in [
            "",
            " 600908000007288315",
            "600908000007288315 ",
            "migu:600908000007288315",
            "id/path",
            "id?query",
            "你好",
        ] {
            assert!(parse_content_id(id).is_err(), "{id:?} must fail");
        }
    }

    #[test]
    fn public_search_rejects_unimplemented_or_silently_ignored_options() {
        let mut account = search_query();
        account.account = Some("default".to_owned());
        assert!(validate_search_query(&account).is_err());

        let mut variant = search_query();
        variant.variant = SearchVariant::Legacy;
        assert!(validate_search_query(&variant).is_err());

        let mut kind = search_query();
        kind.kind = SearchKind::Album;
        assert!(validate_search_query(&kind).is_err());

        let mut highlight = search_query();
        highlight.highlight = true;
        assert!(validate_search_query(&highlight).is_err());

        let mut excessive = search_query();
        excessive.limit = 101;
        assert!(validate_search_query(&excessive).is_err());
    }

    #[test]
    fn page_budget_covers_non_aligned_windows_without_becoming_unbounded() {
        for (offset, limit, expected) in [
            (0_u32, 1_u32, 1_u32),
            (19, 3, 2),
            (19, 100, 6),
            (20, 100, 5),
        ] {
            let skip = offset % UPSTREAM_PAGE_SIZE;
            let required = skip + limit;
            let budget = required
                .saturating_add(UPSTREAM_PAGE_SIZE - 1)
                .checked_div(UPSTREAM_PAGE_SIZE)
                .unwrap_or(MAX_UPSTREAM_PAGES)
                .clamp(1, MAX_UPSTREAM_PAGES);
            assert_eq!(budget, expected);
        }
    }

    #[tokio::test]
    #[ignore = "requires live Migu network access"]
    async fn live_provider_crosses_a_physical_page_boundary() {
        let provider = MiguProvider::new(MiguConfig::default()).expect("create Migu provider");
        let page = provider
            .search(&SearchQuery::tracks("周杰伦", 3, 19))
            .await
            .expect("live cross-page search");
        assert_eq!(page.items.len(), 3);
        assert_eq!(page.pagination.offset, 19);
        assert!(
            page.items
                .iter()
                .all(|track| track.resource_ref.platform() == Platform::Migu)
        );
    }

    #[tokio::test]
    #[ignore = "requires live Migu network access"]
    async fn live_provider_returns_strict_public_track_detail() {
        let provider = MiguProvider::new(MiguConfig::default()).expect("create Migu provider");
        let track = provider
            .track("600908000007288315", None)
            .await
            .expect("live Migu track detail");
        assert_eq!(track.resource_ref.to_string(), "migu:600908000007288315");
        assert_eq!(track.extensions["backend"], "resourceinfo_v1");
        assert!(!track.available_qualities.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires live Migu network access"]
    async fn live_provider_prefers_word_synced_mrc_over_plain_lrc() {
        let provider = MiguProvider::new(MiguConfig::default()).expect("create Migu provider");
        let lyrics = provider
            .lyrics("600908000007288315", None)
            .await
            .expect("live Migu lyrics");
        assert_eq!(lyrics.format, "mrc");
        assert!(lyrics.word_synced.is_some());
        assert!(lyrics.plain.is_some());
    }

    #[tokio::test]
    #[ignore = "requires live Migu media access"]
    async fn live_public_media_distinguishes_full_playback_preview_and_download() {
        let provider = MiguProvider::new(MiguConfig::default()).expect("create Migu provider");

        let free = provider
            .track("600913000000358395", None)
            .await
            .expect("free Migu track");
        let free_rights = provider
            .track_availability("600913000000358395", &TrackAvailabilityRequest::default())
            .await
            .expect("free listening rights");
        assert!(free_rights.playable);
        assert_eq!(free_rights.extensions["limit_length"], false);
        let free_stream = provider
            .stream(&free, &StreamRequest::default())
            .await
            .expect("free public stream");
        assert_eq!(free_stream.requested_quality, tuneweave_core::Quality::Auto);
        assert_eq!(
            free_stream.actual_quality,
            tuneweave_core::Quality::Standard
        );
        assert_eq!(free_stream.trial, None);
        let free_download = provider
            .download(&free, &StreamRequest::default())
            .await
            .expect("free public download");
        assert!(free_download.available);
        assert!(free_download.url.is_some());

        let restricted = provider
            .track("600908000007288315", None)
            .await
            .expect("restricted Migu track");
        let restricted_rights = provider
            .track_availability("600908000007288315", &TrackAvailabilityRequest::default())
            .await
            .expect("restricted listening rights");
        assert!(!restricted_rights.playable);
        assert_eq!(restricted_rights.extensions["limit_length"], true);
        let preview = provider
            .stream(&restricted, &StreamRequest::default())
            .await
            .expect("restricted preview");
        assert_eq!(
            preview.trial,
            Some(tuneweave_core::TrialWindow {
                start_ms: 65_000,
                end_ms: 125_000
            })
        );
        assert_eq!(preview.actual_quality, tuneweave_core::Quality::Standard);
        let blocked_download = provider
            .download(&restricted, &StreamRequest::default())
            .await
            .expect("restricted download result");
        assert!(!blocked_download.available);
        assert!(blocked_download.url.is_none());
        assert_eq!(blocked_download.extensions["preview_url_withheld"], true);
    }
}
