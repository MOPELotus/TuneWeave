//! Platform-neutral domain types and provider interfaces for TuneWeave.

mod auth;
mod capability;
mod error;
mod matcher;
mod model;
mod platform;
mod provider;
mod registry;
mod resolver;

pub use auth::{
    AccountProfile, AuthChallengeRequest, AuthState, ChallengeMethod, PasswordFormat,
    PasswordLoginRequest, PrincipalType, ProviderQrPoll, ProviderQrStart,
};
pub use capability::Capability;
pub use error::{ErrorCode, Result, TuneWeaveError};
pub use matcher::{MatchAssessment, assess_track_match};
pub use model::{
    Album, AlbumListRequest, AlbumStats, AlbumSummary, ArtistSummary, DigitalAlbum,
    DigitalAlbumListRequest, Extensions, LyricContributor, Lyrics, MediaStream, Money, Page,
    PageMeta, PageRequest, PlaybackHistoryEntry, PlaybackHistoryPeriod, PlaybackHistoryRequest,
    Playlist, ProviderDescriptor, Quality, RecommendationRequest, ResolutionAttempt,
    ResolutionStatus, ResolveRequest, SearchKind, SearchQuery, StreamRequest, Track,
    TrackEntitlement, TrialWindow,
};
pub use platform::{ParsePlatformError, ParseResourceRefError, Platform, ResourceRef};
pub use provider::MusicProvider;
pub use registry::ProviderRegistry;
pub use resolver::StreamResolver;
