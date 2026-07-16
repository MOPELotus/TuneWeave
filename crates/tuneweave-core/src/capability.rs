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
    VideoStream,
    QrLogin,
    PasswordLogin,
    PhoneLogin,
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
    PlatformApi,
    PlatformBatch,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn radio_station_capabilities_use_stable_discovery_names() {
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
    }
}
