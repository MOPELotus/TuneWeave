use std::{
    collections::BTreeMap,
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use reqwest::{
    Client, Proxy, StatusCode,
    header::{CONTENT_LENGTH, REFERER},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tuneweave_core::{
    AlbumSummary, ArtistSummary, ErrorCode, Extensions, Platform, Quality, ResourceRef, Result,
    Track, TuneWeaveError,
};
use url::Url;

const SEARCH_ENDPOINT: &str = "https://songsearch.kugou.com/song_search_v2";
const WEB_REFERER: &str = "https://www.kugou.com/";
const WEB_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                             (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const MAX_SEARCH_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Clone, Default)]
pub struct KugouConfig {
    pub proxy_url: Option<String>,
}

impl fmt::Debug for KugouConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KugouConfig")
            .field(
                "proxy_url",
                &self.proxy_url.as_ref().map(|_| "[configured]"),
            )
            .finish()
    }
}

#[derive(Clone)]
pub struct KugouClient {
    http: Client,
    mid: String,
    uuid: String,
}

impl fmt::Debug for KugouClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KugouClient")
            .field("device_identity", &"[redacted]")
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub(crate) struct KugouSearchPage {
    pub total: u64,
    pub tracks: Vec<Track>,
}

#[derive(Serialize)]
struct KugouSearchRequest<'a> {
    keyword: &'a str,
    page: u32,
    pagesize: u32,
    userid: i64,
    clientver: &'static str,
    platform: &'static str,
    filter: u8,
    iscorrection: u8,
    privilege_filter: u8,
    srcappid: u16,
    clienttime: u64,
    mid: &'a str,
    uuid: &'a str,
    dfid: &'static str,
}

