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
const VIDEO_SEARCH_ENDPOINT: &str = "https://api.bilibili.com/x/web-interface/wbi/search/type";
const VIDEO_SEARCH_COMPATIBILITY_ENDPOINT: &str =
    "https://api.bilibili.com/x/web-interface/search/type";
const CREATED_FAVORITE_FOLDERS_ENDPOINT: &str =
    "https://api.bilibili.com/x/v3/fav/folder/created/list-all";
const COLLECTED_PLAYLISTS_ENDPOINT: &str =
    "https://api.bilibili.com/x/v3/fav/folder/collected/list";
const WEB_REFERER: &str = "https://www.bilibili.com/";
const VIDEO_SEARCH_REFERER: &str = "https://search.bilibili.com/";
const VIDEO_SEARCH_WEB_LOCATION: &str = "1430654";
const FAVORITE_FOLDER_WEB_LOCATION: &str = "333.1387";
const COLLECTED_PLAYLIST_PAGE_SIZE: u32 = 70;
const VIDEO_SEARCH_PAGE_SIZE: u32 = 20;
const WEB_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
const MAX_PASSPORT_RESPONSE_BYTES: usize = 1024 * 1024;
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
        -403 => ErrorCode::PermissionDenied,
        -404 | 62002 => ErrorCode::ResourceNotFound,
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
}
