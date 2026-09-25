//! Platform-neutral domain types and provider interfaces for TuneWeave.

mod auth;
mod caller_credential;
mod capability;
mod chart_period;
mod credential_store;
mod error;
mod matcher;
mod model;
mod observability;
mod platform;
mod provider;
mod registry;
mod resolver;
mod scrobble;
mod uni_playlist_document;
mod uni_playlist_store;

pub use chart_period::{ChartPeriod, ChartPeriodSummary};

pub use auth::{
    AccountProfile, AuthAccountChoice, AuthBrowserChallenge, AuthChallengeAction,
    AuthChallengeBackend, AuthChallengeDelivery, AuthChallengeProgress, AuthChallengeRequest,
    AuthChallengeStatus, AuthChallengeValidation, AuthImageAnswerKind, AuthImageChallenge,
    AuthPrincipalStatus, AuthPrincipalStatusRequest, AuthSecurityChallengeRequest, AuthState,
    ChallengeMethod, CredentialImportRequest, CredentialMode, ImportedCredential, PasswordFormat,
    PasswordLoginBackend, PasswordLoginRequest, PrincipalType, ProviderAuthChallenge,
    ProviderAuthResult, ProviderLogoutResult, ProviderQrPoll, ProviderQrStart,
    ProviderSessionRevocationResult, QrVerification, QrVerificationAction, QrVerificationMethod,
    SessionRevocationState, UpSmsInstructions,
};
pub use caller_credential::{
    CALLER_CREDENTIAL_FORMAT, CALLER_CREDENTIAL_HEADER, CallerCredential, ProviderCredential,
};
pub use capability::Capability;
pub use credential_store::{
    AccountCredentialStore, FileAccountCredentialStore, StoredAccountCredential,
};
pub use error::{ErrorCode, Result, TuneWeaveError};
pub use matcher::{MatchAssessment, assess_track_match};
pub use model::{
    AccountDislikeEntry, AccountDislikeKind, AccountDislikeList, AccountDislikeListRequest,
    AccountDislikeMutationAction, AccountDislikeMutationRequest, AccountDislikeMutationResult,
    AiLyricDictionary, AiLyricDictionaryAvailability, AiLyricDictionaryEntry, Album,
    AlbumListRequest, AlbumStats, AlbumSummary, AnonymousSession, AntiCheatToken,
    AntiCheatTokenVersion, Artist, ArtistArea, ArtistBiographySection, ArtistCatalog,
    ArtistCatalogFilterOption, ArtistCatalogFilters, ArtistCatalogRequest, ArtistCategory,
    ArtistChart, ArtistChartArea, ArtistChartEntry, ArtistChartRequest, ArtistContentCount,
    ArtistDescriptionRequest, ArtistGenre, ArtistHomepageIntroduction, ArtistHomepageTab,
    ArtistHomepageTabKind, ArtistHomepageTabMetadata, ArtistHomepageTabRequest, ArtistListRequest,
    ArtistOverview, ArtistStats, ArtistSummary, ArtistTrackListRequest, ArtistTrackOrder,
    ArtistUpdatesRequest, ArtistVideoListRequest, ArtistWorkKind, ArtistWorkUpdate,
    ArtistWorksRequest, AudioCdnDispatch, AudioCdnNode, AudioContent, AudioFileAccess,
    AudioFileBatch, AudioFileRequest, AudioFileRequestItem, AudioRecognition,
    AudioRecognitionMatch, AudioRecognitionRequest, Banner, BannerCatalog, BannerClient,
    BannerListRequest, BannerTargetKind, Chart, ChartCatalog, ChartCatalogRequest,
    ChartCatalogView, ChartGroup, ChartTrackListRequest, ChartTrackPreview, CloudImportRequest,
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
    DimensionChartTrackEntry, DimensionChartTrackSnapshot, Extensions, FavoriteIntelligenceItem,
    FavoriteIntelligenceQueue, FavoriteIntelligenceRequest, GeneralSearchRelated,
    GeneralSearchRelatedTerm, GeneralSearchRequest, GeneralSearchResult, GeneralSearchSection,
    ImageUploadRequest, ImageUploadResult, ImmersiveAudioType, ListeningRightsAction,
    ListeningRightsAd, ListeningRightsAdCatalog, ListeningRightsAdRequest,
    ListeningRightsGainRequest, ListeningRightsGainResult, ListeningRightsMembershipContent,
    ListeningRightsMembershipPreview, ListeningRightsOffer, ListeningRightsRewardEntry,
    ListeningRightsStatus, ListeningRightsStatusRequest, ListeningRightsTimestamp,
    LocalTrackMatchRequest, LocalTrackMatchResult, LyricContributor, Lyrics, LyricsRequest,
    MediaDownload, MediaStream, MembershipSummary, MiguNativeMvFormat, MiguNativeMvStreamRequest,
    Money, MultiStyleLyricTranslation, MultiStyleLyricTranslations, MusicGeneAge,
    MusicGeneAiInterpretation, MusicGeneAttribute, MusicGeneGroove, MusicGeneListeningPeriod,
    MusicGeneListeningReport, MusicGeneMainDescription, MusicGenePersonality, MusicGenePreferences,
    MusicGeneStatus, MusicGeneStatusEntry, MusicGeneTempo, MusicVideoArea, MusicVideoCatalog,
    MusicVideoListRequest, MusicVideoOrder, MusicVideoType, Page, PageMeta, PageRequest,
    PersonalFmRequest, PersonalFmVariant, PlatformApiRequest, PlatformBatchRequest, PlaybackDevice,
    PlaybackHistoryEntry, PlaybackHistoryPeriod, PlaybackHistoryRequest, Playlist,
    PlaylistCatalogKind, PlaylistCatalogRequest, PlaylistCatalogTag, PlaylistCatalogTagGroup,
    PlaylistCatalogTaxonomy, PlaylistCatalogTaxonomyRequest, PlaylistCoverUpdateResult,
    PlaylistCreateRequest, PlaylistDeleteRequest, PlaylistDeleteResult, PlaylistItemKind,
    PlaylistItemMutationAction, PlaylistItemMutationRequest, PlaylistItemMutationResult,
    PlaylistKind, PlaylistMetadataUpdateVariant, PlaylistMutationAction, PlaylistMutationResult,
    PlaylistOccurrenceOrderRequest, PlaylistOccurrenceOrderResult, PlaylistOrderRequest,
    PlaylistOrderResult, PlaylistPlayableEntry, PlaylistPlayableItem, PlaylistSubmission,
    PlaylistSubmissionRecordDeleteRequest, PlaylistSubmissionRecordDeleteResult,
    PlaylistSubmissionRequest, PlaylistSubmissionResult, PlaylistSubmissionStatus,
    PlaylistTrackOccurrence, PlaylistTrackOrderRequest, PlaylistTrackOrderResult,
    PlaylistUpdateRequest, PlaylistVisibility, PlaylistVisibilityUpdateRequest, Podcast,
    PodcastCatalog, PodcastCategory, PodcastCategoryRecommendation, PodcastCategoryRecommendations,
    PodcastChartEntry, PodcastChartKind, PodcastChartRequest, PodcastCreatorChartEntry,
    PodcastCreatorChartKind, PodcastCreatorChartRequest, PodcastEpisode, PodcastEpisodeChartEntry,
    PodcastEpisodeChartKind, PodcastEpisodeChartRequest, PodcastEpisodeCover,
    PodcastEpisodeDeleteRequest, PodcastEpisodeDeleteResult, PodcastEpisodeDisplayStatus,
    PodcastEpisodeFeeFilter, PodcastEpisodeListRequest, PodcastEpisodeLyrics,
    PodcastEpisodeOrderRequest, PodcastEpisodeOrderResult, PodcastEpisodePlaybackHistoryEntry,
    PodcastEpisodeRecommendationRequest, PodcastEpisodeRecommendationSource, PodcastEpisodeStream,
    PodcastEpisodeUploadRequest, PodcastEpisodeUploadResult, PodcastEpisodeVisibility,
    PodcastEpisodeWorkbenchSearchRequest, PodcastListRequest, PodcastTaxonomy, PodcastTaxonomyKind,
    PodcastTaxonomyRequest, ProviderDescriptor, PurchasedAlbum, PurchasedTrack, Quality,
    RadioCatalogOption, RadioPlaybackItem, RadioPlaybackQueue, RadioPlaybackQueueRequest,
    RadioStation, RadioStationCursor, RadioStationListRequest, RadioStyle, RadioStyleCatalog,
    RadioStyleCatalogRequest, RadioStyleSource, RadioTaxonomy, RadioTaxonomyRequest,
    RecentAlbumHistoryEntry, RecentPlaylistHistoryEntry, RecentTrackHistoryEntry,
    RecommendationDislikeRequest, RecommendationDislikeResult, RecommendationFeed,
    RecommendationFeedAction, RecommendationFeedCard, RecommendationFeedCardKind,
    RecommendationFeedCursor, RecommendationFeedDirection, RecommendationFeedNiche,
    RecommendationFeedRequest, RecommendationFeedShelf, RecommendationRequest,
    RecommendationSource, RelatedPlaylistList, RelatedPlaylistRequest, RelatedPlaylistSection,
    RelatedPlaylistSectionKind, RelatedVideoList, RelatedVideoRequest, ResolutionAttempt,
    ResolutionStatus, ResolveRequest, SearchDefaultKeyword, SearchDefaultKeywordRequest,
    SearchItem, SearchKind, SearchMultiMatch, SearchMultiMatchRequest, SearchMultiMatchSection,
    SearchOpaqueItem, SearchQuery, SearchSelector, SearchSuggestion, SearchSuggestionClient,
    SearchSuggestionList, SearchSuggestionRequest, SearchTrendingDetail, SearchTrendingEntry,
    SearchTrendingList, SearchTrendingRequest, SearchVariant, SheetMusic, SheetMusicAvailability,
    SheetMusicList, SheetMusicSource, SimilarArtistList, SimilarArtistRequest, SimilarTrackList,
    SimilarTrackRequest, SimilarTrackSection, SimilarTrackSectionKind,
    SingingAnnotationsAvailability, StreamBatch, StreamOutcome, StreamRequest, StreamVariant,
    StyledRadioStationLibraryRequest, SubscriptionResult, Track, TrackAvailability,
    TrackAvailabilityRequest, TrackCredit, TrackCreditGroup, TrackCredits, TrackDetailBatchRequest,
    TrackDetailRequestItem, TrackEntitlement, TrackFavoriteCount, TrackIdentifierKind, TrackLabel,
    TrackLabelList, TrackVersionList, TrialWindow, UniPlaylist, UniPlaylistClientItemStream,
    UniPlaylistCreateRequest, UniPlaylistDeleteResult, UniPlaylistDocument,
    UniPlaylistDocumentExtensions, UniPlaylistDocumentFormat, UniPlaylistDocumentImportResult,
    UniPlaylistDocumentItem, UniPlaylistDocumentItemExtensions, UniPlaylistDocumentSnapshot,
    UniPlaylistDocumentSnapshotExtensions, UniPlaylistImportRequest, UniPlaylistImportResult,
    UniPlaylistImportSourceRequest, UniPlaylistImportSourceResult, UniPlaylistItem,
    UniPlaylistItemAddRequest, UniPlaylistItemAddResult, UniPlaylistItemDeleteResult,
    UniPlaylistItemInput, UniPlaylistItemKind, UniPlaylistItemOrderRequest,
    UniPlaylistItemOrderResult, UniPlaylistItemSnapshot, UniPlaylistItemStream,
    UniPlaylistMaterializeImportsResult, UniPlaylistMaterializeItemsResult,
    UniPlaylistUpdateRequest, User, UserMusicGene, UserProfile, UserProfileBackend, Video,
    VideoAudioStream, VideoAudioStreamRequest, VideoAudioTier, VideoCatalogOption,
    VideoCodecFamily, VideoDetail, VideoDetailRequest, VideoDynamicRange, VideoKind, VideoPart,
    VideoPartListRequest, VideoPlaybackFormat, VideoPlaybackLanguage, VideoPlaybackLanguageCatalog,
    VideoPlaybackManifest, VideoPlaybackProgressiveSegment, VideoPlaybackRequest,
    VideoPlaybackSegmentBase, VideoPlaybackTrack, VideoPlaybackTrackKind, VideoRecommendationKind,
    VideoRecommendationRequest, VideoRecommendationView, VideoResolution, VideoResourceKind,
    VideoSearchDuration, VideoSearchFilters, VideoSearchOrder, VideoSourceRange, VideoStats,
    VideoStream, VideoStreamRequest, VideoSubtitle, VideoSubtitleCue, VideoSubtitleDocument,
    VideoSubtitleList, VideoSubtitleRequest, VideoSubtitleStyle, VideoTaxonomyKind,
    VideoTaxonomyRequest, VideoTrackQuality, VideoTrackStream, VideoTrackStreamRequest,
};
pub use observability::{UpstreamBusinessClass, UpstreamOutcome, UpstreamRequestSummary};
pub use platform::{ParsePlatformError, ParseResourceRefError, Platform, ResourceRef};
pub use provider::MusicProvider;
pub use registry::ProviderRegistry;
pub use resolver::StreamResolver;
pub use scrobble::{ScrobbleRequest, ScrobbleResult};
pub use uni_playlist_document::UNI_PLAYLIST_DOCUMENT_FORMAT;
pub use uni_playlist_store::{
    DirectoryUniPlaylistStore, FileUniPlaylistStore, MemoryUniPlaylistStore, UniPlaylistStore,
};

mod password_auth;
pub use password_auth::{
    PasswordBrowserProtocol, PasswordChallengeAction, PasswordLoginContext, PasswordLoginIdentity,
    PasswordLoginProgress, PasswordSliderChallenge, PasswordSliderFingerprintAlgorithm,
    PasswordSliderGesture, PasswordSliderPointerSample, PasswordSliderPreparation,
    PasswordSliderProtocol, PasswordVerification, ProviderPasswordChallenge,
};

mod cloud_transfer;
pub use cloud_transfer::{
    CloudUploadPublishMetadata, CloudUploadStep, CloudUploadStepKind, CloudUploadStepResponse,
    CloudUploadStrategy, CloudUploadTransfer, CloudUploadTransferRequest, CloudUploadTransferState,
};