#[derive(Deserialize)]
struct KugouSearchEnvelope {
    status: i64,
    error_code: i64,
    #[serde(default)]
    error_msg: String,
    data: Option<KugouSearchData>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct KugouSearchData {
    pagesize: Option<FlexibleInteger>,
    page: Option<FlexibleInteger>,
    total: Option<FlexibleInteger>,
    lists: Vec<KugouSearchTrack>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct KugouSearchTrack {
    #[serde(
        rename = "MixSongID",
        alias = "mixsongid",
        alias = "mix_song_id",
        alias = "album_audio_id"
    )]
    mix_song_id: Option<FlexibleInteger>,
    #[serde(rename = "ID", alias = "id")]
    id: Option<FlexibleInteger>,
    #[serde(rename = "Audioid", alias = "audioid", alias = "audio_id")]
    audio_id: Option<FlexibleInteger>,
    #[serde(rename = "FileHash", alias = "filehash", alias = "hash")]
    file_hash: String,
    #[serde(rename = "FileSize", alias = "filesize")]
    file_size: Option<FlexibleInteger>,
    #[serde(rename = "Bitrate", alias = "bitrate")]
    bitrate: Option<FlexibleInteger>,
    #[serde(rename = "ExtName", alias = "extname")]
    extension: String,
    #[serde(rename = "SongName", alias = "songname")]
    song_name: String,
    #[serde(rename = "OriSongName", alias = "ori_song_name")]
    original_song_name: String,
    #[serde(rename = "FileName", alias = "filename")]
    file_name: String,
    #[serde(rename = "OtherName", alias = "othername")]
    other_name: String,
    #[serde(rename = "Suffix", alias = "suffix")]
    suffix: String,
    #[serde(rename = "Auxiliary", alias = "auxiliary")]
    auxiliary: String,
    #[serde(rename = "SingerName", alias = "singername")]
    singer_name: String,
    #[serde(rename = "Singers", alias = "singers")]
    singers: Vec<KugouSinger>,
    #[serde(rename = "AlbumID", alias = "album_id")]
    album_id: Option<FlexibleInteger>,
    #[serde(rename = "AlbumName", alias = "album_name")]
    album_name: String,
    #[serde(rename = "Image", alias = "image")]
    image: String,
    #[serde(rename = "Duration", alias = "duration")]
    duration_seconds: Option<FlexibleInteger>,
    #[serde(rename = "PublishDate", alias = "publish_date")]
    published_at: String,
    #[serde(rename = "MvHash", alias = "mvhash")]
    mv_hash: String,
    #[serde(rename = "mvdata")]
    mv_data: Vec<KugouMvIdentity>,
    #[serde(rename = "Privilege", alias = "privilege")]
    privilege: Option<FlexibleInteger>,
    #[serde(rename = "PayType", alias = "pay_type")]
    pay_type: Option<FlexibleInteger>,
    #[serde(rename = "FailProcess", alias = "fail_process")]
    fail_process: Option<FlexibleInteger>,
    #[serde(rename = "HQFileHash", alias = "hqhash")]
    high_hash: String,
    #[serde(rename = "HQFileSize")]
    high_size: Option<FlexibleInteger>,
    #[serde(rename = "HQBitrate")]
    high_bitrate: Option<FlexibleInteger>,
    #[serde(rename = "HQ")]
    high: KugouQualityAsset,
    #[serde(rename = "SQFileHash", alias = "sqhash")]
    lossless_hash: String,
    #[serde(rename = "SQFileSize")]
    lossless_size: Option<FlexibleInteger>,
    #[serde(rename = "SQBitrate")]
    lossless_bitrate: Option<FlexibleInteger>,
    #[serde(rename = "SQ")]
    lossless: KugouQualityAsset,
    #[serde(rename = "ResFileHash")]
    hires_hash: String,
    #[serde(rename = "ResFileSize")]
    hires_size: Option<FlexibleInteger>,
    #[serde(rename = "ResBitrate")]
    hires_bitrate: Option<FlexibleInteger>,
    #[serde(rename = "Res")]
    hires: KugouQualityAsset,
    #[serde(rename = "SuperFileHash")]
    master_hash: String,
    #[serde(rename = "SuperFileSize")]
    master_size: Option<FlexibleInteger>,
    #[serde(rename = "SuperBitrate")]
    master_bitrate: Option<FlexibleInteger>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct KugouSinger {
    id: Option<FlexibleInteger>,
    name: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct KugouMvIdentity {
    id: Option<FlexibleInteger>,
    hash: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct KugouQualityAsset {
    #[serde(rename = "Hash", alias = "hash")]
    hash: String,
    #[serde(rename = "FileSize", alias = "filesize")]
    file_size: Option<FlexibleInteger>,
    #[serde(rename = "BitRate", alias = "bitrate")]
    bitrate: Option<FlexibleInteger>,
    #[serde(rename = "Privilege", alias = "privilege")]
    privilege: Option<FlexibleInteger>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum FlexibleInteger {
    Unsigned(u64),
    Signed(i64),
    String(String),
}

impl FlexibleInteger {
    fn as_u64(&self) -> Option<u64> {
        match self {
            Self::Unsigned(value) => Some(*value),
            Self::Signed(value) => u64::try_from(*value).ok(),
            Self::String(value) => value.trim().parse().ok(),
        }
    }

    fn as_nonempty_string(&self) -> Option<String> {
        match self {
            Self::Unsigned(value) => Some(value.to_string()),
            Self::Signed(value) => Some(value.to_string()),
            Self::String(value) => nonempty(value).map(str::to_owned),
        }
    }

    fn as_resource_id(&self) -> Option<String> {
        let value = self.as_nonempty_string()?;
        (value != "0" && !value.starts_with('-')).then_some(value)
    }
}

impl KugouClient {
    pub fn new(config: &KugouConfig) -> Result<Self> {
        let mut builder = Client::builder()
            .user_agent(WEB_USER_AGENT)
            .redirect(Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20));
        if let Some(proxy_url) = config
            .proxy_url
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        {
            let proxy = Proxy::all(proxy_url).map_err(|_| {
                TuneWeaveError::invalid_request("KuGou proxy URL is invalid")
                    .with_platform(Platform::Kugou)
            })?;
            builder = builder.proxy(proxy);
        }
        let http = builder.build().map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "failed to build KuGou HTTP client",
            )
            .with_platform(Platform::Kugou)
        })?;
        let identity = hex::encode_upper(rand::random::<[u8; 16]>());
        Ok(Self {
            http,
            mid: identity.clone(),
            uuid: identity,
        })
    }

