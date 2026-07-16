use std::collections::BTreeSet;

use async_trait::async_trait;

use crate::{
    AccountProfile, Album, AlbumListRequest, AlbumStats, Artist, ArtistStats, AuthChallengeRequest,
    Capability, DigitalAlbum, DigitalAlbumChartEntry, DigitalAlbumChartRequest,
    DigitalAlbumListRequest, Lyrics, MediaStream, Page, PageRequest, PasswordLoginRequest,
    Platform, PlatformApiRequest, PlaybackHistoryEntry, PlaybackHistoryRequest, Playlist,
    ProviderDescriptor, ProviderQrPoll, ProviderQrStart, RecommendationRequest, Result,
    SearchQuery, StreamRequest, SubscriptionResult, Track, TrackEntitlement, TuneWeaveError, User,
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

    async fn track(&self, _id: &str, _account: Option<&str>) -> Result<Track> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::TrackDetail,
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

    async fn artist(&self, _id: &str, _account: Option<&str>) -> Result<Artist> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::ArtistDetail,
        ))
    }

    async fn artist_stats(&self, _id: &str, _account: Option<&str>) -> Result<ArtistStats> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::ArtistStats,
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

    async fn platform_api(&self, _request: &PlatformApiRequest) -> Result<serde_json::Value> {
        Err(TuneWeaveError::unsupported(
            self.platform(),
            Capability::PlatformApi,
        ))
    }
}
