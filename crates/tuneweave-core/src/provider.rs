use std::collections::BTreeSet;

use async_trait::async_trait;

use crate::{
    AccountProfile, Album, AlbumListRequest, AlbumStats, Artist, ArtistChart, ArtistChartRequest,
    ArtistListRequest, ArtistOverview, ArtistStats, ArtistTrackListRequest, ArtistUpdatesRequest,
    ArtistVideoListRequest, ArtistWorkUpdate, ArtistWorksRequest, AudioRecognition,
    AudioRecognitionRequest, AuthChallengeRequest, AuthChallengeValidation, AuthPrincipalStatus,
    AuthPrincipalStatusRequest, Banner, BannerListRequest, Capability, ChartCatalog,
    ChartCatalogRequest, CloudImportRequest, CloudImportResult, CloudLyricsRequest,
    CloudMatchRequest, CloudMatchResult, CloudTrack, CloudTrackDeleteRequest,
    CloudTrackDeleteResult, CloudTrackDetailRequest, CloudUploadCompleteRequest,
    CloudUploadRequest, CloudUploadResult, CloudUploadTicket, CloudUploadTicketRequest,
    CommentDeleteRequest, CommentListRequest, CommentMutationResult, CommentPage,
    CommentReactionListRequest, CommentReactionMutationRequest, CommentReactionMutationResult,
    CommentReactionPage, CommentReportRequest, CommentReportResult, CommentThreadStatsBatch,
    CommentThreadStatsRequest, CommentWriteRequest, CountryCallingCodeGroup,
    CountryCallingCodeListRequest, DigitalAlbum, DigitalAlbumChartEntry, DigitalAlbumChartRequest,
    DigitalAlbumListRequest, DimensionChart, DimensionChartRequest, DimensionChartTrackSnapshot,
    ErrorCode, Extensions, ImageUploadRequest, ImageUploadResult, LocalTrackMatchRequest,
    LocalTrackMatchResult, Lyrics, MediaDownload, MediaStream, MembershipSummary, Page,
    PageRequest, PasswordLoginRequest, Platform, PlatformApiRequest, PlatformBatchRequest,
    PlaybackHistoryEntry, PlaybackHistoryRequest, Playlist, PlaylistCoverUpdateResult,
    PlaylistCreateRequest, PlaylistDeleteRequest, PlaylistDeleteResult, PlaylistItemMutationAction,
    PlaylistItemMutationRequest, PlaylistItemMutationResult, PlaylistMutationResult,
    PlaylistOrderRequest, PlaylistOrderResult, PlaylistTrackOrderRequest, PlaylistTrackOrderResult,
    PlaylistUpdateRequest, Podcast, PodcastEpisode, PodcastEpisodeListRequest,
    PodcastEpisodeLyrics, PodcastEpisodeStream, PodcastListRequest, PodcastTaxonomy,
    ProviderDescriptor, ProviderQrPoll, ProviderQrStart, RadioStation, RadioStationListRequest,
    RadioTaxonomy, RadioTaxonomyRequest, RecommendationRequest, ResolutionStatus, Result,
    SearchDefaultKeyword, SearchDefaultKeywordRequest, SearchItem, SearchKind, SearchMultiMatch,
    SearchMultiMatchRequest, SearchQuery, SearchSuggestionList, SearchSuggestionRequest,
    SearchTrendingList, SearchTrendingRequest, StreamBatch, StreamOutcome, StreamRequest,
    SubscriptionResult, Track, TrackAvailability, TrackAvailabilityRequest, TrackEntitlement,
    TuneWeaveError, User, Video, VideoDetail, VideoDetailRequest, VideoStats, VideoStream,
    VideoStreamRequest,
};

/// A dynamically registered music platform adapter.
#[async_trait]
pub trait MusicProvider: Send + Sync {
    fn platform(&self) -> Platform;

    fn name(&self) -> &'static str;

