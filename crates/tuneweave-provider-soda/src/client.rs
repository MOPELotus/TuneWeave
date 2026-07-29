use std::{collections::BTreeMap, fmt, time::Duration};

use reqwest::{
    Client, Proxy, StatusCode,
    header::{CONTENT_LENGTH, LOCATION},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tuneweave_core::{
    AlbumSummary, ArtistSummary, ErrorCode, Extensions, LyricContributor, Lyrics, Platform,
    Quality, ResourceRef, Result, Track, TrackAvailability, TrackAvailabilityRequest,
    TuneWeaveError,
};
use url::Url;

use crate::identity::{
    SodaTrackIdentity, SodaTrackIdentityInput, classify_track_identity,
    parse_short_redirect_location,
};

pub(crate) const UPSTREAM_SEARCH_PAGE_SIZE: u32 = 20;
const SEARCH_ENDPOINT: &str = "https://api.qishui.com/luna/pc/search/track";
const SODA_APP_ID: &str = "386088";
const MAX_API_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_LYRIC_CONTENT_BYTES: usize = 4 * 1024 * 1024;
const MAX_LYRIC_LINES: usize = 20_000;
const MAX_WORDS_PER_LINE: usize = 2_000;
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

#[derive(Serialize)]
struct SodaTrackDetailQuery<'a> {
    track_id: &'a str,
    media_type: &'static str,
    aid: &'static str,
    device_platform: &'static str,
    channel: &'static str,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct SodaTrackDetailEnvelope {
    status_info: SodaStatusInfo,
    track: Option<SodaTrack>,
    risk_result: Option<i64>,
    lyric: SodaLyricPayload,
    track_player: Option<SodaTrackPlayer>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct SodaLyricPayload {
    content: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct SodaTrackPlayer {
    expire_at: u64,
    media_id: String,
    video_model: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct SodaVideoModel {
    status: i64,
    message: String,
    video_id: String,
    enable_ssl: bool,
    video_duration: f64,
    media_type: String,
    url_expire: u64,
    video_list: Vec<SodaVideoVariant>,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct SodaVideoVariant {
    main_url: String,
    backup_url: FlexibleStringList,
    video_meta: SodaVideoMeta,
    encrypt_info: SodaMediaEncryption,
}

#[derive(Default, Deserialize)]
#[serde(untagged)]
enum FlexibleStringList {
    One(String),
    Many(Vec<String>),
    #[default]
    Null,
}

impl FlexibleStringList {
    fn into_vec(self) -> Vec<String> {
        match self {
            Self::One(value) => vec![value],
            Self::Many(values) => values,
            Self::Null => Vec::new(),
        }
    }
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct SodaVideoMeta {
    quality: String,
    vtype: String,
    bitrate: u64,
    size: u64,
    codec_type: String,
    real_bitrate: u64,
    audio_sample_rate: String,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct SodaMediaEncryption {
    encrypt: bool,
    kid: String,
    spade_a: String,
    encryption_method: String,
}

#[derive(Serialize)]
struct SodaPublicMediaSpec {
    quality: String,
    format: String,
    codec: String,
    bitrate: u64,
    real_bitrate: u64,
    size: u64,
    sample_rate_hz: Option<u64>,
    encrypted: bool,
    encryption_method: Option<String>,
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
    state: SodaTrackState,
    stats: SodaTrackStats,
    song_maker_team: SodaSongMakerTeam,
    media_type: String,
    chorus: Option<SodaTimedSegment>,
    first_vocal: Option<SodaTimedSegment>,
    lang_codes: Vec<String>,
    sharable_platforms: Vec<String>,
    tags: Vec<SodaTrackTag>,
    karaoke: SodaKaraoke,
    vocal: Option<i64>,
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
    only_vip_download: Option<bool>,
    only_vip_playable: Option<bool>,
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

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaTrackState {
    offline: Option<bool>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaTrackStats {
    count_collected: u64,
    count_comment: u64,
    count_shared: u64,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaSongMakerTeam {
    composers: Vec<SodaCredit>,
    lyricists: Vec<SodaCredit>,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaCredit {
    name: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaTimedSegment {
    start: u64,
    duration: u64,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaTrackTag {
    category: SodaTagLevel,
    first_level_tag: SodaTagLevel,
    second_level_tag: SodaTagLevel,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaTagLevel {
    tag_id: u64,
    tag_name: String,
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default)]
struct SodaKaraoke {
    supported: bool,
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

    async fn fetch_track_v2_body(
        &self,
        identity: &SodaTrackIdentity,
        operation: &str,
    ) -> Result<Vec<u8>> {
        let response = self
            .http
            .get("https://api.qishui.com/luna/pc/track_v2")
            .query(&SodaTrackDetailQuery {
                track_id: identity.id(),
                media_type: "track",
                aid: SODA_APP_ID,
                device_platform: "web",
                channel: "pc_web",
            })
            .send()
            .await
            .map_err(soda_network_error)?;
        read_bounded_response(response, operation).await
    }

    pub(crate) async fn track_detail(&self, identity: &SodaTrackIdentity) -> Result<Track> {
        let body = self
            .fetch_track_v2_body(identity, "Soda track detail")
            .await?;
        parse_track_detail_response(&body, identity)
    }

    pub(crate) async fn lyrics(&self, identity: &SodaTrackIdentity) -> Result<Lyrics> {
        let body = self.fetch_track_v2_body(identity, "Soda lyrics").await?;
        parse_lyrics_response(&body, identity)
    }

    pub(crate) async fn track_availability(
        &self,
        identity: &SodaTrackIdentity,
        request: &TrackAvailabilityRequest,
    ) -> Result<TrackAvailability> {
        let body = self
            .fetch_track_v2_body(identity, "Soda track availability")
            .await?;
        parse_track_availability_response(&body, identity, request)
    }

    pub async fn resolve_track_identity(&self, input: &str) -> Result<SodaTrackIdentity> {
        let short_url = match classify_track_identity(input)? {
            SodaTrackIdentityInput::Direct(identity) => return Ok(identity),
            SodaTrackIdentityInput::ShortLink(url) => url,
        };
        let response = self
            .http
            .head(short_url)
            .send()
            .await
            .map_err(soda_network_error)?;
        if !response.status().is_redirection() {
            return Err(soda_upstream_error(
                "Soda short link did not return a redirect",
            ));
        }
        let location = response
            .headers()
            .get(LOCATION)
            .and_then(|value| value.to_str().ok())
            .ok_or_else(|| soda_upstream_error("Soda short link omitted its redirect location"))?;
        parse_short_redirect_location(location)
            .map_err(|_| soda_upstream_error("Soda short link returned an untrusted destination"))
    }
}

fn parse_search_response(body: &[u8], requested_cursor: u32) -> Result<SodaSearchPage> {
    let envelope: SodaSearchEnvelope = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda search returned malformed JSON"))?;
    validate_status_metadata(&envelope.status_info, "Soda search")?;

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
        .map(|track| track.and_then(|track| map_track(track, "official_pc_track_search")))
        .collect::<Result<Vec<_>>>()?;
    Ok(SodaSearchPage {
        tracks,
        next_cursor,
        has_more: group.has_more,
    })
}

fn parse_track_detail_response(body: &[u8], identity: &SodaTrackIdentity) -> Result<Track> {
    let envelope: SodaTrackDetailEnvelope = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda track detail returned malformed JSON"))?;
    validate_status_metadata(&envelope.status_info, "Soda track detail")?;
    if envelope.risk_result.is_some_and(|value| value != 0) {
        return Err(soda_upstream_error(
            "Soda track detail was rejected by platform risk control",
        ));
    }
    let track = envelope
        .track
        .ok_or_else(|| soda_upstream_error("Soda track detail omitted its track payload"))?;
    if track.id.trim() != identity.id() {
        return Err(soda_upstream_error(
            "Soda track detail returned a mismatched track identity",
        ));
    }
    if !track.media_type.trim().is_empty() && track.media_type.trim() != "track" {
        return Err(soda_upstream_error(
            "Soda track detail returned a non-track media type",
        ));
    }
    let mut mapped = map_track(track, "official_pc_track_v2")?;
    mapped.extensions.insert(
        "canonical_share_url".to_owned(),
        json!(identity.canonical_url()),
    );
    Ok(mapped)
}

fn parse_lyrics_response(body: &[u8], identity: &SodaTrackIdentity) -> Result<Lyrics> {
    let envelope: SodaTrackDetailEnvelope = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda lyrics returned malformed JSON"))?;
    validate_status_metadata(&envelope.status_info, "Soda lyrics")?;
    if envelope.risk_result.is_some_and(|value| value != 0) {
        return Err(soda_upstream_error(
            "Soda lyrics were rejected by platform risk control",
        ));
    }
    let track = envelope
        .track
        .ok_or_else(|| soda_upstream_error("Soda lyrics omitted the track payload"))?;
    if track.id.trim() != identity.id() {
        return Err(soda_upstream_error(
            "Soda lyrics returned a mismatched track identity",
        ));
    }
    if !track.media_type.trim().is_empty() && track.media_type.trim() != "track" {
        return Err(soda_upstream_error(
            "Soda lyrics returned a non-track media type",
        ));
    }
    let parsed = parse_word_synced_lyrics(&envelope.lyric.content)?;
    let contributors = track
        .song_maker_team
        .lyricists
        .into_iter()
        .filter_map(|credit| bounded_text(&credit.name, 1_000))
        .map(|name| LyricContributor {
            role: "lyricist".to_owned(),
            resource_ref: None,
            name,
        })
        .collect();
    let mut extensions = Extensions::new();
    extensions.insert("backend".to_owned(), json!("official_pc_track_v2"));
    extensions.insert("line_count".to_owned(), json!(parsed.line_count));
    extensions.insert("word_count".to_owned(), json!(parsed.word_count));
    extensions.insert("line_time_unit".to_owned(), json!("milliseconds"));
    extensions.insert(
        "word_offset_origin".to_owned(),
        json!("relative_to_line_start"),
    );
    extensions.insert("plain_derived_from_word_synced".to_owned(), json!(true));
    extensions.insert("unknown_word_tag_field_preserved".to_owned(), json!(true));
    Ok(Lyrics {
        track_ref: identity.resource_ref()?,
        plain: Some(parsed.plain),
        translated: None,
        romanized: None,
        word_synced: Some(parsed.word_synced),
        singing_annotations: None,
        singing_annotations_timestamp: None,
        format: "krc".to_owned(),
        contributors,
        extensions,
    })
}

fn parse_track_availability_response(
    body: &[u8],
    identity: &SodaTrackIdentity,
    request: &TrackAvailabilityRequest,
) -> Result<TrackAvailability> {
    let envelope: SodaTrackDetailEnvelope = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda availability returned malformed JSON"))?;
    validate_status_metadata(&envelope.status_info, "Soda availability")?;
    if envelope.risk_result.is_some_and(|value| value != 0) {
        return Err(soda_upstream_error(
            "Soda availability was rejected by platform risk control",
        ));
    }
    let track = envelope
        .track
        .ok_or_else(|| soda_upstream_error("Soda availability omitted the track payload"))?;
    if track.id.trim() != identity.id() {
        return Err(soda_upstream_error(
            "Soda availability returned a mismatched track identity",
        ));
    }
    if !track.media_type.trim().is_empty() && track.media_type.trim() != "track" {
        return Err(soda_upstream_error(
            "Soda availability returned a non-track media type",
        ));
    }

    let mut extensions = Extensions::new();
    extensions.insert("backend".to_owned(), json!("official_pc_track_v2"));
    extensions.insert(
        "catalog_only_vip_playable".to_owned(),
        json!(track.label_info.only_vip_playable),
    );
    if track.state.offline == Some(true) {
        extensions.insert("unavailable_reason".to_owned(), json!("offline"));
        return Ok(TrackAvailability {
            track_ref: identity.resource_ref()?,
            playable: false,
            requested_bitrate: request.bitrate,
            actual_bitrate: None,
            platform_code: None,
            message: "Soda reported that this track is offline".to_owned(),
            extensions,
        });
    }

    let Some(player) = envelope.track_player else {
        extensions.insert("preview_available".to_owned(), json!(false));
        return Ok(TrackAvailability {
            track_ref: identity.resource_ref()?,
            playable: false,
            requested_bitrate: request.bitrate,
            actual_bitrate: None,
            platform_code: None,
            message: "Soda did not authorize anonymous media".to_owned(),
            extensions,
        });
    };
    let media = validate_player_model(&track, &player, request.bitrate, envelope.status_info.now)?;
    extensions.insert("preview_available".to_owned(), json!(media.preview));
    extensions.insert("encrypted".to_owned(), json!(media.encrypted));
    extensions.insert(
        "requires_local_decryption".to_owned(),
        json!(media.encrypted),
    );
    extensions.insert("media_duration_ms".to_owned(), json!(media.duration_ms));
    extensions.insert("available_qualities".to_owned(), json!(media.qualities));
    extensions.insert("media_specs".to_owned(), json!(media.specs));
    extensions.insert(
        "media_expires_at_epoch_seconds".to_owned(),
        json!(media.expires_at),
    );
    if media.preview {
        let (start_ms, duration_ms) = validated_preview_window(&track, media.duration_ms)?;
        extensions.insert("preview_start_ms".to_owned(), json!(start_ms));
        extensions.insert("preview_duration_ms".to_owned(), json!(duration_ms));
        extensions.insert(
            "preview_actual_bitrate".to_owned(),
            json!(media.selected_bitrate),
        );
    }
    Ok(TrackAvailability {
        track_ref: identity.resource_ref()?,
        playable: !media.preview,
        requested_bitrate: request.bitrate,
        actual_bitrate: (!media.preview).then_some(media.selected_bitrate),
        platform_code: Some(media.platform_code),
        message: if media.preview {
            "Soda only permits an anonymous preview".to_owned()
        } else {
            "ok".to_owned()
        },
        extensions,
    })
}

struct ValidatedSodaMedia {
    preview: bool,
    encrypted: bool,
    duration_ms: u64,
    selected_bitrate: u64,
    platform_code: i64,
    expires_at: u64,
    qualities: Vec<Quality>,
    specs: Vec<SodaPublicMediaSpec>,
}

fn validate_player_model(
    track: &SodaTrack,
    player: &SodaTrackPlayer,
    requested_bitrate: u64,
    upstream_now: u64,
) -> Result<ValidatedSodaMedia> {
    if player.video_model.is_empty() || player.video_model.len() > MAX_API_RESPONSE_BYTES as usize {
        return Err(soda_upstream_error(
            "Soda availability omitted a bounded player model",
        ));
    }
    let model: SodaVideoModel = serde_json::from_str(&player.video_model)
        .map_err(|_| soda_upstream_error("Soda availability returned a malformed player model"))?;
    let duration_ms = seconds_to_milliseconds(model.video_duration).ok_or_else(|| {
        soda_upstream_error("Soda availability returned an invalid media duration")
    })?;
    if model.status != 10
        || model.message.trim() != "success"
        || model.media_type.trim() != "audio"
        || !model.enable_ssl
        || model.video_id.trim().is_empty()
        || model.video_id.trim() != player.media_id.trim()
        || model.video_list.is_empty()
        || model.video_list.len() > 16
    {
        return Err(soda_upstream_error(
            "Soda availability returned inconsistent player metadata",
        ));
    }
    let full = !track.vid.trim().is_empty()
        && player.media_id.trim() == track.vid.trim()
        && duration_ms.abs_diff(track.duration) <= 2_000;
    let preview_id = track
        .audition_info
        .as_ref()
        .map(|info| info.vid.trim())
        .filter(|value| !value.is_empty())
        .or_else(|| {
            track
                .preview
                .as_ref()
                .map(|preview| preview.vid.trim())
                .filter(|value| !value.is_empty())
        });
    let preview = !full
        && preview_id.is_some_and(|value| value == player.media_id.trim())
        && track
            .audition_info
            .as_ref()
            .map(|info| info.duration_ms)
            .or_else(|| track.preview.as_ref().map(|preview| preview.duration))
            .is_some_and(|expected| duration_ms.abs_diff(expected) <= 2_000);
    if !full && !preview {
        return Err(soda_upstream_error(
            "Soda availability could not classify full or preview media",
        ));
    }

    let mut specs = Vec::with_capacity(model.video_list.len());
    let mut qualities = Vec::new();
    let mut encrypted = false;
    let mut selectable_bitrates = Vec::new();
    for variant in model.video_list {
        let spec = validate_video_variant(variant)?;
        if let Some(quality) = map_quality(&spec.quality)
            && !qualities.contains(&quality)
        {
            qualities.push(quality);
        }
        encrypted |= spec.encrypted;
        selectable_bitrates.push(spec.bitrate);
        specs.push(spec);
    }
    qualities = [
        Quality::Low,
        Quality::Standard,
        Quality::High,
        Quality::Lossless,
        Quality::Hires,
        Quality::Spatial,
    ]
    .into_iter()
    .filter(|quality| qualities.contains(quality))
    .collect();
    let selected_bitrate = select_bitrate(&selectable_bitrates, requested_bitrate)
        .ok_or_else(|| soda_upstream_error("Soda availability omitted a usable bitrate"))?;
    let expires_at = [player.expire_at, model.url_expire]
        .into_iter()
        .filter(|value| *value > 0)
        .min()
        .ok_or_else(|| soda_upstream_error("Soda availability omitted media expiry"))?;
    if upstream_now == 0
        || expires_at <= upstream_now
        || expires_at > upstream_now.saturating_add(2 * 24 * 60 * 60)
    {
        return Err(soda_upstream_error(
            "Soda availability returned an invalid media expiry",
        ));
    }
    Ok(ValidatedSodaMedia {
        preview,
        encrypted,
        duration_ms,
        selected_bitrate,
        platform_code: model.status,
        expires_at,
        qualities,
        specs,
    })
}

fn validate_video_variant(variant: SodaVideoVariant) -> Result<SodaPublicMediaSpec> {
    let meta = variant.video_meta;
    let backup_urls = variant.backup_url.into_vec();
    if meta.quality.is_empty()
        || meta.quality.len() > 64
        || meta.vtype.is_empty()
        || meta.vtype.len() > 32
        || meta.codec_type.is_empty()
        || meta.codec_type.len() > 32
        || meta.bitrate == 0
        || meta.bitrate > 10_000_000
        || meta.real_bitrate == 0
        || meta.real_bitrate > 10_000_000
        || meta.size == 0
        || meta.size > 10 * 1024 * 1024 * 1024
        || backup_urls.len() > 4
    {
        return Err(soda_upstream_error(
            "Soda availability returned an invalid media specification",
        ));
    }
    validate_media_url(&variant.main_url)?;
    for url in &backup_urls {
        validate_media_url(url)?;
    }
    let encryption_method = if variant.encrypt_info.encrypt {
        if variant.encrypt_info.encryption_method != "cenc-aes-ctr"
            || variant.encrypt_info.kid.is_empty()
            || variant.encrypt_info.kid.len() > 512
            || variant.encrypt_info.spade_a.is_empty()
            || variant.encrypt_info.spade_a.len() > 16 * 1024
        {
            return Err(soda_upstream_error(
                "Soda availability returned unsupported media encryption",
            ));
        }
        Some(variant.encrypt_info.encryption_method)
    } else {
        None
    };
    let sample_rate_hz = canonical_positive_decimal(&meta.audio_sample_rate)
        .and_then(|value| value.parse::<u64>().ok());
    Ok(SodaPublicMediaSpec {
        quality: meta.quality,
        format: meta.vtype,
        codec: meta.codec_type,
        bitrate: meta.bitrate,
        real_bitrate: meta.real_bitrate,
        size: meta.size,
        sample_rate_hz,
        encrypted: variant.encrypt_info.encrypt,
        encryption_method,
    })
}

fn validate_media_url(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 8_192 {
        return Err(soda_upstream_error(
            "Soda availability returned an invalid media URL",
        ));
    }
    let url = Url::parse(value)
        .map_err(|_| soda_upstream_error("Soda availability returned an invalid media URL"))?;
    let host = url
        .host_str()
        .ok_or_else(|| soda_upstream_error("Soda availability returned an invalid media URL"))?;
    let prefix = host
        .strip_suffix("-luna.douyinvod.com")
        .ok_or_else(|| soda_upstream_error("Soda availability returned an untrusted media host"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || !matches!(url.port(), None | Some(443))
        || url.fragment().is_some()
        || url.path().is_empty()
        || prefix.is_empty()
        || !prefix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(soda_upstream_error(
            "Soda availability returned an untrusted media URL",
        ));
    }
    Ok(())
}

fn validated_preview_window(track: &SodaTrack, media_duration_ms: u64) -> Result<(u64, u64)> {
    let audition = track.audition_info.as_ref();
    let preview = track.preview.as_ref();
    if let (Some(audition), Some(preview)) = (audition, preview)
        && (audition.vid.trim() != preview.vid.trim()
            || audition.start_time_ms != preview.start
            || audition.duration_ms.abs_diff(preview.duration) > 100)
    {
        return Err(soda_upstream_error(
            "Soda availability returned conflicting preview windows",
        ));
    }
    let (start, duration) = audition
        .map(|value| (value.start_time_ms, value.duration_ms))
        .or_else(|| preview.map(|value| (value.start, value.duration)))
        .ok_or_else(|| soda_upstream_error("Soda availability omitted its preview window"))?;
    if duration == 0
        || duration.abs_diff(media_duration_ms) > 2_000
        || start.saturating_add(duration) > track.duration.saturating_add(2_000)
    {
        return Err(soda_upstream_error(
            "Soda availability returned an invalid preview window",
        ));
    }
    Ok((start, duration))
}

fn seconds_to_milliseconds(value: f64) -> Option<u64> {
    if !value.is_finite() || value <= 0.0 || value > 24.0 * 60.0 * 60.0 {
        return None;
    }
    let milliseconds = (value * 1_000.0).round();
    (milliseconds > 0.0 && milliseconds <= u64::MAX as f64).then_some(milliseconds as u64)
}

fn select_bitrate(values: &[u64], requested: u64) -> Option<u64> {
    values
        .iter()
        .copied()
        .filter(|value| *value <= requested)
        .max()
        .or_else(|| values.iter().copied().min())
}

struct ParsedSodaLyrics {
    plain: String,
    word_synced: String,
    line_count: usize,
    word_count: usize,
}

fn parse_word_synced_lyrics(raw: &str) -> Result<ParsedSodaLyrics> {
    if raw.is_empty() || raw.len() > MAX_LYRIC_CONTENT_BYTES {
        return Err(soda_upstream_error(
            "Soda lyrics contained no bounded lyric content",
        ));
    }
    if raw
        .chars()
        .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(soda_upstream_error(
            "Soda lyrics contained unsupported control characters",
        ));
    }

    let mut plain = String::with_capacity(raw.len());
    let mut normalized = String::with_capacity(raw.len());
    let mut line_count = 0_usize;
    let mut word_count = 0_usize;
    for source_line in raw.lines() {
        let line = source_line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        line_count = line_count.saturating_add(1);
        if line_count > MAX_LYRIC_LINES {
            return Err(soda_upstream_error(
                "Soda lyrics exceeded the line count limit",
            ));
        }
        let close = line
            .find(']')
            .filter(|_| line.starts_with('['))
            .ok_or_else(|| soda_upstream_error("Soda lyrics contained a malformed line header"))?;
        let (line_start, line_duration) = parse_pair(&line[1..close], "line header")?;
        if line_duration == 0
            || line_start > 24 * 60 * 60 * 1_000
            || line_duration > 24 * 60 * 60 * 1_000
        {
            return Err(soda_upstream_error(
                "Soda lyrics contained an invalid line time range",
            ));
        }
        let payload = &line[close + 1..];
        let text = parse_word_payload(payload, line_duration, &mut word_count)?;
        if text.is_empty() {
            return Err(soda_upstream_error(
                "Soda lyrics contained an empty timed line",
            ));
        }
        if !normalized.is_empty() {
            normalized.push('\n');
            plain.push('\n');
        }
        normalized.push_str(line);
        plain.push_str(&format_lrc_timestamp(line_start));
        plain.push_str(&text);
    }
    if line_count == 0 || word_count == 0 {
        return Err(soda_upstream_error(
            "Soda lyrics did not contain word-synchronized lines",
        ));
    }
    Ok(ParsedSodaLyrics {
        plain,
        word_synced: normalized,
        line_count,
        word_count,
    })
}

fn parse_word_payload(
    payload: &str,
    line_duration: u64,
    total_words: &mut usize,
) -> Result<String> {
    let mut remaining = payload;
    let mut text = String::new();
    let mut previous_offset = 0_u64;
    let mut words_in_line = 0_usize;
    while !remaining.is_empty() {
        if !remaining.starts_with('<') {
            return Err(soda_upstream_error(
                "Soda lyrics contained text without a word timing tag",
            ));
        }
        let close = remaining
            .find('>')
            .ok_or_else(|| soda_upstream_error("Soda lyrics contained an open word timing tag"))?;
        let (offset, duration, _unknown) = parse_triplet(&remaining[1..close], "word tag")?;
        if duration == 0
            || offset < previous_offset
            || offset.saturating_add(duration) > line_duration
        {
            return Err(soda_upstream_error(
                "Soda lyrics contained an invalid word time range",
            ));
        }
        previous_offset = offset;
        remaining = &remaining[close + 1..];
        let next = remaining.find('<').unwrap_or(remaining.len());
        let word = &remaining[..next];
        if word.is_empty() {
            return Err(soda_upstream_error(
                "Soda lyrics contained an empty timed word",
            ));
        }
        text.push_str(word);
        words_in_line = words_in_line.saturating_add(1);
        *total_words = total_words.saturating_add(1);
        if words_in_line > MAX_WORDS_PER_LINE {
            return Err(soda_upstream_error(
                "Soda lyrics exceeded the per-line word limit",
            ));
        }
        remaining = &remaining[next..];
    }
    Ok(text)
}

fn parse_pair(value: &str, context: &str) -> Result<(u64, u64)> {
    let mut fields = value.split(',');
    let first = parse_lyric_u64(fields.next(), context)?;
    let second = parse_lyric_u64(fields.next(), context)?;
    if fields.next().is_some() {
        return Err(soda_upstream_error(format!(
            "Soda lyrics contained a malformed {context}"
        )));
    }
    Ok((first, second))
}

fn parse_triplet(value: &str, context: &str) -> Result<(u64, u64, i64)> {
    let mut fields = value.split(',');
    let first = parse_lyric_u64(fields.next(), context)?;
    let second = parse_lyric_u64(fields.next(), context)?;
    let third = fields
        .next()
        .and_then(|field| field.parse::<i64>().ok())
        .ok_or_else(|| {
            soda_upstream_error(format!("Soda lyrics contained a malformed {context}"))
        })?;
    if fields.next().is_some() {
        return Err(soda_upstream_error(format!(
            "Soda lyrics contained a malformed {context}"
        )));
    }
    Ok((first, second, third))
}

fn parse_lyric_u64(value: Option<&str>, context: &str) -> Result<u64> {
    let value = value
        .filter(|value| canonical_nonnegative_decimal(value).is_some())
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or_else(|| {
            soda_upstream_error(format!("Soda lyrics contained a malformed {context}"))
        })?;
    Ok(value)
}

fn format_lrc_timestamp(milliseconds: u64) -> String {
    let minutes = milliseconds / 60_000;
    let seconds = milliseconds % 60_000 / 1_000;
    let millis = milliseconds % 1_000;
    format!("[{minutes:02}:{seconds:02}.{millis:03}]")
}

fn validate_status_metadata(status: &SodaStatusInfo, operation: &str) -> Result<()> {
    if status.log_id.len() > 256 || status.now_ts_ms < status.now.saturating_mul(1_000) {
        return Err(soda_upstream_error(format!(
            "{operation} returned invalid status metadata"
        )));
    }
    Ok(())
}

fn map_track(source: SodaTrack, backend: &'static str) -> Result<Track> {
    validate_track_metadata_bounds(&source)?;
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
    if source.state.offline == Some(true) {
        track.playable = Some(false);
    }

    let mut extensions = Extensions::new();
    extensions.insert("backend".to_owned(), json!(backend));
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
    extensions.insert("stats".to_owned(), json!(source.stats));
    extensions.insert("credits".to_owned(), json!(source.song_maker_team));
    if let Some(chorus) = source.chorus {
        extensions.insert("chorus".to_owned(), json!(chorus));
    }
    if let Some(first_vocal) = source.first_vocal {
        extensions.insert("first_vocal".to_owned(), json!(first_vocal));
    }
    if !source.lang_codes.is_empty() {
        extensions.insert("language_codes".to_owned(), json!(source.lang_codes));
    }
    if !source.sharable_platforms.is_empty() {
        extensions.insert(
            "sharable_platforms".to_owned(),
            json!(source.sharable_platforms),
        );
    }
    if !source.tags.is_empty() {
        extensions.insert("tags".to_owned(), json!(source.tags));
    }
    extensions.insert("karaoke".to_owned(), json!(source.karaoke));
    if let Some(vocal) = source.vocal {
        extensions.insert("vocal".to_owned(), json!(vocal));
    }
    extensions.insert(
        "catalog_rights_are_not_live_playback".to_owned(),
        json!(true),
    );
    track.extensions = extensions;
    Ok(track)
}

fn validate_track_metadata_bounds(source: &SodaTrack) -> Result<()> {
    let bounded = source.artists.len() <= 128
        && source.bit_rates.len() <= 32
        && source
            .preview
            .as_ref()
            .is_none_or(|value| value.bit_rates.len() <= 32)
        && source.label_info.quality_map.len() <= 32
        && source.label_info.quality_only_vip_can_download.len() <= 32
        && source.label_info.quality_only_vip_can_play.len() <= 32
        && source.song_maker_team.composers.len() <= 128
        && source.song_maker_team.lyricists.len() <= 128
        && source.lang_codes.len() <= 32
        && source.sharable_platforms.len() <= 32
        && source.tags.len() <= 128;
    let text_bounded = source
        .artists
        .iter()
        .all(|artist| artist.name.len() <= 1_000 && artist.simple_display_name.len() <= 1_000)
        && source
            .song_maker_team
            .composers
            .iter()
            .chain(source.song_maker_team.lyricists.iter())
            .all(|credit| credit.name.len() <= 1_000)
        && source
            .lang_codes
            .iter()
            .chain(source.sharable_platforms.iter())
            .all(|value| value.len() <= 128)
        && source.tags.iter().all(|tag| {
            [&tag.category, &tag.first_level_tag, &tag.second_level_tag]
                .into_iter()
                .all(|level| level.tag_name.len() <= 256)
        });
    if !bounded || !text_bounded {
        return Err(soda_upstream_error(
            "Soda track metadata exceeded structural limits",
        ));
    }
    Ok(())
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

    const TRACK_DETAIL_RESPONSE: &str = r#"{
      "status_info":{"log_id":"safe-detail","now":1785336862,"now_ts_ms":1785336862374},
      "risk_result":0,
      "track":{
        "id":"7304719759323564095","name":"落了白","duration":180822,
        "media_type":"track","vid":"v03ad6g10000cli4pgjc77u93k8r7pbg",
        "artists":[{"id":"6795393014723250177","name":"蒋雪儿Snow.J"}],
        "album":{"id":"7304719759323580479","name":"落了白","release_date":1606665600,"url_cover":{"uri":"tos-cn-v-2774c002/cover","urls":["https://p6-luna.douyinpic.com/img/"]}},
        "bit_rates":[{"br":66163,"size":1495476,"quality":"medium"},{"br":260264,"size":5882690,"quality":"highest"}],
        "state":{"offline":false},
        "stats":{"count_collected":3548864,"count_comment":9024,"count_shared":18667},
        "song_maker_team":{"composers":[{"name":"刘涛"}],"lyricists":[{"name":"堇临|刘涛"}]},
        "chorus":{"start":29184,"duration":0},"first_vocal":{"start":3072,"duration":26112},
        "lang_codes":["ZH"],"sharable_platforms":["link","wechat"],
        "tags":[{"category":{"tag_id":1,"tag_name":"Genre"},"first_level_tag":{"tag_id":6730932201380170498,"tag_name":"Pop"}}],
        "karaoke":{"supported":true},"vocal":1,
        "label_info":{"only_vip_download":true,"quality_only_vip_can_play":["lossless"]}
      },
      "lyric":{"content":"not part of track detail"},
      "track_player":{"url_player_info":"https://vod-luna.douyin.com/?token=private","video_model":"private-player-model"}
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
    fn track_detail_maps_rich_metadata_without_exposing_ephemeral_playback_data() {
        let identity =
            SodaTrackIdentity::parse("7304719759323564095").expect("valid Soda track identity");
        let track = parse_track_detail_response(TRACK_DETAIL_RESPONSE.as_bytes(), &identity)
            .expect("parse Soda track detail");
        assert_eq!(track.resource_ref.to_string(), "soda:7304719759323564095");
        assert_eq!(track.name, "落了白");
        assert_eq!(track.duration_ms, Some(180_822));
        assert_eq!(track.available_qualities, vec![Quality::Low, Quality::High]);
        assert_eq!(track.playable, None);
        assert_eq!(track.extensions["backend"], "official_pc_track_v2");
        assert_eq!(track.extensions["stats"]["count_collected"], 3_548_864);
        assert_eq!(track.extensions["credits"]["composers"][0]["name"], "刘涛");
        assert_eq!(track.extensions["karaoke"]["supported"], true);
        assert_eq!(
            track.extensions["canonical_share_url"],
            "https://www.qishui.com/track/7304719759323564095"
        );
        let serialized = serde_json::to_string(&track).expect("serialize Soda track detail");
        assert!(!serialized.contains("private"));
        assert!(!serialized.contains("url_player_info"));
        assert!(!serialized.contains("video_model"));
    }

    #[test]
    fn track_detail_rejects_identity_media_risk_and_structure_drift() {
        let identity =
            SodaTrackIdentity::parse("7304719759323564095").expect("valid Soda track identity");
        let wrong_id = TRACK_DETAIL_RESPONSE.replace("7304719759323564095", "7304719759323564096");
        assert!(parse_track_detail_response(wrong_id.as_bytes(), &identity).is_err());
        let wrong_media =
            TRACK_DETAIL_RESPONSE.replace("\"media_type\":\"track\"", "\"media_type\":\"podcast\"");
        assert!(parse_track_detail_response(wrong_media.as_bytes(), &identity).is_err());
        let risk = TRACK_DETAIL_RESPONSE.replace("\"risk_result\":0", "\"risk_result\":1");
        assert!(parse_track_detail_response(risk.as_bytes(), &identity).is_err());
        let oversized = TRACK_DETAIL_RESPONSE.replace(
            "\"lang_codes\":[\"ZH\"]",
            &format!("\"lang_codes\":{}", json!(vec!["ZH"; 33])),
        );
        assert!(parse_track_detail_response(oversized.as_bytes(), &identity).is_err());
    }

    #[test]
    fn lyrics_keep_word_sync_primary_and_derive_plain_without_overwriting_it() {
        let identity =
            SodaTrackIdentity::parse("7304719759323564095").expect("valid Soda track identity");
        let raw = "[3150,1000]<0,400,0>你<400,600,0>好\n[5000,500]<0,500,7>！";
        let mut response: serde_json::Value =
            serde_json::from_str(TRACK_DETAIL_RESPONSE).expect("detail fixture JSON");
        response["lyric"]["content"] = json!(raw);
        let body = serde_json::to_vec(&response).expect("serialize lyric fixture");
        let lyrics = parse_lyrics_response(&body, &identity).expect("parse Soda lyrics");
        assert_eq!(lyrics.track_ref.to_string(), "soda:7304719759323564095");
        assert_eq!(lyrics.format, "krc");
        assert_eq!(lyrics.word_synced.as_deref(), Some(raw));
        assert_eq!(
            lyrics.plain.as_deref(),
            Some("[00:03.150]你好\n[00:05.000]！")
        );
        assert_eq!(lyrics.translated, None);
        assert_eq!(lyrics.romanized, None);
        assert_eq!(lyrics.extensions["line_count"], 2);
        assert_eq!(lyrics.extensions["word_count"], 3);
        assert_eq!(lyrics.extensions["plain_derived_from_word_synced"], true);
        assert_eq!(lyrics.contributors[0].role, "lyricist");
        assert_eq!(lyrics.contributors[0].name, "堇临|刘涛");
    }

    #[test]
    fn lyrics_reject_malformed_or_lossy_word_timing_instead_of_downgrading() {
        for raw in [
            "plain text only",
            "[0,1000]text without tag",
            "[0,1000]<0,0,0>zero duration",
            "[0,1000]<900,200,0>past line end",
            "[0,1000]<0,500,0>",
            "[0,1000]<0,500>missing field",
            "[0,1000]<500,200,0>后<100,200,0>前",
        ] {
            assert!(parse_word_synced_lyrics(raw).is_err(), "{raw}");
        }
        assert!(parse_word_synced_lyrics("[0,1000]<0,500,0>好\0").is_err());
    }

    fn availability_fixture(preview: bool) -> Vec<u8> {
        let mut response: serde_json::Value =
            serde_json::from_str(TRACK_DETAIL_RESPONSE).expect("detail fixture JSON");
        let full_vid = "v03ad6g10000cli4pgjc77u93k8r7pbg";
        let preview_vid = "v10ad6g50000d6po467og65ocf8m2mcg";
        response["track"]["vid"] = json!(full_vid);
        response["track"]["preview"] = json!({
            "vid": preview_vid,
            "start": 107_904,
            "duration": 60_001,
            "bit_rates": []
        });
        response["track"]["audition_info"] = json!({
            "vid": preview_vid,
            "start_time_ms": 107_904,
            "duration_ms": 60_001
        });
        let media_id = if preview { preview_vid } else { full_vid };
        let duration = if preview { 60.001 } else { 180.822 };
        let variants = [
            ("highest", 260_477_u64, 1_953_579_u64),
            ("higher", 132_424_u64, 993_185_u64),
            ("medium", 68_413_u64, 513_101_u64),
        ]
        .into_iter()
        .map(|(quality, bitrate, size)| {
            json!({
                "main_url": format!("https://v1-test-luna.douyinvod.com/media/{quality}?a=1"),
                "backup_url": [format!("https://v2-test-luna.douyinvod.com/media/{quality}?a=1")],
                "video_meta": {
                    "quality": quality,
                    "vtype": "m4a",
                    "bitrate": bitrate,
                    "real_bitrate": bitrate,
                    "size": size,
                    "codec_type": "aac",
                    "audio_sample_rate": "44100",
                    "file_id": "not-exported",
                    "file_hash": "not-exported"
                },
                "encrypt_info": {
                    "encrypt": true,
                    "kid": "private-kid",
                    "spade_a": "private-spade",
                    "encryption_method": "cenc-aes-ctr"
                }
            })
        })
        .collect::<Vec<_>>();
        let model = json!({
            "status": 10,
            "message": "success",
            "video_id": media_id,
            "enable_ssl": true,
            "video_duration": duration,
            "media_type": "audio",
            "url_expire": 1_785_423_262_u64,
            "video_list": variants
        });
        response["track_player"] = json!({
            "expire_at": 1_785_423_262_u64,
            "media_id": media_id,
            "video_model": serde_json::to_string(&model).expect("serialize video model"),
            "url_player_info": "https://vod-luna.douyin.com/?token=private"
        });
        serde_json::to_vec(&response).expect("serialize availability fixture")
    }

    fn mutate_availability_fixture(mutator: impl FnOnce(&mut serde_json::Value)) -> Vec<u8> {
        let mut response: serde_json::Value =
            serde_json::from_slice(&availability_fixture(false)).expect("availability JSON");
        let model_text = response["track_player"]["video_model"]
            .as_str()
            .expect("video model");
        let mut model: serde_json::Value =
            serde_json::from_str(model_text).expect("video model JSON");
        mutator(&mut model);
        response["track_player"]["video_model"] =
            json!(serde_json::to_string(&model).expect("serialize changed model"));
        serde_json::to_vec(&response).expect("serialize changed availability")
    }

    #[test]
    fn availability_distinguishes_full_media_from_preview_and_hides_crypto_material() {
        let identity =
            SodaTrackIdentity::parse("7304719759323564095").expect("valid Soda track identity");
        let request = TrackAvailabilityRequest::new(200_000);
        let full =
            parse_track_availability_response(&availability_fixture(false), &identity, &request)
                .expect("parse full Soda media");
        assert!(full.playable);
        assert_eq!(full.actual_bitrate, Some(132_424));
        assert_eq!(full.extensions["preview_available"], false);
        assert_eq!(full.extensions["encrypted"], true);
        assert_eq!(full.extensions["requires_local_decryption"], true);

        let preview =
            parse_track_availability_response(&availability_fixture(true), &identity, &request)
                .expect("parse Soda preview media");
        assert!(!preview.playable);
        assert_eq!(preview.actual_bitrate, None);
        assert_eq!(preview.extensions["preview_available"], true);
        assert_eq!(preview.extensions["preview_start_ms"], 107_904);
        assert_eq!(preview.extensions["preview_duration_ms"], 60_001);
        assert_eq!(preview.extensions["preview_actual_bitrate"], 132_424);
        let serialized = serde_json::to_string(&preview).expect("serialize Soda availability");
        for secret in [
            "private-kid",
            "private-spade",
            "url_player_info",
            "token=private",
            "file_id",
            "file_hash",
        ] {
            assert!(!serialized.contains(secret), "must hide {secret}");
        }
    }

    #[test]
    fn availability_rejects_untrusted_media_identity_encryption_and_expiry() {
        let identity =
            SodaTrackIdentity::parse("7304719759323564095").expect("valid Soda track identity");
        let request = TrackAvailabilityRequest::default();
        let evil = mutate_availability_fixture(|model| {
            model["video_list"][0]["main_url"] = json!("https://evil.example/media");
        });
        assert!(parse_track_availability_response(&evil, &identity, &request).is_err());
        let wrong_id = mutate_availability_fixture(|model| {
            model["video_id"] = json!("unexpected-media");
        });
        assert!(parse_track_availability_response(&wrong_id, &identity, &request).is_err());
        let method = mutate_availability_fixture(|model| {
            model["video_list"][0]["encrypt_info"]["encryption_method"] = json!("unknown");
        });
        assert!(parse_track_availability_response(&method, &identity, &request).is_err());
        let expiry = mutate_availability_fixture(|model| {
            model["url_expire"] = json!(1_785_336_000_u64);
        });
        assert!(parse_track_availability_response(&expiry, &identity, &request).is_err());
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

    #[tokio::test]
    #[ignore = "requires live Soda network access"]
    async fn live_official_short_link_resolves_without_following_arbitrary_redirects() {
        let identity = SodaClient::test_client()
            .resolve_track_identity("https://qishui.douyin.com/s/iQeFw9cE/")
            .await
            .expect("resolve official Soda short link");
        assert_eq!(identity.id(), "7304719759323564095");
        assert_eq!(
            identity.resource_ref().expect("Soda reference").to_string(),
            "soda:7304719759323564095"
        );
    }

    #[tokio::test]
    #[ignore = "requires live Soda network access"]
    async fn live_anonymous_track_detail_preserves_identity_and_hides_player_tokens() {
        let identity =
            SodaTrackIdentity::parse("7304719759323564095").expect("valid Soda track identity");
        let track = SodaClient::test_client()
            .track_detail(&identity)
            .await
            .expect("live Soda track detail");
        assert_eq!(track.resource_ref.to_string(), "soda:7304719759323564095");
        assert_eq!(track.name, "落了白");
        assert_eq!(track.duration_ms, Some(180_822));
        assert_eq!(track.extensions["backend"], "official_pc_track_v2");
        let serialized = serde_json::to_string(&track).expect("serialize live Soda track");
        assert!(!serialized.contains("url_player_info"));
        assert!(!serialized.contains("video_model"));
        assert!(!serialized.contains("vod-luna.douyin.com"));
    }

    #[tokio::test]
    #[ignore = "requires live Soda network access"]
    async fn live_anonymous_lyrics_keep_word_and_plain_tracks_separate() {
        let identity =
            SodaTrackIdentity::parse("7304719759323564095").expect("valid Soda track identity");
        let lyrics = SodaClient::test_client()
            .lyrics(&identity)
            .await
            .expect("live Soda lyrics");
        let plain = lyrics.plain.expect("derived plain Soda lyrics");
        let word_synced = lyrics.word_synced.expect("word-synchronized Soda lyrics");
        assert_eq!(lyrics.format, "krc");
        assert!(plain.contains("[00:"));
        assert!(!plain.contains('<'));
        assert!(word_synced.contains('<'));
        assert!(word_synced.len() > plain.len());
        assert!(
            lyrics.extensions["line_count"]
                .as_u64()
                .is_some_and(|count| count > 0)
        );
        assert!(
            lyrics.extensions["word_count"]
                .as_u64()
                .is_some_and(|count| count > 0)
        );
        assert_eq!(lyrics.translated, None);
        assert_eq!(lyrics.romanized, None);
    }

    #[tokio::test]
    #[ignore = "requires live Soda network access"]
    async fn live_anonymous_availability_distinguishes_full_and_preview_media() {
        let client = SodaClient::test_client();
        let request = TrackAvailabilityRequest::new(200_000);
        let free =
            SodaTrackIdentity::parse("6911353635137914887").expect("valid free Soda identity");
        let free = client
            .track_availability(&free, &request)
            .await
            .expect("live free Soda availability");
        assert!(free.playable);
        assert!(free.actual_bitrate.is_some());
        assert_eq!(free.extensions["preview_available"], false);
        assert_eq!(free.extensions["encrypted"], true);

        let paid =
            SodaTrackIdentity::parse("7304719759323564095").expect("valid paid Soda identity");
        let paid = client
            .track_availability(&paid, &request)
            .await
            .expect("live preview Soda availability");
        assert!(!paid.playable);
        assert_eq!(paid.actual_bitrate, None);
        assert_eq!(paid.extensions["preview_available"], true);
        assert_eq!(paid.extensions["preview_start_ms"], 107_904);
        assert_eq!(paid.extensions["preview_duration_ms"], 60_001);
    }
}
