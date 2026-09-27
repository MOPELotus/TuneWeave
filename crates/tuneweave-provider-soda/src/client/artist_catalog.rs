use std::collections::BTreeSet;

use crate::login::SodaCredential;
use tuneweave_core::Artist;

use super::*;

pub(crate) mod account_detail;

const MAX_ITEMS: usize = 10_000;
const MAX_PAGES: u32 = 128;
const MAX_PAGE_ITEMS: usize = 1000;
const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
const TOTAL_BUDGET: Duration = Duration::from_secs(60);

#[derive(Debug)]
pub(crate) struct SodaArtistCatalogue<T> {
    pub items: Vec<T>,
    pub reported_total: Option<u64>,
    pub upstream_pages: u32,
    pub backend: &'static str,
    pub source_user_id: Option<String>,
}

#[derive(Debug)]
struct CatalogueResponse {
    body: Vec<u8>,
    headers: reqwest::header::HeaderMap,
}

#[derive(Deserialize)]
struct Status {
    status_code: Option<i64>,
    now: u64,
    now_ts_ms: u64,
}

#[derive(Deserialize)]
struct PageState {
    status_code: Option<i64>,
    status_info: Status,
    #[serde(default)]
    has_more: bool,
    #[serde(default, deserialize_with = "present_cursor")]
    next_cursor: Option<String>,
}

fn present_cursor<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error> {
    String::deserialize(deserializer).map(Some)
}

impl PageState {
    fn parse(body: &[u8]) -> Result<Self> {
        let state: Self = serde_json::from_slice(body)
            .map_err(|_| invalid("Soda artist catalogue returned malformed status"))?;
        if [state.status_code, state.status_info.status_code]
            .into_iter()
            .flatten()
            .any(|code| code != 0)
            || state.status_info.now == 0
            || state.status_info.now_ts_ms / 1000 != state.status_info.now
            || state.next_cursor.as_deref().is_some_and(|cursor| {
                !cursor.is_empty() && canonical_nonnegative_decimal(cursor) != Some(cursor)
            })
        {
            return Err(invalid(
                "Soda artist catalogue returned invalid status or cursor",
            ));
        }
        Ok(state)
    }
}

#[derive(Deserialize)]
struct Profile {
    artist_info: ArtistCounts,
}

#[derive(Deserialize)]
struct ArtistCounts {
    count_albums: Option<u64>,
    #[serde(flatten)]
    metadata: super::artist::SodaArtistMetadata,
}

#[derive(Deserialize)]
struct TrackPage {
    #[serde(default)]
    tracks: Vec<SodaTrack>,
    albums: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize)]
struct AlbumPage {
    #[serde(default)]
    albums: Vec<CatalogueAlbum>,
    tracks: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize)]
struct CatalogueAlbum {
    count_tracks: Option<u64>,
    #[serde(flatten)]
    metadata: SodaAlbumMetadata,
}

pub(crate) trait CatalogueItem: Sized + serde::Serialize {
    const PATH: &'static str;
    const BACKEND: &'static str;
    const LOG_PATH: &'static str;
    const ACCOUNT_BACKEND: &'static str;
    fn count(artist: &Artist) -> Option<u64>;
    fn parse(body: &[u8], artist_id: &str) -> Result<Vec<Self>>;
    fn id(&self) -> &str;
    fn extensions(&mut self) -> &mut Extensions;
}

impl CatalogueItem for Track {
    const ACCOUNT_BACKEND: &'static str = "official_pc_account_artist_tracks";
    fn extensions(&mut self) -> &mut Extensions {
        &mut self.extensions
    }
    const PATH: &'static str = "tracks";
    const BACKEND: &'static str = "official_pc_artist_tracks";
    const LOG_PATH: &'static str = "/luna/pc/artists/{artist_id}/tracks";
    fn count(artist: &Artist) -> Option<u64> {
        artist.track_count
    }
    fn id(&self) -> &str {
        &self.id
    }
    fn parse(body: &[u8], artist_id: &str) -> Result<Vec<Self>> {
        let page: TrackPage = serde_json::from_slice(body)
            .map_err(|_| invalid("Soda artist tracks returned malformed data"))?;
        if page.albums.is_some() || page.tracks.len() > MAX_PAGE_ITEMS {
            return Err(invalid(
                "Soda artist tracks returned an invalid physical page",
            ));
        }
        page.tracks
            .into_iter()
            .map(|track| {
                validate_credits(&track.artists, artist_id)?;
                map_track(track, Self::BACKEND)
            })
            .collect()
    }
}

