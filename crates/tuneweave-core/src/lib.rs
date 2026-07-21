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
mod uni_playlist_store;

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
    Album, AlbumListRequest, AlbumStats, AlbumSummary, AnonymousSession, AntiCheatToken,
    AntiCheatTokenVersion, Artist, ArtistArea, ArtistBiographySection, ArtistCategory, ArtistChart,
    ArtistChartArea, ArtistChartEntry, ArtistChartRequest, ArtistContentCount, ArtistListRequest,
    ArtistOverview, ArtistStats, ArtistSummary, ArtistTrackListRequest, ArtistTrackOrder,
    ArtistUpdatesRequest, ArtistVideoListRequest, ArtistWorkKind, ArtistWorkUpdate,
    ArtistWorksRequest, AudioRecognition, AudioRecognitionMatch, AudioRecognitionRequest, Banner,
    BannerCatalog, BannerClient, BannerListRequest, BannerTargetKind, Chart, ChartCatalog,
    ChartCatalogRequest, ChartCatalogView, ChartGroup, ChartTrackPreview, CloudImportRequest,
    CloudImportResult, CloudLyricsRequest, CloudMatchRequest, CloudMatchResult, CloudTrack,
    CloudTrackDeleteRequest, CloudTrackDeleteResult, CloudTrackDetailRequest,
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
    ImageUploadResult, ListeningRightsAd, ListeningRightsAdCatalog, ListeningRightsAdRequest,
    ListeningRightsGainRequest, ListeningRightsGainResult, ListeningRightsTimestamp,
    LocalTrackMatchRequest, LocalTrackMatchResult, LyricContributor, Lyrics, MediaDownload,
    MediaStream, MembershipSummary, Money, MusicVideoArea, MusicVideoCatalog,
    MusicVideoListRequest, MusicVideoOrder, MusicVideoType, Page, PageMeta, PageRequest,
    PersonalFmRequest, PersonalFmVariant, PlatformApiRequest, PlatformBatchRequest, PlaybackDevice,
    PlaybackHistoryEntry, PlaybackHistoryPeriod, PlaybackHistoryRequest, Playlist,
    PlaylistCoverUpdateResult, PlaylistCreateRequest, PlaylistDeleteRequest, PlaylistDeleteResult,
    PlaylistItemKind, PlaylistItemMutationAction, PlaylistItemMutationRequest,
    PlaylistItemMutationResult, PlaylistKind, PlaylistMetadataUpdateVariant,
    PlaylistMutationAction, PlaylistMutationResult, PlaylistOrderRequest, PlaylistOrderResult,
    PlaylistPlayableItem, PlaylistTrackOrderRequest, PlaylistTrackOrderResult,
    PlaylistUpdateRequest, PlaylistVisibility, Podcast, PodcastCatalog, PodcastCategory,
    PodcastCategoryRecommendation, PodcastCategoryRecommendations, PodcastChartEntry,
    PodcastChartKind, PodcastChartRequest, PodcastCreatorChartEntry, PodcastCreatorChartKind,
    PodcastCreatorChartRequest, PodcastEpisode, PodcastEpisodeChartEntry, PodcastEpisodeChartKind,
    PodcastEpisodeChartRequest, PodcastEpisodeDeleteRequest, PodcastEpisodeDeleteResult,
    PodcastEpisodeDisplayStatus, PodcastEpisodeFeeFilter, PodcastEpisodeListRequest,
    PodcastEpisodeLyrics, PodcastEpisodeOrderRequest, PodcastEpisodeOrderResult,
    PodcastEpisodePlaybackHistoryEntry, PodcastEpisodeRecommendationRequest,
    PodcastEpisodeRecommendationSource, PodcastEpisodeStream, PodcastEpisodeUploadRequest,
    PodcastEpisodeUploadResult, PodcastEpisodeVisibility, PodcastEpisodeWorkbenchSearchRequest,
    PodcastListRequest, PodcastTaxonomy, PodcastTaxonomyKind, PodcastTaxonomyRequest,
    ProviderDescriptor, Quality, RadioCatalogOption, RadioPlaybackItem, RadioPlaybackQueue,
    RadioPlaybackQueueRequest, RadioStation, RadioStationCursor, RadioStationListRequest,
    RadioStyle, RadioStyleCatalog, RadioStyleCatalogRequest, RadioStyleSource, RadioTaxonomy,
    RadioTaxonomyRequest, RecommendationDislikeRequest, RecommendationDislikeResult,
    RecommendationRequest, RecommendationSource, ResolutionAttempt, ResolutionStatus,
    ResolveRequest, SearchDefaultKeyword, SearchDefaultKeywordRequest, SearchItem, SearchKind,
    SearchMultiMatch, SearchMultiMatchRequest, SearchMultiMatchSection, SearchOpaqueItem,
    SearchQuery, SearchSuggestion, SearchSuggestionClient, SearchSuggestionList,
    SearchSuggestionRequest, SearchTrendingDetail, SearchTrendingEntry, SearchTrendingList,
    SearchTrendingRequest, SearchVariant, StreamBatch, StreamOutcome, StreamRequest, StreamVariant,
    StyledRadioStationLibraryRequest, SubscriptionResult, Track, TrackAvailability,
    TrackAvailabilityRequest, TrackEntitlement, TrialWindow, UniPlaylist, UniPlaylistCreateRequest,
    UniPlaylistImportRequest, UniPlaylistImportResult, UniPlaylistImportSourceRequest,
    UniPlaylistImportSourceResult, UniPlaylistItem, UniPlaylistItemAddRequest,
    UniPlaylistItemAddResult, UniPlaylistItemDeleteResult, UniPlaylistItemInput,
    UniPlaylistItemKind, UniPlaylistItemOrderRequest, UniPlaylistItemOrderResult,
    UniPlaylistItemSnapshot, User, UserProfile, UserProfileBackend, Video, VideoCatalogOption,
    VideoDetail, VideoDetailRequest, VideoKind, VideoRecommendationKind,
    VideoRecommendationRequest, VideoRecommendationView, VideoResolution, VideoResourceKind,
    VideoStats, VideoStream, VideoStreamRequest, VideoTaxonomyKind, VideoTaxonomyRequest,
};
pub use platform::{ParsePlatformError, ParseResourceRefError, Platform, ResourceRef};
pub use provider::MusicProvider;
pub use registry::ProviderRegistry;
pub use resolver::StreamResolver;
pub use uni_playlist_store::{FileUniPlaylistStore, MemoryUniPlaylistStore, UniPlaylistStore};
