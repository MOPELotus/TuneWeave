use std::{
    collections::BTreeMap,
    fmt,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use aws_lc_rs::{
    digest::{SHA256, digest},
    hmac,
};
use base64::{
    Engine as _,
    engine::general_purpose::{STANDARD as BASE64, URL_SAFE_NO_PAD as BASE64_URL_SAFE_NO_PAD},
};
use num_bigint::BigUint;
use qrcode::{QrCode, render::svg};
use reqwest::{
    Client, Proxy, StatusCode,
    header::{COOKIE, HeaderMap, REFERER, SET_COOKIE},
    redirect::Policy,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tuneweave_core::{AccountCredentialStore, ErrorCode, Platform, Result, TuneWeaveError};
use url::Url;

use crate::wbi::WbiKeys;

const PASSPORT_QR_GENERATE_ENDPOINT: &str =
    "https://passport.bilibili.com/x/passport-login/web/qrcode/generate?source=main-fe-header";
const PASSPORT_QR_POLL_ENDPOINT: &str =
    "https://passport.bilibili.com/x/passport-login/web/qrcode/poll";
const COOKIE_INFO_ENDPOINT: &str = "https://passport.bilibili.com/x/passport-login/web/cookie/info";
const COOKIE_CORRESPOND_ENDPOINT: &str = "https://www.bilibili.com/correspond/1/";
const COOKIE_REFRESH_ENDPOINT: &str =
    "https://passport.bilibili.com/x/passport-login/web/cookie/refresh";
const COOKIE_CONFIRM_ENDPOINT: &str =
    "https://passport.bilibili.com/x/passport-login/web/confirm/refresh";
const LOGOUT_ENDPOINT: &str = "https://passport.bilibili.com/login/exit/v2";
const NAV_ENDPOINT: &str = "https://api.bilibili.com/x/web-interface/nav";
const DEVICE_IDENTITY_ENDPOINT: &str = "https://api.bilibili.com/x/frontend/finger/spi";
const WEB_HOME_ENDPOINT: &str = "https://www.bilibili.com/";
const WEB_TICKET_ENDPOINT: &str =
    "https://api.bilibili.com/bapis/bilibili.api.ticket.v1.Ticket/GenWebTicket";
const SEARCH_SUGGESTION_ENDPOINT: &str = "https://s.search.bilibili.com/main/suggest";
const SEARCH_TRENDING_ENDPOINT: &str = "https://api.bilibili.com/x/web-interface/wbi/search/square";
const VIDEO_SEARCH_ENDPOINT: &str = "https://api.bilibili.com/x/web-interface/wbi/search/type";
const VIDEO_SEARCH_COMPATIBILITY_ENDPOINT: &str =
    "https://api.bilibili.com/x/web-interface/search/type";
const VIDEO_DETAIL_ENDPOINT: &str = "https://api.bilibili.com/x/web-interface/view";
const VIDEO_PLAYER_ENDPOINT: &str = "https://api.bilibili.com/x/player/wbi/v2";
const VIDEO_PLAYBACK_ENDPOINT: &str = "https://api.bilibili.com/x/player/wbi/playurl";
const CREATED_FAVORITE_FOLDERS_ENDPOINT: &str =
    "https://api.bilibili.com/x/v3/fav/folder/created/list-all";
const FAVORITE_FOLDER_DETAIL_ENDPOINT: &str = "https://api.bilibili.com/x/v3/fav/folder/info";
const FAVORITE_FOLDER_MEDIA_ENDPOINT: &str = "https://api.bilibili.com/x/v3/fav/resource/list";
const COLLECTED_PLAYLISTS_ENDPOINT: &str =
    "https://api.bilibili.com/x/v3/fav/folder/collected/list";
const SPACE_PLAYLISTS_ENDPOINT: &str =
    "https://api.bilibili.com/x/polymer/web-space/seasons_series_list";
const SEASON_ARCHIVES_ENDPOINT: &str =
    "https://api.bilibili.com/x/polymer/web-space/seasons_archives_list";
const WEB_REFERER: &str = "https://www.bilibili.com/";
const VIDEO_SEARCH_REFERER: &str = "https://search.bilibili.com/";
const VIDEO_SEARCH_WEB_LOCATION: &str = "1430654";
const FAVORITE_FOLDER_WEB_LOCATION: &str = "333.1387";
const COLLECTED_PLAYLIST_PAGE_SIZE: u32 = 70;
const SPACE_PLAYLIST_PAGE_SIZE: u32 = 20;
const SPACE_PLAYLIST_WEB_LOCATION: &str = "333.999";
const SEASON_ARCHIVE_PAGE_SIZE: u32 = 30;
const FAVORITE_MEDIA_PAGE_SIZE: u32 = 20;
const VIDEO_SEARCH_PAGE_SIZE: u32 = 20;
const WEB_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const MAX_PASSPORT_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_SUBTITLE_RESPONSE_BYTES: usize = 4 * 1024 * 1024;
const WBI_CACHE_LIFETIME: Duration = Duration::from_secs(12 * 60 * 60);
const WEB_TICKET_HMAC_KEY: &[u8] = b"XgwSnGZ1p";
const WEB_TICKET_EXPIRY_MARGIN: Duration = Duration::from_secs(5 * 60);
const VIDEO_SEARCH_COMPATIBILITY_LIFETIME: Duration = Duration::from_secs(10 * 60);
const COOKIE_REFRESH_RSA_MODULUS_BASE64URL: &str = concat!(
    "y4HdjgJHBlbaBN04VERG4qNBIFHP6a3GozCl75AihQloSWCXC5HDNgyinEnhaQ_4",
    "-gaMud_GF50elYXLlCToR9se9Z8z433U3KjM-3Yx7ptKkmQNAMggQwAVKgq3zYAoi",
    "dNEWuxpkY_mAitTSRLnsJW-NCTa0bqBFF6Wm1MxgfE",
);
const COOKIE_REFRESH_RSA_EXPONENT: u32 = 65_537;

#[derive(Clone, Default)]
pub struct BilibiliConfig {
    pub proxy_url: Option<String>,
    pub credential_store: Option<Arc<dyn AccountCredentialStore>>,
}

impl fmt::Debug for BilibiliConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BilibiliConfig")
            .field(
                "proxy_url",
                &self.proxy_url.as_ref().map(|_| "[configured]"),
            )
            .field(
                "credential_store_configured",
                &self.credential_store.is_some(),
            )
            .finish()
    }
}

#[derive(Clone)]
pub struct BilibiliClient {
    http: Client,
    web_state: Arc<Mutex<BilibiliWebState>>,
}

#[derive(Default)]
struct BilibiliWebState {
    device: Option<BilibiliWebDevice>,
    wbi: Option<CachedWbiKeys>,
    ticket: Option<CachedWebTicket>,
    query_visit_id: Option<String>,
    video_search_challenged_at: Option<Instant>,
}

#[derive(Clone)]
struct CachedWbiKeys {
    keys: WbiKeys,
    cached_at: Instant,
}

#[derive(Clone, Eq, PartialEq)]
struct BilibiliWebDevice {
    buvid3: String,
    buvid4: String,
    b_nut: String,
}

impl fmt::Debug for BilibiliWebDevice {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BilibiliWebDevice")
            .field("buvid3_configured", &(!self.buvid3.is_empty()))
            .field("buvid4_configured", &(!self.buvid4.is_empty()))
            .field("b_nut_configured", &(!self.b_nut.is_empty()))
            .finish()
    }
}

impl BilibiliWebDevice {
    fn cookie_header(&self) -> String {
        let mut value = format!("buvid3={}; buvid4={}", self.buvid3, self.buvid4);
        if !self.b_nut.is_empty() {
            value.push_str("; b_nut=");
            value.push_str(&self.b_nut);
        }
        value
    }
}

#[derive(Clone)]
struct CachedWebTicket {
    ticket: String,
    cached_at: Instant,
    lifetime: Duration,
}

impl CachedWebTicket {
    fn is_current(&self) -> bool {
        self.cached_at
            .elapsed()
            .saturating_add(WEB_TICKET_EXPIRY_MARGIN)
            < self.lifetime
    }
}

impl fmt::Debug for CachedWebTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CachedWebTicket")
            .field("ticket", &"[redacted]")
            .field("is_current", &self.is_current())
            .finish()
    }
}