    pub(crate) async fn search_tracks_page(
        &self,
        keyword: &str,
        page: u32,
        page_size: u32,
    ) -> Result<KugouSearchPage> {
        let request = KugouSearchRequest {
            keyword,
            page,
            pagesize: page_size,
            userid: -1,
            clientver: "",
            platform: "WebFilter",
            filter: 2,
            iscorrection: 1,
            privilege_filter: 0,
            srcappid: 2919,
            clienttime: unix_seconds_now(),
            mid: &self.mid,
            uuid: &self.uuid,
            dfid: "-",
        };
        let response = self
            .http
            .get(SEARCH_ENDPOINT)
            .header(REFERER, WEB_REFERER)
            .query(&request)
            .send()
            .await
            .map_err(kugou_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(kugou_http_error(status));
        }
        if response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .is_some_and(|length| length > MAX_SEARCH_RESPONSE_BYTES)
        {
            return Err(kugou_upstream_error(
                "KuGou search response exceeded the size limit",
            ));
        }
        let bytes = response.bytes().await.map_err(kugou_network_error)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SEARCH_RESPONSE_BYTES {
            return Err(kugou_upstream_error(
                "KuGou search response exceeded the size limit",
            ));
        }
        parse_search_response(&bytes)
    }
}

fn parse_search_response(bytes: &[u8]) -> Result<KugouSearchPage> {
    let envelope: KugouSearchEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| kugou_upstream_error("KuGou search returned malformed JSON"))?;
    if envelope.status != 1 || envelope.error_code != 0 {
        return Err(
            kugou_upstream_error("KuGou search rejected the request").with_details(json!({
                "platform_code": envelope.error_code,
                "platform_message": safe_upstream_message(&envelope.error_msg),
            })),
        );
    }
    let data = envelope
        .data
        .ok_or_else(|| kugou_upstream_error("KuGou search response omitted data"))?;
    let reported_page_size = data.pagesize.as_ref().and_then(FlexibleInteger::as_u64);
    let reported_page = data.page.as_ref().and_then(FlexibleInteger::as_u64);
    let total = data
        .total
        .as_ref()
        .and_then(FlexibleInteger::as_u64)
        .ok_or_else(|| kugou_upstream_error("KuGou search response omitted a valid total"))?;
    let tracks = data
        .lists
        .into_iter()
        .map(map_search_track)
        .collect::<Result<Vec<_>>>()?;
    if reported_page == Some(0) || reported_page_size == Some(0) && !tracks.is_empty() {
        return Err(kugou_upstream_error(
            "KuGou search returned inconsistent pagination",
        ));
    }
    Ok(KugouSearchPage { total, tracks })
}

