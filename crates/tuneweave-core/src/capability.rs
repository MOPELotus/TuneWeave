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
    SearchVideos,
    SearchMixed,
    SearchVoices,
    SearchDefault,
    SearchTrending,
    SearchSuggestions,
    SearchMultiMatch,
    SearchLocalTrackMatch,
    UserMembership,
    AudioRecognition,
    Banners,
    RadioTaxonomy,
    RadioStationDetail,
    RadioStationList,
    RadioStationSubscriptionWrite,
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
    VideoStream,
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
    AccountRadioStations,
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
    PlaylistWrite,
    Favorites,
    ListeningHistory,
    Recommendations,
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
}