struct BilibiliWebRequestContext {
    query: String,
    cookie_header: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliVideoSearchPage {
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
    pub page_count: u32,
    pub search_id: String,
    pub videos: Vec<BilibiliSearchVideo>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliSearchVideo {
    pub aid: u64,
    pub bvid: Option<String>,
    pub title: String,
    pub author: String,
    pub author_id: u64,
    pub description: String,
    pub cover_url: String,
    pub duration_seconds: u64,
    pub duration_text: String,
    pub play_count: Option<u64>,
    pub danmaku_count: Option<u64>,
    pub favorite_count: Option<u64>,
    pub comment_count: Option<u64>,
    pub published_at: Option<u64>,
    pub sent_at: Option<u64>,
    pub category_id: Option<String>,
    pub category_name: Option<String>,
    pub tags: Vec<String>,
    pub hit_columns: Vec<String>,
    pub paid: Option<bool>,
    pub collaborative: Option<bool>,
    pub rank_score: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliSearchSuggestionSet {
    pub response_code: i64,
    pub experiment: Option<String>,
    pub search_token: Option<String>,
    pub reported_total: Option<u64>,
    pub user_feature: Option<String>,
    pub suggestions: Vec<BilibiliSearchSuggestion>,
    pub extensions: BTreeMap<String, Value>,
    pub result_extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliSearchSuggestion {
    pub keyword: String,
    pub term: Option<String>,
    pub display_text: String,
    pub highlight_ranges: Vec<BilibiliTextRange>,
    pub reference: u64,
    pub spid: u64,
    pub suggestion_type: Option<String>,
    pub item_feature: Option<String>,
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliSearchTrending {
    pub title: String,
    pub track_id: Option<String>,
    pub entries: Vec<BilibiliSearchTrendingEntry>,
    pub top_list: Vec<Value>,
    pub extensions: BTreeMap<String, Value>,
    pub trending_extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliSearchTrendingEntry {
    pub keyword: String,
    pub display_text: String,
    pub icon_url: Option<String>,
    pub action_uri: Option<String>,
    pub action_kind: Option<String>,
    pub heat_score: Option<u64>,
    pub word_type: Option<i64>,
    pub extensions: BTreeMap<String, Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct BilibiliTextRange {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliCreatedFavoriteFolders {
    pub owner_id: u64,
    pub folders: Vec<BilibiliCreatedFavoriteFolder>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliCreatedFavoriteFolder {
    pub media_id: u64,
    pub folder_id: u64,
    pub owner_id: u64,
    pub attributes: u64,
    pub title: String,
    pub favorite_state: bool,
    pub media_count: u64,
    pub child_friendly: bool,
    pub child_friendly_description: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliFavoriteFolder {
    pub media_id: u64,
    pub folder_id: u64,
    pub owner: BilibiliFavoriteFolderOwner,
    pub attributes: u64,
    pub title: String,
    pub cover_url: Option<String>,
    pub cover_type: u64,
    pub description: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub invalid: bool,
    pub favorite_state: bool,
    pub like_state: bool,
    pub media_count: u64,
    pub pinned: bool,
    pub child_friendly: bool,
    pub child_friendly_description: String,
    pub counts: BilibiliFavoriteFolderCounts,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliFavoriteFolderOwner {
    pub id: u64,
    pub name: String,
    pub avatar_url: Option<String>,
    pub followed: bool,
    pub vip_type: u64,
    pub vip_status: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliFavoriteFolderCounts {
    pub collect: u64,
    pub play: u64,
    pub thumb_up: u64,
    pub share: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliFavoriteMediaPage {
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
    pub has_more: bool,
    pub folder: BilibiliFavoriteFolder,
    pub medias: Vec<BilibiliFavoriteMedia>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliFavoriteMedia {
    pub aid: u64,
    pub bvid: Option<String>,
    pub title: String,
    pub cover_url: Option<String>,
    pub description: String,
    pub part_count: u64,
    pub duration_seconds: u64,
    pub owner: Option<BilibiliCollectedPlaylistOwner>,
    pub invalid: bool,
    pub collect_count: u64,
    pub play_count: u64,
    pub danmaku_count: u64,
    pub created_at: u64,
    pub published_at: u64,
    pub favorited_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliVideoView {
    pub aid: u64,
    pub bvid: String,
    pub title: String,
    pub description: String,
    pub dynamic_text: String,
    pub cover_url: String,
    pub duration_seconds: u64,
    pub published_at: u64,
    pub created_at: u64,
    pub state: i64,
    pub category_id: u64,
    pub category_id_v2: Option<u64>,
    pub category_name: Option<String>,
    pub category_name_v2: Option<String>,
    pub copyright: u64,
    pub owner: BilibiliCollectedPlaylistOwner,
    pub stats: BilibiliVideoViewStats,
    pub rights: BilibiliVideoRights,
    pub parts: Vec<BilibiliVideoPart>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliVideoViewStats {
    pub view: u64,
    pub danmaku: u64,
    pub reply: u64,
    pub favorite: u64,
    pub coin: u64,
    pub share: u64,
    pub like: u64,
    pub now_rank: u64,
    pub his_rank: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliVideoRights {
    pub download: bool,
    pub movie: bool,
    pub pay: bool,
    pub high_bitrate: bool,
    pub no_reprint: bool,
    pub ugc_pay: bool,
    pub cooperation: bool,
    pub interactive: bool,
    pub panoramic: bool,
    pub no_share: bool,
    pub free_watch: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliVideoPart {
    pub cid: u64,
    pub page: u64,
    pub source: String,
    pub title: String,
    pub duration_seconds: u64,
    pub width: u64,
    pub height: u64,
    pub rotated: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliSubtitleCatalog {
    pub aid: u64,
    pub bvid: String,
    pub cid: u64,
    pub requires_login: bool,
    pub can_submit: bool,
    pub default_language: Option<String>,
    pub default_language_label: Option<String>,
    pub subtitles: Vec<BilibiliSubtitle>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliSubtitle {
    pub id: u64,
    pub id_string: String,
    pub language: String,
    pub label: String,
    pub locked: bool,
    pub resource_url: Url,
    pub subtitle_type: i64,
    pub ai_type: i64,
    pub ai_status: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BilibiliSubtitleBody {
    pub source_language: Option<String>,
    pub source_type: Option<String>,
    pub source_version: Option<String>,
    pub font_size: Option<f64>,
    pub font_color: Option<String>,
    pub background_alpha: Option<f64>,
    pub background_color: Option<String>,
    pub stroke: Option<String>,
    pub cues: Vec<BilibiliSubtitleCue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BilibiliSubtitleCue {
    pub id: Option<String>,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
    pub position: Option<u32>,
    pub music_confidence: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BilibiliPlaybackManifest {
    pub aid: u64,
    pub bvid: String,
    pub cid: u64,
    pub duration_ms: u64,
    pub current_quality: u32,
    pub format: String,
    pub accepted_formats: Vec<String>,
    pub accepted_qualities: Vec<u32>,
    pub formats: Vec<BilibiliPlaybackFormat>,
    pub video_codec_id: Option<i64>,
    pub seek_parameter: Option<String>,
    pub seek_type: Option<String>,
    pub minimum_buffer_time: Option<f64>,
    pub video_tracks: Vec<BilibiliPlaybackTrack>,
    pub audio_tracks: Vec<BilibiliPlaybackTrack>,
    pub dolby_audio_tracks: Vec<BilibiliPlaybackTrack>,
    pub lossless_audio_tracks: Vec<BilibiliPlaybackTrack>,
    pub dolby_type: Option<u32>,
    pub lossless_display: Option<bool>,
    pub progressive_segments: Vec<BilibiliProgressiveSegment>,
    pub selected_audio_language: Option<String>,
    pub selected_production_type: Option<u32>,
    pub languages: Option<BilibiliPlaybackLanguageCatalog>,
    pub last_play_time_ms: Option<u64>,
    pub last_play_cid: Option<u64>,
    pub expires_at_epoch_seconds: Option<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliPlaybackTrack {
    pub id: u32,
    pub url: String,
    pub backup_urls: Vec<String>,
    pub bandwidth: u64,
    pub mime_type: String,
    pub codecs: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub frame_rate: Option<String>,
    pub sample_aspect_ratio: Option<String>,
    pub start_with_sap: Option<u32>,
    pub segment_base: Option<BilibiliSegmentBase>,
    pub codec_id: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliSegmentBase {
    pub initialization: String,
    pub index_range: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliProgressiveSegment {
    pub order: u32,
    pub duration_ms: u64,
    pub size: u64,
    pub url: String,
    pub backup_urls: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliPlaybackFormat {
    pub quality: u32,
    pub format: String,
    pub description: String,
    pub display_description: String,
    pub superscript: Option<String>,
    pub codecs: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliPlaybackLanguageCatalog {
    pub supported: bool,
    pub items: Vec<BilibiliPlaybackLanguage>,
    pub open_message: Option<String>,
    pub close_message: Option<String>,
    pub default_title: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliPlaybackLanguage {
    pub language: String,
    pub title: String,
    pub subtitle_language: Option<String>,
    pub video_detected: Option<bool>,
    pub mouth_shape_changed: Option<bool>,
    pub production_type: Option<u32>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliCollectedPlaylistPage {
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
    pub has_more: bool,
    pub playlists: Vec<BilibiliCollectedPlaylist>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum BilibiliCollectedPlaylistKind {
    FavoriteFolder,
    Season,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliCollectedPlaylist {
    pub kind: BilibiliCollectedPlaylistKind,
    pub id: u64,
    pub folder_id: Option<u64>,
    pub owner: Option<BilibiliCollectedPlaylistOwner>,
    pub attributes: u64,
    pub attribute_description: String,
    pub title: String,
    pub cover_url: Option<String>,
    pub description: String,
    pub cover_type: u64,
    pub created_at: u64,
    pub updated_at: u64,
    pub invalid: bool,
    pub favorite_state: bool,
    pub media_count: u64,
    pub view_count: Option<u64>,
    pub pinned: Option<bool>,
    pub deep_link: Option<String>,
    pub bvid: Option<String>,
    pub child_friendly: bool,
    pub child_friendly_description: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliCollectedPlaylistOwner {
    pub id: u64,
    pub name: String,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliSpacePlaylistPage {
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
    pub has_more: bool,
    pub playlists: Vec<BilibiliSpacePlaylist>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub(crate) enum BilibiliSpacePlaylistKind {
    Season,
    Series,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliSpacePlaylist {
    pub kind: BilibiliSpacePlaylistKind,
    pub id: u64,
    pub owner_id: u64,
    pub name: String,
    pub display_title: Option<String>,
    pub description: String,
    pub cover_url: Option<String>,
    pub category: u64,
    pub track_count: u64,
    pub created_at: u64,
    pub published_at: u64,
    pub updated_at: u64,
    pub state: Option<u64>,
    pub creator_mode: Option<String>,
    pub keywords: Vec<String>,
    pub recent_aids: Vec<u64>,
    pub preview_aids: Vec<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliSeasonArchivePage {
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
    pub has_more: bool,
    pub season: BilibiliSpacePlaylist,
    pub archives: Vec<BilibiliSeasonArchive>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BilibiliSeasonArchive {
    pub aid: u64,
    pub bvid: String,
    pub title: String,
    pub cover_url: String,
    pub duration_seconds: u64,
    pub created_at: u64,
    pub published_at: u64,
    pub interactive: bool,
    pub playback_position: Option<i64>,
    pub state: i64,
    pub paid: bool,
    pub view_count: u64,
    pub danmaku_count: Option<u64>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum BilibiliVideoSearchOrder {
    #[default]
    Relevance,
    MostPlayed,
    Newest,
    MostDanmaku,
    MostFavorited,
    MostCommented,
}

impl BilibiliVideoSearchOrder {
    const fn parameter(self) -> &'static str {
        match self {
            Self::Relevance => "totalrank",
            Self::MostPlayed => "click",
            Self::Newest => "pubdate",
            Self::MostDanmaku => "dm",
            Self::MostFavorited => "stow",
            Self::MostCommented => "scores",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum BilibiliVideoSearchDuration {
    #[default]
    Any,
    UnderTenMinutes,
    TenToThirtyMinutes,
    ThirtyToSixtyMinutes,
    OverSixtyMinutes,
}

impl BilibiliVideoSearchDuration {
    const fn parameter(self) -> &'static str {
        match self {
            Self::Any => "0",
            Self::UnderTenMinutes => "1",
            Self::TenToThirtyMinutes => "2",
            Self::ThirtyToSixtyMinutes => "3",
            Self::OverSixtyMinutes => "4",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct BilibiliVideoSearchFilters {
    pub order: BilibiliVideoSearchOrder,
    pub duration: BilibiliVideoSearchDuration,
    pub category_id: Option<u32>,
}

impl BilibiliVideoSearchFilters {
    fn category_parameter(self) -> String {
        self.category_id.unwrap_or_default().to_string()
    }
}

impl fmt::Debug for BilibiliWebRequestContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BilibiliWebRequestContext")
            .field("query", &"[redacted]")
            .field("cookie_header", &"[redacted]")
            .finish()
    }
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) struct BilibiliQrStart {
    pub qrcode_key: String,
    pub url: String,
    pub image_data_url: String,
}

#[derive(Clone, Eq, PartialEq)]
pub(crate) enum BilibiliQrPoll {
    Waiting,
    Scanned,
    Expired,
    Confirmed {
        credential: BilibiliCredential,
        timestamp_ms: Option<u64>,
    },
    Failed {
        code: i64,
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BilibiliCredentialRefresh {
    pub credential: BilibiliCredential,
    pub status: BilibiliSessionStatus,
    pub refreshed: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BilibiliLogoutOutcome {
    LoggedOut,
    CredentialExpired,
}

impl fmt::Debug for BilibiliQrPoll {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Waiting => formatter.write_str("Waiting"),
            Self::Scanned => formatter.write_str("Scanned"),
            Self::Expired => formatter.write_str("Expired"),
            Self::Confirmed { timestamp_ms, .. } => formatter
                .debug_struct("Confirmed")
                .field("credential", &"[redacted]")
                .field("timestamp_ms", timestamp_ms)
                .finish(),
            Self::Failed { code, message } => formatter
                .debug_struct("Failed")
                .field("code", code)
                .field("message", message)
                .finish(),
        }
    }
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct BilibiliCredential {
    pub dede_user_id: String,
    pub dede_user_id_ck_md5: String,
    pub sessdata: String,
    pub bili_jct: String,
    pub sid: Option<String>,
    pub refresh_token: String,
}

impl fmt::Debug for BilibiliCredential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BilibiliCredential")
            .field("user_id_configured", &(!self.dede_user_id.is_empty()))
            .field("has_sessdata", &(!self.sessdata.is_empty()))
            .field("has_refresh_token", &(!self.refresh_token.is_empty()))
            .finish()
    }
}

impl BilibiliCredential {
    pub(crate) fn normalize(mut self) -> Result<Self> {
        self.dede_user_id = self.dede_user_id.trim().to_owned();
        self.dede_user_id_ck_md5 = self.dede_user_id_ck_md5.trim().to_owned();
        self.sessdata = self.sessdata.trim().to_owned();
        self.bili_jct = self.bili_jct.trim().to_owned();
        self.sid = self
            .sid
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        self.refresh_token = self.refresh_token.trim().to_owned();
        if self
            .dede_user_id
            .parse::<u64>()
            .ok()
            .filter(|user_id| *user_id > 0)
            .is_none()
        {
            return Err(invalid_credential(
                "Bilibili credential has an invalid user ID",
            ));
        }
        if self.dede_user_id_ck_md5.len() != 16
            || !self
                .dede_user_id_ck_md5
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(invalid_credential(
                "Bilibili credential has an invalid user checksum",
            ));
        }
        if self.bili_jct.len() != 32 || !self.bili_jct.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err(invalid_credential(
                "Bilibili credential has an invalid CSRF token",
            ));
        }
        for (name, value, limit) in [
            ("session", self.sessdata.as_str(), 4096),
            ("refresh token", self.refresh_token.as_str(), 4096),
            ("sid", self.sid.as_deref().unwrap_or(""), 128),
        ] {
            if value.is_empty() && name != "sid" {
                return Err(invalid_credential(format!(
                    "Bilibili credential is missing its {name}"
                )));
            }
            if value.len() > limit
                || value
                    .bytes()
                    .any(|byte| byte.is_ascii_control() || byte == b';')
            {
                return Err(invalid_credential(format!(
                    "Bilibili credential contains invalid {name} data"
                )));
            }
        }
        Ok(self)
    }

    pub(crate) fn user_id(&self) -> &str {
        &self.dede_user_id
    }

    fn cookie_header(&self) -> String {
        let mut cookies = vec![
            format!("DedeUserID={}", self.dede_user_id),
            format!("DedeUserID__ckMd5={}", self.dede_user_id_ck_md5),
            format!("SESSDATA={}", self.sessdata),
            format!("bili_jct={}", self.bili_jct),
        ];
        if let Some(sid) = &self.sid {
            cookies.push(format!("sid={sid}"));
        }
        cookies.join("; ")
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BilibiliSessionStatus {
    pub authenticated: bool,
    pub user_id: Option<String>,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub extensions: BTreeMap<String, Value>,
}

impl fmt::Debug for BilibiliQrStart {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BilibiliQrStart")
            .field("qrcode_key", &"[redacted]")
            .field("url", &"[redacted]")
            .field("has_image_data_url", &true)
            .finish()
    }
}

#[derive(Deserialize)]
struct PassportResponse<T> {
    code: i64,
    #[serde(default)]
    message: String,
    data: Option<T>,
}

#[derive(Deserialize)]
struct QrGenerateData {
    url: String,
    qrcode_key: String,
}

#[derive(Deserialize)]
struct QrPollData {
    #[serde(default)]
    url: String,
    #[serde(default)]
    refresh_token: String,
    #[serde(default)]
    timestamp: Option<u64>,
    code: i64,
    #[serde(default)]
    message: String,
}

#[derive(Clone, Copy, Debug, Deserialize)]
struct CookieInfoData {
    refresh: bool,
    timestamp: u64,
}

#[derive(Deserialize)]
struct CookieRefreshData {
    status: i64,
    #[serde(default)]
    message: String,
    refresh_token: String,
}

#[derive(Deserialize)]
struct DeviceIdentityData {
    b_3: String,
    b_4: String,
}

#[derive(Deserialize)]
struct WebTicketData {
    ticket: String,
    created_at: u64,
    ttl: u64,
    nav: WebTicketNav,
}

#[derive(Deserialize)]
struct WebTicketNav {
    img: String,
    sub: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum VideoSearchPayload {
    Results(VideoSearchData),
    Voucher { v_voucher: String },
}

#[derive(Deserialize)]
struct VideoSearchData {
    page: u32,
    pagesize: u32,
    #[serde(rename = "numResults")]
    num_results: FlexibleU64,
    #[serde(rename = "numPages")]
    num_pages: u32,
    seid: FlexibleText,
    #[serde(default)]
    result: Vec<VideoSearchItem>,
}

#[derive(Deserialize)]
struct VideoSearchItem {
    #[serde(default)]
    r#type: String,
    aid: FlexibleU64,
    #[serde(default)]
    bvid: String,
    title: String,
    #[serde(default)]
    author: String,
    mid: FlexibleU64,
    #[serde(default)]
    description: String,
    pic: String,
    duration: String,
    #[serde(default)]
    play: Option<FlexibleU64>,
    #[serde(default)]
    video_review: Option<FlexibleU64>,
    #[serde(default)]
    favorites: Option<FlexibleU64>,
    #[serde(default)]
    review: Option<FlexibleU64>,
    #[serde(default)]
    pubdate: Option<FlexibleU64>,
    #[serde(default)]
    senddate: Option<FlexibleU64>,
    #[serde(default)]
    typeid: Option<FlexibleText>,
    #[serde(default)]
    typename: Option<String>,
    #[serde(default)]
    tag: String,
    #[serde(default)]
    hit_columns: Option<Vec<String>>,
    #[serde(default)]
    is_pay: Option<FlexibleU64>,
    #[serde(default)]
    is_union_video: Option<FlexibleU64>,
    #[serde(default)]
    rank_score: Option<FlexibleU64>,
}

#[derive(Deserialize)]
struct SearchSuggestionResponse {
    code: i64,
    #[serde(default)]
    exp_str: String,
    #[serde(default)]
    result: Option<SearchSuggestionResult>,
    #[serde(default)]
    stoken: String,
    #[serde(default)]
    total_count: Option<FlexibleU64>,
    #[serde(default)]
    user_feature: String,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct SearchSuggestionResult {
    #[serde(default)]
    tag: Option<Vec<SearchSuggestionItem>>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct SearchSuggestionItem {
    value: String,
    #[serde(default)]
    term: String,
    #[serde(default)]
    name: String,
    r#ref: FlexibleU64,
    spid: FlexibleU64,
    #[serde(default)]
    r#type: String,
    #[serde(default)]
    item_feature: String,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct SearchTrendingData {
    trending: SearchTrendingCatalog,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct SearchTrendingCatalog {
    title: String,
    #[serde(default)]
    trackid: String,
    list: Vec<SearchTrendingItem>,
    #[serde(default)]
    top_list: Vec<Value>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct SearchTrendingItem {
    keyword: String,
    #[serde(default)]
    show_name: String,
    #[serde(default)]
    icon: String,
    #[serde(default)]
    uri: String,
    #[serde(default)]
    goto: String,
    #[serde(default)]
    heat_score: Option<FlexibleU64>,
    #[serde(default)]
    word_type: Option<i64>,
    #[serde(flatten)]
    extra: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct VideoViewData {
    aid: u64,
    bvid: String,
    videos: u64,
    tid: u64,
    #[serde(default)]
    tid_v2: Option<u64>,
    tname: String,
    #[serde(default)]
    tname_v2: String,
    copyright: u64,
    pic: String,
    title: String,
    #[serde(default)]
    pubdate: u64,
    #[serde(default)]
    ctime: u64,
    #[serde(default)]
    desc: String,
    state: i64,
    duration: u64,
    rights: VideoViewRights,
    owner: CollectedPlaylistOwner,
    stat: VideoViewStats,
    #[serde(default)]
    dynamic: String,
    cid: u64,
    #[serde(default)]
    pages: Vec<VideoViewPart>,
}

#[derive(Deserialize)]
struct VideoViewRights {
    #[serde(default)]
    download: u64,
    #[serde(default)]
    movie: u64,
    #[serde(default)]
    pay: u64,
    #[serde(default)]
    hd5: u64,
    #[serde(default)]
    no_reprint: u64,
    #[serde(default)]
    ugc_pay: u64,
    #[serde(default)]
    is_cooperation: u64,
    #[serde(default)]
    is_stein_gate: u64,
    #[serde(default)]
    is_360: u64,
    #[serde(default)]
    no_share: u64,
    #[serde(default)]
    free_watch: u64,
}

#[derive(Deserialize)]
struct VideoViewStats {
    aid: u64,
    #[serde(default)]
    view: u64,
    #[serde(default)]
    danmaku: u64,
    #[serde(default)]
    reply: u64,
    #[serde(default)]
    favorite: u64,
    #[serde(default)]
    coin: u64,
    #[serde(default)]
    share: u64,
    #[serde(default)]
    now_rank: u64,
    #[serde(default)]
    his_rank: u64,
    #[serde(default)]
    like: u64,
}

#[derive(Deserialize)]
struct VideoViewPart {
    cid: u64,
    page: u64,
    #[serde(rename = "from")]
    source: String,
    part: String,
    duration: u64,
    #[serde(default)]
    dimension: VideoViewDimension,
}

#[derive(Default, Deserialize)]
struct VideoViewDimension {
    #[serde(default)]
    width: u64,
    #[serde(default)]
    height: u64,
    #[serde(default)]
    rotate: u64,
}

#[derive(Deserialize)]
struct VideoPlayerData {
    aid: u64,
    bvid: String,
    cid: u64,
    #[serde(default)]
    need_login_subtitle: bool,
    #[serde(default)]
    subtitle: Option<VideoPlayerSubtitleContainer>,
}

#[derive(Default, Deserialize)]
struct VideoPlayerSubtitleContainer {
    #[serde(default)]
    allow_submit: bool,
    #[serde(default)]
    lan: String,
    #[serde(default)]
    lan_doc: String,
    #[serde(default)]
    subtitles: Vec<VideoPlayerSubtitle>,
}

#[derive(Deserialize)]
struct VideoPlayerSubtitle {
    id: u64,
    id_str: String,
    lan: String,
    lan_doc: String,
    is_lock: bool,
    subtitle_url: String,
    #[serde(rename = "type")]
    subtitle_type: i64,
    ai_type: i64,
    ai_status: i64,
}

#[derive(Deserialize)]
struct SubtitleBodyData {
    #[serde(default)]
    font_size: Option<f64>,
    #[serde(default)]
    font_color: Option<String>,
    #[serde(default)]
    background_alpha: Option<f64>,
    #[serde(default)]
    background_color: Option<String>,
    #[serde(default, rename = "Stroke")]
    stroke: Option<String>,
    #[serde(default, rename = "type")]
    source_type: Option<FlexibleText>,
    #[serde(default)]
    lang: Option<FlexibleText>,
    #[serde(default)]
    version: Option<FlexibleText>,
    body: Vec<SubtitleCueData>,
}

#[derive(Deserialize)]
struct SubtitleCueData {
    #[serde(rename = "from")]
    start_seconds: f64,
    #[serde(rename = "to")]
    end_seconds: f64,
    #[serde(default)]
    sid: Option<FlexibleText>,
    #[serde(default)]
    location: Option<u32>,
    content: String,
    #[serde(default)]
    music: Option<f64>,
}

#[derive(Deserialize)]
struct PlaybackData {
    quality: u32,
    format: String,
    timelength: u64,
    #[serde(default)]
    accept_format: String,
    #[serde(default)]
    accept_description: Vec<String>,
    #[serde(default)]
    accept_quality: Vec<u32>,
    #[serde(default)]
    video_codecid: Option<i64>,
    #[serde(default)]
    seek_param: String,
    #[serde(default)]
    seek_type: String,
    #[serde(default)]
    dash: Option<PlaybackDashData>,
    #[serde(default)]
    durl: Option<Vec<ProgressiveSegmentData>>,
    #[serde(default)]
    support_formats: Vec<PlaybackFormatData>,
    #[serde(default)]
    cur_language: String,
    #[serde(default)]
    cur_production_type: Option<u32>,
    #[serde(default)]
    language: Option<PlaybackLanguageCatalogData>,
    #[serde(default)]
    last_play_time: Option<i64>,
    #[serde(default)]
    last_play_cid: Option<u64>,
}

#[derive(Deserialize)]
struct PlaybackDashData {
    duration: f64,
    #[serde(default, rename = "minBufferTime")]
    minimum_buffer_time_camel: Option<f64>,
    #[serde(default)]
    min_buffer_time: Option<f64>,
    #[serde(default)]
    video: Vec<PlaybackTrackData>,
    #[serde(default)]
    audio: Option<Vec<PlaybackTrackData>>,
    #[serde(default)]
    dolby: Option<PlaybackDolbyData>,
    #[serde(default)]
    flac: Option<PlaybackFlacData>,
}

#[derive(Deserialize)]
struct PlaybackDolbyData {
    #[serde(default, rename = "type")]
    dolby_type: Option<u32>,
    #[serde(default)]
    audio: Option<Vec<PlaybackTrackData>>,
}

#[derive(Deserialize)]
struct PlaybackFlacData {
    #[serde(default)]
    display: Option<bool>,
    #[serde(default)]
    audio: Option<PlaybackTrackData>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct PlaybackTrackData {
    id: u32,
    #[serde(default, rename = "baseUrl")]
    base_url_camel: Option<String>,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default, rename = "backupUrl")]
    backup_urls_camel: Option<Vec<String>>,
    #[serde(default)]
    backup_url: Option<Vec<String>>,
    bandwidth: u64,
    #[serde(default, rename = "mimeType")]
    mime_type_camel: Option<String>,
    #[serde(default)]
    mime_type: Option<String>,
    codecs: String,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default, rename = "frameRate")]
    frame_rate_camel: Option<String>,
    #[serde(default)]
    frame_rate: Option<String>,
    #[serde(default)]
    sar: Option<String>,
    #[serde(default, rename = "startWithSap")]
    start_with_sap_camel: Option<u32>,
    #[serde(default)]
    start_with_sap: Option<u32>,
    #[serde(default, rename = "SegmentBase")]
    segment_base_camel: Option<SegmentBaseData>,
    #[serde(default)]
    segment_base: Option<SegmentBaseData>,
    #[serde(default)]
    codecid: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
struct SegmentBaseData {
    #[serde(default, rename = "Initialization")]
    initialization_camel: Option<String>,
    #[serde(default)]
    initialization: Option<String>,
    #[serde(default, rename = "indexRange")]
    index_range_camel: Option<String>,
    #[serde(default)]
    index_range: Option<String>,
}

#[derive(Deserialize)]
struct ProgressiveSegmentData {
    order: u32,
    length: u64,
    size: u64,
    url: String,
    #[serde(default)]
    backup_url: Vec<String>,
}

#[derive(Deserialize)]
struct PlaybackFormatData {
    quality: u32,
    format: String,
    #[serde(default)]
    new_description: String,
    #[serde(default)]
    display_desc: String,
    #[serde(default)]
    superscript: String,
    #[serde(default)]
    codecs: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct PlaybackLanguageCatalogData {
    support: bool,
    #[serde(default)]
    items: Vec<PlaybackLanguageData>,
    #[serde(default)]
    open_toast: String,
    #[serde(default)]
    close_toast: String,
    #[serde(default)]
    default_title: String,
}

#[derive(Deserialize)]
struct PlaybackLanguageData {
    lang: String,
    title: String,
    #[serde(default)]
    subtitle_lang: String,
    #[serde(default)]
    video_detext: Option<bool>,
    #[serde(default)]
    video_mouth_shape_change: Option<bool>,
    #[serde(default)]
    production_type: Option<u32>,
}

#[derive(Deserialize)]
struct CreatedFavoriteFoldersData {
    count: u64,
    #[serde(default)]
    list: Option<Vec<CreatedFavoriteFolderItem>>,
    #[serde(default, rename = "season")]
    _season: Option<()>,
}

#[derive(Deserialize)]
struct CreatedFavoriteFolderItem {
    id: u64,
    fid: u64,
    mid: u64,
    attr: u64,
    title: String,
    fav_state: u64,
    media_count: u64,
    #[serde(default)]
    is_kid_playlist: bool,
    #[serde(default)]
    kid_playlist_desc: String,
}

#[derive(Deserialize)]
struct FavoriteFolderData {
    id: u64,
    fid: u64,
    mid: u64,
    attr: u64,
    title: String,
    #[serde(default)]
    cover: String,
    upper: FavoriteFolderOwner,
    #[serde(default)]
    cover_type: u64,
    #[serde(default)]
    cnt_info: FavoriteFolderCounts,
    #[serde(rename = "type")]
    kind: u64,
    #[serde(default)]
    intro: String,
    #[serde(default)]
    ctime: u64,
    #[serde(default)]
    mtime: u64,
    state: u64,
    fav_state: u64,
    like_state: u64,
    media_count: u64,
    #[serde(default)]
    is_top: bool,
    #[serde(default)]
    is_kid_playlist: bool,
    #[serde(default)]
    kid_playlist_desc: String,
}

#[derive(Deserialize)]
struct FavoriteFolderOwner {
    mid: u64,
    name: String,
    #[serde(default)]
    face: String,
    #[serde(default)]
    followed: bool,
    #[serde(default)]
    vip_type: u64,
    #[serde(default)]
    vip_statue: u64,
}

#[derive(Default, Deserialize)]
struct FavoriteFolderCounts {
    #[serde(default)]
    collect: u64,
    #[serde(default)]
    play: u64,
    #[serde(default)]
    thumb_up: u64,
    #[serde(default)]
    share: u64,
}

#[derive(Deserialize)]
struct FavoriteMediaData {
    info: FavoriteFolderData,
    #[serde(default)]
    medias: Option<Vec<FavoriteMediaItem>>,
    has_more: bool,
}

#[derive(Deserialize)]
struct FavoriteMediaItem {
    id: u64,
    #[serde(rename = "type")]
    kind: u64,
    title: String,
    #[serde(default)]
    cover: String,
    #[serde(default)]
    intro: String,
    #[serde(default)]
    page: u64,
    #[serde(default)]
    duration: u64,
    #[serde(default)]
    upper: Option<CollectedPlaylistOwner>,
    attr: u64,
    #[serde(default)]
    cnt_info: FavoriteMediaCounts,
    #[serde(default)]
    ctime: u64,
    #[serde(default)]
    pubtime: u64,
    #[serde(default)]
    fav_time: u64,
    #[serde(default)]
    bv_id: String,
    #[serde(default)]
    bvid: String,
}

#[derive(Default, Deserialize)]
struct FavoriteMediaCounts {
    #[serde(default)]
    collect: u64,
    #[serde(default)]
    play: u64,
    #[serde(default)]
    danmaku: u64,
}

#[derive(Deserialize)]
struct CollectedPlaylistsData {
    count: u64,
    #[serde(default)]
    list: Option<Vec<CollectedPlaylistItem>>,
    #[serde(default)]
    has_more: Option<bool>,
}

#[derive(Deserialize)]
struct CollectedPlaylistItem {
    id: u64,
    #[serde(default)]
    fid: u64,
    #[serde(default)]
    mid: u64,
    #[serde(default)]
    attr: u64,
    #[serde(default)]
    attr_desc: String,
    title: String,
    #[serde(default)]
    cover: String,
    #[serde(default)]
    upper: Option<CollectedPlaylistOwner>,
    #[serde(default)]
    cover_type: u64,
    #[serde(default)]
    intro: String,
    #[serde(default)]
    ctime: u64,
    #[serde(default)]
    mtime: u64,
    state: u64,
    fav_state: u64,
    media_count: u64,
    #[serde(default)]
    view_count: Option<u64>,
    #[serde(default)]
    is_top: Option<bool>,
    #[serde(default, rename = "type")]
    kind: Option<u64>,
    #[serde(default)]
    link: String,
    #[serde(default)]
    bvid: String,
    #[serde(default)]
    is_kid_playlist: bool,
    #[serde(default)]
    kid_playlist_desc: String,
}

#[derive(Deserialize)]
struct CollectedPlaylistOwner {
    #[serde(default)]
    mid: u64,
    #[serde(default)]
    name: String,
    #[serde(default)]
    face: String,
}

#[derive(Deserialize)]
struct SpacePlaylistsData {
    items_lists: SpacePlaylistLists,
}

#[derive(Deserialize)]
struct SpacePlaylistLists {
    page: SpacePlaylistPageMeta,
    #[serde(default)]
    seasons_list: Vec<SpaceSeasonItem>,
    #[serde(default)]
    series_list: Vec<SpaceSeriesItem>,
}

#[derive(Deserialize)]
struct SpacePlaylistPageMeta {
    page_num: u32,
    page_size: u32,
    total: u64,
}

#[derive(Deserialize)]
struct SpaceSeasonItem {
    #[serde(default)]
    archives: Vec<SpacePlaylistArchive>,
    meta: SpaceSeasonMeta,
    #[serde(default)]
    recent_aids: Vec<u64>,
}

#[derive(Deserialize)]
struct SpaceSeasonMeta {
    #[serde(default)]
    category: u64,
    #[serde(default)]
    cover: String,
    #[serde(default)]
    description: String,
    mid: u64,
    name: String,
    #[serde(default)]
    ptime: u64,
    season_id: u64,
    total: u64,
    #[serde(default)]
    title: String,
}

#[derive(Deserialize)]
struct SpaceSeriesItem {
    #[serde(default)]
    archives: Vec<SpacePlaylistArchive>,
    meta: SpaceSeriesMeta,
    #[serde(default)]
    recent_aids: Vec<u64>,
}

#[derive(Deserialize)]
struct SpaceSeriesMeta {
    #[serde(default)]
    category: u64,
    #[serde(default)]
    cover: String,
    #[serde(default)]
    creator: String,
    #[serde(default)]
    ctime: u64,
    #[serde(default)]
    description: String,
    #[serde(default)]
    keywords: Vec<String>,
    #[serde(default)]
    last_update_ts: u64,
    mid: u64,
    #[serde(default)]
    mtime: u64,
    name: String,
    #[serde(default)]
    raw_keywords: String,
    series_id: u64,
    #[serde(default)]
    state: u64,
    total: u64,
}

#[derive(Deserialize)]
struct SpacePlaylistArchive {
    aid: u64,
    bvid: String,
}

#[derive(Deserialize)]
struct SeasonArchivesData {
    #[serde(default)]
    aids: Vec<u64>,
    #[serde(default)]
    archives: Vec<SeasonArchiveItem>,
    meta: SpaceSeasonMeta,
    page: SpacePlaylistPageMeta,
}

#[derive(Deserialize)]
struct SeasonArchiveItem {
    aid: u64,
    bvid: String,
    #[serde(default)]
    ctime: u64,
    duration: u64,
    #[serde(default)]
    interactive_video: bool,
    pic: String,
    #[serde(default)]
    playback_position: Option<i64>,
    #[serde(default)]
    pubdate: u64,
    #[serde(default)]
    stat: SeasonArchiveStats,
    #[serde(default)]
    state: i64,
    title: String,
    #[serde(default)]
    ugc_pay: u64,
}

#[derive(Default, Deserialize)]
struct SeasonArchiveStats {
    #[serde(default)]
    view: u64,
    #[serde(default)]
    danmaku: Option<u64>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum FlexibleU64 {
    Number(u64),
    Text(String),
}

impl FlexibleU64 {
    fn get(&self) -> Option<u64> {
        match self {
            Self::Number(value) => Some(*value),
            Self::Text(value) => value.parse().ok(),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum FlexibleText {
    Text(String),
    Number(u64),
}

impl FlexibleText {
    fn into_string(self) -> String {
        match self {
            Self::Text(value) => value,
            Self::Number(value) => value.to_string(),
        }
    }
}

#[derive(Deserialize)]
struct LogoutResponse {
    code: i64,
    #[serde(default)]
    status: Option<bool>,
    #[serde(default)]
    message: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct NavData {
    #[serde(default, alias = "isLogin")]
    is_login: bool,
    #[serde(default)]
    email_verified: Option<i64>,
    #[serde(default)]
    face: Option<String>,
    #[serde(default)]
    level_info: Option<NavLevelInfo>,
    #[serde(default)]
    mid: Option<u64>,
    #[serde(default)]
    mobile_verified: Option<i64>,
    #[serde(default)]
    money: Option<f64>,
    #[serde(default)]
    moral: Option<f64>,
    #[serde(default)]
    official: Option<NavOfficial>,
    #[serde(default, alias = "officialVerify")]
    official_verify: Option<NavOfficialVerify>,
    #[serde(default)]
    pendant: Option<NavPendant>,
    #[serde(default)]
    scores: Option<i64>,
    #[serde(default)]
    uname: Option<String>,
    #[serde(default, alias = "vipDueDate")]
    vip_due_date_ms: Option<u64>,
    #[serde(default, alias = "vipStatus")]
    vip_status: Option<i64>,
    #[serde(default, alias = "vipType")]
    vip_type: Option<i64>,
    #[serde(default)]
    vip_pay_type: Option<i64>,
    #[serde(default)]
    vip_theme_type: Option<i64>,
    #[serde(default)]
    vip_label: Option<NavVipLabel>,
    #[serde(default)]
    vip_avatar_subscript: Option<i64>,
    #[serde(default)]
    vip_nickname_color: Option<String>,
    #[serde(default)]
    vip: Option<NavVip>,
    #[serde(default)]
    wallet: Option<NavWallet>,
    #[serde(default)]
    has_shop: Option<bool>,
    #[serde(default)]
    shop_url: Option<String>,
    #[serde(default)]
    allowance_count: Option<i64>,
    #[serde(default)]
    answer_status: Option<i64>,
    #[serde(default)]
    is_senior_member: Option<i64>,
    #[serde(default)]
    wbi_img: Option<NavWbiImage>,
    #[serde(default)]
    is_jury: Option<bool>,
    #[serde(default, flatten, skip_serializing)]
    _extra: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NavLevelInfo {
    current_level: i64,
    current_min: i64,
    current_exp: i64,
    next_exp: NavLevelThreshold,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum NavLevelThreshold {
    Number(i64),
    Text(String),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NavOfficial {
    role: i64,
    title: String,
    desc: String,
    #[serde(rename = "type")]
    kind: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NavOfficialVerify {
    #[serde(rename = "type")]
    kind: i64,
    desc: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NavPendant {
    pid: i64,
    name: String,
    image: String,
    expire: i64,
    #[serde(default)]
    image_enhance: Option<String>,
    #[serde(default)]
    image_enhance_frame: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NavVipLabel {
    #[serde(default)]
    path: String,
    #[serde(default)]
    text: String,
    #[serde(default)]
    label_theme: String,
    #[serde(default)]
    text_color: String,
    #[serde(default)]
    bg_style: Option<i64>,
    #[serde(default)]
    bg_color: String,
    #[serde(default)]
    border_color: String,
    #[serde(default)]
    use_img_label: Option<bool>,
    #[serde(default)]
    img_label_uri_hans: String,
    #[serde(default)]
    img_label_uri_hant: String,
    #[serde(default)]
    img_label_uri_hans_static: String,
    #[serde(default)]
    img_label_uri_hant_static: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NavVip {
    #[serde(rename = "type")]
    kind: i64,
    status: i64,
    due_date: u64,
    vip_pay_type: i64,
    theme_type: i64,
    label: NavVipLabel,
    avatar_subscript: i64,
    nickname_color: String,
    role: i64,
    avatar_subscript_url: String,
    tv_vip_status: i64,
    tv_vip_pay_type: i64,
    tv_due_date: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NavWallet {
    mid: u64,
    bcoin_balance: f64,
    coupon_balance: f64,
    coupon_due_time: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct NavWbiImage {
    img_url: String,
    sub_url: String,
}

impl BilibiliClient {
    pub fn new(config: &BilibiliConfig) -> Result<Self> {
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
                TuneWeaveError::invalid_request("Bilibili proxy URL is invalid")
                    .with_platform(Platform::Bilibili)
            })?;
            builder = builder.proxy(proxy);
        }
        let http = builder.build().map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "failed to build Bilibili HTTP client",
            )
            .with_platform(Platform::Bilibili)
        })?;
        Ok(Self {
            http,
            web_state: Arc::new(Mutex::new(BilibiliWebState::default())),
        })
    }

    pub(crate) async fn create_qr_login(&self) -> Result<BilibiliQrStart> {
        let response = self
            .http
            .get(PASSPORT_QR_GENERATE_ENDPOINT)
            .send()
            .await
            .map_err(passport_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(passport_http_error(status));
        }
        let bytes = response.bytes().await.map_err(passport_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili QR response exceeded the size limit",
            ));
        }
        parse_qr_generate_response(&bytes)
    }

    pub(crate) async fn poll_qr_login(&self, qrcode_key: &str) -> Result<BilibiliQrPoll> {
        validate_qrcode_key(qrcode_key)?;
        let mut endpoint = Url::parse(PASSPORT_QR_POLL_ENDPOINT).map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "Bilibili QR poll endpoint is invalid",
            )
            .with_platform(Platform::Bilibili)
        })?;
        endpoint
            .query_pairs_mut()
            .append_pair("qrcode_key", qrcode_key)
            .append_pair("source", "main-fe-header");
        let response = self
            .http
            .get(endpoint)
            .send()
            .await
            .map_err(passport_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(passport_http_error(status));
        }
        let headers = response.headers().clone();
        let bytes = response.bytes().await.map_err(passport_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili QR poll response exceeded the size limit",
            ));
        }
        parse_qr_poll_response(&bytes, &headers)
    }

    pub(crate) async fn session_status(
        &self,
        credential: Option<&BilibiliCredential>,
    ) -> Result<BilibiliSessionStatus> {
        let mut request = self.http.get(NAV_ENDPOINT);
        if let Some(credential) = credential {
            request = request.header(COOKIE, credential.cookie_header());
        }
        let response = request.send().await.map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error("Bilibili session endpoint", status));
        }
        let bytes = response.bytes().await.map_err(bilibili_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili session response exceeded the size limit",
            ));
        }
        parse_session_response(&bytes, credential.map(BilibiliCredential::user_id))
    }

    pub(crate) async fn refresh_credential(
        &self,
        credential: &BilibiliCredential,
    ) -> Result<BilibiliCredentialRefresh> {
        let info = self.cookie_refresh_info(credential).await?;
        if !info.refresh {
            let status = require_authenticated_session(
                self.session_status(Some(credential)).await?,
                "Bilibili credential is no longer authenticated",
            )?;
            return Ok(BilibiliCredentialRefresh {
                credential: credential.clone(),
                status,
                refreshed: false,
            });
        }
        if info.timestamp == 0 {
            return Err(bilibili_upstream_error(
                "Bilibili cookie refresh did not return a valid timestamp",
            ));
        }

        let correspond_path = cookie_correspond_path(info.timestamp)?;
        let refresh_csrf = self
            .cookie_refresh_csrf(credential, &correspond_path)
            .await?;
        let refreshed = self.rotate_cookie(credential, &refresh_csrf).await?;
        if refreshed.user_id() != credential.user_id() {
            return Err(bilibili_upstream_error(
                "Bilibili cookie refresh returned a different account identity",
            ));
        }
        self.confirm_cookie_refresh(&refreshed, &credential.refresh_token)
            .await?;
        let status = require_authenticated_session(
            self.session_status(Some(&refreshed)).await?,
            "Bilibili refreshed credential was not authenticated",
        )?;
        Ok(BilibiliCredentialRefresh {
            credential: refreshed,
            status,
            refreshed: true,
        })
    }

    pub(crate) async fn logout(
        &self,
        credential: &BilibiliCredential,
    ) -> Result<BilibiliLogoutOutcome> {
        let response = self
            .http
            .post(LOGOUT_ENDPOINT)
            .header(COOKIE, credential.cookie_header())
            .form(&[("biliCSRF", credential.bili_jct.as_str())])
            .send()
            .await
            .map_err(passport_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(passport_http_error(status));
        }
        let bytes = response.bytes().await.map_err(passport_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili logout response exceeded the size limit",
            ));
        }
        parse_logout_response(&bytes)
    }

    pub(crate) async fn search_suggestions(
        &self,
        keyword: &str,
        credential: Option<&BilibiliCredential>,
    ) -> Result<BilibiliSearchSuggestionSet> {
        validate_search_keyword(keyword)?;
        let device = self.web_device().await?;
        let mut endpoint = Url::parse(SEARCH_SUGGESTION_ENDPOINT).map_err(|_| {
            bilibili_internal_error("Bilibili search suggestion endpoint is invalid")
        })?;
        endpoint
            .query_pairs_mut()
            .append_pair("term", keyword)
            .append_pair("main_ver", "v1")
            .append_pair("highlight", "")
            .append_pair("func", "suggest")
            .append_pair("suggest_type", "accurate")
            .append_pair("sub_type", "tag")
            .append_pair(
                "userid",
                credential.map_or("0", BilibiliCredential::user_id),
            )
            .append_pair("bangumi_acc_num", "1")
            .append_pair("special_acc_num", "1")
            .append_pair("topic_acc_num", "1")
            .append_pair("upuser_acc_num", "1")
            .append_pair("tag_num", "10")
            .append_pair("special_num", "10")
            .append_pair("bangumi_num", "10")
            .append_pair("upuser_num", "3")
            .append_pair("buvid", &device.buvid3)
            .append_pair("spmid", "333.1007");
        let device_cookie = device.cookie_header();
        let cookie_header = credential.map_or(device_cookie.clone(), |credential| {
            format!("{}; {device_cookie}", credential.cookie_header())
        });
        let response = self
            .http
            .get(endpoint)
            .header(COOKIE, cookie_header)
            .header(REFERER, VIDEO_SEARCH_REFERER)
            .send()
            .await
            .map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error("Bilibili search suggestion", status));
        }
        let bytes = response.bytes().await.map_err(bilibili_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili search suggestion response exceeded the size limit",
            ));
        }
        parse_search_suggestion_response(&bytes)
    }

    pub(crate) async fn trending_searches(
        &self,
        credential: Option<&BilibiliCredential>,
    ) -> Result<BilibiliSearchTrending> {
        let context = self
            .signed_web_context(
                &[
                    ("limit".to_owned(), "50".to_owned()),
                    ("platform".to_owned(), "web".to_owned()),
                ],
                credential,
            )
            .await?;
        let endpoint = format!("{SEARCH_TRENDING_ENDPOINT}?{}", context.query);
        let response = self
            .http
            .get(endpoint)
            .header(COOKIE, context.cookie_header)
            .header(REFERER, VIDEO_SEARCH_REFERER)
            .send()
            .await
            .map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error("Bilibili trending search", status));
        }
        let bytes = response.bytes().await.map_err(bilibili_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili trending search response exceeded the size limit",
            ));
        }
        parse_search_trending_response(&bytes)
    }

    pub(crate) async fn search_videos_page(
        &self,
        keyword: &str,
        page: u32,
        filters: BilibiliVideoSearchFilters,
        credential: Option<&BilibiliCredential>,
    ) -> Result<BilibiliVideoSearchPage> {
        validate_search_keyword(keyword)?;
        if !(1..=50).contains(&page) {
            return Err(invalid_bilibili_request(
                "Bilibili video search page must be between 1 and 50",
            ));
        }
        if self.video_search_compatibility_active()? {
            return self
                .search_videos_compatibility_page(keyword, page, filters, credential)
                .await;
        }
        let query_visit_id = self.web_query_visit_id()?;
        let category_id = filters.category_parameter();
        let context = self
            .signed_web_context(
                &[
                    ("__refresh__".to_owned(), "true".to_owned()),
                    ("_extra".to_owned(), String::new()),
                    ("context".to_owned(), String::new()),
                    ("search_type".to_owned(), "video".to_owned()),
                    ("keyword".to_owned(), keyword.to_owned()),
                    ("order".to_owned(), filters.order.parameter().to_owned()),
                    (
                        "duration".to_owned(),
                        filters.duration.parameter().to_owned(),
                    ),
                    ("tids".to_owned(), category_id),
                    ("page".to_owned(), page.to_string()),
                    ("page_size".to_owned(), VIDEO_SEARCH_PAGE_SIZE.to_string()),
                    ("pubtime_begin_s".to_owned(), "0".to_owned()),
                    ("pubtime_end_s".to_owned(), "0".to_owned()),
                    ("from_source".to_owned(), String::new()),
                    ("from_spmid".to_owned(), "333.337".to_owned()),
                    ("platform".to_owned(), "pc".to_owned()),
                    ("highlight".to_owned(), "1".to_owned()),
                    ("single_column".to_owned(), "0".to_owned()),
                    ("qv_id".to_owned(), query_visit_id),
                    ("ad_resource".to_owned(), "5654".to_owned()),
                    ("source_tag".to_owned(), "3".to_owned()),
                    ("gaia_vtoken".to_owned(), String::new()),
                    ("category_id".to_owned(), String::new()),
                    ("dynamic_offset".to_owned(), "0".to_owned()),
                    ("web_roll_page".to_owned(), "0".to_owned()),
                    (
                        "web_location".to_owned(),
                        VIDEO_SEARCH_WEB_LOCATION.to_owned(),
                    ),
                ],
                credential,
            )
            .await?;
        let endpoint = format!("{VIDEO_SEARCH_ENDPOINT}?{}", context.query);
        let bytes = self
            .video_search_response(endpoint, Some(&context.cookie_header))
            .await?;
        match parse_video_search_response(&bytes, page) {
            Ok(result) => Ok(result),
            Err(error) if is_video_search_risk_challenge(&error) => {
                self.mark_video_search_challenged()?;
                self.search_videos_compatibility_page(keyword, page, filters, credential)
                    .await
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) async fn video_view(
        &self,
        identity: &crate::BilibiliVideoIdentity,
        credential: Option<&BilibiliCredential>,
    ) -> Result<BilibiliVideoView> {
        let (parameter, value) = match identity {
            crate::BilibiliVideoIdentity::Aid(aid) => ("aid", aid.to_string()),
            crate::BilibiliVideoIdentity::Bvid(bvid) => ("bvid", bvid.clone()),
            crate::BilibiliVideoIdentity::Episode(_) | crate::BilibiliVideoIdentity::Season(_) => {
                return Err(invalid_bilibili_request(
                    "Bilibili archive details require an AID or BVID",
                ));
            }
        };
        let mut endpoint = Url::parse(VIDEO_DETAIL_ENDPOINT)
            .map_err(|_| bilibili_internal_error("Bilibili video detail endpoint is invalid"))?;
        endpoint.query_pairs_mut().append_pair(parameter, &value);
        let referer = format!("https://www.bilibili.com/video/{value}");
        let mut request = self.http.get(endpoint).header(REFERER, referer);
        if let Some(credential) = credential {
            request = request.header(COOKIE, credential.cookie_header());
        }
        let response = request.send().await.map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error("Bilibili video detail", status));
        }
        let bytes = response.bytes().await.map_err(bilibili_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili video detail response exceeded the size limit",
            ));
        }
        parse_video_view_response(&bytes, identity)
    }

    pub(crate) async fn video_subtitles(
        &self,
        aid: u64,
        bvid: &str,
        cid: u64,
        credential: Option<&BilibiliCredential>,
    ) -> Result<BilibiliSubtitleCatalog> {
        if aid == 0 || cid == 0 {
            return Err(invalid_bilibili_request(
                "Bilibili subtitle identity must be positive",
            ));
        }
        crate::BilibiliVideoIdentity::parse(bvid).map_err(|_| {
            invalid_bilibili_request("Bilibili subtitle request contains an invalid BVID")
        })?;
        let context = self
            .signed_web_context(
                &[
                    ("aid".to_owned(), aid.to_string()),
                    ("bvid".to_owned(), bvid.to_owned()),
                    ("cid".to_owned(), cid.to_string()),
                ],
                credential,
            )
            .await?;
        let endpoint = format!("{VIDEO_PLAYER_ENDPOINT}?{}", context.query);
        let response = self
            .http
            .get(endpoint)
            .header(COOKIE, context.cookie_header)
            .header(REFERER, format!("https://www.bilibili.com/video/{bvid}"))
            .send()
            .await
            .map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error("Bilibili subtitle catalog", status));
        }
        let bytes = response.bytes().await.map_err(bilibili_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili subtitle catalog response exceeded the size limit",
            ));
        }
        parse_video_subtitle_catalog(&bytes, aid, bvid, cid)
    }

    pub(crate) async fn playback_manifest(
        &self,
        aid: u64,
        bvid: &str,
        cid: u64,
        audio_language: Option<&str>,
        credential: Option<&BilibiliCredential>,
    ) -> Result<BilibiliPlaybackManifest> {
        if aid == 0 || cid == 0 {
            return Err(invalid_bilibili_request(
                "Bilibili playback AID and CID must be positive",
            ));
        }
        if !matches!(
            crate::BilibiliVideoIdentity::parse(bvid),
            Ok(crate::BilibiliVideoIdentity::Bvid(ref value)) if value == bvid
        ) {
            return Err(invalid_bilibili_request(
                "Bilibili playback request contains an invalid BVID",
            ));
        }
        let audio_language = audio_language
            .map(validate_playback_language_parameter)
            .transpose()?;
        let mut parameters = vec![
            ("avid".to_owned(), aid.to_string()),
            ("cid".to_owned(), cid.to_string()),
            ("fnval".to_owned(), "4048".to_owned()),
            ("fnver".to_owned(), "0".to_owned()),
            ("fourk".to_owned(), "1".to_owned()),
            ("from_client".to_owned(), "BROWSER".to_owned()),
            ("otype".to_owned(), "json".to_owned()),
            ("qn".to_owned(), "127".to_owned()),
            ("support_multi_audio".to_owned(), "true".to_owned()),
        ];
        if credential.is_none() {
            parameters.push(("try_look".to_owned(), "1".to_owned()));
        }
        if let Some(audio_language) = audio_language.as_deref() {
            parameters.push(("cur_language".to_owned(), audio_language.to_owned()));
        }
        let context = self.signed_web_context(&parameters, credential).await?;
        let endpoint = format!("{VIDEO_PLAYBACK_ENDPOINT}?{}", context.query);
        let mut response = self
            .http
            .get(endpoint)
            .header(REFERER, format!("https://www.bilibili.com/video/{bvid}"))
            .header(COOKIE, context.cookie_header)
            .send()
            .await
            .map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error("Bilibili playback manifest", status));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_SUBTITLE_RESPONSE_BYTES as u64)
        {
            return Err(bilibili_upstream_error(
                "Bilibili playback manifest response exceeded the size limit",
            ));
        }
        let mut bytes = Vec::with_capacity(
            response
                .content_length()
                .and_then(|length| usize::try_from(length).ok())
                .unwrap_or_default()
                .min(MAX_SUBTITLE_RESPONSE_BYTES),
        );
        while let Some(chunk) = response.chunk().await.map_err(bilibili_network_error)? {
            if bytes.len().saturating_add(chunk.len()) > MAX_SUBTITLE_RESPONSE_BYTES {
                return Err(bilibili_upstream_error(
                    "Bilibili playback manifest response exceeded the size limit",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        parse_playback_manifest(&bytes, aid, bvid, cid)
    }

    pub(crate) async fn subtitle_body(
        &self,
        resource_url: &Url,
        bvid: &str,
    ) -> Result<BilibiliSubtitleBody> {
        let resource_url = normalize_bilibili_subtitle_url(resource_url.as_str())?;
        if !matches!(
            crate::BilibiliVideoIdentity::parse(bvid),
            Ok(crate::BilibiliVideoIdentity::Bvid(ref value)) if value == bvid
        ) {
            return Err(invalid_bilibili_request(
                "Bilibili subtitle body request contains an invalid BVID",
            ));
        }
        let mut response = self
            .http
            .get(resource_url)
            .header(REFERER, format!("https://www.bilibili.com/video/{bvid}"))
            .send()
            .await
            .map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error("Bilibili subtitle body", status));
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_SUBTITLE_RESPONSE_BYTES as u64)
        {
            return Err(bilibili_upstream_error(
                "Bilibili subtitle body response exceeded the size limit",
            ));
        }
        let mut bytes = Vec::with_capacity(
            response
                .content_length()
                .and_then(|length| usize::try_from(length).ok())
                .unwrap_or_default()
                .min(MAX_SUBTITLE_RESPONSE_BYTES),
        );
        while let Some(chunk) = response.chunk().await.map_err(bilibili_network_error)? {
            if bytes.len().saturating_add(chunk.len()) > MAX_SUBTITLE_RESPONSE_BYTES {
                return Err(bilibili_upstream_error(
                    "Bilibili subtitle body response exceeded the size limit",
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        parse_subtitle_body(&bytes)
    }

    pub(crate) async fn created_favorite_folders(
        &self,
        owner_id: u64,
        credential: Option<&BilibiliCredential>,
    ) -> Result<BilibiliCreatedFavoriteFolders> {
        if owner_id == 0 {
            return Err(invalid_bilibili_request(
                "Bilibili favorite folder owner ID must be positive",
            ));
        }
        let mut endpoint = Url::parse(CREATED_FAVORITE_FOLDERS_ENDPOINT).map_err(|_| {
            bilibili_internal_error("Bilibili created favorite folders endpoint is invalid")
        })?;
        endpoint
            .query_pairs_mut()
            .append_pair("up_mid", &owner_id.to_string())
            .append_pair("type", "2")
            .append_pair("web_location", FAVORITE_FOLDER_WEB_LOCATION);
        let referer = format!("https://space.bilibili.com/{owner_id}/favlist");
        let mut request = self.http.get(endpoint).header(REFERER, referer);
        if let Some(credential) = credential {
            request = request.header(COOKIE, credential.cookie_header());
        }
        let response = request.send().await.map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error(
                "Bilibili created favorite folders",
                status,
            ));
        }
        let bytes = response.bytes().await.map_err(bilibili_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili created favorite folders response exceeded the size limit",
            ));
        }
        parse_created_favorite_folders_response(&bytes, owner_id)
    }

    pub(crate) async fn favorite_folder(
        &self,
        media_id: u64,
        credential: Option<&BilibiliCredential>,
    ) -> Result<BilibiliFavoriteFolder> {
        if media_id == 0 {
            return Err(invalid_bilibili_request(
                "Bilibili favorite folder media ID must be positive",
            ));
        }
        let mut endpoint = Url::parse(FAVORITE_FOLDER_DETAIL_ENDPOINT)
            .map_err(|_| bilibili_internal_error("Bilibili favorite folder endpoint is invalid"))?;
        endpoint
            .query_pairs_mut()
            .append_pair("media_id", &media_id.to_string());
        let mut request = self.http.get(endpoint).header(REFERER, WEB_REFERER);
        if let Some(credential) = credential {
            request = request.header(COOKIE, credential.cookie_header());
        }
        let response = request.send().await.map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error("Bilibili favorite folder", status));
        }
        let bytes = response.bytes().await.map_err(bilibili_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili favorite folder response exceeded the size limit",
            ));
        }
        parse_favorite_folder_response(&bytes, media_id)
    }

    pub(crate) async fn favorite_media_page(
        &self,
        media_id: u64,
        page: u32,
        credential: Option<&BilibiliCredential>,
    ) -> Result<BilibiliFavoriteMediaPage> {
        if media_id == 0 || page == 0 {
            return Err(invalid_bilibili_request(
                "Bilibili favorite folder media ID and page must be positive",
            ));
        }
        let mut endpoint = Url::parse(FAVORITE_FOLDER_MEDIA_ENDPOINT)
            .map_err(|_| bilibili_internal_error("Bilibili favorite media endpoint is invalid"))?;
        endpoint
            .query_pairs_mut()
            .append_pair("media_id", &media_id.to_string())
            .append_pair("pn", &page.to_string())
            .append_pair("ps", &FAVORITE_MEDIA_PAGE_SIZE.to_string())
            .append_pair("order", "mtime")
            .append_pair("type", "0")
            .append_pair("tid", "0")
            .append_pair("keyword", "")
            .append_pair("platform", "web");
        let mut request = self.http.get(endpoint).header(REFERER, WEB_REFERER);
        if let Some(credential) = credential {
            request = request.header(COOKIE, credential.cookie_header());
        }
        let response = request.send().await.map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error("Bilibili favorite media", status));
        }
        let bytes = response.bytes().await.map_err(bilibili_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili favorite media response exceeded the size limit",
            ));
        }
        parse_favorite_media_response(&bytes, media_id, page, FAVORITE_MEDIA_PAGE_SIZE)
    }

    pub(crate) async fn collected_playlists_page(
        &self,
        user_id: u64,
        page: u32,
        credential: Option<&BilibiliCredential>,
    ) -> Result<BilibiliCollectedPlaylistPage> {
        if user_id == 0 || page == 0 {
            return Err(invalid_bilibili_request(
                "Bilibili collected playlist user ID and page must be positive",
            ));
        }
        let mut endpoint = Url::parse(COLLECTED_PLAYLISTS_ENDPOINT).map_err(|_| {
            bilibili_internal_error("Bilibili collected playlists endpoint is invalid")
        })?;
        endpoint
            .query_pairs_mut()
            .append_pair("up_mid", &user_id.to_string())
            .append_pair("ps", &COLLECTED_PLAYLIST_PAGE_SIZE.to_string())
            .append_pair("pn", &page.to_string())
            .append_pair("platform", "web");
        let referer = format!("https://space.bilibili.com/{user_id}/favlist");
        let mut request = self.http.get(endpoint).header(REFERER, referer);
        if let Some(credential) = credential {
            request = request.header(COOKIE, credential.cookie_header());
        }
        let response = request.send().await.map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error("Bilibili collected playlists", status));
        }
        let bytes = response.bytes().await.map_err(bilibili_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili collected playlists response exceeded the size limit",
            ));
        }
        parse_collected_playlists_response(&bytes, page, COLLECTED_PLAYLIST_PAGE_SIZE)
    }

    pub(crate) async fn space_playlists_page(
        &self,
        user_id: u64,
        page: u32,
        credential: Option<&BilibiliCredential>,
    ) -> Result<BilibiliSpacePlaylistPage> {
        if user_id == 0 || page == 0 {
            return Err(invalid_bilibili_request(
                "Bilibili space playlist user ID and page must be positive",
            ));
        }
        let context = self
            .signed_web_context(
                &[
                    ("mid".to_owned(), user_id.to_string()),
                    ("page_num".to_owned(), page.to_string()),
                    ("page_size".to_owned(), SPACE_PLAYLIST_PAGE_SIZE.to_string()),
                    (
                        "web_location".to_owned(),
                        SPACE_PLAYLIST_WEB_LOCATION.to_owned(),
                    ),
                ],
                credential,
            )
            .await?;
        let endpoint = format!("{SPACE_PLAYLISTS_ENDPOINT}?{}", context.query);
        let referer = format!("https://space.bilibili.com/{user_id}/lists");
        let response = self
            .http
            .get(endpoint)
            .header(REFERER, referer)
            .header(COOKIE, context.cookie_header)
            .send()
            .await
            .map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error("Bilibili space playlists", status));
        }
        let bytes = response.bytes().await.map_err(bilibili_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili space playlists response exceeded the size limit",
            ));
        }
        parse_space_playlists_response(&bytes, user_id, page, SPACE_PLAYLIST_PAGE_SIZE)
    }

    pub(crate) async fn season_archives_page(
        &self,
        season_id: u64,
        page: u32,
        credential: Option<&BilibiliCredential>,
    ) -> Result<BilibiliSeasonArchivePage> {
        if season_id == 0 || page == 0 {
            return Err(invalid_bilibili_request(
                "Bilibili season ID and page must be positive",
            ));
        }
        let context = self
            .signed_web_context(
                &[
                    ("mid".to_owned(), "0".to_owned()),
                    ("season_id".to_owned(), season_id.to_string()),
                    ("sort_reverse".to_owned(), "false".to_owned()),
                    ("page_num".to_owned(), page.to_string()),
                    ("page_size".to_owned(), SEASON_ARCHIVE_PAGE_SIZE.to_string()),
                    (
                        "web_location".to_owned(),
                        SPACE_PLAYLIST_WEB_LOCATION.to_owned(),
                    ),
                ],
                credential,
            )
            .await?;
        let endpoint = format!("{SEASON_ARCHIVES_ENDPOINT}?{}", context.query);
        let response = self
            .http
            .get(endpoint)
            .header(REFERER, WEB_REFERER)
            .header(COOKIE, context.cookie_header)
            .send()
            .await
            .map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error("Bilibili season archives", status));
        }
        let bytes = response.bytes().await.map_err(bilibili_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili season archives response exceeded the size limit",
            ));
        }
        parse_season_archives_response(&bytes, season_id, page, SEASON_ARCHIVE_PAGE_SIZE)
    }

    async fn search_videos_compatibility_page(
        &self,
        keyword: &str,
        page: u32,
        filters: BilibiliVideoSearchFilters,
        credential: Option<&BilibiliCredential>,
    ) -> Result<BilibiliVideoSearchPage> {
        let mut endpoint = Url::parse(VIDEO_SEARCH_COMPATIBILITY_ENDPOINT).map_err(|_| {
            bilibili_internal_error("Bilibili video search compatibility endpoint is invalid")
        })?;
        let category_id = filters.category_parameter();
        endpoint
            .query_pairs_mut()
            .append_pair("search_type", "video")
            .append_pair("keyword", keyword)
            .append_pair("order", filters.order.parameter())
            .append_pair("duration", filters.duration.parameter())
            .append_pair("tids", &category_id)
            .append_pair("page", &page.to_string());
        let cookie_header = credential.map(BilibiliCredential::cookie_header);
        let bytes = self
            .video_search_response(endpoint.to_string(), cookie_header.as_deref())
            .await?;
        parse_video_search_response(&bytes, page)
    }

    async fn video_search_response(
        &self,
        endpoint: String,
        cookie_header: Option<&str>,
    ) -> Result<Vec<u8>> {
        let mut request = self
            .http
            .get(endpoint)
            .header(REFERER, VIDEO_SEARCH_REFERER);
        if let Some(cookie_header) = cookie_header {
            request = request.header(COOKIE, cookie_header);
        }
        let response = request.send().await.map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error("Bilibili video search", status));
        }
        let bytes = response.bytes().await.map_err(bilibili_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili video search response exceeded the size limit",
            ));
        }
        Ok(bytes.to_vec())
    }

    fn video_search_compatibility_active(&self) -> Result<bool> {
        let mut state = self.web_state()?;
        let Some(challenged_at) = state.video_search_challenged_at else {
            return Ok(false);
        };
        if challenged_at.elapsed() < VIDEO_SEARCH_COMPATIBILITY_LIFETIME {
            return Ok(true);
        }
        state.video_search_challenged_at = None;
        Ok(false)
    }

    fn mark_video_search_challenged(&self) -> Result<()> {
        self.web_state()?.video_search_challenged_at = Some(Instant::now());
        Ok(())
    }

    fn web_query_visit_id(&self) -> Result<String> {
        let mut state = self.web_state()?;
        if let Some(query_visit_id) = &state.query_visit_id {
            return Ok(query_visit_id.clone());
        }
        const ALPHABET: &[u8; 62] =
            b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
        let query_visit_id = rand::random::<[u8; 32]>()
            .into_iter()
            .map(|byte| char::from(ALPHABET[usize::from(byte) % ALPHABET.len()]))
            .collect::<String>();
        state.query_visit_id = Some(query_visit_id.clone());
        Ok(query_visit_id)
    }

    async fn signed_web_context(
        &self,
        parameters: &[(String, String)],
        credential: Option<&BilibiliCredential>,
    ) -> Result<BilibiliWebRequestContext> {
        let device = self.web_device().await?;
        let ticket = self.web_ticket(credential, &device).await?;
        let keys = self.wbi_keys(credential, &device).await?;
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| bilibili_internal_error("system time precedes the Unix epoch"))?
            .as_secs();
        let query = keys.sign(parameters, timestamp)?;
        let device_cookie = format!("{}; bili_ticket={}", device.cookie_header(), ticket.ticket);
        let cookie_header = credential.map_or(device_cookie.clone(), |credential| {
            format!("{}; {device_cookie}", credential.cookie_header())
        });
        Ok(BilibiliWebRequestContext {
            query,
            cookie_header,
        })
    }

    async fn web_device(&self) -> Result<BilibiliWebDevice> {
        if let Some(device) = self.web_state()?.device.clone() {
            return Ok(device);
        }
        let response = self
            .http
            .get(DEVICE_IDENTITY_ENDPOINT)
            .header(REFERER, WEB_REFERER)
            .send()
            .await
            .map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error(
                "Bilibili device identity endpoint",
                status,
            ));
        }
        let bytes = response.bytes().await.map_err(bilibili_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili device identity response exceeded the size limit",
            ));
        }
        let mut device = parse_device_identity_response(&bytes)?;
        device.b_nut = self.web_cookie_timestamp(&device).await?;
        self.web_state()?.device = Some(device.clone());
        Ok(device)
    }

    async fn web_cookie_timestamp(&self, device: &BilibiliWebDevice) -> Result<String> {
        let response = self
            .http
            .head(WEB_HOME_ENDPOINT)
            .header(COOKIE, device.cookie_header())
            .send()
            .await
            .map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error(
                "Bilibili web identity endpoint",
                status,
            ));
        }
        let value = response
            .headers()
            .get_all(SET_COOKIE)
            .iter()
            .filter_map(|header| header.to_str().ok())
            .filter_map(|header| header.split(';').next())
            .filter_map(|pair| pair.split_once('='))
            .find_map(|(name, value)| (name == "b_nut").then(|| value.to_owned()))
            .ok_or_else(|| bilibili_upstream_error("Bilibili web identity did not return b_nut"))?;
        if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(bilibili_upstream_error(
                "Bilibili web identity returned an invalid b_nut",
            ));
        }
        Ok(value)
    }

    async fn web_ticket(
        &self,
        credential: Option<&BilibiliCredential>,
        device: &BilibiliWebDevice,
    ) -> Result<CachedWebTicket> {
        if let Some(ticket) = self.web_state()?.ticket.clone()
            && ticket.is_current()
        {
            return Ok(ticket);
        }
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| bilibili_internal_error("system time precedes the Unix epoch"))?
            .as_secs();
        let key = hmac::Key::new(hmac::HMAC_SHA256, WEB_TICKET_HMAC_KEY);
        let signature = hex::encode(hmac::sign(&key, format!("ts{timestamp}").as_bytes()));
        let mut endpoint = Url::parse(WEB_TICKET_ENDPOINT)
            .map_err(|_| bilibili_internal_error("Bilibili web ticket endpoint is invalid"))?;
        endpoint
            .query_pairs_mut()
            .append_pair("key_id", "ec02")
            .append_pair("hexsign", &signature)
            .append_pair("context[ts]", &timestamp.to_string())
            .append_pair(
                "csrf",
                credential.map_or("", |credential| credential.bili_jct.as_str()),
            );
        let device_cookie = device.cookie_header();
        let cookie_header = credential.map_or(device_cookie.clone(), |credential| {
            format!("{}; {device_cookie}", credential.cookie_header())
        });
        let response = self
            .http
            .post(endpoint)
            .header(COOKIE, cookie_header)
            .header(REFERER, WEB_REFERER)
            .send()
            .await
            .map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error("Bilibili web ticket endpoint", status));
        }
        let bytes = response.bytes().await.map_err(bilibili_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili web ticket response exceeded the size limit",
            ));
        }
        let (ticket, keys) = parse_web_ticket_response(&bytes)?;
        let mut state = self.web_state()?;
        state.ticket = Some(ticket.clone());
        state.wbi = Some(CachedWbiKeys {
            keys,
            cached_at: Instant::now(),
        });
        Ok(ticket)
    }

    async fn wbi_keys(
        &self,
        credential: Option<&BilibiliCredential>,
        device: &BilibiliWebDevice,
    ) -> Result<WbiKeys> {
        if let Some(cached) = self.web_state()?.wbi.clone()
            && cached.cached_at.elapsed() < WBI_CACHE_LIFETIME
        {
            return Ok(cached.keys);
        }
        let device_cookie = device.cookie_header();
        let cookie_header = credential.map_or(device_cookie.clone(), |credential| {
            format!("{}; {device_cookie}", credential.cookie_header())
        });
        let response = self
            .http
            .get(NAV_ENDPOINT)
            .header(COOKIE, cookie_header)
            .header(REFERER, WEB_REFERER)
            .send()
            .await
            .map_err(bilibili_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(bilibili_http_error("Bilibili WBI key endpoint", status));
        }
        let bytes = response.bytes().await.map_err(bilibili_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili WBI key response exceeded the size limit",
            ));
        }
        let keys = parse_wbi_keys_response(&bytes)?;
        self.web_state()?.wbi = Some(CachedWbiKeys {
            keys: keys.clone(),
            cached_at: Instant::now(),
        });
        Ok(keys)
    }

    fn web_state(&self) -> Result<std::sync::MutexGuard<'_, BilibiliWebState>> {
        self.web_state
            .lock()
            .map_err(|_| bilibili_internal_error("Bilibili web identity cache is unavailable"))
    }

    async fn cookie_refresh_info(&self, credential: &BilibiliCredential) -> Result<CookieInfoData> {
        let response = self
            .http
            .get(COOKIE_INFO_ENDPOINT)
            .header(COOKIE, credential.cookie_header())
            .send()
            .await
            .map_err(passport_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(passport_http_error(status));
        }
        let bytes = response.bytes().await.map_err(passport_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili cookie status response exceeded the size limit",
            ));
        }
        parse_cookie_info_response(&bytes)
    }

    async fn cookie_refresh_csrf(
        &self,
        credential: &BilibiliCredential,
        correspond_path: &str,
    ) -> Result<String> {
        validate_correspond_path(correspond_path)?;
        let endpoint = format!("{COOKIE_CORRESPOND_ENDPOINT}{correspond_path}");
        let response = self
            .http
            .get(endpoint)
            .header(COOKIE, credential.cookie_header())
            .send()
            .await
            .map_err(passport_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(passport_http_error(status));
        }
        let bytes = response.bytes().await.map_err(passport_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili cookie correspondence response exceeded the size limit",
            ));
        }
        parse_refresh_csrf_html(&bytes)
    }

    async fn rotate_cookie(
        &self,
        credential: &BilibiliCredential,
        refresh_csrf: &str,
    ) -> Result<BilibiliCredential> {
        validate_refresh_csrf(refresh_csrf)?;
        let response = self
            .http
            .post(COOKIE_REFRESH_ENDPOINT)
            .header(COOKIE, credential.cookie_header())
            .form(&[
                ("csrf", credential.bili_jct.as_str()),
                ("refresh_csrf", refresh_csrf),
                ("source", "main_web"),
                ("refresh_token", credential.refresh_token.as_str()),
            ])
            .send()
            .await
            .map_err(passport_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(passport_http_error(status));
        }
        let headers = response.headers().clone();
        let bytes = response.bytes().await.map_err(passport_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili cookie refresh response exceeded the size limit",
            ));
        }
        parse_cookie_refresh_response(&bytes, &headers, credential.user_id())
    }

    async fn confirm_cookie_refresh(
        &self,
        credential: &BilibiliCredential,
        previous_refresh_token: &str,
    ) -> Result<()> {
        let response = self
            .http
            .post(COOKIE_CONFIRM_ENDPOINT)
            .header(COOKIE, credential.cookie_header())
            .form(&[
                ("csrf", credential.bili_jct.as_str()),
                ("refresh_token", previous_refresh_token),
            ])
            .send()
            .await
            .map_err(passport_network_error)?;
        let status = response.status();
        if !status.is_success() {
            return Err(passport_http_error(status));
        }
        let bytes = response.bytes().await.map_err(passport_network_error)?;
        if bytes.len() > MAX_PASSPORT_RESPONSE_BYTES {
            return Err(bilibili_upstream_error(
                "Bilibili cookie confirmation response exceeded the size limit",
            ));
        }
        parse_cookie_confirm_response(&bytes)
    }
}

