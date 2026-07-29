use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use async_trait::async_trait;
use serde_json::json;
use tuneweave_core::{
    AudioContent, Capability, Extensions, Lyrics, LyricsRequest, MediaDownload, MediaStream,
    MusicProvider, Page, PageMeta, PageRequest, Platform, Playlist, Quality, Result, SearchKind,
    SearchQuery, SearchVariant, StreamRequest, StreamVariant, Track, TrackAvailability,
    TrackAvailabilityRequest, TrialWindow, TuneWeaveError,
};

use crate::{
    client::{SodaClient, SodaConfig, SodaPlayback, UPSTREAM_SEARCH_PAGE_SIZE},
    identity::SodaTrackIdentity,
};

const MAX_UPSTREAM_PAGES_PER_SEARCH: u32 = 6;
const UPSTREAM_PLAYLIST_PAGE_SIZE: u32 = 100;
const MAX_UPSTREAM_PLAYLIST_PAGES: u32 = 128;

#[derive(Clone)]
pub struct SodaProvider {
    client: SodaClient,
}

impl fmt::Debug for SodaProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SodaProvider")
            .finish_non_exhaustive()
    }
}

impl SodaProvider {
    pub fn new(config: SodaConfig) -> Result<Self> {
        Ok(Self {
            client: SodaClient::new(&config)?,
        })
    }

