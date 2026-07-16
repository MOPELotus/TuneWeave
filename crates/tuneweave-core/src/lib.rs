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
    AccountProfile, AuthChallengeRequest, AuthChallengeValidation, AuthPrincipalStatus,
    AuthPrincipalStatusRequest, AuthState, ChallengeMethod, PasswordFormat, PasswordLoginRequest,
    PrincipalType, ProviderQrPoll, ProviderQrStart,
};
pub use capability::Capability;
pub use error::{ErrorCode, Result, TuneWeaveError};
pub use matcher::{MatchAssessment, assess_track_match};
pub use model::{
    Album, AlbumListRequest, AlbumStats, AlbumSummary, Artist, ArtistArea, ArtistBiographySection,
    ArtistCategory, ArtistContentCount, ArtistListRequest, ArtistOverview, ArtistStats,
    ArtistSummary, ArtistTrackListRequest, ArtistTrackOrder, ArtistUpdatesRequest,
    ArtistVideoListRequest, ArtistWorkKind, ArtistWorkUpdate, ArtistWorksRequest, AudioRecognition,
    AudioRecognitionMatch, AudioRecognitionRequest, Banner, BannerClient, BannerListRequest,
    BannerTargetKind, CloudImportRequest, CloudImportResult, CloudLyricsRequest, CloudMatchRequest,
    CloudMatchResult, CloudUploadCompleteRequest, CloudUploadRequest, CloudUploadResult,
    CloudUploadTicket, CloudUploadTicketRequest, Comment, CommentDeleteRequest, CommentListRequest,
    CommentListView, CommentMutationAction, CommentMutationResult, CommentPage, CommentReaction,
    CommentReactionKind, CommentReactionListRequest, CommentReactionPage, CommentReplyReference,
    CommentSort, CommentTarget, CommentTargetKind, CommentThreadStats, CommentThreadStatsBatch,
    CommentThreadStatsRequest, CommentWriteRequest, CreatorSummary, DigitalAlbum,
    DigitalAlbumChartEntry, DigitalAlbumChartKind, DigitalAlbumChartPeriod,
    DigitalAlbumChartRequest, DigitalAlbumListRequest, DimensionChart, DimensionChartRequest,
    DimensionChartTrackEntry, DimensionChartTrackSnapshot, Extensions, ImageUploadRequest,
    ImageUploadResult, LyricContributor, Lyrics, MediaStream, Money, Page, PageMeta, PageRequest,
    PlatformApiRequest, PlatformBatchRequest, PlaybackHistoryEntry, PlaybackHistoryPeriod,
    PlaybackHistoryRequest, Playlist, ProviderDescriptor, Quality, RadioCatalogOption,
    RadioStation, RadioStationCursor, RadioStationListRequest, RadioTaxonomy, RadioTaxonomyRequest,
    RecommendationRequest, ResolutionAttempt, ResolutionStatus, ResolveRequest, SearchItem,
    SearchKind, SearchOpaqueItem, SearchQuery, StreamRequest, SubscriptionResult, Track,
    TrackAvailability, TrackAvailabilityRequest, TrackEntitlement, TrialWindow, User, Video,
    VideoKind,
};
pub use platform::{ParsePlatformError, ParseResourceRefError, Platform, ResourceRef};
pub use provider::MusicProvider;
pub use registry::ProviderRegistry;
pub use resolver::StreamResolver;
