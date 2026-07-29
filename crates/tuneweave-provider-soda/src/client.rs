use std::{collections::BTreeMap, fmt, time::Duration};

use reqwest::{Client, Proxy, StatusCode, header::CONTENT_LENGTH, redirect::Policy};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tuneweave_core::{
    AlbumSummary, ArtistSummary, ErrorCode, Extensions, Platform, Quality, ResourceRef, Result,
    Track, TuneWeaveError,
};
use url::Url;

pub(crate) const UPSTREAM_SEARCH_PAGE_SIZE: u32 = 20;
const SEARCH_ENDPOINT: &str = "https://api.qishui.com/luna/pc/search/track";
const SODA_APP_ID: &str = "386088";
const MAX_API_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const USER_AGENT: &str = "TuneWeave/0.1 (Soda public music provider)";

#[derive(Clone, Default)]
pub struct SodaConfig {
    pub proxy_url: Option<String>,
}

impl fmt::Debug for SodaConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SodaConfig")
            .field(
                "proxy_url",
                &self.proxy_url.as_ref().map(|_| "[configured]"),
            )
            .finish()
    }
}

#[derive(Clone)]
pub struct SodaClient {
    http: Client,
}

impl fmt::Debug for SodaClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("SodaClient").finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub(crate) struct SodaSearchPage {
    pub tracks: Vec<Track>,
    pub next_cursor: Option<u32>,
    pub has_more: bool,
}

