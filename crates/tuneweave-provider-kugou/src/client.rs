use std::{
    collections::BTreeMap,
    fmt,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use md5::{Digest, Md5};
use reqwest::{
    Client, Proxy, StatusCode,
    header::{CONTENT_LENGTH, CONTENT_TYPE, REFERER},
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
const ANDROID_GATEWAY: &str = "https://gateway.kugou.com";
const WEB_REFERER: &str = "https://www.kugou.com/";
const WEB_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                             (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const ANDROID_USER_AGENT: &str = "Android15-1070-11083-46-0-DiscoveryDRADProtocol-wifi";
const ANDROID_SIGNATURE_SALT: &str = "OIlwieks28dk2k092lksi2UIkp";
const ANDROID_APP_ID: u16 = 1005;
const ANDROID_CLIENT_VERSION: u32 = 20489;
const MAX_API_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;

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

#[derive(Clone, Copy)]
enum AndroidEndpoint {
    TrackMetadata,
    AudioMetadata,
}

impl AndroidEndpoint {
    const fn path(self) -> &'static str {
        match self {
            Self::TrackMetadata => "/kmr/v2/audio",
            Self::AudioMetadata => "/v1/audio/audio",
        }
    }

    const fn router(self) -> &'static str {
        match self {
            Self::TrackMetadata => "openapi.kugou.com",
            Self::AudioMetadata => "kmr.service.kugou.com",
        }
    }

    const fn kg_tid(self) -> Option<&'static str> {
        match self {
            Self::TrackMetadata => Some("238"),
            Self::AudioMetadata => None,
        }
    }
}

#[derive(Serialize)]
struct AndroidQuery<'a> {
    dfid: &'static str,
    mid: &'a str,
    uuid: &'static str,
    appid: u16,
    clientver: u32,
    clienttime: u64,
    signature: String,
}

#[derive(Serialize)]
struct TrackMetadataRequest {
    data: [TrackMetadataIdentity; 1],
    fields: &'static str,
}

