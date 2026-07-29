use std::{collections::BTreeMap, fmt, time::Duration};

use reqwest::{
    Client, Proxy, StatusCode,
    header::{ACCEPT, CONTENT_LENGTH},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tuneweave_core::{
    AlbumSummary, ArtistSummary, ErrorCode, Extensions, Lyrics, MediaDownload, MediaStream,
    Platform, Quality, ResourceRef, Result, StreamRequest, StreamVariant, Track, TrackAvailability,
    TrackAvailabilityRequest, TrialWindow, TuneWeaveError,
};
use url::Url;

const SEARCH_ENDPOINT: &str = "https://app.c.nf.migu.cn/bmw/search/song/v1.0";
const RESOURCE_INFO_ENDPOINT: &str =
    "https://app.u.nf.migu.cn/MIGUM2.0/v1.0/content/resourceinfo.do";
const LISTENING_RIGHTS_ENDPOINT: &str = "https://app.c.nf.migu.cn/strategy/pc/can-listen/v1.0";
const PUBLIC_STREAM_ENDPOINT: &str = "https://c.musicapp.migu.cn/strategy/listen-url/h5/v2.4";
const MEDIA_HOST: &str = "d.musicapp.migu.cn";
const PUBLIC_AUDIO_HOST: &str = "freetyst.nf.migu.cn";
const MAX_API_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_LYRIC_RESPONSE_BYTES: u64 = 4 * 1024 * 1024;
const USER_AGENT: &str = "TuneWeave/0.1 (Migu public music provider)";
const PUBLIC_STREAM_KEY: &[u8] = b"Jk8qzuePiJ1qE3mDYhLQ3T73DtDoAhLP";
const MRC_DELTA: i64 = 2_654_435_769;
const MRC_KEY: [i64; 4] = [
    27_303_562_373_562_475,
    18_014_862_372_307_051,
    22_799_692_160_172_081,
    34_058_940_340_699_235,
];

#[derive(Clone, Default)]
pub struct MiguConfig {
    pub proxy_url: Option<String>,
}

impl fmt::Debug for MiguConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MiguConfig")
            .field(
                "proxy_url",
                &self.proxy_url.as_ref().map(|_| "[configured]"),
            )
            .finish()
    }
}

#[derive(Clone)]
pub struct MiguClient {
    http: Client,
}