    #[must_use]
    pub const fn from_client(client: SodaClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl MusicProvider for SodaProvider {
    fn platform(&self) -> Platform {
        Platform::Soda
    }

    fn name(&self) -> &'static str {
        "Soda Music"
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::AudioDownload,
            Capability::AudioStream,
            Capability::PlaylistRead,
            Capability::SearchTracks,
            Capability::TrackDetail,
            Capability::Lyrics,
            Capability::TrackAvailability,
        ])
    }

    async fn search(&self, query: &SearchQuery) -> Result<Page<Track>> {
        validate_search_query(query)?;
        let start_cursor = query.offset / UPSTREAM_SEARCH_PAGE_SIZE * UPSTREAM_SEARCH_PAGE_SIZE;
        let skip = usize::try_from(query.offset % UPSTREAM_SEARCH_PAGE_SIZE)
            .map_err(|_| soda_invalid_request("Soda search offset is too large"))?;
        let requested = usize::try_from(query.limit)
            .map_err(|_| soda_invalid_request("Soda search limit is too large"))?;
        let needed = skip.saturating_add(requested);
        let mut cursor = start_cursor;
        let mut tracks = Vec::with_capacity(needed);
        let mut pages_fetched = 0_u32;
        let mut upstream_has_more = false;
        let mut upstream_next_cursor = None;

        while tracks.len() < needed {
            if pages_fetched >= MAX_UPSTREAM_PAGES_PER_SEARCH {
                return Err(soda_upstream_error(
                    "Soda search exceeded the bounded upstream page count",
                ));
            }
            let page = self
                .client
                .search_tracks_page(query.query.trim(), cursor)
                .await?;
            pages_fetched = pages_fetched.saturating_add(1);
            tracks.extend(page.tracks);
            upstream_has_more = page.has_more;
            upstream_next_cursor = page.next_cursor;
            if !page.has_more {
                break;
            }
            cursor = page
                .next_cursor
                .ok_or_else(|| soda_upstream_error("Soda search continuation cursor was lost"))?;
        }

        let buffered_after_skip = tracks.len().saturating_sub(skip);
        let mut items = tracks.into_iter().skip(skip).collect::<Vec<_>>();
        items.truncate(requested);
        let returned = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let consumed = query.offset.saturating_add(returned);
        let has_buffered_more = buffered_after_skip > items.len();
        let has_more = has_buffered_more || upstream_has_more;
        let mut extensions = Extensions::new();
        extensions.insert("backend".to_owned(), json!("official_pc_track_search"));
        extensions.insert(
            "upstream_page_size".to_owned(),
            json!(UPSTREAM_SEARCH_PAGE_SIZE),
        );
        extensions.insert("upstream_pages_fetched".to_owned(), json!(pages_fetched));
        extensions.insert("upstream_cursor_start".to_owned(), json!(start_cursor));
        if let Some(next_cursor) = upstream_next_cursor {
            extensions.insert("upstream_next_cursor".to_owned(), json!(next_cursor));
        }
        extensions.insert("anonymous_device_required".to_owned(), json!(false));
        extensions.insert("request_signature_required".to_owned(), json!(false));

        Ok(Page {
            items,
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
            return Err(soda_invalid_request(
                "Soda public track detail does not accept an account",
            ));
        }
        let identity = self.client.resolve_track_identity(id).await?;
        self.client.track_detail(&identity).await
    }

    async fn lyrics(&self, id: &str, account: Option<&str>) -> Result<Lyrics> {
        validate_lyrics_request(account, None, false)?;
        let identity = self.client.resolve_track_identity(id).await?;
        self.client.lyrics(&identity).await
    }

    async fn lyrics_with_options(&self, id: &str, request: &LyricsRequest) -> Result<Lyrics> {
        validate_lyrics_request(
            request.account.as_deref(),
            request.song_type,
            request.singing_annotations,
        )?;
        let identity = self.client.resolve_track_identity(id).await?;
        self.client.lyrics(&identity).await
    }

    async fn track_availability(
        &self,
        id: &str,
        request: &TrackAvailabilityRequest,
    ) -> Result<TrackAvailability> {
        validate_availability_request(request)?;
        let identity = self.client.resolve_track_identity(id).await?;
        self.client.track_availability(&identity, request).await
    }

    async fn stream(&self, track: &Track, request: &StreamRequest) -> Result<MediaStream> {
        let identity = canonical_media_identity(track)?;
        validate_media_request(request)?;
        let requested_bitrate = requested_media_bitrate(request);
        let playback = self.client.playback(&identity, requested_bitrate).await?;
        let url = local_content_url(&identity, &playback);
        Ok(MediaStream {
            url,
            backup_urls: Vec::new(),
            headers: BTreeMap::new(),
            expires_at: None,
            format: Some(delivery_format(&playback).to_owned()),
            codec: Some(playback.codec.clone()),
            bitrate: Some(playback.bitrate),
            size: playback.size,
            duration_ms: Some(playback.duration_ms),
            requested_quality: request.quality,
            actual_quality: playback.quality,
            trial: playback.preview.then(|| TrialWindow {
                start_ms: playback.preview_start_ms.unwrap_or_default(),
                end_ms: playback
                    .preview_start_ms
                    .unwrap_or_default()
                    .saturating_add(playback.preview_duration_ms.unwrap_or(playback.duration_ms)),
            }),
            origin_track: Some(track.resource_ref.clone()),
            resolved_track: track.resource_ref.clone(),
            resolved_platform: Platform::Soda,
            match_score: Some(1.0),
            attempts: Vec::new(),
        })
    }

    async fn audio_content(&self, track: &Track, request: &StreamRequest) -> Result<AudioContent> {
        let identity = canonical_media_identity(track)?;
        validate_media_request(request)?;
        self.client
            .audio_content(&identity, requested_media_bitrate(request))
            .await
    }

    async fn download(&self, track: &Track, request: &StreamRequest) -> Result<MediaDownload> {
        let identity = canonical_media_identity(track)?;
        validate_media_request(request)?;
        let playback = self
            .client
            .playback(&identity, requested_media_bitrate(request))
            .await?;
        let available = !playback.preview;
        let mut extensions = Extensions::new();
        extensions.insert("backend".to_owned(), json!("official_pc_track_v2"));
        extensions.insert("local_delivery".to_owned(), json!(true));
        extensions.insert("encrypted_upstream".to_owned(), json!(playback.encrypted));
        extensions.insert("preview_url_withheld".to_owned(), json!(!available));
        Ok(MediaDownload {
            track_ref: track.resource_ref.clone(),
            platform: Platform::Soda,
            available,
            url: available.then(|| local_content_url(&identity, &playback)),
            headers: BTreeMap::new(),
            expires_at: None,
            format: Some(delivery_format(&playback).to_owned()),
            codec: Some(playback.codec),
            bitrate: Some(playback.bitrate),
            size: available.then_some(playback.size).flatten(),
            duration_ms: Some(playback.duration_ms),
            requested_quality: request.quality,
            actual_quality: playback.quality,
            platform_code: Some(playback.platform_code),
            fee: None,
            message: (!available).then(|| {
                "Soda only authorized a preview; a full download is unavailable".to_owned()
            }),
            extensions,
        })
    }

    async fn playlist(&self, id: &str, account: Option<&str>) -> Result<Playlist> {
        if account.is_some() {
            return Err(soda_invalid_request(
                "Soda public playlists do not accept an account",
            ));
        }
        let playlist_id = parse_playlist_id(id)?;
        Ok(self.client.playlist_page(playlist_id, 0, 1).await?.playlist)
    }

    async fn playlist_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        let playlist_id = parse_playlist_id(id)?;
        validate_playlist_page(request)?;
        let requested = usize::try_from(request.limit)
            .map_err(|_| soda_invalid_request("Soda playlist limit is too large"))?;
        let requested_start = u64::from(request.offset);
        let requested_end = requested_start
            .checked_add(u64::from(request.limit))
            .ok_or_else(|| soda_invalid_request("Soda playlist window overflowed"))?;
        let mut cursor = 0_u64;
        let mut seen_cursors = BTreeSet::new();
        let mut snapshot = None;
        let mut visible_position = 0_u64;
        let mut tracks = Vec::with_capacity(requested);
        let mut pages_fetched = 0_u32;

        loop {
            if pages_fetched >= MAX_UPSTREAM_PLAYLIST_PAGES {
                return Err(soda_upstream_error(
                    "Soda playlist exceeded the bounded upstream page count",
                ));
            }
            if !seen_cursors.insert(cursor) {
                return Err(soda_upstream_error(
                    "Soda playlist repeated an upstream cursor",
                ));
            }
            let page = self
                .client
                .playlist_page(playlist_id, cursor, UPSTREAM_PLAYLIST_PAGE_SIZE)
                .await?;
            pages_fetched = pages_fetched.saturating_add(1);
            let current_snapshot = (page.total, page.raw_total, page.updated_at);
            if let Some(expected) = snapshot {
                if current_snapshot != expected {
                    return Err(soda_upstream_error(
                        "Soda playlist changed during pagination",
                    ));
                }
            } else {
                snapshot = Some(current_snapshot);
                if requested_start >= page.total {
                    return Ok(soda_playlist_page(
                        Vec::new(),
                        request,
                        page.total,
                        pages_fetched,
                        cursor,
                        page.raw_total,
                    ));
                }
            }

            for mut track in page.tracks {
                if visible_position >= requested_start && tracks.len() < requested {
                    track
                        .extensions
                        .insert("playlist_position".to_owned(), json!(visible_position));
                    tracks.push(track);
                }
                visible_position = visible_position.checked_add(1).ok_or_else(|| {
                    soda_upstream_error("Soda playlist visible position overflowed")
                })?;
            }
            if tracks.len() == requested || visible_position >= page.total {
                break;
            }
            if !page.has_more {
                return Err(soda_upstream_error(
                    "Soda playlist pagination ended before its visible track total",
                ));
            }
            cursor = page
                .next_cursor
                .ok_or_else(|| soda_upstream_error("Soda playlist continuation cursor was lost"))?;
        }

        let (total, raw_total, _) = snapshot
            .ok_or_else(|| soda_upstream_error("Soda playlist snapshot was not established"))?;
        if visible_position < total.min(requested_end) {
            return Err(soda_upstream_error(
                "Soda playlist pagination ended before the requested window",
            ));
        }
        Ok(soda_playlist_page(
            tracks,
            request,
            total,
            pages_fetched,
            cursor,
            raw_total,
        ))
    }
}