fn map_search_track(item: KugouSearchTrack) -> Result<Track> {
    let album_audio_id = item
        .mix_song_id
        .as_ref()
        .and_then(FlexibleInteger::as_resource_id);
    let track_id = album_audio_id
        .clone()
        .or_else(|| item.id.as_ref().and_then(FlexibleInteger::as_resource_id))
        .or_else(|| {
            item.audio_id
                .as_ref()
                .and_then(FlexibleInteger::as_resource_id)
        })
        .or_else(|| nonempty(&item.file_hash).map(str::to_owned))
        .ok_or_else(|| kugou_upstream_error("KuGou search result omitted a stable identity"))?;
    let name = nonempty(&item.song_name)
        .or_else(|| nonempty(&item.original_song_name))
        .or_else(|| nonempty(&item.file_name))
        .ok_or_else(|| kugou_upstream_error("KuGou search result omitted a track name"))?;
    let resource_ref = ResourceRef::new(Platform::Kugou, track_id)
        .map_err(|_| kugou_upstream_error("KuGou search returned an invalid track identity"))?;
    let mut track = Track::new(resource_ref, name);

    for alias in [&item.other_name, &item.suffix, &item.auxiliary]
        .into_iter()
        .filter_map(|value| nonempty(value))
    {
        if alias != track.name && !track.aliases.iter().any(|value| value == alias) {
            track.aliases.push(alias.to_owned());
        }
    }
    track.artists = map_singers(&item.singers, &item.singer_name);
    track.album = map_album(item.album_id.as_ref(), &item.album_name, &item.image);
    track.duration_ms = item
        .duration_seconds
        .as_ref()
        .and_then(FlexibleInteger::as_u64)
        .and_then(|seconds| seconds.checked_mul(1_000));
    track.mv_ref = map_mv(&item.mv_data, &item.mv_hash);

    let standard = quality_asset(
        &item.file_hash,
        item.file_size.as_ref(),
        item.bitrate.as_ref(),
        None,
    );
    let high = merged_quality_asset(
        &item.high_hash,
        item.high_size.as_ref(),
        item.high_bitrate.as_ref(),
        &item.high,
    );
    let lossless = merged_quality_asset(
        &item.lossless_hash,
        item.lossless_size.as_ref(),
        item.lossless_bitrate.as_ref(),
        &item.lossless,
    );
    let hires = merged_quality_asset(
        &item.hires_hash,
        item.hires_size.as_ref(),
        item.hires_bitrate.as_ref(),
        &item.hires,
    );
    let master = quality_asset(
        &item.master_hash,
        item.master_size.as_ref(),
        item.master_bitrate.as_ref(),
        None,
    );
    let mut qualities = BTreeMap::new();
    add_quality(
        &mut qualities,
        &mut track.available_qualities,
        "standard",
        Quality::Standard,
        standard,
    );
    add_quality(
        &mut qualities,
        &mut track.available_qualities,
        "high",
        Quality::High,
        high,
    );
    add_quality(
        &mut qualities,
        &mut track.available_qualities,
        "lossless",
        Quality::Lossless,
        lossless,
    );
    add_quality(
        &mut qualities,
        &mut track.available_qualities,
        "hires",
        Quality::Hires,
        hires,
    );
    add_quality(
        &mut qualities,
        &mut track.available_qualities,
        "master",
        Quality::Master,
        master,
    );

    insert_optional(
        &mut track.extensions,
        "hash",
        nonempty(&item.file_hash).map(str::to_owned),
    );
    insert_optional(&mut track.extensions, "album_audio_id", album_audio_id);
    insert_optional(
        &mut track.extensions,
        "audio_id",
        item.audio_id
            .as_ref()
            .and_then(FlexibleInteger::as_resource_id),
    );
    insert_optional(
        &mut track.extensions,
        "published_at",
        nonempty(&item.published_at).map(str::to_owned),
    );
    insert_optional(
        &mut track.extensions,
        "extension",
        nonempty(&item.extension).map(str::to_owned),
    );
    insert_optional_u64(
        &mut track.extensions,
        "privilege",
        item.privilege.as_ref().and_then(FlexibleInteger::as_u64),
    );
    insert_optional_u64(
        &mut track.extensions,
        "pay_type",
        item.pay_type.as_ref().and_then(FlexibleInteger::as_u64),
    );
    insert_optional_u64(
        &mut track.extensions,
        "fail_process",
        item.fail_process.as_ref().and_then(FlexibleInteger::as_u64),
    );
    track
        .extensions
        .insert("qualities".to_owned(), json!(qualities));
    track
        .extensions
        .insert("search_backend".to_owned(), json!("song_search_v2"));
    Ok(track)
}

fn map_singers(singers: &[KugouSinger], fallback: &str) -> Vec<ArtistSummary> {
    let mut artists = singers
        .iter()
        .filter_map(|singer| {
            let name = nonempty(&singer.name)?;
            let resource_ref = singer
                .id
                .as_ref()
                .and_then(FlexibleInteger::as_resource_id)
                .and_then(|id| ResourceRef::new(Platform::Kugou, id).ok());
            Some(ArtistSummary {
                resource_ref,
                name: name.to_owned(),
            })
        })
        .collect::<Vec<_>>();
    if artists.is_empty() {
        if let Some(name) = nonempty(fallback) {
            artists.push(ArtistSummary {
                resource_ref: None,
                name: name.to_owned(),
            });
        }
    }
    artists
}

fn map_album(id: Option<&FlexibleInteger>, name: &str, image: &str) -> Option<AlbumSummary> {
    let name = nonempty(name)?;
    let resource_ref = id
        .and_then(FlexibleInteger::as_resource_id)
        .and_then(|id| ResourceRef::new(Platform::Kugou, id).ok());
    Some(AlbumSummary {
        resource_ref,
        name: name.to_owned(),
        cover_url: normalize_image_url(image),
    })
}

fn map_mv(identities: &[KugouMvIdentity], fallback_hash: &str) -> Option<ResourceRef> {
    identities
        .iter()
        .find_map(|identity| {
            identity
                .id
                .as_ref()
                .and_then(FlexibleInteger::as_resource_id)
                .or_else(|| nonempty(&identity.hash).map(|hash| format!("hash:{hash}")))
        })
        .or_else(|| nonempty(fallback_hash).map(|hash| format!("hash:{hash}")))
        .and_then(|id| ResourceRef::new(Platform::Kugou, id).ok())
}