fn parse_device_identity_response(bytes: &[u8]) -> Result<BilibiliWebDevice> {
    let response: PassportResponse<DeviceIdentityData> =
        serde_json::from_slice(bytes).map_err(|_| {
            bilibili_upstream_error("Bilibili device identity endpoint returned invalid JSON")
        })?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili device identity",
            response.code,
            &response.message,
        ));
    }
    let data = response.data.ok_or_else(|| {
        bilibili_upstream_error("Bilibili device identity response did not contain data")
    })?;
    validate_cookie_value(&data.b_3, "buvid3", 512)?;
    validate_cookie_value(&data.b_4, "buvid4", 1024)?;
    Ok(BilibiliWebDevice {
        buvid3: data.b_3,
        buvid4: data.b_4,
        b_nut: String::new(),
    })
}

fn parse_web_ticket_response(bytes: &[u8]) -> Result<(CachedWebTicket, WbiKeys)> {
    let response: PassportResponse<WebTicketData> =
        serde_json::from_slice(bytes).map_err(|_| {
            bilibili_upstream_error("Bilibili web ticket endpoint returned invalid JSON")
        })?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili web ticket request",
            response.code,
            &response.message,
        ));
    }
    let data = response.data.ok_or_else(|| {
        bilibili_upstream_error("Bilibili web ticket response did not contain data")
    })?;
    if data.created_at == 0
        || data.ttl <= WEB_TICKET_EXPIRY_MARGIN.as_secs()
        || data.ttl > 7 * 24 * 60 * 60
        || data.ticket.len() < 64
        || data.ticket.len() > 4096
        || data.ticket.split('.').count() != 3
        || data
            .ticket
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && !matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(bilibili_upstream_error(
            "Bilibili web ticket response contained invalid credentials",
        ));
    }
    let keys = WbiKeys::from_image_urls(&data.nav.img, &data.nav.sub)
        .map_err(|_| bilibili_upstream_error("Bilibili web ticket returned invalid WBI keys"))?;
    Ok((
        CachedWebTicket {
            ticket: data.ticket,
            cached_at: Instant::now(),
            lifetime: Duration::from_secs(data.ttl),
        },
        keys,
    ))
}

fn parse_search_suggestion_response(bytes: &[u8]) -> Result<BilibiliSearchSuggestionSet> {
    let response: SearchSuggestionResponse = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili search suggestion returned invalid JSON"))?;
    if !matches!(response.code, 0 | 3) {
        return Err(platform_business_error(
            "Bilibili search suggestion",
            response.code,
            "",
        ));
    }
    let (items, result_extensions) = response
        .result
        .map(|result| (result.tag.unwrap_or_default(), result.extra))
        .unwrap_or_default();
    if items.len() > 10 {
        return Err(bilibili_upstream_error(
            "Bilibili search suggestion exceeded the documented result limit",
        ));
    }
    if response.code == 3 && !items.is_empty() {
        return Err(bilibili_upstream_error(
            "Bilibili empty search suggestion response contained results",
        ));
    }
    let reported_total = response
        .total_count
        .map(|total| {
            total.get().filter(|total| *total <= 10).ok_or_else(|| {
                bilibili_upstream_error("Bilibili search suggestion returned an invalid total")
            })
        })
        .transpose()?;
    if reported_total.is_some_and(|total| total < items.len() as u64) {
        return Err(bilibili_upstream_error(
            "Bilibili search suggestion total was smaller than its results",
        ));
    }
    let suggestions = items
        .into_iter()
        .map(map_search_suggestion_item)
        .collect::<Result<Vec<_>>>()?;
    Ok(BilibiliSearchSuggestionSet {
        response_code: response.code,
        experiment: optional_search_suggestion_text(
            &response.exp_str,
            "experiment identifier",
            4096,
        )?,
        search_token: optional_search_suggestion_text(&response.stoken, "search token", 4096)?,
        reported_total,
        user_feature: optional_search_suggestion_text(
            &response.user_feature,
            "user feature",
            4096,
        )?,
        suggestions,
        extensions: response.extra,
        result_extensions,
    })
}

fn parse_search_trending_response(bytes: &[u8]) -> Result<BilibiliSearchTrending> {
    let response: PassportResponse<SearchTrendingData> = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili trending search returned invalid JSON"))?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili trending search",
            response.code,
            &response.message,
        ));
    }
    let data = response.data.ok_or_else(|| {
        bilibili_upstream_error("Bilibili trending search response did not contain data")
    })?;
    if data.trending.list.len() > 50 || data.trending.top_list.len() > 50 {
        return Err(bilibili_upstream_error(
            "Bilibili trending search exceeded the requested limit",
        ));
    }
    let title =
        validated_bilibili_text(&data.trending.title, "trending search catalog title", 1024)?;
    let track_id = optional_bounded_text(&data.trending.trackid, "trending search track ID", 512)?;
    let entries = data
        .trending
        .list
        .into_iter()
        .map(map_search_trending_item)
        .collect::<Result<Vec<_>>>()?;
    Ok(BilibiliSearchTrending {
        title,
        track_id,
        entries,
        top_list: data.trending.top_list,
        extensions: data.extra,
        trending_extensions: data.trending.extra,
    })
}

fn map_search_trending_item(item: SearchTrendingItem) -> Result<BilibiliSearchTrendingEntry> {
    let keyword = validated_bilibili_text(&item.keyword, "trending search keyword", 1024)?;
    let display_text = if item.show_name.trim().is_empty() {
        keyword.clone()
    } else {
        validated_bilibili_text(&item.show_name, "trending search display text", 2048)?
    };
    let heat_score = item
        .heat_score
        .map(|score| {
            score.get().ok_or_else(|| {
                bilibili_upstream_error("Bilibili trending search returned an invalid heat score")
            })
        })
        .transpose()?;
    if item.word_type.is_some_and(|word_type| word_type < 0) {
        return Err(bilibili_upstream_error(
            "Bilibili trending search returned an invalid word type",
        ));
    }
    Ok(BilibiliSearchTrendingEntry {
        keyword,
        display_text,
        icon_url: normalize_bilibili_image_url(&item.icon, "trending search icon")?,
        action_uri: normalize_bilibili_action_uri(&item.uri)?,
        action_kind: optional_bounded_text(&item.goto, "trending search action kind", 256)?,
        heat_score,
        word_type: item.word_type,
        extensions: item.extra,
    })
}

fn normalize_bilibili_action_uri(value: &str) -> Result<Option<String>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > 4096 || value.chars().any(char::is_control) {
        return Err(bilibili_upstream_error(
            "Bilibili trending search returned an invalid action URI",
        ));
    }
    let value = value
        .strip_prefix("http://")
        .map_or_else(|| value.to_owned(), |rest| format!("https://{rest}"));
    let uri = Url::parse(&value).map_err(|_| {
        bilibili_upstream_error("Bilibili trending search returned an invalid action URI")
    })?;
    let safe = match uri.scheme() {
        "https" => uri.host_str().is_some_and(|host| {
            host == "bilibili.com"
                || host.ends_with(".bilibili.com")
                || host == "b23.tv"
                || host.ends_with(".b23.tv")
        }),
        "bilibili" => uri.host_str().is_some(),
        _ => false,
    };
    if !safe || uri.port().is_some() || !uri.username().is_empty() || uri.password().is_some() {
        return Err(bilibili_upstream_error(
            "Bilibili trending search returned an unsafe action URI",
        ));
    }
    Ok(Some(value))
}

fn map_search_suggestion_item(item: SearchSuggestionItem) -> Result<BilibiliSearchSuggestion> {
    let term = optional_search_suggestion_text(&item.term, "term", 1024)?;
    let keyword_source = if item.value.trim().is_empty() {
        term.as_deref().unwrap_or_default()
    } else {
        &item.value
    };
    let keyword = validated_search_suggestion_text(keyword_source, "keyword", 1024)?;
    let (display_text, highlight_ranges) = if item.name.trim().is_empty() {
        (keyword.clone(), Vec::new())
    } else {
        parse_search_suggestion_display(&item.name)?
    };
    let reference = item.r#ref.get().ok_or_else(|| {
        bilibili_upstream_error("Bilibili search suggestion returned an invalid reference")
    })?;
    let spid = item.spid.get().ok_or_else(|| {
        bilibili_upstream_error("Bilibili search suggestion returned an invalid SPID")
    })?;
    Ok(BilibiliSearchSuggestion {
        keyword,
        term,
        display_text,
        highlight_ranges,
        reference,
        spid,
        suggestion_type: optional_search_suggestion_text(&item.r#type, "suggestion type", 256)?,
        item_feature: optional_search_suggestion_text(&item.item_feature, "item feature", 4096)?,
        extensions: item.extra,
    })
}

fn parse_search_suggestion_display(value: &str) -> Result<(String, Vec<BilibiliTextRange>)> {
    const HIGHLIGHT_OPEN: &str = "<em class=\"suggest_high_light\">";
    const HIGHLIGHT_CLOSE: &str = "</em>";

    let mut remaining = value.trim();
    let mut display = String::with_capacity(remaining.len());
    let mut ranges = Vec::new();
    let mut highlight_start = None;
    while !remaining.is_empty() {
        if let Some(rest) = remaining.strip_prefix(HIGHLIGHT_OPEN) {
            if highlight_start.is_some() {
                return Err(bilibili_upstream_error(
                    "Bilibili search suggestion returned nested highlight markup",
                ));
            }
            highlight_start = Some(display.chars().count());
            remaining = rest;
            continue;
        }
        if let Some(rest) = remaining.strip_prefix(HIGHLIGHT_CLOSE) {
            let start = highlight_start.take().ok_or_else(|| {
                bilibili_upstream_error(
                    "Bilibili search suggestion returned unmatched highlight markup",
                )
            })?;
            let end = display.chars().count();
            if start == end {
                return Err(bilibili_upstream_error(
                    "Bilibili search suggestion returned an empty highlight",
                ));
            }
            ranges.push(BilibiliTextRange { start, end });
            remaining = rest;
            continue;
        }
        if let Some((decoded, consumed)) = decode_search_suggestion_entity(remaining) {
            display.push(decoded);
            remaining = &remaining[consumed..];
            continue;
        }
        let character = remaining
            .chars()
            .next()
            .expect("non-empty search suggestion display");
        if character == '<' || character.is_control() {
            return Err(bilibili_upstream_error(
                "Bilibili search suggestion returned unsafe display markup",
            ));
        }
        display.push(character);
        remaining = &remaining[character.len_utf8()..];
    }
    if highlight_start.is_some() {
        return Err(bilibili_upstream_error(
            "Bilibili search suggestion returned unclosed highlight markup",
        ));
    }
    let display = validated_search_suggestion_text(&display, "display text", 2048)?;
    Ok((display, ranges))
}

fn decode_search_suggestion_entity(value: &str) -> Option<(char, usize)> {
    for (entity, decoded) in [
        ("&amp;", '&'),
        ("&lt;", '<'),
        ("&gt;", '>'),
        ("&quot;", '"'),
        ("&#39;", '\''),
        ("&apos;", '\''),
    ] {
        if value.starts_with(entity) {
            return Some((decoded, entity.len()));
        }
    }
    let scan_end = value.len().min(16);
    let end = value.get(..scan_end)?.find(';')?;
    let entity = value.get(2..end)?;
    let code_point = if value.starts_with("&#x") || value.starts_with("&#X") {
        u32::from_str_radix(value.get(3..end)?, 16).ok()?
    } else if value.starts_with("&#") {
        entity.parse().ok()?
    } else {
        return None;
    };
    let character = char::from_u32(code_point)?;
    (!character.is_control()).then_some((character, end + 1))
}

fn optional_search_suggestion_text(
    value: &str,
    context: &str,
    limit: usize,
) -> Result<Option<String>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    validated_search_suggestion_text(value, context, limit).map(Some)
}

fn validated_search_suggestion_text(value: &str, context: &str, limit: usize) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > limit || value.chars().any(char::is_control) {
        return Err(bilibili_upstream_error(format!(
            "Bilibili search suggestion returned an invalid {context}"
        )));
    }
    Ok(value.to_owned())
}

fn parse_video_search_response(
    bytes: &[u8],
    requested_page: u32,
) -> Result<BilibiliVideoSearchPage> {
    let response: PassportResponse<VideoSearchPayload> = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili video search returned invalid JSON"))?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili video search",
            response.code,
            &response.message,
        ));
    }
    let data = match response.data.ok_or_else(|| {
        bilibili_upstream_error("Bilibili video search response did not contain data")
    })? {
        VideoSearchPayload::Results(data) => data,
        VideoSearchPayload::Voucher { v_voucher } => {
            if v_voucher.trim().is_empty() || v_voucher.len() > 4096 {
                return Err(bilibili_upstream_error(
                    "Bilibili video search returned an invalid risk-control challenge",
                ));
            }
            return Err(TuneWeaveError::new(
                ErrorCode::RateLimited,
                "Bilibili video search was challenged",
            )
            .with_platform(Platform::Bilibili)
            .retryable(true)
            .with_details(json!({ "platform_code": 0, "risk_challenge": true })));
        }
    };
    if data.page != requested_page || data.pagesize != VIDEO_SEARCH_PAGE_SIZE || data.num_pages > 50
    {
        return Err(bilibili_upstream_error(
            "Bilibili video search returned inconsistent pagination",
        ));
    }
    let total = data
        .num_results
        .get()
        .filter(|total| *total <= 1_000)
        .ok_or_else(|| {
            bilibili_upstream_error("Bilibili video search returned an invalid total")
        })?;
    let mut videos = Vec::with_capacity(data.result.len());
    for item in data.result {
        videos.push(map_video_search_item(item)?);
    }
    Ok(BilibiliVideoSearchPage {
        page: data.page,
        page_size: data.pagesize,
        total,
        page_count: data.num_pages,
        search_id: validated_search_text(&data.seid.into_string(), "search ID", 256)?,
        videos,
    })
}