fn soda_playlist_page(
    tracks: Vec<Track>,
    request: &PageRequest,
    total: u64,
    pages_fetched: u32,
    final_cursor: u64,
    raw_total: Option<u64>,
) -> Page<Track> {
    let returned = u32::try_from(tracks.len()).unwrap_or(u32::MAX);
    let consumed = request.offset.saturating_add(returned);
    let has_more = u64::from(consumed) < total;
    let mut extensions = Extensions::new();
    extensions.insert("backend".to_owned(), json!("official_pc_playlist_detail"));
    extensions.insert(
        "upstream_page_size".to_owned(),
        json!(UPSTREAM_PLAYLIST_PAGE_SIZE),
    );
    extensions.insert("upstream_pages_fetched".to_owned(), json!(pages_fetched));
    extensions.insert("upstream_final_cursor".to_owned(), json!(final_cursor));
    if let Some(value) = raw_total {
        extensions.insert("upstream_raw_resource_count".to_owned(), json!(value));
    }
    Page {
        items: tracks,
        pagination: PageMeta {
            limit: request.limit,
            offset: request.offset,
            total: Some(total),
            next_offset: (has_more && returned > 0).then_some(consumed),
            has_more,
            extensions,
        },
    }
}

fn parse_playlist_id(id: &str) -> Result<&str> {
    let parsed = id.parse::<u64>().map_err(|_| {
        soda_invalid_request("Soda playlist ID must be a canonical positive integer")
    })?;
    if parsed == 0 || parsed.to_string() != id {
        return Err(soda_invalid_request(
            "Soda playlist ID must be a canonical positive integer",
        ));
    }
    Ok(id)
}

