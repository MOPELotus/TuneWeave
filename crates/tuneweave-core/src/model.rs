use std::{collections::BTreeMap, fmt};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Capability, Platform, ResourceRef};

pub type Extensions = BTreeMap<String, Value>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchKind {
    #[default]
    Track,
    Album,
    Artist,
    Playlist,
    User,
    Mv,
    Lyric,
    RadioStation,
    Podcast,
    Video,
    Mixed,
    Voice,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchVariant {
    #[default]
    Default,
    Legacy,
    Cloud,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    #[default]
    Auto,
    Low,
    Standard,
    Higher,
    High,
    Lossless,
    Hires,
    Surround,
    Spatial,
    Dolby,
    Master,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub query: String,
    pub kind: SearchKind,
    #[serde(default)]
    pub variant: SearchVariant,
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
}

impl SearchQuery {
    pub fn tracks(query: impl Into<String>, limit: u32, offset: u32) -> Self {
        Self {
            query: query.into(),
            kind: SearchKind::Track,
            variant: SearchVariant::Default,
            limit,
            offset,
            account: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchDefaultKeywordRequest {
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchDefaultKeyword {
    pub keyword: String,
    pub display_text: String,
    pub kind: Option<SearchKind>,
    pub image_url: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchTrendingDetail {
    Brief,
    #[default]
    Full,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchTrendingRequest {
    pub detail: SearchTrendingDetail,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchTrendingEntry {
    pub rank: u32,
    pub keyword: String,
    pub description: Option<String>,
    pub score: Option<u64>,
    pub icon_type: Option<i64>,
    pub icon_url: Option<String>,
    pub target_url: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchTrendingList {
    pub detail: SearchTrendingDetail,
    pub entries: Vec<SearchTrendingEntry>,
    pub extensions: Extensions,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchSuggestionClient {
    #[default]
    Web,
    Mobile,
    Pc,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchSuggestionRequest {
    pub query: String,
    pub client: SearchSuggestionClient,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchSuggestion {
    pub keyword: String,
    pub kind: Option<SearchKind>,
    pub display_text: Option<String>,
    pub icon_url: Option<String>,
    pub resource: Option<SearchItem>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchSuggestionList {
    pub query: String,
    pub client: SearchSuggestionClient,
    pub suggestions: Vec<SearchSuggestion>,
    pub recommendations: Vec<SearchSuggestion>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchMultiMatchRequest {
    pub query: String,
    pub kind: SearchKind,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchMultiMatchSection {
    pub section: String,
    pub kind: Option<SearchKind>,
    pub items: Vec<SearchItem>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchMultiMatch {
    pub query: String,
    pub requested_kind: SearchKind,
    pub sections: Vec<SearchMultiMatchSection>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LocalTrackMatchRequest {
    pub title: String,
    pub album: String,
    pub artist: String,
    pub duration_ms: u64,
    pub md5: String,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LocalTrackMatchResult {
    pub md5: String,
    pub matches: Vec<Track>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MembershipSummary {
    pub user_ref: Option<ResourceRef>,
    pub level: Option<u32>,
    pub active: Option<bool>,
    pub annual_count: Option<i64>,
    pub expires_at: Option<String>,
    pub icon_url: Option<String>,
    pub extensions: Extensions,
}

/// A provider-managed anonymous session used for public requests.
///
/// The cookie is returned for compatibility with platform registration APIs, but providers own
/// its lifecycle and callers cannot inject it into subsequent unified requests.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AnonymousSession {
    pub device_id: String,
    pub cookie: String,
    pub registered: bool,
    pub refreshed: bool,
    pub extensions: Extensions,
}

impl fmt::Debug for AnonymousSession {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AnonymousSession")
            .field("device_id", &self.device_id)
            .field("cookie", &"[REDACTED]")
            .field("registered", &self.registered)
            .field("refreshed", &self.refreshed)
            .field("extensions", &self.extensions)
            .finish()
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AntiCheatTokenVersion {
    V2,
    #[default]
    V3,
}

#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct AntiCheatToken {
    pub version: AntiCheatTokenVersion,
    pub token: String,
    pub registered: bool,
    pub refreshed: bool,
    pub extensions: Extensions,
}

impl fmt::Debug for AntiCheatToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AntiCheatToken")
            .field("version", &self.version)
            .field("token", &"[REDACTED]")
            .field("registered", &self.registered)
            .field("refreshed", &self.refreshed)
            .field("extensions", &self.extensions)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ListeningRightsAdRequest {
    pub type_ids: Vec<String>,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListeningRightsAd {
    pub id: String,
    pub request_uid: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListeningRightsAdCatalog {
    pub request_uid: Option<String>,
    pub ads: Vec<ListeningRightsAd>,
    pub message: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ListeningRightsTimestamp {
    Milliseconds(u64),
    Reference(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListeningRightsGainRequest {
    pub request_uid: Option<String>,
    pub creative_type: i64,
    pub exposure_time: Option<ListeningRightsTimestamp>,
    pub click_time: Option<ListeningRightsTimestamp>,
    pub rights_gain_method: i64,
    pub rights_gain_duration: Option<i64>,
    pub extra_rights_gain_method: Option<i64>,
    pub extra_rights_gain_duration: Option<i64>,
    pub next_rights_gain_duration: Option<i64>,
    pub source: Option<String>,
    pub rights_ext_json: Option<String>,
    pub app_info: Option<Value>,
    pub installed: Option<i64>,
    pub type_ids: Vec<String>,
    pub account: Option<String>,
}

impl Default for ListeningRightsGainRequest {
    fn default() -> Self {
        Self {
            request_uid: None,
            creative_type: 2,
            exposure_time: None,
            click_time: None,
            rights_gain_method: 2,
            rights_gain_duration: None,
            extra_rights_gain_method: None,
            extra_rights_gain_duration: None,
            next_rights_gain_duration: None,
            source: None,
            rights_ext_json: None,
            app_info: None,
            installed: None,
            type_ids: vec!["400002_0".to_owned()],
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ListeningRightsGainResult {
    pub request_uid: Option<String>,
    pub granted: Option<bool>,
    pub platform_code: Option<i64>,
    pub message: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "data", rename_all = "snake_case")]
pub enum SearchItem {
    Track(Track),
    Album(Album),
    Artist(Artist),
    Playlist(Playlist),
    User(User),
    Video(Video),
    RadioStation(RadioStation),
    Podcast(Podcast),
    Opaque(SearchOpaqueItem),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SearchOpaqueItem {
    pub platform: Platform,
    pub kind: String,
    pub id: Option<String>,
    pub title: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AudioRecognitionRequest {
    pub fingerprint: String,
    pub duration_seconds: u32,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioRecognitionMatch {
    pub track: Track,
    pub start_time_ms: Option<u64>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AudioRecognition {
    pub matches: Vec<AudioRecognitionMatch>,
    pub query_id: Option<String>,
    pub no_match_reason: Option<i64>,
    pub extensions: Extensions,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BannerClient {
    #[default]
    Pc,
    Android,
    Iphone,
    Ipad,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BannerCatalog {
    #[default]
    Music,
    Podcast,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BannerListRequest {
    pub catalog: BannerCatalog,
    pub client: BannerClient,
    pub account: Option<String>,
}

impl BannerListRequest {
    #[must_use]
    pub fn new(client: BannerClient) -> Self {
        Self {
            catalog: BannerCatalog::Music,
            client,
            account: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BannerTargetKind {
    Track,
    Album,
    Artist,
    Playlist,
    Video,
    PodcastEpisode,
    Web,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Banner {
    pub id: Option<String>,
    pub title: Option<String>,
    pub image_url: String,
    pub target_ref: Option<ResourceRef>,
    pub target_kind: BannerTargetKind,
    pub url: Option<String>,
    pub exclusive: Option<bool>,
    pub extensions: Extensions,
}

#[derive(Clone, Eq, PartialEq)]
pub struct ImageUploadRequest {
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
    pub image_size: Option<u32>,
    pub crop_x: Option<u32>,
    pub crop_y: Option<u32>,
    pub account: Option<String>,
}

impl fmt::Debug for ImageUploadRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ImageUploadRequest")
            .field("filename", &self.filename)
            .field("content_type", &self.content_type)
            .field("data_len", &self.data.len())
            .field("image_size", &self.image_size)
            .field("crop_x", &self.crop_x)
            .field("crop_y", &self.crop_y)
            .field("account", &self.account)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageUploadResult {
    pub url: Option<String>,
    pub image_id: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Eq, PartialEq)]
pub struct CloudUploadRequest {
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
    pub bitrate: u64,
    pub song_name: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub account: Option<String>,
}

impl CloudUploadRequest {
    pub const DEFAULT_BITRATE: u64 = 999_000;
}

impl fmt::Debug for CloudUploadRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CloudUploadRequest")
            .field("filename", &self.filename)
            .field("content_type", &self.content_type)
            .field("data_len", &self.data.len())
            .field("bitrate", &self.bitrate)
            .field("song_name", &self.song_name)
            .field("artist", &self.artist)
            .field("album", &self.album)
            .field("account", &self.account)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CloudUploadTicketRequest {
    pub md5: String,
    pub file_size: u64,
    pub filename: String,
    pub bitrate: u64,
    pub content_type: Option<String>,
    pub account: Option<String>,
}

impl CloudUploadTicketRequest {
    #[must_use]
    pub fn new(md5: impl Into<String>, file_size: u64, filename: impl Into<String>) -> Self {
        Self {
            md5: md5.into(),
            file_size,
            filename: filename.into(),
            bitrate: CloudUploadRequest::DEFAULT_BITRATE,
            content_type: None,
            account: None,
        }
    }
}

#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct CloudUploadTicket {
    pub upload_required: bool,
    pub provisional_track_id: Option<String>,
    pub resource_id: String,
    pub upload_method: String,
    pub upload_url: String,
    pub upload_headers: BTreeMap<String, String>,
    pub extensions: Extensions,
}

impl fmt::Debug for CloudUploadTicket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CloudUploadTicket")
            .field("upload_required", &self.upload_required)
            .field("provisional_track_id", &self.provisional_track_id)
            .field("resource_id", &self.resource_id)
            .field("upload_method", &self.upload_method)
            .field("upload_url", &self.upload_url)
            .field(
                "upload_header_names",
                &self.upload_headers.keys().collect::<Vec<_>>(),
            )
            .field("extensions", &self.extensions)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CloudUploadCompleteRequest {
    pub provisional_track_id: String,
    pub resource_id: String,
    pub md5: String,
    pub filename: String,
    pub song_name: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub bitrate: u64,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CloudUploadResult {
    pub track_ref: Option<ResourceRef>,
    pub upload_required: Option<bool>,
    pub uploaded: Option<bool>,
    pub published: bool,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CloudImportRequest {
    pub md5: String,
    pub source_track_id: Option<String>,
    pub bitrate: u64,
    pub file_size: u64,
    pub file_type: String,
    pub song_name: String,
    pub artist: String,
    pub album: String,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CloudImportResult {
    pub track_ref: Option<ResourceRef>,
    pub imported: bool,
    pub already_present: Option<bool>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CloudLyricsRequest {
    pub user_id: String,
    pub track_id: String,
    pub account: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CloudMatchRequest {
    pub user_id: String,
    pub cloud_track_id: String,
    pub target_track_id: Option<String>,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CloudMatchResult {
    pub cloud_track_ref: ResourceRef,
    pub target_track_ref: Option<ResourceRef>,
    pub matched: bool,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CloudTrack {
    #[serde(rename = "ref")]
    pub cloud_track_ref: ResourceRef,
    pub track: Track,
    pub filename: Option<String>,
    pub file_size: Option<u64>,
    pub file_type: Option<String>,
    pub bitrate: Option<u64>,
    pub md5: Option<String>,
    pub added_at: Option<String>,
    pub matched_track_ref: Option<ResourceRef>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CloudTrackDetailRequest {
    pub track_refs: Vec<ResourceRef>,
    pub account: Option<String>,
}

impl CloudTrackDetailRequest {
    #[must_use]
    pub fn new(track_refs: Vec<ResourceRef>) -> Self {
        Self {
            track_refs,
            account: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CloudTrackDeleteRequest {
    pub track_refs: Vec<ResourceRef>,
    pub account: Option<String>,
}

impl CloudTrackDeleteRequest {
    #[must_use]
    pub fn new(track_refs: Vec<ResourceRef>) -> Self {
        Self {
            track_refs,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CloudTrackDeleteResult {
    pub track_refs: Vec<ResourceRef>,
    pub deleted: bool,
    pub extensions: Extensions,
}

/// A provider-specific API request exposed below a platform extension route.
///
/// `protocol` is intentionally opaque to the core crate. Each provider owns
/// the accepted values and must constrain the upstream destination itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlatformApiRequest {
    pub uri: String,
    pub data: Value,
    pub protocol: Option<String>,
    pub account: Option<String>,
}

impl PlatformApiRequest {
    #[must_use]
    pub fn new(uri: impl Into<String>, data: Value) -> Self {
        Self {
            uri: uri.into(),
            data,
            protocol: None,
            account: None,
        }
    }
}

/// A provider-specific collection of API calls executed as one upstream batch.
///
/// Request keys and values remain provider-owned. Providers must validate every
/// target and keep transport credentials outside this payload.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlatformBatchRequest {
    pub requests: BTreeMap<String, Value>,
    pub protocol: Option<String>,
    pub encrypted_response: bool,
    pub account: Option<String>,
}

impl PlatformBatchRequest {
    #[must_use]
    pub fn new(requests: BTreeMap<String, Value>) -> Self {
        Self {
            requests,
            protocol: None,
            encrypted_response: false,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadioCatalogOption {
    pub id: String,
    pub name: String,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadioTaxonomy {
    pub categories: Vec<RadioCatalogOption>,
    pub regions: Vec<RadioCatalogOption>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RadioTaxonomyRequest {
    pub account: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RadioStyleCatalogRequest {
    pub sources: Vec<u32>,
    pub account: Option<String>,
}

impl Default for RadioStyleCatalogRequest {
    fn default() -> Self {
        Self {
            sources: vec![0],
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadioStyle {
    pub id: String,
    pub name: String,
    pub localized_name: Option<String>,
    pub description: String,
    pub channels: Vec<RadioStation>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadioStyleSource {
    pub id: u32,
    pub styles: Vec<RadioStyle>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadioStyleCatalog {
    pub sources: Vec<RadioStyleSource>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RadioPlaybackQueueRequest {
    pub limit: u32,
    pub account: Option<String>,
}

impl RadioPlaybackQueueRequest {
    #[must_use]
    pub const fn new(limit: u32) -> Self {
        Self {
            limit,
            account: None,
        }
    }
}

impl Default for RadioPlaybackQueueRequest {
    fn default() -> Self {
        Self::new(5)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadioPlaybackItem {
    #[serde(rename = "ref")]
    pub resource_ref: ResourceRef,
    pub platform: Platform,
    pub id: String,
    pub station_ref: ResourceRef,
    pub title: String,
    pub artist: Option<String>,
    pub cover_url: Option<String>,
    pub blur_cover_url: Option<String>,
    pub stream_url: Option<String>,
    pub duration_ms: Option<u64>,
    pub waveform: Vec<f64>,
    pub extensions: Extensions,
}

impl RadioPlaybackItem {
    #[must_use]
    pub fn new(
        resource_ref: ResourceRef,
        station_ref: ResourceRef,
        title: impl Into<String>,
    ) -> Self {
        Self {
            platform: resource_ref.platform(),
            id: resource_ref.id().to_owned(),
            resource_ref,
            station_ref,
            title: title.into(),
            artist: None,
            cover_url: None,
            blur_cover_url: None,
            stream_url: None,
            duration_ms: None,
            waveform: Vec::new(),
            extensions: Extensions::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadioPlaybackQueue {
    pub station_ref: ResourceRef,
    pub items: Vec<RadioPlaybackItem>,
    pub total: Option<u64>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StyledRadioStationLibraryRequest {
    pub sources: Vec<u32>,
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
}

impl StyledRadioStationLibraryRequest {
    #[must_use]
    pub fn new(sources: Vec<u32>, limit: u32) -> Self {
        Self {
            sources,
            limit,
            offset: 0,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RadioStation {
    #[serde(rename = "ref")]
    pub resource_ref: ResourceRef,
    pub platform: Platform,
    pub id: String,
    pub name: String,
    pub description: String,
    pub cover_url: Option<String>,
    pub category: Option<String>,
    pub region: Option<String>,
    pub stream_url: Option<String>,
    pub current_program: Option<String>,
    pub subscribed: Option<bool>,
    pub extensions: Extensions,
}

impl RadioStation {
    #[must_use]
    pub fn new(resource_ref: ResourceRef, name: impl Into<String>) -> Self {
        Self {
            platform: resource_ref.platform(),
            id: resource_ref.id().to_owned(),
            resource_ref,
            name: name.into(),
            description: String::new(),
            cover_url: None,
            category: None,
            region: None,
            stream_url: None,
            current_program: None,
            subscribed: None,
            extensions: Extensions::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RadioStationCursor {
    pub id: String,
    pub score: i64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RadioStationListRequest {
    pub limit: u32,
    pub offset: u32,
    pub category_id: Option<String>,
    pub region_id: Option<String>,
    pub cursor: Option<RadioStationCursor>,
    pub account: Option<String>,
}

impl RadioStationListRequest {
    #[must_use]
    pub fn new(limit: u32) -> Self {
        Self {
            limit,
            offset: 0,
            category_id: None,
            region_id: None,
            cursor: None,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PodcastCategory {
    pub id: String,
    pub name: String,
    pub icon_url: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PodcastTaxonomy {
    pub categories: Vec<PodcastCategory>,
    pub extensions: Extensions,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PodcastTaxonomyKind {
    #[default]
    All,
    NonHot,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PodcastTaxonomyRequest {
    pub kind: PodcastTaxonomyKind,
    pub account: Option<String>,
}

impl PodcastTaxonomyRequest {
    #[must_use]
    pub const fn new(kind: PodcastTaxonomyKind) -> Self {
        Self {
            kind,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PodcastCategoryRecommendation {
    pub category: PodcastCategory,
    pub podcasts: Vec<Podcast>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PodcastCategoryRecommendations {
    pub sections: Vec<PodcastCategoryRecommendation>,
    pub extensions: Extensions,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PodcastCatalog {
    #[default]
    Featured,
    Hot,
    CategoryFeatured,
    CategoryHot,
    Personalized,
    TodayPreferred,
    Paid,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PodcastListRequest {
    pub catalog: PodcastCatalog,
    pub category_id: Option<String>,
    pub limit: u32,
    pub offset: u32,
    pub page: Option<u32>,
    pub account: Option<String>,
}

impl PodcastListRequest {
    #[must_use]
    pub const fn new(catalog: PodcastCatalog, limit: u32, offset: u32) -> Self {
        Self {
            catalog,
            category_id: None,
            limit,
            offset,
            page: None,
            account: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PodcastChartKind {
    #[default]
    New,
    Hot,
    Paid,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PodcastChartRequest {
    pub kind: PodcastChartKind,
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
}

impl PodcastChartRequest {
    #[must_use]
    pub const fn new(kind: PodcastChartKind, limit: u32, offset: u32) -> Self {
        Self {
            kind,
            limit,
            offset,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PodcastChartEntry {
    pub rank: u32,
    pub previous_rank: Option<i64>,
    pub score: Option<u64>,
    pub podcast: Podcast,
    pub extensions: Extensions,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PodcastCreatorChartKind {
    #[default]
    Newcomer,
    Popular,
    Trending24Hours,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PodcastCreatorChartRequest {
    pub kind: PodcastCreatorChartKind,
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
}

impl PodcastCreatorChartRequest {
    #[must_use]
    pub const fn new(kind: PodcastCreatorChartKind, limit: u32, offset: u32) -> Self {
        Self {
            kind,
            limit,
            offset,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PodcastCreatorChartEntry {
    pub rank: u32,
    pub previous_rank: Option<i64>,
    pub score: Option<u64>,
    pub follower_count: Option<u64>,
    pub creator: User,
    pub extensions: Extensions,
}

/// An on-demand spoken-audio show. This is intentionally separate from a live radio station.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Podcast {
    #[serde(rename = "ref")]
    pub resource_ref: ResourceRef,
    pub platform: Platform,
    pub id: String,
    pub name: String,
    pub description: String,
    pub cover_url: Option<String>,
    pub creator: Option<CreatorSummary>,
    pub category: Option<String>,
    pub secondary_category: Option<String>,
    pub episode_count: Option<u64>,
    pub subscriber_count: Option<u64>,
    pub play_count: Option<u64>,
    pub subscribed: Option<bool>,
    pub paid: Option<bool>,
    pub purchased: Option<bool>,
    pub price: Option<Money>,
    pub created_at: Option<String>,
    pub extensions: Extensions,
}

impl Podcast {
    #[must_use]
    pub fn new(resource_ref: ResourceRef, name: impl Into<String>) -> Self {
        Self {
            platform: resource_ref.platform(),
            id: resource_ref.id().to_owned(),
            resource_ref,
            name: name.into(),
            description: String::new(),
            cover_url: None,
            creator: None,
            category: None,
            secondary_category: None,
            episode_count: None,
            subscriber_count: None,
            play_count: None,
            subscribed: None,
            paid: None,
            purchased: None,
            price: None,
            created_at: None,
            extensions: Extensions::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PodcastEpisode {
    #[serde(rename = "ref")]
    pub resource_ref: ResourceRef,
    pub platform: Platform,
    pub id: String,
    pub podcast_ref: Option<ResourceRef>,
    pub name: String,
    pub description: String,
    pub cover_url: Option<String>,
    pub creator: Option<CreatorSummary>,
    pub audio: Option<Track>,
    pub duration_ms: Option<u64>,
    pub published_at: Option<String>,
    pub serial_number: Option<u64>,
    pub listener_count: Option<u64>,
    pub liked_count: Option<u64>,
    pub comment_count: Option<u64>,
    pub share_count: Option<u64>,
    pub subscribed: Option<bool>,
    pub has_lyrics: Option<bool>,
    pub paid: Option<bool>,
    pub purchased: Option<bool>,
    pub extensions: Extensions,
}

impl PodcastEpisode {
    #[must_use]
    pub fn new(resource_ref: ResourceRef, name: impl Into<String>) -> Self {
        Self {
            platform: resource_ref.platform(),
            id: resource_ref.id().to_owned(),
            resource_ref,
            podcast_ref: None,
            name: name.into(),
            description: String::new(),
            cover_url: None,
            creator: None,
            audio: None,
            duration_ms: None,
            published_at: None,
            serial_number: None,
            listener_count: None,
            liked_count: None,
            comment_count: None,
            share_count: None,
            subscribed: None,
            has_lyrics: None,
            paid: None,
            purchased: None,
            extensions: Extensions::new(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PodcastEpisodeListRequest {
    pub limit: u32,
    pub offset: u32,
    pub ascending: bool,
    pub account: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PodcastEpisodeOrderRequest {
    pub episode_ref: ResourceRef,
    pub position: u32,
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PodcastEpisodeOrderResult {
    pub podcast_ref: ResourceRef,
    pub episode_ref: ResourceRef,
    pub position: u32,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PodcastEpisodeDeleteRequest {
    pub episode_refs: Vec<ResourceRef>,
    pub account: Option<String>,
}

impl PodcastEpisodeDeleteRequest {
    #[must_use]
    pub fn new(episode_refs: Vec<ResourceRef>) -> Self {
        Self {
            episode_refs,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PodcastEpisodeDeleteResult {
    pub episode_refs: Vec<ResourceRef>,
    pub deleted: bool,
    pub extensions: Extensions,
}

#[derive(Clone, Eq, PartialEq)]
pub struct PodcastEpisodeUploadRequest {
    pub filename: String,
    pub content_type: String,
    pub data: Vec<u8>,
    pub name: Option<String>,
    pub cover_image_id: String,
    pub category_id: String,
    pub second_category_id: String,
    pub description: String,
    pub privacy: bool,
    pub publish_time_ms: u64,
    pub auto_publish: bool,
    pub auto_publish_text: String,
    pub order_no: u32,
    pub composed_track_refs: Vec<ResourceRef>,
    pub account: Option<String>,
}

impl fmt::Debug for PodcastEpisodeUploadRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PodcastEpisodeUploadRequest")
            .field("filename", &self.filename)
            .field("content_type", &self.content_type)
            .field("data_len", &self.data.len())
            .field("name", &self.name)
            .field("cover_image_id", &self.cover_image_id)
            .field("category_id", &self.category_id)
            .field("second_category_id", &self.second_category_id)
            .field("description", &self.description)
            .field("privacy", &self.privacy)
            .field("publish_time_ms", &self.publish_time_ms)
            .field("auto_publish", &self.auto_publish)
            .field("auto_publish_text", &self.auto_publish_text)
            .field("order_no", &self.order_no)
            .field("composed_track_refs", &self.composed_track_refs)
            .field("account", &self.account)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PodcastEpisodeUploadResult {
    pub podcast_ref: ResourceRef,
    pub episode_refs: Vec<ResourceRef>,
    pub name: String,
    pub uploaded: bool,
    pub publish_time_ms: u64,
    pub extensions: Extensions,
}

impl PodcastEpisodeListRequest {
    #[must_use]
    pub const fn new(limit: u32, offset: u32) -> Self {
        Self {
            limit,
            offset,
            ascending: false,
            account: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PodcastEpisodeDisplayStatus {
    Auditing,
    OnlySelfSee,
    Online,
    SchedulePublish,
    TranscodeFailed,
    Publishing,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PodcastEpisodeVisibility {
    Public,
    Private,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PodcastEpisodeFeeFilter {
    #[default]
    All,
    Free,
    Paid,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PodcastEpisodeWorkbenchSearchRequest {
    pub query: Option<String>,
    pub display_status: Option<PodcastEpisodeDisplayStatus>,
    pub visibility: Option<PodcastEpisodeVisibility>,
    pub fee_type: Option<PodcastEpisodeFeeFilter>,
    pub podcast_id: Option<String>,
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
}

impl PodcastEpisodeWorkbenchSearchRequest {
    #[must_use]
    pub const fn new(limit: u32, offset: u32) -> Self {
        Self {
            query: None,
            display_status: None,
            visibility: None,
            fee_type: None,
            podcast_id: None,
            limit,
            offset,
            account: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PodcastEpisodeChartKind {
    #[default]
    Popular,
    Trending24Hours,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PodcastEpisodeChartRequest {
    pub kind: PodcastEpisodeChartKind,
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
}

impl PodcastEpisodeChartRequest {
    #[must_use]
    pub const fn new(kind: PodcastEpisodeChartKind, limit: u32, offset: u32) -> Self {
        Self {
            kind,
            limit,
            offset,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PodcastEpisodeChartEntry {
    pub rank: u32,
    pub previous_rank: Option<i64>,
    pub score: Option<u64>,
    pub episode: PodcastEpisode,
    pub extensions: Extensions,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PodcastEpisodeRecommendationSource {
    #[default]
    Personalized,
    Category,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PodcastEpisodeRecommendationRequest {
    #[serde(default)]
    pub source: PodcastEpisodeRecommendationSource,
    pub category_id: Option<String>,
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
}

impl PodcastEpisodeRecommendationRequest {
    #[must_use]
    pub const fn new(source: PodcastEpisodeRecommendationSource, limit: u32, offset: u32) -> Self {
        Self {
            source,
            category_id: None,
            limit,
            offset,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PodcastEpisodeStream {
    #[serde(rename = "ref")]
    pub episode_ref: ResourceRef,
    pub audio_ref: ResourceRef,
    pub stream: MediaStream,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PodcastEpisodeLyrics {
    #[serde(rename = "ref")]
    pub episode_ref: ResourceRef,
    pub audio_ref: Option<ResourceRef>,
    pub lyrics: Lyrics,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaybackDevice {
    pub operating_system: Option<String>,
    pub name: Option<String>,
    pub icon_url: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PodcastEpisodePlaybackHistoryEntry {
    pub episode: PodcastEpisode,
    pub played_at: Option<String>,
    pub device: Option<PlaybackDevice>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PageMeta {
    pub limit: u32,
    pub offset: u32,
    pub total: Option<u64>,
    pub next_offset: Option<u32>,
    pub has_more: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub pagination: PageMeta,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PageRequest {
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
}

impl PageRequest {
    #[must_use]
    pub fn new(limit: u32, offset: u32) -> Self {
        Self {
            limit,
            offset,
            account: None,
        }
    }
}

/// The media or social resource whose comment thread is being addressed.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentTargetKind {
    Track,
    Mv,
    Playlist,
    Album,
    RadioEpisode,
    Video,
    Event,
    RadioStation,
}

/// A platform-qualified comment thread target.
///
/// Event-like targets use their complete platform thread id as the resource reference id.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommentTarget {
    #[serde(rename = "ref")]
    pub resource_ref: ResourceRef,
    pub kind: CommentTargetKind,
}

impl CommentTarget {
    #[must_use]
    pub const fn new(resource_ref: ResourceRef, kind: CommentTargetKind) -> Self {
        Self { resource_ref, kind }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommentWriteRequest {
    pub target: CommentTarget,
    pub content: String,
    pub reply_to: Option<String>,
    pub account: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommentDeleteRequest {
    pub target: CommentTarget,
    pub comment_id: String,
    pub account: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentMutationAction {
    Create,
    Reply,
    Delete,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommentMutationResult {
    pub target: CommentTarget,
    pub comment_id: Option<String>,
    pub action: CommentMutationAction,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommentReportRequest {
    pub target: CommentTarget,
    pub comment_id: String,
    pub reason: String,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommentReportResult {
    pub target: CommentTarget,
    pub comment_id: String,
    pub reason: String,
    pub submitted: bool,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CountryCallingCodeListRequest {
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CountryCallingCode {
    pub calling_code: String,
    pub region_code: String,
    pub name: String,
    pub english_name: String,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CountryCallingCodeGroup {
    pub label: String,
    pub entries: Vec<CountryCallingCode>,
    pub extensions: Extensions,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentListView {
    #[default]
    All,
    Hot,
    Replies,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentSort {
    Recommended,
    Hot,
    Time,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommentListRequest {
    pub target: CommentTarget,
    pub view: CommentListView,
    pub sort: Option<CommentSort>,
    pub limit: u32,
    pub offset: u32,
    pub page: Option<u32>,
    pub cursor: Option<String>,
    pub before_time_ms: Option<u64>,
    pub parent_comment_id: Option<String>,
    pub include_replies: bool,
    pub account: Option<String>,
}

impl CommentListRequest {
    #[must_use]
    pub const fn new(target: CommentTarget, limit: u32) -> Self {
        Self {
            target,
            view: CommentListView::All,
            sort: None,
            limit,
            offset: 0,
            page: None,
            cursor: None,
            before_time_ms: None,
            parent_comment_id: None,
            include_replies: true,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommentReplyReference {
    pub comment_id: Option<String>,
    pub content: String,
    pub author: Option<User>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Comment {
    pub platform: Platform,
    pub id: String,
    pub content: String,
    pub author: Option<User>,
    pub created_at_ms: Option<u64>,
    pub created_at_text: Option<String>,
    pub liked: Option<bool>,
    pub like_count: Option<u64>,
    pub parent_comment_id: Option<String>,
    pub reply_count: Option<u64>,
    pub replied_to: Vec<CommentReplyReference>,
    pub ip_location: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommentPage {
    pub target: CommentTarget,
    pub comments: Vec<Comment>,
    pub hot_comments: Vec<Comment>,
    pub top_comments: Vec<Comment>,
    pub current_comment: Option<Comment>,
    pub pagination: PageMeta,
    pub extensions: Extensions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommentReactionKind {
    Like,
    Hug,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommentReactionListRequest {
    pub target: CommentTarget,
    pub comment_id: String,
    pub target_user_ref: ResourceRef,
    pub kind: CommentReactionKind,
    pub limit: u32,
    pub page: u32,
    pub cursor: Option<String>,
    pub id_cursor: Option<String>,
    pub account: Option<String>,
}

impl CommentReactionListRequest {
    #[must_use]
    pub const fn new(
        target: CommentTarget,
        comment_id: String,
        target_user_ref: ResourceRef,
        kind: CommentReactionKind,
        limit: u32,
    ) -> Self {
        Self {
            target,
            comment_id,
            target_user_ref,
            kind,
            limit,
            page: 1,
            cursor: None,
            id_cursor: None,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommentReaction {
    pub kind: CommentReactionKind,
    pub user: User,
    pub content: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommentReactionPage {
    pub target: CommentTarget,
    pub comment_id: String,
    pub target_user_ref: ResourceRef,
    pub kind: CommentReactionKind,
    pub reactions: Vec<CommentReaction>,
    pub current_comment: Option<Comment>,
    pub pagination: PageMeta,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommentReactionMutationRequest {
    pub target: CommentTarget,
    pub comment_id: String,
    pub kind: CommentReactionKind,
    pub active: bool,
    pub target_user_ref: Option<ResourceRef>,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommentReactionMutationResult {
    pub target: CommentTarget,
    pub comment_id: String,
    pub kind: CommentReactionKind,
    pub active: bool,
    pub target_user_ref: Option<ResourceRef>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommentThreadStatsRequest {
    pub kind: CommentTargetKind,
    pub resource_refs: Vec<ResourceRef>,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommentThreadStats {
    pub target: CommentTarget,
    pub requested_ref: Option<ResourceRef>,
    pub liked: Option<bool>,
    pub like_count: Option<u64>,
    pub comment_count: Option<u64>,
    pub comment_count_text: Option<String>,
    pub share_count: Option<u64>,
    pub comment_upgraded: Option<bool>,
    pub musician_comment_count: Option<u64>,
    pub latest_liked_users: Vec<User>,
    pub comments: Vec<Comment>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CommentThreadStatsBatch {
    pub kind: CommentTargetKind,
    pub requested_refs: Vec<ResourceRef>,
    pub stats: Vec<CommentThreadStats>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AlbumListRequest {
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
    pub area: Option<String>,
    pub catalog: Option<String>,
}

impl AlbumListRequest {
    #[must_use]
    pub fn new(limit: u32, offset: u32) -> Self {
        Self {
            limit,
            offset,
            account: None,
            area: None,
            catalog: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DigitalAlbumListRequest {
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
    pub area: Option<String>,
    pub kind: Option<String>,
    pub catalog: Option<String>,
}

impl DigitalAlbumListRequest {
    #[must_use]
    pub fn new(limit: u32, offset: u32) -> Self {
        Self {
            limit,
            offset,
            account: None,
            area: None,
            kind: None,
            catalog: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DigitalAlbumChartPeriod {
    #[default]
    Daily,
    Week,
    Year,
    Total,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DigitalAlbumChartKind {
    #[default]
    Album,
    Single,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DigitalAlbumChartRequest {
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
    pub period: DigitalAlbumChartPeriod,
    pub kind: DigitalAlbumChartKind,
    pub year: Option<u16>,
}

impl DigitalAlbumChartRequest {
    #[must_use]
    pub fn new(limit: u32, offset: u32) -> Self {
        Self {
            limit,
            offset,
            account: None,
            period: DigitalAlbumChartPeriod::Daily,
            kind: DigitalAlbumChartKind::Album,
            year: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackHistoryPeriod {
    #[default]
    AllTime,
    Week,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlaybackHistoryRequest {
    pub period: PlaybackHistoryPeriod,
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
}

impl PlaybackHistoryRequest {
    #[must_use]
    pub fn new(period: PlaybackHistoryPeriod, limit: u32, offset: u32) -> Self {
        Self {
            period,
            limit,
            offset,
            account: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecommendationRequest {
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
    pub refresh: bool,
    #[serde(default)]
    pub source: RecommendationSource,
    pub area_id: Option<u64>,
}

impl RecommendationRequest {
    #[must_use]
    pub fn new(limit: u32, offset: u32) -> Self {
        Self {
            limit,
            offset,
            account: None,
            refresh: false,
            source: RecommendationSource::Daily,
            area_id: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecommendationSource {
    #[default]
    Daily,
    Personalized,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoRecommendationKind {
    #[default]
    Mv,
    Exclusive,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoRecommendationView {
    #[default]
    Featured,
    Catalog,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VideoRecommendationRequest {
    pub kind: VideoRecommendationKind,
    pub view: VideoRecommendationView,
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
}

impl Default for VideoRecommendationRequest {
    fn default() -> Self {
        Self {
            kind: VideoRecommendationKind::Mv,
            view: VideoRecommendationView::Featured,
            limit: 30,
            offset: 0,
            account: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonalFmVariant {
    #[default]
    Classic,
    Mode,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PersonalFmRequest {
    pub variant: PersonalFmVariant,
    pub mode: Option<String>,
    pub sub_mode: Option<String>,
    pub limit: u32,
    pub account: Option<String>,
}

impl Default for PersonalFmRequest {
    fn default() -> Self {
        Self {
            variant: PersonalFmVariant::Classic,
            mode: None,
            sub_mode: None,
            limit: 3,
            account: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecommendationDislikeRequest {
    pub track_ref: ResourceRef,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RecommendationDislikeResult {
    pub track_ref: ResourceRef,
    pub applied: bool,
    pub extensions: Extensions,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtistCategory {
    #[default]
    All,
    Male,
    Female,
    Group,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtistArea {
    #[default]
    All,
    Chinese,
    Western,
    Japanese,
    Korean,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtistListRequest {
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
    pub category: ArtistCategory,
    pub area: ArtistArea,
    pub initial: Option<String>,
}

impl ArtistListRequest {
    #[must_use]
    pub fn new(limit: u32, offset: u32) -> Self {
        Self {
            limit,
            offset,
            account: None,
            category: ArtistCategory::All,
            area: ArtistArea::All,
            initial: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoKind {
    #[default]
    All,
    Mv,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtistVideoListRequest {
    pub limit: u32,
    pub offset: u32,
    pub cursor: Option<String>,
    pub account: Option<String>,
    pub kind: VideoKind,
    pub order: Option<String>,
}

impl ArtistVideoListRequest {
    #[must_use]
    pub fn new(limit: u32, offset: u32) -> Self {
        Self {
            limit,
            offset,
            cursor: None,
            account: None,
            kind: VideoKind::All,
            order: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArtistTrackOrder {
    #[default]
    Hot,
    Time,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtistTrackListRequest {
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
    pub order: ArtistTrackOrder,
}

impl ArtistTrackListRequest {
    #[must_use]
    pub fn new(limit: u32, offset: u32) -> Self {
        Self {
            limit,
            offset,
            account: None,
            order: ArtistTrackOrder::Hot,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtistUpdatesRequest {
    pub limit: u32,
    pub before_ms: Option<u64>,
    pub account: Option<String>,
}

impl ArtistUpdatesRequest {
    #[must_use]
    pub fn new(limit: u32) -> Self {
        Self {
            limit,
            before_ms: None,
            account: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtistWorksRequest {
    pub limit: u32,
    pub before_ms: Option<u64>,
    pub source_type: u32,
    pub first_request: bool,
    pub account: Option<String>,
}

impl ArtistWorksRequest {
    #[must_use]
    pub fn new(limit: u32) -> Self {
        Self {
            limit,
            before_ms: None,
            source_type: 1,
            first_request: true,
            account: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtistWorkKind {
    Track,
    Video,
    Mixed,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtistWorkUpdate {
    pub source_type: u32,
    pub kind: ArtistWorkKind,
    pub published_at: Option<String>,
    pub artist: Option<ArtistSummary>,
    pub title: Option<String>,
    pub cover_url: Option<String>,
    pub tracks: Vec<Track>,
    pub videos: Vec<Video>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtistSummary {
    #[serde(rename = "ref")]
    pub resource_ref: Option<ResourceRef>,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtistBiographySection {
    pub title: String,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Artist {
    #[serde(rename = "ref")]
    pub resource_ref: ResourceRef,
    pub platform: Platform,
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub description: String,
    pub biography_sections: Vec<ArtistBiographySection>,
    pub avatar_url: Option<String>,
    pub cover_url: Option<String>,
    pub album_count: Option<u64>,
    pub track_count: Option<u64>,
    pub mv_count: Option<u64>,
    pub video_count: Option<u64>,
    pub identities: Vec<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtistOverview {
    pub artist: Artist,
    pub featured_tracks: Vec<Track>,
    pub has_more_tracks: bool,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtistContentCount {
    pub category: Option<String>,
    pub count: u64,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtistStats {
    pub artist_ref: ResourceRef,
    pub followed: Option<bool>,
    pub follower_count: Option<u64>,
    pub video_counts: Vec<ArtistContentCount>,
    pub online_concert_count: Option<u64>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct User {
    #[serde(rename = "ref")]
    pub resource_ref: ResourceRef,
    pub platform: Platform,
    pub id: String,
    pub name: String,
    pub avatar_url: Option<String>,
    pub signature: Option<String>,
    pub followed: Option<bool>,
    pub mutual: Option<bool>,
    pub extensions: Extensions,
}

/// A platform-neutral public user profile.
///
/// Platform-specific identity, privacy, binding, badge, and social fields remain available in
/// `extensions`; the stable fields below cover the profile data shared by music platforms.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserProfile {
    pub user: User,
    pub level: Option<u32>,
    pub listened_track_count: Option<u64>,
    pub playlist_count: Option<u64>,
    pub playlist_subscriber_count: Option<u64>,
    pub following_count: Option<u64>,
    pub follower_count: Option<u64>,
    pub event_count: Option<u64>,
    pub birthday: Option<String>,
    pub created_at: Option<String>,
    pub background_url: Option<String>,
    pub description: Option<String>,
    pub public_listening_history: Option<bool>,
    pub extensions: Extensions,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserProfileBackend {
    Legacy,
    #[default]
    Modern,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CreatorSummary {
    #[serde(rename = "ref")]
    pub resource_ref: Option<ResourceRef>,
    pub name: String,
    pub avatar_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Video {
    #[serde(rename = "ref")]
    pub resource_ref: ResourceRef,
    pub platform: Platform,
    pub id: String,
    pub title: String,
    pub creators: Vec<CreatorSummary>,
    pub description: String,
    pub cover_url: Option<String>,
    pub duration_ms: Option<u64>,
    pub published_at: Option<String>,
    pub play_count: Option<u64>,
    pub subscribed: Option<bool>,
    pub extensions: Extensions,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicVideoCatalog {
    #[default]
    All,
    Latest,
    Exclusive,
    TimelineAll,
    TimelineRecommended,
    Group,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicVideoArea {
    #[default]
    All,
    MainlandChina,
    HongKongTaiwan,
    Western,
    Japan,
    Korea,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicVideoType {
    #[default]
    All,
    Official,
    Original,
    Live,
    Netease,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MusicVideoOrder {
    #[default]
    Rising,
    Hot,
    New,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MusicVideoListRequest {
    pub catalog: MusicVideoCatalog,
    pub limit: u32,
    pub offset: u32,
    pub area: Option<MusicVideoArea>,
    pub video_type: Option<MusicVideoType>,
    pub order: Option<MusicVideoOrder>,
    pub group_id: Option<String>,
    pub account: Option<String>,
}

impl MusicVideoListRequest {
    #[must_use]
    pub fn new(catalog: MusicVideoCatalog, limit: u32, offset: u32) -> Self {
        Self {
            catalog,
            limit,
            offset,
            area: None,
            video_type: None,
            order: None,
            group_id: None,
            account: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoTaxonomyKind {
    #[default]
    Categories,
    Groups,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VideoTaxonomyRequest {
    pub kind: VideoTaxonomyKind,
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
}

impl VideoTaxonomyRequest {
    #[must_use]
    pub fn new(kind: VideoTaxonomyKind, limit: u32, offset: u32) -> Self {
        Self {
            kind,
            limit,
            offset,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoCatalogOption {
    pub id: String,
    pub name: String,
    pub url: Option<String>,
    pub selected: Option<bool>,
    pub related_video_type: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VideoResourceKind {
    #[default]
    Mv,
    Video,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VideoDetailRequest {
    pub kind: VideoResourceKind,
    pub account: Option<String>,
}

impl VideoDetailRequest {
    #[must_use]
    pub fn new(kind: VideoResourceKind) -> Self {
        Self {
            kind,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoResolution {
    pub resolution: u32,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub size: Option<u64>,
    pub format: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoDetail {
    pub kind: VideoResourceKind,
    pub video: Video,
    pub resolutions: Vec<VideoResolution>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoStats {
    pub video_ref: ResourceRef,
    pub kind: VideoResourceKind,
    pub liked: Option<bool>,
    pub like_count: Option<u64>,
    pub comment_count: Option<u64>,
    pub share_count: Option<u64>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VideoStreamRequest {
    pub kind: VideoResourceKind,
    pub resolution: u32,
    pub account: Option<String>,
}

impl VideoStreamRequest {
    pub const DEFAULT_RESOLUTION: u32 = 1080;

    #[must_use]
    pub fn new(kind: VideoResourceKind, resolution: u32) -> Self {
        Self {
            kind,
            resolution,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VideoStream {
    pub video_ref: ResourceRef,
    pub platform: Platform,
    pub available: bool,
    pub url: Option<String>,
    pub backup_urls: Vec<String>,
    pub headers: BTreeMap<String, String>,
    pub expires_at: Option<String>,
    pub format: Option<String>,
    pub codec: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub size: Option<u64>,
    pub duration_ms: Option<u64>,
    pub requested_resolution: u32,
    pub actual_resolution: Option<u32>,
    pub platform_code: Option<i64>,
    pub fee: Option<i64>,
    pub message: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AlbumSummary {
    #[serde(rename = "ref")]
    pub resource_ref: Option<ResourceRef>,
    pub name: String,
    pub cover_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Album {
    #[serde(rename = "ref")]
    pub resource_ref: ResourceRef,
    pub platform: Platform,
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub artists: Vec<ArtistSummary>,
    pub description: String,
    pub cover_url: Option<String>,
    pub published_at: Option<String>,
    pub track_count: Option<u64>,
    pub company: Option<String>,
    pub kind: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlbumStats {
    pub album_ref: ResourceRef,
    pub subscribed: Option<bool>,
    pub subscriber_count: Option<u64>,
    pub comment_count: Option<u64>,
    pub share_count: Option<u64>,
    pub like_count: Option<u64>,
    pub on_sale: Option<bool>,
    pub subscribed_at: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SubscriptionResult {
    pub resource_ref: ResourceRef,
    pub subscribed: bool,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackEntitlement {
    pub track_ref: ResourceRef,
    pub playable: Option<bool>,
    pub downloadable: Option<bool>,
    pub play_bitrate: Option<u64>,
    pub download_bitrate: Option<u64>,
    pub max_play_bitrate: Option<u64>,
    pub max_download_bitrate: Option<u64>,
    pub play_quality: Option<Quality>,
    pub download_quality: Option<Quality>,
    pub available_qualities: Vec<Quality>,
    pub fee: Option<i64>,
    pub paid: Option<bool>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TrackAvailabilityRequest {
    pub bitrate: u64,
    pub account: Option<String>,
}

impl TrackAvailabilityRequest {
    pub const DEFAULT_BITRATE: u64 = 999_000;

    #[must_use]
    pub fn new(bitrate: u64) -> Self {
        Self {
            bitrate,
            account: None,
        }
    }
}

impl Default for TrackAvailabilityRequest {
    fn default() -> Self {
        Self::new(Self::DEFAULT_BITRATE)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TrackAvailability {
    pub track_ref: ResourceRef,
    pub playable: bool,
    pub requested_bitrate: u64,
    pub actual_bitrate: Option<u64>,
    pub platform_code: Option<i64>,
    pub message: String,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Money {
    pub amount: f64,
    pub currency: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DigitalAlbum {
    #[serde(rename = "ref")]
    pub resource_ref: ResourceRef,
    pub platform: Platform,
    pub id: String,
    pub name: String,
    pub artists: Vec<ArtistSummary>,
    pub description: String,
    pub cover_url: Option<String>,
    pub published_at: Option<String>,
    pub price: Option<Money>,
    pub is_free: Option<bool>,
    pub purchasable: Option<bool>,
    pub purchased: Option<bool>,
    pub sale_count: Option<u64>,
    pub track_count: Option<u64>,
    pub tags: Vec<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DigitalAlbumChartEntry {
    pub rank: u32,
    pub rank_change: Option<i64>,
    pub product: DigitalAlbum,
    pub extensions: Extensions,
}

/// Selects one provider-native presentation of the general music chart catalog.
///
/// Providers that expose fewer variants may map every value to their richest catalog while
/// preserving the requested value and native response in `extensions`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChartCatalogView {
    Overview,
    #[default]
    Summary,
    Modern,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChartCatalogRequest {
    pub view: ChartCatalogView,
    pub account: Option<String>,
}

impl ChartCatalogRequest {
    #[must_use]
    pub fn new(view: ChartCatalogView) -> Self {
        Self {
            view,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartTrackPreview {
    pub rank: Option<u32>,
    pub previous_rank: Option<u32>,
    pub rank_change: Option<i64>,
    pub track_ref: Option<ResourceRef>,
    pub name: String,
    pub byline: Option<String>,
    pub cover_url: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Chart {
    #[serde(rename = "ref")]
    pub resource_ref: Option<ResourceRef>,
    pub platform: Platform,
    pub id: Option<String>,
    pub name: String,
    pub description: String,
    pub cover_url: Option<String>,
    pub update_frequency: Option<String>,
    pub updated_at_ms: Option<u64>,
    pub track_count: Option<u64>,
    pub play_count: Option<u64>,
    pub subscribed: Option<bool>,
    pub playable: Option<bool>,
    pub target_kind: Option<String>,
    pub target_url: Option<String>,
    pub previews: Vec<ChartTrackPreview>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartGroup {
    pub code: Option<String>,
    pub name: String,
    pub display_type: Option<String>,
    pub target_url: Option<String>,
    pub charts: Vec<Chart>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartCatalog {
    pub platform: Platform,
    pub view: ChartCatalogView,
    pub groups: Vec<ChartGroup>,
    pub extensions: Extensions,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtistChartArea {
    #[default]
    Chinese,
    Western,
    Korean,
    Japanese,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtistChartRequest {
    pub area: ArtistChartArea,
    pub account: Option<String>,
}

impl ArtistChartRequest {
    #[must_use]
    pub fn new(area: ArtistChartArea) -> Self {
        Self {
            area,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtistChartEntry {
    pub rank: u32,
    pub previous_rank: Option<u32>,
    pub rank_change: Option<i64>,
    pub score: Option<u64>,
    pub artist: Artist,
    pub extensions: Extensions,
}

/// A complete provider-native artist chart snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtistChart {
    pub platform: Platform,
    pub area: ArtistChartArea,
    pub updated_at_ms: Option<u64>,
    pub entries: Vec<ArtistChartEntry>,
    pub extensions: Extensions,
}

/// Identifies one platform-defined chart dimension, such as a city or style.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DimensionChartRequest {
    pub chart_code: String,
    pub target_id: String,
    pub target_type: String,
    pub account: Option<String>,
}

impl DimensionChartRequest {
    #[must_use]
    pub fn new(
        chart_code: impl Into<String>,
        target_id: impl Into<String>,
        target_type: impl Into<String>,
    ) -> Self {
        Self {
            chart_code: chart_code.into(),
            target_id: target_id.into(),
            target_type: target_type.into(),
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DimensionChart {
    #[serde(rename = "ref")]
    pub resource_ref: ResourceRef,
    pub platform: Platform,
    pub id: String,
    pub chart_code: String,
    pub target_id: String,
    pub target_type: String,
    pub name: String,
    pub description: String,
    pub cover_url: Option<String>,
    pub updated_at_ms: Option<u64>,
    pub play_count: Option<u64>,
    pub share_count: Option<u64>,
    pub comment_count: Option<u64>,
    pub supports_comments: Option<bool>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DimensionChartTrackEntry {
    pub rank: u32,
    pub previous_rank: Option<u32>,
    pub rank_change: Option<i64>,
    pub track: Track,
    pub reason: Option<String>,
    pub reason_id: Option<String>,
    pub score: Option<f64>,
    pub ratio: Option<f64>,
    pub collected: Option<bool>,
    pub extensions: Extensions,
}

/// A complete, non-paginated snapshot returned by a dimension chart.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct DimensionChartTrackSnapshot {
    pub chart_ref: ResourceRef,
    pub chart_code: String,
    pub target_id: String,
    pub target_type: String,
    pub entries: Vec<DimensionChartTrackEntry>,
    pub period_label: Option<String>,
    pub groups: BTreeMap<String, String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Track {
    #[serde(rename = "ref")]
    pub resource_ref: ResourceRef,
    pub platform: Platform,
    pub id: String,
    pub name: String,
    pub aliases: Vec<String>,
    pub artists: Vec<ArtistSummary>,
    pub album: Option<AlbumSummary>,
    pub duration_ms: Option<u64>,
    pub isrc: Option<String>,
    pub mv_ref: Option<ResourceRef>,
    pub playable: Option<bool>,
    pub available_qualities: Vec<Quality>,
    pub extensions: Extensions,
}

impl Track {
    pub fn new(resource_ref: ResourceRef, name: impl Into<String>) -> Self {
        Self {
            platform: resource_ref.platform(),
            id: resource_ref.id().to_owned(),
            resource_ref,
            name: name.into(),
            aliases: Vec::new(),
            artists: Vec::new(),
            album: None,
            duration_ms: None,
            isrc: None,
            mv_ref: None,
            playable: None,
            available_qualities: Vec::new(),
            extensions: Extensions::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaybackHistoryEntry {
    pub track: Track,
    pub play_count: Option<u64>,
    pub score: Option<u64>,
    pub last_played_at: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Playlist {
    #[serde(rename = "ref")]
    pub resource_ref: ResourceRef,
    pub platform: Platform,
    pub id: String,
    pub name: String,
    pub description: String,
    pub cover_url: Option<String>,
    pub creator: Option<ArtistSummary>,
    pub track_count: Option<u64>,
    pub tags: Vec<String>,
    pub subscribed: Option<bool>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
    pub extensions: Extensions,
}

/// A typed playable entry exposed by a platform playlist implementation.
///
/// Providers may override the default track-only bridge when their playlists contain videos,
/// podcast episodes, radio stations, or another supported playable resource category.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "resource", rename_all = "snake_case")]
pub enum PlaylistPlayableItem {
    Track(Track),
    Video(VideoDetail),
    PodcastEpisode(Box<PodcastEpisode>),
    RadioStation(RadioStation),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UniPlaylistCreateRequest {
    pub name: String,
    pub description: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UniPlaylistImportSourceRequest {
    #[serde(rename = "ref")]
    pub playlist_ref: ResourceRef,
    #[serde(rename = "type")]
    pub source_type: String,
    pub account: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UniPlaylistImportRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub sources: Vec<UniPlaylistImportSourceRequest>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UniPlaylistImportSourceResult {
    #[serde(rename = "ref")]
    pub playlist_ref: ResourceRef,
    pub platform: Platform,
    pub id: String,
    #[serde(rename = "type")]
    pub source_type: String,
    pub name: String,
    pub item_count: u64,
    pub account: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UniPlaylistImportResult {
    pub playlist: UniPlaylist,
    pub sources: Vec<UniPlaylistImportSourceResult>,
    pub extensions: Extensions,
}

impl UniPlaylistCreateRequest {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: String::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UniPlaylist {
    #[serde(rename = "ref")]
    pub resource_ref: ResourceRef,
    pub platform: Platform,
    pub id: String,
    pub name: String,
    pub description: String,
    pub item_count: u64,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub extensions: Extensions,
}

impl UniPlaylist {
    #[must_use]
    pub fn new(
        resource_ref: ResourceRef,
        name: impl Into<String>,
        description: impl Into<String>,
        created_at_ms: u64,
    ) -> Self {
        let platform = resource_ref.platform();
        let id = resource_ref.id().to_owned();
        Self {
            resource_ref,
            platform,
            id,
            name: name.into(),
            description: description.into(),
            item_count: 0,
            created_at_ms,
            updated_at_ms: created_at_ms,
            extensions: Extensions::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UniPlaylistItemKind {
    Track,
    Mv,
    Video,
    PodcastEpisode,
    RadioStation,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UniPlaylistItemSnapshot {
    pub title: String,
    pub artists: Vec<String>,
    pub album: Option<String>,
    pub duration_ms: Option<u64>,
    pub isrc: Option<String>,
    pub cover_url: Option<String>,
    pub version_tags: Vec<String>,
    pub extensions: Extensions,
}

impl UniPlaylistItemSnapshot {
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            artists: Vec::new(),
            album: None,
            duration_ms: None,
            isrc: None,
            cover_url: None,
            version_tags: Vec::new(),
            extensions: Extensions::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UniPlaylistItem {
    pub id: String,
    pub position: u64,
    pub kind: UniPlaylistItemKind,
    pub source_ref: ResourceRef,
    pub snapshot: UniPlaylistItemSnapshot,
    pub added_at_ms: u64,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UniPlaylistItemInput {
    #[serde(rename = "ref")]
    pub resource_ref: ResourceRef,
    pub kind: UniPlaylistItemKind,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UniPlaylistItemAddRequest {
    pub items: Vec<UniPlaylistItemInput>,
    pub accounts: BTreeMap<Platform, String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UniPlaylistItemAddResult {
    pub playlist: UniPlaylist,
    pub items: Vec<UniPlaylistItem>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UniPlaylistItemDeleteResult {
    pub playlist: UniPlaylist,
    pub item: UniPlaylistItem,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct UniPlaylistItemOrderRequest {
    pub item_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UniPlaylistItemOrderResult {
    pub playlist: UniPlaylist,
    pub items: Vec<UniPlaylistItem>,
    pub changed: bool,
    pub extensions: Extensions,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaylistVisibility {
    #[default]
    Public,
    Private,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaylistKind {
    #[default]
    Normal,
    Video,
    Shared,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlaylistCreateRequest {
    pub name: String,
    pub visibility: PlaylistVisibility,
    pub kind: PlaylistKind,
    pub account: Option<String>,
}

impl PlaylistCreateRequest {
    #[must_use]
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            visibility: PlaylistVisibility::Public,
            kind: PlaylistKind::Normal,
            account: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaylistMetadataUpdateVariant {
    #[default]
    Default,
    Batch,
    Individual,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlaylistUpdateRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub tags: Option<Vec<String>>,
    pub variant: PlaylistMetadataUpdateVariant,
    pub account: Option<String>,
}

impl PlaylistUpdateRequest {
    #[must_use]
    pub fn new() -> Self {
        Self {
            name: None,
            description: None,
            tags: None,
            variant: PlaylistMetadataUpdateVariant::Default,
            account: None,
        }
    }
}

impl Default for PlaylistUpdateRequest {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaylistMutationAction {
    Create,
    Update,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaylistMutationResult {
    pub playlist_ref: ResourceRef,
    pub action: PlaylistMutationAction,
    pub playlist: Option<Playlist>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlaylistDeleteRequest {
    pub playlist_refs: Vec<ResourceRef>,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaylistDeleteResult {
    pub playlist_refs: Vec<ResourceRef>,
    pub extensions: Extensions,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaylistItemMutationAction {
    Add,
    Remove,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlaylistItemKind {
    #[default]
    Track,
    Video,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlaylistItemMutationRequest {
    pub item_refs: Vec<ResourceRef>,
    pub kind: PlaylistItemKind,
    pub account: Option<String>,
}

impl PlaylistItemMutationRequest {
    #[must_use]
    pub fn new(item_refs: Vec<ResourceRef>, kind: PlaylistItemKind) -> Self {
        Self {
            item_refs,
            kind,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaylistItemMutationResult {
    pub playlist_ref: ResourceRef,
    pub item_refs: Vec<ResourceRef>,
    pub kind: PlaylistItemKind,
    pub action: PlaylistItemMutationAction,
    pub snapshot_id: Option<String>,
    pub cloud_track_count: Option<u64>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlaylistTrackOrderRequest {
    pub track_refs: Vec<ResourceRef>,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaylistTrackOrderResult {
    pub playlist_ref: ResourceRef,
    pub track_refs: Vec<ResourceRef>,
    pub snapshot_id: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PlaylistOrderRequest {
    pub playlist_refs: Vec<ResourceRef>,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaylistOrderResult {
    pub playlist_refs: Vec<ResourceRef>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaylistCoverUpdateResult {
    pub playlist_ref: ResourceRef,
    pub image: ImageUploadResult,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LyricContributor {
    pub role: String,
    #[serde(rename = "ref")]
    pub resource_ref: Option<ResourceRef>,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Lyrics {
    pub track_ref: ResourceRef,
    pub plain: Option<String>,
    pub translated: Option<String>,
    pub romanized: Option<String>,
    pub word_synced: Option<String>,
    pub format: String,
    pub contributors: Vec<LyricContributor>,
    pub extensions: Extensions,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamVariant {
    #[default]
    Default,
    Legacy,
    Modern,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StreamRequest {
    pub quality: Quality,
    #[serde(default)]
    pub variant: StreamVariant,
    #[serde(default)]
    pub bitrate: Option<u64>,
    pub account: Option<String>,
}

impl Default for StreamRequest {
    fn default() -> Self {
        Self {
            quality: Quality::Auto,
            variant: StreamVariant::Default,
            bitrate: None,
            account: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolveRequest {
    pub quality: Quality,
    #[serde(default)]
    pub variant: StreamVariant,
    #[serde(default)]
    pub bitrate: Option<u64>,
    pub playback_platforms: Vec<Platform>,
    pub fallback: bool,
    pub accounts: BTreeMap<Platform, String>,
    pub strict_match: bool,
}

impl Default for ResolveRequest {
    fn default() -> Self {
        Self {
            quality: Quality::Auto,
            variant: StreamVariant::Default,
            bitrate: None,
            playback_platforms: Vec::new(),
            fallback: true,
            accounts: BTreeMap::new(),
            strict_match: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TrialWindow {
    pub start_ms: u64,
    pub end_ms: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolutionStatus {
    Success,
    NoMatch,
    Unavailable,
    AuthenticationRequired,
    PermissionDenied,
    UpstreamError,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StreamOutcome {
    pub track_ref: ResourceRef,
    pub status: ResolutionStatus,
    pub stream: Option<MediaStream>,
    pub error_code: Option<crate::ErrorCode>,
    pub error: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct StreamBatch {
    pub outcomes: Vec<StreamOutcome>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolutionAttempt {
    pub platform: Platform,
    pub account: Option<String>,
    pub candidate: Option<ResourceRef>,
    pub match_score: Option<f64>,
    pub status: ResolutionStatus,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaStream {
    pub url: String,
    pub backup_urls: Vec<String>,
    pub headers: BTreeMap<String, String>,
    pub expires_at: Option<String>,
    pub format: Option<String>,
    pub codec: Option<String>,
    pub bitrate: Option<u64>,
    pub size: Option<u64>,
    pub duration_ms: Option<u64>,
    pub requested_quality: Quality,
    pub actual_quality: Quality,
    pub trial: Option<TrialWindow>,
    pub origin_track: Option<ResourceRef>,
    pub resolved_track: ResourceRef,
    pub resolved_platform: Platform,
    pub match_score: Option<f64>,
    pub attempts: Vec<ResolutionAttempt>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MediaDownload {
    #[serde(rename = "ref")]
    pub track_ref: ResourceRef,
    pub platform: Platform,
    pub available: bool,
    pub url: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub expires_at: Option<String>,
    pub format: Option<String>,
    pub codec: Option<String>,
    pub bitrate: Option<u64>,
    pub size: Option<u64>,
    pub duration_ms: Option<u64>,
    pub requested_quality: Quality,
    pub actual_quality: Quality,
    pub platform_code: Option<i64>,
    pub fee: Option<i64>,
    pub message: Option<String>,
    pub extensions: Extensions,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProviderDescriptor {
    pub platform: Platform,
    pub name: String,
    pub capabilities: Vec<Capability>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn track_constructor_keeps_reference_fields_consistent() {
        let reference = ResourceRef::new(Platform::Netease, "123").expect("valid reference");
        let track = Track::new(reference.clone(), "Example");

        assert_eq!(track.resource_ref, reference);
        assert_eq!(track.platform, Platform::Netease);
        assert_eq!(track.id, "123");
    }

    #[test]
    fn money_keeps_decimal_amount_and_currency() {
        let money = Money {
            amount: 22.0,
            currency: "CNY".to_owned(),
        };

        let value = serde_json::to_value(money).expect("serialize money");
        assert_eq!(value["amount"], 22.0);
        assert_eq!(value["currency"], "CNY");
    }

    #[test]
    fn platform_api_request_keeps_protocol_and_account_provider_owned() {
        let mut request =
            PlatformApiRequest::new("/api/search/get", serde_json::json!({ "s": "TuneWeave" }));
        request.protocol = Some("eapi".to_owned());
        request.account = Some("default".to_owned());

        assert_eq!(request.uri, "/api/search/get");
        assert_eq!(request.data["s"], "TuneWeave");
        assert_eq!(request.protocol.as_deref(), Some("eapi"));
        assert_eq!(request.account.as_deref(), Some("default"));
    }

    #[test]
    fn platform_batch_request_keeps_dynamic_provider_calls() {
        let mut requests = BTreeMap::new();
        requests.insert(
            "/api/v2/banner/get".to_owned(),
            serde_json::json!({ "clientType": "pc" }),
        );
        let mut request = PlatformBatchRequest::new(requests);
        request.protocol = Some("eapi".to_owned());
        request.encrypted_response = true;
        request.account = Some("default".to_owned());

        assert_eq!(request.requests.len(), 1);
        assert_eq!(request.requests["/api/v2/banner/get"]["clientType"], "pc");
        assert_eq!(request.protocol.as_deref(), Some("eapi"));
        assert!(request.encrypted_response);
        assert_eq!(request.account.as_deref(), Some("default"));
    }

    #[test]
    fn radio_taxonomy_keeps_provider_ids_as_strings() {
        let taxonomy = RadioTaxonomy {
            categories: vec![RadioCatalogOption {
                id: "1".to_owned(),
                name: "音乐台".to_owned(),
                extensions: Extensions::new(),
            }],
            regions: vec![RadioCatalogOption {
                id: "407".to_owned(),
                name: "网络台".to_owned(),
                extensions: Extensions::new(),
            }],
            extensions: Extensions::new(),
        };

        let value = serde_json::to_value(taxonomy).expect("serialize radio taxonomy");
        assert_eq!(value["categories"][0]["id"], "1");
        assert_eq!(value["regions"][0]["id"], "407");
    }

    #[test]
    fn radio_style_catalog_preserves_hierarchy_and_source_qualified_channels() {
        let channel_ref =
            ResourceRef::new(Platform::Netease, "difm:0:10505").expect("valid channel reference");
        let channel = RadioStation::new(channel_ref, "Deep Progressive House");
        let catalog = RadioStyleCatalog {
            sources: vec![RadioStyleSource {
                id: 0,
                styles: vec![RadioStyle {
                    id: "difm:0:1020".to_owned(),
                    name: "New".to_owned(),
                    localized_name: Some("新晋".to_owned()),
                    description: "New electronic channels".to_owned(),
                    channels: vec![channel],
                    extensions: Extensions::new(),
                }],
                extensions: Extensions::new(),
            }],
            extensions: Extensions::new(),
        };

        let value = serde_json::to_value(catalog).expect("serialize radio style catalog");
        assert_eq!(value["sources"][0]["id"], 0);
        assert_eq!(value["sources"][0]["styles"][0]["id"], "difm:0:1020");
        assert_eq!(
            value["sources"][0]["styles"][0]["channels"][0]["ref"],
            "netease:difm:0:10505"
        );
        assert_eq!(
            value["sources"][0]["styles"][0]["channels"][0]["id"],
            "difm:0:10505"
        );
        assert_eq!(RadioStyleCatalogRequest::default().sources, vec![0]);
    }

    #[test]
    fn radio_playback_queue_keeps_station_item_and_direct_stream_distinct() {
        let station_ref =
            ResourceRef::new(Platform::Netease, "difm:0:10505").expect("valid station reference");
        let item_ref = ResourceRef::new(Platform::Netease, "difm-track:0:10505:199222851")
            .expect("valid playback item reference");
        let mut item = RadioPlaybackItem::new(
            item_ref,
            station_ref.clone(),
            "Green Forest (Dezza & Rylan Taggart Remix)",
        );
        item.artist = Some("Max Freegrant & Slow Fish".to_owned());
        item.stream_url = Some("https://example.test/difm.mp3".to_owned());
        item.duration_ms = Some(351_000);
        item.waveform = vec![0.0003, 0.2434];
        let queue = RadioPlaybackQueue {
            station_ref,
            items: vec![item],
            total: Some(1),
            extensions: Extensions::new(),
        };

        let value = serde_json::to_value(queue).expect("serialize radio playback queue");
        assert_eq!(value["station_ref"], "netease:difm:0:10505");
        assert_eq!(
            value["items"][0]["ref"],
            "netease:difm-track:0:10505:199222851"
        );
        assert_eq!(value["items"][0]["station_ref"], "netease:difm:0:10505");
        assert_eq!(value["items"][0]["duration_ms"], 351_000);
        assert_eq!(value["items"][0]["waveform"][1], 0.2434);
        assert_eq!(RadioPlaybackQueueRequest::default().limit, 5);
    }

    #[test]
    fn styled_radio_station_library_request_keeps_sources_and_account_explicit() {
        let mut request = StyledRadioStationLibraryRequest::new(vec![0, 1, 2], 25);
        request.account = Some("radio-user".to_owned());

        let value = serde_json::to_value(request).expect("serialize styled radio library request");
        assert_eq!(value["sources"][0], 0);
        assert_eq!(value["sources"][1], 1);
        assert_eq!(value["sources"][2], 2);
        assert_eq!(value["limit"], 25);
        assert_eq!(value["offset"], 0);
        assert_eq!(value["account"], "radio-user");
    }

    #[test]
    fn radio_station_constructor_keeps_reference_fields_consistent() {
        let reference = ResourceRef::new(Platform::Netease, "362").expect("valid reference");
        let mut station = RadioStation::new(reference.clone(), "金山区广播电视台综合广播");
        station.region = Some("上海".to_owned());
        station.subscribed = Some(false);

        assert_eq!(station.resource_ref, reference);
        assert_eq!(station.platform, Platform::Netease);
        assert_eq!(station.id, "362");
        assert_eq!(station.region.as_deref(), Some("上海"));
        assert_eq!(station.subscribed, Some(false));
    }

    #[test]
    fn radio_station_list_request_keeps_filter_and_cursor_ids_opaque() {
        let mut request = RadioStationListRequest::new(20);
        request.offset = 100;
        request.category_id = Some("music:featured".to_owned());
        request.region_id = Some("region:network".to_owned());
        request.cursor = Some(RadioStationCursor {
            id: "station:172".to_owned(),
            score: 1542,
        });
        request.account = Some("radio-user".to_owned());

        let value = serde_json::to_value(request).expect("serialize radio station list request");
        assert_eq!(value["limit"], 20);
        assert_eq!(value["offset"], 100);
        assert_eq!(value["category_id"], "music:featured");
        assert_eq!(value["region_id"], "region:network");
        assert_eq!(value["cursor"]["id"], "station:172");
        assert_eq!(value["cursor"]["score"], 1542);
        assert_eq!(value["account"], "radio-user");
    }

    #[test]
    fn podcast_and_episode_keep_show_audio_and_stream_identity_distinct() {
        let podcast_ref =
            ResourceRef::new(Platform::Netease, "336355127").expect("valid podcast reference");
        let episode_ref =
            ResourceRef::new(Platform::Netease, "1367665101").expect("valid episode reference");
        let audio_ref =
            ResourceRef::new(Platform::Netease, "478446370").expect("valid audio reference");
        let mut podcast = Podcast::new(podcast_ref.clone(), "代码时间");
        podcast.price = Some(Money {
            amount: 12.9,
            currency: "CNY".to_owned(),
        });
        let mut episode = PodcastEpisode::new(episode_ref.clone(), "一期节目");
        episode.podcast_ref = Some(podcast_ref.clone());
        episode.audio = Some(Track::new(audio_ref.clone(), "一期节目"));

        assert_eq!(podcast.resource_ref, podcast_ref);
        let podcast_value = serde_json::to_value(podcast).expect("serialize podcast");
        assert_eq!(podcast_value["price"]["amount"], 12.9);
        assert_eq!(podcast_value["price"]["currency"], "CNY");
        assert_eq!(episode.resource_ref, episode_ref);
        assert_eq!(
            episode.audio.as_ref().map(|audio| &audio.resource_ref),
            Some(&audio_ref)
        );
        let value = serde_json::to_value(episode).expect("serialize podcast episode");
        assert_eq!(value["ref"], "netease:1367665101");
        assert_eq!(value["podcast_ref"], "netease:336355127");
        assert_eq!(value["audio"]["ref"], "netease:478446370");
    }

    #[test]
    fn podcast_catalog_contract_keeps_discovery_modes_and_category_ids_explicit() {
        let category = PodcastCategory {
            id: "2001".to_owned(),
            name: "创作与翻唱".to_owned(),
            icon_url: Some("https://example.test/category.png".to_owned()),
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(category).expect("serialize podcast category");
        assert_eq!(value["id"], "2001");
        assert_eq!(value["icon_url"], "https://example.test/category.png");

        let taxonomy = PodcastTaxonomy {
            categories: vec![serde_json::from_value(value).expect("deserialize category")],
            extensions: Extensions::new(),
        };
        assert_eq!(taxonomy.categories.len(), 1);

        let mut taxonomy_request = PodcastTaxonomyRequest::new(PodcastTaxonomyKind::NonHot);
        taxonomy_request.account = Some("spoken-word".to_owned());
        let value =
            serde_json::to_value(taxonomy_request).expect("serialize podcast taxonomy request");
        assert_eq!(value["kind"], "non_hot");
        assert_eq!(value["account"], "spoken-word");

        let recommendations = PodcastCategoryRecommendations {
            sections: vec![PodcastCategoryRecommendation {
                category: PodcastCategory {
                    id: "3".to_owned(),
                    name: "情感".to_owned(),
                    icon_url: None,
                    extensions: Extensions::new(),
                },
                podcasts: vec![Podcast::new(
                    ResourceRef::new(Platform::Netease, "526564706")
                        .expect("valid podcast reference"),
                    "伴听FM",
                )],
                extensions: Extensions::new(),
            }],
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(recommendations)
            .expect("serialize podcast category recommendations");
        assert_eq!(value["sections"][0]["category"]["id"], "3");
        assert_eq!(
            value["sections"][0]["podcasts"][0]["ref"],
            "netease:526564706"
        );

        let mut request = PodcastListRequest::new(PodcastCatalog::CategoryHot, 30, 60);
        request.category_id = Some("2001".to_owned());
        request.page = Some(2);
        request.account = Some("spoken-word".to_owned());
        let value = serde_json::to_value(request).expect("serialize podcast list request");
        assert_eq!(value["catalog"], "category_hot");
        assert_eq!(value["category_id"], "2001");
        assert_eq!(value["limit"], 30);
        assert_eq!(value["offset"], 60);
        assert_eq!(value["page"], 2);
        assert_eq!(value["account"], "spoken-word");
    }

    #[test]
    fn podcast_episode_list_defaults_to_newest_first_without_hiding_account() {
        let mut request = PodcastEpisodeListRequest::new(30, 60);
        request.account = Some("spoken-word".to_owned());

        assert_eq!(request.limit, 30);
        assert_eq!(request.offset, 60);
        assert!(!request.ascending);
        assert_eq!(request.account.as_deref(), Some("spoken-word"));
    }

    #[test]
    fn podcast_episode_order_keeps_list_episode_and_position_identity_distinct() {
        let podcast_ref =
            ResourceRef::new(Platform::Netease, "336355127").expect("podcast reference");
        let episode_ref =
            ResourceRef::new(Platform::Netease, "2058695201").expect("episode reference");
        let request = PodcastEpisodeOrderRequest {
            episode_ref: episode_ref.clone(),
            position: 4,
            limit: 20,
            offset: 40,
            account: Some("studio-user".to_owned()),
        };
        let value = serde_json::to_value(&request).expect("serialize podcast episode order");
        assert_eq!(value["episode_ref"], "netease:2058695201");
        assert_eq!(value["position"], 4);
        assert_eq!(value["limit"], 20);
        assert_eq!(value["offset"], 40);
        assert_eq!(value["account"], "studio-user");

        let result = PodcastEpisodeOrderResult {
            podcast_ref,
            episode_ref,
            position: request.position,
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(result).expect("serialize podcast episode order result");
        assert_eq!(value["podcast_ref"], "netease:336355127");
        assert_eq!(value["episode_ref"], "netease:2058695201");
        assert_eq!(value["position"], 4);
    }

    #[test]
    fn podcast_episode_delete_keeps_ordered_cross_platform_identity_typed() {
        let refs = vec![
            ResourceRef::new(Platform::Netease, "2058695201").expect("first episode reference"),
            ResourceRef::new(Platform::Netease, "2058695202").expect("second episode reference"),
        ];
        let mut request = PodcastEpisodeDeleteRequest::new(refs.clone());
        request.account = Some("studio-user".to_owned());
        let value = serde_json::to_value(&request).expect("serialize podcast episode deletion");
        assert_eq!(
            value["episode_refs"],
            serde_json::json!(["netease:2058695201", "netease:2058695202"])
        );
        assert_eq!(value["account"], "studio-user");

        let result = PodcastEpisodeDeleteResult {
            episode_refs: refs,
            deleted: true,
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(result).expect("serialize podcast episode delete result");
        assert_eq!(value["deleted"], true);
        assert_eq!(value["episode_refs"][0], "netease:2058695201");
    }

    #[test]
    fn podcast_episode_upload_redacts_audio_and_keeps_metadata_typed() {
        let request = PodcastEpisodeUploadRequest {
            filename: "一期节目.mp3".to_owned(),
            content_type: "audio/mpeg".to_owned(),
            data: b"private-audio-content".to_vec(),
            name: Some("第一期".to_owned()),
            cover_image_id: "109951168000000000".to_owned(),
            category_id: "3".to_owned(),
            second_category_id: "14".to_owned(),
            description: "节目介绍".to_owned(),
            privacy: true,
            publish_time_ms: 1_784_194_692_000,
            auto_publish: true,
            auto_publish_text: "新节目".to_owned(),
            order_no: 2,
            composed_track_refs: vec![
                ResourceRef::new(Platform::Netease, "1859245776")
                    .expect("composed track reference"),
            ],
            account: Some("studio-user".to_owned()),
        };
        let debug = format!("{request:?}");
        assert!(debug.contains("data_len: 21"));
        assert!(!debug.contains("private-audio-content"));

        let result = PodcastEpisodeUploadResult {
            podcast_ref: ResourceRef::new(Platform::Netease, "336355127")
                .expect("podcast reference"),
            episode_refs: vec![
                ResourceRef::new(Platform::Netease, "2058695201").expect("episode reference"),
            ],
            name: request.name.expect("upload name"),
            uploaded: true,
            publish_time_ms: request.publish_time_ms,
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(result).expect("serialize podcast episode upload");
        assert_eq!(value["podcast_ref"], "netease:336355127");
        assert_eq!(value["episode_refs"][0], "netease:2058695201");
        assert_eq!(value["uploaded"], true);
        assert_eq!(value["publish_time_ms"], 1_784_194_692_000_u64);
    }

    #[test]
    fn podcast_workbench_search_keeps_every_filter_typed() {
        let mut request = PodcastEpisodeWorkbenchSearchRequest::new(200, 400);
        request.query = Some("一期".to_owned());
        request.display_status = Some(PodcastEpisodeDisplayStatus::SchedulePublish);
        request.visibility = Some(PodcastEpisodeVisibility::Private);
        request.fee_type = Some(PodcastEpisodeFeeFilter::Paid);
        request.podcast_id = Some("336355127".to_owned());
        request.account = Some("studio-user".to_owned());

        let value = serde_json::to_value(request).expect("serialize workbench search request");
        assert_eq!(value["query"], "一期");
        assert_eq!(value["display_status"], "schedule_publish");
        assert_eq!(value["visibility"], "private");
        assert_eq!(value["fee_type"], "paid");
        assert_eq!(value["podcast_id"], "336355127");
        assert_eq!(value["limit"], 200);
        assert_eq!(value["offset"], 400);
        assert_eq!(value["account"], "studio-user");
    }

    #[test]
    fn podcast_chart_keeps_rank_movement_and_podcast_identity_explicit() {
        let mut request = PodcastChartRequest::new(PodcastChartKind::Paid, 30, 0);
        request.account = Some("spoken-word".to_owned());
        let podcast = Podcast::new(
            ResourceRef::new(Platform::Netease, "1490425014").expect("valid podcast reference"),
            "猫平安逆袭传奇",
        );
        let entry = PodcastChartEntry {
            rank: 1,
            previous_rank: Some(-1),
            score: Some(193_200),
            podcast,
            extensions: Extensions::new(),
        };

        let request_value = serde_json::to_value(request).expect("serialize chart request");
        assert_eq!(request_value["kind"], "paid");
        assert_eq!(request_value["account"], "spoken-word");
        let entry_value = serde_json::to_value(entry).expect("serialize chart entry");
        assert_eq!(entry_value["rank"], 1);
        assert_eq!(entry_value["previous_rank"], -1);
        assert_eq!(entry_value["score"], 193_200);
        assert_eq!(entry_value["podcast"]["ref"], "netease:1490425014");
    }

    #[test]
    fn podcast_creator_chart_keeps_rank_followers_and_user_identity_explicit() {
        let mut request =
            PodcastCreatorChartRequest::new(PodcastCreatorChartKind::Trending24Hours, 30, 0);
        request.account = Some("spoken-word".to_owned());
        let creator_ref =
            ResourceRef::new(Platform::Netease, "287921940").expect("valid creator reference");
        let entry = PodcastCreatorChartEntry {
            rank: 1,
            previous_rank: Some(7),
            score: Some(1_339_233),
            follower_count: Some(76_488),
            creator: User {
                platform: Platform::Netease,
                id: "287921940".to_owned(),
                resource_ref: creator_ref,
                name: "开心锤锤".to_owned(),
                avatar_url: Some("https://example.test/avatar.jpg".to_owned()),
                signature: None,
                followed: None,
                mutual: None,
                extensions: Extensions::new(),
            },
            extensions: Extensions::new(),
        };

        let request_value = serde_json::to_value(request).expect("serialize chart request");
        assert_eq!(request_value["kind"], "trending24_hours");
        assert_eq!(request_value["account"], "spoken-word");
        let entry_value = serde_json::to_value(entry).expect("serialize chart entry");
        assert_eq!(entry_value["rank"], 1);
        assert_eq!(entry_value["previous_rank"], 7);
        assert_eq!(entry_value["score"], 1_339_233);
        assert_eq!(entry_value["follower_count"], 76_488);
        assert_eq!(entry_value["creator"]["ref"], "netease:287921940");
    }

    #[test]
    fn podcast_episode_chart_keeps_rank_movement_and_episode_identity_explicit() {
        let mut request =
            PodcastEpisodeChartRequest::new(PodcastEpisodeChartKind::Trending24Hours, 30, 0);
        request.account = Some("spoken-word".to_owned());
        let episode = PodcastEpisode::new(
            ResourceRef::new(Platform::Netease, "3724712156").expect("valid episode reference"),
            "贪生Pass",
        );
        let entry = PodcastEpisodeChartEntry {
            rank: 1,
            previous_rank: Some(-1),
            score: Some(302_820),
            episode,
            extensions: Extensions::new(),
        };

        let request_value = serde_json::to_value(request).expect("serialize chart request");
        assert_eq!(request_value["kind"], "trending24_hours");
        assert_eq!(request_value["account"], "spoken-word");
        let entry_value = serde_json::to_value(entry).expect("serialize chart entry");
        assert_eq!(entry_value["rank"], 1);
        assert_eq!(entry_value["previous_rank"], -1);
        assert_eq!(entry_value["score"], 302_820);
        assert_eq!(entry_value["episode"]["ref"], "netease:3724712156");
    }

    #[test]
    fn podcast_episode_recommendations_keep_source_category_and_page_controls_distinct() {
        let mut request = PodcastEpisodeRecommendationRequest::new(
            PodcastEpisodeRecommendationSource::Category,
            10,
            20,
        );
        request.category_id = Some("2".to_owned());
        request.account = Some("spoken-word".to_owned());

        let value = serde_json::to_value(request).expect("serialize episode recommendation");
        assert_eq!(value["source"], "category");
        assert_eq!(value["category_id"], "2");
        assert_eq!(value["limit"], 10);
        assert_eq!(value["offset"], 20);
        assert_eq!(value["account"], "spoken-word");
        assert_eq!(
            PodcastEpisodeRecommendationSource::default(),
            PodcastEpisodeRecommendationSource::Personalized
        );
    }

    #[test]
    fn podcast_episode_playback_history_keeps_episode_time_and_device_explicit() {
        let entry = PodcastEpisodePlaybackHistoryEntry {
            episode: PodcastEpisode::new(
                ResourceRef::new(Platform::Netease, "2059302984").expect("valid episode reference"),
                "叽叽 - 静悄悄",
            ),
            played_at: Some("2024-01-01T00:00:00Z".to_owned()),
            device: Some(PlaybackDevice {
                operating_system: Some("android".to_owned()),
                name: Some("Android".to_owned()),
                icon_url: Some("https://example.test/android.png".to_owned()),
                extensions: Extensions::new(),
            }),
            extensions: Extensions::new(),
        };

        let value = serde_json::to_value(entry).expect("serialize episode playback history");
        assert_eq!(value["episode"]["ref"], "netease:2059302984");
        assert_eq!(value["played_at"], "2024-01-01T00:00:00Z");
        assert_eq!(value["device"]["operating_system"], "android");
        assert_eq!(value["device"]["name"], "Android");
    }

    #[test]
    fn audio_recognition_keeps_track_and_match_offset_typed() {
        let request = AudioRecognitionRequest {
            fingerprint: "encoded-fingerprint".to_owned(),
            duration_seconds: 6,
            account: None,
        };
        assert_eq!(request.duration_seconds, 6);

        let recognition = AudioRecognition {
            matches: vec![AudioRecognitionMatch {
                track: Track::new(
                    ResourceRef::new(Platform::Netease, "185809").expect("valid track reference"),
                    "晴天",
                ),
                start_time_ms: Some(1_500),
                extensions: Extensions::new(),
            }],
            query_id: Some("query-1".to_owned()),
            no_match_reason: None,
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(recognition).expect("serialize audio recognition");
        assert_eq!(value["matches"][0]["track"]["ref"], "netease:185809");
        assert_eq!(value["matches"][0]["start_time_ms"], 1_500);
        assert_eq!(value["query_id"], "query-1");
    }

    #[test]
    fn multi_match_preserves_section_order_and_resource_types() {
        let result = SearchMultiMatch {
            query: "海阔天空".to_owned(),
            requested_kind: SearchKind::Track,
            sections: vec![SearchMultiMatchSection {
                section: "artist".to_owned(),
                kind: Some(SearchKind::Artist),
                items: vec![SearchItem::Artist(Artist {
                    resource_ref: ResourceRef::new(Platform::Netease, "11127")
                        .expect("valid artist reference"),
                    platform: Platform::Netease,
                    id: "11127".to_owned(),
                    name: "Beyond".to_owned(),
                    aliases: Vec::new(),
                    description: String::new(),
                    biography_sections: Vec::new(),
                    avatar_url: None,
                    cover_url: None,
                    album_count: None,
                    track_count: None,
                    mv_count: None,
                    video_count: None,
                    identities: Vec::new(),
                    extensions: Extensions::new(),
                })],
                extensions: Extensions::new(),
            }],
            extensions: Extensions::new(),
        };

        let value = serde_json::to_value(result).expect("serialize multi-match search");
        assert_eq!(value["query"], "海阔天空");
        assert_eq!(value["requested_kind"], "track");
        assert_eq!(value["sections"][0]["section"], "artist");
        assert_eq!(value["sections"][0]["kind"], "artist");
        assert_eq!(value["sections"][0]["items"][0]["type"], "artist");
        assert_eq!(
            value["sections"][0]["items"][0]["data"]["ref"],
            "netease:11127"
        );
    }

    #[test]
    fn local_track_match_keeps_milliseconds_checksum_and_candidates_stable() {
        let request = LocalTrackMatchRequest {
            title: "富士山下".to_owned(),
            album: String::new(),
            artist: "陈奕迅".to_owned(),
            duration_ms: 259_210,
            md5: "bd708d006912a09d827f02e754cf8e56".to_owned(),
            account: Some("default".to_owned()),
        };
        assert_eq!(request.duration_ms, 259_210);

        let result = LocalTrackMatchResult {
            md5: request.md5,
            matches: vec![Track::new(
                ResourceRef::new(Platform::Netease, "65766").expect("valid track reference"),
                "富士山下",
            )],
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(result).expect("serialize local track match");
        assert_eq!(value["md5"], "bd708d006912a09d827f02e754cf8e56");
        assert_eq!(value["matches"][0]["ref"], "netease:65766");
    }

    #[test]
    fn membership_summary_keeps_unknown_status_fields_nullable() {
        let summary = MembershipSummary {
            user_ref: Some(
                ResourceRef::new(Platform::Netease, "32953014").expect("valid user reference"),
            ),
            level: Some(7),
            active: None,
            annual_count: Some(-1),
            expires_at: None,
            icon_url: Some("https://example.test/vip.png".to_owned()),
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(summary).expect("serialize membership summary");
        assert_eq!(value["user_ref"], "netease:32953014");
        assert_eq!(value["level"], 7);
        assert!(value["active"].is_null());
        assert_eq!(value["annual_count"], -1);
        assert!(value["expires_at"].is_null());
    }

    #[test]
    fn personal_fm_and_dislike_contracts_keep_mode_and_track_identity_explicit() {
        let recommendations = RecommendationRequest {
            limit: 10,
            offset: 0,
            account: Some("default".to_owned()),
            refresh: false,
            source: RecommendationSource::Personalized,
            area_id: Some(7),
        };
        let value =
            serde_json::to_value(recommendations).expect("serialize recommendation request");
        assert_eq!(value["source"], "personalized");
        assert_eq!(value["area_id"], 7);

        let videos = VideoRecommendationRequest {
            kind: VideoRecommendationKind::Exclusive,
            view: VideoRecommendationView::Catalog,
            limit: 60,
            offset: 120,
            account: Some("default".to_owned()),
        };
        let value = serde_json::to_value(videos).expect("serialize video recommendation request");
        assert_eq!(value["kind"], "exclusive");
        assert_eq!(value["view"], "catalog");
        assert_eq!(value["offset"], 120);

        let request = PersonalFmRequest {
            variant: PersonalFmVariant::Mode,
            mode: Some("SCENE_RCMD".to_owned()),
            sub_mode: Some("FOCUS".to_owned()),
            limit: 3,
            account: Some("default".to_owned()),
        };
        let value = serde_json::to_value(request).expect("serialize personal FM request");
        assert_eq!(value["variant"], "mode");
        assert_eq!(value["mode"], "SCENE_RCMD");
        assert_eq!(value["sub_mode"], "FOCUS");

        let result = RecommendationDislikeResult {
            track_ref: ResourceRef::new(Platform::Netease, "347230")
                .expect("valid track reference"),
            applied: true,
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(result).expect("serialize dislike result");
        assert_eq!(value["track_ref"], "netease:347230");
        assert_eq!(value["applied"], true);
    }

    #[test]
    fn anti_cheat_tokens_serialize_for_the_api_but_are_redacted_from_debug_output() {
        let token = AntiCheatToken {
            version: AntiCheatTokenVersion::V2,
            token: "temporary-secret-token".to_owned(),
            registered: true,
            refreshed: false,
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(&token).expect("serialize anti-cheat token");
        assert_eq!(value["version"], "v2");
        assert_eq!(value["token"], "temporary-secret-token");
        assert_eq!(value["registered"], true);
        let debug = format!("{token:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("temporary-secret-token"));
    }

    #[test]
    fn anonymous_sessions_serialize_for_compatibility_but_redact_debug_output() {
        let session = AnonymousSession {
            device_id: "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123".to_owned(),
            cookie: "MUSIC_A=anonymous-secret".to_owned(),
            registered: true,
            refreshed: false,
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(&session).expect("serialize anonymous session");
        assert_eq!(value["cookie"], "MUSIC_A=anonymous-secret");
        assert_eq!(value["registered"], true);
        let debug = format!("{session:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("anonymous-secret"));
    }

    #[test]
    fn listening_rights_ads_keep_the_request_uid_and_opaque_platform_payload() {
        let catalog = ListeningRightsAdCatalog {
            request_uid: Some("req-1".to_owned()),
            ads: vec![ListeningRightsAd {
                id: "400002_0".to_owned(),
                request_uid: Some("req-1".to_owned()),
                extensions: Extensions::from([("raw".to_owned(), serde_json::json!({"id": 1}))]),
            }],
            message: None,
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(catalog).expect("serialize listening-rights ads");
        assert_eq!(value["request_uid"], "req-1");
        assert_eq!(value["ads"][0]["id"], "400002_0");
        assert_eq!(value["ads"][0]["extensions"]["raw"]["id"], 1);
    }

    #[test]
    fn listening_rights_gain_keeps_reference_timestamps_and_optional_status_explicit() {
        let request = ListeningRightsGainRequest {
            request_uid: Some("req-1".to_owned()),
            exposure_time: Some(ListeningRightsTimestamp::Reference(
                "1784194692000".to_owned(),
            )),
            click_time: Some(ListeningRightsTimestamp::Milliseconds(1_784_194_692_001)),
            app_info: Some(serde_json::json!({"package": "music"})),
            ..ListeningRightsGainRequest::default()
        };
        let request = serde_json::to_value(request).expect("serialize listening-rights gain");
        assert_eq!(request["creative_type"], 2);
        assert_eq!(request["exposure_time"], "1784194692000");
        assert_eq!(request["click_time"], 1_784_194_692_001_u64);
        assert_eq!(request["type_ids"], serde_json::json!(["400002_0"]));

        let result = ListeningRightsGainResult {
            request_uid: Some("req-1".to_owned()),
            granted: None,
            platform_code: Some(200),
            message: None,
            extensions: Extensions::new(),
        };
        let result = serde_json::to_value(result).expect("serialize listening-rights result");
        assert!(result["granted"].is_null());
        assert_eq!(result["platform_code"], 200);
    }

    #[test]
    fn stream_contract_preserves_modern_quality_tiers_variants_and_batch_failures() {
        for (quality, name) in [
            (Quality::Higher, "higher"),
            (Quality::High, "high"),
            (Quality::Surround, "surround"),
            (Quality::Spatial, "spatial"),
            (Quality::Dolby, "dolby"),
            (Quality::Master, "master"),
        ] {
            assert_eq!(
                serde_json::to_value(quality).expect("serialize quality"),
                name
            );
        }

        let request = StreamRequest {
            quality: Quality::Spatial,
            variant: StreamVariant::Modern,
            bitrate: Some(999_000),
            account: Some("vip".to_owned()),
        };
        let value = serde_json::to_value(request).expect("serialize stream request");
        assert_eq!(value["quality"], "spatial");
        assert_eq!(value["variant"], "modern");
        assert_eq!(value["bitrate"], 999_000);

        let resolve = ResolveRequest {
            variant: StreamVariant::Legacy,
            ..ResolveRequest::default()
        };
        let value = serde_json::to_value(resolve).expect("serialize resolve request");
        assert_eq!(value["variant"], "legacy");

        let outcome = StreamOutcome {
            track_ref: ResourceRef::new(Platform::Netease, "1969519579")
                .expect("valid track reference"),
            status: ResolutionStatus::PermissionDenied,
            stream: None,
            error_code: Some(crate::ErrorCode::PermissionDenied),
            error: Some("not playable".to_owned()),
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(StreamBatch {
            outcomes: vec![outcome],
            extensions: Extensions::new(),
        })
        .expect("serialize stream batch");
        assert_eq!(value["outcomes"][0]["track_ref"], "netease:1969519579");
        assert_eq!(value["outcomes"][0]["status"], "permission_denied");
        assert_eq!(value["outcomes"][0]["error_code"], "permission_denied");
        assert!(value["outcomes"][0]["stream"].is_null());
    }

    #[test]
    fn download_contract_keeps_unavailable_results_and_exact_quality_explicit() {
        let download = MediaDownload {
            track_ref: ResourceRef::new(Platform::Netease, "2709812973")
                .expect("valid download track reference"),
            platform: Platform::Netease,
            available: false,
            url: None,
            headers: BTreeMap::new(),
            expires_at: None,
            format: None,
            codec: None,
            bitrate: Some(0),
            size: Some(0),
            duration_ms: Some(0),
            requested_quality: Quality::Spatial,
            actual_quality: Quality::Auto,
            platform_code: Some(-110),
            fee: Some(0),
            message: None,
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(download).expect("serialize media download");
        assert_eq!(value["ref"], "netease:2709812973");
        assert_eq!(value["available"], false);
        assert!(value["url"].is_null());
        assert_eq!(value["requested_quality"], "spatial");
        assert_eq!(value["actual_quality"], "auto");
        assert_eq!(value["platform_code"], -110);
    }

    #[test]
    fn image_upload_request_redacts_binary_data_and_result_is_stable() {
        let request = ImageUploadRequest {
            filename: "avatar.png".to_owned(),
            content_type: "image/png".to_owned(),
            data: b"private-image-data".to_vec(),
            image_size: Some(300),
            crop_x: Some(0),
            crop_y: Some(0),
            account: Some("default".to_owned()),
        };
        let debug = format!("{request:?}");
        assert!(debug.contains("data_len: 18"));
        assert!(!debug.contains("private-image-data"));

        let result = ImageUploadResult {
            url: Some("https://example.test/avatar.png".to_owned()),
            image_id: Some("109951168000000000".to_owned()),
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(result).expect("serialize image upload result");
        assert_eq!(value["url"], "https://example.test/avatar.png");
        assert_eq!(value["image_id"], "109951168000000000");
    }

    #[test]
    fn cloud_upload_models_redact_bytes_and_direct_upload_credentials() {
        let request = CloudUploadRequest {
            filename: "反方向的钟.flac".to_owned(),
            content_type: "audio/flac".to_owned(),
            data: b"private-audio-content".to_vec(),
            bitrate: CloudUploadRequest::DEFAULT_BITRATE,
            song_name: None,
            artist: None,
            album: None,
            account: Some("default".to_owned()),
        };
        let debug = format!("{request:?}");
        assert!(debug.contains("data_len: 21"));
        assert!(!debug.contains("private-audio-content"));

        let ticket_request = CloudUploadTicketRequest::new(
            "d02b8ab79d91c01167ba31e349fe5275",
            50_412_168,
            "最伟大的作品.flac",
        );
        assert_eq!(ticket_request.bitrate, 999_000);

        let ticket = CloudUploadTicket {
            upload_required: true,
            provisional_track_id: Some("123".to_owned()),
            resource_id: "resource-1".to_owned(),
            upload_method: "POST".to_owned(),
            upload_url: "https://upload.example.test/object".to_owned(),
            upload_headers: BTreeMap::from([
                ("Content-MD5".to_owned(), ticket_request.md5.clone()),
                ("x-nos-token".to_owned(), "secret-upload-token".to_owned()),
            ]),
            extensions: Extensions::new(),
        };
        let debug = format!("{ticket:?}");
        assert!(debug.contains("x-nos-token"));
        assert!(!debug.contains("secret-upload-token"));
        let value = serde_json::to_value(ticket).expect("serialize upload ticket");
        assert_eq!(value["upload_method"], "POST");
        assert_eq!(
            value["upload_headers"]["x-nos-token"],
            "secret-upload-token"
        );
    }

    #[test]
    fn cloud_library_contract_keeps_file_track_and_batch_semantics_distinct() {
        let cloud_ref =
            ResourceRef::new(Platform::Netease, "19723756").expect("cloud track reference");
        let matched_ref =
            ResourceRef::new(Platform::Netease, "185809").expect("matched track reference");
        let cloud_track = CloudTrack {
            cloud_track_ref: cloud_ref.clone(),
            track: Track::new(cloud_ref.clone(), "反方向的钟"),
            filename: Some("反方向的钟.flac".to_owned()),
            file_size: Some(50_412_168),
            file_type: Some("flac".to_owned()),
            bitrate: Some(1_652_000),
            md5: Some("d02b8ab79d91c01167ba31e349fe5275".to_owned()),
            added_at: Some("2026-07-17T00:00:00Z".to_owned()),
            matched_track_ref: Some(matched_ref.clone()),
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(&cloud_track).expect("serialize cloud track");
        assert_eq!(value["ref"], "netease:19723756");
        assert_eq!(value["track"]["ref"], "netease:19723756");
        assert_eq!(value["matched_track_ref"], "netease:185809");
        assert_eq!(value["bitrate"], 1_652_000);

        let references = vec![cloud_ref.clone(), cloud_ref.clone(), matched_ref];
        let mut detail = CloudTrackDetailRequest::new(references.clone());
        detail.account = Some("locker".to_owned());
        assert_eq!(detail.track_refs, references);
        assert_eq!(detail.account.as_deref(), Some("locker"));

        let delete = CloudTrackDeleteRequest::new(detail.track_refs.clone());
        let result = CloudTrackDeleteResult {
            track_refs: delete.track_refs,
            deleted: true,
            extensions: Extensions::new(),
        };
        assert_eq!(result.track_refs.len(), 3);
        assert_eq!(result.track_refs[0], result.track_refs[1]);
        assert!(result.deleted);
    }

    #[test]
    fn banner_keeps_client_and_target_semantics_typed() {
        let mut request = BannerListRequest::new(BannerClient::Pc);
        request.catalog = BannerCatalog::Podcast;
        assert_eq!(
            serde_json::to_value(request.catalog).expect("serialize banner catalog"),
            serde_json::json!("podcast")
        );
        assert_eq!(
            serde_json::to_value(request.client).expect("serialize banner client"),
            "pc"
        );

        let banner = Banner {
            id: Some("4862548".to_owned()),
            title: Some("播客精选".to_owned()),
            image_url: "https://example.test/banner.jpg".to_owned(),
            target_ref: Some(
                ResourceRef::new(Platform::Netease, "3402163617").expect("valid target reference"),
            ),
            target_kind: BannerTargetKind::PodcastEpisode,
            url: Some("orpheus://program/3402163617".to_owned()),
            exclusive: Some(false),
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(banner).expect("serialize banner");
        assert_eq!(value["target_ref"], "netease:3402163617");
        assert_eq!(value["target_kind"], "podcast_episode");
        assert_eq!(value["title"], "播客精选");
    }

    #[test]
    fn digital_album_chart_request_uses_public_api_defaults() {
        let request = DigitalAlbumChartRequest::new(20, 0);

        assert_eq!(request.period, DigitalAlbumChartPeriod::Daily);
        assert_eq!(request.kind, DigitalAlbumChartKind::Album);
        assert_eq!(request.year, None);
        assert_eq!(
            serde_json::to_value(request.period).expect("serialize period"),
            "daily"
        );
        assert_eq!(
            serde_json::to_value(request.kind).expect("serialize kind"),
            "album"
        );
    }

    #[test]
    fn chart_contract_preserves_catalog_variants_and_rank_snapshots() {
        let request = ChartCatalogRequest::new(ChartCatalogView::Modern);
        assert_eq!(request.view, ChartCatalogView::Modern);
        assert_eq!(request.account, None);

        let chart = Chart {
            resource_ref: Some(
                ResourceRef::new(Platform::Netease, "19723756").expect("valid chart reference"),
            ),
            platform: Platform::Netease,
            id: Some("19723756".to_owned()),
            name: "飙升榜".to_owned(),
            description: "每天热度上升最快的歌曲".to_owned(),
            cover_url: Some("https://example.test/chart.jpg".to_owned()),
            update_frequency: Some("每天更新".to_owned()),
            updated_at_ms: Some(1_784_170_805_374),
            track_count: Some(100),
            play_count: None,
            subscribed: None,
            playable: Some(true),
            target_kind: Some("playlist".to_owned()),
            target_url: None,
            previews: vec![ChartTrackPreview {
                rank: Some(1),
                previous_rank: Some(5),
                rank_change: Some(4),
                track_ref: Some(
                    ResourceRef::new(Platform::Netease, "3404238777")
                        .expect("valid track reference"),
                ),
                name: "周旋".to_owned(),
                byline: Some("王以太/艾热 AIR".to_owned()),
                cover_url: None,
                extensions: Extensions::new(),
            }],
            extensions: Extensions::new(),
        };
        let catalog = ChartCatalog {
            platform: Platform::Netease,
            view: ChartCatalogView::Modern,
            groups: vec![ChartGroup {
                code: Some("OFFICIAL".to_owned()),
                name: "官方榜".to_owned(),
                display_type: Some("HORIZONTAL".to_owned()),
                target_url: None,
                charts: vec![chart],
                extensions: Extensions::new(),
            }],
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(catalog).expect("serialize chart catalog");
        assert_eq!(value["view"], "modern");
        assert_eq!(value["groups"][0]["code"], "OFFICIAL");
        assert_eq!(value["groups"][0]["charts"][0]["ref"], "netease:19723756");
        assert_eq!(
            value["groups"][0]["charts"][0]["previews"][0]["rank_change"],
            4
        );
    }

    #[test]
    fn artist_chart_contract_keeps_area_and_previous_rank_explicit() {
        let request = ArtistChartRequest::new(ArtistChartArea::Western);
        assert_eq!(request.area, ArtistChartArea::Western);
        assert_eq!(
            serde_json::to_value(request.area).expect("serialize artist chart area"),
            "western"
        );

        let artist = Artist {
            resource_ref: ResourceRef::new(Platform::Netease, "3684")
                .expect("valid artist reference"),
            platform: Platform::Netease,
            id: "3684".to_owned(),
            name: "林俊杰".to_owned(),
            aliases: vec!["JJ Lin".to_owned()],
            description: String::new(),
            biography_sections: Vec::new(),
            avatar_url: None,
            cover_url: None,
            album_count: Some(73),
            track_count: Some(598),
            mv_count: None,
            video_count: None,
            identities: Vec::new(),
            extensions: Extensions::new(),
        };
        let chart = ArtistChart {
            platform: Platform::Netease,
            area: ArtistChartArea::Western,
            updated_at_ms: Some(1_784_170_805_374),
            entries: vec![ArtistChartEntry {
                rank: 1,
                previous_rank: Some(3),
                rank_change: Some(2),
                score: Some(63_562_038),
                artist,
                extensions: Extensions::new(),
            }],
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(chart).expect("serialize artist chart");
        assert_eq!(value["area"], "western");
        assert_eq!(value["entries"][0]["previous_rank"], 3);
        assert_eq!(value["entries"][0]["artist"]["ref"], "netease:3684");
    }

    #[test]
    fn dimension_chart_snapshot_keeps_dimension_and_rank_semantics() {
        let request = DimensionChartRequest::new("CITY_SONG_CHART", "110000", "CITY");
        assert_eq!(request.chart_code, "CITY_SONG_CHART");
        assert_eq!(request.target_id, "110000");
        assert_eq!(request.target_type, "CITY");

        let chart_ref = ResourceRef::new(Platform::Netease, "CITY_SONG_CHART#110000@CITY#")
            .expect("valid chart reference");
        let snapshot = DimensionChartTrackSnapshot {
            chart_ref,
            chart_code: "CITY_SONG_CHART".to_owned(),
            target_id: "110000".to_owned(),
            target_type: "CITY".to_owned(),
            entries: vec![DimensionChartTrackEntry {
                rank: 1,
                previous_rank: Some(4),
                rank_change: Some(3),
                track: Track::new(
                    ResourceRef::new(Platform::Netease, "210049").expect("valid track reference"),
                    "布拉格广场",
                ),
                reason: Some("本周热度上升".to_owned()),
                reason_id: Some("city-popular".to_owned()),
                score: None,
                ratio: None,
                collected: Some(false),
                extensions: Extensions::new(),
            }],
            period_label: Some("本周".to_owned()),
            groups: BTreeMap::from([("CITY".to_owned(), "城市".to_owned())]),
            extensions: Extensions::new(),
        };

        let value = serde_json::to_value(snapshot).expect("serialize chart snapshot");
        assert_eq!(value["chart_ref"], "netease:CITY_SONG_CHART#110000@CITY#");
        assert_eq!(value["entries"][0]["previous_rank"], 4);
        assert_eq!(value["entries"][0]["rank_change"], 3);
        assert_eq!(value["entries"][0]["track"]["ref"], "netease:210049");
    }

    #[test]
    fn subscription_result_serializes_the_resource_reference() {
        let result = SubscriptionResult {
            resource_ref: ResourceRef::new(Platform::Netease, "32311")
                .expect("valid album reference"),
            subscribed: true,
            extensions: Extensions::new(),
        };

        let value = serde_json::to_value(result).expect("serialize subscription result");
        assert_eq!(value["resource_ref"], "netease:32311");
        assert_eq!(value["subscribed"], true);
    }

    #[test]
    fn track_availability_separates_requested_and_actual_bitrate() {
        let request = TrackAvailabilityRequest::default();
        assert_eq!(request.bitrate, 999_000);

        let availability = TrackAvailability {
            track_ref: ResourceRef::new(Platform::Netease, "1969519579")
                .expect("valid track reference"),
            playable: true,
            requested_bitrate: request.bitrate,
            actual_bitrate: Some(320_000),
            platform_code: Some(200),
            message: "ok".to_owned(),
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(availability).expect("serialize track availability");
        assert_eq!(value["track_ref"], "netease:1969519579");
        assert_eq!(value["playable"], true);
        assert_eq!(value["requested_bitrate"], 999_000);
        assert_eq!(value["actual_bitrate"], 320_000);
    }

    #[test]
    fn artist_serializes_stable_identity_and_biography_fields() {
        let artist = Artist {
            resource_ref: ResourceRef::new(Platform::Netease, "6452")
                .expect("valid artist reference"),
            platform: Platform::Netease,
            id: "6452".to_owned(),
            name: "周杰伦".to_owned(),
            aliases: vec!["Jay Chou".to_owned(), "周董".to_owned()],
            description: "歌手、词曲作者与制作人。".to_owned(),
            biography_sections: vec![ArtistBiographySection {
                title: "人物简介".to_owned(),
                text: "跨平台统一歌手传记。".to_owned(),
            }],
            avatar_url: Some("https://example.test/avatar.jpg".to_owned()),
            cover_url: Some("https://example.test/cover.jpg".to_owned()),
            album_count: Some(44),
            track_count: Some(568),
            mv_count: Some(9),
            video_count: Some(8),
            identities: vec!["作曲".to_owned()],
            extensions: Extensions::new(),
        };

        let value = serde_json::to_value(&artist).expect("serialize artist");
        assert_eq!(value["ref"], "netease:6452");
        assert_eq!(value["platform"], "netease");
        assert_eq!(value["biography_sections"][0]["title"], "人物简介");
        assert_eq!(value["track_count"], 568);

        let overview = ArtistOverview {
            artist,
            featured_tracks: vec![Track::new(
                ResourceRef::new(Platform::Netease, "210049").expect("valid track reference"),
                "布拉格广场",
            )],
            has_more_tracks: true,
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(overview).expect("serialize artist overview");
        assert_eq!(value["artist"]["ref"], "netease:6452");
        assert_eq!(value["featured_tracks"][0]["ref"], "netease:210049");
        assert_eq!(value["has_more_tracks"], true);
    }

    #[test]
    fn artist_list_request_uses_cross_platform_filter_defaults() {
        let request = ArtistListRequest::new(30, 0);

        assert_eq!(request.category, ArtistCategory::All);
        assert_eq!(request.area, ArtistArea::All);
        assert_eq!(request.initial, None);
        assert_eq!(
            serde_json::to_value(ArtistCategory::Group).expect("serialize artist category"),
            "group"
        );
        assert_eq!(
            serde_json::to_value(ArtistArea::Western).expect("serialize artist area"),
            "western"
        );
    }

    #[test]
    fn video_serializes_reusable_creator_and_media_metadata() {
        let video = Video {
            resource_ref: ResourceRef::new(Platform::Netease, "22695250")
                .expect("valid video reference"),
            platform: Platform::Netease,
            id: "22695250".to_owned(),
            title: "任性 (5525 Live版)".to_owned(),
            creators: vec![CreatorSummary {
                resource_ref: Some(
                    ResourceRef::new(Platform::Netease, "6452").expect("valid creator reference"),
                ),
                name: "周杰伦".to_owned(),
                avatar_url: None,
            }],
            description: String::new(),
            cover_url: Some("https://example.test/cover.jpg".to_owned()),
            duration_ms: Some(266_000),
            published_at: Some("2025-02-23".to_owned()),
            play_count: Some(100_726),
            subscribed: Some(false),
            extensions: Extensions::new(),
        };

        let value = serde_json::to_value(video).expect("serialize video");
        assert_eq!(value["ref"], "netease:22695250");
        assert_eq!(value["creators"][0]["ref"], "netease:6452");
        assert_eq!(value["duration_ms"], 266_000);
        assert_eq!(value["subscribed"], false);
    }

    #[test]
    fn video_detail_stats_and_stream_contracts_keep_mv_semantics_explicit() {
        let reference =
            ResourceRef::new(Platform::Netease, "22695250").expect("valid video reference");
        let detail_request = VideoDetailRequest::new(VideoResourceKind::Mv);
        assert_eq!(detail_request.kind, VideoResourceKind::Mv);

        let stream_request = VideoStreamRequest::new(
            VideoResourceKind::Mv,
            VideoStreamRequest::DEFAULT_RESOLUTION,
        );
        assert_eq!(stream_request.resolution, 1080);
        let stream = VideoStream {
            video_ref: reference.clone(),
            platform: Platform::Netease,
            available: true,
            url: Some("https://example.test/video.mp4".to_owned()),
            backup_urls: Vec::new(),
            headers: BTreeMap::new(),
            expires_at: None,
            format: Some("mp4".to_owned()),
            codec: None,
            width: None,
            height: Some(1080),
            size: Some(177_950_120),
            duration_ms: Some(266_000),
            requested_resolution: 1080,
            actual_resolution: Some(1080),
            platform_code: Some(200),
            fee: Some(0),
            message: None,
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(stream).expect("serialize video stream");
        assert_eq!(value["video_ref"], "netease:22695250");
        assert_eq!(value["available"], true);
        assert_eq!(value["requested_resolution"], 1080);
        assert_eq!(value["actual_resolution"], 1080);

        let stats = VideoStats {
            video_ref: reference,
            kind: VideoResourceKind::Mv,
            liked: Some(false),
            like_count: Some(4_662),
            comment_count: Some(675),
            share_count: Some(1_399),
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(stats).expect("serialize video stats");
        assert_eq!(value["kind"], "mv");
        assert_eq!(value["like_count"], 4_662);
    }

    #[test]
    fn playlist_write_contracts_separate_metadata_tracks_and_account_order() {
        let mut create = PlaylistCreateRequest::new("跨平台收藏");
        create.visibility = PlaylistVisibility::Private;
        create.kind = PlaylistKind::Shared;
        create.account = Some("personal".to_owned());
        let value = serde_json::to_value(create).expect("serialize playlist create request");
        assert_eq!(value["name"], "跨平台收藏");
        assert_eq!(value["visibility"], "private");
        assert_eq!(value["kind"], "shared");

        let mut update = PlaylistUpdateRequest::new();
        update.description = Some(String::new());
        update.tags = Some(vec!["华语".to_owned(), "现场".to_owned()]);
        update.variant = PlaylistMetadataUpdateVariant::Individual;
        let value = serde_json::to_value(update).expect("serialize playlist update request");
        assert_eq!(value["description"], "");
        assert_eq!(value["tags"], serde_json::json!(["华语", "现场"]));
        assert_eq!(value["variant"], "individual");

        let playlist_ref =
            ResourceRef::new(Platform::Netease, "987654").expect("valid playlist reference");
        let track_refs = vec![
            ResourceRef::new(Platform::Netease, "185809").expect("valid track reference"),
            ResourceRef::new(Platform::Netease, "1974443814").expect("valid track reference"),
        ];
        let videos = PlaylistItemMutationRequest::new(track_refs.clone(), PlaylistItemKind::Video);
        let value = serde_json::to_value(videos).expect("serialize playlist item request");
        assert_eq!(
            value["item_refs"],
            serde_json::json!(["netease:185809", "netease:1974443814"])
        );
        assert_eq!(value["kind"], "video");

        let result = PlaylistItemMutationResult {
            playlist_ref: playlist_ref.clone(),
            item_refs: track_refs.clone(),
            kind: PlaylistItemKind::Track,
            action: PlaylistItemMutationAction::Add,
            snapshot_id: Some("snapshot-1".to_owned()),
            cloud_track_count: Some(0),
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(result).expect("serialize playlist item result");
        assert_eq!(value["playlist_ref"], "netease:987654");
        assert_eq!(value["kind"], "track");
        assert_eq!(value["action"], "add");

        let track_order = PlaylistTrackOrderRequest {
            track_refs,
            account: Some("personal".to_owned()),
        };
        let value = serde_json::to_value(track_order).expect("serialize playlist track order");
        assert_eq!(value["track_refs"][0], "netease:185809");

        let order = PlaylistOrderRequest {
            playlist_refs: vec![playlist_ref],
            account: Some("personal".to_owned()),
        };
        let value = serde_json::to_value(order).expect("serialize playlist order request");
        assert_eq!(value["playlist_refs"][0], "netease:987654");

        let delete = PlaylistDeleteRequest {
            playlist_refs: vec![
                ResourceRef::new(Platform::Netease, "987654").expect("valid playlist reference"),
                ResourceRef::new(Platform::Netease, "987654")
                    .expect("duplicate playlist reference"),
            ],
            account: Some("personal".to_owned()),
        };
        let value = serde_json::to_value(delete).expect("serialize playlist delete request");
        assert_eq!(
            value["playlist_refs"],
            serde_json::json!(["netease:987654", "netease:987654"])
        );
    }

    #[test]
    fn uni_playlist_keeps_local_identity_counts_and_millisecond_timestamps_explicit() {
        let reference =
            ResourceRef::new(Platform::Uni, "pl_01abcdefghijklmnop").expect("valid Uni reference");
        let playlist = UniPlaylist::new(
            reference,
            "Cross-platform favorites",
            "Mixed sources in exact order",
            1_753_137_600_000,
        );
        let value = serde_json::to_value(playlist).expect("serialize Uni Playlist");
        assert_eq!(value["ref"], "uni:pl_01abcdefghijklmnop");
        assert_eq!(value["platform"], "uni");
        assert_eq!(value["item_count"], 0);
        assert_eq!(value["created_at_ms"], 1_753_137_600_000_u64);
        assert_eq!(value["updated_at_ms"], value["created_at_ms"]);

        let request = UniPlaylistCreateRequest::new("Imported favorites");
        assert!(request.description.is_empty());

        let input = UniPlaylistItemInput {
            resource_ref: ResourceRef::new(Platform::Netease, "185809")
                .expect("valid source reference"),
            kind: UniPlaylistItemKind::Track,
        };
        let request = UniPlaylistItemAddRequest {
            items: vec![input],
            accounts: BTreeMap::from([(Platform::Netease, "vip".to_owned())]),
        };
        let value = serde_json::to_value(request).expect("serialize Uni Playlist item request");
        assert_eq!(value["items"][0]["ref"], "netease:185809");
        assert_eq!(value["items"][0]["kind"], "track");
        assert_eq!(value["accounts"]["netease"], "vip");

        let import = UniPlaylistImportRequest {
            name: None,
            description: None,
            sources: vec![UniPlaylistImportSourceRequest {
                playlist_ref: ResourceRef::new(Platform::Bilibili, "3629748")
                    .expect("valid collection reference"),
                source_type: "season".to_owned(),
                account: None,
            }],
        };
        let value = serde_json::to_value(import).expect("serialize Uni Playlist import request");
        assert_eq!(value["sources"][0]["ref"], "bilibili:3629748");
        assert_eq!(value["sources"][0]["type"], "season");
        assert_eq!(
            serde_json::to_value(UniPlaylistItemKind::RadioStation)
                .expect("serialize radio station item kind"),
            "radio_station"
        );
    }

    #[test]
    fn artist_updates_request_keeps_account_and_timestamp_cursor_separate() {
        let mut request = ArtistUpdatesRequest::new(20);
        request.before_ms = Some(1_720_000_000_000);
        request.account = Some("personal".to_owned());

        let value = serde_json::to_value(request).expect("serialize artist updates request");
        assert_eq!(value["limit"], 20);
        assert_eq!(value["before_ms"], 1_720_000_000_000_u64);
        assert_eq!(value["account"], "personal");
    }

    #[test]
    fn artist_track_list_request_defaults_to_hot_order() {
        let mut request = ArtistTrackListRequest::new(100, 20);
        request.account = Some("personal".to_owned());

        assert_eq!(request.order, ArtistTrackOrder::Hot);
        let value = serde_json::to_value(request).expect("serialize artist track list request");
        assert_eq!(value["limit"], 100);
        assert_eq!(value["offset"], 20);
        assert_eq!(value["account"], "personal");
        assert_eq!(value["order"], "hot");
        assert_eq!(
            serde_json::to_value(ArtistTrackOrder::Time).expect("serialize artist track order"),
            "time"
        );
    }

    #[test]
    fn artist_works_request_and_update_keep_mixed_resources_typed() {
        let mut request = ArtistWorksRequest::new(10);
        request.before_ms = Some(1_720_000_000_000);
        request.first_request = false;
        request.account = Some("personal".to_owned());
        let request_value = serde_json::to_value(request).expect("serialize artist works request");
        assert_eq!(request_value["source_type"], 1);
        assert_eq!(request_value["first_request"], false);

        let update = ArtistWorkUpdate {
            source_type: 1,
            kind: ArtistWorkKind::Track,
            published_at: Some("2024-07-03".to_owned()),
            artist: Some(ArtistSummary {
                resource_ref: Some(
                    ResourceRef::new(Platform::Netease, "6452").expect("valid artist reference"),
                ),
                name: "周杰伦".to_owned(),
            }),
            title: Some("新专辑".to_owned()),
            cover_url: None,
            tracks: vec![Track::new(
                ResourceRef::new(Platform::Netease, "2099001").expect("valid track reference"),
                "新歌",
            )],
            videos: Vec::new(),
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(update).expect("serialize artist work update");
        assert_eq!(value["kind"], "track");
        assert_eq!(value["tracks"][0]["ref"], "netease:2099001");
        assert_eq!(
            serde_json::to_value(ArtistWorkKind::Mixed).expect("serialize mixed artist work"),
            "mixed"
        );
    }

    #[test]
    fn artist_stats_keep_provider_categories_without_guessing_their_meaning() {
        let stats = ArtistStats {
            artist_ref: ResourceRef::new(Platform::Netease, "6452")
                .expect("valid artist reference"),
            followed: Some(false),
            follower_count: Some(13_704_928),
            video_counts: vec![ArtistContentCount {
                category: Some("0".to_owned()),
                count: 9,
                extensions: Extensions::new(),
            }],
            online_concert_count: Some(0),
            extensions: Extensions::new(),
        };

        let value = serde_json::to_value(stats).expect("serialize artist stats");
        assert_eq!(value["artist_ref"], "netease:6452");
        assert_eq!(value["followed"], false);
        assert_eq!(value["follower_count"], 13_704_928);
        assert_eq!(value["video_counts"][0]["category"], "0");
        assert_eq!(value["video_counts"][0]["count"], 9);
    }

    #[test]
    fn user_serializes_a_cross_platform_identity_and_relationship_state() {
        let user = User {
            resource_ref: ResourceRef::new(Platform::Netease, "6298206519")
                .expect("valid user reference"),
            platform: Platform::Netease,
            id: "6298206519".to_owned(),
            name: "轻手揍人丸".to_owned(),
            avatar_url: Some("https://example.test/avatar.jpg".to_owned()),
            signature: Some("111".to_owned()),
            followed: Some(false),
            mutual: Some(false),
            extensions: Extensions::new(),
        };

        let value = serde_json::to_value(user).expect("serialize user");
        assert_eq!(value["ref"], "netease:6298206519");
        assert_eq!(value["name"], "轻手揍人丸");
        assert_eq!(value["followed"], false);
        assert_eq!(value["mutual"], false);
    }

    #[test]
    fn catalog_search_items_keep_resource_types_and_opaque_platform_data_explicit() {
        let track = SearchItem::Track(Track::new(
            ResourceRef::new(Platform::Netease, "185809").expect("valid track reference"),
            "反方向的钟",
        ));
        let value = serde_json::to_value(track).expect("serialize typed search item");
        assert_eq!(value["type"], "track");
        assert_eq!(value["data"]["ref"], "netease:185809");

        let podcast = SearchItem::Podcast(Podcast::new(
            ResourceRef::new(Platform::Netease, "336355127").expect("valid podcast reference"),
            "代码时间",
        ));
        let value = serde_json::to_value(podcast).expect("serialize podcast search item");
        assert_eq!(value["type"], "podcast");
        assert_eq!(value["data"]["ref"], "netease:336355127");

        let mut extensions = Extensions::new();
        extensions.insert(
            "response".to_owned(),
            serde_json::json!({ "resource": "voice" }),
        );
        let opaque = SearchItem::Opaque(SearchOpaqueItem {
            platform: Platform::Netease,
            kind: "voice".to_owned(),
            id: Some("34001".to_owned()),
            title: Some("声音节目".to_owned()),
            extensions,
        });
        let value = serde_json::to_value(opaque).expect("serialize opaque search item");
        assert_eq!(value["type"], "opaque");
        assert_eq!(value["data"]["platform"], "netease");
        assert_eq!(value["data"]["kind"], "voice");
        assert_eq!(value["data"]["extensions"]["response"]["resource"], "voice");
    }

    #[test]
    fn search_kinds_cover_every_reference_cloudsearch_branch() {
        let kinds = [
            SearchKind::Track,
            SearchKind::Album,
            SearchKind::Artist,
            SearchKind::Playlist,
            SearchKind::User,
            SearchKind::Mv,
            SearchKind::Lyric,
            SearchKind::RadioStation,
            SearchKind::Podcast,
            SearchKind::Video,
            SearchKind::Mixed,
            SearchKind::Voice,
        ];
        let values = kinds
            .into_iter()
            .map(|kind| serde_json::to_value(kind).expect("serialize search kind"))
            .collect::<Vec<_>>();
        assert_eq!(
            values,
            vec![
                "track",
                "album",
                "artist",
                "playlist",
                "user",
                "mv",
                "lyric",
                "radio_station",
                "podcast",
                "video",
                "mixed",
                "voice",
            ]
        );
    }

    #[test]
    fn search_variants_keep_default_legacy_and_cloud_backends_explicit() {
        let values = [
            SearchVariant::Default,
            SearchVariant::Legacy,
            SearchVariant::Cloud,
        ]
        .into_iter()
        .map(|variant| serde_json::to_value(variant).expect("serialize search variant"))
        .collect::<Vec<_>>();
        assert_eq!(values, vec!["default", "legacy", "cloud"]);

        let query = SearchQuery::tracks("反方向的钟", 30, 0);
        assert_eq!(query.variant, SearchVariant::Default);
    }

    #[test]
    fn default_search_keywords_separate_query_and_display_text() {
        let prompt = SearchDefaultKeyword {
            keyword: "周旋".to_owned(),
            display_text: "🔥周旋 最近很火哦".to_owned(),
            kind: Some(SearchKind::Track),
            image_url: None,
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(prompt).expect("serialize default search keyword");
        assert_eq!(value["keyword"], "周旋");
        assert_eq!(value["display_text"], "🔥周旋 最近很火哦");
        assert_eq!(value["kind"], "track");
        assert!(value["image_url"].is_null());
    }

    #[test]
    fn trending_searches_keep_rank_detail_and_optional_rich_fields_explicit() {
        let list = SearchTrendingList {
            detail: SearchTrendingDetail::Full,
            entries: vec![SearchTrendingEntry {
                rank: 1,
                keyword: "薛之谦".to_owned(),
                description: Some("热门搜索".to_owned()),
                score: Some(107_509),
                icon_type: Some(4),
                icon_url: Some("https://example.test/hot.png".to_owned()),
                target_url: None,
                extensions: Extensions::new(),
            }],
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(list).expect("serialize trending searches");
        assert_eq!(value["detail"], "full");
        assert_eq!(value["entries"][0]["rank"], 1);
        assert_eq!(value["entries"][0]["keyword"], "薛之谦");
        assert_eq!(value["entries"][0]["score"], 107_509);
        assert_eq!(value["entries"][0]["icon_type"], 4);
    }

    #[test]
    fn search_suggestions_keep_client_resources_and_recommendations_separate() {
        let list = SearchSuggestionList {
            query: "海阔天空".to_owned(),
            client: SearchSuggestionClient::Web,
            suggestions: vec![SearchSuggestion {
                keyword: "海阔天空".to_owned(),
                kind: Some(SearchKind::Track),
                display_text: None,
                icon_url: None,
                resource: Some(SearchItem::Track(Track::new(
                    ResourceRef::new(Platform::Netease, "1357375695")
                        .expect("valid suggestion track"),
                    "海阔天空",
                ))),
                extensions: Extensions::new(),
            }],
            recommendations: Vec::new(),
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(list).expect("serialize search suggestions");
        assert_eq!(value["client"], "web");
        assert_eq!(value["suggestions"][0]["kind"], "track");
        assert_eq!(value["suggestions"][0]["resource"]["type"], "track");
        assert_eq!(
            value["suggestions"][0]["resource"]["data"]["ref"],
            "netease:1357375695"
        );
        assert_eq!(value["recommendations"], serde_json::json!([]));
    }

    #[test]
    fn page_metadata_only_serializes_extensions_when_present() {
        let mut metadata = PageMeta {
            limit: 25,
            offset: 0,
            total: Some(2),
            next_offset: None,
            has_more: false,
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(&metadata).expect("serialize empty metadata");
        assert!(value.get("extensions").is_none());

        metadata
            .extensions
            .insert("paid_count".to_owned(), Value::from(1));
        let value = serde_json::to_value(metadata).expect("serialize extended metadata");
        assert_eq!(value["extensions"]["paid_count"], 1);
    }

    #[test]
    fn comment_mutations_keep_platform_target_and_action_explicit() {
        let target = CommentTarget::new(
            ResourceRef::new(Platform::Netease, "185809").expect("valid track reference"),
            CommentTargetKind::Track,
        );
        let request = CommentWriteRequest {
            target: target.clone(),
            content: "统一评论".to_owned(),
            reply_to: Some("1438569889".to_owned()),
            account: Some("personal".to_owned()),
        };
        let request = serde_json::to_value(request).expect("serialize comment write request");
        assert_eq!(request["target"]["ref"], "netease:185809");
        assert_eq!(request["target"]["kind"], "track");
        assert_eq!(request["reply_to"], "1438569889");

        let result = CommentMutationResult {
            target,
            comment_id: Some("1535550516319".to_owned()),
            action: CommentMutationAction::Reply,
            extensions: Extensions::new(),
        };
        let result = serde_json::to_value(result).expect("serialize comment mutation result");
        assert_eq!(result["action"], "reply");
        assert_eq!(result["comment_id"], "1535550516319");
    }

    #[test]
    fn comment_pages_keep_lists_sorting_and_reply_relationships_typed() {
        let target = CommentTarget::new(
            ResourceRef::new(Platform::Netease, "185809").expect("valid track reference"),
            CommentTargetKind::Track,
        );
        let mut request = CommentListRequest::new(target.clone(), 20);
        request.sort = Some(CommentSort::Time);
        request.cursor = Some("1582035919432".to_owned());
        assert_eq!(request.view, CommentListView::All);
        assert!(request.include_replies);

        let comment = Comment {
            platform: Platform::Netease,
            id: "3160990055".to_owned(),
            content: "看不见你的笑，我怎么睡得着".to_owned(),
            author: None,
            created_at_ms: Some(1_582_035_919_432),
            created_at_text: Some("2020-02-18".to_owned()),
            liked: Some(false),
            like_count: Some(5_646),
            parent_comment_id: None,
            reply_count: None,
            replied_to: vec![CommentReplyReference {
                comment_id: Some("100".to_owned()),
                content: "原评论".to_owned(),
                author: None,
                extensions: Extensions::new(),
            }],
            ip_location: Some("上海".to_owned()),
            extensions: Extensions::new(),
        };
        let page = CommentPage {
            target,
            comments: vec![comment],
            hot_comments: Vec::new(),
            top_comments: Vec::new(),
            current_comment: None,
            pagination: PageMeta {
                limit: 20,
                offset: 0,
                total: Some(68_334),
                next_offset: Some(20),
                has_more: true,
                extensions: Extensions::new(),
            },
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(page).expect("serialize comment page");
        assert_eq!(value["target"]["ref"], "netease:185809");
        assert_eq!(value["comments"][0]["id"], "3160990055");
        assert_eq!(value["comments"][0]["replied_to"][0]["comment_id"], "100");
        assert_eq!(value["pagination"]["total"], 68_334);
        assert_eq!(
            serde_json::to_value(request.sort).expect("serialize comment sort"),
            serde_json::json!("time")
        );
    }

    #[test]
    fn comment_reaction_pages_keep_users_kind_and_dual_cursors_explicit() {
        let target = CommentTarget::new(
            ResourceRef::new(Platform::Netease, "863481066").expect("valid track reference"),
            CommentTargetKind::Track,
        );
        let target_user_ref =
            ResourceRef::new(Platform::Netease, "285516405").expect("valid user reference");
        let mut request = CommentReactionListRequest::new(
            target.clone(),
            "1167145843".to_owned(),
            target_user_ref.clone(),
            CommentReactionKind::Hug,
            2,
        );
        request.cursor = Some("04-八月-2020 17:46:25:000".to_owned());
        request.id_cursor = Some("362576849".to_owned());

        let user = User {
            resource_ref: ResourceRef::new(Platform::Netease, "2121989064")
                .expect("valid reacting user reference"),
            platform: Platform::Netease,
            id: "2121989064".to_owned(),
            name: "清梦初仄".to_owned(),
            avatar_url: Some("https://example.test/avatar.jpg".to_owned()),
            signature: None,
            followed: Some(false),
            mutual: None,
            extensions: Extensions::new(),
        };
        let page = CommentReactionPage {
            target,
            comment_id: request.comment_id.clone(),
            target_user_ref,
            kind: CommentReactionKind::Hug,
            reactions: vec![CommentReaction {
                kind: CommentReactionKind::Hug,
                user,
                content: Some("给了 Puddin_of_Harley_Quinn 一个抱抱".to_owned()),
                extensions: Extensions::new(),
            }],
            current_comment: None,
            pagination: PageMeta {
                limit: 2,
                offset: 0,
                total: Some(150),
                next_offset: Some(2),
                has_more: true,
                extensions: Extensions::from([
                    ("next_cursor".to_owned(), serde_json::json!("cursor")),
                    ("next_id_cursor".to_owned(), serde_json::json!(362576849)),
                ]),
            },
            extensions: Extensions::new(),
        };

        let request = serde_json::to_value(request).expect("serialize reaction request");
        assert_eq!(request["kind"], "hug");
        assert_eq!(request["target_user_ref"], "netease:285516405");
        assert_eq!(request["id_cursor"], "362576849");

        let page = serde_json::to_value(page).expect("serialize reaction page");
        assert_eq!(page["kind"], "hug");
        assert_eq!(page["reactions"][0]["user"]["ref"], "netease:2121989064");
        assert_eq!(page["pagination"]["total"], 150);
    }

    #[test]
    fn comment_thread_stats_keep_requested_and_canonical_targets_distinct() {
        let requested = ResourceRef::new(Platform::Netease, "89ADDE33C0AAE8EC14B99F6750DB954D")
            .expect("valid video reference");
        let canonical =
            ResourceRef::new(Platform::Netease, "2335163").expect("valid canonical reference");
        let batch = CommentThreadStatsBatch {
            kind: CommentTargetKind::Video,
            requested_refs: vec![requested.clone()],
            stats: vec![CommentThreadStats {
                target: CommentTarget::new(canonical, CommentTargetKind::Video),
                requested_ref: Some(requested),
                liked: Some(false),
                like_count: Some(20),
                comment_count: Some(1_123),
                comment_count_text: Some("1000+".to_owned()),
                share_count: Some(30),
                comment_upgraded: Some(false),
                musician_comment_count: Some(0),
                latest_liked_users: Vec::new(),
                comments: Vec::new(),
                extensions: Extensions::new(),
            }],
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(batch).expect("serialize comment stats batch");
        assert_eq!(
            value["requested_refs"][0],
            "netease:89ADDE33C0AAE8EC14B99F6750DB954D"
        );
        assert_eq!(value["stats"][0]["target"]["ref"], "netease:2335163");
        assert_eq!(
            value["stats"][0]["requested_ref"],
            "netease:89ADDE33C0AAE8EC14B99F6750DB954D"
        );
        assert_eq!(value["stats"][0]["comment_count"], 1_123);
    }

    #[test]
    fn comment_reaction_mutations_keep_action_target_and_optional_user_explicit() {
        let request = CommentReactionMutationRequest {
            target: CommentTarget::new(
                ResourceRef::new(Platform::Netease, "185809").expect("valid track reference"),
                CommentTargetKind::Track,
            ),
            comment_id: "12840183".to_owned(),
            kind: CommentReactionKind::Like,
            active: true,
            target_user_ref: None,
            account: Some("personal".to_owned()),
        };
        let result = CommentReactionMutationResult {
            target: request.target.clone(),
            comment_id: request.comment_id.clone(),
            kind: request.kind,
            active: request.active,
            target_user_ref: request.target_user_ref.clone(),
            extensions: Extensions::new(),
        };
        let request = serde_json::to_value(request).expect("serialize reaction mutation request");
        assert_eq!(request["target"]["ref"], "netease:185809");
        assert_eq!(request["comment_id"], "12840183");
        assert_eq!(request["kind"], "like");
        assert_eq!(request["active"], true);
        assert!(request["target_user_ref"].is_null());

        let result = serde_json::to_value(result).expect("serialize reaction mutation result");
        assert_eq!(result["kind"], "like");
        assert_eq!(result["active"], true);
    }

    #[test]
    fn comment_reports_keep_target_comment_reason_and_submission_state_explicit() {
        let request = CommentReportRequest {
            target: CommentTarget::new(
                ResourceRef::new(Platform::Netease, "2058263032").expect("valid track reference"),
                CommentTargetKind::Track,
            ),
            comment_id: "123456789".to_owned(),
            reason: "人身攻击".to_owned(),
            account: Some("personal".to_owned()),
        };
        let result = CommentReportResult {
            target: request.target.clone(),
            comment_id: request.comment_id.clone(),
            reason: request.reason.clone(),
            submitted: true,
            extensions: Extensions::new(),
        };

        let request = serde_json::to_value(request).expect("serialize comment report request");
        assert_eq!(request["target"]["ref"], "netease:2058263032");
        assert_eq!(request["comment_id"], "123456789");
        assert_eq!(request["reason"], "人身攻击");
        assert_eq!(request["account"], "personal");

        let result = serde_json::to_value(result).expect("serialize comment report result");
        assert_eq!(result["target"]["kind"], "track");
        assert_eq!(result["comment_id"], "123456789");
        assert_eq!(result["reason"], "人身攻击");
        assert_eq!(result["submitted"], true);
    }

    #[test]
    fn country_calling_codes_keep_group_and_localized_names_explicit() {
        let groups = vec![CountryCallingCodeGroup {
            label: "常用".to_owned(),
            entries: vec![CountryCallingCode {
                calling_code: "86".to_owned(),
                region_code: "CN".to_owned(),
                name: "中国".to_owned(),
                english_name: "China".to_owned(),
                extensions: Extensions::new(),
            }],
            extensions: Extensions::new(),
        }];
        let value = serde_json::to_value(groups).expect("serialize country calling codes");
        assert_eq!(value[0]["label"], "常用");
        assert_eq!(value[0]["entries"][0]["calling_code"], "86");
        assert_eq!(value[0]["entries"][0]["region_code"], "CN");
        assert_eq!(value[0]["entries"][0]["name"], "中国");
        assert_eq!(value[0]["entries"][0]["english_name"], "China");
    }

    #[test]
    fn user_profile_keeps_identity_counts_and_backend_serialization_stable() {
        let profile = UserProfile {
            user: User {
                resource_ref: ResourceRef::new(Platform::Netease, "32953014")
                    .expect("valid user reference"),
                platform: Platform::Netease,
                id: "32953014".to_owned(),
                name: "binaryify".to_owned(),
                avatar_url: None,
                signature: None,
                followed: Some(false),
                mutual: Some(false),
                extensions: Extensions::new(),
            },
            level: Some(10),
            listened_track_count: Some(35_545),
            playlist_count: Some(21),
            playlist_subscriber_count: Some(10),
            following_count: Some(22),
            follower_count: Some(93),
            event_count: Some(21),
            birthday: None,
            created_at: None,
            background_url: None,
            description: None,
            public_listening_history: Some(false),
            extensions: Extensions::from([(
                "backend".to_owned(),
                serde_json::to_value(UserProfileBackend::Modern)
                    .expect("serialize profile backend"),
            )]),
        };
        let value = serde_json::to_value(profile).expect("serialize user profile");
        assert_eq!(value["user"]["ref"], "netease:32953014");
        assert_eq!(value["listened_track_count"], 35_545);
        assert_eq!(value["playlist_subscriber_count"], 10);
        assert_eq!(value["public_listening_history"], false);
        assert_eq!(value["extensions"]["backend"], "modern");
    }

    #[test]
    fn music_video_catalog_keeps_filters_pagination_and_account_typed() {
        let mut request = MusicVideoListRequest::new(MusicVideoCatalog::All, 30, 60);
        request.area = Some(MusicVideoArea::MainlandChina);
        request.video_type = Some(MusicVideoType::Official);
        request.order = Some(MusicVideoOrder::Hot);
        request.account = Some("viewer".to_owned());

        let value = serde_json::to_value(request).expect("serialize music video request");
        assert_eq!(value["catalog"], "all");
        assert_eq!(value["limit"], 30);
        assert_eq!(value["offset"], 60);
        assert_eq!(value["area"], "mainland_china");
        assert_eq!(value["video_type"], "official");
        assert_eq!(value["order"], "hot");
        assert!(value["group_id"].is_null());
        assert_eq!(value["account"], "viewer");

        let mut grouped = MusicVideoListRequest::new(MusicVideoCatalog::Group, 8, 16);
        grouped.group_id = Some("58100".to_owned());
        let value = serde_json::to_value(grouped).expect("serialize video group request");
        assert_eq!(value["catalog"], "group");
        assert_eq!(value["group_id"], "58100");

        let taxonomy = VideoTaxonomyRequest::new(VideoTaxonomyKind::Groups, 99, 0);
        let value = serde_json::to_value(taxonomy).expect("serialize video taxonomy request");
        assert_eq!(value["kind"], "groups");
        assert_eq!(value["limit"], 99);
    }
}