fn parse_video_view_response(
    bytes: &[u8],
    requested_identity: &crate::BilibiliVideoIdentity,
) -> Result<BilibiliVideoView> {
    let response: PassportResponse<VideoViewData> = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili video detail returned invalid JSON"))?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili video detail",
            response.code,
            &response.message,
        ));
    }
    let data = response.data.ok_or_else(|| {
        bilibili_upstream_error("Bilibili video detail response did not contain data")
    })?;
    let bvid = match crate::BilibiliVideoIdentity::parse(&data.bvid)
        .map_err(|_| bilibili_upstream_error("Bilibili video detail returned an invalid BVID"))?
    {
        crate::BilibiliVideoIdentity::Bvid(value) => value,
        _ => unreachable!("BVID parser returned another identity type"),
    };
    let identity_matches = match requested_identity {
        crate::BilibiliVideoIdentity::Aid(aid) => *aid == data.aid,
        crate::BilibiliVideoIdentity::Bvid(requested) => requested == &bvid,
        crate::BilibiliVideoIdentity::Episode(_) | crate::BilibiliVideoIdentity::Season(_) => false,
    };
    if !identity_matches
        || data.aid == 0
        || data.videos == 0
        || data.videos > 100_000
        || data.pages.len() as u64 != data.videos
        || data.cid == 0
        || data.tid == 0
        || data.copyright == 0
        || data.copyright > 3
        || data.owner.mid == 0
        || data.stat.aid != data.aid
    {
        return Err(bilibili_upstream_error(
            "Bilibili video detail returned an invalid or conflicting identity",
        ));
    }
    let mut cids = std::collections::BTreeSet::new();
    let mut parts = Vec::with_capacity(data.pages.len());
    for (index, part) in data.pages.into_iter().enumerate() {
        let expected_page = index as u64 + 1;
        if part.cid == 0
            || part.page != expected_page
            || !cids.insert(part.cid)
            || !matches!(part.dimension.rotate, 0 | 1)
            || ((part.dimension.width == 0) != (part.dimension.height == 0))
        {
            return Err(bilibili_upstream_error(
                "Bilibili video detail returned invalid part metadata",
            ));
        }
        parts.push(BilibiliVideoPart {
            cid: part.cid,
            page: part.page,
            source: validated_bilibili_text(&part.source, "video part source", 128)?,
            title: validated_bilibili_text(&part.part, "video part title", 4096)?,
            duration_seconds: part.duration,
            width: part.dimension.width,
            height: part.dimension.height,
            rotated: part.dimension.rotate == 1,
        });
    }
    if parts.first().map(|part| part.cid) != Some(data.cid) {
        return Err(bilibili_upstream_error(
            "Bilibili video detail returned a conflicting first CID",
        ));
    }
    let owner = BilibiliCollectedPlaylistOwner {
        id: data.owner.mid,
        name: validated_bilibili_text(&data.owner.name, "video owner name", 512)?,
        avatar_url: normalize_bilibili_image_url(&data.owner.face, "video owner avatar")?,
    };
    let rights = BilibiliVideoRights {
        download: validated_binary_state(data.rights.download, "video download right")?,
        movie: validated_binary_state(data.rights.movie, "video movie right")?,
        pay: validated_binary_state(data.rights.pay, "video pay right")?,
        high_bitrate: validated_binary_state(data.rights.hd5, "video high bitrate right")?,
        no_reprint: validated_binary_state(data.rights.no_reprint, "video reprint right")?,
        ugc_pay: validated_binary_state(data.rights.ugc_pay, "video UGC pay right")?,
        cooperation: validated_binary_state(data.rights.is_cooperation, "video cooperation right")?,
        interactive: validated_binary_state(data.rights.is_stein_gate, "interactive video right")?,
        panoramic: validated_binary_state(data.rights.is_360, "panoramic video right")?,
        no_share: validated_binary_state(data.rights.no_share, "video share right")?,
        free_watch: validated_binary_state(data.rights.free_watch, "video free watch right")?,
    };
    Ok(BilibiliVideoView {
        aid: data.aid,
        bvid,
        title: validated_bilibili_text(&data.title, "video title", 4096)?,
        description: validated_bilibili_multiline_text(
            &data.desc,
            "video description",
            256 * 1024,
        )?,
        dynamic_text: validated_bilibili_multiline_text(
            &data.dynamic,
            "video dynamic text",
            64 * 1024,
        )?,
        cover_url: normalize_bilibili_image_url(&data.pic, "video cover")?
            .ok_or_else(|| bilibili_upstream_error("Bilibili video detail omitted its cover"))?,
        duration_seconds: data.duration,
        published_at: data.pubdate,
        created_at: data.ctime,
        state: data.state,
        category_id: data.tid,
        category_id_v2: data.tid_v2.filter(|id| *id > 0),
        category_name: optional_bounded_text(&data.tname, "video category name", 512)?,
        category_name_v2: optional_bounded_text(&data.tname_v2, "video category v2 name", 512)?,
        copyright: data.copyright,
        owner,
        stats: BilibiliVideoViewStats {
            view: data.stat.view,
            danmaku: data.stat.danmaku,
            reply: data.stat.reply,
            favorite: data.stat.favorite,
            coin: data.stat.coin,
            share: data.stat.share,
            like: data.stat.like,
            now_rank: data.stat.now_rank,
            his_rank: data.stat.his_rank,
        },
        rights,
        parts,
    })
}

fn parse_video_subtitle_catalog(
    bytes: &[u8],
    requested_aid: u64,
    requested_bvid: &str,
    requested_cid: u64,
) -> Result<BilibiliSubtitleCatalog> {
    let response: PassportResponse<VideoPlayerData> = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili subtitle catalog returned invalid JSON"))?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili subtitle catalog",
            response.code,
            &response.message,
        ));
    }
    let data = response.data.ok_or_else(|| {
        bilibili_upstream_error("Bilibili subtitle catalog response did not contain data")
    })?;
    let bvid = match crate::BilibiliVideoIdentity::parse(&data.bvid).map_err(|_| {
        bilibili_upstream_error("Bilibili subtitle catalog returned an invalid BVID")
    })? {
        crate::BilibiliVideoIdentity::Bvid(value) => value,
        _ => unreachable!("BVID parser returned another identity type"),
    };
    if data.aid != requested_aid
        || bvid != requested_bvid
        || data.cid != requested_cid
        || data.aid == 0
        || data.cid == 0
    {
        return Err(bilibili_upstream_error(
            "Bilibili subtitle catalog returned a conflicting identity",
        ));
    }
    let subtitle = data.subtitle.unwrap_or_default();
    if subtitle.subtitles.len() > 256 {
        return Err(bilibili_upstream_error(
            "Bilibili subtitle catalog exceeded the track limit",
        ));
    }
    let default_language = optional_subtitle_language(&subtitle.lan, "default subtitle language")?;
    let default_language_label =
        optional_bounded_text(&subtitle.lan_doc, "default subtitle language label", 512)?;
    let mut identities = std::collections::BTreeSet::new();
    let mut numeric_identities = std::collections::BTreeSet::new();
    let mut subtitles = Vec::with_capacity(subtitle.subtitles.len());
    for item in subtitle.subtitles {
        let id_string = validate_subtitle_id(&item.id_str)?;
        if item.id == 0
            || !identities.insert(id_string.clone())
            || !numeric_identities.insert(item.id)
        {
            return Err(bilibili_upstream_error(
                "Bilibili subtitle catalog returned duplicate or invalid identities",
            ));
        }
        subtitles.push(BilibiliSubtitle {
            id: item.id,
            id_string,
            language: validate_subtitle_language(&item.lan, "subtitle language")?,
            label: validated_bilibili_text(&item.lan_doc, "subtitle language label", 512)?,
            locked: item.is_lock,
            resource_url: normalize_bilibili_subtitle_url(&item.subtitle_url)?,
            subtitle_type: item.subtitle_type,
            ai_type: item.ai_type,
            ai_status: item.ai_status,
        });
    }
    Ok(BilibiliSubtitleCatalog {
        aid: data.aid,
        bvid,
        cid: data.cid,
        requires_login: data.need_login_subtitle,
        can_submit: subtitle.allow_submit,
        default_language,
        default_language_label,
        subtitles,
    })
}

fn parse_subtitle_body(bytes: &[u8]) -> Result<BilibiliSubtitleBody> {
    let data: SubtitleBodyData = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili subtitle body returned invalid JSON"))?;
    if data.body.len() > 100_000 {
        return Err(bilibili_upstream_error(
            "Bilibili subtitle body exceeded the cue limit",
        ));
    }
    let font_size = validate_optional_subtitle_ratio(data.font_size, "font size", 100.0)?;
    let background_alpha =
        validate_optional_subtitle_ratio(data.background_alpha, "background alpha", 1.0)?;
    let source_language = optional_flexible_subtitle_text(data.lang, "source language", 32)?;
    if let Some(language) = source_language.as_deref() {
        validate_subtitle_language(language, "subtitle body source language")?;
    }
    let source_type = optional_flexible_subtitle_text(data.source_type, "source type", 128)?;
    let source_version = optional_flexible_subtitle_text(data.version, "source version", 128)?;
    let font_color = optional_subtitle_style_text(data.font_color, "font color")?;
    let background_color = optional_subtitle_style_text(data.background_color, "background color")?;
    let stroke = optional_subtitle_style_text(data.stroke, "stroke")?;
    let mut cues = Vec::with_capacity(data.body.len());
    for item in data.body {
        validate_subtitle_seconds(item.start_seconds, "cue start")?;
        validate_subtitle_seconds(item.end_seconds, "cue end")?;
        if item.end_seconds < item.start_seconds {
            return Err(bilibili_upstream_error(
                "Bilibili subtitle body returned a cue with a negative duration",
            ));
        }
        if item.location.is_some_and(|position| position > 1000) {
            return Err(bilibili_upstream_error(
                "Bilibili subtitle body returned an invalid cue position",
            ));
        }
        if item
            .music
            .is_some_and(|confidence| !confidence.is_finite() || !(0.0..=1.0).contains(&confidence))
        {
            return Err(bilibili_upstream_error(
                "Bilibili subtitle body returned an invalid music confidence",
            ));
        }
        cues.push(BilibiliSubtitleCue {
            id: optional_flexible_subtitle_text(item.sid, "cue ID", 64)?,
            start_seconds: item.start_seconds,
            end_seconds: item.end_seconds,
            text: validated_subtitle_cue_text(&item.content)?,
            position: item.location,
            music_confidence: item.music,
        });
    }
    Ok(BilibiliSubtitleBody {
        source_language,
        source_type,
        source_version,
        font_size,
        font_color,
        background_alpha,
        background_color,
        stroke,
        cues,
    })
}

fn parse_playback_manifest(
    bytes: &[u8],
    requested_aid: u64,
    requested_bvid: &str,
    requested_cid: u64,
) -> Result<BilibiliPlaybackManifest> {
    let response: PassportResponse<PlaybackData> = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili playback manifest returned invalid JSON"))?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili playback manifest",
            response.code,
            &response.message,
        ));
    }
    let data = response.data.ok_or_else(|| {
        bilibili_upstream_error("Bilibili playback manifest response did not contain data")
    })?;
    if requested_aid == 0
        || requested_cid == 0
        || data.timelength == 0
        || data.timelength > 30 * 24 * 60 * 60 * 1_000
        || data.quality == 0
    {
        return Err(bilibili_upstream_error(
            "Bilibili playback manifest returned invalid media metadata",
        ));
    }
    let bvid = match crate::BilibiliVideoIdentity::parse(requested_bvid)
        .map_err(|_| bilibili_upstream_error("Bilibili playback manifest used an invalid BVID"))?
    {
        crate::BilibiliVideoIdentity::Bvid(value) if value == requested_bvid => value,
        _ => {
            return Err(bilibili_upstream_error(
                "Bilibili playback manifest used a non-canonical BVID",
            ));
        }
    };
    let format = validated_bilibili_text(&data.format, "playback format", 128)?;
    let accepted_formats = validated_playback_formats(&data.accept_format)?;
    let accepted_qualities = validated_playback_qualities(data.accept_quality)?;
    let descriptions = playback_quality_descriptions(&accepted_qualities, data.accept_description)?;
    let formats = map_playback_formats(data.support_formats, &descriptions)?;
    let seek_parameter = optional_bounded_text(&data.seek_param, "playback seek parameter", 64)?;
    let seek_type = optional_bounded_text(&data.seek_type, "playback seek type", 64)?;
    let selected_audio_language =
        optional_bounded_text(&data.cur_language, "selected playback language", 32)?
            .map(|language| validate_playback_language_parameter(&language))
            .transpose()?;
    let languages = data
        .language
        .map(map_playback_language_catalog)
        .transpose()?;
    let last_play_time_ms = match data.last_play_time {
        Some(value) if value >= 0 => Some(u64::try_from(value).map_err(|_| {
            bilibili_upstream_error("Bilibili playback progress exceeded its supported range")
        })?),
        Some(-1) | None => None,
        Some(_) => {
            return Err(bilibili_upstream_error(
                "Bilibili playback manifest returned invalid playback progress",
            ));
        }
    };
    let last_play_cid = data.last_play_cid.filter(|cid| *cid > 0);

    let mut minimum_buffer_time = None;
    let mut video_tracks = Vec::new();
    let mut audio_tracks = Vec::new();
    let mut dolby_audio_tracks = Vec::new();
    let mut lossless_audio_tracks = Vec::new();
    let mut dolby_type = None;
    let mut lossless_display = None;
    if let Some(dash) = data.dash {
        validate_subtitle_seconds(dash.duration, "DASH duration")?;
        let dash_duration_ms = (dash.duration * 1_000.0).round();
        if dash_duration_ms <= 0.0 || (dash_duration_ms - data.timelength as f64).abs() > 10_000.0 {
            return Err(bilibili_upstream_error(
                "Bilibili playback manifest returned conflicting DASH duration",
            ));
        }
        minimum_buffer_time = reconcile_playback_alias(
            dash.minimum_buffer_time_camel,
            dash.min_buffer_time,
            "minimum buffer time",
        )?;
        if minimum_buffer_time
            .is_some_and(|value| !value.is_finite() || !(0.0..=60.0).contains(&value))
        {
            return Err(bilibili_upstream_error(
                "Bilibili playback manifest returned invalid minimum buffer time",
            ));
        }
        video_tracks = map_playback_tracks(dash.video, true)?;
        audio_tracks = map_playback_tracks(dash.audio.unwrap_or_default(), false)?;
        if let Some(dolby) = dash.dolby {
            dolby_type = dolby.dolby_type.filter(|value| *value > 0);
            dolby_audio_tracks = map_playback_tracks(dolby.audio.unwrap_or_default(), false)?;
        }
        if let Some(lossless) = dash.flac {
            lossless_display = lossless.display;
            if let Some(audio) = lossless.audio {
                lossless_audio_tracks = map_playback_tracks(vec![audio], false)?;
            }
        }
    }
    let progressive_segments = map_progressive_segments(data.durl.unwrap_or_default())?;
    if video_tracks.is_empty()
        && audio_tracks.is_empty()
        && dolby_audio_tracks.is_empty()
        && lossless_audio_tracks.is_empty()
        && progressive_segments.is_empty()
    {
        return Err(bilibili_upstream_error(
            "Bilibili playback manifest returned no playable tracks",
        ));
    }
    let expires_at_epoch_seconds = playback_manifest_expiration(
        &video_tracks,
        &audio_tracks,
        &dolby_audio_tracks,
        &lossless_audio_tracks,
        &progressive_segments,
    )?;
    Ok(BilibiliPlaybackManifest {
        aid: requested_aid,
        bvid,
        cid: requested_cid,
        duration_ms: data.timelength,
        current_quality: data.quality,
        format,
        accepted_formats,
        accepted_qualities,
        formats,
        video_codec_id: data.video_codecid,
        seek_parameter,
        seek_type,
        minimum_buffer_time,
        video_tracks,
        audio_tracks,
        dolby_audio_tracks,
        lossless_audio_tracks,
        dolby_type,
        lossless_display,
        progressive_segments,
        selected_audio_language,
        selected_production_type: data.cur_production_type,
        languages,
        last_play_time_ms,
        last_play_cid,
        expires_at_epoch_seconds,
    })
}

fn validated_playback_formats(value: &str) -> Result<Vec<String>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(Vec::new());
    }
    let formats = value
        .split(',')
        .map(|format| validated_bilibili_text(format, "accepted playback format", 128))
        .collect::<Result<Vec<_>>>()?;
    if formats.len() > 64 {
        return Err(bilibili_upstream_error(
            "Bilibili playback manifest exceeded the accepted format limit",
        ));
    }
    let mut unique = std::collections::BTreeSet::new();
    if formats.iter().any(|format| !unique.insert(format.clone())) {
        return Err(bilibili_upstream_error(
            "Bilibili playback manifest returned duplicate accepted formats",
        ));
    }
    Ok(formats)
}

fn validated_playback_qualities(qualities: Vec<u32>) -> Result<Vec<u32>> {
    if qualities.len() > 64 {
        return Err(bilibili_upstream_error(
            "Bilibili playback manifest exceeded the accepted quality limit",
        ));
    }
    let mut unique = std::collections::BTreeSet::new();
    if qualities
        .iter()
        .any(|quality| *quality == 0 || !unique.insert(*quality))
    {
        return Err(bilibili_upstream_error(
            "Bilibili playback manifest returned invalid accepted qualities",
        ));
    }
    Ok(qualities)
}

fn playback_quality_descriptions(
    qualities: &[u32],
    descriptions: Vec<String>,
) -> Result<BTreeMap<u32, String>> {
    if !descriptions.is_empty() && descriptions.len() != qualities.len() {
        return Err(bilibili_upstream_error(
            "Bilibili playback quality descriptions were inconsistent",
        ));
    }
    qualities
        .iter()
        .copied()
        .zip(descriptions)
        .map(|(quality, description)| {
            validated_bilibili_text(&description, "playback quality description", 256)
                .map(|description| (quality, description))
        })
        .collect()
}

fn map_playback_formats(
    formats: Vec<PlaybackFormatData>,
    descriptions: &BTreeMap<u32, String>,
) -> Result<Vec<BilibiliPlaybackFormat>> {
    if formats.len() > 64 {
        return Err(bilibili_upstream_error(
            "Bilibili playback manifest exceeded the format detail limit",
        ));
    }
    let mut identities = std::collections::BTreeSet::new();
    formats
        .into_iter()
        .map(|format| {
            if format.quality == 0 || !identities.insert(format.quality) {
                return Err(bilibili_upstream_error(
                    "Bilibili playback manifest returned duplicate format details",
                ));
            }
            let name = validated_bilibili_text(&format.format, "playback format detail", 128)?;
            let description =
                optional_bounded_text(&format.new_description, "playback format description", 256)?
                    .or_else(|| descriptions.get(&format.quality).cloned())
                    .ok_or_else(|| {
                        bilibili_upstream_error(
                            "Bilibili playback format detail omitted its description",
                        )
                    })?;
            let display_description =
                optional_bounded_text(&format.display_desc, "playback display description", 256)?
                    .unwrap_or_else(|| description.clone());
            let superscript =
                optional_bounded_text(&format.superscript, "playback superscript", 128)?;
            let codecs = format
                .codecs
                .unwrap_or_default()
                .into_iter()
                .map(|codec| validated_bilibili_text(&codec, "playback format codec", 256))
                .collect::<Result<Vec<_>>>()?;
            if codecs.len() > 16 {
                return Err(bilibili_upstream_error(
                    "Bilibili playback format detail exceeded the codec limit",
                ));
            }
            Ok(BilibiliPlaybackFormat {
                quality: format.quality,
                format: name,
                description,
                display_description,
                superscript,
                codecs,
            })
        })
        .collect()
}

fn map_playback_tracks(
    tracks: Vec<PlaybackTrackData>,
    video: bool,
) -> Result<Vec<BilibiliPlaybackTrack>> {
    if tracks.len() > 512 {
        return Err(bilibili_upstream_error(
            "Bilibili playback manifest exceeded the track limit",
        ));
    }
    let mut identities = std::collections::BTreeSet::new();
    tracks
        .into_iter()
        .map(|track| {
            let track = map_playback_track(track, video)?;
            if !identities.insert((
                track.id,
                track.codecs.clone(),
                track.bandwidth,
                track.url.clone(),
            )) {
                return Err(bilibili_upstream_error(
                    "Bilibili playback manifest returned duplicate tracks",
                ));
            }
            Ok(track)
        })
        .collect()
}

fn map_playback_track(track: PlaybackTrackData, video: bool) -> Result<BilibiliPlaybackTrack> {
    if track.id == 0 || track.bandwidth == 0 {
        return Err(bilibili_upstream_error(
            "Bilibili playback manifest returned an invalid track identity",
        ));
    }
    let url = reconcile_playback_alias(track.base_url_camel, track.base_url, "track URL")?
        .ok_or_else(|| bilibili_upstream_error("Bilibili playback track omitted its URL"))
        .and_then(|url| validate_bilibili_media_url(&url))?;
    let backup_urls = reconcile_playback_alias(
        track.backup_urls_camel,
        track.backup_url,
        "track backup URLs",
    )?
    .unwrap_or_default();
    let backup_urls = validate_bilibili_media_urls(backup_urls, &url)?;
    let mime_type =
        reconcile_playback_alias(track.mime_type_camel, track.mime_type, "track MIME type")?
            .ok_or_else(|| bilibili_upstream_error("Bilibili playback track omitted its MIME type"))
            .and_then(|value| validated_bilibili_text(&value, "playback track MIME type", 128))?;
    let expected_prefix = if video { "video/" } else { "audio/" };
    if !mime_type.starts_with(expected_prefix) {
        return Err(bilibili_upstream_error(
            "Bilibili playback track returned a conflicting MIME type",
        ));
    }
    let codecs = validated_bilibili_text(&track.codecs, "playback track codec", 256)?;
    let width = track.width.filter(|value| *value > 0);
    let height = track.height.filter(|value| *value > 0);
    if video {
        if width.is_none()
            || height.is_none()
            || width.is_some_and(|value| value > 16_384)
            || height.is_some_and(|value| value > 16_384)
        {
            return Err(bilibili_upstream_error(
                "Bilibili playback video track returned invalid dimensions",
            ));
        }
    } else if width.is_some() || height.is_some() {
        return Err(bilibili_upstream_error(
            "Bilibili playback audio track returned video dimensions",
        ));
    }
    let frame_rate =
        reconcile_playback_alias(track.frame_rate_camel, track.frame_rate, "track frame rate")?
            .map(|value| optional_bounded_text(&value, "playback track frame rate", 64))
            .transpose()?
            .flatten();
    let sample_aspect_ratio = track
        .sar
        .map(|value| optional_bounded_text(&value, "playback sample aspect ratio", 64))
        .transpose()?
        .flatten();
    let start_with_sap = reconcile_playback_alias(
        track.start_with_sap_camel,
        track.start_with_sap,
        "track SAP value",
    )?;
    let segment_base_camel = track.segment_base_camel.map(map_segment_base).transpose()?;
    let segment_base_snake = track.segment_base.map(map_segment_base).transpose()?;
    let segment_base =
        reconcile_playback_alias(segment_base_camel, segment_base_snake, "track segment base")?;
    Ok(BilibiliPlaybackTrack {
        id: track.id,
        url,
        backup_urls,
        bandwidth: track.bandwidth,
        mime_type,
        codecs,
        width,
        height,
        frame_rate,
        sample_aspect_ratio,
        start_with_sap,
        segment_base,
        codec_id: track.codecid,
    })
}

fn map_segment_base(segment: SegmentBaseData) -> Result<BilibiliSegmentBase> {
    let initialization = reconcile_playback_alias(
        segment.initialization_camel,
        segment.initialization,
        "segment initialization range",
    )?
    .ok_or_else(|| {
        bilibili_upstream_error("Bilibili playback segment omitted initialization range")
    })?;
    let index_range = reconcile_playback_alias(
        segment.index_range_camel,
        segment.index_range,
        "segment index range",
    )?
    .ok_or_else(|| bilibili_upstream_error("Bilibili playback segment omitted index range"))?;
    validate_byte_range(&initialization, "segment initialization range")?;
    validate_byte_range(&index_range, "segment index range")?;
    Ok(BilibiliSegmentBase {
        initialization,
        index_range,
    })
}

fn validate_byte_range(value: &str, context: &str) -> Result<()> {
    let Some((start, end)) = value.split_once('-') else {
        return Err(bilibili_upstream_error(format!(
            "Bilibili playback returned an invalid {context}"
        )));
    };
    let start = start.parse::<u64>().ok();
    let end = end.parse::<u64>().ok();
    if start.is_none() || end.is_none() || start > end {
        return Err(bilibili_upstream_error(format!(
            "Bilibili playback returned an invalid {context}"
        )));
    }
    Ok(())
}

fn map_progressive_segments(
    segments: Vec<ProgressiveSegmentData>,
) -> Result<Vec<BilibiliProgressiveSegment>> {
    if segments.len() > 256 {
        return Err(bilibili_upstream_error(
            "Bilibili playback manifest exceeded the progressive segment limit",
        ));
    }
    segments
        .into_iter()
        .enumerate()
        .map(|(index, segment)| {
            let expected_order = u32::try_from(index + 1).map_err(|_| {
                bilibili_upstream_error("Bilibili playback segment order overflowed")
            })?;
            if segment.order != expected_order || segment.length == 0 || segment.size == 0 {
                return Err(bilibili_upstream_error(
                    "Bilibili playback manifest returned invalid progressive segments",
                ));
            }
            let url = validate_bilibili_media_url(&segment.url)?;
            let backup_urls = validate_bilibili_media_urls(segment.backup_url, &url)?;
            Ok(BilibiliProgressiveSegment {
                order: segment.order,
                duration_ms: segment.length,
                size: segment.size,
                url,
                backup_urls,
            })
        })
        .collect()
}

fn map_playback_language_catalog(
    catalog: PlaybackLanguageCatalogData,
) -> Result<BilibiliPlaybackLanguageCatalog> {
    if catalog.items.len() > 64 {
        return Err(bilibili_upstream_error(
            "Bilibili playback language catalog exceeded its limit",
        ));
    }
    let mut identities = std::collections::BTreeSet::new();
    let items = catalog
        .items
        .into_iter()
        .map(|item| {
            let language = validate_playback_language_parameter(&item.lang)?;
            if !identities.insert((language.clone(), item.production_type)) {
                return Err(bilibili_upstream_error(
                    "Bilibili playback language catalog returned duplicate entries",
                ));
            }
            Ok(BilibiliPlaybackLanguage {
                language,
                title: validated_bilibili_text(&item.title, "playback language title", 256)?,
                subtitle_language: optional_bounded_text(
                    &item.subtitle_lang,
                    "playback subtitle language",
                    32,
                )?,
                video_detected: item.video_detext,
                mouth_shape_changed: item.video_mouth_shape_change,
                production_type: item.production_type,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(BilibiliPlaybackLanguageCatalog {
        supported: catalog.support,
        items,
        open_message: optional_bounded_text(
            &catalog.open_toast,
            "playback language open message",
            1024,
        )?,
        close_message: optional_bounded_text(
            &catalog.close_toast,
            "playback language close message",
            1024,
        )?,
        default_title: optional_bounded_text(
            &catalog.default_title,
            "playback language default title",
            256,
        )?,
    })
}

fn validate_playback_language_parameter(value: &str) -> Result<String> {
    validate_subtitle_language(value, "playback audio language")
}

fn reconcile_playback_alias<T: PartialEq>(
    camel: Option<T>,
    snake: Option<T>,
    context: &str,
) -> Result<Option<T>> {
    match (camel, snake) {
        (Some(camel), Some(snake)) if camel != snake => Err(bilibili_upstream_error(format!(
            "Bilibili playback manifest returned conflicting {context} aliases"
        ))),
        (Some(value), _) | (_, Some(value)) => Ok(Some(value)),
        (None, None) => Ok(None),
    }
}

fn validate_bilibili_media_urls(urls: Vec<String>, primary: &str) -> Result<Vec<String>> {
    if urls.len() > 16 {
        return Err(bilibili_upstream_error(
            "Bilibili playback track exceeded the backup URL limit",
        ));
    }
    let mut unique = std::collections::BTreeSet::from([primary.to_owned()]);
    urls.into_iter()
        .map(|url| {
            let url = validate_bilibili_media_url(&url)?;
            if !unique.insert(url.clone()) {
                return Err(bilibili_upstream_error(
                    "Bilibili playback track returned duplicate media URLs",
                ));
            }
            Ok(url)
        })
        .collect()
}

fn validate_bilibili_media_url(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 16 * 1024 || value.chars().any(char::is_control) {
        return Err(bilibili_upstream_error(
            "Bilibili playback returned an invalid media URL",
        ));
    }
    let url = Url::parse(value)
        .map_err(|_| bilibili_upstream_error("Bilibili playback returned an invalid media URL"))?;
    let host = url
        .host_str()
        .ok_or_else(|| bilibili_upstream_error("Bilibili playback media URL omitted its host"))?;
    let allowed_host = host == "bilivideo.com"
        || host.ends_with(".bilivideo.com")
        || host == "bilivideo.cn"
        || host.ends_with(".bilivideo.cn")
        || host.ends_with(".akamaized.net");
    if url.scheme() != "https"
        || !allowed_host
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with("/upgcxcode/")
    {
        return Err(bilibili_upstream_error(
            "Bilibili playback returned an unsafe media URL",
        ));
    }
    Ok(url.to_string())
}

fn playback_manifest_expiration(
    video: &[BilibiliPlaybackTrack],
    audio: &[BilibiliPlaybackTrack],
    dolby: &[BilibiliPlaybackTrack],
    lossless: &[BilibiliPlaybackTrack],
    progressive: &[BilibiliProgressiveSegment],
) -> Result<Option<u64>> {
    let track_urls = video
        .iter()
        .chain(audio)
        .chain(dolby)
        .chain(lossless)
        .flat_map(|track| std::iter::once(&track.url).chain(track.backup_urls.iter()));
    let progressive_urls = progressive
        .iter()
        .flat_map(|segment| std::iter::once(&segment.url).chain(segment.backup_urls.iter()));
    track_urls
        .chain(progressive_urls)
        .try_fold(None::<u64>, |expiry, url| {
            let url = Url::parse(url).map_err(|_| {
                bilibili_upstream_error("Bilibili playback media URL became invalid")
            })?;
            let deadline = url
                .query_pairs()
                .find_map(|(name, value)| (name == "deadline").then(|| value.into_owned()))
                .map(|value| {
                    value.parse::<u64>().map_err(|_| {
                        bilibili_upstream_error(
                            "Bilibili playback media URL contained an invalid deadline",
                        )
                    })
                })
                .transpose()?;
            Ok(match (expiry, deadline) {
                (Some(current), Some(deadline)) => Some(current.min(deadline)),
                (None, Some(deadline)) => Some(deadline),
                (expiry, None) => expiry,
            })
        })
}

fn parse_created_favorite_folders_response(
    bytes: &[u8],
    requested_owner_id: u64,
) -> Result<BilibiliCreatedFavoriteFolders> {
    let response: PassportResponse<CreatedFavoriteFoldersData> = serde_json::from_slice(bytes)
        .map_err(|_| {
            bilibili_upstream_error("Bilibili created favorite folders returned invalid JSON")
        })?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili created favorite folders",
            response.code,
            &response.message,
        ));
    }
    let data = response.data.ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::PermissionDenied,
            "Bilibili user favorite folders are not publicly visible",
        )
        .with_platform(Platform::Bilibili)
        .with_details(json!({ "platform_code": 0, "hidden": true }))
    })?;
    if data.count > 10_000 {
        return Err(bilibili_upstream_error(
            "Bilibili created favorite folder count exceeded the supported limit",
        ));
    }
    let items = data.list.unwrap_or_default();
    if items.len() as u64 != data.count {
        return Err(bilibili_upstream_error(
            "Bilibili created favorite folder count was inconsistent",
        ));
    }
    let mut media_ids = std::collections::BTreeSet::new();
    let mut folders = Vec::with_capacity(items.len());
    for item in items {
        if item.id == 0
            || item.fid == 0
            || item.mid != requested_owner_id
            || item.attr > u64::from(u32::MAX)
            || !media_ids.insert(item.id)
        {
            return Err(bilibili_upstream_error(
                "Bilibili created favorite folders returned an invalid identity",
            ));
        }
        let favorite_state = match item.fav_state {
            0 => false,
            1 => true,
            _ => {
                return Err(bilibili_upstream_error(
                    "Bilibili created favorite folders returned an invalid favorite state",
                ));
            }
        };
        let title = validated_bilibili_text(&item.title, "favorite folder title", 1024)?;
        let child_friendly_description = if item.kid_playlist_desc.trim().is_empty() {
            String::new()
        } else {
            validated_bilibili_text(
                &item.kid_playlist_desc,
                "child-friendly favorite folder description",
                4096,
            )?
        };
        folders.push(BilibiliCreatedFavoriteFolder {
            media_id: item.id,
            folder_id: item.fid,
            owner_id: item.mid,
            attributes: item.attr,
            title,
            favorite_state,
            media_count: item.media_count,
            child_friendly: item.is_kid_playlist,
            child_friendly_description,
        });
    }
    Ok(BilibiliCreatedFavoriteFolders {
        owner_id: requested_owner_id,
        folders,
    })
}