fn validate_playlist_page(request: &PageRequest) -> Result<()> {
    if request.account.is_some() {
        return Err(soda_invalid_request(
            "Soda public playlists do not accept an account",
        ));
    }
    if !(1..=100).contains(&request.limit) {
        return Err(soda_invalid_request(
            "Soda playlist limit must be between 1 and 100",
        ));
    }
    Ok(())
}

fn canonical_media_identity(track: &Track) -> Result<SodaTrackIdentity> {
    if track.platform != Platform::Soda
        || track.resource_ref.platform() != Platform::Soda
        || track.id != track.resource_ref.id()
    {
        return Err(soda_invalid_request(
            "Soda media resolution requires a canonical Soda track",
        ));
    }
    SodaTrackIdentity::parse(track.resource_ref.id())
}

fn validate_media_request(request: &StreamRequest) -> Result<()> {
    if request.variant != StreamVariant::Default {
        return Err(soda_invalid_request(
            "Soda public media only supports the default stream variant",
        ));
    }
    if request.account.is_some() {
        return Err(soda_invalid_request(
            "Soda public media does not accept an account",
        ));
    }
    if request.immersive_type.is_some() {
        return Err(soda_invalid_request(
            "Soda public media does not accept immersive_type",
        ));
    }
    if request
        .bitrate
        .is_some_and(|bitrate| bitrate == 0 || bitrate > 10_000_000)
    {
        return Err(soda_invalid_request(
            "Soda public media bitrate must be between 1 and 10000000",
        ));
    }
    if matches!(
        request.quality,
        Quality::Surround | Quality::Dolby | Quality::Master
    ) {
        return Err(soda_invalid_request(
            "Soda public media does not support the requested quality class",
        ));
    }
    Ok(())
}

fn requested_media_bitrate(request: &StreamRequest) -> u64 {
    request.bitrate.unwrap_or(match request.quality {
        Quality::Auto | Quality::Spatial => 10_000_000,
        Quality::Low => 96_000,
        Quality::Standard => 192_000,
        Quality::Higher | Quality::High => 500_000,
        Quality::Lossless => 2_000_000,
        Quality::Hires => 5_000_000,
        Quality::Surround | Quality::Dolby | Quality::Master => 10_000_000,
    })
}

fn local_content_url(identity: &SodaTrackIdentity, playback: &SodaPlayback) -> String {
    format!(
        "/v1/tracks/soda:{}/stream/content?quality={}&bitrate={}",
        identity.id(),
        quality_parameter(playback.quality),
        playback.bitrate
    )
}

const fn quality_parameter(quality: Quality) -> &'static str {
    match quality {
        Quality::Auto => "auto",
        Quality::Low => "low",
        Quality::Standard => "standard",
        Quality::Higher => "higher",
        Quality::High => "high",
        Quality::Lossless => "lossless",
        Quality::Hires => "hires",
        Quality::Surround => "surround",
        Quality::Spatial => "spatial",
        Quality::Dolby => "dolby",
        Quality::Master => "master",
    }
}

fn delivery_format(playback: &SodaPlayback) -> &'static str {
    if playback.codec.eq_ignore_ascii_case("flac") {
        "flac"
    } else {
        "m4a"
    }
}

