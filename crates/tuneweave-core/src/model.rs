use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{Capability, Platform, ResourceRef};

pub type Extensions = BTreeMap<String, Value>;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchKind {
    #[default]
    Track,
    Album,
    Artist,
    Playlist,
    Video,
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
}