impl fmt::Debug for MiguClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_struct("MiguClient").finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub(crate) struct MiguSearchPage {
    pub tracks: Vec<Track>,
    pub has_next: bool,
    pub sequence: Option<String>,
    pub conditions: Vec<MiguSearchCondition>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MiguAudioFormat {
    #[serde(default)]
    resource_type: String,
    #[serde(default)]
    format_type: String,
    #[serde(default)]
    show_tags: Vec<String>,
    #[serde(default)]
    isize: Option<FlexibleU64>,
    #[serde(default)]
    asize: Option<FlexibleU64>,
    #[serde(default)]
    iformat: String,
    #[serde(default)]
    aformat: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MiguSearchCondition {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(rename = "type", default)]
    kind: String,
    #[serde(default)]
    datas: Vec<MiguSearchConditionValue>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MiguSearchConditionValue {
    #[serde(default)]
    title: String,
    #[serde(default)]
    condition_type: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MiguSinger {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    img: String,
    #[serde(default)]
    name_spelling: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct MiguSearchTag {
    #[serde(default)]
    name: String,
    #[serde(rename = "type", default)]
    kind: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct MiguSongExtension {
    disc: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguSong {
    resource_type: String,
    content_id: String,
    song_id: String,
    song_name: String,
    mv_id: String,
    mv_copyright_type: Option<i64>,
    ring_tone_id: String,
    ring_copyright_id: String,
    show_tags: Vec<String>,
    song_pinyin: String,
    audio_formats: Vec<MiguAudioFormat>,
    duration: Option<FlexibleU64>,
    play_num_desc: String,
    copyright_id: String,
    copyright_type: Option<i64>,
    restrict_type: Option<i64>,
    album_id: String,
    album: String,
    album_pinyin: String,
    img1: String,
    img2: String,
    img3: String,
    download_tags: Vec<String>,
    singer_list: Vec<MiguSinger>,
    ext: MiguSongExtension,
    forever_listen_flag: String,
    forever_listen: Option<bool>,
    action_img_url: String,
    shock_ring_id: String,
    has_associated_ring: Option<bool>,
    chorus_start_time: Option<FlexibleU64>,
    product_authorize_usage: String,
    audio_book: String,
    lrc_url: String,
    mrc_url: String,
    search_tags: Vec<MiguSearchTag>,
    more: Vec<MiguSong>,
    highlights: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct MiguAlternateTrack {
    content_id: String,
    song_id: Option<String>,
    name: String,
    resource_type: String,
    artists: Vec<MiguAlternateSinger>,
    album_id: Option<String>,
    album: Option<String>,
    duration_seconds: Option<u64>,
    copyright_id: Option<String>,
    copyright_type: Option<i64>,
    restrict_type: Option<i64>,
    forever_listen: Option<bool>,
    audio_formats: Vec<MiguAudioFormat>,
    cover_url: Option<String>,
    lrc_url: Option<String>,
    mrc_url: Option<String>,
    highlights: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct MiguAlternateSinger {
    id: Option<String>,
    name: String,
    name_spelling: Option<String>,
    image_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum FlexibleU64 {
    Number(u64),
    String(String),
}

impl FlexibleU64 {
    fn get(&self) -> Option<u64> {
        match self {
            Self::Number(value) => Some(*value),
            Self::String(value) => value.parse().ok(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MiguSearchRequest<'a> {
    page_no: u32,
    text: &'a str,
}

#[derive(Deserialize)]
struct MiguSearchEnvelope {
    #[serde(default)]
    code: String,
    #[serde(default)]
    info: String,
    data: Option<MiguSearchData>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguSearchData {
    has_next: bool,
    items: Vec<MiguSearchItem>,
    conditions: Vec<MiguSearchCondition>,
    seq: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct MiguSearchItem {
    song: Option<MiguSong>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MiguResourceInfoRequest<'a> {
    resource_id: &'a str,
    copyright_id: &'static str,
    resource_type: u8,
}

#[derive(Deserialize)]
struct MiguResourceEnvelope {
    #[serde(default)]
    code: String,
    #[serde(default)]
    info: String,
    #[serde(default)]
    resource: Vec<MiguResource>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguResource {
    resource_type: String,
    copyright_id: String,
    content_id: String,
    song_id: String,
    song_name: String,
    singer_id: String,
    singer: String,
    album_id: String,
    album: String,
    album_imgs: Vec<MiguImage>,
    op_num_item: Option<MiguResourceStatistics>,
    tone_control: String,
    related_songs: Vec<MiguRelatedResource>,
    rate_formats: Vec<MiguRateFormat>,
    new_rate_formats: Vec<MiguRateFormat>,
    lrc_url: String,
    mrc_url: String,
    trc_url: String,
    tag_list: Vec<MiguResourceTag>,
    copyright: String,
    valid_status: Option<bool>,
    song_descs: String,
    song_alias_name: String,
    is_in_d_album: String,
    is_in_side_dalbum: String,
    is_in_sales_period: String,
    song_type: String,
    invalidate_date: String,
    dalbum_id: String,
    track_number: String,
    disc: String,
    vip_type: String,
    scope_ofcopyright: String,
    auditions_type: String,
    first_icon: String,
    translate_name: String,
    charge_auditions: String,
    old_charge_auditions: String,
    song_icon: String,
    auditions_length: Option<FlexibleU64>,
    auditions_start_time: Option<FlexibleU64>,
    code_rate: BTreeMap<String, MiguCodeRateRights>,
    vip_flag: String,
    is_download: String,
    copyright_type: String,
    has_mv: String,
    mv_copyright: String,
    top_quality: String,
    first_click: String,
    pre_sale: String,
    is_share: String,
    is_collection: String,
    length: String,
    auditions_flag: String,
    listen_flag: String,
    artists: Vec<MiguResourceArtist>,
    landscap_img: String,
    vip_start_time: String,
    vip_end_time: String,
    vip_logo: String,
    vip_download: String,
    first_publish: String,
    first_start_time: String,
    first_end_time: String,
    show_tag: Vec<String>,
    material_valid_status: Option<bool>,
    need_encrypt: String,
    forever_listen_flag: String,
    forever_listen: Option<bool>,
    action_img_url: String,
    is_recreate: String,
    has_associated_ring: Option<bool>,
    chorus_start_time: Option<FlexibleU64>,
    product_authorize_usage: String,
    audio_book: String,
    originals: Vec<MiguOriginalTrack>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguImage {
    img_size_type: String,
    img: String,
    img_ori: String,
    webp_img: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguResourceArtist {
    id: String,
    name: String,
    name_spelling: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguRelatedResource {
    resource_type: String,
    resource_type_name: String,
    copyright_id: String,
    product_id: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguRateFormat {
    resource_type: String,
    format_type: String,
    format: String,
    size: Option<FlexibleU64>,
    file_type: String,
    price: String,
    android_file_type: String,
    ios_file_type: String,
    ios_size: Option<FlexibleU64>,
    android_size: Option<FlexibleU64>,
    ios_format: String,
    android_format: String,
    ios_accuracy_level: String,
    android_accuracy_level: String,
    show_tag: Vec<String>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguResourceTag {
    resource_type: String,
    tag_id: String,
    tag_name: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguOriginalTrack {
    #[serde(rename = "type")]
    kind: String,
    song_id: String,
    song_name: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguCodeRateRights {
    code_rate_charge_auditions: String,
    code_rate_auditions_length: Option<FlexibleU64>,
    code_rate_charge_auditions_type: String,
    is_code_rate_download: String,
    code_rate_file_size: Option<FlexibleU64>,
    content_id_sq: String,
    quality_icon: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguResourceStatistics {
    play_num: Option<FlexibleU64>,
    play_num_desc: String,
    keep_num: Option<FlexibleU64>,
    keep_num_desc: String,
    comment_num: Option<FlexibleU64>,
    comment_num_desc: String,
    share_num: Option<FlexibleU64>,
    share_num_desc: String,
    live_play_num: Option<FlexibleU64>,
    live_play_num_desc: String,
}

#[derive(Serialize)]
struct MiguResourceRights {
    valid_status: Option<bool>,
    material_valid_status: Option<bool>,
    copyright: Option<String>,
    copyright_type: Option<String>,
    vip_type: Option<String>,
    vip_flag: Option<String>,
    vip_download: Option<String>,
    is_download: Option<String>,
    listen_flag: Option<String>,
    auditions_flag: Option<String>,
    auditions_type: Option<String>,
    charge_auditions: Option<String>,
    old_charge_auditions: Option<String>,
    auditions_length_seconds: Option<u64>,
    auditions_start_seconds: Option<u64>,
    top_quality: Option<String>,
    forever_listen_flag: Option<String>,
    forever_listen: Option<bool>,
    need_encrypt: Option<String>,
    code_rates: BTreeMap<String, MiguCodeRateRights>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MiguListeningRightsRequest<'a> {
    content_ids: &'a str,
}

#[derive(Deserialize)]
struct MiguListeningRightsEnvelope {
    #[serde(default)]
    code: String,
    #[serde(default)]
    info: String,
    data: Option<MiguListeningRightsData>,
}

#[derive(Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguListeningRightsData {
    can_listen_resp_item_list: Vec<MiguListeningRights>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguListeningRights {
    content_id: String,
    can_listen: bool,
    limit_length: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MiguPublicStreamRequest<'a> {
    content_id: &'a str,
    copyright_id: &'a str,
    resource_type: u8,
    net_type: &'static str,
    tone_flag: &'static str,
    scene: &'static str,
    lower_quality_content_id: &'a str,
}

#[derive(Debug, Deserialize)]
struct MiguPlaybackEnvelope {
    #[serde(default)]
    code: String,
    #[serde(default)]
    info: String,
    data: Option<MiguPlaybackData>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguPlaybackData {
    version: String,
    url: String,
    audio_format_type: String,
    auditions_length: Option<FlexibleU64>,
    auditions_start_time: Option<FlexibleU64>,
    cannot_code: String,
    free_listen_type: String,
    dialog_info: Option<MiguPlaybackDialog>,
    song: Option<MiguPlaybackSong>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguPlaybackDialog {
    show_type: Option<i64>,
    text: String,
    pay_complete_text: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct MiguPlaybackSong {
    resource_type: String,
    content_id: String,
    copyright_id: String,
    duration: Option<FlexibleU64>,
}

#[derive(Clone, Copy)]
struct MiguSelectedTone {
    requested_quality: Quality,
    tone_flag: &'static str,
}

struct MiguMediaResolution {
    url: String,
    requested_quality: Quality,
    actual_quality: Quality,
    format: Option<String>,
    codec: Option<String>,
    bitrate: Option<u64>,
    size: Option<u64>,
    duration_ms: Option<u64>,
    trial: Option<TrialWindow>,
    rights: MiguListeningRights,
    playback_version: Option<String>,
    free_listen_type: Option<String>,
    dialog_show_type: Option<i64>,
    dialog_text: Option<String>,
    pay_complete_text: Option<String>,
    requested_tone: &'static str,
    actual_tone: String,
}

struct DownloadedLyric {
    text: String,
    content_type: Option<String>,
    byte_length: usize,
}

#[derive(Clone, Copy)]
enum MiguLyricKind {
    Plain,
    WordSynced,
    Translated,
}

impl MiguLyricKind {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Plain => "lrc",
            Self::WordSynced => "mrc",
            Self::Translated => "trc",
        }
    }
}

impl MiguClient {
    pub fn new(config: &MiguConfig) -> Result<Self> {
        let mut builder = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .redirect(Policy::none())
            .user_agent(USER_AGENT);
        if let Some(proxy_url) = config.proxy_url.as_deref() {
            let proxy = Proxy::all(proxy_url).map_err(|_| {
                migu_invalid_request("Migu proxy configuration is not a valid proxy URL")
            })?;
            builder = builder.proxy(proxy);
        }
        let http = builder.build().map_err(|_| {
            TuneWeaveError::new(ErrorCode::InternalError, "failed to build Migu HTTP client")
                .with_platform(Platform::Migu)
        })?;
        Ok(Self { http })
    }

    #[cfg(test)]
    pub(crate) fn test_client() -> Self {
        Self::new(&MiguConfig::default()).expect("create Migu test client")
    }

    pub(crate) async fn search_tracks_page(
        &self,
        keyword: &str,
        page: u32,
    ) -> Result<MiguSearchPage> {
        let response = self
            .http
            .get(SEARCH_ENDPOINT)
            .header(ACCEPT, "application/json")
            .query(&MiguSearchRequest {
                page_no: page,
                text: keyword,
            })
            .send()
            .await
            .map_err(migu_network_error)?;
        let bytes = read_bounded_response(response, "Migu search").await?;
        parse_search_response(&bytes)
    }

    pub(crate) async fn track_detail(&self, content_id: &str) -> Result<Track> {
        let resource = self.resource_info(content_id).await?;
        map_resource_track(resource)
    }

    pub(crate) async fn lyrics(&self, content_id: &str) -> Result<Lyrics> {
        let resource = self.resource_info(content_id).await?;
        let lrc_url = validated_optional_lyric_url(&resource.lrc_url, "LRC")?;
        let mrc_url = validated_optional_lyric_url(&resource.mrc_url, "MRC")?;
        let trc_url = validated_optional_lyric_url(&resource.trc_url, "TRC")?;
        if lrc_url.is_none() && mrc_url.is_none() {
            return Err(TuneWeaveError::new(
                ErrorCode::ResourceNotFound,
                "Migu lyrics were not found",
            )
            .with_platform(Platform::Migu));
        }

        let lrc = self.download_optional_lyric(lrc_url, MiguLyricKind::Plain);
        let mrc = self.download_optional_lyric(mrc_url, MiguLyricKind::WordSynced);
        let trc = self.download_optional_lyric(trc_url, MiguLyricKind::Translated);
        let (lrc, mrc, trc) = tokio::join!(lrc, mrc, trc);
        map_lyrics(content_id, &resource.copyright_id, lrc, mrc, trc)
    }

    pub(crate) async fn track_availability(
        &self,
        content_id: &str,
        request: &TrackAvailabilityRequest,
    ) -> Result<TrackAvailability> {
        let rights = self.listening_rights(content_id).await?;
        let track_ref = migu_track_ref(content_id)?;
        let mut extensions = Extensions::new();
        extensions.insert("backend".to_owned(), json!("pc_can_listen_v1"));
        extensions.insert("limit_length".to_owned(), json!(rights.limit_length));
        Ok(TrackAvailability {
            track_ref,
            playable: rights.can_listen,
            requested_bitrate: request.bitrate,
            actual_bitrate: None,
            platform_code: Some(0),
            message: if rights.can_listen {
                "ok".to_owned()
            } else if rights.limit_length {
                "Migu only permits a limited preview".to_owned()
            } else {
                "Migu did not authorize full playback".to_owned()
            },
            extensions,
        })
    }

    pub(crate) async fn stream(
        &self,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<MediaStream> {
        let resolution = self.resolve_media(track, request).await?;
        Ok(MediaStream {
            url: resolution.url,
            backup_urls: Vec::new(),
            headers: BTreeMap::new(),
            expires_at: None,
            format: resolution.format,
            codec: resolution.codec,
            bitrate: resolution.bitrate,
            size: resolution.size,
            duration_ms: resolution.duration_ms,
            requested_quality: resolution.requested_quality,
            actual_quality: resolution.actual_quality,
            trial: resolution.trial,
            origin_track: Some(track.resource_ref.clone()),
            resolved_track: track.resource_ref.clone(),
            resolved_platform: Platform::Migu,
            match_score: Some(1.0),
            attempts: Vec::new(),
        })
    }

    pub(crate) async fn download(
        &self,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<MediaDownload> {
        let resolution = self.resolve_media(track, request).await?;
        let available = resolution.rights.can_listen && resolution.trial.is_none();
        let message = (!available).then(|| {
            resolution
                .pay_complete_text
                .as_deref()
                .or(resolution.dialog_text.as_deref())
                .unwrap_or("Migu only returned a preview; a full download is unavailable")
                .to_owned()
        });
        let mut extensions = media_resolution_diagnostics(&resolution);
        extensions.insert("preview_url_withheld".to_owned(), json!(!available));
        Ok(MediaDownload {
            track_ref: track.resource_ref.clone(),
            platform: Platform::Migu,
            available,
            url: available.then_some(resolution.url),
            headers: BTreeMap::new(),
            expires_at: None,
            format: resolution.format,
            codec: resolution.codec,
            bitrate: resolution.bitrate,
            size: available.then_some(resolution.size).flatten(),
            duration_ms: resolution.duration_ms,
            requested_quality: resolution.requested_quality,
            actual_quality: resolution.actual_quality,
            platform_code: Some(0),
            fee: None,
            message,
            extensions,
        })
    }

    async fn resolve_media(
        &self,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<MiguMediaResolution> {
        let content_id = canonical_media_track_id(track)?;
        let selected = select_media_tone(track, request)?;
        let resource = self.resource_info(content_id).await?;
        let copyright_id = canonical_platform_id(&resource.copyright_id)
            .ok_or_else(|| migu_upstream_error("Migu media metadata omitted a copyright ID"))?
            .to_owned();
        let rights = self.listening_rights(content_id);
        let playback = self.public_stream(content_id, &copyright_id, selected.tone_flag);
        let (rights, playback) = tokio::try_join!(rights, playback)?;
        validate_playback_identity(&playback, content_id, &copyright_id)?;
        let actual_tone = canonical_playback_tone(&playback.audio_format_type)?;
        let actual_quality = quality_for_migu_tone(actual_tone)?;
        let url = validate_public_audio_url(&playback.url)?;
        let trial = playback_trial(&rights, &playback)?;
        let (format, codec, bitrate) = media_spec_from_url(&url, actual_tone)?;
        let size = (!url.is_empty())
            .then(|| rate_format_size(&resource, actual_tone))
            .flatten();
        let duration_ms = playback
            .song
            .as_ref()
            .and_then(|song| song.duration.as_ref())
            .and_then(FlexibleU64::get)
            .and_then(|seconds| seconds.checked_mul(1_000))
            .or(parse_duration_text(&resource.length)?)
            .or(track.duration_ms);
        let dialog_show_type = playback
            .dialog_info
            .as_ref()
            .and_then(|dialog| dialog.show_type);
        let dialog_text = playback
            .dialog_info
            .as_ref()
            .and_then(|dialog| bounded_optional(&dialog.text, 512));
        let pay_complete_text = playback
            .dialog_info
            .as_ref()
            .and_then(|dialog| bounded_optional(&dialog.pay_complete_text, 512));
        Ok(MiguMediaResolution {
            url,
            requested_quality: selected.requested_quality,
            actual_quality,
            format,
            codec,
            bitrate,
            size,
            duration_ms,
            trial,
            rights,
            playback_version: bounded_optional(&playback.version, 128),
            free_listen_type: bounded_optional(&playback.free_listen_type, 32),
            dialog_show_type,
            dialog_text,
            pay_complete_text,
            requested_tone: selected.tone_flag,
            actual_tone: actual_tone.to_owned(),
        })
    }

    async fn listening_rights(&self, content_id: &str) -> Result<MiguListeningRights> {
        let response = self
            .http
            .post(LISTENING_RIGHTS_ENDPOINT)
            .header(ACCEPT, "application/json")
            .header("channel", "014X031")
            .json(&MiguListeningRightsRequest {
                content_ids: content_id,
            })
            .send()
            .await
            .map_err(migu_network_error)?;
        let bytes = read_bounded_response(response, "Migu listening rights").await?;
        parse_listening_rights_response(&bytes, content_id)
    }

    async fn public_stream(
        &self,
        content_id: &str,
        copyright_id: &str,
        tone_flag: &'static str,
    ) -> Result<MiguPlaybackData> {
        let response = self
            .http
            .get(PUBLIC_STREAM_ENDPOINT)
            .header(ACCEPT, "application/octet-stream, application/json")
            .header("birth", "h5page")
            .header("channel", "014X031")
            .header("referer", "https://y.migu.cn/")
            .header("location-data", "30.6698676660,104.1229614820")
            .header("location-info", "")
            .query(&MiguPublicStreamRequest {
                content_id,
                copyright_id,
                resource_type: 2,
                net_type: "01",
                tone_flag,
                scene: "",
                lower_quality_content_id: content_id,
            })
            .send()
            .await
            .map_err(migu_network_error)?;
        let bytes = read_bounded_response(response, "Migu public stream").await?;
        parse_public_stream_response(&bytes)
    }

    async fn resource_info(&self, content_id: &str) -> Result<MiguResource> {
        let response = self
            .http
            .get(RESOURCE_INFO_ENDPOINT)
            .header(ACCEPT, "application/json")
            .query(&MiguResourceInfoRequest {
                resource_id: content_id,
                copyright_id: "",
                resource_type: 2,
            })
            .send()
            .await
            .map_err(migu_network_error)?;
        let bytes = read_bounded_response(response, "Migu resource detail").await?;
        parse_resource_record(&bytes, content_id)
    }

    async fn download_optional_lyric(
        &self,
        url: Option<String>,
        kind: MiguLyricKind,
    ) -> Result<Option<DownloadedLyric>> {
        let Some(url) = url else {
            return Ok(None);
        };
        let response = self
            .http
            .get(url)
            .header(ACCEPT, "*/*")
            .header("referer", "https://app.c.nf.migu.cn/")
            .header("channel", "0146921")
            .send()
            .await
            .map_err(migu_network_error)?;
        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| bounded_text(value, 128));
        let bytes = read_bounded_response_with_limit(
            response,
            &format!("Migu {} lyric", kind.as_str().to_ascii_uppercase()),
            MAX_LYRIC_RESPONSE_BYTES,
        )
        .await?;
        let byte_length = bytes.len();
        let text = match kind {
            MiguLyricKind::WordSynced => decrypt_mrc(&bytes)?,
            MiguLyricKind::Plain | MiguLyricKind::Translated => decode_lyric_text(&bytes)?,
        };
        Ok(Some(DownloadedLyric {
            text,
            content_type,
            byte_length,
        }))
    }
}

fn parse_search_response(bytes: &[u8]) -> Result<MiguSearchPage> {
    let envelope: MiguSearchEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| migu_upstream_error("Migu search returned malformed JSON"))?;
    if envelope.code != "000000" {
        return Err(
            migu_upstream_error("Migu search rejected the request").with_details(json!({
                "platform_code": bounded_text(&envelope.code, 64),
                "platform_message": bounded_text(&envelope.info, 256),
            })),
        );
    }
    let data = envelope
        .data
        .ok_or_else(|| migu_upstream_error("Migu search response omitted data"))?;
    let tracks = data
        .items
        .into_iter()
        .map(|item| {
            item.song
                .ok_or_else(|| migu_upstream_error("Migu search result omitted song data"))
                .and_then(map_song)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(MiguSearchPage {
        tracks,
        has_next: data.has_next,
        sequence: nonempty(&data.seq).map(|value| bounded_text(value, 512)),
        conditions: bounded_conditions(data.conditions),
    })
}

#[cfg(test)]
fn parse_resource_response(bytes: &[u8], requested_content_id: &str) -> Result<Track> {
    parse_resource_record(bytes, requested_content_id).and_then(map_resource_track)
}

fn parse_resource_record(bytes: &[u8], requested_content_id: &str) -> Result<MiguResource> {
    let envelope: MiguResourceEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| migu_upstream_error("Migu resource detail returned malformed JSON"))?;
    if envelope.code != "000000" {
        return Err(
            migu_upstream_error("Migu resource detail rejected the request").with_details(json!({
                "platform_code": bounded_text(&envelope.code, 64),
                "platform_message": bounded_text(&envelope.info, 256),
            })),
        );
    }
    if envelope.resource.is_empty() {
        return Err(
            TuneWeaveError::new(ErrorCode::ResourceNotFound, "Migu track was not found")
                .with_platform(Platform::Migu),
        );
    }
    if envelope.resource.len() != 1 {
        return Err(migu_upstream_error(
            "Migu resource detail returned an unexpected number of tracks",
        ));
    }
    let resource =
        envelope.resource.into_iter().next().ok_or_else(|| {
            migu_upstream_error("Migu resource detail omitted the requested track")
        })?;
    if resource.content_id != requested_content_id {
        return Err(migu_upstream_error(
            "Migu resource detail returned a mismatched content ID",
        ));
    }
    Ok(resource)
}

fn parse_listening_rights_response(
    bytes: &[u8],
    requested_content_id: &str,
) -> Result<MiguListeningRights> {
    let envelope: MiguListeningRightsEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| migu_upstream_error("Migu listening rights returned malformed JSON"))?;
    if envelope.code != "000000" {
        return Err(
            migu_upstream_error("Migu listening rights rejected the request").with_details(json!({
                "platform_code": bounded_text(&envelope.code, 64),
                "platform_message": bounded_text(&envelope.info, 256),
            })),
        );
    }
    let data = envelope
        .data
        .ok_or_else(|| migu_upstream_error("Migu listening rights omitted data"))?;
    let [rights] =
        <[MiguListeningRights; 1]>::try_from(data.can_listen_resp_item_list).map_err(|_| {
            migu_upstream_error("Migu listening rights returned an unexpected result count")
        })?;
    if rights.content_id != requested_content_id {
        return Err(migu_upstream_error(
            "Migu listening rights returned a mismatched content ID",
        ));
    }
    if rights.can_listen && rights.limit_length {
        return Err(migu_upstream_error(
            "Migu listening rights returned contradictory full and limited flags",
        ));
    }
    Ok(rights)
}

fn parse_public_stream_response(bytes: &[u8]) -> Result<MiguPlaybackData> {
    let decoded = decrypt_public_stream_response(bytes)?;
    let envelope: MiguPlaybackEnvelope = serde_json::from_slice(&decoded)
        .map_err(|_| migu_upstream_error("Migu public stream returned malformed JSON"))?;
    if envelope.code != "000000" {
        return Err(TuneWeaveError::new(
            ErrorCode::PermissionDenied,
            "Migu did not authorize the requested public media",
        )
        .with_platform(Platform::Migu)
        .with_details(json!({
            "platform_code": bounded_text(&envelope.code, 64),
            "platform_message": bounded_text(&envelope.info, 256),
        })));
    }
    envelope
        .data
        .ok_or_else(|| migu_upstream_error("Migu public stream omitted data"))
}

fn decrypt_public_stream_response(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.len() < 4 || bytes[..3] != [0xab, 0xcd, 0x01] {
        return Err(migu_upstream_error(
            "Migu public stream returned an invalid encrypted envelope",
        ));
    }
    let seed = bytes[3];
    Ok(bytes[4..]
        .iter()
        .enumerate()
        .map(|(index, byte)| {
            byte.wrapping_add(seed)
                .wrapping_sub(PUBLIC_STREAM_KEY[index % PUBLIC_STREAM_KEY.len()])
        })
        .collect())
}

fn canonical_media_track_id(track: &Track) -> Result<&str> {
    if track.platform != Platform::Migu || track.resource_ref.platform() != Platform::Migu {
        return Err(migu_invalid_request(
            "Migu media resolution requires a Migu track",
        ));
    }
    let content_id = track.resource_ref.id();
    if content_id != track.id
        || content_id.is_empty()
        || content_id.len() > 64
        || !content_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(migu_invalid_request(
            "Migu media resolution requires a canonical contentId",
        ));
    }
    Ok(content_id)
}

fn migu_track_ref(content_id: &str) -> Result<ResourceRef> {
    ResourceRef::new(Platform::Migu, content_id.to_owned())
        .map_err(|_| migu_upstream_error("Migu returned an invalid track identity"))
}

fn select_media_tone(track: &Track, request: &StreamRequest) -> Result<MiguSelectedTone> {
    if request.variant != StreamVariant::Default {
        return Err(migu_invalid_request(
            "Migu public media only supports the default stream variant",
        ));
    }
    if request.account.is_some() {
        return Err(migu_invalid_request(
            "Migu public media does not accept an account",
        ));
    }
    if request.immersive_type.is_some() {
        return Err(migu_invalid_request(
            "Migu public media does not accept immersive_type",
        ));
    }
    let tone_flag = if let Some(bitrate) = request.bitrate {
        match bitrate {
            1..=128_000 => "PQ",
            128_001..=320_000 => "HQ",
            _ => {
                return Err(migu_invalid_request(
                    "Migu public media bitrate must be between 1 and 320000; use quality for lossless media",
                ));
            }
        }
    } else {
        match request.quality {
            Quality::Auto => {
                if track.available_qualities.contains(&Quality::Hires) {
                    "ZQ24"
                } else if track.available_qualities.contains(&Quality::Lossless) {
                    "SQ"
                } else if track.available_qualities.contains(&Quality::High) {
                    "HQ"
                } else {
                    "PQ"
                }
            }
            Quality::Low | Quality::Standard => "PQ",
            Quality::Higher | Quality::High => "HQ",
            Quality::Lossless => "SQ",
            Quality::Hires => "ZQ24",
            Quality::Surround | Quality::Spatial | Quality::Dolby | Quality::Master => {
                return Err(migu_invalid_request(
                    "Migu public media does not expose immersive or master quality families",
                ));
            }
        }
    };
    Ok(MiguSelectedTone {
        requested_quality: request.quality,
        tone_flag,
    })
}

fn validate_playback_identity(
    playback: &MiguPlaybackData,
    requested_content_id: &str,
    requested_copyright_id: &str,
) -> Result<()> {
    let Some(song) = playback.song.as_ref() else {
        return Ok(());
    };
    if (!song.resource_type.is_empty() && song.resource_type != "2")
        || (!song.content_id.is_empty() && song.content_id != requested_content_id)
        || (!song.copyright_id.is_empty() && song.copyright_id != requested_copyright_id)
    {
        return Err(migu_upstream_error(
            "Migu public stream returned mismatched media identity",
        ));
    }
    Ok(())
}

fn canonical_playback_tone(value: &str) -> Result<&'static str> {
    match value.trim() {
        "PQ" => Ok("PQ"),
        "HQ" => Ok("HQ"),
        "SQ" => Ok("SQ"),
        "ZQ" => Ok("ZQ"),
        "ZQ24" => Ok("ZQ24"),
        _ => Err(migu_upstream_error(
            "Migu public stream returned an unknown audio format",
        )),
    }
}

fn quality_for_migu_tone(tone: &str) -> Result<Quality> {
    match tone {
        "PQ" => Ok(Quality::Standard),
        "HQ" => Ok(Quality::High),
        "SQ" => Ok(Quality::Lossless),
        "ZQ" | "ZQ24" => Ok(Quality::Hires),
        _ => Err(migu_upstream_error(
            "Migu public stream returned an unknown quality",
        )),
    }
}

fn validate_public_audio_url(value: &str) -> Result<String> {
    if value.is_empty() || value.len() > 8_192 {
        return Err(migu_media_permission_error(
            "Migu did not return a public media URL",
        ));
    }
    let url = Url::parse(value)
        .map_err(|_| migu_upstream_error("Migu public stream returned an invalid media URL"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str() != Some(PUBLIC_AUDIO_HOST)
        || !matches!(url.port(), None | Some(443))
        || !(url.path().starts_with("/public/product8th/product")
            || url.path().starts_with("/public/product9th/product"))
        || url.fragment().is_some()
    {
        return Err(migu_upstream_error(
            "Migu public stream returned an untrusted media URL",
        ));
    }
    let mut timestamp_count = 0_u8;
    let mut key_count = 0_u8;
    let mut session_count = 0_u8;
    let mut pair_count = 0_usize;
    for (key, value) in url.query_pairs() {
        pair_count = pair_count.saturating_add(1);
        if pair_count > 16 || key.len() > 64 || value.len() > 512 {
            return Err(migu_upstream_error(
                "Migu public stream returned an invalid signed URL",
            ));
        }
        match key.as_ref() {
            "Tim" if !value.is_empty() => timestamp_count = timestamp_count.saturating_add(1),
            "Key" if !value.is_empty() => key_count = key_count.saturating_add(1),
            "playSessionId" if !value.is_empty() => {
                session_count = session_count.saturating_add(1);
            }
            _ => {}
        }
    }
    if timestamp_count != 1 || key_count != 1 || session_count != 1 {
        return Err(migu_upstream_error(
            "Migu public stream returned invalid URL authorization fields",
        ));
    }
    Ok(url.to_string())
}

fn playback_trial(
    rights: &MiguListeningRights,
    playback: &MiguPlaybackData,
) -> Result<Option<TrialWindow>> {
    let start = playback
        .auditions_start_time
        .as_ref()
        .and_then(FlexibleU64::get);
    let length = playback
        .auditions_length
        .as_ref()
        .and_then(FlexibleU64::get);
    let trial = match (start, length) {
        (None, None) => None,
        (Some(start), Some(length)) if length > 0 => {
            let start_ms = start
                .checked_mul(1_000)
                .ok_or_else(|| migu_upstream_error("Migu preview start overflowed"))?;
            let end_ms = start
                .checked_add(length)
                .and_then(|value| value.checked_mul(1_000))
                .ok_or_else(|| migu_upstream_error("Migu preview end overflowed"))?;
            Some(TrialWindow { start_ms, end_ms })
        }
        _ => {
            return Err(migu_upstream_error(
                "Migu public stream returned an incomplete preview window",
            ));
        }
    };
    match (rights.can_listen, rights.limit_length, trial) {
        (true, false, None) => Ok(None),
        (false, true, Some(trial)) => Ok(Some(trial)),
        (false, false, _) => Err(migu_media_permission_error(
            "Migu did not authorize public playback",
        )),
        _ => Err(migu_upstream_error(
            "Migu playback response contradicted its listening rights",
        )),
    }
}

fn media_spec_from_url(
    url: &str,
    actual_tone: &str,
) -> Result<(Option<String>, Option<String>, Option<u64>)> {
    let parsed = Url::parse(url)
        .map_err(|_| migu_upstream_error("Migu public stream returned an invalid media URL"))?;
    let extension = parsed
        .path_segments()
        .and_then(Iterator::last)
        .and_then(|name| name.rsplit_once('.').map(|(_, extension)| extension))
        .map(str::to_ascii_lowercase)
        .ok_or_else(|| migu_upstream_error("Migu public media URL omitted a file format"))?;
    match (actual_tone, extension.as_str()) {
        ("PQ", "mp3") => Ok((Some(extension.clone()), Some(extension), Some(128_000))),
        ("HQ", "mp3") => Ok((Some(extension.clone()), Some(extension), Some(320_000))),
        ("SQ" | "ZQ" | "ZQ24", "flac") => Ok((Some(extension.clone()), Some(extension), None)),
        _ => Err(migu_upstream_error(
            "Migu public stream format did not match its audio quality",
        )),
    }
}

fn rate_format_size(resource: &MiguResource, actual_tone: &str) -> Option<u64> {
    resource
        .new_rate_formats
        .iter()
        .chain(&resource.rate_formats)
        .find(|format| tone_matches_rate_format(actual_tone, &format.format_type))
        .and_then(|format| {
            format
                .size
                .as_ref()
                .or(format.android_size.as_ref())
                .or(format.ios_size.as_ref())
                .and_then(FlexibleU64::get)
                .filter(|size| *size > 0)
        })
}

fn tone_matches_rate_format(tone: &str, format: &str) -> bool {
    tone == format || (matches!(tone, "ZQ" | "ZQ24") && matches!(format, "ZQ" | "ZQ24"))
}

fn media_resolution_diagnostics(resolution: &MiguMediaResolution) -> Extensions {
    let mut extensions = Extensions::new();
    extensions.insert("backend".to_owned(), json!("listen_url_h5_v2_4"));
    extensions.insert(
        "requested_tone".to_owned(),
        json!(resolution.requested_tone),
    );
    extensions.insert("actual_tone".to_owned(), json!(resolution.actual_tone));
    extensions.insert(
        "rights".to_owned(),
        json!({
            "can_listen": resolution.rights.can_listen,
            "limit_length": resolution.rights.limit_length,
        }),
    );
    extensions.insert("trial".to_owned(), json!(resolution.trial));
    if let Some(value) = resolution.playback_version.as_deref() {
        extensions.insert("playback_version".to_owned(), json!(value));
    }
    if let Some(value) = resolution.free_listen_type.as_deref() {
        extensions.insert("free_listen_type".to_owned(), json!(value));
    }
    if let Some(value) = resolution.dialog_show_type {
        extensions.insert("dialog_show_type".to_owned(), json!(value));
    }
    extensions
}

fn migu_media_permission_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::PermissionDenied, message).with_platform(Platform::Migu)
}

fn map_lyrics(
    content_id: &str,
    copyright_id: &str,
    lrc: Result<Option<DownloadedLyric>>,
    mrc: Result<Option<DownloadedLyric>>,
    trc: Result<Option<DownloadedLyric>>,
) -> Result<Lyrics> {
    let mut diagnostics = Vec::new();
    let mut first_primary_error = None;
    let lrc = consume_lyric_download(
        MiguLyricKind::Plain,
        lrc,
        &mut diagnostics,
        &mut first_primary_error,
    );
    let mrc = consume_lyric_download(
        MiguLyricKind::WordSynced,
        mrc,
        &mut diagnostics,
        &mut first_primary_error,
    );
    let mut ignored_translation_error = None;
    let trc = consume_lyric_download(
        MiguLyricKind::Translated,
        trc,
        &mut diagnostics,
        &mut ignored_translation_error,
    );
    if lrc.is_none() && mrc.is_none() {
        return Err(first_primary_error.unwrap_or_else(|| {
            TuneWeaveError::new(ErrorCode::ResourceNotFound, "Migu lyrics were not found")
                .with_platform(Platform::Migu)
        }));
    }

    let word_synced = mrc.as_ref().map(|download| download.text.clone());
    let (plain, plain_source) = if let Some(download) = lrc {
        (Some(download.text), "lrc")
    } else if let Some(text) = word_synced.as_deref() {
        (Some(mrc_to_lrc(text)?), "derived_mrc")
    } else {
        (None, "unavailable")
    };
    let format = if word_synced.is_some() { "mrc" } else { "lrc" };
    let track_ref = ResourceRef::new(Platform::Migu, content_id.to_owned())
        .map_err(|_| migu_upstream_error("Migu lyric identity was invalid"))?;
    let mut extensions = Extensions::new();
    extensions.insert("backend".to_owned(), json!("resourceinfo_v1"));
    extensions.insert(
        "copyright_id".to_owned(),
        json!(bounded_text(copyright_id, 64)),
    );
    extensions.insert("plain_source".to_owned(), json!(plain_source));
    extensions.insert("downloads".to_owned(), json!(diagnostics));
    if word_synced.is_some() {
        extensions.insert("word_synced_format".to_owned(), json!("migu_mrc"));
    }
    Ok(Lyrics {
        track_ref,
        plain,
        translated: trc.map(|download| download.text),
        romanized: None,
        word_synced,
        singing_annotations: None,
        singing_annotations_timestamp: None,
        format: format.to_owned(),
        contributors: Vec::new(),
        extensions,
    })
}

fn consume_lyric_download(
    kind: MiguLyricKind,
    result: Result<Option<DownloadedLyric>>,
    diagnostics: &mut Vec<serde_json::Value>,
    first_error: &mut Option<TuneWeaveError>,
) -> Option<DownloadedLyric> {
    match result {
        Ok(Some(download)) => {
            diagnostics.push(json!({
                "format": kind.as_str(),
                "available": true,
                "content_type": download.content_type,
                "byte_length": download.byte_length,
            }));
            Some(download)
        }
        Ok(None) => {
            diagnostics.push(json!({
                "format": kind.as_str(),
                "available": false,
                "reason": "not_advertised",
            }));
            None
        }
        Err(error) => {
            diagnostics.push(json!({
                "format": kind.as_str(),
                "available": false,
                "reason": "download_failed",
                "error_code": error.code,
            }));
            if first_error.is_none() {
                *first_error = Some(error);
            }
            None
        }
    }
}

fn validated_optional_lyric_url(value: &str, format: &str) -> Result<Option<String>> {
    if value.trim().is_empty() {
        return Ok(None);
    }
    normalize_media_url(value).map(Some).ok_or_else(|| {
        migu_upstream_error(format!(
            "Migu {format} lyric returned an untrusted resource URL"
        ))
    })
}

fn decode_lyric_text(bytes: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| migu_upstream_error("Migu lyric response was not valid UTF-8"))?;
    let text = text
        .strip_prefix('\u{feff}')
        .unwrap_or(text)
        .trim_end_matches('\0');
    validate_lyric_text(text)?;
    Ok(text.to_owned())
}

fn decrypt_mrc(bytes: &[u8]) -> Result<String> {
    let encoded = std::str::from_utf8(bytes)
        .map_err(|_| migu_upstream_error("Migu MRC response was not ASCII hexadecimal"))?
        .trim();
    if encoded.starts_with('[') {
        validate_mrc(encoded)?;
        return Ok(encoded.to_owned());
    }
    if encoded.len() < 32
        || encoded.len() % 16 != 0
        || !encoded.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(migu_upstream_error(
            "Migu MRC response had an invalid encrypted shape",
        ));
    }
    let mut words = encoded
        .as_bytes()
        .chunks_exact(16)
        .map(|chunk| {
            let chunk = std::str::from_utf8(chunk).map_err(|_| {
                migu_upstream_error("Migu MRC response contained invalid hexadecimal")
            })?;
            u64::from_str_radix(chunk, 16)
                .map(|value| value as i64)
                .map_err(|_| migu_upstream_error("Migu MRC response contained invalid hexadecimal"))
        })
        .collect::<Result<Vec<_>>>()?;
    decrypt_mrc_words(&mut words);
    let mut utf16 = Vec::with_capacity(words.len().saturating_mul(4));
    for word in words {
        let bytes = (word as u64).to_le_bytes();
        for chunk in bytes.chunks_exact(2) {
            utf16.push(u16::from_le_bytes([chunk[0], chunk[1]]));
        }
    }
    while utf16.last() == Some(&0) {
        utf16.pop();
    }
    let text = String::from_utf16(&utf16)
        .map_err(|_| migu_upstream_error("Migu MRC decrypted to invalid UTF-16LE"))?;
    validate_mrc(&text)?;
    Ok(text)
}

fn decrypt_mrc_words(words: &mut [i64]) {
    if words.is_empty() {
        return;
    }
    let length = words.len();
    let rounds = 6_i64.wrapping_add(52_i64.wrapping_div(length as i64));
    let mut sum = rounds.wrapping_mul(MRC_DELTA);
    let mut current = words[0];
    while sum != 0 {
        let selector = ((sum >> 2) & 3) as usize;
        for index in (1..length).rev() {
            let previous = words[index - 1];
            current = words[index].wrapping_sub(mrc_mix(
                previous,
                current,
                sum,
                MRC_KEY[(index & 3) ^ selector],
            ));
            words[index] = current;
        }
        let previous = words[length - 1];
        current = words[0].wrapping_sub(mrc_mix(previous, current, sum, MRC_KEY[selector]));
        words[0] = current;
        sum = sum.wrapping_sub(MRC_DELTA);
    }
}

fn mrc_mix(previous: i64, current: i64, sum: i64, key: i64) -> i64 {
    let identity = (current ^ sum).wrapping_add(previous ^ key);
    let shifts = ((previous >> 5) ^ current.wrapping_shl(2))
        .wrapping_add((current >> 3) ^ previous.wrapping_shl(4));
    identity ^ shifts
}

fn validate_mrc(text: &str) -> Result<()> {
    validate_lyric_text(text)?;
    let mut timed_lines = 0_usize;
    let mut word_timings = 0_usize;
    for line in text.lines() {
        if let Some((_, _, payload)) = parse_mrc_line(line)? {
            timed_lines = timed_lines.saturating_add(1);
            word_timings =
                word_timings.saturating_add(count_mrc_word_timings(payload).ok_or_else(|| {
                    migu_upstream_error("Migu MRC contained a malformed word timing")
                })?);
        }
    }
    if timed_lines == 0 || word_timings == 0 {
        return Err(migu_upstream_error(
            "Migu MRC did not contain word-synchronized lyric lines",
        ));
    }
    Ok(())
}

fn mrc_to_lrc(text: &str) -> Result<String> {
    let mut lines = Vec::new();
    for line in text.lines() {
        let Some((start_ms, _, payload)) = parse_mrc_line(line)? else {
            continue;
        };
        let words = strip_mrc_word_timings(payload)?;
        let minutes = start_ms / 60_000;
        let seconds = start_ms % 60_000 / 1_000;
        let milliseconds = start_ms % 1_000;
        lines.push(format!(
            "[{minutes:02}:{seconds:02}.{milliseconds:03}]{words}"
        ));
    }
    if lines.is_empty() {
        return Err(migu_upstream_error(
            "Migu MRC did not contain line-synchronized lyrics",
        ));
    }
    Ok(lines.join("\n"))
}

fn parse_mrc_line(line: &str) -> Result<Option<(u64, u64, &str)>> {
    let line = line.trim_start();
    let Some(rest) = line.strip_prefix('[') else {
        return Ok(None);
    };
    let Some((header, payload)) = rest.split_once(']') else {
        return Ok(None);
    };
    let Some((start, duration)) = header.split_once(',') else {
        return Ok(None);
    };
    if start.is_empty()
        || duration.is_empty()
        || !start.bytes().all(|byte| byte.is_ascii_digit())
        || !duration.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Ok(None);
    }
    let start = start
        .parse::<u64>()
        .map_err(|_| migu_upstream_error("Migu MRC line start overflowed"))?;
    let duration = duration
        .parse::<u64>()
        .map_err(|_| migu_upstream_error("Migu MRC line duration overflowed"))?;
    Ok(Some((start, duration, payload)))
}

fn count_mrc_word_timings(payload: &str) -> Option<usize> {
    let mut rest = payload;
    let mut count = 0_usize;
    while let Some(open) = rest.find('(') {
        let after_open = &rest[open + 1..];
        let close = after_open.find(')')?;
        let marker = &after_open[..close];
        if is_mrc_word_timing(marker) {
            count = count.saturating_add(1);
        }
        rest = &after_open[close + 1..];
    }
    Some(count)
}

fn strip_mrc_word_timings(payload: &str) -> Result<String> {
    let mut output = String::with_capacity(payload.len());
    let mut rest = payload;
    while let Some(open) = rest.find('(') {
        output.push_str(&rest[..open]);
        let after_open = &rest[open + 1..];
        let Some(close) = after_open.find(')') else {
            return Err(migu_upstream_error(
                "Migu MRC contained an unterminated word timing",
            ));
        };
        let marker = &after_open[..close];
        if !is_mrc_word_timing(marker) {
            output.push('(');
            output.push_str(marker);
            output.push(')');
        }
        rest = &after_open[close + 1..];
    }
    output.push_str(rest);
    Ok(output)
}

fn is_mrc_word_timing(value: &str) -> bool {
    value.split_once(',').is_some_and(|(start, duration)| {
        !start.is_empty()
            && !duration.is_empty()
            && start.bytes().all(|byte| byte.is_ascii_digit())
            && duration.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn validate_lyric_text(text: &str) -> Result<()> {
    if text.trim().is_empty() {
        return Err(migu_upstream_error("Migu lyric response was empty"));
    }
    if text
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(migu_upstream_error(
            "Migu lyric response contained unsupported control characters",
        ));
    }
    Ok(())
}

fn map_resource_track(resource: MiguResource) -> Result<Track> {
    let content_id = canonical_platform_id(&resource.content_id)
        .ok_or_else(|| migu_upstream_error("Migu resource detail omitted a stable content ID"))?;
    if resource.resource_type != "2" {
        return Err(migu_upstream_error(
            "Migu resource detail returned an incompatible resource type",
        ));
    }
    if canonical_platform_id(&resource.copyright_id).is_none() {
        return Err(migu_upstream_error(
            "Migu resource detail omitted a valid copyright ID",
        ));
    }
    let name = validated_name(&resource.song_name)
        .ok_or_else(|| migu_upstream_error("Migu resource detail omitted a track name"))?;
    let resource_ref = ResourceRef::new(Platform::Migu, content_id.to_owned())
        .map_err(|_| migu_upstream_error("Migu resource detail returned an invalid identity"))?;
    let mut track = Track::new(resource_ref, name);
    push_alias(&mut track.aliases, &track.name, &resource.song_alias_name);
    push_alias(&mut track.aliases, &track.name, &resource.translate_name);
    track.artists = map_resource_artists(&resource);
    track.album = map_resource_album(&resource);
    track.duration_ms = parse_duration_text(&resource.length)?;
    track.mv_ref = resource
        .related_songs
        .iter()
        .find(|related| related.resource_type == "D")
        .and_then(|related| canonical_platform_id(&related.product_id))
        .and_then(|id| ResourceRef::new(Platform::Migu, id.to_owned()).ok());
    track.available_qualities = mapped_rate_qualities(
        resource
            .rate_formats
            .iter()
            .chain(&resource.new_rate_formats),
    );
    if resource.valid_status == Some(false) || resource.material_valid_status == Some(false) {
        track.playable = Some(false);
    }

    track
        .extensions
        .insert("backend".to_owned(), json!("resourceinfo_v1"));
    insert_nonempty(
        &mut track.extensions,
        "copyright_id",
        &resource.copyright_id,
    );
    insert_nonempty(&mut track.extensions, "song_id", &resource.song_id);
    track
        .extensions
        .insert("resource_type".to_owned(), json!(resource.resource_type));
    insert_nonempty(&mut track.extensions, "song_type", &resource.song_type);
    insert_nonempty(
        &mut track.extensions,
        "invalidate_date",
        &resource.invalidate_date,
    );
    insert_nonempty(
        &mut track.extensions,
        "track_number",
        &resource.track_number,
    );
    insert_nonempty(&mut track.extensions, "disc", &resource.disc);
    insert_nonempty(
        &mut track.extensions,
        "tone_control",
        &resource.tone_control,
    );
    insert_nonempty(
        &mut track.extensions,
        "scope_of_copyright",
        &resource.scope_ofcopyright,
    );
    insert_nonempty(
        &mut track.extensions,
        "product_authorize_usage",
        &resource.product_authorize_usage,
    );
    insert_nonempty(&mut track.extensions, "audio_book", &resource.audio_book);
    insert_nonempty(
        &mut track.extensions,
        "digital_album_id",
        &resource.dalbum_id,
    );
    insert_nonempty(
        &mut track.extensions,
        "song_description",
        &resource.song_descs,
    );
    insert_optional_string(
        &mut track.extensions,
        "landscape_image_url",
        normalize_media_url(&resource.landscap_img),
    );
    insert_optional_string(
        &mut track.extensions,
        "action_image_url",
        normalize_media_url(&resource.action_img_url),
    );
    insert_safe_media_url(&mut track.extensions, "lrc_url", &resource.lrc_url);
    insert_safe_media_url(&mut track.extensions, "mrc_url", &resource.mrc_url);
    insert_safe_media_url(&mut track.extensions, "trc_url", &resource.trc_url);
    if let Some(value) = resource
        .chorus_start_time
        .as_ref()
        .and_then(FlexibleU64::get)
    {
        track
            .extensions
            .insert("chorus_start_ms".to_owned(), json!(value));
    }

    let rights = map_resource_rights(&resource);
    track.extensions.insert("rights".to_owned(), json!(rights));
    let rate_formats = bounded_rate_formats(&resource.rate_formats);
    if !rate_formats.is_empty() {
        track
            .extensions
            .insert("rate_formats".to_owned(), json!(rate_formats));
    }
    let new_rate_formats = bounded_rate_formats(&resource.new_rate_formats);
    if !new_rate_formats.is_empty() {
        track
            .extensions
            .insert("new_rate_formats".to_owned(), json!(new_rate_formats));
    }
    let related_resources = bounded_related_resources(&resource.related_songs);
    if !related_resources.is_empty() {
        track
            .extensions
            .insert("related_resources".to_owned(), json!(related_resources));
    }
    let tags = bounded_resource_tags(&resource.tag_list);
    if !tags.is_empty() {
        track.extensions.insert("tags".to_owned(), json!(tags));
    }
    if let Some(statistics) = resource.op_num_item.as_ref().map(bounded_statistics) {
        track
            .extensions
            .insert("statistics".to_owned(), json!(statistics));
    }
    let originals = bounded_original_tracks(&resource.originals);
    if !originals.is_empty() {
        track
            .extensions
            .insert("originals".to_owned(), json!(originals));
    }
    insert_resource_flags(&mut track.extensions, &resource);
    insert_string_list(&mut track.extensions, "show_tags", resource.show_tag);
    Ok(track)
}

fn map_resource_artists(resource: &MiguResource) -> Vec<ArtistSummary> {
    let mut artists = resource
        .artists
        .iter()
        .filter_map(|artist| {
            Some(ArtistSummary {
                resource_ref: canonical_platform_id(&artist.id)
                    .and_then(|id| ResourceRef::new(Platform::Migu, id.to_owned()).ok()),
                name: validated_name(&artist.name)?.to_owned(),
            })
        })
        .collect::<Vec<_>>();
    if artists.is_empty()
        && let Some(name) = validated_name(&resource.singer)
    {
        artists.push(ArtistSummary {
            resource_ref: canonical_platform_id(&resource.singer_id)
                .and_then(|id| ResourceRef::new(Platform::Migu, id.to_owned()).ok()),
            name: name.to_owned(),
        });
    }
    artists
}

fn map_resource_album(resource: &MiguResource) -> Option<AlbumSummary> {
    let name = validated_name(&resource.album)?;
    let resource_ref = canonical_platform_id(&resource.album_id)
        .and_then(|id| ResourceRef::new(Platform::Migu, id.to_owned()).ok());
    let cover_url = preferred_image(&resource.album_imgs);
    Some(AlbumSummary {
        resource_ref,
        name: name.to_owned(),
        cover_url,
    })
}

fn preferred_image(images: &[MiguImage]) -> Option<String> {
    ["03", "02", "01"]
        .into_iter()
        .find_map(|kind| {
            images
                .iter()
                .find(|image| image.img_size_type == kind)
                .and_then(normalized_image)
        })
        .or_else(|| images.iter().find_map(normalized_image))
}

fn normalized_image(image: &MiguImage) -> Option<String> {
    [&image.webp_img, &image.img, &image.img_ori]
        .into_iter()
        .find_map(|value| normalize_media_url(value))
}

fn parse_duration_text(value: &str) -> Result<Option<u64>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let parts = value
        .split(':')
        .map(|part| part.parse::<u64>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| migu_upstream_error("Migu resource detail returned an invalid duration"))?;
    let seconds = match parts.as_slice() {
        [minutes, seconds] if *seconds < 60 => minutes
            .checked_mul(60)
            .and_then(|value| value.checked_add(*seconds)),
        [hours, minutes, seconds] if *minutes < 60 && *seconds < 60 => hours
            .checked_mul(3_600)
            .and_then(|value| {
                minutes
                    .checked_mul(60)
                    .and_then(|part| value.checked_add(part))
            })
            .and_then(|value| value.checked_add(*seconds)),
        _ => None,
    }
    .ok_or_else(|| migu_upstream_error("Migu resource detail returned an invalid duration"))?;
    seconds
        .checked_mul(1_000)
        .map(Some)
        .ok_or_else(|| migu_upstream_error("Migu resource detail duration overflowed"))
}

fn push_alias(aliases: &mut Vec<String>, name: &str, candidate: &str) {
    if let Some(candidate) = validated_name(candidate)
        && candidate != name
        && !aliases.iter().any(|alias| alias == candidate)
    {
        aliases.push(candidate.to_owned());
    }
}

fn mapped_rate_qualities<'a>(
    formats: impl IntoIterator<Item = &'a MiguRateFormat>,
) -> Vec<Quality> {
    let mut qualities = Vec::new();
    for format in formats {
        let quality = match format.format_type.as_str() {
            "LQ" => Some(Quality::Low),
            "PQ" => Some(Quality::Standard),
            "HQ" => Some(Quality::High),
            "SQ" => Some(Quality::Lossless),
            "ZQ" | "ZQ24" => Some(Quality::Hires),
            _ => None,
        };
        if let Some(quality) = quality
            && !qualities.contains(&quality)
        {
            qualities.push(quality);
        }
    }
    qualities
}

fn map_song(song: MiguSong) -> Result<Track> {
    let content_id = canonical_platform_id(&song.content_id)
        .ok_or_else(|| migu_upstream_error("Migu search result omitted a stable content ID"))?;
    if song.resource_type != "2" {
        return Err(migu_upstream_error(
            "Migu song search returned an incompatible resource type",
        ));
    }
    let name = validated_name(&song.song_name)
        .ok_or_else(|| migu_upstream_error("Migu search result omitted a track name"))?;
    let resource_ref = ResourceRef::new(Platform::Migu, content_id.to_owned())
        .map_err(|_| migu_upstream_error("Migu search returned an invalid track identity"))?;
    let mut track = Track::new(resource_ref, name);
    track.artists = song.singer_list.iter().filter_map(map_artist).collect();
    track.album = map_album(&song);
    track.duration_ms = song
        .duration
        .as_ref()
        .and_then(FlexibleU64::get)
        .and_then(|seconds| seconds.checked_mul(1_000));
    track.mv_ref = canonical_platform_id(&song.mv_id)
        .and_then(|id| ResourceRef::new(Platform::Migu, id.to_owned()).ok());
    track.available_qualities = mapped_qualities(&song.audio_formats);

    let alternates = song
        .more
        .iter()
        .filter_map(map_alternate)
        .collect::<Vec<_>>();
    insert_nonempty(&mut track.extensions, "song_id", &song.song_id);
    track
        .extensions
        .insert("resource_type".to_owned(), json!(song.resource_type));
    insert_nonempty(&mut track.extensions, "copyright_id", &song.copyright_id);
    insert_optional(&mut track.extensions, "copyright_type", song.copyright_type);
    insert_optional(&mut track.extensions, "restrict_type", song.restrict_type);
    insert_optional(
        &mut track.extensions,
        "mv_copyright_type",
        song.mv_copyright_type,
    );
    insert_nonempty(&mut track.extensions, "ring_tone_id", &song.ring_tone_id);
    insert_nonempty(
        &mut track.extensions,
        "ring_copyright_id",
        &song.ring_copyright_id,
    );
    insert_nonempty(&mut track.extensions, "shock_ring_id", &song.shock_ring_id);
    insert_optional(
        &mut track.extensions,
        "has_associated_ring",
        song.has_associated_ring,
    );
    insert_nonempty(&mut track.extensions, "song_pinyin", &song.song_pinyin);
    insert_nonempty(&mut track.extensions, "album_pinyin", &song.album_pinyin);
    insert_nonempty(
        &mut track.extensions,
        "play_count_display",
        &song.play_num_desc,
    );
    insert_nonempty(
        &mut track.extensions,
        "forever_listen_flag",
        &song.forever_listen_flag,
    );
    insert_optional(&mut track.extensions, "forever_listen", song.forever_listen);
    insert_nonempty(
        &mut track.extensions,
        "product_authorize_usage",
        &song.product_authorize_usage,
    );
    insert_nonempty(&mut track.extensions, "audio_book", &song.audio_book);
    if let Some(value) = song.chorus_start_time.as_ref().and_then(FlexibleU64::get) {
        track
            .extensions
            .insert("chorus_start_ms".to_owned(), json!(value));
    }
    if !song.ext.disc.trim().is_empty() {
        track
            .extensions
            .insert("disc".to_owned(), json!(song.ext.disc.trim()));
    }
    insert_string_list(&mut track.extensions, "show_tags", song.show_tags);
    insert_string_list(&mut track.extensions, "download_tags", song.download_tags);
    insert_string_list(&mut track.extensions, "highlights", song.highlights);
    let search_tags = bounded_search_tags(&song.search_tags);
    if !search_tags.is_empty() {
        track
            .extensions
            .insert("search_tags".to_owned(), json!(search_tags));
    }
    let audio_formats = bounded_audio_formats(&song.audio_formats);
    if !audio_formats.is_empty() {
        track
            .extensions
            .insert("audio_formats".to_owned(), json!(audio_formats));
    }
    if !alternates.is_empty() {
        track
            .extensions
            .insert("alternate_versions".to_owned(), json!(alternates));
    }
    insert_safe_media_url(&mut track.extensions, "lrc_url", &song.lrc_url);
    insert_safe_media_url(&mut track.extensions, "mrc_url", &song.mrc_url);
    insert_safe_media_url(
        &mut track.extensions,
        "action_image_url",
        &song.action_img_url,
    );
    Ok(track)
}

fn map_artist(singer: &MiguSinger) -> Option<ArtistSummary> {
    let name = validated_name(&singer.name)?;
    let resource_ref = canonical_platform_id(&singer.id)
        .and_then(|id| ResourceRef::new(Platform::Migu, id.to_owned()).ok());
    Some(ArtistSummary {
        resource_ref,
        name: name.to_owned(),
    })
}

fn map_album(song: &MiguSong) -> Option<AlbumSummary> {
    let name = validated_name(&song.album)?;
    let resource_ref = canonical_platform_id(&song.album_id)
        .and_then(|id| ResourceRef::new(Platform::Migu, id.to_owned()).ok());
    let cover_url = [&song.img3, &song.img2, &song.img1]
        .into_iter()
        .find_map(|value| normalize_media_url(value));
    Some(AlbumSummary {
        resource_ref,
        name: name.to_owned(),
        cover_url,
    })
}

fn map_alternate(song: &MiguSong) -> Option<MiguAlternateTrack> {
    let content_id = canonical_platform_id(&song.content_id)?;
    let name = validated_name(&song.song_name)?;
    if song.resource_type != "2" {
        return None;
    }
    Some(MiguAlternateTrack {
        content_id: content_id.to_owned(),
        song_id: canonical_platform_id(&song.song_id).map(str::to_owned),
        name: name.to_owned(),
        resource_type: song.resource_type.clone(),
        artists: song
            .singer_list
            .iter()
            .filter_map(map_alternate_singer)
            .collect(),
        album_id: canonical_platform_id(&song.album_id).map(str::to_owned),
        album: validated_name(&song.album).map(str::to_owned),
        duration_seconds: song.duration.as_ref().and_then(FlexibleU64::get),
        copyright_id: canonical_platform_id(&song.copyright_id).map(str::to_owned),
        copyright_type: song.copyright_type,
        restrict_type: song.restrict_type,
        forever_listen: song.forever_listen,
        audio_formats: bounded_audio_formats(&song.audio_formats),
        cover_url: [&song.img3, &song.img2, &song.img1]
            .into_iter()
            .find_map(|value| normalize_media_url(value)),
        lrc_url: normalize_media_url(&song.lrc_url),
        mrc_url: normalize_media_url(&song.mrc_url),
        highlights: bounded_string_list(&song.highlights, 32, 256),
    })
}

fn map_alternate_singer(singer: &MiguSinger) -> Option<MiguAlternateSinger> {
    Some(MiguAlternateSinger {
        id: canonical_platform_id(&singer.id).map(str::to_owned),
        name: validated_name(&singer.name)?.to_owned(),
        name_spelling: validated_name(&singer.name_spelling).map(str::to_owned),
        image_url: normalize_media_url(&singer.img),
    })
}

fn mapped_qualities(formats: &[MiguAudioFormat]) -> Vec<Quality> {
    let mut qualities = Vec::new();
    for format in formats {
        let quality = match format.format_type.as_str() {
            "PQ" => Some(Quality::Standard),
            "HQ" => Some(Quality::High),
            "SQ" => Some(Quality::Lossless),
            "ZQ" | "ZQ24" => Some(Quality::Hires),
            _ => None,
        };
        if let Some(quality) = quality
            && !qualities.contains(&quality)
        {
            qualities.push(quality);
        }
    }
    qualities
}

fn normalize_media_url(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let url = if value.starts_with('/') {
        if !value.starts_with("/data/oss/") {
            return None;
        }
        Url::parse(&format!("https://{MEDIA_HOST}{value}")).ok()?
    } else {
        Url::parse(value).ok()?
    };
    if url.scheme() != "https"
        || url.host_str() != Some(MEDIA_HOST)
        || url.username() != ""
        || url.password().is_some()
        || !matches!(url.port(), None | Some(443))
        || !url.path().starts_with("/data/oss/")
    {
        return None;
    }
    Some(url.into())
}

fn canonical_platform_id(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 64
        && value.bytes().all(|byte| byte.is_ascii_alphanumeric()))
    .then_some(value)
}

fn validated_name(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()
        && value.len() <= 2_048
        && !value.chars().any(|character| character.is_control()))
    .then_some(value)
}

fn nonempty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn bounded_text(value: &str, max_bytes: usize) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .scan(0_usize, |used, character| {
            let next = used.saturating_add(character.len_utf8());
            (next <= max_bytes).then(|| {
                *used = next;
                character
            })
        })
        .collect()
}

fn bounded_string_list(values: &[String], max_items: usize, max_bytes: usize) -> Vec<String> {
    values
        .iter()
        .take(max_items)
        .map(|value| bounded_text(value, max_bytes))
        .filter(|value| !value.is_empty())
        .collect()
}

fn bounded_conditions(mut values: Vec<MiguSearchCondition>) -> Vec<MiguSearchCondition> {
    values.truncate(16);
    for condition in &mut values {
        condition.id = bounded_text(&condition.id, 128);
        condition.title = bounded_text(&condition.title, 256);
        condition.kind = bounded_text(&condition.kind, 128);
        condition.datas.truncate(32);
        for value in &mut condition.datas {
            value.title = bounded_text(&value.title, 256);
            value.condition_type = bounded_text(&value.condition_type, 128);
        }
    }
    values
}

fn bounded_search_tags(values: &[MiguSearchTag]) -> Vec<MiguSearchTag> {
    values
        .iter()
        .take(64)
        .map(|value| MiguSearchTag {
            name: bounded_text(&value.name, 256),
            kind: bounded_text(&value.kind, 128),
        })
        .filter(|value| !value.name.is_empty() || !value.kind.is_empty())
        .collect()
}

fn bounded_audio_formats(values: &[MiguAudioFormat]) -> Vec<MiguAudioFormat> {
    values
        .iter()
        .take(32)
        .map(|value| MiguAudioFormat {
            resource_type: bounded_text(&value.resource_type, 32),
            format_type: bounded_text(&value.format_type, 32),
            show_tags: bounded_string_list(&value.show_tags, 32, 128),
            isize: value.isize.clone(),
            asize: value.asize.clone(),
            iformat: bounded_text(&value.iformat, 64),
            aformat: bounded_text(&value.aformat, 64),
        })
        .collect()
}

fn map_resource_rights(resource: &MiguResource) -> MiguResourceRights {
    let code_rates = resource
        .code_rate
        .iter()
        .take(32)
        .filter_map(|(key, value)| {
            Some((
                canonical_platform_id(key)?.to_owned(),
                MiguCodeRateRights {
                    code_rate_charge_auditions: bounded_text(&value.code_rate_charge_auditions, 32),
                    code_rate_auditions_length: value.code_rate_auditions_length.clone(),
                    code_rate_charge_auditions_type: bounded_text(
                        &value.code_rate_charge_auditions_type,
                        32,
                    ),
                    is_code_rate_download: bounded_text(&value.is_code_rate_download, 32),
                    code_rate_file_size: value.code_rate_file_size.clone(),
                    content_id_sq: canonical_platform_id(&value.content_id_sq)
                        .unwrap_or_default()
                        .to_owned(),
                    quality_icon: bounded_text(&value.quality_icon, 32),
                },
            ))
        })
        .collect();
    MiguResourceRights {
        valid_status: resource.valid_status,
        material_valid_status: resource.material_valid_status,
        copyright: bounded_optional(&resource.copyright, 32),
        copyright_type: bounded_optional(&resource.copyright_type, 32),
        vip_type: bounded_optional(&resource.vip_type, 32),
        vip_flag: bounded_optional(&resource.vip_flag, 32),
        vip_download: bounded_optional(&resource.vip_download, 32),
        is_download: bounded_optional(&resource.is_download, 32),
        listen_flag: bounded_optional(&resource.listen_flag, 32),
        auditions_flag: bounded_optional(&resource.auditions_flag, 32),
        auditions_type: bounded_optional(&resource.auditions_type, 32),
        charge_auditions: bounded_optional(&resource.charge_auditions, 32),
        old_charge_auditions: bounded_optional(&resource.old_charge_auditions, 32),
        auditions_length_seconds: resource
            .auditions_length
            .as_ref()
            .and_then(FlexibleU64::get),
        auditions_start_seconds: resource
            .auditions_start_time
            .as_ref()
            .and_then(FlexibleU64::get),
        top_quality: bounded_optional(&resource.top_quality, 32),
        forever_listen_flag: bounded_optional(&resource.forever_listen_flag, 32),
        forever_listen: resource.forever_listen,
        need_encrypt: bounded_optional(&resource.need_encrypt, 32),
        code_rates,
    }
}

fn bounded_rate_formats(values: &[MiguRateFormat]) -> Vec<MiguRateFormat> {
    values
        .iter()
        .take(32)
        .map(|value| MiguRateFormat {
            resource_type: bounded_text(&value.resource_type, 32),
            format_type: bounded_text(&value.format_type, 32),
            format: bounded_text(&value.format, 64),
            size: value.size.clone(),
            file_type: bounded_text(&value.file_type, 32),
            price: bounded_text(&value.price, 64),
            android_file_type: bounded_text(&value.android_file_type, 32),
            ios_file_type: bounded_text(&value.ios_file_type, 32),
            ios_size: value.ios_size.clone(),
            android_size: value.android_size.clone(),
            ios_format: bounded_text(&value.ios_format, 64),
            android_format: bounded_text(&value.android_format, 64),
            ios_accuracy_level: bounded_text(&value.ios_accuracy_level, 64),
            android_accuracy_level: bounded_text(&value.android_accuracy_level, 64),
            show_tag: bounded_string_list(&value.show_tag, 32, 128),
        })
        .collect()
}

fn bounded_related_resources(values: &[MiguRelatedResource]) -> Vec<MiguRelatedResource> {
    values
        .iter()
        .take(64)
        .filter_map(|value| {
            Some(MiguRelatedResource {
                resource_type: bounded_text(&value.resource_type, 32),
                resource_type_name: bounded_text(&value.resource_type_name, 128),
                copyright_id: canonical_platform_id(&value.copyright_id)?.to_owned(),
                product_id: canonical_platform_id(&value.product_id)?.to_owned(),
            })
        })
        .collect()
}

fn bounded_resource_tags(values: &[MiguResourceTag]) -> Vec<MiguResourceTag> {
    values
        .iter()
        .take(128)
        .filter_map(|value| {
            Some(MiguResourceTag {
                resource_type: bounded_text(&value.resource_type, 32),
                tag_id: canonical_platform_id(&value.tag_id)?.to_owned(),
                tag_name: validated_name(&value.tag_name)?.to_owned(),
            })
        })
        .collect()
}

fn bounded_statistics(value: &MiguResourceStatistics) -> MiguResourceStatistics {
    MiguResourceStatistics {
        play_num: value.play_num.clone(),
        play_num_desc: bounded_text(&value.play_num_desc, 128),
        keep_num: value.keep_num.clone(),
        keep_num_desc: bounded_text(&value.keep_num_desc, 128),
        comment_num: value.comment_num.clone(),
        comment_num_desc: bounded_text(&value.comment_num_desc, 128),
        share_num: value.share_num.clone(),
        share_num_desc: bounded_text(&value.share_num_desc, 128),
        live_play_num: value.live_play_num.clone(),
        live_play_num_desc: bounded_text(&value.live_play_num_desc, 128),
    }
}

fn bounded_original_tracks(values: &[MiguOriginalTrack]) -> Vec<MiguOriginalTrack> {
    values
        .iter()
        .take(128)
        .filter_map(|value| {
            Some(MiguOriginalTrack {
                kind: bounded_text(&value.kind, 32),
                song_id: canonical_platform_id(&value.song_id)?.to_owned(),
                song_name: validated_name(&value.song_name)?.to_owned(),
            })
        })
        .collect()
}

fn insert_resource_flags(extensions: &mut tuneweave_core::Extensions, resource: &MiguResource) {
    let flags = [
        ("is_in_digital_album", resource.is_in_d_album.as_str()),
        (
            "is_in_side_digital_album",
            resource.is_in_side_dalbum.as_str(),
        ),
        ("is_in_sales_period", resource.is_in_sales_period.as_str()),
        ("has_mv", resource.has_mv.as_str()),
        ("mv_copyright", resource.mv_copyright.as_str()),
        ("first_icon", resource.first_icon.as_str()),
        ("song_icon", resource.song_icon.as_str()),
        ("first_click", resource.first_click.as_str()),
        ("pre_sale", resource.pre_sale.as_str()),
        ("is_share", resource.is_share.as_str()),
        ("is_collection", resource.is_collection.as_str()),
        ("vip_start_time", resource.vip_start_time.as_str()),
        ("vip_end_time", resource.vip_end_time.as_str()),
        ("vip_logo", resource.vip_logo.as_str()),
        ("first_publish", resource.first_publish.as_str()),
        ("first_start_time", resource.first_start_time.as_str()),
        ("first_end_time", resource.first_end_time.as_str()),
        ("is_recreate", resource.is_recreate.as_str()),
    ];
    let values = flags
        .into_iter()
        .filter_map(|(key, value)| bounded_optional(value, 64).map(|value| (key, value)))
        .collect::<BTreeMap<_, _>>();
    if !values.is_empty() {
        extensions.insert("resource_flags".to_owned(), json!(values));
    }
    insert_optional(
        extensions,
        "has_associated_ring",
        resource.has_associated_ring,
    );
}

fn bounded_optional(value: &str, max_bytes: usize) -> Option<String> {
    let value = bounded_text(value, max_bytes);
    (!value.is_empty()).then_some(value)
}

fn insert_nonempty(extensions: &mut tuneweave_core::Extensions, key: &str, value: &str) {
    if let Some(value) = nonempty(value) {
        extensions.insert(key.to_owned(), json!(bounded_text(value, 2_048)));
    }
}

fn insert_optional_string(
    extensions: &mut tuneweave_core::Extensions,
    key: &str,
    value: Option<String>,
) {
    if let Some(value) = value {
        extensions.insert(key.to_owned(), json!(value));
    }
}

fn insert_optional<T: Serialize>(
    extensions: &mut tuneweave_core::Extensions,
    key: &str,
    value: Option<T>,
) {
    if let Some(value) = value {
        extensions.insert(key.to_owned(), json!(value));
    }
}

fn insert_string_list(extensions: &mut tuneweave_core::Extensions, key: &str, values: Vec<String>) {
    let values = bounded_string_list(&values, 64, 256);
    if !values.is_empty() {
        extensions.insert(key.to_owned(), json!(values));
    }
}

fn insert_safe_media_url(extensions: &mut tuneweave_core::Extensions, key: &str, value: &str) {
    if let Some(url) = normalize_media_url(value) {
        extensions.insert(key.to_owned(), json!(url));
    }
}

async fn read_bounded_response(response: reqwest::Response, operation: &str) -> Result<Vec<u8>> {
    read_bounded_response_with_limit(response, operation, MAX_API_RESPONSE_BYTES).await
}

async fn read_bounded_response_with_limit(
    mut response: reqwest::Response,
    operation: &str,
    limit: u64,
) -> Result<Vec<u8>> {
    let status = response.status();
    if !status.is_success() {
        return Err(migu_http_error(status));
    }
    if response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > limit)
    {
        return Err(migu_upstream_error(format!(
            "{operation} response exceeded the size limit"
        )));
    }
    let max_size = usize::try_from(limit).unwrap_or(usize::MAX);
    let initial_capacity = response
        .content_length()
        .and_then(|length| usize::try_from(length).ok())
        .unwrap_or_default()
        .min(max_size);
    let mut bytes = Vec::with_capacity(initial_capacity);
    while let Some(chunk) = response.chunk().await.map_err(migu_network_error)? {
        if bytes.len().saturating_add(chunk.len()) > max_size {
            return Err(migu_upstream_error(format!(
                "{operation} response exceeded the size limit"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn migu_network_error(error: reqwest::Error) -> TuneWeaveError {
    let code = if error.is_timeout() {
        ErrorCode::UpstreamTimeout
    } else {
        ErrorCode::UpstreamError
    };
    TuneWeaveError::new(code, "Migu API request failed")
        .with_platform(Platform::Migu)
        .retryable(true)
}

fn migu_http_error(status: StatusCode) -> TuneWeaveError {
    let code = if status == StatusCode::TOO_MANY_REQUESTS {
        ErrorCode::RateLimited
    } else {
        ErrorCode::UpstreamError
    };
    TuneWeaveError::new(code, format!("Migu API returned HTTP {status}"))
        .with_platform(Platform::Migu)
        .retryable(status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS)
}

fn migu_invalid_request(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Migu)
}

fn migu_upstream_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message).with_platform(Platform::Migu)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_RESPONSE: &str = r#"{
      "code":"000000",
      "info":"操作成功",
      "data":{
        "hasNext":true,
        "seq":"safe-sequence",
        "conditions":[{"id":"1","title":"默认","type":"order","datas":[{"title":"最新","conditionType":"1"}]}],
        "items":[{"song":{
          "resourceType":"2",
          "contentId":"600908000007288315",
          "songId":"1004202180",
          "songName":"告白气球",
          "mvId":"600906000000389715",
          "audioFormats":[
            {"resourceType":"2","formatType":"PQ","isize":"3450883","aformat":"020007"},
            {"resourceType":"2","formatType":"HQ","asize":8626889,"aformat":"020010"},
            {"resourceType":"E","formatType":"SQ","aformat":"011002"},
            {"resourceType":"2","formatType":"ZQ24","aformat":"011005"},
            {"resourceType":"2","formatType":"AV3A","aformat":"020041"}
          ],
          "duration":"215",
          "copyrightId":"60054704028",
          "copyrightType":1,
          "restrictType":1,
          "albumId":"1003767159",
          "album":"周杰伦的床边故事",
          "img3":"/data/oss/resource/cover.webp",
          "singerList":[{"id":"112","name":"周杰伦","img":"https://d.musicapp.migu.cn/data/oss/resource/artist.webp"}],
          "foreverListenFlag":"0",
          "foreverListen":false,
          "lrcUrl":"https://evil.example/lrc",
          "mrcUrl":"https://d.musicapp.migu.cn/data/oss/resource/word",
          "more":[{
            "resourceType":"2",
            "contentId":"600913000007163490",
            "songId":"1125329696",
            "songName":"告白气球 (Live)",
            "duration":264,
            "audioFormats":[{"resourceType":"2","formatType":"AV3A","aformat":"020041"}],
            "singerList":[{"id":"112","name":"周杰伦"}],
            "albumId":"1125329686",
            "album":"巡回演唱会",
            "img3":"/data/oss/resource/live.webp",
            "highlights":["周杰伦"]
          }]
        }}]
      }
    }"#;

    const RESOURCE_RESPONSE: &str = r#"{
      "code":"000000",
      "resource":[{
        "resourceType":"2",
        "copyrightId":"60054704028",
        "contentId":"600908000007288315",
        "songId":"1004202180",
        "songName":"告白气球",
        "singerId":"112",
        "singer":"周杰伦",
        "artists":[{"id":"112","name":"周杰伦","nameSpelling":"zhoujielun"}],
        "albumId":"1003767159",
        "album":"周杰伦的床边故事",
        "albumImgs":[
          {"imgSizeType":"01","img":"https://evil.example/cover"},
          {"imgSizeType":"03","img":"https://d.musicapp.migu.cn/data/oss/resource/cover.webp"}
        ],
        "opNumItem":{"playNum":357169016,"playNumDesc":"3.6亿","keepNum":"913608","keepNumDesc":"91.4万"},
        "toneControl":"111100",
        "relatedSongs":[
          {"resourceType":"E","resourceTypeName":"无损","copyrightId":"60054704028","productId":"600908000007288315"},
          {"resourceType":"D","resourceTypeName":"视频","copyrightId":"600547Y0291","productId":"600906000000389715"}
        ],
        "rateFormats":[
          {"resourceType":"3","formatType":"LQ","format":"000019","size":"1725628","fileType":"mp3"},
          {"resourceType":"2","formatType":"PQ","format":"020007","size":"3450883","fileType":"mp3"},
          {"resourceType":"2","formatType":"AV3A","format":"020041","size":"22426036","fileType":"m4a"}
        ],
        "newRateFormats":[
          {"resourceType":"2","formatType":"PQ","format":"020007","size":"3450883","fileType":"mp3"},
          {"resourceType":"2","formatType":"HQ","format":"020010","size":"8626889","fileType":"mp3"},
          {"resourceType":"E","formatType":"SQ","format":"011002","size":"25117488","androidFileType":"flac"},
          {"resourceType":"2","formatType":"ZQ","format":"011005","androidSize":"33943553","androidFileType":"flac"},
          {"resourceType":"2","formatType":"AV3A","format":"020041","size":"22426036","fileType":"m4a"}
        ],
        "lrcUrl":"https://d.musicapp.migu.cn/data/oss/resource/lrc",
        "mrcUrl":"https://d.musicapp.migu.cn/data/oss/resource/mrc",
        "tagList":[{"resourceType":"2034","tagId":"1000001672","tagName":"流行"}],
        "copyright":"1",
        "validStatus":true,
        "materialValidStatus":true,
        "songAliasName":"Love Confession",
        "translateName":"Love Confession",
        "songType":"01",
        "invalidateDate":"2030-12-31",
        "trackNumber":"8",
        "disc":"Disc 1",
        "vipType":"1",
        "auditionsType":"04",
        "chargeAuditions":"1",
        "auditionsLength":60,
        "auditionsStartTime":"65",
        "codeRate":{"PQ":{"codeRateChargeAuditions":"1","codeRateAuditionsLength":60,"isCodeRateDownload":"0","codeRateFileSize":"3450883"}},
        "vipFlag":"1",
        "isDownload":"0",
        "copyrightType":"1",
        "hasMv":"1",
        "topQuality":"SQ",
        "length":"00:03:35",
        "auditionsFlag":"9",
        "listenFlag":"11",
        "showTag":["vip","isq"],
        "foreverListenFlag":"0",
        "foreverListen":false,
        "chorusStartTime":"149040",
        "originals":[{"type":"9","songId":"1125329696","songName":"告白气球 (Live)"}]
      }]
    }"#;

    #[test]
    fn search_mapping_preserves_stable_identity_metadata_and_exact_formats() {
        let page = parse_search_response(SEARCH_RESPONSE.as_bytes()).expect("parse search");
        assert!(page.has_next);
        assert_eq!(page.sequence.as_deref(), Some("safe-sequence"));
        assert_eq!(page.conditions.len(), 1);
        let track = &page.tracks[0];
        assert_eq!(track.resource_ref.to_string(), "migu:600908000007288315");
        assert_eq!(track.name, "告白气球");
        assert_eq!(track.duration_ms, Some(215_000));
        assert_eq!(track.artists[0].name, "周杰伦");
        assert_eq!(
            track
                .album
                .as_ref()
                .and_then(|album| album.cover_url.as_deref()),
            Some("https://d.musicapp.migu.cn/data/oss/resource/cover.webp")
        );
        assert_eq!(
            track.available_qualities,
            vec![
                Quality::Standard,
                Quality::High,
                Quality::Lossless,
                Quality::Hires
            ]
        );
        let formats = track.extensions["audio_formats"]
            .as_array()
            .expect("audio formats");
        assert_eq!(formats[4]["formatType"], "AV3A");
        assert!(!track.extensions.contains_key("lrc_url"));
        assert_eq!(
            track.extensions["mrc_url"],
            "https://d.musicapp.migu.cn/data/oss/resource/word"
        );
        assert_eq!(
            track.extensions["alternate_versions"][0]["content_id"],
            "600913000007163490"
        );
        assert_eq!(
            track.extensions["alternate_versions"][0]["audio_formats"][0]["formatType"],
            "AV3A"
        );
        assert_eq!(track.playable, None);
    }

    #[test]
    fn search_rejects_business_errors_and_malformed_song_identity() {
        let rejected = br#"{"code":"100001","info":"failed","data":null}"#;
        let error = parse_search_response(rejected).expect_err("business error");
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert_eq!(error.details["platform_code"], "100001");

        for bad in [
            br#"{"code":"000000","data":{"items":[{"song":{"resourceType":"2","contentId":"","songName":"name"}}]}}"#.as_slice(),
            br#"{"code":"000000","data":{"items":[{"song":{"resourceType":"V","contentId":"123","songName":"name"}}]}}"#.as_slice(),
            br#"{"code":"000000","data":{"items":[{"song":{"resourceType":"2","contentId":"123","songName":""}}]}}"#.as_slice(),
        ] {
            assert!(parse_search_response(bad).is_err());
        }
    }

    #[test]
    fn resource_detail_maps_identity_aliases_formats_rights_and_related_mv() {
        let track = parse_resource_response(RESOURCE_RESPONSE.as_bytes(), "600908000007288315")
            .expect("parse resource detail");
        assert_eq!(track.resource_ref.to_string(), "migu:600908000007288315");
        assert_eq!(track.name, "告白气球");
        assert_eq!(track.aliases, vec!["Love Confession"]);
        assert_eq!(
            track.artists[0]
                .resource_ref
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("migu:112")
        );
        assert_eq!(
            track
                .album
                .as_ref()
                .and_then(|album| album.cover_url.as_deref()),
            Some("https://d.musicapp.migu.cn/data/oss/resource/cover.webp")
        );
        assert_eq!(track.duration_ms, Some(215_000));
        assert_eq!(
            track.mv_ref.as_ref().map(ToString::to_string).as_deref(),
            Some("migu:600906000000389715")
        );
        assert_eq!(
            track.available_qualities,
            vec![
                Quality::Low,
                Quality::Standard,
                Quality::High,
                Quality::Lossless,
                Quality::Hires
            ]
        );
        assert_eq!(track.playable, None);
        assert_eq!(track.extensions["backend"], "resourceinfo_v1");
        assert_eq!(
            track.extensions["new_rate_formats"][4]["formatType"],
            "AV3A"
        );
        assert_eq!(track.extensions["rights"]["auditions_start_seconds"], 65);
        assert_eq!(track.extensions["statistics"]["playNum"], 357169016);
        assert_eq!(track.extensions["originals"][0]["songId"], "1125329696");
    }

    #[test]
    fn resource_detail_rejects_missing_duplicate_and_mismatched_identities() {
        let missing = br#"{"code":"000000","resource":[]}"#;
        let error =
            parse_resource_response(missing, "600908000007288315").expect_err("missing track");
        assert_eq!(error.code, ErrorCode::ResourceNotFound);

        let mismatch = RESOURCE_RESPONSE.replace(
            "\"contentId\":\"600908000007288315\"",
            "\"contentId\":\"600908000007288316\"",
        );
        assert!(parse_resource_response(mismatch.as_bytes(), "600908000007288315").is_err());

        let duplicate = RESOURCE_RESPONSE.replace(
            "\"resource\":[{",
            "\"resource\":[{\"resourceType\":\"2\",\"contentId\":\"600908000007288315\",\"songName\":\"first\"},{",
        );
        assert!(parse_resource_response(duplicate.as_bytes(), "600908000007288315").is_err());
    }

    #[test]
    fn resource_detail_only_marks_explicit_invalid_material_unplayable() {
        let invalid = RESOURCE_RESPONSE.replace(
            "\"materialValidStatus\":true",
            "\"materialValidStatus\":false",
        );
        let track = parse_resource_response(invalid.as_bytes(), "600908000007288315")
            .expect("parse invalid material");
        assert_eq!(track.playable, Some(false));
    }

    #[test]
    fn mrc_parser_preserves_word_timing_and_derives_plain_lrc() {
        let mrc = "[1000,2000](1000,500)你(1500,500)好\n\
                   [3000,1000](3000,1000)世界";
        assert!(validate_mrc(mrc).is_ok());
        assert_eq!(
            mrc_to_lrc(mrc).expect("derive LRC"),
            "[00:01.000]你好\n[00:03.000]世界"
        );
        assert_eq!(
            decrypt_mrc(mrc.as_bytes()).expect("accept plaintext MRC"),
            mrc
        );
        assert!(validate_mrc("[1000,2000]line only").is_err());
        assert!(decrypt_mrc(b"not encrypted mrc").is_err());
    }

    #[test]
    fn lyric_mapping_keeps_mrc_primary_without_discarding_plain_or_translation() {
        let mrc = "[1000,2000](1000,500)你(1500,500)好";
        let lyrics = map_lyrics(
            "600908000007288315",
            "60054704028",
            Ok(Some(DownloadedLyric {
                text: "[00:01.00]你好".to_owned(),
                content_type: Some("application/octet-stream".to_owned()),
                byte_length: 20,
            })),
            Ok(Some(DownloadedLyric {
                text: mrc.to_owned(),
                content_type: Some("application/marc".to_owned()),
                byte_length: 64,
            })),
            Ok(Some(DownloadedLyric {
                text: "[00:01.00]hello".to_owned(),
                content_type: Some("text/plain".to_owned()),
                byte_length: 20,
            })),
        )
        .expect("map lyrics");
        assert_eq!(lyrics.format, "mrc");
        assert_eq!(lyrics.plain.as_deref(), Some("[00:01.00]你好"));
        assert_eq!(lyrics.word_synced.as_deref(), Some(mrc));
        assert_eq!(lyrics.translated.as_deref(), Some("[00:01.00]hello"));
        assert_eq!(lyrics.romanized, None);
        assert_eq!(lyrics.extensions["plain_source"], "lrc");
    }

    #[test]
    fn lyric_mapping_derives_plain_from_mrc_and_tolerates_independent_lrc_failure() {
        let mrc = "[1000,2000](1000,500)你(1500,500)好";
        let lyrics = map_lyrics(
            "600908000007288315",
            "60054704028",
            Err(migu_upstream_error("LRC unavailable")),
            Ok(Some(DownloadedLyric {
                text: mrc.to_owned(),
                content_type: Some("application/marc".to_owned()),
                byte_length: 64,
            })),
            Ok(None),
        )
        .expect("map MRC fallback");
        assert_eq!(lyrics.format, "mrc");
        assert_eq!(lyrics.plain.as_deref(), Some("[00:01.000]你好"));
        assert_eq!(lyrics.extensions["plain_source"], "derived_mrc");
        assert_eq!(lyrics.extensions["downloads"][0]["available"], false);
        assert_eq!(lyrics.extensions["downloads"][1]["available"], true);
    }

    fn encrypt_public_stream_fixture(json: &str) -> Vec<u8> {
        let seed = 17_u8;
        let mut encrypted = vec![0xab, 0xcd, 0x01, seed];
        encrypted.extend(json.as_bytes().iter().enumerate().map(|(index, byte)| {
            byte.wrapping_sub(seed)
                .wrapping_add(PUBLIC_STREAM_KEY[index % PUBLIC_STREAM_KEY.len()])
        }));
        encrypted
    }

    fn public_audio_url(extension: &str) -> String {
        format!(
            "https://freetyst.nf.migu.cn/public/product9th/product/example/audio.{extension}?\
             channelid=014X031&msisdn=&Tim=1785325050934&Key=0123456789abcdef&\
             playSessionId=0123456789abcdef0123456789abcdef"
        )
    }

    #[test]
    fn listening_rights_require_one_matching_noncontradictory_item() {
        let response = r#"{
          "code":"000000",
          "info":"操作成功",
          "data":{"canListenRespItemList":[{
            "contentId":"600908000007288315",
            "canListen":false,
            "limitLength":true
          }]}
        }"#;
        let rights = parse_listening_rights_response(response.as_bytes(), "600908000007288315")
            .expect("parse listening rights");
        assert!(!rights.can_listen);
        assert!(rights.limit_length);

        let mismatch = response.replace("600908000007288315", "600908000007288316");
        assert!(
            parse_listening_rights_response(mismatch.as_bytes(), "600908000007288315").is_err()
        );
        let contradictory = response.replace("\"canListen\":false", "\"canListen\":true");
        assert!(
            parse_listening_rights_response(contradictory.as_bytes(), "600908000007288315")
                .is_err()
        );
    }

    #[test]
    fn encrypted_playback_preserves_identity_quality_and_preview_fields() {
        let fixture = format!(
            r#"{{
              "code":"000000",
              "info":"操作成功",
              "data":{{
                "version":"safe-version",
                "url":"{}",
                "audioFormatType":"PQ",
                "auditionsStartTime":65,
                "auditionsLength":60,
                "freeListenType":"2",
                "dialogInfo":{{
                  "showType":0,
                  "text":"preview",
                  "payCompleteText":"full playback requires entitlement"
                }},
                "song":{{
                  "resourceType":"2",
                  "contentId":"600908000007288315",
                  "copyrightId":"60054704028",
                  "duration":215
                }}
              }}
            }}"#,
            public_audio_url("mp3")
        );
        let playback = parse_public_stream_response(&encrypt_public_stream_fixture(&fixture))
            .expect("decrypt playback");
        assert_eq!(playback.audio_format_type, "PQ");
        assert_eq!(
            playback
                .song
                .as_ref()
                .and_then(|song| song.duration.as_ref())
                .and_then(FlexibleU64::get),
            Some(215)
        );
        validate_playback_identity(&playback, "600908000007288315", "60054704028")
            .expect("matching identity");
        assert!(decrypt_public_stream_response(b"plain JSON").is_err());

        let rejected = encrypt_public_stream_fixture(
            r#"{"code":"440013","info":"entitlement required","data":null}"#,
        );
        let error = parse_public_stream_response(&rejected).expect_err("business rejection");
        assert_eq!(error.code, ErrorCode::PermissionDenied);
        assert_eq!(error.details["platform_code"], "440013");
    }

    #[test]
    fn media_selection_keeps_requested_and_actual_quality_distinct() {
        let track = parse_resource_response(RESOURCE_RESPONSE.as_bytes(), "600908000007288315")
            .expect("parse resource detail");
        let automatic = select_media_tone(&track, &StreamRequest::default()).expect("auto tone");
        assert_eq!(automatic.requested_quality, Quality::Auto);
        assert_eq!(automatic.tone_flag, "ZQ24");

        let high = select_media_tone(
            &track,
            &StreamRequest {
                quality: Quality::High,
                ..StreamRequest::default()
            },
        )
        .expect("high tone");
        assert_eq!(high.tone_flag, "HQ");

        let bitrate = select_media_tone(
            &track,
            &StreamRequest {
                bitrate: Some(192_000),
                ..StreamRequest::default()
            },
        )
        .expect("bitrate tone");
        assert_eq!(bitrate.tone_flag, "HQ");

        for request in [
            StreamRequest {
                variant: StreamVariant::Legacy,
                ..StreamRequest::default()
            },
            StreamRequest {
                quality: Quality::Master,
                ..StreamRequest::default()
            },
            StreamRequest {
                bitrate: Some(320_001),
                ..StreamRequest::default()
            },
        ] {
            assert!(select_media_tone(&track, &request).is_err());
        }
    }

    #[test]
    fn preview_windows_follow_explicit_listening_rights() {
        let preview_rights = MiguListeningRights {
            content_id: "600908000007288315".to_owned(),
            can_listen: false,
            limit_length: true,
        };
        let preview = MiguPlaybackData {
            auditions_start_time: Some(FlexibleU64::Number(65)),
            auditions_length: Some(FlexibleU64::Number(60)),
            ..MiguPlaybackData::default()
        };
        assert_eq!(
            playback_trial(&preview_rights, &preview).expect("preview"),
            Some(TrialWindow {
                start_ms: 65_000,
                end_ms: 125_000
            })
        );

        let full_rights = MiguListeningRights {
            content_id: "600913000000358395".to_owned(),
            can_listen: true,
            limit_length: false,
        };
        assert_eq!(
            playback_trial(&full_rights, &MiguPlaybackData::default()).expect("full playback"),
            None
        );
        assert!(playback_trial(&preview_rights, &MiguPlaybackData::default()).is_err());
        assert!(playback_trial(&full_rights, &preview).is_err());
    }

    #[test]
    fn public_audio_urls_are_fixed_signed_https_resources() {
        let mp3 = public_audio_url("mp3");
        assert_eq!(
            validate_public_audio_url(&mp3).expect("trusted public audio URL"),
            mp3
        );
        let product8 = mp3.replace("/product9th/product", "/product8th/product");
        assert_eq!(
            validate_public_audio_url(&product8).expect("trusted legacy public audio URL"),
            product8
        );
        assert_eq!(
            media_spec_from_url(&mp3, "PQ").expect("PQ MP3"),
            (
                Some("mp3".to_owned()),
                Some("mp3".to_owned()),
                Some(128_000)
            )
        );
        assert!(media_spec_from_url(&mp3, "SQ").is_err());
        for value in [
            "http://freetyst.nf.migu.cn/public/product9th/product/a.mp3?Tim=1&Key=2&playSessionId=3",
            "https://evil.example/public/product9th/product/a.mp3?Tim=1&Key=2&playSessionId=3",
            "https://freetyst.nf.migu.cn:444/public/product9th/product/a.mp3?Tim=1&Key=2&playSessionId=3",
            "https://user@freetyst.nf.migu.cn/public/product9th/product/a.mp3?Tim=1&Key=2&playSessionId=3",
            "https://freetyst.nf.migu.cn/private/a.mp3?Tim=1&Key=2&playSessionId=3",
            "https://freetyst.nf.migu.cn/public/product9th/product/a.mp3?Tim=1",
            "https://freetyst.nf.migu.cn/public/product9th/product/a.mp3?Tim=1&Key=2&Key=3&playSessionId=4",
        ] {
            assert!(
                validate_public_audio_url(value).is_err(),
                "{value} must fail"
            );
        }
    }

    #[test]
    fn media_urls_accept_only_fixed_https_migu_storage_paths() {
        assert_eq!(
            normalize_media_url("/data/oss/resource/a.webp").as_deref(),
            Some("https://d.musicapp.migu.cn/data/oss/resource/a.webp")
        );
        assert_eq!(
            normalize_media_url("https://d.musicapp.migu.cn/data/oss/resource/a.webp").as_deref(),
            Some("https://d.musicapp.migu.cn/data/oss/resource/a.webp")
        );
        for value in [
            "http://d.musicapp.migu.cn/data/oss/a",
            "https://evil.example/data/oss/a",
            "https://d.musicapp.migu.cn:444/data/oss/a",
            "https://user@d.musicapp.migu.cn/data/oss/a",
            "/other/path",
        ] {
            assert!(normalize_media_url(value).is_none(), "{value} must fail");
        }
    }

    #[test]
    fn configuration_debug_redacts_proxy_credentials() {
        let config = MiguConfig {
            proxy_url: Some("http://secret@example.test:8080".to_owned()),
        };
        let debug = format!("{config:?}");
        assert!(!debug.contains("secret"));
        assert!(!debug.contains("example.test"));
    }

    #[tokio::test]
    #[ignore = "requires live Migu network access"]
    async fn live_public_search_returns_stable_tracks_over_https() {
        let client = MiguClient::test_client();
        let page = client
            .search_tracks_page("周杰伦", 1)
            .await
            .expect("live Migu search");
        assert!(!page.tracks.is_empty());
        assert!(
            page.tracks
                .iter()
                .all(|track| track.resource_ref.platform() == Platform::Migu)
        );
    }

    #[tokio::test]
    #[ignore = "requires live Migu network access"]
    async fn live_resource_detail_returns_current_identity_and_formats() {
        let client = MiguClient::test_client();
        let track = client
            .track_detail("600908000007288315")
            .await
            .expect("live Migu resource detail");
        assert_eq!(track.resource_ref.to_string(), "migu:600908000007288315");
        assert_eq!(track.extensions["copyright_id"], "60054704028");
        assert!(!track.available_qualities.is_empty());
    }
}