fn validate_search_query(query: &SearchQuery) -> Result<()> {
    if query.kind != SearchKind::Track {
        return Err(TuneWeaveError::unsupported(
            Platform::Soda,
            Capability::SearchTracks,
        ));
    }
    if query.variant != SearchVariant::Default {
        return Err(soda_invalid_request(
            "Soda public track search supports only the default variant",
        ));
    }
    let text = query.query.trim();
    if text.is_empty() || text.len() > 500 {
        return Err(soda_invalid_request(
            "Soda search query must contain between 1 and 500 bytes",
        ));
    }
    if !(1..=100).contains(&query.limit) {
        return Err(soda_invalid_request(
            "Soda search limit must be between 1 and 100",
        ));
    }
    if query.account.is_some() {
        return Err(soda_invalid_request(
            "Soda public track search does not accept an account",
        ));
    }
    if query.search_id.is_some()
        || query.highlight
        || !query.selectors.is_empty()
        || query.video_filters.is_some()
    {
        return Err(soda_invalid_request(
            "Soda public track search does not accept provider-specific search state",
        ));
    }
    Ok(())
}

fn validate_lyrics_request(
    account: Option<&str>,
    song_type: Option<i64>,
    singing_annotations: bool,
) -> Result<()> {
    if account.is_some() {
        return Err(soda_invalid_request(
            "Soda public lyrics do not accept an account",
        ));
    }
    if song_type.is_some() || singing_annotations {
        return Err(soda_invalid_request(
            "Soda lyrics do not accept song_type or singing annotations",
        ));
    }
    Ok(())
}

fn validate_availability_request(request: &TrackAvailabilityRequest) -> Result<()> {
    if request.account.is_some() {
        return Err(soda_invalid_request(
            "Soda public availability does not accept an account",
        ));
    }
    if request.bitrate == 0 || request.bitrate > 10_000_000 {
        return Err(soda_invalid_request(
            "Soda availability bitrate must be between 1 and 10000000",
        ));
    }
    Ok(())
}

fn soda_invalid_request(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Soda)
}