#[derive(Serialize)]
struct SodaSearchQuery<'a> {
    q: &'a str,
    aid: &'static str,
    cursor: u32,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct SodaSearchEnvelope {
    status_info: SodaStatusInfo,
    result_groups: Vec<SodaSearchGroup>,
    extra: SodaSearchExtra,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct SodaStatusInfo {
    log_id: String,
    now: u64,
    now_ts_ms: u64,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct SodaSearchExtra {
    empty_search: Option<u8>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct SodaSearchGroup {
    id: String,
    next_cursor: FlexibleText,
    has_more: bool,
    data: Vec<SodaSearchItem>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct SodaSearchItem {
    entity: SodaSearchEntity,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct SodaSearchEntity {
    track: Option<SodaTrack>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaTrack {
    id: String,
    name: String,
    duration: u64,
    vid: String,
    artists: Vec<SodaArtist>,
    album: SodaAlbum,
    bit_rates: Vec<SodaBitRate>,
    preview: Option<SodaPreview>,
    audition_info: Option<SodaAuditionInfo>,
    label_info: SodaLabelInfo,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaArtist {
    id: String,
    name: String,
    simple_display_name: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaAlbum {
    id: String,
    name: String,
    release_date: i64,
    url_cover: SodaImage,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaImage {
    uri: String,
    urls: Vec<String>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaBitRate {
    br: u64,
    size: u64,
    quality: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaPreview {
    vid: String,
    start: u64,
    duration: u64,
    bit_rates: Vec<SodaBitRate>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaAuditionInfo {
    vid: String,
    start_time_ms: u64,
    duration_ms: u64,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaLabelInfo {
    only_vip_download: bool,
    only_vip_playable: bool,
    quality_only_vip_can_download: Vec<String>,
    quality_only_vip_can_play: Vec<String>,
    quality_map: BTreeMap<String, SodaQualityPolicy>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaQualityPolicy {
    play_detail: Option<SodaQualityBenefit>,
    download_detail: Option<SodaQualityBenefit>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaQualityBenefit {
    condition: String,
    need_vip: bool,
    need_purchase: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(untagged)]
enum FlexibleText {
    Text(String),
    Unsigned(u64),
    Signed(i64),
    #[default]
    Null,
}

impl FlexibleText {
    fn to_u32(&self) -> Option<u32> {
        match self {
            Self::Text(value) => {
                canonical_nonnegative_decimal(value).and_then(|value| value.parse::<u32>().ok())
            }
            Self::Unsigned(value) => u32::try_from(*value).ok(),
            Self::Signed(value) => u32::try_from(*value).ok(),
            Self::Null => None,
        }
    }
}

impl SodaClient {
    pub fn new(config: &SodaConfig) -> Result<Self> {
        let mut builder = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .redirect(Policy::none())
            .user_agent(USER_AGENT);
        if let Some(proxy_url) = config.proxy_url.as_deref() {
            let proxy = Proxy::all(proxy_url).map_err(|_| {
                soda_invalid_request("Soda proxy configuration is not a valid proxy URL")
            })?;
            builder = builder.proxy(proxy);
        }
        let http = builder.build().map_err(|_| {
            TuneWeaveError::new(ErrorCode::InternalError, "failed to build Soda HTTP client")
                .with_platform(Platform::Soda)
        })?;
        Ok(Self { http })
    }

    #[cfg(test)]
    pub(crate) fn test_client() -> Self {
        Self::new(&SodaConfig::default()).expect("build Soda test client")
    }

    pub(crate) async fn search_tracks_page(
        &self,
        query: &str,
        cursor: u32,
    ) -> Result<SodaSearchPage> {
        let response = self
            .http
            .get(SEARCH_ENDPOINT)
            .query(&SodaSearchQuery {
                q: query,
                aid: SODA_APP_ID,
                cursor,
            })
            .send()
            .await
            .map_err(soda_network_error)?;
        let body = read_bounded_response(response, "Soda track search").await?;
        parse_search_response(&body, cursor)
    }
}

fn parse_search_response(body: &[u8], requested_cursor: u32) -> Result<SodaSearchPage> {
    let envelope: SodaSearchEnvelope = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda search returned malformed JSON"))?;
    if envelope.status_info.log_id.len() > 256
        || envelope.status_info.now_ts_ms < envelope.status_info.now.saturating_mul(1_000)
    {
        return Err(soda_upstream_error(
            "Soda search returned invalid status metadata",
        ));
    }

    let mut track_groups = envelope
        .result_groups
        .into_iter()
        .filter(|group| group.id == "tracks");
    let Some(group) = track_groups.next() else {
        if envelope.extra.empty_search == Some(1) {
            return Ok(SodaSearchPage {
                tracks: Vec::new(),
                next_cursor: None,
                has_more: false,
            });
        }
        return Err(soda_upstream_error(
            "Soda search response did not contain the track group",
        ));
    };
    if track_groups.next().is_some() {
        return Err(soda_upstream_error(
            "Soda search response contained duplicate track groups",
        ));
    }
    if group.data.len() > usize::try_from(UPSTREAM_SEARCH_PAGE_SIZE).unwrap_or(usize::MAX) {
        return Err(soda_upstream_error(
            "Soda search returned more than one physical page",
        ));
    }

    let next_cursor = if group.has_more {
        let cursor = group.next_cursor.to_u32().ok_or_else(|| {
            soda_upstream_error("Soda search omitted a valid continuation cursor")
        })?;
        if cursor <= requested_cursor {
            return Err(soda_upstream_error(
                "Soda search continuation cursor did not advance",
            ));
        }
        Some(cursor)
    } else {
        None
    };
    if group.data.is_empty() && group.has_more {
        return Err(soda_upstream_error(
            "Soda search returned an empty page with a continuation cursor",
        ));
    }

    let tracks = group
        .data
        .into_iter()
        .map(|item| {
            item.entity.track.ok_or_else(|| {
                soda_upstream_error("Soda track search item omitted its track payload")
            })
        })
        .map(|track| track.and_then(map_search_track))
        .collect::<Result<Vec<_>>>()?;
    Ok(SodaSearchPage {
        tracks,
        next_cursor,
        has_more: group.has_more,
    })
}

fn map_search_track(source: SodaTrack) -> Result<Track> {
    let track_id = canonical_positive_decimal(&source.id)
        .ok_or_else(|| soda_upstream_error("Soda search returned an invalid track id"))?;
    let name = source.name.trim();
    if name.is_empty() || name.len() > 1_000 {
        return Err(soda_upstream_error(
            "Soda search returned an invalid track name",
        ));
    }

    let track_ref = ResourceRef::new(Platform::Soda, track_id)
        .map_err(|_| soda_upstream_error("Soda search returned an invalid track reference"))?;
    let mut track = Track::new(track_ref, name);
    track.duration_ms =
        (source.duration > 0 && source.duration <= 24 * 60 * 60 * 1_000).then_some(source.duration);
    track.artists = source
        .artists
        .iter()
        .filter_map(map_artist_summary)
        .collect();
    if !source.album.id.trim().is_empty() || !source.album.name.trim().is_empty() {
        let album_ref = canonical_positive_decimal(&source.album.id)
            .and_then(|id| ResourceRef::new(Platform::Soda, id).ok());
        track.album = Some(AlbumSummary {
            resource_ref: album_ref,
            name: bounded_text(&source.album.name, 1_000).unwrap_or_default(),
            cover_url: normalize_image(&source.album.url_cover),
        });
    }
    track.available_qualities = map_qualities(&source.bit_rates);

    let mut extensions = Extensions::new();
    extensions.insert("backend".to_owned(), json!("official_pc_track_search"));
    extensions.insert("media_specs".to_owned(), json!(source.bit_rates));
    extensions.insert("rights".to_owned(), json!(source.label_info));
    if let Some(preview) = source.preview {
        extensions.insert("preview".to_owned(), json!(preview));
    }
    if let Some(audition_info) = source.audition_info {
        extensions.insert("audition_info".to_owned(), json!(audition_info));
    }
    if !source.vid.trim().is_empty() && source.vid.len() <= 256 {
        extensions.insert("media_vid".to_owned(), json!(source.vid));
    }
    if source.album.release_date > 0 {
        extensions.insert(
            "album_release_epoch_seconds".to_owned(),
            json!(source.album.release_date),
        );
    }
    extensions.insert(
        "catalog_rights_are_not_live_playback".to_owned(),
        json!(true),
    );
    track.extensions = extensions;
    Ok(track)
}

fn map_artist_summary(source: &SodaArtist) -> Option<ArtistSummary> {
    let name = bounded_text(
        if source.name.trim().is_empty() {
            &source.simple_display_name
        } else {
            &source.name
        },
        1_000,
    )?;
    let resource_ref = canonical_positive_decimal(&source.id)
        .and_then(|id| ResourceRef::new(Platform::Soda, id).ok());
    Some(ArtistSummary { resource_ref, name })
}

fn map_qualities(specs: &[SodaBitRate]) -> Vec<Quality> {
    let mut qualities = Vec::new();
    for quality in [
        Quality::Low,
        Quality::Standard,
        Quality::High,
        Quality::Lossless,
        Quality::Hires,
        Quality::Spatial,
    ] {
        if specs
            .iter()
            .any(|spec| map_quality(&spec.quality) == Some(quality))
        {
            qualities.push(quality);
        }
    }
    qualities
}

fn map_quality(value: &str) -> Option<Quality> {
    match value.trim().to_ascii_lowercase().as_str() {
        "medium" => Some(Quality::Low),
        "higher" => Some(Quality::Standard),
        "highest" => Some(Quality::High),
        "lossless" => Some(Quality::Lossless),
        "hi_res" | "hires" => Some(Quality::Hires),
        "spatial" => Some(Quality::Spatial),
        _ => None,
    }
}

fn normalize_image(image: &SodaImage) -> Option<String> {
    let uri = image.uri.trim();
    if uri.is_empty()
        || uri.len() > 1_024
        || uri.starts_with('/')
        || uri.contains('?')
        || uri.contains('#')
        || uri.contains('\\')
        || uri
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
        || !uri
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-' | b'.'))
    {
        return None;
    }
    image.urls.iter().find_map(|base| {
        let url = Url::parse(base.trim()).ok()?;
        let host = url.host_str()?;
        let image_node = host.strip_suffix("-luna.douyinpic.com")?;
        if !image_node
            .strip_prefix('p')?
            .bytes()
            .all(|byte| byte.is_ascii_digit())
            || url.scheme() != "https"
            || !url.username().is_empty()
            || url.password().is_some()
            || !matches!(url.port(), None | Some(443))
            || url.path() != "/img/"
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return None;
        }
        let joined = url.join(uri).ok()?;
        (joined.host_str() == Some(host)
            && joined.scheme() == "https"
            && joined.query().is_none()
            && joined.fragment().is_none())
        .then(|| joined.to_string())
    })
}

fn bounded_text(value: &str, maximum_bytes: usize) -> Option<String> {
    let value = value.trim();
    (!value.is_empty() && value.len() <= maximum_bytes).then(|| value.to_owned())
}

fn canonical_positive_decimal(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()
        && !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<u64>().is_ok_and(|value| value > 0))
    .then_some(value)
}

fn canonical_nonnegative_decimal(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()
        && (value == "0" || !value.starts_with('0'))
        && value.bytes().all(|byte| byte.is_ascii_digit()))
    .then_some(value)
}

async fn read_bounded_response(response: reqwest::Response, operation: &str) -> Result<Vec<u8>> {
    let mut response = response;
    let status = response.status();
    if !status.is_success() {
        return Err(soda_http_error(status));
    }
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_API_RESPONSE_BYTES)
    {
        return Err(soda_upstream_error(format!(
            "{operation} response exceeded the size limit"
        )));
    }
    let maximum = usize::try_from(MAX_API_RESPONSE_BYTES).unwrap_or(usize::MAX);
    let mut body = Vec::with_capacity(
        response
            .content_length()
            .and_then(|length| usize::try_from(length).ok())
            .unwrap_or_default()
            .min(maximum),
    );
    while let Some(chunk) = response.chunk().await.map_err(soda_network_error)? {
        if body.len().saturating_add(chunk.len()) > maximum {
            return Err(soda_upstream_error(format!(
                "{operation} response exceeded the size limit"
            )));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(body)
}

fn soda_network_error(error: reqwest::Error) -> TuneWeaveError {
    let code = if error.is_timeout() {
        ErrorCode::UpstreamTimeout
    } else {
        ErrorCode::UpstreamError
    };
    TuneWeaveError::new(code, "Soda API request failed")
        .with_platform(Platform::Soda)
        .retryable(true)
}

fn soda_http_error(status: StatusCode) -> TuneWeaveError {
    let code = if status == StatusCode::TOO_MANY_REQUESTS {
        ErrorCode::RateLimited
    } else {
        ErrorCode::UpstreamError
    };
    TuneWeaveError::new(code, format!("Soda API returned HTTP {status}"))
        .with_platform(Platform::Soda)
        .retryable(status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS)
}

fn soda_invalid_request(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Soda)
}

fn soda_upstream_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message).with_platform(Platform::Soda)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const SEARCH_RESPONSE: &str = r#"{
      "status_info":{"log_id":"safe-log","now":1785335031,"now_ts_ms":1785335031384},
      "result_groups":[{
        "id":"tracks","next_cursor":"20","has_more":true,
        "data":[{"entity":{"track":{
          "id":"7304719759323564095","name":"落了白","duration":180822,
          "vid":"v10ad6g50000d6po467og65ocf8m2mcg",
          "artists":[{"id":"6795393014723250177","name":"蒋雪儿Snow.J","url_avatar":{"uri":"tos-cn-v-2774c002/avatar","urls":["https://p3-luna.douyinpic.com/img/"]}}],
          "album":{"id":"7304719759323580479","name":"落了白","release_date":1606665600,"url_cover":{"uri":"tos-cn-v-2774c002/cover","urls":["https://p6-luna.douyinpic.com/img/"]}},
          "bit_rates":[{"br":66163,"size":1495476,"quality":"medium"},{"br":132219,"size":2988531,"quality":"higher"},{"br":260264,"size":5882690,"quality":"highest"}],
          "preview":{"vid":"preview","start":107904,"duration":60001,"bit_rates":[{"br":68413,"size":513101,"quality":"medium"}]},
          "audition_info":{"vid":"preview","start_time_ms":107904,"duration_ms":60001},
          "label_info":{"only_vip_download":true,"only_vip_playable":true,"quality_only_vip_can_download":["medium","higher","highest","lossless"],"quality_only_vip_can_play":["lossless"],"quality_map":{"lossless":{"play_detail":{"condition":"benefit_play_lossless","need_vip":true,"need_purchase":false}}}}
        }}}]
      }],
      "extra":{"log_extra":"redacted"}
    }"#;

    #[test]
    fn search_maps_stable_identity_metadata_catalog_quality_and_rights() {
        let page = parse_search_response(SEARCH_RESPONSE.as_bytes(), 0)
            .expect("parse Soda search response");
        assert!(page.has_more);
        assert_eq!(page.next_cursor, Some(20));
        assert_eq!(page.tracks.len(), 1);
        let track = &page.tracks[0];
        assert_eq!(track.resource_ref.to_string(), "soda:7304719759323564095");
        assert_eq!(track.name, "落了白");
        assert_eq!(track.duration_ms, Some(180_822));
        assert_eq!(track.artists[0].name, "蒋雪儿Snow.J");
        assert_eq!(
            track.album.as_ref().map(|album| album.name.as_str()),
            Some("落了白")
        );
        assert_eq!(
            track.available_qualities,
            [Quality::Low, Quality::Standard, Quality::High]
        );
        assert_eq!(track.playable, None);
        assert_eq!(track.extensions["rights"]["only_vip_playable"], true);
        assert_eq!(track.extensions["preview"]["duration"], 60_001);
        assert_eq!(
            track
                .album
                .as_ref()
                .and_then(|album| album.cover_url.as_deref()),
            Some("https://p6-luna.douyinpic.com/img/tos-cn-v-2774c002/cover")
        );
    }

    #[test]
    fn search_rejects_cursor_identity_and_group_drift() {
        let stalled = SEARCH_RESPONSE.replace("\"next_cursor\":\"20\"", "\"next_cursor\":\"0\"");
        assert!(parse_search_response(stalled.as_bytes(), 0).is_err());
        let invalid_id = SEARCH_RESPONSE.replace("7304719759323564095", "07304719759323564095");
        assert!(parse_search_response(invalid_id.as_bytes(), 0).is_err());
        let wrong_group = SEARCH_RESPONSE.replace("\"id\":\"tracks\"", "\"id\":\"albums\"");
        assert!(parse_search_response(wrong_group.as_bytes(), 0).is_err());
    }

    #[test]
    fn explicit_empty_search_is_a_stable_empty_page() {
        let page = parse_search_response(
            br#"{"status_info":{"log_id":"x","now":1,"now_ts_ms":1000},"result_groups":[],"extra":{"empty_search":1}}"#,
            0,
        )
        .expect("parse empty Soda search");
        assert!(page.tracks.is_empty());
        assert!(!page.has_more);
        assert_eq!(page.next_cursor, None);
    }

    #[test]
    fn images_accept_only_fixed_https_luna_image_nodes_and_paths() {
        let valid = SodaImage {
            uri: "tos-cn-v-2774c002/a_b-c.jpg".to_owned(),
            urls: vec!["https://p3-luna.douyinpic.com/img/".to_owned()],
        };
        assert_eq!(
            normalize_image(&valid).as_deref(),
            Some("https://p3-luna.douyinpic.com/img/tos-cn-v-2774c002/a_b-c.jpg")
        );
        for (base, uri) in [
            ("http://p3-luna.douyinpic.com/img/", "safe"),
            ("https://p3-luna.douyinpic.com:444/img/", "safe"),
            ("https://user@p3-luna.douyinpic.com/img/", "safe"),
            ("https://p3-luna.douyinpic.com/other/", "safe"),
            ("https://evil.example/img/", "safe"),
            ("https://p3-luna.douyinpic.com/img/", "../secret"),
            ("https://p3-luna.douyinpic.com/img/", "safe?token=x"),
        ] {
            assert!(
                normalize_image(&SodaImage {
                    uri: uri.to_owned(),
                    urls: vec![base.to_owned()],
                })
                .is_none(),
                "{base} {uri} must fail"
            );
        }
    }

    #[test]
    fn public_search_uses_a_fixed_https_endpoint_and_redacted_configuration() {
        let endpoint = Url::parse(SEARCH_ENDPOINT).expect("fixed Soda endpoint");
        assert_eq!(endpoint.scheme(), "https");
        assert_eq!(endpoint.host_str(), Some("api.qishui.com"));
        assert!(endpoint.username().is_empty());
        assert!(endpoint.password().is_none());
        assert!(matches!(endpoint.port(), None | Some(443)));
        assert!(endpoint.query().is_none());
        assert!(endpoint.fragment().is_none());
        assert_eq!(MAX_API_RESPONSE_BYTES, 8 * 1024 * 1024);

        let config = SodaConfig {
            proxy_url: Some("http://user:secret@example.test:8080".to_owned()),
        };
        let debug = format!("{config:?}");
        assert!(debug.contains("[configured]"));
        assert!(!debug.contains("secret"));
        assert!(!debug.contains("example.test"));
        assert_eq!(
            format!("{:?}", SodaClient::test_client()),
            "SodaClient { .. }"
        );
    }

    #[test]
    fn http_statuses_keep_rate_limits_and_retryable_failures_distinct() {
        let limited = soda_http_error(StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(limited.code, ErrorCode::RateLimited);
        assert!(limited.retryable);
        assert_eq!(limited.platform, Some(Platform::Soda));
        assert!(soda_http_error(StatusCode::SERVICE_UNAVAILABLE).retryable);
        assert!(!soda_http_error(StatusCode::BAD_REQUEST).retryable);
    }

    #[tokio::test]
    async fn response_reader_enforces_declared_and_streamed_size_limits() {
        let declared = raw_test_response(
            b"HTTP/1.1 200 OK\r\nContent-Length: 9000000\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(
            read_bounded_response(declared, "declared test")
                .await
                .is_err()
        );

        let bounded = raw_test_response(
            b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\nsafe",
        )
        .await;
        assert_eq!(
            read_bounded_response(bounded, "bounded test")
                .await
                .expect("bounded response"),
            b"safe"
        );
    }

    async fn raw_test_response(raw: &'static [u8]) -> reqwest::Response {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind test HTTP listener");
        let address = listener.local_addr().expect("test listener address");
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept test request");
            let mut request = [0_u8; 1_024];
            let _ = socket.read(&mut request).await.expect("read test request");
            socket.write_all(raw).await.expect("write test response");
            socket.shutdown().await.expect("close test response");
        });
        let response = reqwest::Client::builder()
            .redirect(Policy::none())
            .build()
            .expect("build test HTTP client")
            .get(format!("http://{address}/"))
            .send()
            .await
            .expect("receive test response");
        server.await.expect("test HTTP server");
        response
    }

    #[tokio::test]
    #[ignore = "requires live Soda network access"]
    async fn live_anonymous_search_uses_the_public_aid_without_device_or_signatures() {
        let page = SodaClient::test_client()
            .search_tracks_page("落了白", 0)
            .await
            .expect("live Soda search");
        assert_eq!(page.tracks.len(), 20);
        assert_eq!(page.next_cursor, Some(20));
        assert!(page.has_more);
        assert!(page.tracks.iter().all(|track| {
            track.platform == Platform::Soda
                && track.resource_ref.platform() == Platform::Soda
                && canonical_positive_decimal(track.resource_ref.id()).is_some()
                && !track.name.is_empty()
        }));
        assert!(
            page.tracks
                .iter()
                .any(|track| track.resource_ref.id() == "7304719759323564095")
        );
    }
}