    fn capabilities(&self) -> BTreeSet<Capability>;

    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            platform: self.platform(),
            name: self.name().to_owned(),
            capabilities: self.capabilities().into_iter().collect(),
        }
    }

    fn supports(&self, capability: Capability) -> bool {
        self.capabilities().contains(&capability)
    }

    async fn search(&self, _query: &SearchQuery) -> Result<Page<Track>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::SearchTracks,
        ))
    }

    async fn search_catalog(&self, query: &SearchQuery) -> Result<Page<SearchItem>> {
        if query.kind != SearchKind::Track {
            return Err(TuneWeaveError::unsupported(
                self.platform(),
                search_capability(query.kind),
            ));
        }
        let page = self.search(query).await?;
        Ok(Page {
            items: page.items.into_iter().map(SearchItem::Track).collect(),
            pagination: page.pagination,
        })
    }

    async fn default_search_keyword(
        &self,
        _request: &SearchDefaultKeywordRequest,
    ) -> Result<SearchDefaultKeyword> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::SearchDefault,
        ))
    }

    async fn trending_searches(
        &self,
        _request: &SearchTrendingRequest,
    ) -> Result<SearchTrendingList> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::SearchTrending,
        ))
    }

    async fn search_suggestions(
        &self,
        _request: &SearchSuggestionRequest,
    ) -> Result<SearchSuggestionList> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::SearchSuggestions,
        ))
    }

    async fn search_multi_match(
        &self,
        _request: &SearchMultiMatchRequest,
    ) -> Result<SearchMultiMatch> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::SearchMultiMatch,
        ))
    }

    async fn match_local_track(
        &self,
        _request: &LocalTrackMatchRequest,
    ) -> Result<LocalTrackMatchResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::SearchLocalTrackMatch,
        ))
    }

    async fn user_membership(
        &self,
        _id: Option<&str>,
        _account: Option<&str>,
    ) -> Result<MembershipSummary> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::UserMembership,
        ))
    }

    async fn recognize_audio(
        &self,
        _request: &AudioRecognitionRequest,
    ) -> Result<AudioRecognition> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AudioRecognition,
        ))
    }

    async fn banners(&self, _request: &BannerListRequest) -> Result<Vec<Banner>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::Banners,
        ))
    }

    async fn radio_taxonomy(&self, _request: &RadioTaxonomyRequest) -> Result<RadioTaxonomy> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::RadioTaxonomy,
        ))
    }

    async fn radio_station(&self, _id: &str, _account: Option<&str>) -> Result<RadioStation> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::RadioStationDetail,
        ))
    }

    async fn radio_stations(
        &self,
        _request: &RadioStationListRequest,
    ) -> Result<Page<RadioStation>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::RadioStationList,
        ))
    }

    async fn set_radio_station_subscription(
        &self,
        _id: &str,
        _subscribed: bool,
        _account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::RadioStationSubscriptionWrite,
        ))
    }

    async fn podcast_categories(&self, _account: Option<&str>) -> Result<PodcastTaxonomy> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PodcastCategories,
        ))
    }

    async fn podcasts(&self, _request: &PodcastListRequest) -> Result<Page<Podcast>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PodcastList,
        ))
    }

    async fn podcast(&self, _id: &str, _account: Option<&str>) -> Result<Podcast> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PodcastDetail,
        ))
    }

    async fn podcast_episodes(
        &self,
        _id: &str,
        _request: &PodcastEpisodeListRequest,
    ) -> Result<Page<PodcastEpisode>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PodcastEpisodeList,
        ))
    }

    async fn podcast_episode(&self, _id: &str, _account: Option<&str>) -> Result<PodcastEpisode> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PodcastEpisodeDetail,
        ))
    }

    async fn podcast_episode_stream(
        &self,
        _id: &str,
        _request: &StreamRequest,
    ) -> Result<PodcastEpisodeStream> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PodcastEpisodeStream,
        ))
    }

    async fn podcast_episode_lyrics(
        &self,
        _id: &str,
        _account: Option<&str>,
    ) -> Result<PodcastEpisodeLyrics> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PodcastEpisodeLyrics,
        ))
    }

    async fn track(&self, _id: &str, _account: Option<&str>) -> Result<Track> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::TrackDetail,
        ))
    }

    async fn track_availability(
        &self,
        _id: &str,
        _request: &TrackAvailabilityRequest,
    ) -> Result<TrackAvailability> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::TrackAvailability,
        ))
    }

    async fn album(&self, _id: &str, _account: Option<&str>) -> Result<Album> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AlbumDetail,
        ))
    }

    async fn album_tracks(&self, _id: &str, _request: &PageRequest) -> Result<Page<Track>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AlbumDetail,
        ))
    }

    async fn albums(&self, _request: &AlbumListRequest) -> Result<Page<Album>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AlbumList,
        ))
    }

    async fn album_stats(&self, _id: &str, _account: Option<&str>) -> Result<AlbumStats> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AlbumStats,
        ))
    }

    async fn album_track_entitlements(
        &self,
        _id: &str,
        _request: &PageRequest,
    ) -> Result<Page<TrackEntitlement>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AlbumTrackEntitlements,
        ))
    }

    async fn set_album_subscription(
        &self,
        _id: &str,
        _subscribed: bool,
        _account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AlbumSubscriptionWrite,
        ))
    }

    async fn digital_album(&self, _id: &str, _account: Option<&str>) -> Result<DigitalAlbum> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::DigitalAlbumDetail,
        ))
    }

    async fn digital_albums(
        &self,
        _request: &DigitalAlbumListRequest,
    ) -> Result<Page<DigitalAlbum>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::DigitalAlbumList,
        ))
    }

    async fn digital_album_chart(
        &self,
        _request: &DigitalAlbumChartRequest,
    ) -> Result<Page<DigitalAlbumChartEntry>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::DigitalAlbumCharts,
        ))
    }

    async fn chart_catalog(&self, _request: &ChartCatalogRequest) -> Result<ChartCatalog> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::ChartCatalog,
        ))
    }

    async fn artist_chart(&self, _request: &ArtistChartRequest) -> Result<ArtistChart> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::ArtistCharts,
        ))
    }

    async fn dimension_chart(&self, _request: &DimensionChartRequest) -> Result<DimensionChart> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::DimensionCharts,
        ))
    }

    async fn dimension_chart_tracks(
        &self,
        _request: &DimensionChartRequest,
    ) -> Result<DimensionChartTrackSnapshot> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::DimensionCharts,
        ))
    }

    async fn artist(&self, _id: &str, _account: Option<&str>) -> Result<Artist> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::ArtistDetail,
        ))
    }

    async fn artist_overview(&self, _id: &str, _account: Option<&str>) -> Result<ArtistOverview> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::ArtistOverview,
        ))
    }

    async fn artist_stats(&self, _id: &str, _account: Option<&str>) -> Result<ArtistStats> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::ArtistStats,
        ))
    }

    async fn artists(&self, _request: &ArtistListRequest) -> Result<Page<Artist>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::ArtistList,
        ))
    }

    async fn artist_albums(&self, _id: &str, _request: &PageRequest) -> Result<Page<Album>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::ArtistAlbums,
        ))
    }

    async fn artist_fans(&self, _id: &str, _request: &PageRequest) -> Result<Page<User>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::ArtistFans,
        ))
    }

    async fn artist_videos(
        &self,
        _id: &str,
        _request: &ArtistVideoListRequest,
    ) -> Result<Page<Video>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::ArtistVideos,
        ))
    }

    async fn video(&self, _id: &str, _request: &VideoDetailRequest) -> Result<VideoDetail> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::VideoDetail,
        ))
    }

    async fn video_stats(&self, _id: &str, _request: &VideoDetailRequest) -> Result<VideoStats> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::VideoStats,
        ))
    }

    async fn video_stream(&self, _id: &str, _request: &VideoStreamRequest) -> Result<VideoStream> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::VideoStream,
        ))
    }

    async fn artist_tracks(
        &self,
        _id: &str,
        _request: &ArtistTrackListRequest,
    ) -> Result<Page<Track>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::ArtistTracks,
        ))
    }

    async fn artist_top_tracks(&self, _id: &str, _account: Option<&str>) -> Result<Page<Track>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::ArtistTopTracks,
        ))
    }

    async fn set_artist_subscription(
        &self,
        _id: &str,
        _subscribed: bool,
        _account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::ArtistSubscriptionWrite,
        ))
    }

    async fn playlist(&self, _id: &str, _account: Option<&str>) -> Result<Playlist> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PlaylistRead,
        ))
    }

    async fn playlist_tracks(&self, _id: &str, _request: &PageRequest) -> Result<Page<Track>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PlaylistRead,
        ))
    }

    async fn create_playlist(
        &self,
        _request: &PlaylistCreateRequest,
    ) -> Result<PlaylistMutationResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PlaylistWrite,
        ))
    }

    async fn update_playlist(
        &self,
        _id: &str,
        _request: &PlaylistUpdateRequest,
    ) -> Result<PlaylistMutationResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PlaylistWrite,
        ))
    }

    async fn delete_playlists(
        &self,
        _request: &PlaylistDeleteRequest,
    ) -> Result<PlaylistDeleteResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PlaylistWrite,
        ))
    }

    async fn mutate_playlist_items(
        &self,
        _id: &str,
        _action: PlaylistItemMutationAction,
        _request: &PlaylistItemMutationRequest,
    ) -> Result<PlaylistItemMutationResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PlaylistWrite,
        ))
    }

    async fn reorder_playlist_tracks(
        &self,
        _id: &str,
        _request: &PlaylistTrackOrderRequest,
    ) -> Result<PlaylistTrackOrderResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PlaylistWrite,
        ))
    }

    async fn reorder_account_playlists(
        &self,
        _request: &PlaylistOrderRequest,
    ) -> Result<PlaylistOrderResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PlaylistWrite,
        ))
    }

    async fn update_playlist_cover(
        &self,
        _id: &str,
        _request: &ImageUploadRequest,
    ) -> Result<PlaylistCoverUpdateResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PlaylistWrite,
        ))
    }

    async fn account_playlists(&self, _request: &PageRequest) -> Result<Page<Playlist>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountPlaylists,
        ))
    }

    async fn account_albums(&self, _request: &PageRequest) -> Result<Page<Album>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountAlbums,
        ))
    }

    async fn account_radio_stations(&self, _request: &PageRequest) -> Result<Page<RadioStation>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountRadioStations,
        ))
    }

    async fn account_following_artists(&self, _request: &PageRequest) -> Result<Page<Artist>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountFollowingArtists,
        ))
    }

    async fn account_artist_new_videos(
        &self,
        _request: &ArtistUpdatesRequest,
    ) -> Result<Page<Video>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountArtistNewVideos,
        ))
    }

    async fn account_artist_new_tracks(
        &self,
        _request: &ArtistUpdatesRequest,
    ) -> Result<Page<Track>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountArtistNewTracks,
        ))
    }

    async fn account_artist_new_works(
        &self,
        _request: &ArtistWorksRequest,
    ) -> Result<Page<ArtistWorkUpdate>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountArtistNewWorks,
        ))
    }

    async fn account_artist_new_tracks_play_all(
        &self,
        _account: Option<&str>,
    ) -> Result<Page<Track>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountArtistNewTracksPlayAll,
        ))
    }

    async fn favorite_tracks(&self, _request: &PageRequest) -> Result<Page<Track>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::Favorites,
        ))
    }

    async fn user_favorite_tracks(
        &self,
        _user_id: &str,
        _request: &PageRequest,
    ) -> Result<Page<Track>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::Favorites,
        ))
    }

    async fn account_history(
        &self,
        _request: &PlaybackHistoryRequest,
    ) -> Result<Page<PlaybackHistoryEntry>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::ListeningHistory,
        ))
    }

    async fn user_history(
        &self,
        _user_id: &str,
        _request: &PlaybackHistoryRequest,
    ) -> Result<Page<PlaybackHistoryEntry>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::ListeningHistory,
        ))
    }

    async fn recommended_tracks(&self, _request: &RecommendationRequest) -> Result<Page<Track>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::Recommendations,
        ))
    }

    async fn recommended_playlists(
        &self,
        _request: &RecommendationRequest,
    ) -> Result<Page<Playlist>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::Recommendations,
        ))
    }

    async fn lyrics(&self, _id: &str, _account: Option<&str>) -> Result<Lyrics> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::Lyrics,
        ))
    }

    async fn stream(&self, _track: &Track, _request: &StreamRequest) -> Result<MediaStream> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AudioStream,
        ))
    }

    async fn streams(&self, tracks: &[Track], request: &StreamRequest) -> Result<StreamBatch> {
        let mut outcomes = Vec::with_capacity(tracks.len());
        for track in tracks {
            match self.stream(track, request).await {
                Ok(stream) => outcomes.push(StreamOutcome {
                    track_ref: track.resource_ref.clone(),
                    status: ResolutionStatus::Success,
                    stream: Some(stream),
                    error_code: None,
                    error: None,
                    extensions: Extensions::new(),
                }),
                Err(error) => outcomes.push(StreamOutcome {
                    track_ref: track.resource_ref.clone(),
                    status: stream_error_status(error.code),
                    stream: None,
                    error_code: Some(error.code),
                    error: Some(error.message),
                    extensions: Extensions::from([("details".to_owned(), error.details)]),
                }),
            }
        }
        Ok(StreamBatch {
            outcomes,
            extensions: Extensions::new(),
        })
    }

    async fn download(&self, _track: &Track, _request: &StreamRequest) -> Result<MediaDownload> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AudioDownload,
        ))
    }

    async fn start_qr_login(&self, _login_type: Option<&str>) -> Result<ProviderQrStart> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::QrLogin,
        ))
    }

    async fn poll_qr_login(
        &self,
        _provider_transaction_id: &str,
        _account: &str,
    ) -> Result<ProviderQrPoll> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::QrLogin,
        ))
    }

    async fn password_login(&self, _request: &PasswordLoginRequest) -> Result<AccountProfile> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PasswordLogin,
        ))
    }

    async fn start_auth_challenge(&self, _request: &AuthChallengeRequest) -> Result<()> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PhoneLogin,
        ))
    }

    async fn validate_auth_challenge(
        &self,
        _request: &AuthChallengeRequest,
        _code: &str,
    ) -> Result<AuthChallengeValidation> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::ChallengeValidation,
        ))
    }

    async fn country_calling_codes(
        &self,
        _request: &CountryCallingCodeListRequest,
    ) -> Result<Vec<CountryCallingCodeGroup>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::CountryCallingCodes,
        ))
    }

    async fn auth_principal_status(
        &self,
        _request: &AuthPrincipalStatusRequest,
    ) -> Result<AuthPrincipalStatus> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PrincipalStatus,
        ))
    }

    async fn verify_auth_challenge(
        &self,
        _request: &AuthChallengeRequest,
        _code: &str,
    ) -> Result<AccountProfile> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PhoneLogin,
        ))
    }

    async fn logout(&self, _account: &str) -> Result<bool> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::SessionManagement,
        ))
    }

    async fn session_profile(&self, _account: &str) -> Result<AccountProfile> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::SessionManagement,
        ))
    }

    async fn refresh_session(&self, _account: &str) -> Result<AccountProfile> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::SessionManagement,
        ))
    }

    async fn upload_account_avatar(
        &self,
        _request: &ImageUploadRequest,
    ) -> Result<ImageUploadResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountAvatarWrite,
        ))
    }

    async fn upload_cloud_track(&self, _request: &CloudUploadRequest) -> Result<CloudUploadResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountCloudUpload,
        ))
    }

    async fn cloud_upload_ticket(
        &self,
        _request: &CloudUploadTicketRequest,
    ) -> Result<CloudUploadTicket> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountCloudDirectUpload,
        ))
    }

    async fn complete_cloud_upload(
        &self,
        _request: &CloudUploadCompleteRequest,
    ) -> Result<CloudUploadResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountCloudDirectUpload,
        ))
    }

    async fn import_cloud_track(&self, _request: &CloudImportRequest) -> Result<CloudImportResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountCloudImport,
        ))
    }

    async fn cloud_lyrics(&self, _request: &CloudLyricsRequest) -> Result<Lyrics> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountCloudLyrics,
        ))
    }

    async fn match_cloud_track(&self, _request: &CloudMatchRequest) -> Result<CloudMatchResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountCloudMatch,
        ))
    }

    async fn cloud_tracks(&self, _request: &PageRequest) -> Result<Page<CloudTrack>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountCloudRead,
        ))
    }

    async fn cloud_track_details(
        &self,
        _request: &CloudTrackDetailRequest,
    ) -> Result<Vec<CloudTrack>> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountCloudRead,
        ))
    }

    async fn delete_cloud_tracks(
        &self,
        _request: &CloudTrackDeleteRequest,
    ) -> Result<CloudTrackDeleteResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountCloudDelete,
        ))
    }

    async fn download_cloud_track(
        &self,
        _id: &str,
        _account: Option<&str>,
    ) -> Result<MediaDownload> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::AccountCloudDownload,
        ))
    }

    async fn post_comment(&self, _request: &CommentWriteRequest) -> Result<CommentMutationResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::CommentWrite,
        ))
    }

    async fn delete_comment(
        &self,
        _request: &CommentDeleteRequest,
    ) -> Result<CommentMutationResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::CommentWrite,
        ))
    }

    async fn comments(&self, _request: &CommentListRequest) -> Result<CommentPage> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::CommentsRead,
        ))
    }

    async fn comment_reactions(
        &self,
        _request: &CommentReactionListRequest,
    ) -> Result<CommentReactionPage> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::CommentReactionsRead,
        ))
    }

    async fn set_comment_reaction(
        &self,
        _request: &CommentReactionMutationRequest,
    ) -> Result<CommentReactionMutationResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::CommentReactionsWrite,
        ))
    }

    async fn report_comment(&self, _request: &CommentReportRequest) -> Result<CommentReportResult> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::CommentReportsWrite,
        ))
    }

    async fn comment_thread_stats(
        &self,
        _request: &CommentThreadStatsRequest,
    ) -> Result<CommentThreadStatsBatch> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::CommentThreadStats,
        ))
    }

    async fn platform_api(&self, _request: &PlatformApiRequest) -> Result<serde_json::Value> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PlatformApi,
        ))
    }

    async fn platform_batch(&self, _request: &PlatformBatchRequest) -> Result<serde_json::Value> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PlatformBatch,
        ))
    }
}