impl CatalogueItem for Album {
    const ACCOUNT_BACKEND: &'static str = "official_pc_account_artist_albums";
    fn extensions(&mut self) -> &mut Extensions {
        &mut self.extensions
    }
    const PATH: &'static str = "albums";
    const BACKEND: &'static str = "official_pc_artist_albums";
    const LOG_PATH: &'static str = "/luna/pc/artists/{artist_id}/albums";
    fn count(artist: &Artist) -> Option<u64> {
        artist.album_count
    }
    fn id(&self) -> &str {
        &self.id
    }
    fn parse(body: &[u8], artist_id: &str) -> Result<Vec<Self>> {
        let page: AlbumPage = serde_json::from_slice(body)
            .map_err(|_| invalid("Soda artist albums returned malformed data"))?;
        if page.tracks.is_some() || page.albums.len() > MAX_PAGE_ITEMS {
            return Err(invalid(
                "Soda artist albums returned an invalid physical page",
            ));
        }
        page.albums
            .into_iter()
            .map(|mut source| {
                validate_credits(&source.metadata.artists, artist_id)?;
                if source.metadata.has_error {
                    return Err(invalid(
                        "Soda artist catalogue returned an unavailable album",
                    ));
                }
                source.metadata.count_tracks = source.count_tracks.unwrap_or_default();
                let mut album = map_album_metadata(
                    &source.metadata,
                    &source.metadata.id,
                    source.metadata.count_tracks,
                )?;
                album.track_count = source.count_tracks;
                album
                    .extensions
                    .insert("backend".to_owned(), json!(Self::BACKEND));
                Ok(album)
            })
            .collect()
    }
}

fn validate_credits(artists: &[SodaArtist], expected: &str) -> Result<()> {
    if !artists.iter().any(|a| a.id == expected)
        || artists.iter().any(|a| {
            a.name.trim().is_empty()
                || (!a.id.is_empty() && canonical_positive_decimal(&a.id) != Some(a.id.as_str()))
        })
    {
        return Err(invalid(
            "Soda artist catalogue contained invalid or unrelated credits",
        ));
    }
    Ok(())
}

impl SodaClient {
    pub(crate) async fn artist_catalog_tracks(
        &self,
        id: &str,
    ) -> Result<SodaArtistCatalogue<Track>> {
        self.complete_artist_catalogue::<Track>(id).await
    }

    pub(crate) async fn artist_catalog_albums(
        &self,
        id: &str,
    ) -> Result<SodaArtistCatalogue<Album>> {
        self.complete_artist_catalogue::<Album>(id).await
    }