fn merged_quality_asset(
    flat_hash: &str,
    flat_size: Option<&FlexibleInteger>,
    flat_bitrate: Option<&FlexibleInteger>,
    nested: &KugouQualityAsset,
) -> Option<Value> {
    let hash = nonempty(flat_hash).or_else(|| nonempty(&nested.hash))?;
    Some(json!({
        "hash": hash,
        "size": flat_size
            .and_then(FlexibleInteger::as_u64)
            .or_else(|| nested.file_size.as_ref().and_then(FlexibleInteger::as_u64)),
        "bitrate": flat_bitrate
            .and_then(FlexibleInteger::as_u64)
            .or_else(|| nested.bitrate.as_ref().and_then(FlexibleInteger::as_u64)),
        "privilege": nested.privilege.as_ref().and_then(FlexibleInteger::as_u64),
    }))
}

fn quality_asset(
    hash: &str,
    size: Option<&FlexibleInteger>,
    bitrate: Option<&FlexibleInteger>,
    privilege: Option<&FlexibleInteger>,
) -> Option<Value> {
    let hash = nonempty(hash)?;
    Some(json!({
        "hash": hash,
        "size": size.and_then(FlexibleInteger::as_u64),
        "bitrate": bitrate.and_then(FlexibleInteger::as_u64),
        "privilege": privilege.and_then(FlexibleInteger::as_u64),
    }))
}

fn add_quality(
    assets: &mut BTreeMap<&'static str, Value>,
    qualities: &mut Vec<Quality>,
    name: &'static str,
    quality: Quality,
    asset: Option<Value>,
) {
    if let Some(asset) = asset {
        assets.insert(name, asset);
        qualities.push(quality);
    }
}

fn insert_optional(extensions: &mut Extensions, key: &str, value: Option<String>) {
    if let Some(value) = value {
        extensions.insert(key.to_owned(), Value::String(value));
    }
}

fn insert_optional_u64(extensions: &mut Extensions, key: &str, value: Option<u64>) {
    if let Some(value) = value {
        extensions.insert(key.to_owned(), json!(value));
    }
}

fn normalize_image_url(value: &str) -> Option<String> {
    let value = nonempty(value)?;
    let mut value = value.replace("{size}", "400");
    if let Some(rest) = value.strip_prefix("//") {
        value = format!("https://{rest}");
    } else if let Some(rest) = value.strip_prefix("http://") {
        value = format!("https://{rest}");
    }
    let url = Url::parse(&value).ok()?;
    let host = url.host_str()?;
    let trusted = url.scheme() == "https"
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.fragment().is_none()
        && (host == "kugou.com" || host.ends_with(".kugou.com"));
    trusted.then_some(value)
}

fn nonempty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn safe_upstream_message(value: &str) -> Option<String> {
    let value = nonempty(value)?;
    if value.chars().any(char::is_control) {
        return None;
    }
    Some(value.chars().take(256).collect())
}

fn unix_seconds_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn kugou_network_error(error: reqwest::Error) -> TuneWeaveError {
    let code = if error.is_timeout() {
        ErrorCode::UpstreamTimeout
    } else {
        ErrorCode::UpstreamError
    };
    TuneWeaveError::new(code, "KuGou API request failed")
        .with_platform(Platform::Kugou)
        .retryable(true)
}

fn kugou_http_error(status: StatusCode) -> TuneWeaveError {
    let code = if status == StatusCode::TOO_MANY_REQUESTS {
        ErrorCode::RateLimited
    } else {
        ErrorCode::UpstreamError
    };
    TuneWeaveError::new(code, format!("KuGou API returned HTTP {status}"))
        .with_platform(Platform::Kugou)
        .retryable(status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS)
}

