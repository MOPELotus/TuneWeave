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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PageMeta {
    pub limit: u32,
    pub offset: u32,
    pub total: Option<u64>,
    pub next_offset: Option<u32>,
    pub has_more: bool,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ArtistSummary {
    #[serde(rename = "ref")]
    pub resource_ref: Option<ResourceRef>,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AlbumSummary {
    #[serde(rename = "ref")]
    pub resource_ref: Option<ResourceRef>,
    pub name: String,
    pub cover_url: Option<String>,
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
}