fn parse_favorite_folder_response(
    bytes: &[u8],
    requested_media_id: u64,
) -> Result<BilibiliFavoriteFolder> {
    let response: PassportResponse<FavoriteFolderData> = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili favorite folder returned invalid JSON"))?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili favorite folder",
            response.code,
            &response.message,
        ));
    }
    let data = response.data.ok_or_else(|| {
        bilibili_upstream_error("Bilibili favorite folder response did not contain data")
    })?;
    map_favorite_folder_data(data, requested_media_id)
}

fn map_favorite_folder_data(
    data: FavoriteFolderData,
    requested_media_id: u64,
) -> Result<BilibiliFavoriteFolder> {
    let expected_media_id = data
        .fid
        .checked_mul(100)
        .and_then(|prefix| prefix.checked_add(data.mid % 100));
    if data.id == 0
        || data.id != requested_media_id
        || data.fid == 0
        || data.mid == 0
        || data.upper.mid != data.mid
        || expected_media_id != Some(data.id)
        || data.attr > u64::from(u32::MAX)
        || data.cover_type > u64::from(u32::MAX)
        || data.kind != 11
        || data.upper.vip_type > 2
    {
        return Err(bilibili_upstream_error(
            "Bilibili favorite folder returned an invalid identity or attribute",
        ));
    }
    let invalid = validated_binary_state(data.state, "favorite folder state")?;
    let favorite_state = validated_binary_state(data.fav_state, "favorite folder favorite state")?;
    let like_state = validated_binary_state(data.like_state, "favorite folder like state")?;
    let vip_status =
        validated_binary_state(data.upper.vip_statue, "favorite folder owner VIP state")?;
    let title = validated_bilibili_text(&data.title, "favorite folder title", 1024)?;
    let description =
        validated_bilibili_multiline_text(&data.intro, "favorite folder description", 64 * 1024)?;
    let child_friendly_description = validated_bilibili_multiline_text(
        &data.kid_playlist_desc,
        "child-friendly favorite folder description",
        4096,
    )?;
    let owner = BilibiliFavoriteFolderOwner {
        id: data.upper.mid,
        name: validated_bilibili_text(&data.upper.name, "favorite folder owner name", 512)?,
        avatar_url: normalize_bilibili_image_url(&data.upper.face, "favorite folder owner avatar")?,
        followed: data.upper.followed,
        vip_type: data.upper.vip_type,
        vip_status,
    };
    Ok(BilibiliFavoriteFolder {
        media_id: data.id,
        folder_id: data.fid,
        owner,
        attributes: data.attr,
        title,
        cover_url: normalize_bilibili_image_url(&data.cover, "favorite folder cover")?,
        cover_type: data.cover_type,
        description,
        created_at: data.ctime,
        updated_at: data.mtime,
        invalid,
        favorite_state,
        like_state,
        media_count: data.media_count,
        pinned: data.is_top,
        child_friendly: data.is_kid_playlist,
        child_friendly_description,
        counts: BilibiliFavoriteFolderCounts {
            collect: data.cnt_info.collect,
            play: data.cnt_info.play,
            thumb_up: data.cnt_info.thumb_up,
            share: data.cnt_info.share,
        },
    })
}

fn parse_favorite_media_response(
    bytes: &[u8],
    requested_media_id: u64,
    requested_page: u32,
    requested_page_size: u32,
) -> Result<BilibiliFavoriteMediaPage> {
    let response: PassportResponse<FavoriteMediaData> = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili favorite media returned invalid JSON"))?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili favorite media",
            response.code,
            &response.message,
        ));
    }
    let data = response.data.ok_or_else(|| {
        bilibili_upstream_error("Bilibili favorite media response did not contain data")
    })?;
    let folder = map_favorite_folder_data(data.info, requested_media_id)?;
    if requested_page == 0 || requested_page_size != FAVORITE_MEDIA_PAGE_SIZE {
        return Err(bilibili_upstream_error(
            "Bilibili favorite media returned invalid pagination",
        ));
    }
    let items = data.medias.unwrap_or_default();
    if items.len() > requested_page_size as usize {
        return Err(bilibili_upstream_error(
            "Bilibili favorite media page exceeded the requested size",
        ));
    }
    let page_start = u64::from(requested_page - 1) * u64::from(requested_page_size);
    let page_end = page_start
        .checked_add(items.len() as u64)
        .ok_or_else(|| bilibili_upstream_error("Bilibili favorite media page overflowed"))?;
    if page_end > folder.media_count
        || (page_start < folder.media_count && items.is_empty())
        || data.has_more != (page_end < folder.media_count)
    {
        return Err(bilibili_upstream_error(
            "Bilibili favorite media continuation was inconsistent",
        ));
    }
    let mut identities = std::collections::BTreeSet::new();
    let mut medias = Vec::with_capacity(items.len());
    for item in items {
        let media = map_favorite_media(item)?;
        if !identities.insert(media.aid) {
            return Err(bilibili_upstream_error(
                "Bilibili favorite media page contained duplicate identities",
            ));
        }
        medias.push(media);
    }
    Ok(BilibiliFavoriteMediaPage {
        page: requested_page,
        page_size: requested_page_size,
        total: folder.media_count,
        has_more: data.has_more,
        folder,
        medias,
    })
}

fn map_favorite_media(item: FavoriteMediaItem) -> Result<BilibiliFavoriteMedia> {
    if item.id == 0 || item.kind != 2 || item.page > 100_000 {
        return Err(bilibili_upstream_error(
            "Bilibili favorite media returned an unsupported type or identity",
        ));
    }
    let invalid = match item.attr {
        0 => false,
        1 | 9 => true,
        _ => {
            return Err(bilibili_upstream_error(
                "Bilibili favorite media returned an invalid state",
            ));
        }
    };
    let raw_bvid = [item.bvid.trim(), item.bv_id.trim()]
        .into_iter()
        .filter(|value| !value.is_empty())
        .collect::<std::collections::BTreeSet<_>>();
    if raw_bvid.len() > 1 {
        return Err(bilibili_upstream_error(
            "Bilibili favorite media returned conflicting BVIDs",
        ));
    }
    let bvid = raw_bvid
        .into_iter()
        .next()
        .map(|value| {
            match crate::BilibiliVideoIdentity::parse(value).map_err(|_| {
                bilibili_upstream_error("Bilibili favorite media returned an invalid BVID")
            })? {
                crate::BilibiliVideoIdentity::Bvid(value) => Ok(value),
                _ => unreachable!("BVID parser returned another identity type"),
            }
        })
        .transpose()?;
    if !invalid && (bvid.is_none() || item.page == 0) {
        return Err(bilibili_upstream_error(
            "Bilibili favorite media omitted a playable video identity",
        ));
    }
    let owner = match item.upper {
        Some(owner) if owner.mid > 0 && !owner.name.trim().is_empty() => {
            Some(BilibiliCollectedPlaylistOwner {
                id: owner.mid,
                name: validated_bilibili_text(&owner.name, "favorite media owner name", 512)?,
                avatar_url: normalize_bilibili_image_url(
                    &owner.face,
                    "favorite media owner avatar",
                )?,
            })
        }
        Some(_) | None if invalid => None,
        _ => {
            return Err(bilibili_upstream_error(
                "Bilibili favorite media omitted its owner",
            ));
        }
    };
    Ok(BilibiliFavoriteMedia {
        aid: item.id,
        bvid,
        title: validated_bilibili_text(&item.title, "favorite media title", 4096)?,
        cover_url: normalize_bilibili_image_url(&item.cover, "favorite media cover")?,
        description: validated_bilibili_multiline_text(
            &item.intro,
            "favorite media description",
            64 * 1024,
        )?,
        part_count: item.page,
        duration_seconds: item.duration,
        owner,
        invalid,
        collect_count: item.cnt_info.collect,
        play_count: item.cnt_info.play,
        danmaku_count: item.cnt_info.danmaku,
        created_at: item.ctime,
        published_at: item.pubtime,
        favorited_at: item.fav_time,
    })
}

fn validated_binary_state(value: u64, context: &str) -> Result<bool> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(bilibili_upstream_error(format!(
            "Bilibili returned an invalid {context}"
        ))),
    }
}

fn parse_collected_playlists_response(
    bytes: &[u8],
    requested_page: u32,
    requested_page_size: u32,
) -> Result<BilibiliCollectedPlaylistPage> {
    let response: PassportResponse<CollectedPlaylistsData> = serde_json::from_slice(bytes)
        .map_err(|_| {
            bilibili_upstream_error("Bilibili collected playlists returned invalid JSON")
        })?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili collected playlists",
            response.code,
            &response.message,
        ));
    }
    let data = response.data.ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::PermissionDenied,
            "Bilibili user collected playlists are not publicly visible",
        )
        .with_platform(Platform::Bilibili)
        .with_details(json!({ "platform_code": 0, "hidden": true }))
    })?;
    if requested_page == 0 || !(1..=70).contains(&requested_page_size) || data.count > 1_000_000 {
        return Err(bilibili_upstream_error(
            "Bilibili collected playlists returned invalid pagination",
        ));
    }
    let items = data.list.unwrap_or_default();
    if items.len() > requested_page_size as usize {
        return Err(bilibili_upstream_error(
            "Bilibili collected playlist page exceeded the requested size",
        ));
    }
    let page_start = u64::from(requested_page - 1)
        .checked_mul(u64::from(requested_page_size))
        .ok_or_else(|| {
            bilibili_upstream_error("Bilibili collected playlist page offset overflowed")
        })?;
    let page_end = page_start.checked_add(items.len() as u64).ok_or_else(|| {
        bilibili_upstream_error("Bilibili collected playlist page size overflowed")
    })?;
    if (page_start < data.count && (page_end > data.count || items.is_empty()))
        || (page_start >= data.count && !items.is_empty())
    {
        return Err(bilibili_upstream_error(
            "Bilibili collected playlist page was inconsistent with its total",
        ));
    }
    let has_more = page_end < data.count;
    if data.has_more.is_some_and(|reported| reported != has_more)
        || (has_more && items.len() != requested_page_size as usize)
    {
        return Err(bilibili_upstream_error(
            "Bilibili collected playlist continuation was inconsistent",
        ));
    }
    let mut identities = std::collections::BTreeSet::new();
    let mut playlists = Vec::with_capacity(items.len());
    for item in items {
        let playlist = map_collected_playlist_item(item)?;
        if !identities.insert((playlist.kind, playlist.id)) {
            return Err(bilibili_upstream_error(
                "Bilibili collected playlist page contained duplicate identities",
            ));
        }
        playlists.push(playlist);
    }
    Ok(BilibiliCollectedPlaylistPage {
        page: requested_page,
        page_size: requested_page_size,
        total: data.count,
        has_more,
        playlists,
    })
}

fn map_collected_playlist_item(item: CollectedPlaylistItem) -> Result<BilibiliCollectedPlaylist> {
    if item.id == 0 || item.attr > u64::from(u32::MAX) || item.cover_type > u64::from(u32::MAX) {
        return Err(bilibili_upstream_error(
            "Bilibili collected playlist returned an invalid identity or attribute",
        ));
    }
    let invalid = match item.state {
        0 => false,
        1 => true,
        _ => {
            return Err(bilibili_upstream_error(
                "Bilibili collected playlist returned an invalid state",
            ));
        }
    };
    let favorite_state = match item.fav_state {
        0 => false,
        1 => true,
        _ => {
            return Err(bilibili_upstream_error(
                "Bilibili collected playlist returned an invalid favorite state",
            ));
        }
    };
    let kind = match (item.kind, item.fid) {
        (Some(11), folder_id) if folder_id > 0 => BilibiliCollectedPlaylistKind::FavoriteFolder,
        (Some(21), 0) => BilibiliCollectedPlaylistKind::Season,
        (None, folder_id) if folder_id > 0 => BilibiliCollectedPlaylistKind::FavoriteFolder,
        _ => {
            return Err(bilibili_upstream_error(
                "Bilibili collected playlist returned an unsupported collection type",
            ));
        }
    };
    let folder_id = (kind == BilibiliCollectedPlaylistKind::FavoriteFolder).then_some(item.fid);
    let title = validated_bilibili_text(&item.title, "collected playlist title", 1024)?;
    let cover_url = normalize_bilibili_image_url(&item.cover, "collected playlist cover")?;
    let description = validated_bilibili_multiline_text(
        &item.intro,
        "collected playlist description",
        64 * 1024,
    )?;
    let attribute_description = validated_bilibili_multiline_text(
        &item.attr_desc,
        "collected playlist attribute description",
        4096,
    )?;
    let child_friendly_description = validated_bilibili_multiline_text(
        &item.kid_playlist_desc,
        "child-friendly collected playlist description",
        4096,
    )?;
    let deep_link = optional_bounded_text(&item.link, "collected playlist link", 4096)?;
    let bvid = if item.bvid.trim().is_empty() {
        None
    } else {
        Some(
            match crate::BilibiliVideoIdentity::parse(&item.bvid).map_err(|_| {
                bilibili_upstream_error("Bilibili collected playlist returned an invalid BVID")
            })? {
                crate::BilibiliVideoIdentity::Bvid(value) => value,
                _ => unreachable!("BVID parser returned another identity type"),
            },
        )
    };
    let owner = match item.upper {
        Some(owner) if owner.mid > 0 || !owner.name.trim().is_empty() => {
            if owner.mid == 0 || owner.mid != item.mid {
                return Err(bilibili_upstream_error(
                    "Bilibili collected playlist returned an inconsistent owner",
                ));
            }
            Some(BilibiliCollectedPlaylistOwner {
                id: owner.mid,
                name: validated_bilibili_text(&owner.name, "collected playlist owner name", 512)?,
                avatar_url: normalize_bilibili_image_url(
                    &owner.face,
                    "collected playlist owner avatar",
                )?,
            })
        }
        Some(_) | None if invalid && item.mid == 0 => None,
        _ => {
            return Err(bilibili_upstream_error(
                "Bilibili collected playlist did not return a valid owner",
            ));
        }
    };
    Ok(BilibiliCollectedPlaylist {
        kind,
        id: item.id,
        folder_id,
        owner,
        attributes: item.attr,
        attribute_description,
        title,
        cover_url,
        description,
        cover_type: item.cover_type,
        created_at: item.ctime,
        updated_at: item.mtime,
        invalid,
        favorite_state,
        media_count: item.media_count,
        view_count: item.view_count,
        pinned: item.is_top,
        deep_link,
        bvid,
        child_friendly: item.is_kid_playlist,
        child_friendly_description,
    })
}

fn parse_space_playlists_response(
    bytes: &[u8],
    requested_user_id: u64,
    requested_page: u32,
    requested_page_size: u32,
) -> Result<BilibiliSpacePlaylistPage> {
    let response: PassportResponse<SpacePlaylistsData> = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili space playlists returned invalid JSON"))?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili space playlists",
            response.code,
            &response.message,
        ));
    }
    let data = response.data.ok_or_else(|| {
        bilibili_upstream_error("Bilibili space playlists response did not contain data")
    })?;
    let lists = data.items_lists;
    if requested_page == 0
        || requested_page_size != SPACE_PLAYLIST_PAGE_SIZE
        || lists.page.page_num != requested_page
        || lists.page.page_size != requested_page_size
        || lists.page.total > 100_000
    {
        return Err(bilibili_upstream_error(
            "Bilibili space playlists returned invalid pagination",
        ));
    }
    let returned = lists
        .seasons_list
        .len()
        .checked_add(lists.series_list.len())
        .ok_or_else(|| bilibili_upstream_error("Bilibili space playlist page size overflowed"))?;
    if returned > requested_page_size as usize {
        return Err(bilibili_upstream_error(
            "Bilibili space playlist page exceeded the requested size",
        ));
    }
    let page_start = u64::from(requested_page - 1)
        .checked_mul(u64::from(requested_page_size))
        .ok_or_else(|| bilibili_upstream_error("Bilibili space playlist page offset overflowed"))?;
    let page_end = page_start
        .checked_add(returned as u64)
        .ok_or_else(|| bilibili_upstream_error("Bilibili space playlist page result overflowed"))?;
    if (page_start < lists.page.total && (page_end > lists.page.total || returned == 0))
        || (page_start >= lists.page.total && returned > 0)
    {
        return Err(bilibili_upstream_error(
            "Bilibili space playlist page was inconsistent with its total",
        ));
    }
    let has_more = page_end < lists.page.total;
    if has_more && returned != requested_page_size as usize {
        return Err(bilibili_upstream_error(
            "Bilibili space playlist continuation was inconsistent",
        ));
    }
    let mut identities = std::collections::BTreeSet::new();
    let mut playlists = Vec::with_capacity(returned);
    for item in lists.seasons_list {
        let playlist = map_space_season(item, requested_user_id)?;
        if !identities.insert((playlist.kind, playlist.id)) {
            return Err(bilibili_upstream_error(
                "Bilibili space playlist page contained a duplicate identity",
            ));
        }
        playlists.push(playlist);
    }
    for item in lists.series_list {
        let playlist = map_space_series(item, requested_user_id)?;
        if !identities.insert((playlist.kind, playlist.id)) {
            return Err(bilibili_upstream_error(
                "Bilibili space playlist page contained a duplicate identity",
            ));
        }
        playlists.push(playlist);
    }
    Ok(BilibiliSpacePlaylistPage {
        page: lists.page.page_num,
        page_size: lists.page.page_size,
        total: lists.page.total,
        has_more,
        playlists,
    })
}

fn map_space_season(
    item: SpaceSeasonItem,
    requested_user_id: u64,
) -> Result<BilibiliSpacePlaylist> {
    let meta = item.meta;
    if meta.season_id == 0 || meta.mid != requested_user_id || meta.category > u64::from(u32::MAX) {
        return Err(bilibili_upstream_error(
            "Bilibili space season returned an invalid identity",
        ));
    }
    Ok(BilibiliSpacePlaylist {
        kind: BilibiliSpacePlaylistKind::Season,
        id: meta.season_id,
        owner_id: meta.mid,
        name: validated_bilibili_text(&meta.name, "space season name", 1024)?,
        display_title: optional_bounded_text(&meta.title, "space season title", 1024)?,
        description: validated_bilibili_multiline_text(
            &meta.description,
            "space season description",
            64 * 1024,
        )?,
        cover_url: normalize_bilibili_image_url(&meta.cover, "space season cover")?,
        category: meta.category,
        track_count: meta.total,
        created_at: 0,
        published_at: meta.ptime,
        updated_at: 0,
        state: None,
        creator_mode: None,
        keywords: Vec::new(),
        recent_aids: validated_aid_list(item.recent_aids, "space season recent AIDs")?,
        preview_aids: validated_space_archives(item.archives, "space season previews")?,
    })
}

fn map_space_series(
    item: SpaceSeriesItem,
    requested_user_id: u64,
) -> Result<BilibiliSpacePlaylist> {
    let meta = item.meta;
    if meta.series_id == 0
        || meta.mid != requested_user_id
        || meta.category > u64::from(u32::MAX)
        || meta.state > u64::from(u32::MAX)
    {
        return Err(bilibili_upstream_error(
            "Bilibili space series returned an invalid identity",
        ));
    }
    let mut keywords = meta
        .keywords
        .into_iter()
        .chain(meta.raw_keywords.split(',').map(str::to_owned))
        .map(|keyword| keyword.trim().to_owned())
        .filter(|keyword| !keyword.is_empty())
        .map(|keyword| validated_bilibili_text(&keyword, "space series keyword", 256))
        .collect::<Result<Vec<_>>>()?;
    let mut seen_keywords = std::collections::BTreeSet::new();
    keywords.retain(|keyword| seen_keywords.insert(keyword.clone()));
    Ok(BilibiliSpacePlaylist {
        kind: BilibiliSpacePlaylistKind::Series,
        id: meta.series_id,
        owner_id: meta.mid,
        name: validated_bilibili_text(&meta.name, "space series name", 1024)?,
        display_title: None,
        description: validated_bilibili_multiline_text(
            &meta.description,
            "space series description",
            64 * 1024,
        )?,
        cover_url: normalize_bilibili_image_url(&meta.cover, "space series cover")?,
        category: meta.category,
        track_count: meta.total,
        created_at: meta.ctime,
        published_at: 0,
        updated_at: meta.last_update_ts.max(meta.mtime),
        state: Some(meta.state),
        creator_mode: optional_bounded_text(&meta.creator, "space series creator mode", 256)?,
        keywords,
        recent_aids: validated_aid_list(item.recent_aids, "space series recent AIDs")?,
        preview_aids: validated_space_archives(item.archives, "space series previews")?,
    })
}

fn validated_space_archives(
    archives: Vec<SpacePlaylistArchive>,
    context: &str,
) -> Result<Vec<u64>> {
    if archives.len() > 100 {
        return Err(bilibili_upstream_error(format!(
            "Bilibili returned too many {context}"
        )));
    }
    let mut aids = Vec::with_capacity(archives.len());
    for archive in archives {
        if archive.aid == 0 {
            return Err(bilibili_upstream_error(format!(
                "Bilibili returned an invalid {context}"
            )));
        }
        match crate::BilibiliVideoIdentity::parse(&archive.bvid).map_err(|_| {
            bilibili_upstream_error(format!("Bilibili returned an invalid BVID in {context}"))
        })? {
            crate::BilibiliVideoIdentity::Bvid(_) => {}
            _ => unreachable!("BVID parser returned another identity type"),
        }
        aids.push(archive.aid);
    }
    validated_aid_list(aids, context)
}

fn validated_aid_list(values: Vec<u64>, context: &str) -> Result<Vec<u64>> {
    if values.len() > 100
        || values.contains(&0)
        || values
            .iter()
            .copied()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            != values.len()
    {
        return Err(bilibili_upstream_error(format!(
            "Bilibili returned an invalid {context}"
        )));
    }
    Ok(values)
}

fn parse_season_archives_response(
    bytes: &[u8],
    requested_season_id: u64,
    requested_page: u32,
    requested_page_size: u32,
) -> Result<BilibiliSeasonArchivePage> {
    let response: PassportResponse<SeasonArchivesData> = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili season archives returned invalid JSON"))?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili season archives",
            response.code,
            &response.message,
        ));
    }
    let data = response.data.ok_or_else(|| {
        bilibili_upstream_error("Bilibili season archives response did not contain data")
    })?;
    if requested_page == 0
        || requested_page_size != SEASON_ARCHIVE_PAGE_SIZE
        || data.page.page_num != requested_page
        || data.page.page_size != requested_page_size
        || data.page.total != data.meta.total
        || data.page.total > 1_000_000
        || data.meta.season_id != requested_season_id
        || data.meta.mid == 0
        || data.meta.category > u64::from(u32::MAX)
    {
        return Err(bilibili_upstream_error(
            "Bilibili season archives returned invalid pagination or identity",
        ));
    }
    if data.archives.len() > requested_page_size as usize || data.aids.len() != data.archives.len()
    {
        return Err(bilibili_upstream_error(
            "Bilibili season archive page exceeded the requested size",
        ));
    }
    let page_start = u64::from(requested_page - 1)
        .checked_mul(u64::from(requested_page_size))
        .ok_or_else(|| bilibili_upstream_error("Bilibili season archive page offset overflowed"))?;
    let page_end = page_start
        .checked_add(data.archives.len() as u64)
        .ok_or_else(|| bilibili_upstream_error("Bilibili season archive page result overflowed"))?;
    if (page_start < data.page.total && (page_end > data.page.total || data.archives.is_empty()))
        || (page_start >= data.page.total && !data.archives.is_empty())
    {
        return Err(bilibili_upstream_error(
            "Bilibili season archive page was inconsistent with its total",
        ));
    }
    let has_more = page_end < data.page.total;
    if has_more && data.archives.len() != requested_page_size as usize {
        return Err(bilibili_upstream_error(
            "Bilibili season archive continuation was inconsistent",
        ));
    }
    let mut identities = std::collections::BTreeSet::new();
    let mut archives = Vec::with_capacity(data.archives.len());
    for (reported_aid, archive) in data.aids.into_iter().zip(data.archives) {
        if reported_aid != archive.aid || !identities.insert(archive.aid) {
            return Err(bilibili_upstream_error(
                "Bilibili season archive identities were inconsistent",
            ));
        }
        archives.push(map_season_archive(archive)?);
    }
    let preview_aids = archives.iter().map(|archive| archive.aid).collect();
    let meta = data.meta;
    let season = BilibiliSpacePlaylist {
        kind: BilibiliSpacePlaylistKind::Season,
        id: meta.season_id,
        owner_id: meta.mid,
        name: validated_bilibili_text(&meta.name, "season name", 1024)?,
        display_title: optional_bounded_text(&meta.title, "season title", 1024)?,
        description: validated_bilibili_multiline_text(
            &meta.description,
            "season description",
            64 * 1024,
        )?,
        cover_url: normalize_bilibili_image_url(&meta.cover, "season cover")?,
        category: meta.category,
        track_count: meta.total,
        created_at: 0,
        published_at: meta.ptime,
        updated_at: 0,
        state: None,
        creator_mode: None,
        keywords: Vec::new(),
        recent_aids: Vec::new(),
        preview_aids,
    };
    Ok(BilibiliSeasonArchivePage {
        page: data.page.page_num,
        page_size: data.page.page_size,
        total: data.page.total,
        has_more,
        season,
        archives,
    })
}

fn map_season_archive(item: SeasonArchiveItem) -> Result<BilibiliSeasonArchive> {
    if item.aid == 0
        || !(-1..=100).contains(&item.playback_position.unwrap_or_default())
        || !matches!(item.ugc_pay, 0 | 1)
    {
        return Err(bilibili_upstream_error(
            "Bilibili season archive returned invalid state",
        ));
    }
    let bvid = match crate::BilibiliVideoIdentity::parse(&item.bvid)
        .map_err(|_| bilibili_upstream_error("Bilibili season archive returned an invalid BVID"))?
    {
        crate::BilibiliVideoIdentity::Bvid(value) => value,
        _ => unreachable!("BVID parser returned another identity type"),
    };
    Ok(BilibiliSeasonArchive {
        aid: item.aid,
        bvid,
        title: validated_bilibili_text(&item.title, "season archive title", 4096)?,
        cover_url: normalize_bilibili_image_url(&item.pic, "season archive cover")?.ok_or_else(
            || bilibili_upstream_error("Bilibili season archive did not return a cover"),
        )?,
        duration_seconds: item.duration,
        created_at: item.ctime,
        published_at: item.pubdate,
        interactive: item.interactive_video,
        playback_position: item.playback_position,
        state: item.state,
        paid: item.ugc_pay == 1,
        view_count: item.stat.view,
        danmaku_count: item.stat.danmaku,
    })
}

fn is_video_search_risk_challenge(error: &TuneWeaveError) -> bool {
    error.code == ErrorCode::RateLimited
        && error.details.get("risk_challenge").and_then(Value::as_bool) == Some(true)
}

fn map_video_search_item(item: VideoSearchItem) -> Result<BilibiliSearchVideo> {
    if !item.r#type.is_empty() && item.r#type != "video" {
        return Err(bilibili_upstream_error(
            "Bilibili video search returned a non-video item",
        ));
    }
    let aid =
        item.aid.get().filter(|value| *value > 0).ok_or_else(|| {
            bilibili_upstream_error("Bilibili video search returned an invalid AID")
        })?;
    let bvid = (!item.bvid.trim().is_empty())
        .then(|| crate::BilibiliVideoIdentity::parse(&item.bvid))
        .transpose()
        .map_err(|_| bilibili_upstream_error("Bilibili video search returned an invalid BVID"))?
        .map(|identity| match identity {
            crate::BilibiliVideoIdentity::Bvid(value) => value,
            _ => unreachable!("BVID parser returned another identity type"),
        });
    let author_id = item.mid.get().filter(|value| *value > 0).ok_or_else(|| {
        bilibili_upstream_error("Bilibili video search returned an invalid creator ID")
    })?;
    let title = clean_search_text(&item.title, "title", 1024)?;
    let author = clean_search_text(&item.author, "creator name", 512)?;
    let description = clean_search_text(&item.description, "description", 16 * 1024)?;
    let cover_url = normalize_search_image_url(&item.pic)?;
    let duration_seconds = parse_duration_seconds(&item.duration)?;
    let tags = item
        .tag
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .take(100)
        .map(|value| validated_search_text(value, "tag", 256))
        .collect::<Result<Vec<_>>>()?;
    let hit_columns = item
        .hit_columns
        .unwrap_or_default()
        .into_iter()
        .map(|value| validated_search_text(&value, "hit column", 64))
        .collect::<Result<Vec<_>>>()?;
    Ok(BilibiliSearchVideo {
        aid,
        bvid,
        title,
        author,
        author_id,
        description,
        cover_url,
        duration_seconds,
        duration_text: item.duration,
        play_count: optional_flexible_u64(item.play, "play count")?,
        danmaku_count: optional_flexible_u64(item.video_review, "danmaku count")?,
        favorite_count: optional_flexible_u64(item.favorites, "favorite count")?,
        comment_count: optional_flexible_u64(item.review, "comment count")?,
        published_at: optional_flexible_u64(item.pubdate, "publish timestamp")?,
        sent_at: optional_flexible_u64(item.senddate, "send timestamp")?,
        category_id: item
            .typeid
            .map(FlexibleText::into_string)
            .map(|value| validated_search_text(&value, "category ID", 64))
            .transpose()?,
        category_name: item
            .typename
            .map(|value| clean_search_text(&value, "category name", 256))
            .transpose()?,
        tags,
        hit_columns,
        paid: optional_binary_flag(item.is_pay, "paid flag")?,
        collaborative: optional_binary_flag(item.is_union_video, "collaboration flag")?,
        rank_score: optional_flexible_u64(item.rank_score, "rank score")?,
    })
}

fn optional_flexible_u64(value: Option<FlexibleU64>, context: &str) -> Result<Option<u64>> {
    value
        .map(|value| {
            value.get().ok_or_else(|| {
                bilibili_upstream_error(format!(
                    "Bilibili video search returned an invalid {context}"
                ))
            })
        })
        .transpose()
}

fn optional_binary_flag(value: Option<FlexibleU64>, context: &str) -> Result<Option<bool>> {
    optional_flexible_u64(value, context)?
        .map(|value| match value {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(bilibili_upstream_error(format!(
                "Bilibili video search returned an invalid {context}"
            ))),
        })
        .transpose()
}

fn validate_search_keyword(value: &str) -> Result<()> {
    let value = value.trim();
    if value.is_empty() || value.len() > 512 || value.chars().any(char::is_control) {
        return Err(invalid_bilibili_request(
            "Bilibili video search keyword is invalid",
        ));
    }
    Ok(())
}

fn clean_search_text(value: &str, context: &str, limit: usize) -> Result<String> {
    let value = value
        .replace("<em class=\"keyword\">", "")
        .replace("<em class='keyword'>", "")
        .replace("</em>", "")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'");
    validated_search_text(&value, context, limit)
}

fn validated_search_text(value: &str, context: &str, limit: usize) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > limit || value.chars().any(char::is_control) {
        return Err(bilibili_upstream_error(format!(
            "Bilibili video search returned an invalid {context}"
        )));
    }
    Ok(value.to_owned())
}

fn validated_bilibili_text(value: &str, context: &str, limit: usize) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > limit || value.chars().any(char::is_control) {
        return Err(bilibili_upstream_error(format!(
            "Bilibili returned an invalid {context}"
        )));
    }
    Ok(value.to_owned())
}

fn validated_bilibili_multiline_text(value: &str, context: &str, limit: usize) -> Result<String> {
    let value = value.trim();
    if value.len() > limit
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\r' | '\n' | '\t'))
    {
        return Err(bilibili_upstream_error(format!(
            "Bilibili returned an invalid {context}"
        )));
    }
    Ok(value.to_owned())
}

fn optional_bounded_text(value: &str, context: &str, limit: usize) -> Result<Option<String>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > limit || value.chars().any(char::is_control) {
        return Err(bilibili_upstream_error(format!(
            "Bilibili returned an invalid {context}"
        )));
    }
    Ok(Some(value.to_owned()))
}

fn validate_subtitle_id(value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 32
        || value.starts_with('0')
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(bilibili_upstream_error(
            "Bilibili subtitle catalog returned an invalid string identity",
        ));
    }
    Ok(value.to_owned())
}

fn validate_subtitle_language(value: &str, context: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 32
        || value.starts_with('-')
        || value.ends_with('-')
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_alphanumeric() && byte != b'-')
    {
        return Err(bilibili_upstream_error(format!(
            "Bilibili returned an invalid {context}"
        )));
    }
    Ok(value.to_owned())
}

fn optional_subtitle_language(value: &str, context: &str) -> Result<Option<String>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    validate_subtitle_language(value, context).map(Some)
}

fn optional_flexible_subtitle_text(
    value: Option<FlexibleText>,
    context: &str,
    limit: usize,
) -> Result<Option<String>> {
    value
        .map(FlexibleText::into_string)
        .map_or(Ok(None), |value| {
            optional_bounded_text(&value, context, limit)
        })
}

fn optional_subtitle_style_text(value: Option<String>, context: &str) -> Result<Option<String>> {
    value.map_or(Ok(None), |value| {
        optional_bounded_text(&value, context, 128)
    })
}