fn kugou_upstream_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message).with_platform(Platform::Kugou)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_debug_does_not_expose_proxy_credentials() {
        let config = KugouConfig {
            proxy_url: Some("http://secret:password@127.0.0.1:8080".to_owned()),
        };
        let rendered = format!("{config:?}");
        assert!(rendered.contains("[configured]"));
        assert!(!rendered.contains("secret"));
        assert!(!rendered.contains("password"));
    }

    #[test]
    fn search_response_maps_stable_identity_metadata_and_quality_assets() {
        let response = r#"{
          "status": 1,
          "error_code": 0,
          "error_msg": "",
          "data": {
            "pagesize": 30,
            "page": 1,
            "total": 1,
            "lists": [{
              "MixSongID": "32100650",
              "Audioid": 20505418,
              "FileHash": "BASEHASH",
              "FileSize": 4317292,
              "Bitrate": 128,
              "ExtName": "mp3",
              "SongName": "反方向的钟",
              "OtherName": "Clockwise",
              "SingerName": "周杰伦",
              "Singers": [{"id": 3520, "name": "周杰伦"}],
              "AlbumID": "966846",
              "AlbumName": "Jay",
              "Image": "http://imge.kugou.com/stdmusic/{size}/cover.jpg",
              "Duration": 258,
              "PublishDate": "2000-11-07",
              "Privilege": 10,
              "PayType": 3,
              "FailProcess": 4,
              "HQFileHash": "HIGHHASH",
              "HQFileSize": 10335129,
              "HQBitrate": 320,
              "SQ": {"Hash": "LOSSLESSHASH", "FileSize": 31729524, "BitRate": 940},
              "ResFileHash": "HIRESHASH",
              "ResFileSize": 55296047,
              "ResBitrate": 1639,
              "mvdata": [{"id": "13824228", "hash": "MVHASH"}]
            }]
          }
        }"#;
        let page = parse_search_response(response.as_bytes()).expect("parse search response");
        assert_eq!(page.total, 1);
        let track = &page.tracks[0];
        assert_eq!(track.resource_ref.to_string(), "kugou:32100650");
        assert_eq!(track.name, "反方向的钟");
        assert_eq!(track.aliases, ["Clockwise"]);
        assert_eq!(
            track.artists[0]
                .resource_ref
                .as_ref()
                .expect("artist reference")
                .to_string(),
            "kugou:3520"
        );
        assert_eq!(
            track
                .album
                .as_ref()
                .and_then(|album| album.cover_url.as_deref()),
            Some("https://imge.kugou.com/stdmusic/400/cover.jpg")
        );
        assert_eq!(track.duration_ms, Some(258_000));
        assert_eq!(
            track.mv_ref.as_ref().map(ToString::to_string).as_deref(),
            Some("kugou:13824228")
        );
        assert_eq!(
            track.available_qualities,
            [
                Quality::Standard,
                Quality::High,
                Quality::Lossless,
                Quality::Hires
            ]
        );
        assert_eq!(track.extensions["hash"], "BASEHASH");
        assert_eq!(track.extensions["album_audio_id"], "32100650");
        assert_eq!(
            track.extensions["qualities"]["lossless"]["hash"],
            "LOSSLESSHASH"
        );
        assert!(track.playable.is_none());
    }

    #[test]
    fn search_response_rejects_business_errors_without_echoing_the_body() {
        let error = parse_search_response(
            br#"{"status":1,"error_code":152,"error_msg":"Parameter Error","data":null}"#,
        )
        .expect_err("reject business error");
        let rendered = format!("{error:?}");
        assert!(rendered.contains("152"));
        assert!(!rendered.contains("\"data\""));
    }

    #[test]
    fn search_response_rejects_items_without_identity_or_name() {
        let missing_identity = br#"{
          "status":1,"error_code":0,
          "data":{"pagesize":30,"page":1,"total":1,"lists":[{"SongName":"name"}]}
        }"#;
        assert!(parse_search_response(missing_identity).is_err());

        let missing_name = br#"{
          "status":1,"error_code":0,
          "data":{"pagesize":30,"page":1,"total":1,"lists":[{"FileHash":"HASH"}]}
        }"#;
        assert!(parse_search_response(missing_name).is_err());
    }

    #[test]
    fn image_normalization_only_accepts_trusted_https_kugou_hosts() {
        assert_eq!(
            normalize_image_url("//imge.kugou.com/stdmusic/{size}/cover.jpg").as_deref(),
            Some("https://imge.kugou.com/stdmusic/400/cover.jpg")
        );
        assert!(normalize_image_url("https://example.com/cover.jpg").is_none());
        assert!(normalize_image_url("https://user@imge.kugou.com/cover.jpg").is_none());
        assert!(normalize_image_url("https://imge.kugou.com:444/cover.jpg").is_none());
    }

    #[tokio::test]
    #[ignore = "requires live KuGou network access"]
    async fn live_public_track_search_returns_stable_results() {
        let client = KugouClient::new(&KugouConfig::default()).expect("create client");
        let page = client
            .search_tracks_page("周杰伦", 1, 3)
            .await
            .expect("live KuGou search");
        assert!(page.total > 0);
        assert!(!page.tracks.is_empty());
        assert!(
            page.tracks
                .iter()
                .all(|track| track.platform == Platform::Kugou)
        );
    }
}
