use std::{collections::BTreeSet, fmt};

use async_trait::async_trait;
use serde_json::json;
use tuneweave_core::{
    Capability, Extensions, Lyrics, LyricsRequest, MediaDownload, MediaStream, MusicProvider, Page,
    PageMeta, Platform, Result, SearchKind, SearchQuery, SearchVariant, StreamRequest, Track,
    TuneWeaveError,
};

use crate::client::{KugouClient, KugouConfig};

const UPSTREAM_PAGE_SIZE: u32 = 100;

#[derive(Clone)]
pub struct KugouProvider {
    client: KugouClient,
}

impl fmt::Debug for KugouProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KugouProvider")
            .finish_non_exhaustive()
    }
}

impl KugouProvider {
    pub fn new(config: KugouConfig) -> Result<Self> {
        Ok(Self {
            client: KugouClient::new(&config)?,
        })
    }

    #[must_use]
    pub const fn from_client(client: KugouClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl MusicProvider for KugouProvider {
    fn platform(&self) -> Platform {
        Platform::Kugou
    }

    fn name(&self) -> &'static str {
        "KuGou Music"
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::AudioDownload,
            Capability::AudioStream,
            Capability::Lyrics,
            Capability::SearchTracks,
            Capability::TrackDetail,
        ])
    }

    async fn search(&self, query: &SearchQuery) -> Result<Page<Track>> {
        validate_search_query(query)?;
        let upstream_page = query.offset / UPSTREAM_PAGE_SIZE + 1;
        let skip = usize::try_from(query.offset % UPSTREAM_PAGE_SIZE).unwrap_or(usize::MAX);
        let first = self
            .client
            .search_tracks_page(query.query.trim(), upstream_page, UPSTREAM_PAGE_SIZE)
            .await?;
        let total = first.total;
        let mut tracks = first.tracks.into_iter().skip(skip).collect::<Vec<_>>();
        let requested = usize::try_from(query.limit).unwrap_or(usize::MAX);
        let next_page_needed = tracks.len() < requested
            && u64::from(query.offset)
                .saturating_add(u64::try_from(tracks.len()).unwrap_or(u64::MAX))
                < total;
        if next_page_needed {
            let second = self
                .client
                .search_tracks_page(
                    query.query.trim(),
                    upstream_page.saturating_add(1),
                    UPSTREAM_PAGE_SIZE,
                )
                .await?;
            tracks.extend(second.tracks);
        }
        tracks.truncate(requested);
        let returned = u32::try_from(tracks.len()).unwrap_or(u32::MAX);
        let consumed = query.offset.saturating_add(returned);
        let has_more = u64::from(consumed) < total;
        let mut extensions = Extensions::new();
        extensions.insert("backend".to_owned(), json!("song_search_v2"));
        extensions.insert("upstream_page_size".to_owned(), json!(UPSTREAM_PAGE_SIZE));
        Ok(Page {
            items: tracks,
            pagination: PageMeta {
                limit: query.limit,
                offset: query.offset,
                total: Some(total),
                next_offset: has_more.then_some(consumed),
                has_more,
                extensions,
            },
        })
    }

    async fn track(&self, id: &str, account: Option<&str>) -> Result<Track> {
        if account.is_some() {
            return Err(kugou_invalid_request(
                "KuGou public track detail does not accept an account",
            ));
        }
        let album_audio_id = parse_album_audio_id(id)?;
        self.client.track_detail(album_audio_id).await
    }

    async fn lyrics(&self, id: &str, account: Option<&str>) -> Result<Lyrics> {
        if account.is_some() {
            return Err(kugou_invalid_request(
                "KuGou public lyrics do not accept an account",
            ));
        }
        let album_audio_id = parse_album_audio_id(id)?;
        self.client.lyrics(album_audio_id).await
    }

    async fn lyrics_with_options(&self, id: &str, request: &LyricsRequest) -> Result<Lyrics> {
        validate_lyrics_request(request)?;
        let album_audio_id = parse_album_audio_id(id)?;
        self.client.lyrics(album_audio_id).await
    }

    async fn stream(&self, track: &Track, request: &StreamRequest) -> Result<MediaStream> {
        self.client.stream(track, request).await
    }

    async fn download(&self, track: &Track, request: &StreamRequest) -> Result<MediaDownload> {
        self.client.download(track, request).await
    }
}

fn validate_lyrics_request(request: &LyricsRequest) -> Result<()> {
    if request.account.is_some() {
        return Err(kugou_invalid_request(
            "KuGou public lyrics do not accept an account",
        ));
    }
    if request.song_type.is_some() || request.singing_annotations {
        return Err(kugou_invalid_request(
            "KuGou lyrics do not accept song_type or singing annotations",
        ));
    }
    Ok(())
}

fn parse_album_audio_id(id: &str) -> Result<u64> {
    let parsed = id.parse::<u64>().map_err(|_| {
        kugou_invalid_request("KuGou track ID must be a canonical positive album_audio_id")
    })?;
    if parsed == 0 || parsed.to_string() != id {
        return Err(kugou_invalid_request(
            "KuGou track ID must be a canonical positive album_audio_id",
        ));
    }
    Ok(parsed)
}

fn validate_search_query(query: &SearchQuery) -> Result<()> {
    if query.kind != SearchKind::Track {
        return Err(TuneWeaveError::unsupported(
            Platform::Kugou,
            capability_for_search(query.kind),
        ));
    }
    if query.variant != SearchVariant::Default {
        return Err(kugou_invalid_request(
            "KuGou public track search only supports the default backend",
        ));
    }
    if query.account.is_some() {
        return Err(kugou_invalid_request(
            "KuGou public track search does not accept an account",
        ));
    }
    if query.search_id.is_some() || query.highlight || !query.selectors.is_empty() {
        return Err(kugou_invalid_request(
            "KuGou public track search does not accept search_id, highlight, or selectors",
        ));
    }
    if query.video_filters.is_some() {
        return Err(kugou_invalid_request(
            "KuGou track search does not accept video filters",
        ));
    }
    let keyword = query.query.trim();
    if keyword.is_empty() || keyword.len() > 512 || keyword.chars().any(char::is_control) {
        return Err(kugou_invalid_request(
            "KuGou search query must contain 1 to 512 non-control UTF-8 bytes",
        ));
    }
    if !(1..=100).contains(&query.limit) {
        return Err(kugou_invalid_request(
            "KuGou search limit must be between 1 and 100",
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

fn kugou_invalid_request(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Kugou)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search_query() -> SearchQuery {
        SearchQuery::tracks("反方向的钟", 30, 0)
    }

    #[test]
    fn provider_advertises_only_implemented_public_capabilities() {
        let provider = KugouProvider::new(KugouConfig::default()).expect("create KuGou provider");
        assert_eq!(provider.platform(), Platform::Kugou);
        assert_eq!(
            provider.capabilities(),
            BTreeSet::from([
                Capability::AudioDownload,
                Capability::AudioStream,
                Capability::Lyrics,
                Capability::SearchTracks,
                Capability::TrackDetail
            ])
        );
    }

    #[test]
    fn track_detail_requires_a_canonical_album_audio_identity() {
        assert_eq!(
            parse_album_audio_id("32100650").expect("valid ID"),
            32100650
        );
        for id in ["", "0", "032100650", " 32100650", "+32100650", "-1", "hash"] {
            assert!(parse_album_audio_id(id).is_err(), "{id:?} must fail");
        }
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
    }
}
