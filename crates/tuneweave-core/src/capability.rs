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
    PlaylistWrite,
    Favorites,
    ListeningHistory,
    Recommendations,
    MusicPartner,
    PlatformApi,
}