#[derive(Serialize)]
struct TrackMetadataIdentity {
    entity_id: u64,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct TrackMetadataEnvelope {
    status: i64,
    error_code: i64,
    msg: String,
    data: Vec<TrackMetadata>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct TrackMetadata {
    #[serde(rename = "__status")]
    status: i64,
    authors: Vec<TrackAuthor>,
    album_info: Option<TrackAlbumInfo>,
    #[serde(rename = "class")]
    classifications: Vec<TrackClassification>,
    base: Option<TrackMetadataBase>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct TrackAuthor {
    sisp: Option<FlexibleInteger>,
    identity: Option<FlexibleInteger>,
    base: Option<TrackAuthorBase>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct TrackAuthorBase {
    author_id: Option<FlexibleInteger>,
    author_name: String,
    is_publish: Option<FlexibleInteger>,
    language: String,
    avatar: String,
    identity: Option<FlexibleInteger>,
    #[serde(rename = "type")]
    author_type: Option<FlexibleInteger>,
    country: String,
    birthday: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct TrackAlbumInfo {
    album_id: Option<FlexibleInteger>,
    album_name: String,
    publish_date: String,
    is_publish: Option<FlexibleInteger>,
    cover: String,
    category: Option<FlexibleInteger>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct TrackClassification {
    status: Option<FlexibleInteger>,
    usage: Option<FlexibleInteger>,
    #[serde(rename = "type")]
    class_type: Option<FlexibleInteger>,
    level: Option<FlexibleInteger>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct TrackMetadataBase {
    album_id: Option<FlexibleInteger>,
    songname: String,
    author_name: String,
    album_name: String,
    version: Option<FlexibleInteger>,
    language: String,
    publish_date: String,
    isrc: String,
    wide_audio_id: Option<FlexibleInteger>,
    is_publish: Option<FlexibleInteger>,
    provider: Option<FlexibleInteger>,
    big_pack_id: Option<FlexibleInteger>,
    final_id: Option<FlexibleInteger>,
    audio_id: Option<FlexibleInteger>,
    similar_audio_id: Option<FlexibleInteger>,
    is_hot: Option<FlexibleInteger>,
    #[serde(rename = "_raw_publish")]
    raw_publish: Option<FlexibleInteger>,
    album_audio_id: Option<FlexibleInteger>,
    audio_group_id: Option<FlexibleInteger>,
}

#[derive(Serialize)]
struct AudioMetadataRequest<'a> {
    appid: u16,
    clienttime: u64,
    clientver: u32,
    data: [AudioMetadataIdentity<'a>; 1],
    dfid: &'static str,
    key: String,
    mid: &'a str,
}

#[derive(Serialize)]
struct AudioMetadataIdentity<'a> {
    hash: &'static str,
    audio_id: &'a str,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct AudioMetadataEnvelope {
    status: i64,
    error_code: i64,
    errcode: i64,
    errmsg: String,
    data: Vec<AudioMetadata>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct AudioMetadata {
    timelength: Option<FlexibleInteger>,
    audio_group_id: Option<FlexibleInteger>,
    audio_name: String,
    audio_id: Option<FlexibleInteger>,
    is_publish: Option<FlexibleInteger>,
    language: String,
    hash: String,
    hash_128: String,
    filesize: Option<FlexibleInteger>,
    filesize_128: Option<FlexibleInteger>,
    bitrate: Option<FlexibleInteger>,
    timelength_128: Option<FlexibleInteger>,
    hash_320: String,
    filesize_320: Option<FlexibleInteger>,
    timelength_320: Option<FlexibleInteger>,
    hash_flac: String,
    filesize_flac: Option<FlexibleInteger>,
    bitrate_flac: Option<FlexibleInteger>,
    timelength_flac: Option<FlexibleInteger>,
    hash_ape: String,
    filesize_ape: Option<FlexibleInteger>,
    bitrate_ape: Option<FlexibleInteger>,
    timelength_ape: Option<FlexibleInteger>,
    hash_high: String,
    filesize_high: Option<FlexibleInteger>,
    bitrate_high: Option<FlexibleInteger>,
    timelength_high: Option<FlexibleInteger>,
    hash_super: String,
    filesize_super: Option<FlexibleInteger>,
    bitrate_super: Option<FlexibleInteger>,
    timelength_super: Option<FlexibleInteger>,
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
        let identity = u128::from_be_bytes(rand::random::<[u8; 16]>()).to_string();
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
        let bytes = read_bounded_response(response, "KuGou search").await?;
        parse_search_response(&bytes)
    }

    pub(crate) async fn track_detail(&self, album_audio_id: u64) -> Result<Track> {
        let metadata_request = TrackMetadataRequest {
            data: [TrackMetadataIdentity {
                entity_id: album_audio_id,
            }],
            fields: "base,album_info,authors.base,class",
        };
        let metadata_bytes = self
            .post_android(AndroidEndpoint::TrackMetadata, &metadata_request)
            .await?;
        let metadata = parse_track_metadata(&metadata_bytes, album_audio_id)?;
        let audio_id = metadata
            .base
            .as_ref()
            .and_then(|base| base.audio_id.as_ref())
            .and_then(FlexibleInteger::as_resource_id)
            .ok_or_else(|| kugou_upstream_error("KuGou track detail omitted a valid audio ID"))?;

        let clienttime = unix_milliseconds_now();
        let audio_request = AudioMetadataRequest {
            appid: ANDROID_APP_ID,
            clienttime,
            clientver: ANDROID_CLIENT_VERSION,
            data: [AudioMetadataIdentity {
                hash: "",
                audio_id: &audio_id,
            }],
            dfid: "-",
            key: md5_hex(format!(
                "{ANDROID_APP_ID}{ANDROID_SIGNATURE_SALT}{ANDROID_CLIENT_VERSION}{clienttime}"
            )),
            mid: &self.mid,
        };
        let audio_bytes = self
            .post_android(AndroidEndpoint::AudioMetadata, &audio_request)
            .await?;
        let audio = parse_audio_metadata(&audio_bytes, &audio_id)?;
        map_track_detail(album_audio_id, metadata, audio)
    }

    async fn post_android<T: Serialize>(
        &self,
        endpoint: AndroidEndpoint,
        body: &T,
    ) -> Result<Vec<u8>> {
        let body = serde_json::to_vec(body).map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "failed to serialize KuGou request",
            )
            .with_platform(Platform::Kugou)
        })?;
        let clienttime = unix_seconds_now();
        let signature = android_signature(&self.mid, clienttime, &body);
        let query = AndroidQuery {
            dfid: "-",
            mid: &self.mid,
            uuid: "-",
            appid: ANDROID_APP_ID,
            clientver: ANDROID_CLIENT_VERSION,
            clienttime,
            signature,
        };
        let mut request = self
            .http
            .post(format!("{ANDROID_GATEWAY}{}", endpoint.path()))
            .header("x-router", endpoint.router())
            .header(CONTENT_TYPE, "application/json")
            .header("dfid", "-")
            .header("clienttime", clienttime)
            .header("mid", &self.mid)
            .header("kg-rc", "1")
            .header("kg-thash", "5d816a0")
            .header("kg-rec", "1")
            .header("kg-rf", "B9EDA08A64250DEFFBCADDEE00F8F25F")
            .header("user-agent", ANDROID_USER_AGENT)
            .query(&query)
            .body(body);
        if let Some(kg_tid) = endpoint.kg_tid() {
            request = request.header("kg-tid", kg_tid);
        }
        let response = request.send().await.map_err(kugou_network_error)?;
        read_bounded_response(response, "KuGou track detail").await
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

fn parse_track_metadata(bytes: &[u8], album_audio_id: u64) -> Result<TrackMetadata> {
    let envelope: TrackMetadataEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| kugou_upstream_error("KuGou track detail returned malformed JSON"))?;
    if envelope.status != 1 || envelope.error_code != 0 {
        return Err(
            kugou_upstream_error("KuGou track detail rejected the request").with_details(json!({
                "platform_code": envelope.error_code,
                "platform_message": safe_upstream_message(&envelope.msg),
            })),
        );
    }
    let [metadata] = <[TrackMetadata; 1]>::try_from(envelope.data).map_err(|_| {
        kugou_upstream_error("KuGou track detail returned an unexpected result count")
    })?;
    if metadata.status != 1 {
        return Err(kugou_upstream_error(
            "KuGou track detail did not resolve the requested identity",
        ));
    }
    let returned_id = metadata
        .base
        .as_ref()
        .and_then(|base| base.album_audio_id.as_ref())
        .and_then(FlexibleInteger::as_u64);
    if returned_id != Some(album_audio_id) {
        return Err(kugou_upstream_error(
            "KuGou track detail returned a mismatched album audio identity",
        ));
    }
    Ok(metadata)
}

fn parse_audio_metadata(bytes: &[u8], audio_id: &str) -> Result<AudioMetadata> {
    let envelope: AudioMetadataEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| kugou_upstream_error("KuGou audio metadata returned malformed JSON"))?;
    if envelope.status != 1 || envelope.error_code != 0 || envelope.errcode != 0 {
        let code = if envelope.error_code != 0 {
            envelope.error_code
        } else {
            envelope.errcode
        };
        return Err(
            kugou_upstream_error("KuGou audio metadata rejected the request").with_details(json!({
                "platform_code": code,
                "platform_message": safe_upstream_message(&envelope.errmsg),
            })),
        );
    }
    let [audio] = <[AudioMetadata; 1]>::try_from(envelope.data).map_err(|_| {
        kugou_upstream_error("KuGou audio metadata returned an unexpected result count")
    })?;
    let returned_id = audio
        .audio_id
        .as_ref()
        .and_then(FlexibleInteger::as_resource_id);
    if returned_id.as_deref() != Some(audio_id) {
        return Err(kugou_upstream_error(
            "KuGou audio metadata returned a mismatched audio identity",
        ));
    }
    if nonempty(&audio.audio_name).is_none()
        || nonempty(&audio.hash)
            .or_else(|| nonempty(&audio.hash_128))
            .is_none()
    {
        return Err(kugou_upstream_error(
            "KuGou audio metadata omitted required base metadata",
        ));
    }
    Ok(audio)
}

fn map_track_detail(
    album_audio_id: u64,
    metadata: TrackMetadata,
    audio: AudioMetadata,
) -> Result<Track> {
    let base = metadata
        .base
        .ok_or_else(|| kugou_upstream_error("KuGou track detail omitted base metadata"))?;
    let name = nonempty(&base.songname)
        .ok_or_else(|| kugou_upstream_error("KuGou track detail omitted a track name"))?;
    validate_detail_consistency(&base, metadata.album_info.as_ref(), &audio)?;

    let resource_ref = ResourceRef::new(Platform::Kugou, album_audio_id.to_string())
        .map_err(|_| kugou_upstream_error("KuGou track detail returned an invalid identity"))?;
    let mut track = Track::new(resource_ref, name);
    track.artists = map_detail_artists(&metadata.authors, &base.author_name);
    track.album = map_detail_album(metadata.album_info.as_ref(), &base);
    track.duration_ms = audio.timelength.as_ref().and_then(FlexibleInteger::as_u64);
    track.isrc = nonempty(&base.isrc).map(str::to_owned);

    let standard_hash = nonempty(&audio.hash_128).or_else(|| nonempty(&audio.hash));
    let standard = detail_quality_asset(
        standard_hash,
        audio
            .filesize_128
            .as_ref()
            .and_then(FlexibleInteger::as_u64)
            .or_else(|| audio.filesize.as_ref().and_then(FlexibleInteger::as_u64)),
        audio.bitrate.as_ref().and_then(FlexibleInteger::as_u64),
        audio
            .timelength_128
            .as_ref()
            .and_then(FlexibleInteger::as_u64)
            .or(track.duration_ms),
        "mp3",
    );
    let high = detail_quality_asset(
        nonempty(&audio.hash_320),
        audio
            .filesize_320
            .as_ref()
            .and_then(FlexibleInteger::as_u64),
        Some(320),
        audio
            .timelength_320
            .as_ref()
            .and_then(FlexibleInteger::as_u64),
        "mp3",
    );
    let flac = detail_quality_asset(
        nonempty(&audio.hash_flac),
        audio
            .filesize_flac
            .as_ref()
            .and_then(FlexibleInteger::as_u64),
        audio
            .bitrate_flac
            .as_ref()
            .and_then(FlexibleInteger::as_u64),
        audio
            .timelength_flac
            .as_ref()
            .and_then(FlexibleInteger::as_u64),
        "flac",
    );
    let ape = detail_quality_asset(
        nonempty(&audio.hash_ape),
        audio
            .filesize_ape
            .as_ref()
            .and_then(FlexibleInteger::as_u64),
        audio.bitrate_ape.as_ref().and_then(FlexibleInteger::as_u64),
        audio
            .timelength_ape
            .as_ref()
            .and_then(FlexibleInteger::as_u64),
        "ape",
    );
    let hires = detail_quality_asset(
        nonempty(&audio.hash_high),
        audio
            .filesize_high
            .as_ref()
            .and_then(FlexibleInteger::as_u64),
        audio
            .bitrate_high
            .as_ref()
            .and_then(FlexibleInteger::as_u64),
        audio
            .timelength_high
            .as_ref()
            .and_then(FlexibleInteger::as_u64),
        "flac",
    );
    let master = detail_quality_asset(
        nonempty(&audio.hash_super),
        audio
            .filesize_super
            .as_ref()
            .and_then(FlexibleInteger::as_u64),
        audio
            .bitrate_super
            .as_ref()
            .and_then(FlexibleInteger::as_u64),
        audio
            .timelength_super
            .as_ref()
            .and_then(FlexibleInteger::as_u64),
        "flac",
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
    let lossless = flac.or_else(|| ape.clone());
    add_quality(
        &mut qualities,
        &mut track.available_qualities,
        "lossless",
        Quality::Lossless,
        lossless,
    );
    if let Some(ape) = ape {
        qualities.insert("lossless_ape", ape);
    }
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
        "album_audio_id",
        Some(album_audio_id.to_string()),
    );
    insert_optional(
        &mut track.extensions,
        "audio_id",
        base.audio_id
            .as_ref()
            .and_then(FlexibleInteger::as_resource_id),
    );
    insert_optional(
        &mut track.extensions,
        "hash",
        standard_hash.map(str::to_owned),
    );
    insert_optional(
        &mut track.extensions,
        "language",
        nonempty(&base.language)
            .or_else(|| nonempty(&audio.language))
            .map(str::to_owned),
    );
    insert_optional(
        &mut track.extensions,
        "published_at",
        nonempty(&base.publish_date)
            .or_else(|| {
                metadata
                    .album_info
                    .as_ref()
                    .and_then(|album| nonempty(&album.publish_date))
            })
            .map(str::to_owned),
    );
    for (key, value) in [
        ("version", base.version.as_ref()),
        ("wide_audio_id", base.wide_audio_id.as_ref()),
        ("is_publish", base.is_publish.as_ref()),
        ("provider", base.provider.as_ref()),
        ("big_pack_id", base.big_pack_id.as_ref()),
        ("final_id", base.final_id.as_ref()),
        ("similar_audio_id", base.similar_audio_id.as_ref()),
        ("is_hot", base.is_hot.as_ref()),
        ("raw_publish", base.raw_publish.as_ref()),
        ("audio_group_id", base.audio_group_id.as_ref()),
    ] {
        insert_optional_u64(
            &mut track.extensions,
            key,
            value.and_then(FlexibleInteger::as_u64),
        );
    }
    insert_optional(
        &mut track.extensions,
        "audio_name",
        nonempty(&audio.audio_name).map(str::to_owned),
    );
    insert_optional_u64(
        &mut track.extensions,
        "audio_is_publish",
        audio.is_publish.as_ref().and_then(FlexibleInteger::as_u64),
    );
    track.extensions.insert(
        "artists".to_owned(),
        json!(map_artist_diagnostics(&metadata.authors)),
    );
    track.extensions.insert(
        "classifications".to_owned(),
        json!(map_classifications(&metadata.classifications)),
    );
    if let Some(album) = metadata.album_info.as_ref() {
        track.extensions.insert(
            "album_info".to_owned(),
            json!({
                "album_id": album.album_id.as_ref().and_then(FlexibleInteger::as_u64),
                "album_name": nonempty(&album.album_name),
                "publish_date": nonempty(&album.publish_date),
                "is_publish": album.is_publish.as_ref().and_then(FlexibleInteger::as_u64),
                "category": album.category.as_ref().and_then(FlexibleInteger::as_u64),
            }),
        );
    }
    track
        .extensions
        .insert("qualities".to_owned(), json!(qualities));
    track.extensions.insert(
        "detail_backend".to_owned(),
        json!([
            "openapi.kugou.com/kmr/v2/audio",
            "kmr.service.kugou.com/v1/audio/audio"
        ]),
    );
    Ok(track)
}

fn validate_detail_consistency(
    base: &TrackMetadataBase,
    album: Option<&TrackAlbumInfo>,
    audio: &AudioMetadata,
) -> Result<()> {
    if let Some(album) = album {
        let base_album_id = base.album_id.as_ref().and_then(FlexibleInteger::as_u64);
        let detail_album_id = album.album_id.as_ref().and_then(FlexibleInteger::as_u64);
        if base_album_id.is_some() && detail_album_id.is_some() && base_album_id != detail_album_id
        {
            return Err(kugou_upstream_error(
                "KuGou track detail returned inconsistent album identities",
            ));
        }
        if let (Some(base_name), Some(detail_name)) =
            (nonempty(&base.album_name), nonempty(&album.album_name))
            && base_name != detail_name
        {
            return Err(kugou_upstream_error(
                "KuGou track detail returned inconsistent album names",
            ));
        }
    }
    let base_group = base
        .audio_group_id
        .as_ref()
        .and_then(FlexibleInteger::as_u64);
    let audio_group = audio
        .audio_group_id
        .as_ref()
        .and_then(FlexibleInteger::as_u64);
    if base_group.is_some() && audio_group.is_some() && base_group != audio_group {
        return Err(kugou_upstream_error(
            "KuGou track detail returned inconsistent audio groups",
        ));
    }
    Ok(())
}

fn map_detail_artists(authors: &[TrackAuthor], fallback: &str) -> Vec<ArtistSummary> {
    let artists = authors
        .iter()
        .filter_map(|author| {
            let base = author.base.as_ref()?;
            let name = nonempty(&base.author_name)?;
            let resource_ref = base
                .author_id
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
        map_singers(&[], fallback)
    } else {
        artists
    }
}

fn map_detail_album(
    album: Option<&TrackAlbumInfo>,
    base: &TrackMetadataBase,
) -> Option<AlbumSummary> {
    let name = album
        .and_then(|album| nonempty(&album.album_name))
        .or_else(|| nonempty(&base.album_name))?;
    let id = album
        .and_then(|album| album.album_id.as_ref())
        .or(base.album_id.as_ref());
    Some(AlbumSummary {
        resource_ref: id
            .and_then(FlexibleInteger::as_resource_id)
            .and_then(|id| ResourceRef::new(Platform::Kugou, id).ok()),
        name: name.to_owned(),
        cover_url: album.and_then(|album| normalize_image_url(&album.cover)),
    })
}

fn map_artist_diagnostics(authors: &[TrackAuthor]) -> Vec<Value> {
    authors
        .iter()
        .filter_map(|author| {
            let base = author.base.as_ref()?;
            Some(json!({
                "author_id": base.author_id.as_ref().and_then(FlexibleInteger::as_u64),
                "author_name": nonempty(&base.author_name),
                "is_publish": base.is_publish.as_ref().and_then(FlexibleInteger::as_u64),
                "language": nonempty(&base.language),
                "avatar": normalize_image_url(&base.avatar),
                "identity": base.identity.as_ref().and_then(FlexibleInteger::as_u64),
                "type": base.author_type.as_ref().and_then(FlexibleInteger::as_u64),
                "country": nonempty(&base.country),
                "birthday": nonempty(&base.birthday),
                "relation_sisp": author.sisp.as_ref().and_then(FlexibleInteger::as_u64),
                "relation_identity": author.identity.as_ref().and_then(FlexibleInteger::as_u64),
            }))
        })
        .collect()
}

fn map_classifications(classifications: &[TrackClassification]) -> Vec<Value> {
    classifications
        .iter()
        .map(|class| {
            json!({
                "status": class.status.as_ref().and_then(FlexibleInteger::as_u64),
                "usage": class.usage.as_ref().and_then(FlexibleInteger::as_u64),
                "type": class.class_type.as_ref().and_then(FlexibleInteger::as_u64),
                "level": class.level.as_ref().and_then(FlexibleInteger::as_u64),
            })
        })
        .collect()
}

fn detail_quality_asset(
    hash: Option<&str>,
    size: Option<u64>,
    bitrate: Option<u64>,
    duration_ms: Option<u64>,
    format: &'static str,
) -> Option<Value> {
    let hash = hash?;
    Some(json!({
        "hash": hash,
        "size": size,
        "bitrate": bitrate,
        "duration_ms": duration_ms,
        "format": format,
    }))
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

fn unix_milliseconds_now() -> u64 {
    u64::try_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(u64::MAX)
}

fn android_signature(mid: &str, clienttime: u64, body: &[u8]) -> String {
    let mut digest = Md5::new();
    digest.update(ANDROID_SIGNATURE_SALT);
    digest.update(format!(
        "appid={ANDROID_APP_ID}clienttime={clienttime}clientver={ANDROID_CLIENT_VERSION}dfid=-mid={mid}uuid=-"
    ));
    digest.update(body);
    digest.update(ANDROID_SIGNATURE_SALT);
    hex::encode(digest.finalize())
}

fn md5_hex(value: impl AsRef<[u8]>) -> String {
    hex::encode(Md5::digest(value.as_ref()))
}

async fn read_bounded_response(
    mut response: reqwest::Response,
    operation: &str,
) -> Result<Vec<u8>> {
    let status = response.status();
    if !status.is_success() {
        return Err(kugou_http_error(status));
    }
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > MAX_API_RESPONSE_BYTES)
    {
        return Err(kugou_upstream_error(format!(
            "{operation} response exceeded the size limit"
        )));
    }
    let max_size = usize::try_from(MAX_API_RESPONSE_BYTES).unwrap_or(usize::MAX);
    let initial_capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or_default()
        .min(max_size);
    let mut bytes = Vec::with_capacity(initial_capacity);
    while let Some(chunk) = response.chunk().await.map_err(kugou_network_error)? {
        if bytes.len().saturating_add(chunk.len()) > max_size {
            return Err(kugou_upstream_error(format!(
                "{operation} response exceeded the size limit"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
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

    #[test]
    fn android_signature_matches_the_reference_protocol() {
        let body =
            br#"{"data":[{"entity_id":32100650}],"fields":"base,album_info,authors.base,class"}"#;
        assert_eq!(
            android_signature("12345678901234567890", 1_722_222_222, body),
            "da07c52ea6737d8c77b29f75628d55b2"
        );
    }

    #[test]
    fn detail_resolves_separate_identities_and_preserves_every_quality_tier() {
        let metadata = r#"{
          "msg":"","status":1,"error_code":0,
          "data":[{
            "__status":1,
            "authors":[{
              "sisp":1,"identity":1,
              "base":{
                "author_id":3520,"author_name":"周杰伦","is_publish":1,
                "language":"华语","avatar":"http://singerimg.kugou.com/head/{size}/jay.jpg",
                "identity":1135,"type":1,"country":"中国","birthday":"1979-01-18"
              }
            }],
            "album_info":{
              "album_id":966846,"album_name":"叶惠美","publish_date":"2003-07-31",
              "is_publish":1,"cover":"http://imge.kugou.com/stdmusic/{size}/cover.jpg",
              "category":1
            },
            "class":[{"status":0,"usage":0,"type":3,"level":1}],
            "base":{
              "album_id":966846,"songname":"晴天","author_name":"周杰伦",
              "album_name":"叶惠美","version":1,"language":"国语",
              "publish_date":"2003-07-31","wide_audio_id":20505418,
              "is_publish":1,"provider":0,"big_pack_id":6540815,"final_id":0,
              "audio_id":20505418,"similar_audio_id":20505418,
              "album_audio_id":32100650,"audio_group_id":689
            }
          }]
        }"#;
        let audio = r#"{
          "status":1,"error_code":0,"errcode":0,"errmsg":"",
          "data":[{
            "timelength":"269000","audio_group_id":"689",
            "audio_name":"周杰伦 - 晴天","audio_id":"20505418","is_publish":"1",
            "language":"国语","hash":"BASE","hash_128":"BASE",
            "filesize":"4317292","filesize_128":"4317292","bitrate":"128",
            "timelength_128":"269000","hash_320":"HIGH","filesize_320":"10792943",
            "timelength_320":"269000","hash_flac":"FLAC","filesize_flac":"31729524",
            "bitrate_flac":"940","timelength_flac":"269000","hash_ape":"APE",
            "filesize_ape":"32000000","bitrate_ape":"950","timelength_ape":"269000",
            "hash_high":"HIRES","filesize_high":"55296047","bitrate_high":"1639",
            "timelength_high":"269000","hash_super":"MASTER",
            "filesize_super":"70000000","bitrate_super":"2100",
            "timelength_super":"269000"
          }]
        }"#;
        let metadata = parse_track_metadata(metadata.as_bytes(), 32_100_650)
            .expect("parse track metadata response");
        let audio = parse_audio_metadata(audio.as_bytes(), "20505418")
            .expect("parse audio metadata response");
        let track = map_track_detail(32_100_650, metadata, audio).expect("map detail");

        assert_eq!(track.resource_ref.to_string(), "kugou:32100650");
        assert_eq!(track.name, "晴天");
        assert_eq!(track.duration_ms, Some(269_000));
        assert_eq!(
            track
                .album
                .as_ref()
                .and_then(|album| album.cover_url.as_deref()),
            Some("https://imge.kugou.com/stdmusic/400/cover.jpg")
        );
        assert_eq!(
            track.available_qualities,
            [
                Quality::Standard,
                Quality::High,
                Quality::Lossless,
                Quality::Hires,
                Quality::Master
            ]
        );
        assert_eq!(track.extensions["album_audio_id"], "32100650");
        assert_eq!(track.extensions["audio_id"], "20505418");
        assert_eq!(track.extensions["qualities"]["lossless"]["hash"], "FLAC");
        assert_eq!(track.extensions["qualities"]["lossless_ape"]["hash"], "APE");
        assert!(track.playable.is_none());
    }

    #[test]
    fn detail_rejects_mismatched_album_audio_and_audio_identities() {
        let wrong_album_audio = r#"{
          "status":1,"error_code":0,
          "data":[{"__status":1,"base":{"album_audio_id":1,"audio_id":20505418}}]
        }"#;
        assert!(parse_track_metadata(wrong_album_audio.as_bytes(), 32_100_650).is_err());

        let wrong_audio = r#"{
          "status":1,"error_code":0,"errcode":0,
          "data":[{"audio_id":"32100650","audio_name":"wrong","hash":"HASH"}]
        }"#;
        assert!(parse_audio_metadata(wrong_audio.as_bytes(), "20505418").is_err());
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

    #[tokio::test]
    #[ignore = "requires live KuGou network access"]
    async fn live_public_track_detail_preserves_album_audio_identity() {
        let client = KugouClient::new(&KugouConfig::default()).expect("create client");
        let track = client
            .track_detail(32_100_650)
            .await
            .expect("live KuGou track detail");
        assert_eq!(track.resource_ref.to_string(), "kugou:32100650");
        assert_eq!(track.extensions["audio_id"], "20505418");
        assert!(track.available_qualities.contains(&Quality::Standard));
    }
}