fn validate_optional_subtitle_ratio(
    value: Option<f64>,
    context: &str,
    maximum: f64,
) -> Result<Option<f64>> {
    if value.is_some_and(|value| !value.is_finite() || value < 0.0 || value > maximum) {
        return Err(bilibili_upstream_error(format!(
            "Bilibili subtitle body returned an invalid {context}"
        )));
    }
    Ok(value)
}

fn validate_subtitle_seconds(value: f64, context: &str) -> Result<()> {
    const MAX_SUBTITLE_SECONDS: f64 = 30.0 * 24.0 * 60.0 * 60.0;
    if !value.is_finite() || !(0.0..=MAX_SUBTITLE_SECONDS).contains(&value) {
        return Err(bilibili_upstream_error(format!(
            "Bilibili subtitle body returned an invalid {context}"
        )));
    }
    Ok(())
}

fn validated_subtitle_cue_text(value: &str) -> Result<String> {
    if value.trim().is_empty()
        || value.len() > 64 * 1024
        || value
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\r' | '\n' | '\t'))
    {
        return Err(bilibili_upstream_error(
            "Bilibili subtitle body returned invalid cue text",
        ));
    }
    Ok(value.to_owned())
}

fn normalize_bilibili_subtitle_url(value: &str) -> Result<Url> {
    let value = value.trim();
    if value.is_empty() || value.len() > 8 * 1024 || value.chars().any(char::is_control) {
        return Err(bilibili_upstream_error(
            "Bilibili subtitle catalog returned an invalid resource URL",
        ));
    }
    let value = value
        .strip_prefix("//")
        .map_or_else(|| value.to_owned(), |value| format!("https://{value}"));
    let url = Url::parse(&value).map_err(|_| {
        bilibili_upstream_error("Bilibili subtitle catalog returned an invalid resource URL")
    })?;
    let allowed_host = matches!(
        url.host_str(),
        Some(
            "aisubtitle.hdslb.com"
                | "i0.hdslb.com"
                | "i1.hdslb.com"
                | "i2.hdslb.com"
                | "s1.hdslb.com"
        )
    );
    let path = url.path();
    let legacy_path = path
        .strip_prefix("/bfs/subtitle/")
        .is_some_and(|suffix| subtitle_path_suffix_allowed(suffix) && suffix.ends_with(".json"));
    let ai_path = path
        .strip_prefix("/bfs/ai_subtitle/prod/")
        .is_some_and(subtitle_path_suffix_allowed);
    if url.scheme() != "https"
        || !allowed_host
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !(legacy_path || ai_path)
        || path.len() > 2048
    {
        return Err(bilibili_upstream_error(
            "Bilibili subtitle catalog returned a disallowed resource URL",
        ));
    }
    Ok(url)
}

fn subtitle_path_suffix_allowed(suffix: &str) -> bool {
    !suffix.is_empty()
        && suffix.len() <= 1024
        && suffix
            .split('/')
            .all(|segment| !segment.is_empty() && !matches!(segment, "." | ".."))
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'-' | b'_' | b'.'))
}

fn normalize_bilibili_image_url(value: &str, context: &str) -> Result<Option<String>> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    let value = if let Some(value) = value.strip_prefix("//") {
        format!("https://{value}")
    } else if let Some(value) = value.strip_prefix("http://") {
        format!("https://{value}")
    } else {
        value.to_owned()
    };
    validated_image_url(Some(&value), context)
}

fn normalize_search_image_url(value: &str) -> Result<String> {
    normalize_bilibili_image_url(value, "search cover")?
        .ok_or_else(|| bilibili_upstream_error("Bilibili video search did not return a cover"))
}

fn parse_duration_seconds(value: &str) -> Result<u64> {
    let parts = value
        .split(':')
        .map(|part| part.parse::<u64>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|_| {
            bilibili_upstream_error("Bilibili video search returned an invalid duration")
        })?;
    match parts.as_slice() {
        [minutes, seconds] if *minutes <= 100_000 && *seconds < 60 => minutes
            .checked_mul(60)
            .and_then(|value| value.checked_add(*seconds))
            .ok_or_else(|| bilibili_upstream_error("Bilibili video search duration overflowed")),
        [hours, minutes, seconds] if *hours <= 10_000 && *minutes < 60 && *seconds < 60 => hours
            .checked_mul(3_600)
            .and_then(|value| value.checked_add(minutes * 60))
            .and_then(|value| value.checked_add(*seconds))
            .ok_or_else(|| bilibili_upstream_error("Bilibili video search duration overflowed")),
        _ => Err(bilibili_upstream_error(
            "Bilibili video search returned an invalid duration",
        )),
    }
}

fn parse_wbi_keys_response(bytes: &[u8]) -> Result<WbiKeys> {
    let response: PassportResponse<NavData> = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili WBI key endpoint returned invalid JSON"))?;
    if !matches!(response.code, 0 | -101) {
        return Err(platform_business_error(
            "Bilibili WBI key request",
            response.code,
            &response.message,
        ));
    }
    let image = response
        .data
        .and_then(|data| data.wbi_img)
        .ok_or_else(|| bilibili_upstream_error("Bilibili WBI key response did not contain keys"))?;
    WbiKeys::from_image_urls(&image.img_url, &image.sub_url)
        .map_err(|_| bilibili_upstream_error("Bilibili WBI key response contained invalid keys"))
}

fn validate_cookie_value(value: &str, name: &str, limit: usize) -> Result<()> {
    if value.len() < 16
        || value.len() > limit
        || value
            .bytes()
            .any(|byte| byte.is_ascii_control() || matches!(byte, b';' | b',' | b' '))
    {
        return Err(bilibili_upstream_error(format!(
            "Bilibili device identity returned an invalid {name}"
        )));
    }
    Ok(())
}

fn parse_cookie_info_response(bytes: &[u8]) -> Result<CookieInfoData> {
    let response: PassportResponse<CookieInfoData> =
        serde_json::from_slice(bytes).map_err(|_| {
            bilibili_upstream_error("Bilibili cookie status endpoint returned invalid JSON")
        })?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili cookie status check",
            response.code,
            &response.message,
        ));
    }
    response
        .data
        .ok_or_else(|| bilibili_upstream_error("Bilibili cookie status did not contain data"))
}

fn cookie_correspond_path(timestamp_ms: u64) -> Result<String> {
    if timestamp_ms == 0 {
        return Err(bilibili_upstream_error(
            "Bilibili cookie refresh timestamp was invalid",
        ));
    }
    let modulus_bytes = BASE64_URL_SAFE_NO_PAD
        .decode(COOKIE_REFRESH_RSA_MODULUS_BASE64URL)
        .map_err(|_| bilibili_internal_error("Bilibili cookie refresh key is invalid"))?;
    if modulus_bytes.len() != 128 {
        return Err(bilibili_internal_error(
            "Bilibili cookie refresh key has an invalid size",
        ));
    }
    let plaintext = format!("refresh_{timestamp_ms}");
    let seed = rand::random::<[u8; 32]>();
    let encoded = oaep_sha256_encode(plaintext.as_bytes(), &seed, modulus_bytes.len())?;
    let modulus = BigUint::from_bytes_be(&modulus_bytes);
    let message = BigUint::from_bytes_be(&encoded);
    if message >= modulus {
        return Err(bilibili_internal_error(
            "Bilibili cookie refresh message exceeded its public key",
        ));
    }
    let encrypted = message.modpow(&BigUint::from(COOKIE_REFRESH_RSA_EXPONENT), &modulus);
    let encrypted = encrypted.to_bytes_be();
    if encrypted.len() > modulus_bytes.len() {
        return Err(bilibili_internal_error(
            "Bilibili cookie refresh ciphertext exceeded its public key",
        ));
    }
    let mut padded = vec![0_u8; modulus_bytes.len() - encrypted.len()];
    padded.extend_from_slice(&encrypted);
    let path = hex::encode(padded);
    validate_correspond_path(&path)?;
    Ok(path)
}

fn oaep_sha256_encode(message: &[u8], seed: &[u8; 32], encoded_len: usize) -> Result<Vec<u8>> {
    const HASH_LEN: usize = 32;
    let minimum = HASH_LEN
        .checked_mul(2)
        .and_then(|value| value.checked_add(2))
        .ok_or_else(|| bilibili_internal_error("Bilibili OAEP size calculation overflowed"))?;
    let padding_len = encoded_len
        .checked_sub(minimum)
        .and_then(|value| value.checked_sub(message.len()))
        .ok_or_else(|| {
            bilibili_internal_error("Bilibili cookie refresh message is too large for its key")
        })?;

    let mut database = Vec::with_capacity(encoded_len - HASH_LEN - 1);
    database.extend_from_slice(digest(&SHA256, b"").as_ref());
    database.resize(database.len() + padding_len, 0);
    database.push(1);
    database.extend_from_slice(message);

    let database_mask = mgf1_sha256(seed, database.len())?;
    for (value, mask) in database.iter_mut().zip(database_mask) {
        *value ^= mask;
    }
    let seed_mask = mgf1_sha256(&database, HASH_LEN)?;
    let mut masked_seed = *seed;
    for (value, mask) in masked_seed.iter_mut().zip(seed_mask) {
        *value ^= mask;
    }

    let mut encoded = Vec::with_capacity(encoded_len);
    encoded.push(0);
    encoded.extend_from_slice(&masked_seed);
    encoded.extend_from_slice(&database);
    if encoded.len() != encoded_len {
        return Err(bilibili_internal_error(
            "Bilibili OAEP encoding produced an invalid size",
        ));
    }
    Ok(encoded)
}

fn mgf1_sha256(seed: &[u8], output_len: usize) -> Result<Vec<u8>> {
    let blocks = output_len.div_ceil(32);
    if blocks > u32::MAX as usize {
        return Err(bilibili_internal_error(
            "Bilibili OAEP mask length exceeded its limit",
        ));
    }
    let mut output = Vec::with_capacity(blocks.saturating_mul(32));
    let mut input = Vec::with_capacity(seed.len().saturating_add(4));
    for counter in 0..blocks {
        input.clear();
        input.extend_from_slice(seed);
        input.extend_from_slice(&(counter as u32).to_be_bytes());
        output.extend_from_slice(digest(&SHA256, &input).as_ref());
    }
    output.truncate(output_len);
    Ok(output)
}

fn validate_correspond_path(value: &str) -> Result<()> {
    if value.len() != 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(bilibili_upstream_error(
            "Bilibili cookie correspondence path was invalid",
        ));
    }
    Ok(())
}

fn parse_refresh_csrf_html(bytes: &[u8]) -> Result<String> {
    let html = std::str::from_utf8(bytes).map_err(|_| {
        bilibili_upstream_error("Bilibili cookie correspondence returned invalid text")
    })?;
    let attribute = ["id=\"1-name\"", "id='1-name'"]
        .into_iter()
        .filter_map(|marker| html.find(marker))
        .min()
        .ok_or_else(|| {
            bilibili_upstream_error(
                "Bilibili cookie correspondence did not contain a refresh token",
            )
        })?;
    let content_start = html[attribute..]
        .find('>')
        .map(|offset| attribute + offset + 1)
        .ok_or_else(|| {
            bilibili_upstream_error("Bilibili cookie correspondence returned malformed HTML")
        })?;
    let content_end = html[content_start..]
        .find("</div>")
        .map(|offset| content_start + offset)
        .ok_or_else(|| {
            bilibili_upstream_error("Bilibili cookie correspondence returned malformed HTML")
        })?;
    let refresh_csrf = html[content_start..content_end].trim().to_owned();
    validate_refresh_csrf(&refresh_csrf)?;
    Ok(refresh_csrf)
}

fn validate_refresh_csrf(value: &str) -> Result<()> {
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(bilibili_upstream_error(
            "Bilibili cookie correspondence returned an invalid refresh token",
        ));
    }
    Ok(())
}

fn parse_cookie_refresh_response(
    bytes: &[u8],
    headers: &HeaderMap,
    expected_user_id: &str,
) -> Result<BilibiliCredential> {
    let response: PassportResponse<CookieRefreshData> =
        serde_json::from_slice(bytes).map_err(|_| {
            bilibili_upstream_error("Bilibili cookie refresh endpoint returned invalid JSON")
        })?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili cookie refresh",
            response.code,
            &response.message,
        ));
    }
    let data = response
        .data
        .ok_or_else(|| bilibili_upstream_error("Bilibili cookie refresh did not contain data"))?;
    if data.status != 0 {
        return Err(platform_business_error(
            "Bilibili cookie refresh",
            data.status,
            &data.message,
        ));
    }
    let mut values = response_cookie_pairs(headers)?;
    let credential = BilibiliCredential {
        dede_user_id: take_cookie(&mut values, "DedeUserID")?,
        dede_user_id_ck_md5: take_cookie(&mut values, "DedeUserID__ckMd5")?,
        sessdata: take_cookie(&mut values, "SESSDATA")?,
        bili_jct: take_cookie(&mut values, "bili_jct")?,
        sid: values.remove("sid"),
        refresh_token: data.refresh_token,
    }
    .normalize()
    .map_err(|_| bilibili_upstream_error("Bilibili cookie refresh returned invalid credentials"))?;
    if credential.user_id() != expected_user_id {
        return Err(bilibili_upstream_error(
            "Bilibili cookie refresh returned a different account identity",
        ));
    }
    Ok(credential)
}

fn parse_cookie_confirm_response(bytes: &[u8]) -> Result<()> {
    let response: PassportResponse<Value> = serde_json::from_slice(bytes).map_err(|_| {
        bilibili_upstream_error("Bilibili cookie confirmation endpoint returned invalid JSON")
    })?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili cookie confirmation",
            response.code,
            &response.message,
        ));
    }
    Ok(())
}

fn parse_logout_response(bytes: &[u8]) -> Result<BilibiliLogoutOutcome> {
    let response = match serde_json::from_slice::<LogoutResponse>(bytes) {
        Ok(response) => response,
        Err(_) if looks_like_html(bytes) => return Ok(BilibiliLogoutOutcome::CredentialExpired),
        Err(_) => {
            return Err(bilibili_upstream_error(
                "Bilibili logout endpoint returned invalid JSON",
            ));
        }
    };
    if response.code == -101 {
        return Ok(BilibiliLogoutOutcome::CredentialExpired);
    }
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili logout",
            response.code,
            &response.message,
        ));
    }
    if response.status != Some(true) {
        return Err(bilibili_upstream_error(
            "Bilibili logout did not confirm success",
        ));
    }
    Ok(BilibiliLogoutOutcome::LoggedOut)
}

fn looks_like_html(bytes: &[u8]) -> bool {
    let prefix = String::from_utf8_lossy(&bytes[..bytes.len().min(512)]).to_ascii_lowercase();
    prefix.contains("<!doctype html") || prefix.contains("<html")
}

fn require_authenticated_session(
    status: BilibiliSessionStatus,
    message: &str,
) -> Result<BilibiliSessionStatus> {
    if status.authenticated {
        Ok(status)
    } else {
        Err(
            TuneWeaveError::new(ErrorCode::AuthenticationRequired, message)
                .with_platform(Platform::Bilibili),
        )
    }
}

fn parse_qr_generate_response(bytes: &[u8]) -> Result<BilibiliQrStart> {
    let response: PassportResponse<QrGenerateData> = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili QR endpoint returned invalid JSON"))?;
    if response.code != 0 {
        let message = response.message.trim();
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            if message.is_empty() {
                format!("Bilibili QR creation failed with code {}", response.code)
            } else {
                format!("Bilibili QR creation failed: {message}")
            },
        )
        .with_platform(Platform::Bilibili)
        .with_details(json!({ "platform_code": response.code })));
    }
    let data = response
        .data
        .ok_or_else(|| bilibili_upstream_error("Bilibili QR response did not contain data"))?;
    validate_qrcode_key(&data.qrcode_key)?;
    validate_qr_login_url(&data.url, &data.qrcode_key)?;
    let image_data_url = qr_image_data_url(&data.url)?;
    Ok(BilibiliQrStart {
        qrcode_key: data.qrcode_key,
        url: data.url,
        image_data_url,
    })
}

fn parse_qr_poll_response(bytes: &[u8], headers: &HeaderMap) -> Result<BilibiliQrPoll> {
    let response: PassportResponse<QrPollData> = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili QR poll endpoint returned invalid JSON"))?;
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili QR polling",
            response.code,
            &response.message,
        ));
    }
    let data = response
        .data
        .ok_or_else(|| bilibili_upstream_error("Bilibili QR poll response did not contain data"))?;
    match data.code {
        86101 => Ok(BilibiliQrPoll::Waiting),
        86090 => Ok(BilibiliQrPoll::Scanned),
        86038 => Ok(BilibiliQrPoll::Expired),
        0 => {
            let credential =
                credential_from_qr_confirmation(headers, &data.url, &data.refresh_token)?;
            Ok(BilibiliQrPoll::Confirmed {
                credential,
                timestamp_ms: data.timestamp.filter(|timestamp| *timestamp > 0),
            })
        }
        code => Ok(BilibiliQrPoll::Failed {
            code,
            message: if data.message.trim().is_empty() {
                format!("Bilibili QR login failed with code {code}")
            } else {
                data.message
            },
        }),
    }
}

fn parse_session_response(
    bytes: &[u8],
    expected_user_id: Option<&str>,
) -> Result<BilibiliSessionStatus> {
    let response: PassportResponse<NavData> = serde_json::from_slice(bytes)
        .map_err(|_| bilibili_upstream_error("Bilibili session endpoint returned invalid JSON"))?;
    if response.code == -101 {
        return Ok(unauthenticated_session(
            expected_user_id,
            response.code,
            None,
        ));
    }
    if response.code != 0 {
        return Err(platform_business_error(
            "Bilibili session check",
            response.code,
            &response.message,
        ));
    }
    let data = response
        .data
        .ok_or_else(|| bilibili_upstream_error("Bilibili session response did not contain data"))?;
    if !data.is_login {
        return Ok(unauthenticated_session(
            expected_user_id,
            response.code,
            Some(data),
        ));
    }
    let user_id = data
        .mid
        .filter(|mid| *mid > 0)
        .map(|mid| mid.to_string())
        .ok_or_else(|| {
            bilibili_upstream_error(
                "Bilibili authenticated session did not contain a valid user ID",
            )
        })?;
    if expected_user_id.is_some_and(|expected| expected != user_id) {
        return Err(bilibili_upstream_error(
            "Bilibili session user does not match the selected credential",
        ));
    }
    let nickname = validated_display_text(data.uname.as_deref(), "nickname")?;
    let avatar_url = validated_image_url(data.face.as_deref(), "avatar")?;
    validate_binary_flag(data.email_verified, "email verification")?;
    validate_binary_flag(data.mobile_verified, "mobile verification")?;
    validate_binary_flag(data.vip_status, "VIP status")?;
    let mut extensions = session_extensions(&data)?;
    extensions.insert("platform_code".to_owned(), json!(response.code));
    Ok(BilibiliSessionStatus {
        authenticated: true,
        user_id: Some(user_id),
        nickname,
        avatar_url,
        extensions,
    })
}

fn unauthenticated_session(
    expected_user_id: Option<&str>,
    platform_code: i64,
    data: Option<NavData>,
) -> BilibiliSessionStatus {
    let mut extensions = BTreeMap::from([("platform_code".to_owned(), json!(platform_code))]);
    if let Some(data) = data
        && let Ok(value) = serde_json::to_value(data)
    {
        extensions.insert("nav".to_owned(), value);
    }
    BilibiliSessionStatus {
        authenticated: false,
        user_id: expected_user_id.map(str::to_owned),
        nickname: None,
        avatar_url: None,
        extensions,
    }
}

fn session_extensions(data: &NavData) -> Result<BTreeMap<String, Value>> {
    let nav = serde_json::to_value(data).map_err(|_| {
        TuneWeaveError::new(
            ErrorCode::InternalError,
            "failed to serialize Bilibili account details",
        )
        .with_platform(Platform::Bilibili)
    })?;
    Ok(BTreeMap::from([("nav".to_owned(), nav)]))
}

fn validate_binary_flag(value: Option<i64>, context: &str) -> Result<()> {
    if value.is_some_and(|value| !matches!(value, 0 | 1)) {
        return Err(bilibili_upstream_error(format!(
            "Bilibili session returned an invalid {context} flag"
        )));
    }
    Ok(())
}

fn validated_display_text(value: Option<&str>, context: &str) -> Result<Option<String>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    if value.len() > 512 || value.chars().any(char::is_control) {
        return Err(bilibili_upstream_error(format!(
            "Bilibili session returned an invalid {context}"
        )));
    }
    Ok(Some(value.to_owned()))
}

fn validated_image_url(value: Option<&str>, context: &str) -> Result<Option<String>> {
    let Some(value) = value.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let url = Url::parse(value).map_err(|_| {
        bilibili_upstream_error(format!("Bilibili returned an invalid {context} URL"))
    })?;
    let trusted_host = url.host_str().is_some_and(|host| {
        host == "hdslb.com"
            || host.ends_with(".hdslb.com")
            || host == "biliimg.com"
            || host.ends_with(".biliimg.com")
    });
    if url.scheme() != "https"
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || !trusted_host
    {
        return Err(bilibili_upstream_error(format!(
            "Bilibili returned an unsafe {context} URL"
        )));
    }
    Ok(Some(value.to_owned()))
}

fn credential_from_qr_confirmation(
    headers: &HeaderMap,
    confirmation_url: &str,
    refresh_token: &str,
) -> Result<BilibiliCredential> {
    let mut values = response_cookie_pairs(headers)?;
    let missing_required_cookie = ["DedeUserID", "DedeUserID__ckMd5", "SESSDATA", "bili_jct"]
        .into_iter()
        .any(|name| !values.contains_key(name));
    if missing_required_cookie && !confirmation_url.trim().is_empty() {
        for (name, value) in trusted_confirmation_query(confirmation_url)? {
            values.entry(name).or_insert(value);
        }
    }
    BilibiliCredential {
        dede_user_id: take_cookie(&mut values, "DedeUserID")?,
        dede_user_id_ck_md5: take_cookie(&mut values, "DedeUserID__ckMd5")?,
        sessdata: take_cookie(&mut values, "SESSDATA")?,
        bili_jct: take_cookie(&mut values, "bili_jct")?,
        sid: values.remove("sid"),
        refresh_token: refresh_token.to_owned(),
    }
    .normalize()
    .map_err(|_| {
        bilibili_upstream_error("Bilibili confirmed QR login returned invalid credentials")
    })
}

fn response_cookie_pairs(headers: &HeaderMap) -> Result<BTreeMap<String, String>> {
    let mut cookies = BTreeMap::new();
    for header in headers.get_all(SET_COOKIE) {
        let value = header.to_str().map_err(|_| {
            bilibili_upstream_error("Bilibili response returned an invalid cookie header")
        })?;
        let Some((name, value)) = value
            .split(';')
            .next()
            .and_then(|pair| pair.split_once('='))
        else {
            continue;
        };
        if matches!(
            name,
            "DedeUserID" | "DedeUserID__ckMd5" | "SESSDATA" | "bili_jct" | "sid"
        ) && !value.is_empty()
        {
            cookies.insert(name.to_owned(), value.to_owned());
        }
    }
    Ok(cookies)
}

fn trusted_confirmation_query(value: &str) -> Result<BTreeMap<String, String>> {
    let url = Url::parse(value)
        .map_err(|_| bilibili_upstream_error("Bilibili QR confirmation returned an invalid URL"))?;
    let trusted_target = matches!(
        (url.host_str(), url.path()),
        (Some("passport.biligame.com"), "/crossDomain")
            | (
                Some("passport.bilibili.com"),
                "/x/passport-login/web/crossDomain"
            )
    );
    if url.scheme() != "https" || url.port().is_some() || !trusted_target {
        return Err(bilibili_upstream_error(
            "Bilibili QR confirmation returned an untrusted URL",
        ));
    }
    Ok(url
        .query_pairs()
        .filter(|(name, value)| {
            matches!(
                name.as_ref(),
                "DedeUserID" | "DedeUserID__ckMd5" | "SESSDATA" | "bili_jct" | "sid"
            ) && !value.is_empty()
        })
        .map(|(name, value)| (name.into_owned(), value.into_owned()))
        .collect())
}

fn take_cookie(values: &mut BTreeMap<String, String>, name: &str) -> Result<String> {
    values.remove(name).ok_or_else(|| {
        bilibili_upstream_error(format!(
            "Bilibili authentication response did not return {name}"
        ))
    })
}

fn validate_qrcode_key(value: &str) -> Result<()> {
    if value.len() != 32 || !value.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(bilibili_upstream_error(
            "Bilibili QR response contained an invalid key",
        ));
    }
    Ok(())
}

fn validate_qr_login_url(value: &str, qrcode_key: &str) -> Result<()> {
    let url = Url::parse(value)
        .map_err(|_| bilibili_upstream_error("Bilibili QR response contained an invalid URL"))?;
    let trusted_path = matches!(
        (url.host_str(), url.path()),
        (Some("passport.bilibili.com"), "/h5-app/passport/login/scan")
            | (Some("account.bilibili.com"), "/h5/account-h5/auth/scan-web")
    );
    let trusted = url.scheme() == "https" && url.port().is_none() && trusted_path;
    let matching_key = url
        .query_pairs()
        .any(|(name, value)| name == "qrcode_key" && value == qrcode_key);
    if !trusted || !matching_key {
        return Err(bilibili_upstream_error(
            "Bilibili QR response contained an untrusted login URL",
        ));
    }
    Ok(())
}

fn qr_image_data_url(url: &str) -> Result<String> {
    let code = QrCode::new(url.as_bytes()).map_err(|_| {
        bilibili_upstream_error("Bilibili QR login URL could not be encoded as an image")
    })?;
    let image = code
        .render::<svg::Color>()
        .min_dimensions(320, 320)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build();
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        BASE64.encode(image.as_bytes())
    ))
}

fn passport_network_error(error: reqwest::Error) -> TuneWeaveError {
    let code = if error.is_timeout() {
        ErrorCode::UpstreamTimeout
    } else {
        ErrorCode::UpstreamError
    };
    TuneWeaveError::new(code, "Bilibili Passport request failed")
        .with_platform(Platform::Bilibili)
        .retryable(true)
}

fn passport_http_error(status: StatusCode) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("Bilibili Passport returned HTTP {status}"),
    )
    .with_platform(Platform::Bilibili)
    .retryable(status.is_server_error())
}

fn bilibili_network_error(error: reqwest::Error) -> TuneWeaveError {
    let code = if error.is_timeout() {
        ErrorCode::UpstreamTimeout
    } else {
        ErrorCode::UpstreamError
    };
    TuneWeaveError::new(code, "Bilibili API request failed")
        .with_platform(Platform::Bilibili)
        .retryable(true)
}

fn bilibili_http_error(context: &str, status: StatusCode) -> TuneWeaveError {
    let code = match status {
        StatusCode::BAD_REQUEST => ErrorCode::InvalidRequest,
        StatusCode::UNAUTHORIZED => ErrorCode::AuthenticationRequired,
        StatusCode::FORBIDDEN => ErrorCode::PermissionDenied,
        StatusCode::NOT_FOUND => ErrorCode::ResourceNotFound,
        StatusCode::PRECONDITION_FAILED | StatusCode::TOO_MANY_REQUESTS => ErrorCode::RateLimited,
        _ => ErrorCode::UpstreamError,
    };
    TuneWeaveError::new(code, format!("{context} returned HTTP {status}"))
        .with_platform(Platform::Bilibili)
        .retryable(
            status.is_server_error()
                || matches!(
                    status,
                    StatusCode::PRECONDITION_FAILED | StatusCode::TOO_MANY_REQUESTS
                ),
        )
}

fn platform_business_error(context: &str, code: i64, message: &str) -> TuneWeaveError {
    let error_code = match code {
        -101 | -111 | 2202 | 86038 | 86095 => ErrorCode::AuthenticationRequired,
        -400 | -304 | 400 => ErrorCode::InvalidRequest,
        -403 | 62004 | 62012 => ErrorCode::PermissionDenied,
        -404 | 11010 | 62002 => ErrorCode::ResourceNotFound,
        -352 | -412 => ErrorCode::RateLimited,
        _ => ErrorCode::UpstreamError,
    };
    let message = if message.trim().is_empty() {
        format!("{context} failed with code {code}")
    } else {
        format!("{context} failed: {message}")
    };
    TuneWeaveError::new(error_code, message)
        .with_platform(Platform::Bilibili)
        .retryable(matches!(code, -352 | -412))
        .with_details(json!({ "platform_code": code }))
}

fn bilibili_upstream_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message)
        .with_platform(Platform::Bilibili)
        .retryable(true)
}

fn bilibili_internal_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::InternalError, message).with_platform(Platform::Bilibili)
}

fn invalid_credential(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Bilibili)
}

fn invalid_bilibili_request(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Bilibili)
}

#[cfg(test)]
mod tests {
    use super::*;

    const QR_KEY: &str = "8587cf8106a0b863c46d6bab913537f6";

