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
    ArtistDetail,
    PlaylistRead,
    Lyrics,
    AudioStream,
    VideoStream,
    QrLogin,
    PhoneLogin,
    AccountProfile,
    AccountPlaylists,
    PlaylistWrite,
    Favorites,
    Recommendations,
    MusicPartner,
}
