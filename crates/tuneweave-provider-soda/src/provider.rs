use std::{collections::BTreeSet, fmt};

use async_trait::async_trait;
use serde_json::json;
use tuneweave_core::{
    Capability, Extensions, Lyrics, LyricsRequest, MusicProvider, Page, PageMeta, Platform, Result,
    SearchKind, SearchQuery, SearchVariant, Track, TuneWeaveError,
};

use crate::client::{SodaClient, SodaConfig, UPSTREAM_SEARCH_PAGE_SIZE};

const MAX_UPSTREAM_PAGES_PER_SEARCH: u32 = 6;

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
            Capability::SearchTracks,
            Capability::TrackDetail,
            Capability::Lyrics,
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
                Capability::SearchTracks,
                Capability::TrackDetail,
                Capability::Lyrics,
            ])
        );
        assert!(provider.supports(Capability::TrackDetail));
        assert!(provider.supports(Capability::Lyrics));
        assert!(!provider.supports(Capability::AudioStream));
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
    }
}