    fn qr_fixture(url: &str, key: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "code": 0,
            "message": "0",
            "ttl": 1,
            "data": {
                "url": url,
                "qrcode_key": key
            }
        }))
        .expect("serialize QR fixture")
    }

    fn poll_fixture(code: i64, message: &str, url: &str, refresh_token: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "code": 0,
            "message": "0",
            "ttl": 1,
            "data": {
                "url": url,
                "refresh_token": refresh_token,
                "timestamp": if code == 0 { 1_662_363_009_601_u64 } else { 0 },
                "code": code,
                "message": message
            }
        }))
        .expect("serialize QR poll fixture")
    }

    fn nav_fixture(code: i64, data: Value) -> Vec<u8> {
        serde_json::to_vec(&json!({
            "code": code,
            "message": if code == 0 { "0" } else { "账号未登录" },
            "ttl": 1,
            "data": data
        }))
        .expect("serialize nav fixture")
    }

    fn rotated_cookie_headers(user_id: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for cookie in [
            format!("DedeUserID={user_id}; Path=/"),
            "DedeUserID__ckMd5=abcdef0123456789; Path=/".to_owned(),
            "SESSDATA=rotated%2Csession; Path=/; HttpOnly; Secure".to_owned(),
            "bili_jct=abcdef0123456789abcdef0123456789; Path=/".to_owned(),
            "sid=rotated-sid; Path=/".to_owned(),
        ] {
            headers.append(SET_COOKIE, cookie.parse().expect("cookie header"));
        }
        headers
    }

    #[test]
    fn qr_creation_maps_the_reference_response_without_exposing_the_key() {
        let url = format!(
            "https://passport.bilibili.com/h5-app/passport/login/scan?navhide=1&qrcode_key={QR_KEY}&from="
        );
        let start = parse_qr_generate_response(&qr_fixture(&url, QR_KEY)).expect("valid QR start");
        assert_eq!(start.qrcode_key, QR_KEY);
        assert_eq!(start.url, url);
        assert!(
            start
                .image_data_url
                .starts_with("data:image/svg+xml;base64,")
        );
        let debug = format!("{start:?}");
        assert!(!debug.contains(QR_KEY));
        assert!(!debug.contains("passport/login/scan"));

        let current_url = format!(
            "https://account.bilibili.com/h5/account-h5/auth/scan-web?navhide=1&callback=close&qrcode_key={QR_KEY}&from="
        );
        let current = parse_qr_generate_response(&qr_fixture(&current_url, QR_KEY))
            .expect("current QR start");
        assert_eq!(current.url, current_url);
    }

    #[test]
    fn qr_creation_rejects_untrusted_urls_keys_and_business_errors() {
        for (url, key) in [
            (
                format!("https://example.com/h5-app/passport/login/scan?qrcode_key={QR_KEY}"),
                QR_KEY,
            ),
            (
                "https://passport.bilibili.com/h5-app/passport/login/scan?qrcode_key=other"
                    .to_owned(),
                QR_KEY,
            ),
            (
                "https://passport.bilibili.com/h5-app/passport/login/scan?qrcode_key=short"
                    .to_owned(),
                "short",
            ),
        ] {
            let error = parse_qr_generate_response(&qr_fixture(&url, key))
                .expect_err("invalid QR response must fail");
            assert_eq!(error.code, ErrorCode::UpstreamError);
        }

        let error =
            parse_qr_generate_response(br#"{"code":-1,"message":"rate limited","data":null}"#)
                .expect_err("business error must fail");
        assert_eq!(error.details["platform_code"], -1);
    }

    #[test]
    fn qr_poll_preserves_all_reference_states() {
        let headers = HeaderMap::new();
        assert_eq!(
            parse_qr_poll_response(&poll_fixture(86101, "未扫码", "", ""), &headers)
                .expect("waiting state"),
            BilibiliQrPoll::Waiting
        );
        assert_eq!(
            parse_qr_poll_response(&poll_fixture(86090, "二维码已扫码未确认", "", ""), &headers)
                .expect("scanned state"),
            BilibiliQrPoll::Scanned
        );
        assert_eq!(
            parse_qr_poll_response(&poll_fixture(86038, "二维码已失效", "", ""), &headers)
                .expect("expired state"),
            BilibiliQrPoll::Expired
        );
        assert_eq!(
            parse_qr_poll_response(&poll_fixture(12345, "unknown state", "", ""), &headers)
                .expect("failed state"),
            BilibiliQrPoll::Failed {
                code: 12345,
                message: "unknown state".to_owned()
            }
        );
    }

    #[test]
    fn qr_confirmation_prefers_response_cookies_and_redacts_credentials() {
        let mut headers = HeaderMap::new();
        for cookie in [
            "SESSDATA=private%2Csession; Path=/; HttpOnly; Secure",
            "bili_jct=0123456789abcdef0123456789abcdef; Path=/",
            "DedeUserID=47275982; Path=/",
            "DedeUserID__ckMd5=0123456789abcdef; Path=/",
            "sid=private-sid; Path=/",
        ] {
            headers.append(SET_COOKIE, cookie.parse().expect("cookie header"));
        }
        let outcome = parse_qr_poll_response(
            &poll_fixture(
                0,
                "",
                "https://passport.biligame.com/crossDomain?DedeUserID=999&SESSDATA=fallback",
                "private-refresh",
            ),
            &headers,
        )
        .expect("confirmed state");
        let BilibiliQrPoll::Confirmed {
            credential,
            timestamp_ms,
        } = outcome
        else {
            panic!("expected confirmed QR state");
        };
        assert_eq!(credential.user_id(), "47275982");
        assert_eq!(credential.sessdata, "private%2Csession");
        assert_eq!(timestamp_ms, Some(1_662_363_009_601));
        let debug = format!("{credential:?}");
        assert!(!debug.contains("private"));
        assert!(
            !format!(
                "{:?}",
                BilibiliQrPoll::Confirmed {
                    credential,
                    timestamp_ms
                }
            )
            .contains("private")
        );
    }

    #[test]
    fn qr_confirmation_rejects_missing_or_untrusted_credential_sources() {
        let headers = HeaderMap::new();
        for url in [
            "",
            "https://example.com/crossDomain?DedeUserID=47275982",
            "http://passport.biligame.com/crossDomain?DedeUserID=47275982",
        ] {
            let error =
                parse_qr_poll_response(&poll_fixture(0, "", url, "private-refresh"), &headers)
                    .expect_err("incomplete confirmation must fail");
            assert_eq!(error.code, ErrorCode::UpstreamError);
            assert!(!format!("{error:?}").contains("private-refresh"));
        }
    }

    #[test]
    fn credential_validation_rejects_zero_padded_zero_user_ids() {
        let error = BilibiliCredential {
            dede_user_id: "00".to_owned(),
            dede_user_id_ck_md5: "0123456789abcdef".to_owned(),
            sessdata: "session".to_owned(),
            bili_jct: "0123456789abcdef0123456789abcdef".to_owned(),
            sid: None,
            refresh_token: "refresh".to_owned(),
        }
        .normalize()
        .expect_err("zero-valued user IDs must fail");
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn session_status_maps_authenticated_and_anonymous_responses() {
        let authenticated = parse_session_response(
            &nav_fixture(
                0,
                json!({
                    "isLogin": true,
                    "email_verified": 1,
                    "mobile_verified": 0,
                    "face": "https://i0.hdslb.com/bfs/face/avatar.jpg",
                    "mid": 47275982,
                    "uname": "Lotus",
                    "vipDueDate": 1_700_000_000_000_u64,
                    "vipStatus": 1,
                    "vipType": 2,
                    "wbi_img": {
                        "img_url": "https://i0.hdslb.com/bfs/wbi/image.png",
                        "sub_url": "https://i0.hdslb.com/bfs/wbi/sub.png"
                    },
                    "new_upstream_field": { "preserved_in_parser_only": true }
                }),
            ),
            Some("47275982"),
        )
        .expect("authenticated nav response");
        assert!(authenticated.authenticated);
        assert_eq!(authenticated.user_id.as_deref(), Some("47275982"));
        assert_eq!(authenticated.nickname.as_deref(), Some("Lotus"));
        assert_eq!(
            authenticated.avatar_url.as_deref(),
            Some("https://i0.hdslb.com/bfs/face/avatar.jpg")
        );
        assert_eq!(authenticated.extensions["platform_code"], 0);
        assert_eq!(authenticated.extensions["nav"]["vip_status"], 1);
        assert!(
            authenticated.extensions["nav"]
                .get("new_upstream_field")
                .is_none()
        );

        let anonymous = parse_session_response(
            &nav_fixture(-101, json!({ "isLogin": false })),
            Some("47275982"),
        )
        .expect("anonymous nav response");
        assert!(!anonymous.authenticated);
        assert_eq!(anonymous.user_id.as_deref(), Some("47275982"));
        assert_eq!(anonymous.extensions["platform_code"], -101);
    }

    #[test]
    fn session_status_rejects_identity_mismatch_flags_and_unsafe_avatar_urls() {
        for (data, expected_message) in [
            (
                json!({ "isLogin": true, "mid": 1, "uname": "other" }),
                "does not match",
            ),
            (
                json!({ "isLogin": true, "mid": 47275982, "email_verified": 2 }),
                "verification flag",
            ),
            (
                json!({
                    "isLogin": true,
                    "mid": 47275982,
                    "face": "javascript:alert(1)"
                }),
                "avatar URL",
            ),
        ] {
            let error = parse_session_response(&nav_fixture(0, data), Some("47275982"))
                .expect_err("invalid session response must fail");
            assert_eq!(error.code, ErrorCode::UpstreamError);
            assert!(error.message.contains(expected_message));
        }
    }

    #[test]
    fn session_cookie_excludes_refresh_material_and_debug_redacts_secrets() {
        let credential = BilibiliCredential {
            dede_user_id: "47275982".to_owned(),
            dede_user_id_ck_md5: "0123456789abcdef".to_owned(),
            sessdata: "private-session".to_owned(),
            bili_jct: "0123456789abcdef0123456789abcdef".to_owned(),
            sid: Some("private-sid".to_owned()),
            refresh_token: "private-refresh".to_owned(),
        }
        .normalize()
        .expect("credential");
        let cookie = credential.cookie_header();
        assert!(cookie.contains("SESSDATA=private-session"));
        assert!(cookie.contains("sid=private-sid"));
        assert!(!cookie.contains("private-refresh"));
        let debug = format!("{credential:?}");
        assert!(!debug.contains("private"));
    }

    #[test]
    fn cookie_refresh_status_preserves_required_flag_and_server_timestamp() {
        let needed = parse_cookie_info_response(
            br#"{"code":0,"message":"0","data":{"refresh":true,"timestamp":1684466082562}}"#,
        )
        .expect("refresh status");
        assert!(needed.refresh);
        assert_eq!(needed.timestamp, 1_684_466_082_562);

        let current = parse_cookie_info_response(
            br#"{"code":0,"message":"0","data":{"refresh":false,"timestamp":1684466082562}}"#,
        )
        .expect("current status");
        assert!(!current.refresh);

        let error =
            parse_cookie_info_response(br#"{"code":-101,"message":"not logged in","data":null}"#)
                .expect_err("expired cookie must fail refresh status");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[test]
    fn correspond_path_uses_fixed_rsa_oaep_shape() {
        let first = cookie_correspond_path(1_684_466_082_562).expect("first path");
        let second = cookie_correspond_path(1_684_466_082_562).expect("second path");
        assert_eq!(first.len(), 256);
        assert!(
            first
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        );
        assert_ne!(first, second, "OAEP encryption must use fresh randomness");
    }

    #[test]
    fn cookie_correspondence_extracts_only_a_valid_refresh_csrf() {
        let csrf = parse_refresh_csrf_html(
            br#"<html><body><div class="token" id="1-name">b0cc8411ded2f9db2cff2edb3123acac</div></body></html>"#,
        )
        .expect("refresh csrf");
        assert_eq!(csrf, "b0cc8411ded2f9db2cff2edb3123acac");

        for invalid in [
            br#"<html><div id="other">b0cc8411ded2f9db2cff2edb3123acac</div></html>"#.as_slice(),
            br#"<html><div id='1-name'>not-a-token</div></html>"#.as_slice(),
            b"\xff\xfe".as_slice(),
        ] {
            assert!(parse_refresh_csrf_html(invalid).is_err());
        }
    }

    #[test]
    fn cookie_rotation_requires_a_complete_same_identity_generation() {
        let response = br#"{"code":0,"message":"0","data":{"status":0,"message":"","refresh_token":"rotated-refresh"}}"#;
        let credential = parse_cookie_refresh_response(
            response,
            &rotated_cookie_headers("47275982"),
            "47275982",
        )
        .expect("rotated credential");
        assert_eq!(credential.user_id(), "47275982");
        assert_eq!(credential.sessdata, "rotated%2Csession");
        assert_eq!(credential.refresh_token, "rotated-refresh");
        assert!(!format!("{credential:?}").contains("rotated-refresh"));

        let mismatch =
            parse_cookie_refresh_response(response, &rotated_cookie_headers("999"), "47275982")
                .expect_err("identity mismatch must fail");
        assert!(mismatch.message.contains("different account identity"));

        let missing = parse_cookie_refresh_response(response, &HeaderMap::new(), "47275982")
            .expect_err("missing cookies must fail");
        assert_eq!(missing.code, ErrorCode::UpstreamError);
    }

    #[test]
    fn cookie_confirmation_and_logout_preserve_terminal_semantics() {
        parse_cookie_confirm_response(br#"{"code":0,"message":"0","ttl":1}"#)
            .expect("refresh confirmation");
        let confirmation_error =
            parse_cookie_confirm_response(br#"{"code":-111,"message":"csrf rejected","ttl":1}"#)
                .expect_err("confirmation failure");
        assert_eq!(confirmation_error.code, ErrorCode::AuthenticationRequired);

        assert_eq!(
            parse_logout_response(br#"{"code":0,"status":true,"ts":1663034005}"#)
                .expect("logged out"),
            BilibiliLogoutOutcome::LoggedOut
        );
        assert_eq!(
            parse_logout_response(b"<!DOCTYPE html><html><body>login</body></html>")
                .expect("expired HTML session"),
            BilibiliLogoutOutcome::CredentialExpired
        );
        assert_eq!(
            parse_logout_response(br#"{"code":-101,"message":"not logged in"}"#)
                .expect("expired JSON session"),
            BilibiliLogoutOutcome::CredentialExpired
        );
        let csrf_error =
            parse_logout_response(br#"{"code":2202,"status":false,"message":"csrf rejected"}"#)
                .expect_err("invalid csrf must fail without claiming logout");
        assert_eq!(csrf_error.code, ErrorCode::AuthenticationRequired);
    }

    #[test]
    fn web_device_and_wbi_keys_are_strictly_parsed() {
        let client = BilibiliClient::new(&BilibiliConfig::default()).expect("Bilibili client");
        let first_query_visit_id = client.web_query_visit_id().expect("query visit ID");
        let second_query_visit_id = client.web_query_visit_id().expect("cached query visit ID");
        assert_eq!(first_query_visit_id, second_query_visit_id);
        assert_eq!(first_query_visit_id.len(), 32);
        assert!(
            first_query_visit_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric())
        );
        assert!(
            !client
                .video_search_compatibility_active()
                .expect("initial search protocol")
        );
        client
            .mark_video_search_challenged()
            .expect("mark challenged search");
        assert!(
            client
                .video_search_compatibility_active()
                .expect("cached search protocol")
        );

        let mut device = parse_device_identity_response(
            br#"{"code":0,"message":"ok","data":{"b_3":"54E5EFC1-3C8F-F690-2261-439E4F6A20A979439infoc","b_4":"F6E0FD4B-520C-1902-4F7B-E461D8D1F5AB79044-024072309-666onEZSnlHVPjoRp4kDYg=="}}"#,
        )
        .expect("device identity");
        device.b_nut = "1721975923".to_owned();
        assert!(device.cookie_header().contains("buvid3="));
        assert!(device.cookie_header().contains("buvid4="));
        assert!(device.cookie_header().contains("b_nut=1721975923"));
        assert!(!format!("{device:?}").contains("54E5EFC1"));

        let keys = parse_wbi_keys_response(&nav_fixture(
            -101,
            json!({
                "isLogin": false,
                "wbi_img": {
                    "img_url": "https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png",
                    "sub_url": "https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png"
                }
            }),
        ))
        .expect("anonymous WBI keys");
        assert!(
            keys.sign(&[("keyword".to_owned(), "音乐".to_owned())], 1)
                .expect("signed query")
                .contains("w_rid=")
        );

        assert!(
            parse_device_identity_response(
                br#"{"code":0,"message":"ok","data":{"b_3":"unsafe;cookie","b_4":"valid-device-value"}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn web_ticket_keeps_ticket_and_wbi_lifetimes_together() {
        let response = serde_json::to_vec(&json!({
            "code": 0,
            "message": "OK",
            "data": {
                "ticket": concat!(
                    "eyJhbGciOiJIUzI1NiIsImtpZCI6InMwMyIsInR5cCI6IkpXVCJ9.",
                    "eyJleHAiOjE3MjM2OTMwODAsImlhdCI6MTcyMzQzMzgyMCwicGx0IjotMX0.",
                    "efOwv7i4m0ykABrXEDHGAechU2AByMcP_-3EYpQrNKs"
                ),
                "created_at": 1723433820_u64,
                "ttl": 259200,
                "nav": {
                    "img": "https://i0.hdslb.com/bfs/wbi/7cd084941338484aae1ad9425b84077c.png",
                    "sub": "https://i0.hdslb.com/bfs/wbi/4932caff0ff746eab6f01bf08b70ac45.png"
                }
            }
        }))
        .expect("ticket fixture");
        let (ticket, keys) = parse_web_ticket_response(&response).expect("web ticket");
        assert!(ticket.is_current());
        assert!(!format!("{ticket:?}").contains("eyJ"));
        assert!(
            keys.sign(&[("keyword".to_owned(), "音乐".to_owned())], 1)
                .expect("signed query")
                .contains("w_rid=")
        );

        for invalid in [
            br#"{"code":400,"message":"bad request","data":null}"#.as_slice(),
            br#"{"code":0,"message":"OK","data":{"ticket":"short","created_at":1,"ttl":259200,"nav":{"img":"","sub":""}}}"#
                .as_slice(),
        ] {
            assert!(parse_web_ticket_response(invalid).is_err());
        }
    }

    #[test]
    fn search_suggestions_separate_plain_display_text_and_highlight_ranges() {
        let response = serde_json::to_vec(&json!({
            "exp_str": "106301_106700",
            "code": 0,
            "result": {
                "tag": [{
                    "value": "洛天依歌曲",
                    "term": "洛天依歌曲",
                    "ref": 0,
                    "name": "<em class=\"suggest_high_light\">洛天依</em>&amp;歌曲",
                    "spid": 5,
                    "type": "",
                    "item_feature": "",
                    "future_field": {"kept": true}
                }],
                "future_bucket": ["kept"]
            },
            "stoken": "4020133863501304726",
            "total_count": "1",
            "debug_response_info": {"debug_record": []},
            "user_feature": ""
        }))
        .expect("suggestion fixture");
        let result = parse_search_suggestion_response(&response).expect("suggestions");
        assert_eq!(result.response_code, 0);
        assert_eq!(result.reported_total, Some(1));
        assert_eq!(result.suggestions[0].keyword, "洛天依歌曲");
        assert_eq!(result.suggestions[0].display_text, "洛天依&歌曲");
        assert_eq!(
            result.suggestions[0].highlight_ranges,
            [BilibiliTextRange { start: 0, end: 3 }]
        );
        assert_eq!(
            result.suggestions[0].extensions["future_field"]["kept"],
            true
        );
        assert_eq!(result.result_extensions["future_bucket"][0], "kept");
        assert!(result.extensions.contains_key("debug_response_info"));
    }

    #[test]
    fn search_suggestions_accept_the_platform_empty_code_and_reject_unsafe_markup() {
        let empty = parse_search_suggestion_response(
            br#"{"exp_str":"","code":3,"result":{"tag":[]},"stoken":"1","total_count":0}"#,
        )
        .expect("empty suggestion response");
        assert!(empty.suggestions.is_empty());
        assert_eq!(empty.response_code, 3);

        for malformed in [
            br#"{"code":0,"result":{"tag":[{"value":"safe","name":"<script>unsafe</script>","ref":0,"spid":5}]},"total_count":1}"#
                .as_slice(),
            br#"{"code":0,"result":{"tag":[{"value":"safe","name":"<em class=\"suggest_high_light\">safe","ref":0,"spid":5}]},"total_count":1}"#
                .as_slice(),
            br#"{"code":3,"result":{"tag":[{"value":"unexpected","name":"unexpected","ref":0,"spid":5}]},"total_count":1}"#
                .as_slice(),
        ] {
            assert_eq!(
                parse_search_suggestion_response(malformed)
                    .expect_err("malformed suggestion")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn trending_search_preserves_rank_metadata_and_normalizes_safe_urls() {
        let response = serde_json::to_vec(&json!({
            "code": 0,
            "message": "OK",
            "ttl": 1,
            "data": {
                "trending": {
                    "title": "bilibili热搜",
                    "trackid": "16377692631482314192",
                    "list": [{
                        "keyword": "KSG 重庆狼队",
                        "show_name": "重庆狼队战胜KSG",
                        "icon": "http://i0.hdslb.com/bfs/activity-plat/hot.png",
                        "uri": "bilibili://search?keyword=KSG",
                        "goto": "search",
                        "heat_score": "648274",
                        "word_type": 5,
                        "future_entry": true
                    }],
                    "top_list": [{"future": "kept"}],
                    "future_catalog": 1
                },
                "future_data": "kept"
            }
        }))
        .expect("trending fixture");
        let result = parse_search_trending_response(&response).expect("trending search");
        assert_eq!(result.title, "bilibili热搜");
        assert_eq!(result.track_id.as_deref(), Some("16377692631482314192"));
        assert_eq!(result.entries[0].keyword, "KSG 重庆狼队");
        assert_eq!(result.entries[0].display_text, "重庆狼队战胜KSG");
        assert_eq!(result.entries[0].heat_score, Some(648_274));
        assert_eq!(result.entries[0].word_type, Some(5));
        assert_eq!(
            result.entries[0].icon_url.as_deref(),
            Some("https://i0.hdslb.com/bfs/activity-plat/hot.png")
        );
        assert_eq!(
            result.entries[0].action_uri.as_deref(),
            Some("bilibili://search?keyword=KSG")
        );
        assert_eq!(result.entries[0].extensions["future_entry"], true);
        assert_eq!(result.top_list[0]["future"], "kept");
        assert_eq!(result.extensions["future_data"], "kept");
        assert_eq!(result.trending_extensions["future_catalog"], 1);
    }

    #[test]
    fn trending_search_rejects_business_errors_and_unsafe_actions() {
        for malformed in [
            br#"{"code":-400,"message":"bad request","data":null}"#.as_slice(),
            br#"{"code":0,"message":"OK","data":{"trending":{"title":"hot","trackid":"1","list":[{"keyword":"safe","show_name":"safe","icon":"","uri":"https://evil.example/redirect","goto":"","heat_score":1}],"top_list":[]}}}"#
                .as_slice(),
            br#"{"code":0,"message":"OK","data":{"trending":{"title":"hot","trackid":"1","list":[{"keyword":"safe","show_name":"safe","icon":"","uri":"","goto":"","heat_score":"invalid"}],"top_list":[]}}}"#
                .as_slice(),
            br#"{"code":0,"message":"OK","data":{"trending":{"title":"hot","trackid":"1","list":[{"keyword":"safe","show_name":"safe","icon":"","uri":"","goto":"","word_type":-1}],"top_list":[]}}}"#
                .as_slice(),
        ] {
            assert!(
                parse_search_trending_response(malformed).is_err(),
                "malformed trending response must fail"
            );
        }
        assert_eq!(
            normalize_bilibili_action_uri("http://www.bilibili.com/video/BV1xx411c7mD")
                .expect("trusted HTTP action")
                .as_deref(),
            Some("https://www.bilibili.com/video/BV1xx411c7mD")
        );
    }

    #[test]
    fn video_search_mapping_preserves_identity_counts_and_pagination() {
        let filters = BilibiliVideoSearchFilters {
            order: BilibiliVideoSearchOrder::MostFavorited,
            duration: BilibiliVideoSearchDuration::ThirtyToSixtyMinutes,
            category_id: Some(3),
        };
        assert_eq!(filters.order.parameter(), "stow");
        assert_eq!(filters.duration.parameter(), "3");
        assert_eq!(filters.category_parameter(), "3");

        let response = serde_json::to_vec(&json!({
            "code": 0,
            "message": "0",
            "data": {
                "seid": 8850295244740510044_u64,
                "page": 2,
                "pagesize": VIDEO_SEARCH_PAGE_SIZE,
                "numResults": "41",
                "numPages": 3,
                "result": [{
                    "type": "video",
                    "aid": 78977417,
                    "bvid": "BV1KJ411C7Un",
                    "title": "初音<em class=\"keyword\">未来</em>",
                    "author": "MitchieM",
                    "mid": 5669526,
                    "description": "音乐 &amp; 视频",
                    "pic": "//i1.hdslb.com/bfs/archive/cover.jpg",
                    "duration": "4:02",
                    "play": "2915520",
                    "video_review": 14572,
                    "favorites": 114102,
                    "review": 6124,
                    "pubdate": 1579877678,
                    "senddate": 1593099008,
                    "typeid": "30",
                    "typename": "VOCALOID·UTAU",
                    "tag": "音乐,初音未来",
                    "hit_columns": ["title", "tag"],
                    "is_pay": 0,
                    "is_union_video": 1,
                    "rank_score": 109020056
                }]
            }
        }))
        .expect("search fixture");
        let page = parse_video_search_response(&response, 2).expect("search page");
        assert_eq!(page.total, 41);
        assert_eq!(page.page_count, 3);
        assert_eq!(page.search_id, "8850295244740510044");
        assert_eq!(page.videos[0].bvid.as_deref(), Some("BV1KJ411C7Un"));
        assert_eq!(page.videos[0].title, "初音未来");
        assert_eq!(page.videos[0].description, "音乐 & 视频");
        assert_eq!(page.videos[0].duration_seconds, 242);
        assert_eq!(page.videos[0].play_count, Some(2_915_520));
        assert_eq!(page.videos[0].collaborative, Some(true));
        assert_eq!(page.videos[0].hit_columns, ["title", "tag"]);

        let nullable_hit_columns: VideoSearchItem = serde_json::from_value(json!({
            "type": "video",
            "aid": 1,
            "bvid": "BV1xx411c7mD",
            "title": "title",
            "author": "author",
            "mid": 1,
            "description": "",
            "pic": "//i0.hdslb.com/bfs/archive/cover.jpg",
            "duration": "0:01",
            "hit_columns": null
        }))
        .expect("nullable hit columns");
        assert!(nullable_hit_columns.hit_columns.is_none());
    }

    #[test]
    fn video_search_maps_business_errors_without_exposing_challenges() {
        let blocked = parse_video_search_response(
            br#"{"code":-412,"message":"request blocked","data":null}"#,
            1,
        )
        .expect_err("blocked search");
        assert_eq!(blocked.code, ErrorCode::RateLimited);
        assert!(blocked.retryable);
        assert!(!is_video_search_risk_challenge(&blocked));
        let blocked_http =
            bilibili_http_error("Bilibili video search", StatusCode::PRECONDITION_FAILED);
        assert_eq!(blocked_http.code, ErrorCode::RateLimited);
        assert!(blocked_http.retryable);

        let voucher = parse_video_search_response(
            br#"{"code":0,"message":"0","data":{"v_voucher":"private-voucher"}}"#,
            1,
        )
        .expect_err("risk challenge");
        assert_eq!(voucher.code, ErrorCode::RateLimited);
        assert!(is_video_search_risk_challenge(&voucher));
        assert!(!format!("{voucher:?}").contains("private-voucher"));

        let malformed = parse_video_search_response(
            br#"{"code":0,"message":"0","data":{"seid":"1","page":2,"pagesize":30,"numResults":0,"numPages":0,"result":[]}}"#,
            2,
        )
        .expect_err("changed page size");
        assert_eq!(malformed.code, ErrorCode::UpstreamError);
    }

    #[test]
    fn video_view_preserves_identity_parts_rights_and_rejects_drift() {
        let fixture = json!({
            "code": 0,
            "message": "OK",
            "data": {
                "aid": 85440373,
                "bvid": "BV117411r7R1",
                "videos": 1,
                "tid": 28,
                "tid_v2": 2061,
                "tname": "",
                "tname_v2": "人力VOCALOID",
                "copyright": 1,
                "pic": "http://i1.hdslb.com/bfs/archive/cover.jpg",
                "title": "视频标题",
                "pubdate": 1580377255,
                "ctime": 1580212263,
                "desc": "第一行\n第二行",
                "state": 0,
                "duration": 486,
                "rights": {
                    "download": 1,
                    "movie": 0,
                    "pay": 0,
                    "hd5": 1,
                    "no_reprint": 1,
                    "ugc_pay": 0,
                    "is_cooperation": 0,
                    "is_stein_gate": 0,
                    "is_360": 0,
                    "no_share": 0,
                    "free_watch": 1
                },
                "owner": {
                    "mid": 101,
                    "name": "UP 主",
                    "face": "//i0.hdslb.com/bfs/face/avatar.jpg"
                },
                "stat": {
                    "aid": 85440373,
                    "view": 100,
                    "danmaku": 20,
                    "reply": 10,
                    "favorite": 9,
                    "coin": 8,
                    "share": 7,
                    "now_rank": 0,
                    "his_rank": 3,
                    "like": 30
                },
                "dynamic": "动态文本",
                "cid": 146044693,
                "pages": [{
                    "cid": 146044693,
                    "page": 1,
                    "from": "vupload",
                    "part": "正片",
                    "duration": 486,
                    "dimension": {"width": 1920, "height": 1080, "rotate": 0}
                }]
            }
        });
        let bytes = serde_json::to_vec(&fixture).expect("video view fixture");
        let view = parse_video_view_response(
            &bytes,
            &crate::BilibiliVideoIdentity::Bvid("BV117411r7R1".to_owned()),
        )
        .expect("video view");
        assert_eq!(view.aid, 85_440_373);
        assert_eq!(view.category_name, None);
        assert_eq!(view.category_name_v2.as_deref(), Some("人力VOCALOID"));
        assert_eq!(view.parts[0].cid, 146_044_693);
        assert_eq!(view.parts[0].width, 1920);
        assert!(view.rights.download);
        assert!(view.rights.high_bitrate);
        assert_eq!(view.cover_url, "https://i1.hdslb.com/bfs/archive/cover.jpg");

        for (pointer, value) in [
            ("/data/stat/aid", json!(1)),
            ("/data/pages/0/page", json!(2)),
            ("/data/rights/download", json!(2)),
            ("/data/bvid", json!("BV1Q541167Qg")),
        ] {
            let mut malformed = fixture.clone();
            *malformed.pointer_mut(pointer).expect("fixture field") = value;
            let bytes = serde_json::to_vec(&malformed).expect("malformed fixture");
            let error = parse_video_view_response(
                &bytes,
                &crate::BilibiliVideoIdentity::Bvid("BV117411r7R1".to_owned()),
            )
            .expect_err("video view drift");
            assert_eq!(error.code, ErrorCode::UpstreamError);
        }
    }

    #[test]
    fn subtitle_catalog_preserves_tracks_without_exposing_untrusted_hosts() {
        let fixture = json!({
            "code": 0,
            "message": "0",
            "data": {
                "aid": 60977932,
                "bvid": "BV1Jt411P77c",
                "cid": 106101299,
                "need_login_subtitle": false,
                "subtitle": {
                    "allow_submit": true,
                    "lan": "zh-CN",
                    "lan_doc": "中文（中国）",
                    "subtitles": [{
                        "id": 13643112644608002_u64,
                        "id_str": "13643112644608002",
                        "lan": "zh-Hans",
                        "lan_doc": "中文（简体）",
                        "is_lock": true,
                        "subtitle_url": "//aisubtitle.hdslb.com/bfs/ai_subtitle/prod/c49b18a284739d99df1e3723cdf72c0c82db98e0?auth_key=redacted",
                        "type": 0,
                        "ai_type": 0,
                        "ai_status": 0
                    }, {
                        "id": 13643200114196484_u64,
                        "id_str": "13643200114196484",
                        "lan": "en-US",
                        "lan_doc": "英语（美国）",
                        "is_lock": false,
                        "subtitle_url": "https://i0.hdslb.com/bfs/subtitle/2b38bc0f5d7671176964d4c3de441ed37568500c.json",
                        "type": 0,
                        "ai_type": 1,
                        "ai_status": 2
                    }]
                }
            }
        });
        let bytes = serde_json::to_vec(&fixture).expect("subtitle fixture");
        let catalog = parse_video_subtitle_catalog(&bytes, 60_977_932, "BV1Jt411P77c", 106_101_299)
            .expect("subtitle catalog");
        assert_eq!(catalog.subtitles.len(), 2);
        assert_eq!(catalog.default_language.as_deref(), Some("zh-CN"));
        assert_eq!(catalog.subtitles[0].id_string, "13643112644608002");
        assert_eq!(
            catalog.subtitles[0].resource_url.host_str(),
            Some("aisubtitle.hdslb.com")
        );
        assert!(catalog.subtitles[0].locked);
        assert_eq!(catalog.subtitles[1].ai_type, 1);

        for (pointer, value) in [
            ("/data/cid", json!(1)),
            (
                "/data/subtitle/subtitles/0/subtitle_url",
                json!("https://127.0.0.1/bfs/subtitle/internal.json"),
            ),
            (
                "/data/subtitle/subtitles/0/id_str",
                json!("13643200114196484"),
            ),
            (
                "/data/subtitle/subtitles/0/lan",
                json!("zh-Hans?redirect=true"),
            ),
        ] {
            let mut malformed = fixture.clone();
            *malformed.pointer_mut(pointer).expect("fixture field") = value;
            let bytes = serde_json::to_vec(&malformed).expect("malformed subtitle fixture");
            let error =
                parse_video_subtitle_catalog(&bytes, 60_977_932, "BV1Jt411P77c", 106_101_299)
                    .expect_err("subtitle catalog drift");
            assert_eq!(error.code, ErrorCode::UpstreamError);
        }
    }

    #[test]
    fn subtitle_body_preserves_current_ai_schema_as_typed_cues() {
        let bytes = serde_json::to_vec(&json!({
            "font_size": 0.4,
            "font_color": "#FFFFFF",
            "background_alpha": 0.5,
            "background_color": "#9C27B0",
            "Stroke": "none",
            "type": "AIsubtitle",
            "lang": "zh",
            "version": "v1.7.0.4",
            "body": [{
                "from": 12.18,
                "to": 18.18,
                "sid": 1,
                "location": 2,
                "content": "♪ 保留字幕正文中的空白 ♪",
                "music": 0.9999
            }, {
                "from": 18.2,
                "to": 20.0,
                "sid": "2",
                "content": "第二句"
            }]
        }))
        .expect("subtitle body fixture");

        let body = parse_subtitle_body(&bytes).expect("subtitle body");
        assert_eq!(body.source_language.as_deref(), Some("zh"));
        assert_eq!(body.source_type.as_deref(), Some("AIsubtitle"));
        assert_eq!(body.source_version.as_deref(), Some("v1.7.0.4"));
        assert_eq!(body.font_size, Some(0.4));
        assert_eq!(body.font_color.as_deref(), Some("#FFFFFF"));
        assert_eq!(body.background_alpha, Some(0.5));
        assert_eq!(body.background_color.as_deref(), Some("#9C27B0"));
        assert_eq!(body.stroke.as_deref(), Some("none"));
        assert_eq!(body.cues.len(), 2);
        assert_eq!(body.cues[0].id.as_deref(), Some("1"));
        assert_eq!(body.cues[0].start_seconds, 12.18);
        assert_eq!(body.cues[0].end_seconds, 18.18);
        assert_eq!(body.cues[0].position, Some(2));
        assert_eq!(body.cues[0].music_confidence, Some(0.9999));
        assert_eq!(body.cues[1].id.as_deref(), Some("2"));
    }

    #[test]
    fn subtitle_body_rejects_invalid_style_timing_and_cue_fields() {
        let fixture = json!({
            "font_size": 0.4,
            "background_alpha": 0.5,
            "lang": "zh",
            "body": [{
                "from": 1.0,
                "to": 2.0,
                "sid": 1,
                "location": 2,
                "content": "有效字幕",
                "music": 0.25
            }]
        });

        for (pointer, value) in [
            ("/font_size", json!(-0.1)),
            ("/background_alpha", json!(1.1)),
            ("/lang", json!("zh\nredirect")),
            ("/body/0/from", json!(-1.0)),
            ("/body/0/to", json!(0.5)),
            ("/body/0/location", json!(1001)),
            ("/body/0/content", json!(" \t ")),
            ("/body/0/music", json!(1.1)),
        ] {
            let mut malformed = fixture.clone();
            *malformed.pointer_mut(pointer).expect("fixture field") = value;
            let bytes = serde_json::to_vec(&malformed).expect("malformed subtitle body fixture");
            let error = parse_subtitle_body(&bytes).expect_err("subtitle body drift");
            assert_eq!(error.code, ErrorCode::UpstreamError, "{pointer}");
        }
    }

    #[test]
    fn subtitle_resource_urls_allow_only_fixed_https_cdn_paths() {
        for allowed in [
            "https://aisubtitle.hdslb.com/bfs/ai_subtitle/prod/opaque-id?auth_key=secret",
            "//i0.hdslb.com/bfs/subtitle/opaque.json",
        ] {
            let url = normalize_bilibili_subtitle_url(allowed).expect("trusted subtitle URL");
            assert_eq!(url.scheme(), "https");
        }

        for rejected in [
            "http://aisubtitle.hdslb.com/bfs/ai_subtitle/prod/opaque",
            "https://example.test/bfs/ai_subtitle/prod/opaque",
            "https://127.0.0.1/bfs/subtitle/opaque.json",
            "https://aisubtitle.hdslb.com:8443/bfs/ai_subtitle/prod/opaque",
            "https://user@aisubtitle.hdslb.com/bfs/ai_subtitle/prod/opaque",
            "https://aisubtitle.hdslb.com/bfs/ai_subtitle/prod/../opaque",
            "https://aisubtitle.hdslb.com/bfs/ai_subtitle/prod/%2e%2e/opaque",
            "https://aisubtitle.hdslb.com/bfs/other/opaque.json",
            "file:///bfs/subtitle/opaque.json",
        ] {
            let error =
                normalize_bilibili_subtitle_url(rejected).expect_err("disallowed subtitle URL");
            assert_eq!(error.code, ErrorCode::UpstreamError, "{rejected}");
        }
    }

    fn playback_track_fixture(id: u32, media: &str, codec: &str, codec_id: i64) -> Value {
        let base = format!(
            "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/99/12/106101299/{media}.m4s?deadline=2000000000"
        );
        let backup = format!(
            "https://upos-sz-mirrorali.bilivideo.com/upgcxcode/99/12/106101299/{media}.m4s?deadline=1999999999"
        );
        let video = media.starts_with("video");
        json!({
            "id": id,
            "baseUrl": base,
            "base_url": base,
            "backupUrl": [backup],
            "backup_url": [backup],
            "bandwidth": if video { 537253 } else { 112268 },
            "mimeType": if video { "video/mp4" } else { "audio/mp4" },
            "mime_type": if video { "video/mp4" } else { "audio/mp4" },
            "codecs": codec,
            "width": if video { 960 } else { 0 },
            "height": if video { 540 } else { 0 },
            "frameRate": if video { "29.412" } else { "" },
            "frame_rate": if video { "29.412" } else { "" },
            "sar": if video { "1:1" } else { "" },
            "startWithSap": if video { 1 } else { 0 },
            "start_with_sap": if video { 1 } else { 0 },
            "SegmentBase": {
                "Initialization": "0-994",
                "indexRange": "995-2370"
            },
            "segment_base": {
                "initialization": "0-994",
                "index_range": "995-2370"
            },
            "codecid": codec_id
        })
    }

    fn playback_manifest_fixture() -> Value {
        json!({
            "code": 0,
            "message": "OK",
            "data": {
                "quality": 64,
                "format": "flv720",
                "timelength": 212000,
                "accept_format": "flv720,flv480,mp4",
                "accept_description": ["720P 高清", "480P 清晰", "360P 流畅"],
                "accept_quality": [64, 32, 16],
                "video_codecid": 7,
                "seek_param": "start",
                "seek_type": "offset",
                "dash": {
                    "duration": 212,
                    "minBufferTime": 1.5,
                    "min_buffer_time": 1.5,
                    "video": [
                        playback_track_fixture(64, "video-avc", "avc1.64001F", 7),
                        playback_track_fixture(64, "video-av1", "av01.0.08M.08", 13)
                    ],
                    "audio": [
                        playback_track_fixture(30232, "audio-aac", "mp4a.40.2", 0)
                    ],
                    "dolby": {
                        "type": 2,
                        "audio": [
                            playback_track_fixture(30250, "audio-dolby", "ec-3", 0)
                        ]
                    },
                    "flac": {
                        "display": true,
                        "audio": playback_track_fixture(30251, "audio-flac", "fLaC", 0)
                    }
                },
                "support_formats": [{
                    "quality": 64,
                    "format": "flv720",
                    "new_description": "720P 高清",
                    "display_desc": "720P",
                    "superscript": "",
                    "codecs": ["avc1.64001F", "av01.0.08M.08"]
                }, {
                    "quality": 32,
                    "format": "flv480",
                    "new_description": "480P 清晰",
                    "display_desc": "480P",
                    "superscript": "",
                    "codecs": ["avc1.64001F"]
                }, {
                    "quality": 16,
                    "format": "mp4",
                    "new_description": "360P 流畅",
                    "display_desc": "360P",
                    "superscript": "",
                    "codecs": ["avc1.64001E"]
                }],
                "cur_language": "en",
                "cur_production_type": 1,
                "language": {
                    "support": true,
                    "items": [{
                        "lang": "en",
                        "title": "English",
                        "subtitle_lang": "en-US",
                        "video_detext": true,
                        "video_mouth_shape_change": false,
                        "production_type": 1
                    }],
                    "open_toast": "已切换英语音轨",
                    "close_toast": "已切回原音",
                    "default_title": "原音"
                },
                "last_play_time": 12345,
                "last_play_cid": 106101299
            }
        })
    }

    #[test]
    fn playback_manifest_preserves_dash_codecs_audio_tiers_and_aliases() {
        let bytes =
            serde_json::to_vec(&playback_manifest_fixture()).expect("playback manifest fixture");
        let manifest = parse_playback_manifest(&bytes, 60_977_932, "BV1Jt411P77c", 106_101_299)
            .expect("playback manifest");

        assert_eq!(manifest.current_quality, 64);
        assert_eq!(manifest.duration_ms, 212_000);
        assert_eq!(manifest.accepted_qualities, vec![64, 32, 16]);
        assert_eq!(manifest.formats.len(), 3);
        assert_eq!(manifest.minimum_buffer_time, Some(1.5));
        assert_eq!(manifest.video_tracks.len(), 2);
        assert_eq!(manifest.video_tracks[0].codecs, "avc1.64001F");
        assert_eq!(manifest.video_tracks[1].codec_id, Some(13));
        assert_eq!(manifest.audio_tracks[0].id, 30_232);
        assert_eq!(manifest.dolby_audio_tracks[0].id, 30_250);
        assert_eq!(manifest.lossless_audio_tracks[0].id, 30_251);
        assert_eq!(manifest.dolby_type, Some(2));
        assert_eq!(manifest.lossless_display, Some(true));
        assert_eq!(manifest.selected_audio_language.as_deref(), Some("en"));
        assert_eq!(manifest.last_play_time_ms, Some(12_345));
        assert_eq!(manifest.last_play_cid, Some(106_101_299));
        assert_eq!(manifest.expires_at_epoch_seconds, Some(1_999_999_999));
        assert_eq!(
            manifest.video_tracks[0]
                .segment_base
                .as_ref()
                .expect("segment base")
                .index_range,
            "995-2370"
        );
    }

    #[test]
    fn playback_manifest_preserves_progressive_segments_without_dash() {
        let mut fixture = playback_manifest_fixture();
        fixture["data"]["dash"] = Value::Null;
        fixture["data"]["durl"] = json!([{
            "order": 1,
            "length": 212000,
            "size": 70486426,
            "url": "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/99/12/106101299/video.mp4?deadline=2000000000",
            "backup_url": [
                "https://upos-sz-mirrorali.bilivideo.com/upgcxcode/99/12/106101299/video.mp4?deadline=1999999999"
            ]
        }]);
        let bytes = serde_json::to_vec(&fixture).expect("progressive playback fixture");
        let manifest = parse_playback_manifest(&bytes, 60_977_932, "BV1Jt411P77c", 106_101_299)
            .expect("progressive playback manifest");

        assert!(manifest.video_tracks.is_empty());
        assert!(manifest.audio_tracks.is_empty());
        assert_eq!(manifest.progressive_segments.len(), 1);
        assert_eq!(manifest.progressive_segments[0].order, 1);
        assert_eq!(manifest.progressive_segments[0].duration_ms, 212_000);
        assert_eq!(manifest.progressive_segments[0].size, 70_486_426);
        assert_eq!(manifest.expires_at_epoch_seconds, Some(1_999_999_999));
    }

    #[test]
    fn playback_manifest_rejects_alias_identity_url_and_timing_drift() {
        for (pointer, value) in [
            (
                "/data/dash/video/0/base_url",
                json!(
                    "https://upos-sz-mirrorcos.bilivideo.com/upgcxcode/99/12/106101299/other.m4s?deadline=2000000000"
                ),
            ),
            (
                "/data/dash/video/0/baseUrl",
                json!("https://127.0.0.1/upgcxcode/99/12/106101299/video.m4s?deadline=2000000000"),
            ),
            ("/data/dash/video/0/width", json!(0)),
            ("/data/dash/video/0/mimeType", json!("audio/mp4")),
            ("/data/dash/duration", json!(1)),
            ("/data/accept_quality/1", json!(64)),
            (
                "/data/dash/audio/0/SegmentBase/Initialization",
                json!("995-0"),
            ),
            (
                "/data/dash/audio/0/backupUrl/0",
                json!("https://example.test/upgcxcode/audio.m4s?deadline=2000000000"),
            ),
        ] {
            let mut malformed = playback_manifest_fixture();
            *malformed.pointer_mut(pointer).expect("fixture field") = value;
            let bytes =
                serde_json::to_vec(&malformed).expect("malformed playback manifest fixture");
            let error = parse_playback_manifest(&bytes, 60_977_932, "BV1Jt411P77c", 106_101_299)
                .expect_err("playback manifest drift");
            assert_eq!(error.code, ErrorCode::UpstreamError, "{pointer}");
        }
    }

    #[test]
    fn subtitle_catalog_preserves_login_required_empty_state() {
        let bytes = serde_json::to_vec(&json!({
            "code": 0,
            "message": "0",
            "data": {
                "aid": 60977932,
                "bvid": "BV1Jt411P77c",
                "cid": 106101299,
                "need_login_subtitle": true,
                "subtitle": {
                    "allow_submit": false,
                    "lan": "",
                    "lan_doc": "",
                    "subtitles": []
                }
            }
        }))
        .expect("empty subtitle fixture");
        let catalog = parse_video_subtitle_catalog(&bytes, 60_977_932, "BV1Jt411P77c", 106_101_299)
            .expect("empty subtitle catalog");
        assert!(catalog.requires_login);
        assert!(catalog.subtitles.is_empty());
        assert_eq!(catalog.default_language, None);
        assert_eq!(catalog.default_language_label, None);
    }

    #[test]
    fn created_favorite_folders_preserve_typed_identity_and_visibility() {
        let response = serde_json::to_vec(&json!({
            "code": 0,
            "message": "OK",
            "data": {
                "count": 2,
                "list": [{
                    "id": 44233921,
                    "fid": 442339,
                    "mid": 7792521,
                    "attr": 0,
                    "title": "默认收藏夹",
                    "fav_state": 0,
                    "media_count": 178,
                    "is_kid_playlist": false,
                    "kid_playlist_desc": ""
                }, {
                    "id": 90210021,
                    "fid": 902100,
                    "mid": 7792521,
                    "attr": 3,
                    "title": "私有音乐",
                    "fav_state": 1,
                    "media_count": 12,
                    "is_kid_playlist": true,
                    "kid_playlist_desc": "适合青少年"
                }],
                "season": null
            }
        }))
        .expect("favorite folders fixture");
        let folders =
            parse_created_favorite_folders_response(&response, 7_792_521).expect("folders");
        assert_eq!(folders.owner_id, 7_792_521);
        assert_eq!(folders.folders.len(), 2);
        assert_eq!(folders.folders[0].media_id, 44_233_921);
        assert_eq!(folders.folders[0].folder_id, 442_339);
        assert!(!folders.folders[0].favorite_state);
        assert_eq!(folders.folders[1].attributes, 3);
        assert!(folders.folders[1].favorite_state);
        assert!(folders.folders[1].child_friendly);
    }

    #[test]
    fn created_favorite_folders_distinguish_privacy_from_empty_and_reject_drift() {
        let hidden = parse_created_favorite_folders_response(
            br#"{"code":0,"message":"OK","data":null}"#,
            7_792_521,
        )
        .expect_err("hidden folders");
        assert_eq!(hidden.code, ErrorCode::PermissionDenied);
        assert_eq!(hidden.details["hidden"], true);

        let empty = parse_created_favorite_folders_response(
            br#"{"code":0,"message":"OK","data":{"count":0,"list":null,"season":null}}"#,
            7_792_521,
        )
        .expect("empty visible folders");
        assert!(empty.folders.is_empty());

        for malformed in [
            br#"{"code":0,"message":"OK","data":{"count":2,"list":[],"season":null}}"#.as_slice(),
            br#"{"code":0,"message":"OK","data":{"count":1,"list":[{"id":1,"fid":1,"mid":9,"attr":0,"title":"x","fav_state":0,"media_count":0}],"season":null}}"#
                .as_slice(),
            br#"{"code":0,"message":"OK","data":{"count":1,"list":[{"id":1,"fid":1,"mid":7792521,"attr":0,"title":"x","fav_state":2,"media_count":0}],"season":null}}"#
                .as_slice(),
        ] {
            let error = parse_created_favorite_folders_response(malformed, 7_792_521)
                .expect_err("malformed favorite folders");
            assert_eq!(error.code, ErrorCode::UpstreamError);
        }
    }

    #[test]
    fn favorite_folder_detail_preserves_owner_privacy_and_counts() {
        let response = serde_json::to_vec(&json!({
            "code": 0,
            "message": "OK",
            "data": {
                "id": 2883236382_u64,
                "fid": 28832363,
                "mid": 47275982,
                "attr": 22,
                "title": "相声",
                "cover": "http://i2.hdslb.com/bfs/archive/folder.jpg",
                "upper": {
                    "mid": 47275982,
                    "name": "荷花-Lotus",
                    "face": "//i2.hdslb.com/bfs/face/avatar.jpg",
                    "followed": false,
                    "vip_type": 1,
                    "vip_statue": 0
                },
                "cover_type": 2,
                "cnt_info": {
                    "collect": 3,
                    "play": 2059,
                    "thumb_up": 7,
                    "share": 11
                },
                "type": 11,
                "intro": "公开收藏夹\n音视频内容",
                "ctime": 1705401630,
                "mtime": 1705925782,
                "state": 0,
                "fav_state": 1,
                "like_state": 0,
                "media_count": 99,
                "is_top": true,
                "is_kid_playlist": false,
                "kid_playlist_desc": ""
            }
        }))
        .expect("favorite folder fixture");
        let folder =
            parse_favorite_folder_response(&response, 2_883_236_382).expect("favorite folder");
        assert_eq!(folder.folder_id, 28_832_363);
        assert_eq!(folder.owner.id, 47_275_982);
        assert_eq!(folder.owner.name, "荷花-Lotus");
        assert_eq!(
            folder.cover_url.as_deref(),
            Some("https://i2.hdslb.com/bfs/archive/folder.jpg")
        );
        assert_eq!(folder.description, "公开收藏夹\n音视频内容");
        assert!(!folder.invalid);
        assert!(folder.favorite_state);
        assert!(!folder.like_state);
        assert!(folder.pinned);
        assert_eq!(folder.counts.play, 2_059);
        assert_eq!(folder.media_count, 99);
    }

    #[test]
    fn favorite_folder_detail_rejects_identity_and_state_drift() {
        let base = json!({
            "code": 0,
            "message": "OK",
            "data": {
                "id": 1052622027,
                "fid": 10526220,
                "mid": 686127,
                "attr": 54,
                "title": "收藏夹",
                "cover": "",
                "upper": {
                    "mid": 686127,
                    "name": "创建者",
                    "face": "",
                    "followed": false,
                    "vip_type": 2,
                    "vip_statue": 1
                },
                "cover_type": 2,
                "cnt_info": {},
                "type": 11,
                "intro": "",
                "ctime": 0,
                "mtime": 0,
                "state": 0,
                "fav_state": 0,
                "like_state": 0,
                "media_count": 1
            }
        });
        for (pointer, value) in [
            ("/data/id", json!(1052622028)),
            ("/data/upper/mid", json!(686128)),
            ("/data/type", json!(21)),
            ("/data/like_state", json!(2)),
        ] {
            let mut malformed = base.clone();
            *malformed.pointer_mut(pointer).expect("fixture field") = value;
            let bytes = serde_json::to_vec(&malformed).expect("malformed fixture");
            let error = parse_favorite_folder_response(&bytes, 1_052_622_027)
                .expect_err("favorite folder drift");
            assert_eq!(error.code, ErrorCode::UpstreamError);
        }
        let missing = parse_favorite_folder_response(
            r#"{"code":11010,"message":"内容不存在","data":null}"#.as_bytes(),
            1,
        )
        .expect_err("missing favorite folder");
        assert_eq!(missing.code, ErrorCode::ResourceNotFound);
    }

    #[test]
    fn collected_playlists_keep_folders_seasons_and_invalid_entries_distinct() {
        let response = serde_json::to_vec(&json!({
            "code": 0,
            "message": "OK",
            "data": {
                "count": 3,
                "list": [{
                    "id": 1513762000_u64,
                    "fid": 15137620,
                    "mid": 3493115920454890_u64,
                    "attr": 22,
                    "attr_desc": "",
                    "title": "色差即坏苹果001宇宙",
                    "cover": "http://i0.hdslb.com/bfs/archive/folder.jpg",
                    "upper": {
                        "mid": 3493115920454890_u64,
                        "name": "收藏夹作者",
                        "face": "//i0.hdslb.com/bfs/face/avatar.jpg"
                    },
                    "cover_type": 2,
                    "intro": "第一行\n第二行",
                    "ctime": 1563394571,
                    "mtime": 1563394572,
                    "state": 0,
                    "fav_state": 1,
                    "media_count": 55,
                    "view_count": 10,
                    "is_top": true,
                    "type": 11,
                    "link": "",
                    "bvid": "",
                    "is_kid_playlist": false,
                    "kid_playlist_desc": ""
                }, {
                    "id": 4641954,
                    "fid": 0,
                    "mid": 1868902080,
                    "attr": 0,
                    "title": "2025哔哩哔哩拜年纪",
                    "cover": "https://archive.biliimg.com/bfs/archive/season.jpg",
                    "upper": {
                        "mid": 1868902080,
                        "name": "哔哩哔哩拜年纪",
                        "face": ""
                    },
                    "cover_type": 0,
                    "intro": "",
                    "ctime": 0,
                    "mtime": 1738078200,
                    "state": 0,
                    "fav_state": 1,
                    "media_count": 46,
                    "type": 21,
                    "link": "bilibili://video/113884295860962?is_from_ugc_season=1",
                    "bvid": "",
                    "is_kid_playlist": false,
                    "kid_playlist_desc": ""
                }, {
                    "id": 1291813,
                    "fid": 0,
                    "mid": 0,
                    "attr": 1,
                    "title": "该合集已失效",
                    "cover": "",
                    "upper": { "mid": 0, "name": "", "face": "" },
                    "cover_type": 0,
                    "intro": "",
                    "ctime": 0,
                    "mtime": 0,
                    "state": 1,
                    "fav_state": 0,
                    "media_count": 0,
                    "type": 21
                }],
                "has_more": false
            }
        }))
        .expect("collected playlists fixture");
        let page =
            parse_collected_playlists_response(&response, 1, 70).expect("collected playlists");
        assert_eq!(page.total, 3);
        assert!(!page.has_more);
        assert_eq!(
            page.playlists[0].kind,
            BilibiliCollectedPlaylistKind::FavoriteFolder
        );
        assert_eq!(page.playlists[0].folder_id, Some(15_137_620));
        assert_eq!(
            page.playlists[0].cover_url.as_deref(),
            Some("https://i0.hdslb.com/bfs/archive/folder.jpg")
        );
        assert_eq!(page.playlists[0].description, "第一行\n第二行");
        assert_eq!(
            page.playlists[1].kind,
            BilibiliCollectedPlaylistKind::Season
        );
        assert_eq!(page.playlists[1].folder_id, None);
        assert_eq!(
            page.playlists[1].cover_url.as_deref(),
            Some("https://archive.biliimg.com/bfs/archive/season.jpg")
        );
        assert!(page.playlists[2].invalid);
        assert!(page.playlists[2].owner.is_none());
    }

    #[test]
    fn collected_playlists_enforce_privacy_pagination_and_known_types() {
        let hidden =
            parse_collected_playlists_response(br#"{"code":0,"message":"OK","data":null}"#, 1, 70)
                .expect_err("hidden collected playlists");
        assert_eq!(hidden.code, ErrorCode::PermissionDenied);

        let beyond_end = parse_collected_playlists_response(
            br#"{"code":0,"message":"OK","data":{"count":1,"list":null,"has_more":false}}"#,
            2,
            70,
        )
        .expect("page beyond end");
        assert!(beyond_end.playlists.is_empty());

        for malformed in [
            br#"{"code":0,"message":"OK","data":{"count":71,"list":[],"has_more":true}}"#.as_slice(),
            br#"{"code":0,"message":"OK","data":{"count":1,"list":[{"id":1,"fid":0,"mid":1,"title":"x","upper":{"mid":1,"name":"u","face":""},"state":0,"fav_state":1,"media_count":1,"type":99}],"has_more":false}}"#
                .as_slice(),
            br#"{"code":0,"message":"OK","data":{"count":1,"list":[{"id":1,"fid":1,"mid":1,"title":"x","upper":{"mid":2,"name":"u","face":""},"state":0,"fav_state":1,"media_count":1,"type":11}],"has_more":false}}"#
                .as_slice(),
        ] {
            let error = parse_collected_playlists_response(malformed, 1, 70)
                .expect_err("malformed collected playlists");
            assert_eq!(error.code, ErrorCode::UpstreamError);
        }
    }

    #[test]
    fn space_playlists_preserve_seasons_series_previews_and_metadata() {
        let response = serde_json::to_vec(&json!({
            "code": 0,
            "message": "OK",
            "data": {
                "items_lists": {
                    "page": { "page_num": 1, "page_size": 20, "total": 2 },
                    "seasons_list": [{
                        "archives": [{
                            "aid": 343807541,
                            "bvid": "BV1t94y1D79E"
                        }],
                        "meta": {
                            "category": 0,
                            "cover": "https://archive.biliimg.com/bfs/archive/season.jpg",
                            "description": "第一行\n第二行",
                            "mid": 37737161,
                            "name": "合集·拾枝杂谈",
                            "ptime": 1694682652,
                            "season_id": 587216,
                            "total": 10,
                            "title": "拾枝杂谈"
                        },
                        "recent_aids": [343807541]
                    }],
                    "series_list": [{
                        "archives": [{
                            "aid": 284063097,
                            "bvid": "BV1Fc411x7xF"
                        }],
                        "meta": {
                            "category": 1,
                            "cover": "http://i0.hdslb.com/bfs/archive/series.jpg",
                            "creator": "auto",
                            "ctime": 1705401630,
                            "description": "Kotlin 学习路线",
                            "keywords": ["", "Kotlin"],
                            "last_update_ts": 1705925782,
                            "mid": 37737161,
                            "mtime": 1705925781,
                            "name": "Kotlin开心路线",
                            "raw_keywords": "Kotlin,构建",
                            "series_id": 3908327,
                            "state": 2,
                            "total": 3
                        },
                        "recent_aids": [284063097]
                    }]
                }
            }
        }))
        .expect("space playlists fixture");
        let page =
            parse_space_playlists_response(&response, 37_737_161, 1, 20).expect("space playlists");
        assert_eq!(page.total, 2);
        assert!(!page.has_more);
        assert_eq!(page.playlists[0].kind, BilibiliSpacePlaylistKind::Season);
        assert_eq!(page.playlists[0].id, 587_216);
        assert_eq!(page.playlists[0].display_title.as_deref(), Some("拾枝杂谈"));
        assert_eq!(page.playlists[0].recent_aids, [343_807_541]);
        assert_eq!(page.playlists[1].kind, BilibiliSpacePlaylistKind::Series);
        assert_eq!(
            page.playlists[1].cover_url.as_deref(),
            Some("https://i0.hdslb.com/bfs/archive/series.jpg")
        );
        assert_eq!(page.playlists[1].keywords, ["Kotlin", "构建"]);
        assert_eq!(page.playlists[1].state, Some(2));
    }

    #[test]
    fn space_playlists_reject_pagination_identity_and_preview_drift() {
        for malformed in [
            br#"{"code":0,"message":"OK","data":{"items_lists":{"page":{"page_num":2,"page_size":20,"total":0},"seasons_list":[],"series_list":[]}}}"#
                .as_slice(),
            br#"{"code":0,"message":"OK","data":{"items_lists":{"page":{"page_num":1,"page_size":20,"total":1},"seasons_list":[{"archives":[],"meta":{"mid":9,"name":"x","season_id":1,"total":0},"recent_aids":[]}],"series_list":[]}}}"#
                .as_slice(),
            br#"{"code":0,"message":"OK","data":{"items_lists":{"page":{"page_num":1,"page_size":20,"total":1},"seasons_list":[],"series_list":[{"archives":[{"aid":1,"bvid":"invalid"}],"meta":{"mid":37737161,"name":"x","series_id":1,"total":1},"recent_aids":[1]}]}}}"#
                .as_slice(),
        ] {
            let error = parse_space_playlists_response(malformed, 37_737_161, 1, 20)
                .expect_err("malformed space playlists");
            assert_eq!(error.code, ErrorCode::UpstreamError);
        }
    }

    #[test]
    fn season_archives_keep_detail_metadata_and_video_identity_together() {
        let response = serde_json::to_vec(&json!({
            "code": 0,
            "message": "OK",
            "data": {
                "aids": [400546145],
                "archives": [{
                    "aid": 400546145,
                    "bvid": "BV1qo4y1L73P",
                    "ctime": 1682777426,
                    "duration": 335,
                    "interactive_video": false,
                    "pic": "http://i2.hdslb.com/bfs/archive/video.jpg",
                    "playback_position": -1,
                    "pubdate": 1682777425,
                    "stat": { "view": 52743, "danmaku": 12 },
                    "state": 0,
                    "title": "搜索引擎乱象",
                    "ugc_pay": 0
                }],
                "meta": {
                    "category": 0,
                    "cover": "https://archive.biliimg.com/bfs/archive/season.jpg",
                    "description": "白马首席讲师吐槽系列视频",
                    "mid": 37737161,
                    "name": "水浅王八多，真假白马说",
                    "ptime": 1682777425,
                    "season_id": 1227671,
                    "total": 1
                },
                "page": { "page_num": 1, "page_size": 30, "total": 1 }
            }
        }))
        .expect("season archives fixture");
        let page =
            parse_season_archives_response(&response, 1_227_671, 1, 30).expect("season archives");
        assert_eq!(page.total, 1);
        assert_eq!(page.season.owner_id, 37_737_161);
        assert_eq!(page.season.id, 1_227_671);
        assert_eq!(
            page.season.cover_url.as_deref(),
            Some("https://archive.biliimg.com/bfs/archive/season.jpg")
        );
        assert_eq!(page.archives[0].aid, 400_546_145);
        assert_eq!(page.archives[0].bvid, "BV1qo4y1L73P");
        assert_eq!(page.archives[0].playback_position, Some(-1));
        assert_eq!(page.archives[0].danmaku_count, Some(12));
        assert!(!page.archives[0].paid);
    }

    #[test]
    fn season_archives_reject_mismatched_ids_pages_and_state() {
        for malformed in [
            br#"{"code":0,"message":"OK","data":{"aids":[2],"archives":[{"aid":1,"bvid":"BV1qo4y1L73P","duration":1,"pic":"https://i0.hdslb.com/bfs/archive/x.jpg","title":"x","ugc_pay":0}],"meta":{"mid":1,"name":"x","season_id":9,"total":1},"page":{"page_num":1,"page_size":30,"total":1}}}"#
                .as_slice(),
            br#"{"code":0,"message":"OK","data":{"aids":[],"archives":[],"meta":{"mid":1,"name":"x","season_id":9,"total":1},"page":{"page_num":1,"page_size":30,"total":1}}}"#
                .as_slice(),
            br#"{"code":0,"message":"OK","data":{"aids":[1],"archives":[{"aid":1,"bvid":"BV1qo4y1L73P","duration":1,"pic":"https://i0.hdslb.com/bfs/archive/x.jpg","playback_position":101,"title":"x","ugc_pay":0}],"meta":{"mid":1,"name":"x","season_id":9,"total":1},"page":{"page_num":1,"page_size":30,"total":1}}}"#
                .as_slice(),
        ] {
            let error = parse_season_archives_response(malformed, 9, 1, 30)
                .expect_err("malformed season archives");
            assert_eq!(error.code, ErrorCode::UpstreamError);
        }
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili Passport access"]
    async fn live_qr_creation_returns_a_trusted_scannable_url() {
        let client = BilibiliClient::new(&BilibiliConfig::default()).expect("Bilibili client");
        let start = client.create_qr_login().await.expect("live QR creation");
        validate_qrcode_key(&start.qrcode_key).expect("valid live QR key");
        validate_qr_login_url(&start.url, &start.qrcode_key).expect("trusted live QR URL");
        assert!(
            start
                .image_data_url
                .starts_with("data:image/svg+xml;base64,")
        );
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili Passport access"]
    async fn live_new_qr_poll_starts_in_the_waiting_state() {
        let client = BilibiliClient::new(&BilibiliConfig::default()).expect("Bilibili client");
        let start = client.create_qr_login().await.expect("live QR creation");
        let state = client
            .poll_qr_login(&start.qrcode_key)
            .await
            .expect("live QR polling");
        assert_eq!(state, BilibiliQrPoll::Waiting);
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili API access"]
    async fn live_anonymous_session_reports_logged_out() {
        let client = BilibiliClient::new(&BilibiliConfig::default()).expect("Bilibili client");
        let session = client
            .session_status(None)
            .await
            .expect("live anonymous session status");
        assert!(!session.authenticated);
        assert!(session.user_id.is_none());
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili search access"]
    async fn live_anonymous_video_search_uses_provider_managed_web_identity() {
        let client = BilibiliClient::new(&BilibiliConfig::default()).expect("Bilibili client");
        let page = client
            .search_videos_page("周杰伦", 1, BilibiliVideoSearchFilters::default(), None)
            .await
            .expect("live video search");
        assert_eq!(page.page, 1);
        assert_eq!(page.page_size, VIDEO_SEARCH_PAGE_SIZE);
        assert!(!page.videos.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili favorite folder access"]
    async fn live_public_created_favorite_folders_are_available_anonymously() {
        let client = BilibiliClient::new(&BilibiliConfig::default()).expect("Bilibili client");
        let folders = client
            .created_favorite_folders(7_792_521, None)
            .await
            .expect("live public favorite folders");
        assert_eq!(folders.owner_id, 7_792_521);
        assert!(!folders.folders.is_empty());
        assert!(
            folders
                .folders
                .iter()
                .all(|folder| folder.owner_id == 7_792_521)
        );
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili favorite folder access"]
    async fn live_public_favorite_folder_detail_is_available_anonymously() {
        let client = BilibiliClient::new(&BilibiliConfig::default()).expect("Bilibili client");
        let folder = client
            .favorite_folder(2_883_236_382, None)
            .await
            .expect("live public favorite folder");
        assert_eq!(folder.media_id, 2_883_236_382);
        assert_eq!(folder.folder_id, 28_832_363);
        assert_eq!(folder.owner.id, 47_275_982);
        assert_eq!(folder.media_count, 99);
        assert!(!folder.invalid);
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili collected playlist access"]
    async fn live_public_collected_playlists_include_typed_seasons_and_folders() {
        let client = BilibiliClient::new(&BilibiliConfig::default()).expect("Bilibili client");
        let page = client
            .collected_playlists_page(293_793_435, 1, None)
            .await
            .expect("live public collected playlists");
        assert_eq!(page.page, 1);
        assert_eq!(page.page_size, COLLECTED_PLAYLIST_PAGE_SIZE);
        assert!(!page.playlists.is_empty());
        assert!(
            page.playlists
                .iter()
                .any(|playlist| playlist.kind == BilibiliCollectedPlaylistKind::Season)
        );
        assert!(
            page.playlists
                .iter()
                .any(|playlist| playlist.kind == BilibiliCollectedPlaylistKind::FavoriteFolder)
        );
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili space playlist access"]
    async fn live_space_playlists_use_wbi_and_preserve_seasons_and_series() {
        let client = BilibiliClient::new(&BilibiliConfig::default()).expect("Bilibili client");
        let page = client
            .space_playlists_page(37_737_161, 1, None)
            .await
            .expect("live space playlists");
        assert_eq!(page.page, 1);
        assert_eq!(page.page_size, SPACE_PLAYLIST_PAGE_SIZE);
        assert!(!page.playlists.is_empty());
        assert!(
            page.playlists
                .iter()
                .any(|playlist| playlist.kind == BilibiliSpacePlaylistKind::Season)
        );
        assert!(
            page.playlists
                .iter()
                .any(|playlist| playlist.kind == BilibiliSpacePlaylistKind::Series)
        );
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili season access"]
    async fn live_season_detail_resolves_owner_with_zero_mid() {
        let client = BilibiliClient::new(&BilibiliConfig::default()).expect("Bilibili client");
        let page = client
            .season_archives_page(3_629_748, 1, None)
            .await
            .expect("live season detail");
        assert_eq!(page.season.id, 3_629_748);
        assert_eq!(page.season.owner_id, 327_961_371);
        assert_eq!(page.total, 617);
        assert_eq!(page.archives.len(), SEASON_ARCHIVE_PAGE_SIZE as usize);
        assert!(page.has_more);
    }
}
