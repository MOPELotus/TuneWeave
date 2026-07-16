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
    Video,
    Mixed,
    Voice,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    #[default]
    Auto,
    Low,
    Standard,
    High,
    Lossless,
    Hires,
    Spatial,
    Master,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SearchQuery {
    pub query: String,
    pub kind: SearchKind,
    pub limit: u32,
    pub offset: u32,
    pub account: Option<String>,
}

impl SearchQuery {
    pub fn tracks(query: impl Into<String>, limit: u32, offset: u32) -> Self {
        Self {
            query: query.into(),
            kind: SearchKind::Track,
            limit,
            offset,
            account: None,
        }
    }
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BannerListRequest {
    pub client: BannerClient,
    pub account: Option<String>,
}

impl BannerListRequest {
    #[must_use]
    pub fn new(client: BannerClient) -> Self {
        Self {
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
}

impl RecommendationRequest {
    #[must_use]
    pub fn new(limit: u32, offset: u32) -> Self {
        Self {
            limit,
            offset,
            account: None,
            refresh: false,
        }
    }
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StreamRequest {
    pub quality: Quality,
    pub account: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ResolveRequest {
    pub quality: Quality,
    pub playback_platforms: Vec<Platform>,
    pub fallback: bool,
    pub accounts: BTreeMap<Platform, String>,
    pub strict_match: bool,
}

impl Default for ResolveRequest {
    fn default() -> Self {
        Self {
            quality: Quality::Auto,
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
    fn banner_keeps_client_and_target_semantics_typed() {
        let request = BannerListRequest::new(BannerClient::Iphone);
        assert_eq!(
            serde_json::to_value(request.client).expect("serialize banner client"),
            "iphone"
        );

        let banner = Banner {
            id: Some("4862548".to_owned()),
            title: Some("新歌首发".to_owned()),
            image_url: "https://example.test/banner.jpg".to_owned(),
            target_ref: Some(
                ResourceRef::new(Platform::Netease, "3402163617").expect("valid target reference"),
            ),
            target_kind: BannerTargetKind::Track,
            url: Some("https://music.163.com/song?id=3402163617".to_owned()),
            exclusive: Some(false),
            extensions: Extensions::new(),
        };
        let value = serde_json::to_value(banner).expect("serialize banner");
        assert_eq!(value["target_ref"], "netease:3402163617");
        assert_eq!(value["target_kind"], "track");
        assert_eq!(value["title"], "新歌首发");
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
                "video",
                "mixed",
                "voice",
            ]
        );
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
}
