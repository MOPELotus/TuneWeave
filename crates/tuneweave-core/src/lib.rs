//! Platform-neutral domain types and provider interfaces for TuneWeave.

mod auth;
mod capability;
mod credential_store;
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
pub use credential_store::{
    AccountCredentialStore, FileAccountCredentialStore, StoredAccountCredential,
};
pub use error::{ErrorCode, Result, TuneWeaveError};
pub use matcher::{MatchAssessment, assess_track_match};
pub use model::{
    Album, AlbumListRequest, AlbumStats, AlbumSummary, Artist, ArtistArea, ArtistBiographySection,
    ArtistCategory, ArtistChart, ArtistChartArea, ArtistChartEntry, ArtistChartRequest,
    ArtistContentCount, ArtistListRequest, ArtistOverview, ArtistStats, ArtistSummary,
    ArtistTrackListRequest, ArtistTrackOrder, ArtistUpdatesRequest, ArtistVideoListRequest,
    ArtistWorkKind, ArtistWorkUpdate, ArtistWorksRequest, AudioRecognition, AudioRecognitionMatch,
    AudioRecognitionRequest, Banner, BannerClient, BannerListRequest, BannerTargetKind, Chart,
    ChartCatalog, ChartCatalogRequest, ChartCatalogView, ChartGroup, ChartTrackPreview,
    CloudImportRequest, CloudImportResult, CloudLyricsRequest, CloudMatchRequest, CloudMatchResult,
    CloudTrack, CloudTrackDeleteRequest, CloudTrackDeleteResult, CloudTrackDetailRequest,
    CloudUploadCompleteRequest, CloudUploadRequest, CloudUploadResult, CloudUploadTicket,
    CloudUploadTicketRequest, Comment, CommentDeleteRequest, CommentListRequest, CommentListView,
    CommentMutationAction, CommentMutationResult, CommentPage, CommentReaction,
    CommentReactionKind, CommentReactionListRequest, CommentReactionMutationRequest,
    CommentReactionMutationResult, CommentReactionPage, CommentReplyReference,
    CommentReportRequest, CommentReportResult, CommentSort, CommentTarget, CommentTargetKind,
    CommentThreadStats, CommentThreadStatsBatch, CommentThreadStatsRequest, CommentWriteRequest,
    CountryCallingCode, CountryCallingCodeGroup, CountryCallingCodeListRequest, CreatorSummary,
    DigitalAlbum, DigitalAlbumChartEntry, DigitalAlbumChartKind, DigitalAlbumChartPeriod,
    DigitalAlbumChartRequest, DigitalAlbumListRequest, DimensionChart, DimensionChartRequest,
    DimensionChartTrackEntry, DimensionChartTrackSnapshot, Extensions, ImageUploadRequest,
    ImageUploadResult, LocalTrackMatchRequest, LocalTrackMatchResult, LyricContributor, Lyrics,
    MediaDownload, MediaStream, MembershipSummary, Money, Page, PageMeta, PageRequest,
    PlatformApiRequest, PlatformBatchRequest, PlaybackHistoryEntry, PlaybackHistoryPeriod,
    PlaybackHistoryRequest, Playlist, PlaylistCoverUpdateResult, PlaylistCreateRequest,
    PlaylistDeleteRequest, PlaylistDeleteResult, PlaylistItemKind, PlaylistItemMutationAction,
    PlaylistItemMutationRequest, PlaylistItemMutationResult, PlaylistKind,
    PlaylistMetadataUpdateVariant, PlaylistMutationAction, PlaylistMutationResult,
    PlaylistOrderRequest, PlaylistOrderResult, PlaylistTrackOrderRequest, PlaylistTrackOrderResult,
    PlaylistUpdateRequest, PlaylistVisibility, Podcast, PodcastCatalog, PodcastCategory,
    PodcastChartEntry, PodcastChartKind, PodcastChartRequest, PodcastEpisode,
    PodcastEpisodeChartEntry, PodcastEpisodeChartKind, PodcastEpisodeChartRequest,
    PodcastEpisodeListRequest, PodcastEpisodeLyrics, PodcastEpisodeStream, PodcastListRequest,
    PodcastTaxonomy, ProviderDescriptor, Quality, RadioCatalogOption, RadioStation,
    RadioStationCursor, RadioStationListRequest, RadioTaxonomy, RadioTaxonomyRequest,
    RecommendationRequest, ResolutionAttempt, ResolutionStatus, ResolveRequest,
    SearchDefaultKeyword, SearchDefaultKeywordRequest, SearchItem, SearchKind, SearchMultiMatch,
    SearchMultiMatchRequest, SearchMultiMatchSection, SearchOpaqueItem, SearchQuery,
    SearchSuggestion, SearchSuggestionClient, SearchSuggestionList, SearchSuggestionRequest,
    SearchTrendingDetail, SearchTrendingEntry, SearchTrendingList, SearchTrendingRequest,
    SearchVariant, StreamBatch, StreamOutcome, StreamRequest, StreamVariant, SubscriptionResult,
    Track, TrackAvailability, TrackAvailabilityRequest, TrackEntitlement, TrialWindow, User, Video,
    VideoDetail, VideoDetailRequest, VideoKind, VideoResolution, VideoResourceKind, VideoStats,
    VideoStream, VideoStreamRequest,
};
pub use platform::{ParsePlatformError, ParseResourceRefError, Platform, ResourceRef};
pub use provider::MusicProvider;
pub use registry::ProviderRegistry;
pub use resolver::StreamResolver;
