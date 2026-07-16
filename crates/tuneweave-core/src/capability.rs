use serde::{Deserialize, Serialize};

/// A provider feature that can be advertised through service discovery.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    SearchTracks,
    SearchAlbums,
    SearchArtists,
    SearchPlaylists,
    SearchVideos,
    AudioRecognition,
    Banners,
    RadioTaxonomy,
    RadioStationDetail,
    RadioStationList,
    TrackDetail,
    AlbumDetail,
    AlbumList,
    AlbumStats,
    AlbumTrackEntitlements,
    AlbumSubscriptionWrite,
    DigitalAlbumDetail,
    DigitalAlbumList,
    DigitalAlbumCharts,
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
    PlaylistWrite,
    Favorites,
    ListeningHistory,
    Recommendations,
    MusicPartner,
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
    }
}