fn search_capability(kind: SearchKind) -> Capability {
    match kind {
        SearchKind::Track => Capability::SearchTracks,
        SearchKind::Album => Capability::SearchAlbums,
        SearchKind::Artist => Capability::SearchArtists,
        SearchKind::Playlist => Capability::SearchPlaylists,
        SearchKind::User => Capability::SearchUsers,
        SearchKind::Mv => Capability::SearchMvs,
        SearchKind::Lyric => Capability::SearchLyrics,
        SearchKind::RadioStation => Capability::SearchRadioStations,
        SearchKind::Video => Capability::SearchVideos,
        SearchKind::Mixed => Capability::SearchMixed,
        SearchKind::Voice => Capability::SearchVoices,
    }
}

fn stream_error_status(code: ErrorCode) -> ResolutionStatus {
    match code {
        ErrorCode::AuthenticationRequired => ResolutionStatus::AuthenticationRequired,
        ErrorCode::PermissionDenied => ResolutionStatus::PermissionDenied,
        ErrorCode::MatchRejected => ResolutionStatus::NoMatch,
        ErrorCode::CapabilityNotSupported
        | ErrorCode::PlatformUnavailable
        | ErrorCode::ResourceNotFound => ResolutionStatus::Unavailable,
        ErrorCode::InvalidRequest
        | ErrorCode::Conflict
        | ErrorCode::RateLimited
        | ErrorCode::UpstreamError
        | ErrorCode::UpstreamTimeout
        | ErrorCode::InternalError => ResolutionStatus::UpstreamError,
    }
}
