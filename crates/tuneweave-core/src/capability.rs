use serde::{Deserialize, Serialize};

/// A provider feature that can be advertised through service discovery.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    SearchTracks,
    SearchAlbums,
    SearchArtists,
    SearchPlaylists,
    SearchUsers,
    SearchMvs,
    SearchLyrics,
    SearchRadioStations,
    SearchPodcasts,
    SearchVideos,
    SearchMixed,
    SearchVoices,
    SearchDefault,
    SearchTrending,
    SearchSuggestions,
    SearchMultiMatch,
    SearchLocalTrackMatch,
    UserProfileLegacy,
    UserProfileModern,
    UserMembership,
    UserMembershipClientInfo,
    AnonymousSession,
    AntiCheatToken,
    ListeningRightsAds,
    ListeningRightsGain,
    AudioRecognition,
    Banners,
    RadioTaxonomy,
    RadioStationDetail,
    RadioStationList,
    RadioStationSubscriptionWrite,
    PodcastCategories,
    PodcastCategoryRecommendations,
    PodcastList,
    PodcastCharts,
    PodcastCreatorCharts,
    PodcastDetail,
    PodcastWorkbenchDetail,
    PodcastSubscriptionWrite,
    PodcastEpisodeList,
    PodcastEpisodeWorkbenchList,
    PodcastEpisodeWorkbenchSearch,
    PodcastEpisodeCharts,
    PodcastEpisodeDetail,
    PodcastEpisodeWorkbenchDetail,
    PodcastEpisodeStream,
    PodcastEpisodeLyrics,
    TrackDetail,
    TrackAvailability,
    AlbumDetail,
    AlbumList,
    AlbumStats,
    AlbumTrackEntitlements,
    AlbumSubscriptionWrite,
    DigitalAlbumDetail,
    DigitalAlbumList,
    DigitalAlbumCharts,
    ChartCatalog,
    ArtistCharts,
    DimensionCharts,
    ArtistDetail,
    ArtistOverview,
    ArtistStats,
    ArtistList,
    ArtistAlbums,
    ArtistFans,
    ArtistVideos,
    ArtistTracks,
    ArtistTopTracks,
    ArtistSubscriptionWrite,
    PlaylistRead,
    Lyrics,
    AudioStream,
    AudioStreamBatch,
    AudioDownload,
    VideoCatalog,
    VideoTaxonomy,
    VideoDetail,
    VideoStats,
    VideoStream,
    VideoSubscriptionWrite,
    QrLogin,
    PasswordLogin,
    PhoneLogin,
    CountryCallingCodes,
    ChallengeValidation,
    PrincipalStatus,
    SessionManagement,
    AccountProfile,
    AccountPlaylists,
    AccountAlbums,
    AccountVideos,
    AccountRadioStations,
    AccountPodcasts,
    AccountCreatedPodcasts,
    AccountFollowingArtists,
    AccountArtistNewVideos,
    AccountArtistNewTracks,
    AccountArtistNewWorks,
    AccountArtistNewTracksPlayAll,
    AccountAvatarWrite,
    AccountCloudUpload,
    AccountCloudDirectUpload,
    AccountCloudImport,
    AccountCloudLyrics,
    AccountCloudMatch,
    AccountCloudRead,
    AccountCloudDelete,
    AccountCloudDownload,
    PlaylistWrite,
    Favorites,
    ListeningHistory,
    Recommendations,
    VideoRecommendations,
    PodcastEpisodeRecommendations,
    PersonalFm,
    RecommendationFeedback,
    MusicPartner,
    CommentWrite,
    CommentsRead,
    CommentReactionsRead,
    CommentReactionsWrite,
    CommentReportsWrite,
    CommentThreadStats,
    PlatformApi,
    PlatformBatch,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radio_station_capabilities_use_stable_discovery_names() {
        assert_eq!(
            serde_json::to_value(Capability::SearchDefault)
                .expect("serialize default search capability"),
            serde_json::json!("search_default")
        );
        assert_eq!(
            serde_json::to_value(Capability::SearchTrending)
                .expect("serialize trending search capability"),
            serde_json::json!("search_trending")
        );
        assert_eq!(
            serde_json::to_value(Capability::SearchSuggestions)
                .expect("serialize search suggestions capability"),
            serde_json::json!("search_suggestions")
        );
        assert_eq!(
            serde_json::to_value(Capability::SearchMultiMatch)
                .expect("serialize multi-match search capability"),
            serde_json::json!("search_multi_match")
        );
        assert_eq!(
            serde_json::to_value(Capability::SearchLocalTrackMatch)
                .expect("serialize local track match capability"),
            serde_json::json!("search_local_track_match")
        );
        assert_eq!(
            serde_json::to_value(Capability::UserMembership)
                .expect("serialize user membership capability"),
            serde_json::json!("user_membership")
        );
        assert_eq!(
            serde_json::to_value(Capability::UserMembershipClientInfo)
                .expect("serialize client membership capability"),
            serde_json::json!("user_membership_client_info")
        );
        assert_eq!(
            serde_json::to_value(Capability::AnonymousSession)
                .expect("serialize anonymous session capability"),
            serde_json::json!("anonymous_session")
        );
        assert_eq!(
            serde_json::to_value(Capability::AntiCheatToken)
                .expect("serialize anti-cheat token capability"),
            serde_json::json!("anti_cheat_token")
        );
        assert_eq!(
            serde_json::to_value(Capability::ListeningRightsAds)
                .expect("serialize listening-rights ad capability"),
            serde_json::json!("listening_rights_ads")
        );
        assert_eq!(
            serde_json::to_value(Capability::ListeningRightsGain)
                .expect("serialize listening-rights gain capability"),
            serde_json::json!("listening_rights_gain")
        );
        assert_eq!(
            serde_json::to_value(Capability::PersonalFm).expect("serialize personal FM capability"),
            serde_json::json!("personal_fm")
        );
        assert_eq!(
            serde_json::to_value(Capability::VideoRecommendations)
                .expect("serialize video recommendations capability"),
            serde_json::json!("video_recommendations")
        );
        assert_eq!(
            serde_json::to_value(Capability::PodcastEpisodeRecommendations)
                .expect("serialize podcast episode recommendations capability"),
            serde_json::json!("podcast_episode_recommendations")
        );
        assert_eq!(
            serde_json::to_value(Capability::RecommendationFeedback)
                .expect("serialize recommendation feedback capability"),
            serde_json::json!("recommendation_feedback")
        );
        assert_eq!(
            serde_json::to_value(Capability::AudioStreamBatch)
                .expect("serialize batch audio stream capability"),
            serde_json::json!("audio_stream_batch")
        );
        assert_eq!(
            serde_json::to_value(Capability::AudioDownload)
                .expect("serialize audio download capability"),
            serde_json::json!("audio_download")
        );
        assert_eq!(
            serde_json::to_value(Capability::ChartCatalog)
                .expect("serialize chart catalog capability"),
            serde_json::json!("chart_catalog")
        );
        assert_eq!(
            serde_json::to_value(Capability::ArtistCharts)
                .expect("serialize artist charts capability"),
            serde_json::json!("artist_charts")
        );
        assert_eq!(
            serde_json::to_value(Capability::VideoDetail)
                .expect("serialize video detail capability"),
            serde_json::json!("video_detail")
        );
        assert_eq!(
            serde_json::to_value(Capability::VideoStats).expect("serialize video stats capability"),
            serde_json::json!("video_stats")
        );
        assert_eq!(
            serde_json::to_value(Capability::VideoStream)
                .expect("serialize video stream capability"),
            serde_json::json!("video_stream")
        );
        assert_eq!(
            serde_json::to_value(Capability::VideoSubscriptionWrite)
                .expect("serialize video subscription capability"),
            serde_json::json!("video_subscription_write")
        );
        assert_eq!(
            serde_json::to_value(Capability::VideoTaxonomy)
                .expect("serialize video taxonomy capability"),
            serde_json::json!("video_taxonomy")
        );
        assert_eq!(
            serde_json::to_value(Capability::AccountVideos)
                .expect("serialize account videos capability"),
            serde_json::json!("account_videos")
        );
        assert_eq!(
            serde_json::to_value(Capability::RadioStationDetail).expect("serialize capability"),
            serde_json::json!("radio_station_detail")
        );
        assert_eq!(
            serde_json::to_value(Capability::RadioStationList).expect("serialize capability"),
            serde_json::json!("radio_station_list")
        );
        assert_eq!(
            serde_json::to_value(Capability::RadioStationSubscriptionWrite)
                .expect("serialize capability"),
            serde_json::json!("radio_station_subscription_write")
        );
        assert_eq!(
            serde_json::to_value(Capability::PodcastCategories).expect("serialize capability"),
            serde_json::json!("podcast_categories")
        );
        assert_eq!(
            serde_json::to_value(Capability::PodcastCategoryRecommendations)
                .expect("serialize capability"),
            serde_json::json!("podcast_category_recommendations")
        );
        assert_eq!(
            serde_json::to_value(Capability::PodcastList).expect("serialize capability"),
            serde_json::json!("podcast_list")
        );
        assert_eq!(
            serde_json::to_value(Capability::PodcastCharts).expect("serialize capability"),
            serde_json::json!("podcast_charts")
        );
        assert_eq!(
            serde_json::to_value(Capability::PodcastCreatorCharts).expect("serialize capability"),
            serde_json::json!("podcast_creator_charts")
        );
        assert_eq!(
            serde_json::to_value(Capability::PodcastDetail).expect("serialize capability"),
            serde_json::json!("podcast_detail")
        );
        assert_eq!(
            serde_json::to_value(Capability::PodcastSubscriptionWrite)
                .expect("serialize capability"),
            serde_json::json!("podcast_subscription_write")
        );
        assert_eq!(
            serde_json::to_value(Capability::AccountPodcasts).expect("serialize capability"),
            serde_json::json!("account_podcasts")
        );
        assert_eq!(
            serde_json::to_value(Capability::AccountCreatedPodcasts).expect("serialize capability"),
            serde_json::json!("account_created_podcasts")
        );
        assert_eq!(
            serde_json::to_value(Capability::PodcastEpisodeList).expect("serialize capability"),
            serde_json::json!("podcast_episode_list")
        );
        assert_eq!(
            serde_json::to_value(Capability::PodcastEpisodeWorkbenchList)
                .expect("serialize capability"),
            serde_json::json!("podcast_episode_workbench_list")
        );
        assert_eq!(
            serde_json::to_value(Capability::PodcastEpisodeWorkbenchSearch)
                .expect("serialize capability"),
            serde_json::json!("podcast_episode_workbench_search")
        );
        assert_eq!(
            serde_json::to_value(Capability::PodcastEpisodeCharts).expect("serialize capability"),
            serde_json::json!("podcast_episode_charts")
        );
        assert_eq!(
            serde_json::to_value(Capability::PodcastEpisodeDetail).expect("serialize capability"),
            serde_json::json!("podcast_episode_detail")
        );
        assert_eq!(
            serde_json::to_value(Capability::PodcastEpisodeWorkbenchDetail)
                .expect("serialize capability"),
            serde_json::json!("podcast_episode_workbench_detail")
        );
        assert_eq!(
            serde_json::to_value(Capability::PodcastWorkbenchDetail).expect("serialize capability"),
            serde_json::json!("podcast_workbench_detail")
        );
        assert_eq!(
            serde_json::to_value(Capability::PodcastEpisodeStream).expect("serialize capability"),
            serde_json::json!("podcast_episode_stream")
        );
        assert_eq!(
            serde_json::to_value(Capability::PodcastEpisodeLyrics).expect("serialize capability"),
            serde_json::json!("podcast_episode_lyrics")
        );
        assert_eq!(
            serde_json::to_value(Capability::ChallengeValidation).expect("serialize capability"),
            serde_json::json!("challenge_validation")
        );
        assert_eq!(
            serde_json::to_value(Capability::PrincipalStatus).expect("serialize capability"),
            serde_json::json!("principal_status")
        );
        assert_eq!(
            serde_json::to_value(Capability::CountryCallingCodes)
                .expect("serialize country calling codes capability"),
            serde_json::json!("country_calling_codes")
        );
        assert_eq!(
            serde_json::to_value(Capability::SearchRadioStations)
                .expect("serialize search capability"),
            serde_json::json!("search_radio_stations")
        );
        assert_eq!(
            serde_json::to_value(Capability::SearchPodcasts)
                .expect("serialize podcast search capability"),
            serde_json::json!("search_podcasts")
        );
        assert_eq!(
            serde_json::to_value(Capability::SearchVoices).expect("serialize search capability"),
            serde_json::json!("search_voices")
        );
        assert_eq!(
            serde_json::to_value(Capability::CommentWrite).expect("serialize comment capability"),
            serde_json::json!("comment_write")
        );
        assert_eq!(
            serde_json::to_value(Capability::CommentsRead).expect("serialize comments capability"),
            serde_json::json!("comments_read")
        );
        assert_eq!(
            serde_json::to_value(Capability::CommentReactionsRead)
                .expect("serialize comment reactions capability"),
            serde_json::json!("comment_reactions_read")
        );
        assert_eq!(
            serde_json::to_value(Capability::CommentReactionsWrite)
                .expect("serialize comment reaction write capability"),
            serde_json::json!("comment_reactions_write")
        );
        assert_eq!(
            serde_json::to_value(Capability::CommentReportsWrite)
                .expect("serialize comment report write capability"),
            serde_json::json!("comment_reports_write")
        );
        assert_eq!(
            serde_json::to_value(Capability::CommentThreadStats)
                .expect("serialize comment thread stats capability"),
            serde_json::json!("comment_thread_stats")
        );
    }

    #[test]
    fn cloud_library_capabilities_use_stable_discovery_names() {
        assert_eq!(
            serde_json::to_value(Capability::AccountCloudRead)
                .expect("serialize cloud read capability"),
            serde_json::json!("account_cloud_read")
        );
        assert_eq!(
            serde_json::to_value(Capability::AccountCloudDelete)
                .expect("serialize cloud delete capability"),
            serde_json::json!("account_cloud_delete")
        );
        assert_eq!(
            serde_json::to_value(Capability::AccountCloudDownload)
                .expect("serialize cloud download capability"),
            serde_json::json!("account_cloud_download")
        );
    }

    #[test]
    fn user_profile_backends_keep_distinct_discovery_names() {
        assert_eq!(
            serde_json::to_value(Capability::UserProfileLegacy)
                .expect("serialize legacy user profile capability"),
            serde_json::json!("user_profile_legacy")
        );
        assert_eq!(
            serde_json::to_value(Capability::UserProfileModern)
                .expect("serialize modern user profile capability"),
            serde_json::json!("user_profile_modern")
        );
        assert_eq!(
            serde_json::to_value(Capability::VideoCatalog)
                .expect("serialize music video catalog capability"),
            serde_json::json!("video_catalog")
        );
    }
}
