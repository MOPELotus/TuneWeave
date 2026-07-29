use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    io::Read,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use flate2::read::ZlibDecoder;
use md5::{Digest, Md5};
use reqwest::{
    Client, Proxy, StatusCode,
    header::{CONTENT_LENGTH, CONTENT_TYPE, REFERER},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tuneweave_core::{
    AlbumSummary, ArtistSummary, ErrorCode, Extensions, LyricContributor, Lyrics, MediaDownload,
    MediaStream, Platform, Quality, ResourceRef, Result, StreamRequest, StreamVariant, Track,
    TrialWindow, TuneWeaveError,
};
use url::Url;

const SEARCH_ENDPOINT: &str = "https://songsearch.kugou.com/song_search_v2";
const ANDROID_GATEWAY: &str = "https://gateway.kugou.com";
const LYRIC_SEARCH_ENDPOINT: &str = "https://lyrics.kugou.com/v1/search";
const LYRIC_DOWNLOAD_ENDPOINT: &str = "https://lyrics.kugou.com/download";
const TRACKER_ENDPOINT: &str = "https://gateway.kugou.com/v5/url";
const WEB_REFERER: &str = "https://www.kugou.com/";
const WEB_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
                             (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";
const ANDROID_USER_AGENT: &str = "Android15-1070-11083-46-0-DiscoveryDRADProtocol-wifi";
const ANDROID_SIGNATURE_SALT: &str = "OIlwieks28dk2k092lksi2UIkp";
const TRACKER_KEY_SALT: &str = "57ae12eb6890223e355ccfcb74edf70d";
const ANDROID_APP_ID: u16 = 1005;
const ANDROID_CLIENT_VERSION: u32 = 20489;
const MAX_API_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_LYRIC_TEXT_BYTES: usize = 4 * 1024 * 1024;
const KRC_XOR_KEY: [u8; 16] = [
    64, 71, 97, 119, 94, 50, 116, 71, 81, 54, 49, 45, 206, 210, 110, 105,
];

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
    Privilege,
}

impl AndroidEndpoint {
    const fn path(self) -> &'static str {
        match self {
            Self::TrackMetadata => "/kmr/v2/audio",
            Self::AudioMetadata => "/v1/audio/audio",
            Self::Privilege => "/v2/get_res_privilege/lite",
        }
    }

    const fn router(self) -> &'static str {
        match self {
            Self::TrackMetadata => "openapi.kugou.com",
            Self::AudioMetadata => "kmr.service.kugou.com",
            Self::Privilege => "media.store.kugou.com",
        }
    }

    const fn kg_tid(self) -> Option<&'static str> {
        match self {
            Self::TrackMetadata => Some("238"),
            Self::AudioMetadata | Self::Privilege => None,
        }
    }

    const fn operation(self) -> &'static str {
        match self {
            Self::TrackMetadata => "KuGou track detail",
            Self::AudioMetadata => "KuGou audio metadata",
            Self::Privilege => "KuGou media privilege",
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

#[derive(Serialize)]
struct LyricSearchQuery<'a> {
    album_audio_id: u64,
    appid: u16,
    clientver: u32,
    duration: u64,
    hash: &'a str,
    keyword: &'a str,
    lrctxt: u8,
    man: &'static str,
    signature: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct LyricSearchEnvelope {
    status: i64,
    info: String,
    errcode: i64,
    errmsg: String,
    keyword: String,
    proposal: String,
    has_complete_right: Option<FlexibleInteger>,
    expire: Option<FlexibleInteger>,
    candidates: Vec<LyricCandidate>,
    ugccandidates: Vec<Value>,
    ai_candidates: Vec<Value>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct LyricCandidate {
    id: String,
    product_from: String,
    accesskey: String,
    can_score: bool,
    singer: String,
    song: String,
    duration: Option<FlexibleInteger>,
    uid: String,
    nickname: String,
    origiuid: String,
    transuid: String,
    sounduid: String,
    originame: String,
    transname: String,
    soundname: String,
    language: String,
    krctype: Option<FlexibleInteger>,
    hitlayer: Option<FlexibleInteger>,
    hitcasemask: Option<FlexibleInteger>,
    adjust: Option<FlexibleInteger>,
    score: Option<FlexibleInteger>,
    contenttype: Option<FlexibleInteger>,
    content_format: Option<FlexibleInteger>,
    download_id: String,
}

#[derive(Serialize)]
struct LyricDownloadQuery<'a> {
    accesskey: &'a str,
    appid: u16,
    charset: &'static str,
    client: &'static str,
    clienttime: u64,
    clientver: u32,
    dfid: &'static str,
    fmt: &'static str,
    id: &'a str,
    mid: &'a str,
    uuid: &'static str,
    ver: u8,
    signature: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct LyricDownloadEnvelope {
    status: i64,
    info: String,
    error_code: i64,
    fmt: String,
    contenttype: Option<FlexibleInteger>,
    #[serde(rename = "_source")]
    source: String,
    charset: String,
    content: String,
    id: String,
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum RequestedLyricFormat {
    Krc,
    Lrc,
}

impl RequestedLyricFormat {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Krc => "krc",
            Self::Lrc => "lrc",
        }
    }
}

struct DecodedLyric {
    text: String,
    format: &'static str,
    content_type: Option<u64>,
    source: Option<String>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct KrcLanguagePayload {
    version: Option<FlexibleInteger>,
    content: Vec<KrcLanguageSection>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct KrcLanguageSection {
    #[serde(rename = "type")]
    section_type: Option<FlexibleInteger>,
    #[serde(rename = "lyricContent")]
    lyric_content: Vec<Vec<String>>,
}

struct EmbeddedLyricLanguages {
    translated: Option<String>,
    romanized: Option<String>,
    extensions: Vec<Value>,
    version: Option<u64>,
}

#[derive(Serialize)]
struct PrivilegeRequest<'a> {
    appid: u16,
    area_code: u8,
    behavior: &'static str,
    clientver: u32,
    need_hash_offset: u8,
    relate: u8,
    support_verify: u8,
    resource: [PrivilegeIdentity<'a>; 1],
    qualities: [&'static str; 9],
}

#[derive(Serialize)]
struct PrivilegeIdentity<'a> {
    #[serde(rename = "type")]
    resource_type: &'static str,
    page_id: u8,
    hash: &'a str,
    album_id: u64,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct PrivilegeEnvelope {
    status: i64,
    error_code: i64,
    message: String,
    data: Vec<PrivilegeResource>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct PrivilegeResource {
    #[serde(rename = "type")]
    resource_type: String,
    id: Option<FlexibleInteger>,
    album_id: Option<FlexibleInteger>,
    album_audio_id: Option<FlexibleInteger>,
    hash: String,
    name: String,
    level: Option<FlexibleInteger>,
    quality: String,
    expire: Option<FlexibleInteger>,
    publish: Option<FlexibleInteger>,
    is_publish: Option<FlexibleInteger>,
    privilege: Option<FlexibleInteger>,
    status: Option<FlexibleInteger>,
    fail_process: Option<FlexibleInteger>,
    pay_type: Option<FlexibleInteger>,
    price: Option<FlexibleInteger>,
    info: PrivilegeInfo,
    popup: PrivilegePopup,
    trans_param: PrivilegeTransParam,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct PrivilegeInfo {
    duration: Option<FlexibleInteger>,
    filesize: Option<FlexibleInteger>,
    bitrate: Option<FlexibleInteger>,
    extname: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct PrivilegePopup {
    title: String,
    content: String,
    btn_name: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct PrivilegeTransParam {
    hash_offset: Option<HashOffset>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct HashOffset {
    start_byte: Option<FlexibleInteger>,
    end_byte: Option<FlexibleInteger>,
    start_ms: Option<FlexibleInteger>,
    end_ms: Option<FlexibleInteger>,
    offset_hash: String,
    clip_hash: String,
    file_type: Option<FlexibleInteger>,
}

#[derive(Serialize)]
struct TrackerQuery<'a> {
    #[serde(rename = "IsFreePart")]
    is_free_part: u8,
    album_audio_id: u64,
    album_id: u64,
    appid: u16,
    area_code: u8,
    behavior: &'static str,
    #[serde(rename = "cdnBackup")]
    cdn_backup: u8,
    clienttime: u64,
    clientver: u32,
    cmd: u8,
    dfid: &'static str,
    hash: &'a str,
    key: String,
    mid: &'a str,
    module: &'static str,
    page_id: u64,
    pid: u8,
    pidversion: u32,
    ppage_id: &'static str,
    quality: &'a str,
    ssa_flag: &'static str,
    uuid: &'static str,
    version: u32,
    signature: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct TrackerEnvelope {
    status: i64,
    q: Option<FlexibleInteger>,
    url: Vec<String>,
    #[serde(rename = "backupUrl")]
    backup_url: Vec<String>,
    #[serde(rename = "timeLength")]
    time_length: Option<FlexibleInteger>,
    #[serde(rename = "fileSize")]
    file_size: Option<FlexibleInteger>,
    #[serde(rename = "bitRate")]
    bit_rate: Option<FlexibleInteger>,
    #[serde(rename = "fileName")]
    file_name: String,
    #[serde(rename = "extName")]
    extension: String,
    hash: String,
    priv_status: Option<FlexibleInteger>,
    fail_process: Vec<String>,
    hash_offset: Option<HashOffset>,
}

struct SelectedMediaSpec {
    requested_quality: Quality,
    actual_quality: Quality,
    tracker_quality: &'static str,
    hash: String,
    size: Option<u64>,
    bitrate: Option<u64>,
    duration_ms: Option<u64>,
    format: Option<String>,
}

struct MediaResolution {
    spec: SelectedMediaSpec,
    url: Option<String>,
    backup_urls: Vec<String>,
    trial: Option<TrialWindow>,
    privilege: PrivilegeResource,
    tracker: TrackerEnvelope,
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

    pub(crate) async fn lyrics(&self, album_audio_id: u64) -> Result<Lyrics> {
        let track = self.track_detail(album_audio_id).await?;
        let hash = track
            .extensions
            .get("hash")
            .and_then(Value::as_str)
            .ok_or_else(|| kugou_upstream_error("KuGou track detail omitted the lyric hash"))?;
        let duration = track
            .duration_ms
            .ok_or_else(|| kugou_upstream_error("KuGou track detail omitted lyric duration"))?;
        let keyword = lyric_search_keyword(&track);
        let (candidate, search_diagnostics) = self
            .search_lyric_candidate(album_audio_id, hash, duration, &keyword)
            .await?;

        let krc = self
            .download_lyric(&candidate, RequestedLyricFormat::Krc)
            .await;
        let lrc = self
            .download_lyric(&candidate, RequestedLyricFormat::Lrc)
            .await;
        map_lyrics(album_audio_id, candidate, search_diagnostics, krc, lrc)
    }

    pub(crate) async fn stream(
        &self,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<MediaStream> {
        let resolution = self.resolve_media(track, request, true).await?;
        let url = resolution.url.clone().ok_or_else(|| {
            media_permission_error(request, &resolution.privilege, &resolution.tracker)
        })?;
        Ok(MediaStream {
            url,
            backup_urls: resolution.backup_urls,
            headers: BTreeMap::new(),
            expires_at: None,
            format: resolution.spec.format.clone(),
            codec: resolution.spec.format.clone(),
            bitrate: resolution
                .tracker
                .bit_rate
                .as_ref()
                .and_then(FlexibleInteger::as_u64)
                .or(resolution.spec.bitrate),
            size: resolution
                .trial
                .as_ref()
                .and_then(|_| trial_size_from_offsets(resolution.tracker.hash_offset.as_ref()))
                .or_else(|| {
                    resolution
                        .tracker
                        .file_size
                        .as_ref()
                        .and_then(FlexibleInteger::as_u64)
                })
                .or(resolution.spec.size),
            duration_ms: resolution
                .tracker
                .time_length
                .as_ref()
                .and_then(FlexibleInteger::as_u64)
                .and_then(|seconds| seconds.checked_mul(1_000))
                .or(resolution.spec.duration_ms)
                .or(track.duration_ms),
            requested_quality: resolution.spec.requested_quality,
            actual_quality: resolution.spec.actual_quality,
            trial: resolution.trial,
            origin_track: Some(track.resource_ref.clone()),
            resolved_track: track.resource_ref.clone(),
            resolved_platform: Platform::Kugou,
            match_score: Some(1.0),
            attempts: Vec::new(),
        })
    }

    pub(crate) async fn download(
        &self,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<MediaDownload> {
        let resolution = self.resolve_media(track, request, false).await?;
        let available = resolution.url.is_some();
        let message = (!available)
            .then(|| {
                nonempty(&resolution.privilege.popup.title)
                    .or_else(|| nonempty(&resolution.privilege.popup.content))
                    .map(str::to_owned)
            })
            .flatten()
            .or_else(|| {
                (!available).then(|| "KuGou did not return a full downloadable file".to_owned())
            });
        let mut extensions = Extensions::new();
        extensions.insert(
            "privilege".to_owned(),
            privilege_diagnostics(&resolution.privilege),
        );
        extensions.insert(
            "tracker".to_owned(),
            tracker_diagnostics(&resolution.tracker),
        );
        extensions.insert(
            "trial".to_owned(),
            json!(trial_from_offsets(
                resolution.tracker.hash_offset.as_ref().or(resolution
                    .privilege
                    .trans_param
                    .hash_offset
                    .as_ref())
            )),
        );
        Ok(MediaDownload {
            track_ref: track.resource_ref.clone(),
            platform: Platform::Kugou,
            available,
            url: resolution.url,
            headers: BTreeMap::new(),
            expires_at: None,
            format: resolution.spec.format.clone(),
            codec: resolution.spec.format,
            bitrate: resolution.spec.bitrate,
            size: resolution.spec.size,
            duration_ms: resolution.spec.duration_ms.or(track.duration_ms),
            requested_quality: resolution.spec.requested_quality,
            actual_quality: resolution.spec.actual_quality,
            platform_code: Some(resolution.tracker.status),
            fee: resolution
                .privilege
                .pay_type
                .as_ref()
                .and_then(FlexibleInteger::as_u64)
                .and_then(|value| i64::try_from(value).ok()),
            message,
            extensions,
        })
    }

    async fn resolve_media(
        &self,
        track: &Track,
        request: &StreamRequest,
        allow_trial: bool,
    ) -> Result<MediaResolution> {
        let album_audio_id = canonical_track_id(track)?;
        let album_id = canonical_album_id(track)?;
        let spec = select_media_spec(track, request)?;
        let privilege = self
            .media_privilege(album_audio_id, album_id, &spec.hash)
            .await?;
        let mut tracker = self.tracker(album_audio_id, album_id, &spec, false).await?;
        let mut used_trial = false;
        if tracker.status != 1 && allow_trial {
            tracker = self.tracker(album_audio_id, album_id, &spec, true).await?;
            used_trial = tracker.status == 1;
        }
        let (url, backup_urls) = map_tracker_urls(&tracker)?;
        if tracker.status == 1 && url.is_none() {
            return Err(kugou_upstream_error(
                "KuGou tracker reported success without a media URL",
            ));
        }
        let trial = used_trial
            .then(|| {
                trial_from_offsets(
                    tracker
                        .hash_offset
                        .as_ref()
                        .or(privilege.trans_param.hash_offset.as_ref()),
                )
            })
            .flatten();
        Ok(MediaResolution {
            spec,
            url,
            backup_urls,
            trial,
            privilege,
            tracker,
        })
    }

    async fn media_privilege(
        &self,
        album_audio_id: u64,
        album_id: u64,
        hash: &str,
    ) -> Result<PrivilegeResource> {
        let request = PrivilegeRequest {
            appid: ANDROID_APP_ID,
            area_code: 1,
            behavior: "play",
            clientver: ANDROID_CLIENT_VERSION,
            need_hash_offset: 1,
            relate: 1,
            support_verify: 1,
            resource: [PrivilegeIdentity {
                resource_type: "audio",
                page_id: 0,
                hash,
                album_id,
            }],
            qualities: [
                "128",
                "320",
                "flac",
                "high",
                "viper_atmos",
                "viper_tape",
                "viper_clear",
                "super",
                "multitrack",
            ],
        };
        let bytes = self
            .post_android(AndroidEndpoint::Privilege, &request)
            .await?;
        parse_privilege_response(&bytes, album_audio_id, album_id, hash)
    }

    async fn tracker(
        &self,
        album_audio_id: u64,
        album_id: u64,
        spec: &SelectedMediaSpec,
        free_part: bool,
    ) -> Result<TrackerEnvelope> {
        let clienttime = unix_seconds_now();
        let hash = spec.hash.to_ascii_lowercase();
        let key = md5_hex(format!(
            "{hash}{TRACKER_KEY_SALT}{ANDROID_APP_ID}{}0",
            self.mid
        ));
        let parameters = BTreeMap::from([
            ("IsFreePart", u8::from(free_part).to_string()),
            ("album_audio_id", album_audio_id.to_string()),
            ("album_id", album_id.to_string()),
            ("appid", ANDROID_APP_ID.to_string()),
            ("area_code", "1".to_owned()),
            ("behavior", "play".to_owned()),
            ("cdnBackup", "1".to_owned()),
            ("clienttime", clienttime.to_string()),
            ("clientver", "11430".to_owned()),
            ("cmd", "26".to_owned()),
            ("dfid", "-".to_owned()),
            ("hash", hash.clone()),
            ("key", key.clone()),
            ("mid", self.mid.clone()),
            ("module", String::new()),
            ("page_id", "151369488".to_owned()),
            ("pid", "2".to_owned()),
            ("pidversion", "3001".to_owned()),
            ("ppage_id", "463467626,350369493,788954147".to_owned()),
            ("quality", spec.tracker_quality.to_owned()),
            ("ssa_flag", "is_fromtrack".to_owned()),
            ("uuid", "-".to_owned()),
            ("version", "11430".to_owned()),
        ]);
        let query = TrackerQuery {
            is_free_part: u8::from(free_part),
            album_audio_id,
            album_id,
            appid: ANDROID_APP_ID,
            area_code: 1,
            behavior: "play",
            cdn_backup: 1,
            clienttime,
            clientver: 11430,
            cmd: 26,
            dfid: "-",
            hash: &hash,
            key,
            mid: &self.mid,
            module: "",
            page_id: 151_369_488,
            pid: 2,
            pidversion: 3001,
            ppage_id: "463467626,350369493,788954147",
            quality: spec.tracker_quality,
            ssa_flag: "is_fromtrack",
            uuid: "-",
            version: 11430,
            signature: android_signature_for_parameters(&parameters, &[]),
        };
        let response = self
            .android_get(TRACKER_ENDPOINT, clienttime)
            .header("x-router", "trackercdn.kugou.com")
            .query(&query)
            .send()
            .await
            .map_err(kugou_network_error)?;
        if response.headers().contains_key("ssa-code") {
            return Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "KuGou tracker requires additional verification",
            )
            .with_platform(Platform::Kugou)
            .with_details(json!({ "verification_required": true })));
        }
        let bytes = read_bounded_response(response, "KuGou media tracker").await?;
        parse_tracker_response(&bytes, &spec.hash)
    }

    async fn search_lyric_candidate(
        &self,
        album_audio_id: u64,
        hash: &str,
        duration: u64,
        keyword: &str,
    ) -> Result<(LyricCandidate, Value)> {
        let parameters = BTreeMap::from([
            ("album_audio_id", album_audio_id.to_string()),
            ("appid", ANDROID_APP_ID.to_string()),
            ("clientver", ANDROID_CLIENT_VERSION.to_string()),
            ("duration", duration.to_string()),
            ("hash", hash.to_owned()),
            ("keyword", keyword.to_owned()),
            ("lrctxt", "1".to_owned()),
            ("man", "no".to_owned()),
        ]);
        let query = LyricSearchQuery {
            album_audio_id,
            appid: ANDROID_APP_ID,
            clientver: ANDROID_CLIENT_VERSION,
            duration,
            hash,
            keyword,
            lrctxt: 1,
            man: "no",
            signature: android_signature_for_parameters(&parameters, &[]),
        };
        let response = self
            .android_get(LYRIC_SEARCH_ENDPOINT, unix_seconds_now())
            .query(&query)
            .send()
            .await
            .map_err(kugou_network_error)?;
        let bytes = read_bounded_response(response, "KuGou lyric search").await?;
        parse_lyric_search_response(&bytes)
    }

    async fn download_lyric(
        &self,
        candidate: &LyricCandidate,
        format: RequestedLyricFormat,
    ) -> Result<DecodedLyric> {
        let clienttime = unix_seconds_now();
        let parameters = BTreeMap::from([
            ("accesskey", candidate.accesskey.clone()),
            ("appid", ANDROID_APP_ID.to_string()),
            ("charset", "utf8".to_owned()),
            ("client", "android".to_owned()),
            ("clienttime", clienttime.to_string()),
            ("clientver", ANDROID_CLIENT_VERSION.to_string()),
            ("dfid", "-".to_owned()),
            ("fmt", format.as_str().to_owned()),
            ("id", candidate.id.clone()),
            ("mid", self.mid.clone()),
            ("uuid", "-".to_owned()),
            ("ver", "1".to_owned()),
        ]);
        let query = LyricDownloadQuery {
            accesskey: &candidate.accesskey,
            appid: ANDROID_APP_ID,
            charset: "utf8",
            client: "android",
            clienttime,
            clientver: ANDROID_CLIENT_VERSION,
            dfid: "-",
            fmt: format.as_str(),
            id: &candidate.id,
            mid: &self.mid,
            uuid: "-",
            ver: 1,
            signature: android_signature_for_parameters(&parameters, &[]),
        };
        let response = self
            .android_get(LYRIC_DOWNLOAD_ENDPOINT, clienttime)
            .query(&query)
            .send()
            .await
            .map_err(kugou_network_error)?;
        let bytes = read_bounded_response(response, "KuGou lyric download").await?;
        parse_lyric_download_response(&bytes, candidate, format)
    }

    fn android_get(&self, endpoint: &'static str, clienttime: u64) -> reqwest::RequestBuilder {
        self.http
            .get(endpoint)
            .header("dfid", "-")
            .header("clienttime", clienttime)
            .header("mid", &self.mid)
            .header("kg-rc", "1")
            .header("kg-thash", "5d816a0")
            .header("kg-rec", "1")
            .header("kg-rf", "B9EDA08A64250DEFFBCADDEE00F8F25F")
            .header("user-agent", ANDROID_USER_AGENT)
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
        read_bounded_response(response, endpoint.operation()).await
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

fn canonical_track_id(track: &Track) -> Result<u64> {
    if track.platform != Platform::Kugou || track.resource_ref.platform() != Platform::Kugou {
        return Err(kugou_invalid_media_request(
            "KuGou media resolution requires a KuGou track",
        ));
    }
    canonical_positive_u64(track.resource_ref.id()).ok_or_else(|| {
        kugou_invalid_media_request("KuGou media resolution requires a canonical album_audio_id")
    })
}

fn canonical_album_id(track: &Track) -> Result<u64> {
    let Some(reference) = track
        .album
        .as_ref()
        .and_then(|album| album.resource_ref.as_ref())
    else {
        return Ok(0);
    };
    if reference.platform() != Platform::Kugou {
        return Err(kugou_upstream_error(
            "KuGou track metadata contained a foreign album identity",
        ));
    }
    canonical_positive_u64(reference.id()).ok_or_else(|| {
        kugou_upstream_error("KuGou track metadata contained an invalid album identity")
    })
}

fn select_media_spec(track: &Track, request: &StreamRequest) -> Result<SelectedMediaSpec> {
    if request.variant != StreamVariant::Default {
        return Err(kugou_invalid_media_request(
            "KuGou public media only supports the default stream variant",
        ));
    }
    if request.account.is_some() {
        return Err(kugou_invalid_media_request(
            "KuGou public media does not accept an account",
        ));
    }
    if request.immersive_type.is_some() {
        return Err(kugou_invalid_media_request(
            "KuGou public media does not accept immersive_type",
        ));
    }
    let requested_quality = request.quality;
    let target = if let Some(bitrate) = request.bitrate {
        match bitrate {
            1..=128_000 => Quality::Standard,
            128_001..=320_000 => Quality::High,
            _ => {
                return Err(kugou_invalid_media_request(
                    "KuGou public media bitrate must be between 1 and 320000",
                ));
            }
        }
    } else {
        match request.quality {
            Quality::Auto | Quality::Low | Quality::Standard => Quality::Standard,
            Quality::Higher | Quality::High => Quality::High,
            Quality::Lossless => Quality::Lossless,
            Quality::Hires => Quality::Hires,
            Quality::Master => Quality::Master,
            Quality::Surround | Quality::Spatial | Quality::Dolby => {
                return Err(kugou_invalid_media_request(
                    "KuGou public media does not yet expose immersive quality families",
                ));
            }
        }
    };
    let fallbacks: &[(&str, Quality, &str, &str)] = match target {
        Quality::Master => &[
            ("master", Quality::Master, "super", "flac"),
            ("hires", Quality::Hires, "high", "flac"),
            ("lossless", Quality::Lossless, "flac", "flac"),
            ("high", Quality::High, "320", "mp3"),
            ("standard", Quality::Standard, "128", "mp3"),
        ],
        Quality::Hires => &[
            ("hires", Quality::Hires, "high", "flac"),
            ("lossless", Quality::Lossless, "flac", "flac"),
            ("high", Quality::High, "320", "mp3"),
            ("standard", Quality::Standard, "128", "mp3"),
        ],
        Quality::Lossless => &[
            ("lossless", Quality::Lossless, "flac", "flac"),
            ("high", Quality::High, "320", "mp3"),
            ("standard", Quality::Standard, "128", "mp3"),
        ],
        Quality::High => &[
            ("high", Quality::High, "320", "mp3"),
            ("standard", Quality::Standard, "128", "mp3"),
        ],
        _ => &[("standard", Quality::Standard, "128", "mp3")],
    };
    let assets = track
        .extensions
        .get("qualities")
        .and_then(Value::as_object)
        .ok_or_else(|| kugou_upstream_error("KuGou track omitted media quality metadata"))?;
    for (key, actual_quality, tracker_quality, fallback_format) in fallbacks {
        let Some(asset) = assets.get(*key).and_then(Value::as_object) else {
            continue;
        };
        let Some(hash) = asset.get("hash").and_then(Value::as_str).map(str::trim) else {
            continue;
        };
        if hash.len() != 32 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(kugou_upstream_error(
                "KuGou track contained an invalid media hash",
            ));
        }
        return Ok(SelectedMediaSpec {
            requested_quality,
            actual_quality: *actual_quality,
            tracker_quality,
            hash: hash.to_ascii_uppercase(),
            size: asset.get("size").and_then(Value::as_u64),
            bitrate: asset.get("bitrate").and_then(Value::as_u64).map(|value| {
                if value <= 10_000 {
                    value.saturating_mul(1_000)
                } else {
                    value
                }
            }),
            duration_ms: asset.get("duration_ms").and_then(Value::as_u64),
            format: asset
                .get("format")
                .and_then(Value::as_str)
                .map(str::to_owned)
                .or_else(|| Some((*fallback_format).to_owned())),
        });
    }
    Err(TuneWeaveError::new(
        ErrorCode::ResourceNotFound,
        "KuGou track has no media asset for the requested quality",
    )
    .with_platform(Platform::Kugou))
}

fn parse_privilege_response(
    bytes: &[u8],
    album_audio_id: u64,
    album_id: u64,
    hash: &str,
) -> Result<PrivilegeResource> {
    let envelope: PrivilegeEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| kugou_upstream_error("KuGou media privilege returned malformed JSON"))?;
    if envelope.status != 1 || envelope.error_code != 0 {
        return Err(
            kugou_upstream_error("KuGou media privilege rejected the request").with_details(
                json!({
                    "platform_code": envelope.error_code,
                    "platform_message": safe_upstream_message(&envelope.message),
                }),
            ),
        );
    }
    let [resource] = <[PrivilegeResource; 1]>::try_from(envelope.data).map_err(|_| {
        kugou_upstream_error("KuGou media privilege returned an unexpected result count")
    })?;
    let returned_album_audio_id = resource
        .album_audio_id
        .as_ref()
        .and_then(FlexibleInteger::as_u64);
    let returned_album_id = resource.album_id.as_ref().and_then(FlexibleInteger::as_u64);
    if resource.resource_type != "audio"
        || returned_album_audio_id != Some(album_audio_id)
        || returned_album_id != Some(album_id)
        || !resource.hash.eq_ignore_ascii_case(hash)
    {
        return Err(kugou_upstream_error(
            "KuGou media privilege returned mismatched media identities",
        ));
    }
    Ok(resource)
}

fn parse_tracker_response(bytes: &[u8], requested_hash: &str) -> Result<TrackerEnvelope> {
    let tracker: TrackerEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| kugou_upstream_error("KuGou media tracker returned malformed JSON"))?;
    if !matches!(tracker.status, 1 | 2) {
        return Err(
            kugou_upstream_error("KuGou media tracker rejected the request").with_details(json!({
                "platform_code": tracker.status,
            })),
        );
    }
    if tracker.status == 1 && !tracker.hash.eq_ignore_ascii_case(requested_hash) {
        return Err(kugou_upstream_error(
            "KuGou media tracker returned a mismatched media hash",
        ));
    }
    if tracker.status != 1 && (!tracker.url.is_empty() || !tracker.backup_url.is_empty()) {
        return Err(kugou_upstream_error(
            "KuGou media tracker returned URLs for a rejected request",
        ));
    }
    Ok(tracker)
}

fn map_tracker_urls(tracker: &TrackerEnvelope) -> Result<(Option<String>, Vec<String>)> {
    if tracker.status != 1 {
        return Ok((None, Vec::new()));
    }
    let mut seen = BTreeSet::new();
    let mut urls = Vec::new();
    for url in tracker.url.iter().chain(&tracker.backup_url) {
        let url = normalize_media_url(url)?;
        if seen.insert(url.clone()) {
            urls.push(url);
        }
    }
    let primary = urls.first().cloned();
    let backups = urls.into_iter().skip(1).collect();
    Ok((primary, backups))
}

fn normalize_media_url(value: &str) -> Result<String> {
    let value = nonempty(value)
        .ok_or_else(|| kugou_upstream_error("KuGou media tracker returned an empty URL"))?;
    let value = if let Some(rest) = value.strip_prefix("http://") {
        format!("https://{rest}")
    } else {
        value.to_owned()
    };
    let url = Url::parse(&value)
        .map_err(|_| kugou_upstream_error("KuGou media tracker returned an invalid URL"))?;
    let host = url
        .host_str()
        .ok_or_else(|| kugou_upstream_error("KuGou media tracker URL omitted a host"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
        || !(host == "kugou.com" || host.ends_with(".kugou.com"))
    {
        return Err(kugou_upstream_error(
            "KuGou media tracker returned an untrusted URL",
        ));
    }
    Ok(value)
}

fn trial_from_offsets(offset: Option<&HashOffset>) -> Option<TrialWindow> {
    let offset = offset?;
    let start_ms = offset.start_ms.as_ref().and_then(FlexibleInteger::as_u64)?;
    let end_ms = offset.end_ms.as_ref().and_then(FlexibleInteger::as_u64)?;
    (end_ms > start_ms).then_some(TrialWindow { start_ms, end_ms })
}

fn trial_size_from_offsets(offset: Option<&HashOffset>) -> Option<u64> {
    let offset = offset?;
    let start = offset
        .start_byte
        .as_ref()
        .and_then(FlexibleInteger::as_u64)?;
    let end = offset.end_byte.as_ref().and_then(FlexibleInteger::as_u64)?;
    (end >= start).then(|| end.saturating_sub(start).saturating_add(1))
}

fn privilege_diagnostics(resource: &PrivilegeResource) -> Value {
    json!({
        "audio_id": resource.id.as_ref().and_then(FlexibleInteger::as_u64),
        "album_audio_id": resource.album_audio_id.as_ref().and_then(FlexibleInteger::as_u64),
        "name": nonempty(&resource.name),
        "level": resource.level.as_ref().and_then(FlexibleInteger::as_u64),
        "quality": nonempty(&resource.quality),
        "expires_in_seconds": resource.expire.as_ref().and_then(FlexibleInteger::as_u64),
        "publish": resource.publish.as_ref().and_then(FlexibleInteger::as_u64),
        "is_publish": resource.is_publish.as_ref().and_then(FlexibleInteger::as_u64),
        "privilege": resource.privilege.as_ref().and_then(FlexibleInteger::as_u64),
        "status": resource.status.as_ref().and_then(FlexibleInteger::as_u64),
        "fail_process": resource.fail_process.as_ref().and_then(FlexibleInteger::as_u64),
        "pay_type": resource.pay_type.as_ref().and_then(FlexibleInteger::as_u64),
        "price": resource.price.as_ref().and_then(FlexibleInteger::as_u64),
        "media": {
            "duration_ms": resource.info.duration.as_ref().and_then(FlexibleInteger::as_u64),
            "size": resource.info.filesize.as_ref().and_then(FlexibleInteger::as_u64),
            "bitrate": resource.info.bitrate.as_ref().and_then(FlexibleInteger::as_u64),
            "format": nonempty(&resource.info.extname),
        },
        "popup": {
            "title": nonempty(&resource.popup.title),
            "content": nonempty(&resource.popup.content),
            "button": nonempty(&resource.popup.btn_name),
        },
        "trial": hash_offset_diagnostics(resource.trans_param.hash_offset.as_ref()),
    })
}

fn tracker_diagnostics(tracker: &TrackerEnvelope) -> Value {
    json!({
        "status": tracker.status,
        "quality_code": tracker.q.as_ref().and_then(FlexibleInteger::as_u64),
        "private_status": tracker.priv_status.as_ref().and_then(FlexibleInteger::as_u64),
        "fail_process": tracker.fail_process,
        "file_name": nonempty(&tracker.file_name),
        "format": nonempty(&tracker.extension),
        "hash": nonempty(&tracker.hash),
        "trial": hash_offset_diagnostics(tracker.hash_offset.as_ref()),
    })
}

fn hash_offset_diagnostics(offset: Option<&HashOffset>) -> Value {
    let Some(offset) = offset else {
        return Value::Null;
    };
    json!({
        "start_byte": offset.start_byte.as_ref().and_then(FlexibleInteger::as_u64),
        "end_byte": offset.end_byte.as_ref().and_then(FlexibleInteger::as_u64),
        "start_ms": offset.start_ms.as_ref().and_then(FlexibleInteger::as_u64),
        "end_ms": offset.end_ms.as_ref().and_then(FlexibleInteger::as_u64),
        "offset_hash": nonempty(&offset.offset_hash),
        "clip_hash": nonempty(&offset.clip_hash),
        "file_type": offset.file_type.as_ref().and_then(FlexibleInteger::as_u64),
    })
}

fn media_permission_error(
    request: &StreamRequest,
    privilege: &PrivilegeResource,
    tracker: &TrackerEnvelope,
) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::PermissionDenied,
        "KuGou did not authorize this media stream",
    )
    .with_platform(Platform::Kugou)
    .with_details(json!({
        "requested_quality": request.quality,
        "privilege": privilege_diagnostics(privilege),
        "tracker": tracker_diagnostics(tracker),
        "trial_available": trial_from_offsets(
            tracker
                .hash_offset
                .as_ref()
                .or(privilege.trans_param.hash_offset.as_ref())
        ).is_some(),
    }))
}

fn canonical_positive_u64(value: &str) -> Option<u64> {
    let parsed = value.parse::<u64>().ok()?;
    (parsed > 0 && parsed.to_string() == value).then_some(parsed)
}

fn kugou_invalid_media_request(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Kugou)
}

fn parse_lyric_search_response(bytes: &[u8]) -> Result<(LyricCandidate, Value)> {
    let envelope: LyricSearchEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| kugou_upstream_error("KuGou lyric search returned malformed JSON"))?;
    if envelope.status != 200 || envelope.errcode != 200 {
        return Err(
            kugou_upstream_error("KuGou lyric search rejected the request").with_details(json!({
                "platform_code": envelope.errcode,
                "platform_message": safe_upstream_message(
                    nonempty(&envelope.errmsg).unwrap_or(&envelope.info)
                ),
            })),
        );
    }
    if envelope.candidates.len() > 1_000 {
        return Err(kugou_upstream_error(
            "KuGou lyric search returned too many candidates",
        ));
    }
    let candidate_count = envelope.candidates.len();
    let proposal = nonempty(&envelope.proposal);
    let selected_index = envelope
        .candidates
        .iter()
        .position(|candidate| {
            proposal == Some(candidate.id.trim()) && valid_lyric_candidate(candidate)
        })
        .or_else(|| envelope.candidates.iter().position(valid_lyric_candidate))
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::ResourceNotFound,
                "KuGou did not return a usable lyric candidate",
            )
            .with_platform(Platform::Kugou)
        })?;
    let selected_by_proposal = proposal == Some(envelope.candidates[selected_index].id.trim());
    let candidate = envelope
        .candidates
        .into_iter()
        .nth(selected_index)
        .expect("selected lyric candidate index must exist");
    let diagnostics = json!({
        "keyword": nonempty(&envelope.keyword),
        "proposal": proposal,
        "candidate_count": candidate_count,
        "ugc_candidate_count": envelope.ugccandidates.len(),
        "ai_candidate_count": envelope.ai_candidates.len(),
        "selected_by_proposal": selected_by_proposal,
        "has_complete_right": envelope
            .has_complete_right
            .as_ref()
            .and_then(FlexibleInteger::as_u64),
        "expires_in_seconds": envelope.expire.as_ref().and_then(FlexibleInteger::as_u64),
    });
    Ok((candidate, diagnostics))
}

fn valid_lyric_candidate(candidate: &LyricCandidate) -> bool {
    canonical_positive_decimal(&candidate.id)
        && candidate.accesskey.len() == 32
        && candidate
            .accesskey
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
        && [&candidate.singer, &candidate.song, &candidate.nickname]
            .into_iter()
            .all(|value| !value.chars().any(disallowed_text_control))
}

fn parse_lyric_download_response(
    bytes: &[u8],
    candidate: &LyricCandidate,
    requested: RequestedLyricFormat,
) -> Result<DecodedLyric> {
    let envelope: LyricDownloadEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| kugou_upstream_error("KuGou lyric download returned malformed JSON"))?;
    if envelope.status != 200 || envelope.error_code != 0 {
        return Err(
            kugou_upstream_error("KuGou lyric download rejected the request").with_details(json!({
                "platform_code": envelope.error_code,
                "platform_message": safe_upstream_message(&envelope.info),
                "requested_format": requested.as_str(),
            })),
        );
    }
    if envelope.id.trim() != candidate.id {
        return Err(kugou_upstream_error(
            "KuGou lyric download returned a mismatched candidate identity",
        ));
    }
    if !matches!(envelope.fmt.trim(), "krc" | "lrc") {
        return Err(kugou_upstream_error(
            "KuGou lyric download returned an unknown format",
        ));
    }
    let decoded = BASE64.decode(envelope.content.as_bytes()).map_err(|_| {
        kugou_upstream_error("KuGou lyric download returned invalid base64 content")
    })?;
    if decoded.len() > MAX_API_RESPONSE_BYTES as usize {
        return Err(kugou_upstream_error(
            "KuGou lyric content exceeded the compressed size limit",
        ));
    }
    let (text, format) = if decoded.starts_with(b"krc1") {
        (decode_krc(&decoded)?, "krc")
    } else {
        let text = String::from_utf8(decoded).map_err(|_| {
            kugou_upstream_error("KuGou lyric download returned invalid UTF-8 text")
        })?;
        validate_lyric_text(&text)?;
        (text, "lrc")
    };
    Ok(DecodedLyric {
        text,
        format,
        content_type: envelope
            .contenttype
            .as_ref()
            .and_then(FlexibleInteger::as_u64),
        source: nonempty(&envelope.source).map(str::to_owned),
    })
}

fn decode_krc(bytes: &[u8]) -> Result<String> {
    if bytes.len() <= 4 || !bytes.starts_with(b"krc1") {
        return Err(kugou_upstream_error(
            "KuGou KRC content omitted its file header",
        ));
    }
    let mut compressed = bytes[4..].to_vec();
    for (index, byte) in compressed.iter_mut().enumerate() {
        *byte ^= KRC_XOR_KEY[index % KRC_XOR_KEY.len()];
    }
    let mut decoder = ZlibDecoder::new(compressed.as_slice())
        .take(u64::try_from(MAX_LYRIC_TEXT_BYTES + 1).unwrap_or(u64::MAX));
    let mut decoded = Vec::new();
    decoder
        .read_to_end(&mut decoded)
        .map_err(|_| kugou_upstream_error("KuGou KRC content could not be decompressed"))?;
    if decoded.len() > MAX_LYRIC_TEXT_BYTES {
        return Err(kugou_upstream_error(
            "KuGou KRC text exceeded the size limit",
        ));
    }
    let text = String::from_utf8(decoded)
        .map_err(|_| kugou_upstream_error("KuGou KRC content was not valid UTF-8"))?;
    validate_lyric_text(&text)?;
    Ok(text)
}

fn validate_lyric_text(text: &str) -> Result<()> {
    if text.is_empty()
        || text.len() > MAX_LYRIC_TEXT_BYTES
        || text.chars().any(disallowed_text_control)
    {
        return Err(kugou_upstream_error(
            "KuGou lyric content was empty or malformed",
        ));
    }
    Ok(())
}

fn disallowed_text_control(character: char) -> bool {
    character.is_control() && !matches!(character, '\n' | '\r' | '\t')
}

fn map_lyrics(
    album_audio_id: u64,
    candidate: LyricCandidate,
    search_diagnostics: Value,
    krc: Result<DecodedLyric>,
    lrc: Result<DecodedLyric>,
) -> Result<Lyrics> {
    let krc_failed = krc.is_err();
    let lrc_failed = lrc.is_err();
    if krc_failed && lrc_failed {
        return match krc {
            Err(error) => Err(error),
            Ok(_) => unreachable!("both lyric downloads were already checked as failed"),
        };
    }

    let mut word_synced = None;
    let mut plain = None;
    let mut download_diagnostics = Vec::new();
    for (requested, result) in [
        (RequestedLyricFormat::Krc, krc),
        (RequestedLyricFormat::Lrc, lrc),
    ] {
        let Ok(download) = result else {
            download_diagnostics.push(json!({
                "requested_format": requested.as_str(),
                "available": false,
            }));
            continue;
        };
        download_diagnostics.push(json!({
            "requested_format": requested.as_str(),
            "available": true,
            "actual_format": download.format,
            "content_type": download.content_type,
            "source": download.source,
        }));
        if download.format == "krc" {
            if word_synced.is_none() {
                word_synced = Some(download.text);
            }
        } else if plain.is_none() {
            plain = Some(download.text);
        }
    }
    if plain.is_none() {
        plain = word_synced.as_deref().and_then(krc_to_lrc);
    }
    if plain.is_none() && word_synced.is_none() {
        return Err(kugou_upstream_error(
            "KuGou lyric downloads did not contain a supported format",
        ));
    }

    let embedded = word_synced
        .as_deref()
        .map(parse_krc_languages)
        .transpose()?
        .unwrap_or_else(|| EmbeddedLyricLanguages {
            translated: None,
            romanized: None,
            extensions: Vec::new(),
            version: None,
        });
    let track_ref = ResourceRef::new(Platform::Kugou, album_audio_id.to_string())
        .map_err(|_| kugou_upstream_error("KuGou lyric identity was invalid"))?;
    let contributors = lyric_contributors(&candidate);
    let format = if word_synced.is_some() { "krc" } else { "lrc" }.to_owned();
    let mut extensions = Extensions::new();
    extensions.insert("search".to_owned(), search_diagnostics);
    extensions.insert(
        "candidate".to_owned(),
        json!({
            "id": candidate.id,
            "download_id": nonempty(&candidate.download_id),
            "product_from": nonempty(&candidate.product_from),
            "can_score": candidate.can_score,
            "singer": nonempty(&candidate.singer),
            "song": nonempty(&candidate.song),
            "duration_ms": candidate.duration.as_ref().and_then(FlexibleInteger::as_u64),
            "language": nonempty(&candidate.language),
            "krc_type": candidate.krctype.as_ref().and_then(FlexibleInteger::as_u64),
            "hit_layer": candidate.hitlayer.as_ref().and_then(FlexibleInteger::as_u64),
            "hit_case_mask": candidate.hitcasemask.as_ref().and_then(FlexibleInteger::as_u64),
            "adjust": candidate.adjust.as_ref().and_then(FlexibleInteger::as_u64),
            "score": candidate.score.as_ref().and_then(FlexibleInteger::as_u64),
            "content_type": candidate.contenttype.as_ref().and_then(FlexibleInteger::as_u64),
            "content_format": candidate
                .content_format
                .as_ref()
                .and_then(FlexibleInteger::as_u64),
        }),
    );
    extensions.insert("downloads".to_owned(), json!(download_diagnostics));
    if !embedded.extensions.is_empty() {
        extensions.insert(
            "embedded_languages".to_owned(),
            json!({
                "version": embedded.version,
                "sections": embedded.extensions,
            }),
        );
    }
    Ok(Lyrics {
        track_ref,
        plain,
        translated: embedded.translated,
        romanized: embedded.romanized,
        word_synced,
        singing_annotations: None,
        singing_annotations_timestamp: None,
        format,
        contributors,
        extensions,
    })
}

fn parse_krc_languages(text: &str) -> Result<EmbeddedLyricLanguages> {
    let Some(encoded) = text.lines().find_map(|line| {
        line.strip_prefix("[language:")
            .and_then(|value| value.strip_suffix(']'))
    }) else {
        return Ok(EmbeddedLyricLanguages {
            translated: None,
            romanized: None,
            extensions: Vec::new(),
            version: None,
        });
    };
    if encoded.len() > MAX_LYRIC_TEXT_BYTES {
        return Err(kugou_upstream_error(
            "KuGou embedded lyric languages exceeded the size limit",
        ));
    }
    let bytes = BASE64.decode(encoded).map_err(|_| {
        kugou_upstream_error("KuGou embedded lyric languages were not valid base64")
    })?;
    let payload: KrcLanguagePayload = serde_json::from_slice(&bytes)
        .map_err(|_| kugou_upstream_error("KuGou embedded lyric languages were malformed"))?;
    if payload.content.len() > 16 {
        return Err(kugou_upstream_error(
            "KuGou embedded lyric languages returned too many sections",
        ));
    }
    let mut translated = None;
    let mut romanized = None;
    let mut extensions = Vec::new();
    for section in payload.content {
        if section.lyric_content.len() > 20_000
            || section.lyric_content.iter().any(|line| line.len() > 256)
        {
            return Err(kugou_upstream_error(
                "KuGou embedded lyric language section was too large",
            ));
        }
        let section_type = section
            .section_type
            .as_ref()
            .and_then(FlexibleInteger::as_u64);
        let lines = section
            .lyric_content
            .into_iter()
            .map(|line| line.concat())
            .collect::<Vec<_>>();
        let joined = lines.join("\n");
        if joined.len() > MAX_LYRIC_TEXT_BYTES || joined.chars().any(disallowed_text_control) {
            return Err(kugou_upstream_error(
                "KuGou embedded lyric language section was malformed",
            ));
        }
        match section_type {
            Some(1) if translated.is_none() && !joined.is_empty() => {
                translated = Some(joined.clone());
            }
            Some(0) if romanized.is_none() && !joined.is_empty() => {
                romanized = Some(joined.clone());
            }
            _ => {}
        }
        extensions.push(json!({
            "type": section_type,
            "lines": lines,
        }));
    }
    Ok(EmbeddedLyricLanguages {
        translated,
        romanized,
        extensions,
        version: payload.version.as_ref().and_then(FlexibleInteger::as_u64),
    })
}

fn krc_to_lrc(text: &str) -> Option<String> {
    let mut output = String::new();
    for line in text.lines() {
        if let Some((timing, content)) =
            line.strip_prefix('[').and_then(|line| line.split_once(']'))
            && let Some((start, _duration)) = timing.split_once(',')
            && let Ok(start_ms) = start.parse::<u64>()
        {
            let minutes = start_ms / 60_000;
            let seconds = (start_ms % 60_000) / 1_000;
            let centiseconds = (start_ms % 1_000) / 10;
            let content = strip_krc_word_timings(content);
            output.push_str(&format!(
                "[{minutes:02}:{seconds:02}.{centiseconds:02}]{content}\n"
            ));
        } else if line.starts_with('[')
            && !line.starts_with("[language:")
            && !line.chars().any(disallowed_text_control)
        {
            output.push_str(line);
            output.push('\n');
        }
    }
    (!output.is_empty()).then_some(output)
}

fn strip_krc_word_timings(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(start) = rest.find('<') {
        output.push_str(&rest[..start]);
        let Some(end) = rest[start + 1..].find('>') else {
            output.push_str(&rest[start..]);
            return output;
        };
        rest = &rest[start + end + 2..];
    }
    output.push_str(rest);
    output
}

fn lyric_search_keyword(track: &Track) -> String {
    let artists = track
        .artists
        .iter()
        .map(|artist| artist.name.trim())
        .filter(|name| !name.is_empty())
        .collect::<Vec<_>>()
        .join("、");
    let keyword = if artists.is_empty() {
        track.name.clone()
    } else {
        format!("{artists} - {}", track.name)
    };
    truncate_utf8(&keyword, 512)
}

fn truncate_utf8(value: &str, maximum_bytes: usize) -> String {
    if value.len() <= maximum_bytes {
        return value.to_owned();
    }
    let boundary = value
        .char_indices()
        .map(|(index, _)| index)
        .take_while(|index| *index <= maximum_bytes)
        .last()
        .unwrap_or_default();
    value[..boundary].to_owned()
}

fn lyric_contributors(candidate: &LyricCandidate) -> Vec<LyricContributor> {
    let mut contributors = Vec::new();
    push_lyric_contributor(
        &mut contributors,
        "uploader",
        &candidate.uid,
        &candidate.nickname,
    );
    push_lyric_contributor(
        &mut contributors,
        "original",
        &candidate.origiuid,
        &candidate.originame,
    );
    push_lyric_contributor(
        &mut contributors,
        "translation",
        &candidate.transuid,
        &candidate.transname,
    );
    push_lyric_contributor(
        &mut contributors,
        "romanization",
        &candidate.sounduid,
        &candidate.soundname,
    );
    contributors
}

fn push_lyric_contributor(
    contributors: &mut Vec<LyricContributor>,
    role: &str,
    id: &str,
    name: &str,
) {
    let name = nonempty(name);
    let resource_ref = canonical_positive_decimal(id)
        .then(|| ResourceRef::new(Platform::Kugou, id.trim().to_owned()).ok())
        .flatten();
    if name.is_some() || resource_ref.is_some() {
        contributors.push(LyricContributor {
            role: role.to_owned(),
            resource_ref,
            name: name.unwrap_or("").to_owned(),
        });
    }
}

fn canonical_positive_decimal(value: &str) -> bool {
    let value = value.trim();
    value
        .parse::<u64>()
        .is_ok_and(|parsed| parsed > 0 && parsed.to_string() == value)
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
    let parameters = BTreeMap::from([
        ("appid", ANDROID_APP_ID.to_string()),
        ("clienttime", clienttime.to_string()),
        ("clientver", ANDROID_CLIENT_VERSION.to_string()),
        ("dfid", "-".to_owned()),
        ("mid", mid.to_owned()),
        ("uuid", "-".to_owned()),
    ]);
    android_signature_for_parameters(&parameters, body)
}

fn android_signature_for_parameters(parameters: &BTreeMap<&str, String>, body: &[u8]) -> String {
    let mut digest = Md5::new();
    digest.update(ANDROID_SIGNATURE_SALT);
    for (key, value) in parameters {
        digest.update(key.as_bytes());
        digest.update(b"=");
        digest.update(value.as_bytes());
    }
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

    fn lyric_candidate() -> LyricCandidate {
        LyricCandidate {
            id: "274944371".to_owned(),
            product_from: "official".to_owned(),
            accesskey: "2A7B35884B3C20E9D3281686BA59A3F8".to_owned(),
            singer: "artist".to_owned(),
            song: "song".to_owned(),
            duration: Some(FlexibleInteger::Unsigned(269_000)),
            ..LyricCandidate::default()
        }
    }

    fn encode_krc_fixture(text: &str) -> Vec<u8> {
        use std::io::Write as _;

        use flate2::{Compression, write::ZlibEncoder};

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(text.as_bytes())
            .expect("compress KRC text");
        let compressed = encoder.finish().expect("finish KRC compression");
        let mut bytes = b"krc1".to_vec();
        bytes.extend(
            compressed
                .into_iter()
                .enumerate()
                .map(|(index, byte)| byte ^ KRC_XOR_KEY[index % KRC_XOR_KEY.len()]),
        );
        bytes
    }

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
    fn lyric_search_prefers_the_platform_proposal_without_exposing_access_key() {
        let response = r#"{
          "status":200,"info":"OK","errcode":200,"errmsg":"OK",
          "keyword":"artist - song","proposal":"20","expire":7200,
          "candidates":[
            {"id":"10","accesskey":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","song":"other"},
            {
              "id":"20","accesskey":"BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB",
              "product_from":"official","singer":"artist","song":"song",
              "duration":269000,"contenttype":0,"content_format":1
            }
          ]
        }"#;
        let (candidate, diagnostics) =
            parse_lyric_search_response(response.as_bytes()).expect("parse lyric candidates");
        assert_eq!(candidate.id, "20");
        assert_eq!(diagnostics["selected_by_proposal"], true);
        assert_eq!(diagnostics["candidate_count"], 2);
        assert!(!diagnostics.to_string().contains("BBBB"));
    }

    #[test]
    fn krc_decoder_enforces_header_xor_zlib_and_utf8() {
        let text = "[0,1000]<0,500,0>逐<500,500,0>字\n";
        let encoded = encode_krc_fixture(text);
        assert_eq!(decode_krc(&encoded).expect("decode KRC"), text);
        assert!(decode_krc(b"not-krc").is_err());

        let mut corrupt = encoded;
        corrupt[4] ^= 0xff;
        assert!(decode_krc(&corrupt).is_err());
    }

    #[test]
    fn lyric_download_detects_actual_content_instead_of_trusting_requested_format() {
        let candidate = lyric_candidate();
        let krc = encode_krc_fixture("[0,1000]<0,1000,0>word\n");
        let response = json!({
            "status": 200,
            "info": "OK",
            "error_code": 0,
            "fmt": "krc",
            "contenttype": 0,
            "_source": "bss",
            "content": BASE64.encode(krc),
            "id": candidate.id,
        });
        let decoded = parse_lyric_download_response(
            response.to_string().as_bytes(),
            &candidate,
            RequestedLyricFormat::Krc,
        )
        .expect("decode lyric response");
        assert_eq!(decoded.format, "krc");
        assert!(decoded.text.contains("<0,1000,0>"));

        let fallback = json!({
            "status": 200,
            "info": "OK",
            "error_code": 0,
            "fmt": "krc",
            "contenttype": 1,
            "_source": "bss",
            "content": BASE64.encode("[00:00.00]line"),
            "id": candidate.id,
        });
        let decoded = parse_lyric_download_response(
            fallback.to_string().as_bytes(),
            &candidate,
            RequestedLyricFormat::Krc,
        )
        .expect("decode LRC fallback");
        assert_eq!(decoded.format, "lrc");
    }

    #[test]
    fn rich_krc_remains_primary_when_plain_lrc_is_also_available() {
        let languages = BASE64.encode(
            br#"{"version":1,"content":[{"type":1,"lyricContent":[["translated"]]},{"type":0,"lyricContent":[["romanized"]]}]}"#,
        );
        let krc = format!("[language:{languages}]\n[1000,1000]<0,500,0>逐<500,500,0>字\n");
        let lyrics = map_lyrics(
            32_100_650,
            lyric_candidate(),
            json!({"candidate_count": 1}),
            Ok(DecodedLyric {
                text: krc,
                format: "krc",
                content_type: Some(0),
                source: Some("bss".to_owned()),
            }),
            Ok(DecodedLyric {
                text: "[00:01.00]逐字\n".to_owned(),
                format: "lrc",
                content_type: Some(1),
                source: Some("bss".to_owned()),
            }),
        )
        .expect("map lyrics");
        assert_eq!(lyrics.format, "krc");
        assert_eq!(lyrics.plain.as_deref(), Some("[00:01.00]逐字\n"));
        assert!(
            lyrics
                .word_synced
                .as_deref()
                .is_some_and(|text| text.contains("<0,500,0>"))
        );
        assert_eq!(lyrics.translated.as_deref(), Some("translated"));
        assert_eq!(lyrics.romanized.as_deref(), Some("romanized"));
    }

    #[test]
    fn krc_can_produce_a_plain_lrc_fallback_without_losing_word_timing() {
        let krc = "[ti:song]\n[1000,1200]<0,500,0>逐<500,700,0>字\n";
        assert_eq!(
            krc_to_lrc(krc).as_deref(),
            Some("[ti:song]\n[00:01.00]逐字\n")
        );
    }

    #[test]
    fn media_selection_falls_back_by_quality_without_confusing_requested_quality() {
        let mut track = Track::new(
            ResourceRef::new(Platform::Kugou, "32100650").expect("track reference"),
            "track",
        );
        track.album = Some(AlbumSummary {
            resource_ref: Some(
                ResourceRef::new(Platform::Kugou, "966846").expect("album reference"),
            ),
            name: "album".to_owned(),
            cover_url: None,
        });
        track.extensions.insert(
            "qualities".to_owned(),
            json!({
                "standard": {
                    "hash": "B3A52A7A958BF0AED0EBFBA2E9A818B7",
                    "size": 4317292,
                    "bitrate": 128,
                    "duration_ms": 269000,
                    "format": "mp3"
                }
            }),
        );
        let request = StreamRequest {
            quality: Quality::Hires,
            ..StreamRequest::default()
        };
        let selected = select_media_spec(&track, &request).expect("select lower media");
        assert_eq!(selected.requested_quality, Quality::Hires);
        assert_eq!(selected.actual_quality, Quality::Standard);
        assert_eq!(selected.tracker_quality, "128");
        assert_eq!(selected.bitrate, Some(128_000));
    }

    #[test]
    fn tracker_urls_upgrade_only_trusted_kugou_hosts_and_deduplicate_backups() {
        let tracker = TrackerEnvelope {
            status: 1,
            hash: "HASH".to_owned(),
            url: vec![
                "http://fsandroid.kugou.com/path/audio.mp3".to_owned(),
                "http://fsmobile.tx.kugou.com/path/audio.mp3".to_owned(),
            ],
            backup_url: vec!["http://fsandroid.kugou.com/path/audio.mp3".to_owned()],
            ..TrackerEnvelope::default()
        };
        let (primary, backups) = map_tracker_urls(&tracker).expect("map tracker URLs");
        assert_eq!(
            primary.as_deref(),
            Some("https://fsandroid.kugou.com/path/audio.mp3")
        );
        assert_eq!(backups, ["https://fsmobile.tx.kugou.com/path/audio.mp3"]);

        let mut untrusted = tracker;
        untrusted.url = vec!["https://example.com/audio.mp3".to_owned()];
        assert!(map_tracker_urls(&untrusted).is_err());
    }

    #[test]
    fn privilege_and_trial_parsers_keep_media_identities_and_byte_window_exact() {
        let response = r#"{
          "status":1,"error_code":0,"message":"",
          "data":[{
            "type":"audio","id":"20505418","album_id":"966846",
            "album_audio_id":"32100650","hash":"BASEHASH","name":"track",
            "quality":"128","privilege":10,"status":0,"fail_process":12,"pay_type":3,
            "info":{"duration":269000,"filesize":4317292,"bitrate":128,"extname":"mp3"},
            "trans_param":{"hash_offset":{
              "start_byte":0,"end_byte":960115,"start_ms":0,"end_ms":60000,
              "offset_hash":"OFFSET","clip_hash":"CLIP","file_type":0
            }}
          }]
        }"#;
        let privilege =
            parse_privilege_response(response.as_bytes(), 32_100_650, 966_846, "basehash")
                .expect("parse privilege");
        let offset = privilege
            .trans_param
            .hash_offset
            .as_ref()
            .expect("trial offsets");
        assert_eq!(
            trial_from_offsets(Some(offset)),
            Some(TrialWindow {
                start_ms: 0,
                end_ms: 60_000
            })
        );
        assert_eq!(trial_size_from_offsets(Some(offset)), Some(960_116));
        assert!(parse_privilege_response(response.as_bytes(), 1, 966_846, "basehash").is_err());
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

    #[tokio::test]
    #[ignore = "requires live KuGou network access"]
    async fn live_public_lyrics_preserve_krc_over_lrc() {
        let client = KugouClient::new(&KugouConfig::default()).expect("create client");
        let lyrics = client.lyrics(32_100_650).await.expect("live KuGou lyrics");
        assert_eq!(lyrics.track_ref.to_string(), "kugou:32100650");
        assert_eq!(lyrics.format, "krc");
        assert!(
            lyrics.word_synced.as_deref().is_some_and(|text| {
                text.contains("[249072,3848]") && text.contains("<0,200,0>")
            })
        );
        assert!(
            lyrics
                .plain
                .as_deref()
                .is_some_and(|text| text.contains("[04:09.07]"))
        );
    }
}