fn soda_upstream_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(tuneweave_core::ErrorCode::UpstreamError, message)
        .with_platform(Platform::Soda)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_advertises_only_verified_public_capabilities() {
        let provider = SodaProvider::new(SodaConfig::default()).expect("Soda provider");
        assert_eq!(provider.platform(), Platform::Soda);
        assert_eq!(provider.name(), "Soda Music");
        assert_eq!(
            provider.capabilities(),
            BTreeSet::from([
                Capability::AudioDownload,
                Capability::AudioStream,
                Capability::PlaylistRead,
                Capability::SearchTracks,
                Capability::TrackDetail,
                Capability::Lyrics,
                Capability::TrackAvailability,
            ])
        );
        assert!(provider.supports(Capability::TrackDetail));
        assert!(provider.supports(Capability::Lyrics));
        assert!(provider.supports(Capability::TrackAvailability));
        assert!(provider.supports(Capability::AudioStream));
        assert!(provider.supports(Capability::AudioDownload));
        assert!(provider.supports(Capability::PlaylistRead));
    }

    #[test]
    fn public_search_rejects_accounts_foreign_options_and_unbounded_inputs() {
        assert!(validate_search_query(&SearchQuery::tracks("落了白", 20, 0)).is_ok());

        let mut query = SearchQuery::tracks("落了白", 20, 0);
        query.account = Some("default".to_owned());
        assert!(validate_search_query(&query).is_err());

        let mut query = SearchQuery::tracks("落了白", 20, 0);
        query.variant = SearchVariant::Legacy;
        assert!(validate_search_query(&query).is_err());

        let mut query = SearchQuery::tracks("落了白", 20, 0);
        query.highlight = true;
        assert!(validate_search_query(&query).is_err());

        assert!(validate_search_query(&SearchQuery::tracks("", 20, 0)).is_err());
        assert!(validate_search_query(&SearchQuery::tracks("落了白", 101, 0)).is_err());
    }

    #[tokio::test]
    async fn public_track_detail_rejects_accounts_before_network_access() {
        let provider = SodaProvider::new(SodaConfig::default()).expect("Soda provider");
        let error = provider
            .track("7304719759323564095", Some("default"))
            .await
            .expect_err("Soda public detail must reject accounts");
        assert_eq!(error.code, tuneweave_core::ErrorCode::InvalidRequest);
        assert_eq!(error.platform, Some(Platform::Soda));

        let error = provider
            .lyrics("7304719759323564095", Some("default"))
            .await
            .expect_err("Soda public lyrics must reject accounts");
        assert_eq!(error.code, tuneweave_core::ErrorCode::InvalidRequest);

        let request = LyricsRequest {
            singing_annotations: true,
            ..LyricsRequest::default()
        };
        let error = provider
            .lyrics_with_options("7304719759323564095", &request)
            .await
            .expect_err("Soda lyrics must reject foreign lyric controls");
        assert_eq!(error.code, tuneweave_core::ErrorCode::InvalidRequest);

        let availability = TrackAvailabilityRequest {
            bitrate: 200_000,
            account: Some("default".to_owned()),
        };
        let error = provider
            .track_availability("7304719759323564095", &availability)
            .await
            .expect_err("Soda availability must reject accounts");
        assert_eq!(error.code, tuneweave_core::ErrorCode::InvalidRequest);
        let error = provider
            .track_availability("7304719759323564095", &TrackAvailabilityRequest::new(0))
            .await
            .expect_err("Soda availability must reject zero bitrate");
        assert_eq!(error.code, tuneweave_core::ErrorCode::InvalidRequest);
    }

    #[test]
    fn public_media_request_maps_quality_and_builds_only_local_delivery_urls() {
        let request = StreamRequest {
            quality: Quality::Lossless,
            ..StreamRequest::default()
        };
        assert!(validate_media_request(&request).is_ok());
        assert_eq!(requested_media_bitrate(&request), 2_000_000);

        let request = StreamRequest {
            quality: Quality::Low,
            bitrate: Some(123_456),
            ..StreamRequest::default()
        };
        assert_eq!(requested_media_bitrate(&request), 123_456);

        let identity = SodaTrackIdentity::parse("7304719759323564095").expect("Soda identity");
        let playback = SodaPlayback {
            preview: false,
            preview_start_ms: None,
            preview_duration_ms: None,
            duration_ms: 180_822,
            bitrate: 132_424,
            quality: Quality::Standard,
            codec: "aac".to_owned(),
            size: Some(3_000_000),
            platform_code: 10,
            encrypted: true,
        };
        assert_eq!(
            local_content_url(&identity, &playback),
            "/v1/tracks/soda:7304719759323564095/stream/content?quality=standard&bitrate=132424"
        );
        assert!(!local_content_url(&identity, &playback).contains("http"));

        for quality in [Quality::Surround, Quality::Dolby, Quality::Master] {
            let request = StreamRequest {
                quality,
                ..StreamRequest::default()
            };
            assert!(validate_media_request(&request).is_err());
        }
        let account = StreamRequest {
            account: Some("default".to_owned()),
            ..StreamRequest::default()
        };
        assert!(validate_media_request(&account).is_err());
    }

    #[test]
    fn public_playlists_require_canonical_ids_and_bounded_anonymous_pages() {
        assert_eq!(
            parse_playlist_id("7200303561195061287").expect("playlist ID"),
            "7200303561195061287"
        );
        for value in ["", "0", "01", "-1", "playlist:1", "1/path", " 1"] {
            assert!(parse_playlist_id(value).is_err(), "{value:?} must fail");
        }

        assert!(validate_playlist_page(&PageRequest::new(100, 0)).is_ok());
        assert!(validate_playlist_page(&PageRequest::new(0, 0)).is_err());
        assert!(validate_playlist_page(&PageRequest::new(101, 0)).is_err());
        let account = PageRequest {
            account: Some("default".to_owned()),
            ..PageRequest::new(20, 0)
        };
        assert!(validate_playlist_page(&account).is_err());

        let page = soda_playlist_page(Vec::new(), &PageRequest::new(20, 398), 398, 1, 0, Some(506));
        assert_eq!(page.pagination.total, Some(398));
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.next_offset, None);
        assert_eq!(
            page.pagination.extensions["upstream_raw_resource_count"],
            506
        );
    }
}