    async fn complete_artist_catalogue<T: CatalogueItem>(
        &self,
        id: &str,
    ) -> Result<SodaArtistCatalogue<T>> {
        if canonical_positive_decimal(id) != Some(id) {
            return Err(soda_invalid_request(
                "Soda artist ID must be a canonical positive decimal",
            ));
        }
        tokio::time::timeout(
            TOTAL_BUDGET,
            self.collect_artist_catalogue::<T, _>(id, None, |_, _| Ok(())),
        )
        .await
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Soda artist catalogue exceeded its total time budget",
            )
            .with_platform(Platform::Soda)
            .retryable(true)
        })?
    }

    pub(crate) async fn collect_artist_catalogue<T: CatalogueItem, F>(
        &self,
        id: &str,
        mut source: Option<SodaCredential>,
        mut checkpoint: F,
    ) -> Result<SodaArtistCatalogue<T>>
    where
        F: FnMut(&SodaCredential, Option<SodaCredential>) -> Result<()> + Send,
    {
        if canonical_positive_decimal(id) != Some(id) {
            return Err(soda_invalid_request(
                "Soda artist ID must be a canonical positive decimal",
            ));
        }
        if source.as_ref().is_some_and(|s| s.user_id().is_none()) {
            return Err(authentication_required());
        }
        let mut sources = source.iter().cloned().collect::<Vec<_>>();
        let response = self
            .artist_catalog_request(id, None, "", MAX_TOTAL_BYTES, source.as_ref())
            .await;
        if let Some(current) = &source {
            checkpoint(current, None)?;
        }
        let response = response?;
        let body = response.body;
        let mut bytes = body.len();
        let profile: Profile = serde_json::from_slice(&body)
            .map_err(|_| invalid("Soda artist catalogue omitted artist metadata"))?;
        let mut artist = super::artist::map_artist_metadata(
            profile.artist_info.metadata,
            "official_pc_artist_detail",
        )?;
        artist.album_count = profile.artist_info.count_albums;
        let reported_total = T::count(&artist);
        if artist.id != id || reported_total.is_some_and(|n| n > MAX_ITEMS as u64) {
            return Err(invalid(
                "Soda artist catalogue identity or total exceeded supported bounds",
            ));
        }
        accept_rotation(
            &mut source,
            &mut sources,
            &response.headers,
            &mut checkpoint,
        )?;
        let mut items = Vec::new();
        let mut seen = BTreeSet::new();
        let mut cursor = String::new();
        for page_number in 1..=MAX_PAGES {
            let response = self
                .artist_catalog_request(
                    id,
                    Some((T::PATH, T::LOG_PATH)),
                    &cursor,
                    MAX_TOTAL_BYTES - bytes,
                    source.as_ref(),
                )
                .await;
            if let Some(current) = &source {
                checkpoint(current, None)?;
            }
            let response = response?;
            let body = response.body;
            bytes = bytes.saturating_add(body.len());
            if bytes > MAX_TOTAL_BYTES {
                return Err(invalid(
                    "Soda artist catalogue exceeded its aggregate size limit",
                ));
            }
            let state = PageState::parse(&body)?;
            // Profile responses have no cursor. Directory responses, including
            // the observed empty catalogue, provide one even on their last page.
            let next_cursor = state
                .next_cursor
                .ok_or_else(|| invalid("Soda artist catalogue omitted its directory cursor"))?;
            let page = T::parse(&body, id)?;
            if items.len().saturating_add(page.len()) > MAX_ITEMS
                || (state.has_more && page.is_empty())
            {
                return Err(invalid(
                    "Soda artist catalogue returned an invalid page size",
                ));
            }
            for item in page {
                if !seen.insert(item.id().to_owned()) {
                    return Err(invalid("Soda artist catalogue repeated a resource"));
                }
                items.push(item);
            }
            if reported_total.is_some_and(|n| (items.len() as u64) > n) {
                return Err(invalid(
                    "Soda artist catalogue exceeded its independent artist count",
                ));
            }
            if !state.has_more {
                if reported_total.is_some_and(|n| n != items.len() as u64) {
                    return Err(invalid(
                        "Soda artist catalogue did not match its independent artist count",
                    ));
                }
                accept_rotation(
                    &mut source,
                    &mut sources,
                    &response.headers,
                    &mut checkpoint,
                )?;
                let source_user_id = source.as_ref().and_then(|s| s.user_id()).map(str::to_owned);
                if let Some(uid) = &source_user_id {
                    for item in &mut items {
                        let extensions = item.extensions();
                        extensions.insert("backend".into(), json!(T::ACCOUNT_BACKEND));
                        extensions.insert("source_user_id".into(), json!(uid));
                        extensions.insert("authenticated".into(), json!(true));
                    }
                    crate::account::reject_secrets(
                        &serde_json::to_value(&items).map_err(|_| {
                            invalid("Soda artist catalogue metadata could not be encoded")
                        })?,
                        &sources,
                    )?;
                }
                return Ok(SodaArtistCatalogue {
                    items,
                    reported_total,
                    upstream_pages: page_number,
                    backend: if source_user_id.is_some() {
                        T::ACCOUNT_BACKEND
                    } else {
                        T::BACKEND
                    },
                    source_user_id,
                });
            }
            let next = next_cursor
                .parse::<u64>()
                .map_err(|_| invalid("Soda artist catalogue omitted its continuation cursor"))?;
            if next <= cursor.parse::<u64>().unwrap_or_default() {
                return Err(invalid(
                    "Soda artist catalogue continuation did not advance",
                ));
            }
            accept_rotation(
                &mut source,
                &mut sources,
                &response.headers,
                &mut checkpoint,
            )?;
            cursor = next_cursor;
        }
        Err(invalid("Soda artist catalogue exceeded its page budget"))
    }

    async fn artist_catalog_request(
        &self,
        id: &str,
        kind: Option<(&'static str, &'static str)>,
        cursor: &str,
        remaining_bytes: usize,
        credential: Option<&SodaCredential>,
    ) -> Result<CatalogueResponse> {
        let started = Instant::now();
        let mut status = None;
        let endpoint = kind.map_or("/luna/pc/artists/{artist_id}", |(_, path)| path);
        let result =
            async {
                if remaining_bytes == 0 {
                    return Err(invalid(
                        "Soda artist catalogue exhausted its aggregate size limit",
                    ));
                }
                let mut url = Url::parse("https://api.qishui.com")
                    .map_err(|_| invalid("Soda artist catalogue endpoint is invalid"))?;
                url.set_path(&format!(
                    "/luna/pc/artists/{id}{}",
                    kind.map_or(String::new(), |(name, _)| format!("/{name}"))
                ));
                url.query_pairs_mut()
                    .append_pair("aid", SODA_APP_ID)
                    .append_pair("app_name", "luna_pc")
                    .append_pair("device_platform", "windows")
                    .append_pair("version_name", "2.1.0")
                    .append_pair("version_code", "20010000")
                    .append_pair("channel", "official");
                if kind.is_some() {
                    url.query_pairs_mut()
                        .append_pair("cursor", cursor)
                        .append_pair("count", "50");
                }
                if credential.is_some() {
                    let device = self.login_device()?;
                    url.query_pairs_mut()
                        .append_pair("device_id", &device.device_id)
                        .append_pair("iid", &device.install_id)
                        .append_pair("fp", &device.device_id);
                }
                let mut request = self
                    .login_request(reqwest::Method::GET, url)
                    .header(reqwest::header::ACCEPT, "application/json");
                if let Some(credential) = credential {
                    request = request.header(reqwest::header::COOKIE, credential.cookie_header()?);
                }
                let mut response = self.send_login_request(request).await?;
                status = Some(response.status());
                if credential.is_some() && response.status() == StatusCode::UNAUTHORIZED {
                    return Err(authentication_required());
                }
                if response.status() != StatusCode::OK {
                    return Err(soda_http_error(response.status()));
                }
                if response.headers().contains_key("bdturing-verify") {
                    return Err(TuneWeaveError::new(ErrorCode::CapabilityNotSupported,
                    "Soda artist catalogue requires an additional platform verification challenge")
                    .with_platform(Platform::Soda));
                }
                if !response
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .is_some_and(|v| {
                        v.split(';')
                            .next()
                            .unwrap_or_default()
                            .trim()
                            .eq_ignore_ascii_case("application/json")
                    })
                {
                    return Err(invalid(
                        "Soda artist catalogue returned an unexpected content type",
                    ));
                }
                let limit = remaining_bytes.min(MAX_API_RESPONSE_BYTES as usize);
                if response.content_length().is_some_and(|n| n > limit as u64) {
                    return Err(invalid(
                        "Soda artist catalogue exceeded its response size limit",
                    ));
                }
                let mut body = Vec::new();
                let headers = response.headers().clone();
                while let Some(chunk) = response.chunk().await.map_err(soda_network_error)? {
                    if body.len().saturating_add(chunk.len()) > limit {
                        return Err(invalid(
                            "Soda artist catalogue exceeded its response size limit",
                        ));
                    }
                    body.extend_from_slice(&chunk);
                }
                if credential.is_some() {
                    let value: serde_json::Value = serde_json::from_slice(&body)
                        .map_err(|_| invalid("Soda artist catalogue returned malformed data"))?;
                    for code in [
                        value.get("status_code"),
                        value.get("status_info").and_then(|s| s.get("status_code")),
                    ]
                    .into_iter()
                    .flatten()
                    {
                        let code = code.as_i64().ok_or_else(|| {
                            invalid("Soda artist catalogue returned invalid status")
                        })?;
                        if code == 1_000_016 {
                            return Err(authentication_required());
                        }
                        if code != 0 {
                            return Err(invalid("Soda artist catalogue request was rejected"));
                        }
                    }
                }
                PageState::parse(&body)?;
                Ok(CatalogueResponse { body, headers })
            }
            .await;
        self.log_upstream_request(
            "artist_catalogue",
            "api.qishui.com",
            endpoint,
            status,
            started,
            &result,
        );
        result
    }
}

fn accept_rotation<F>(
    source: &mut Option<SodaCredential>,
    sources: &mut Vec<SodaCredential>,
    headers: &reqwest::header::HeaderMap,
    checkpoint: &mut F,
) -> Result<()>
where
    F: FnMut(&SodaCredential, Option<SodaCredential>) -> Result<()>,
{
    if let Some(current) = source {
        let updated = current.with_response_cookies(headers)?;
        checkpoint(current, Some(updated.clone()))?;
        sources.push(updated.clone());
        *current = updated;
    }
    Ok(())
}

fn authentication_required() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "Soda artist catalogue session is not authenticated",
    )
    .with_platform(Platform::Soda)
}

fn invalid(message: &'static str) -> TuneWeaveError {
    soda_upstream_error(message)
}

#[cfg(test)]
mod tests;
