use std::{
    collections::BTreeSet,
    fmt,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use serde_json::json;
use tuneweave_core::{
    AccountCredentialStore, AccountProfile, Album, Capability, CredentialImportRequest,
    CredentialMode, DigitalAlbum, Extensions, Lyrics, LyricsRequest, MediaDownload, MediaStream,
    MusicProvider, Page, PageMeta, PageRequest, Platform, Playlist, ProviderAuthResult,
    ProviderCredential, ProviderLogoutResult, Result, SearchItem, SearchKind, SearchQuery,
    SearchVariant, StreamRequest, Track, TrackAvailability, TrackAvailabilityRequest,
    TuneWeaveError, UserProfileBackend,
};

use crate::client::{MiguClient, MiguConfig, MiguSearchCondition};

mod account_avatar;
mod account_download;
mod account_media;
mod account_playlists;
mod account_read;
mod album_collections;
mod albums;
mod artist_directory;
mod artist_subscription;
mod artists;
mod catalog;
mod charts;
mod favorites;
mod following_artists;
mod library;
mod membership;
mod password;
mod playlist_collection;
mod playlist_write;
mod purchase_source;
mod purchased_album_source;
mod purchased_albums;
mod purchases;
mod search_suggestions;
mod search_trending;
mod session;
mod similar_artists;
mod sms;
mod user_profile;
mod video_playback;
mod videos;

const UPSTREAM_PAGE_SIZE: u32 = 20;
const MAX_UPSTREAM_PAGES: u32 = 6;
const UPSTREAM_PLAYLIST_PAGE_SIZE: u32 = 50;
const MAX_UPSTREAM_PLAYLIST_PAGES: u32 = 3;

#[derive(Clone)]
pub struct MiguProvider {
    client: MiguClient,
    credential_store: Option<Arc<dyn AccountCredentialStore>>,
    caller_credential: Option<Arc<Mutex<crate::credential::MiguCredential>>>,
    response_credential: Arc<Mutex<Option<ProviderCredential>>>,
    auth_mutation: Arc<tokio::sync::Mutex<()>>,
    passport_transactions: Arc<Mutex<sms::PassportTransactions>>,
}

impl fmt::Debug for MiguProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MiguProvider")
            .finish_non_exhaustive()
    }
}

impl MiguProvider {
    pub fn new(config: MiguConfig) -> Result<Self> {
        Ok(Self {
            client: MiguClient::new(&config)?,
            credential_store: config.credential_store.clone(),
            caller_credential: None,
            response_credential: Arc::default(),
            auth_mutation: Arc::default(),
            passport_transactions: Arc::default(),
        })
    }

    #[must_use]
    pub fn from_client(client: MiguClient) -> Self {
        Self {
            client,
            credential_store: None,
            caller_credential: None,
            response_credential: Arc::default(),
            auth_mutation: Arc::default(),
            passport_transactions: Arc::default(),
        }
    }
}

#[async_trait]
impl MusicProvider for MiguProvider {
    fn platform(&self) -> Platform {
        Platform::Migu
    }

    fn name(&self) -> &'static str {
        "Migu Music"
    }

    async fn begin_password_login(
        &self,
        request: &tuneweave_core::PasswordLoginRequest,
        mode: CredentialMode,
    ) -> Result<tuneweave_core::PasswordLoginProgress> {
        self.begin_password(request, mode).await
    }

    async fn advance_password_login(
        &self,
        challenge: &tuneweave_core::ProviderPasswordChallenge,
        action: &tuneweave_core::PasswordChallengeAction,
    ) -> Result<tuneweave_core::PasswordLoginProgress> {
        self.advance_password(challenge, action).await
    }

    async fn begin_auth_challenge(
        &self,
        request: &tuneweave_core::AuthChallengeRequest,
        mode: CredentialMode,
    ) -> Result<tuneweave_core::ProviderAuthChallenge> {
        self.begin_sms(request, mode).await
    }

    async fn complete_auth_challenge(
        &self,
        challenge: &tuneweave_core::ProviderAuthChallenge,
        code: &str,
    ) -> Result<ProviderAuthResult> {
        self.complete_sms(challenge, code).await
    }

    async fn auth_challenge_status(
        &self,
        challenge: &tuneweave_core::ProviderAuthChallenge,
    ) -> Result<tuneweave_core::AuthChallengeStatus> {
        self.sms_status(challenge)
    }

    async fn advance_auth_challenge(
        &self,
        challenge: &tuneweave_core::ProviderAuthChallenge,
        action: &tuneweave_core::AuthChallengeAction,
    ) -> Result<tuneweave_core::AuthChallengeProgress> {
        match action {
            tuneweave_core::AuthChallengeAction::SubmitCode { code } => {
                self.submit_sms(challenge, code).await
            }
            _ => self.advance_sms_image(challenge, action).await,
        }
    }

    fn with_caller_credential(
        &self,
        credential: &ProviderCredential,
    ) -> Result<Arc<dyn MusicProvider>> {
        Ok(Arc::new(self.caller_scope(credential)?))
    }
    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        Ok(self
            .response_credential
            .lock()
            .map_err(|_| {
                crate::credential::error(
                    tuneweave_core::ErrorCode::InternalError,
                    "Migu caller response state is unavailable",
                )
            })?
            .take())
    }
    async fn password_login(
        &self,
        request: &tuneweave_core::PasswordLoginRequest,
    ) -> Result<AccountProfile> {
        Ok(self
            .password_session(request, CredentialMode::Server)
            .await?
            .profile)
    }
    async fn password_login_with_mode(
        &self,
        request: &tuneweave_core::PasswordLoginRequest,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        self.password_session(request, mode).await
    }
    async fn import_credential(
        &self,
        request: &CredentialImportRequest,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        self.import_session(request, mode).await
    }
    async fn session_profile(&self, account: &str) -> Result<AccountProfile> {
        self.read_session_profile(account).await
    }
    async fn user_profile(
        &self,
        id: &str,
        backend: UserProfileBackend,
        account: Option<&str>,
    ) -> Result<tuneweave_core::UserProfile> {
        self.read_user_profile(id, backend, account).await
    }
    async fn user_membership(
        &self,
        id: Option<&str>,
        account: Option<&str>,
    ) -> Result<tuneweave_core::MembershipSummary> {
        self.read_membership(id, account, false).await
    }
    async fn user_membership_client_info(
        &self,
        id: Option<&str>,
        account: Option<&str>,
    ) -> Result<tuneweave_core::MembershipSummary> {
        self.read_membership(id, account, true).await
    }
    async fn refresh_session(&self, account: &str) -> Result<AccountProfile> {
        Ok(self
            .refresh_owned_session(account, None, CredentialMode::Server)
            .await?
            .profile)
    }
    async fn refresh_session_with_ownership(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        self.refresh_owned_session(account, source, mode).await
    }
    async fn logout(&self, account: &str) -> Result<bool> {
        Ok(self
            .logout_owned_session(account, None, CredentialMode::Server)
            .await?
            .removed)
    }
    async fn logout_with_ownership(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<ProviderLogoutResult> {
        self.logout_owned_session(account, source, mode).await
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::PasswordLogin,
            Capability::PhoneLogin,
            Capability::CredentialImport,
            Capability::SessionManagement,
            Capability::CallerManagedCredentials,
            Capability::AccountProfile,
            Capability::AccountAvatarWrite,
            Capability::UserProfileModern,
            Capability::AccountPurchasedTracks,
            Capability::AccountPurchasedAlbums,
            Capability::AccountPlaylists,
            Capability::AccountAlbums,
            Capability::AccountDigitalAlbums,
            Capability::AccountFollowingArtists,
            Capability::ArtistSubscriptionWrite,
            Capability::AlbumSubscriptionWrite,
            Capability::DigitalAlbumSubscriptionWrite,
            Capability::Favorites,
            Capability::TrackSubscriptionWrite,
            Capability::PlaylistSubscriptionWrite,
            Capability::PlaylistWrite,
            Capability::PlaylistOccurrenceRead,
            Capability::PlaylistOccurrenceWrite,
            Capability::UserMembership,
            Capability::UserMembershipClientInfo,
            Capability::AudioDownload,
            Capability::AudioStream,
            Capability::Lyrics,
            Capability::PlaylistRead,
            Capability::SearchTracks,
            Capability::SearchPlaylists,
            Capability::SearchArtists,
            Capability::SearchSuggestions,
            Capability::SearchTrending,
            Capability::ArtistCatalog,
            Capability::ArtistDetail,
            Capability::ArtistOverview,
            Capability::SimilarArtists,
            Capability::ArtistTracks,
            Capability::ArtistAlbums,
            Capability::ArtistDigitalAlbums,
            Capability::ArtistVideos,
            Capability::SearchMvs,
            Capability::VideoDetail,
            Capability::VideoStats,
            Capability::VideoStream,
            Capability::SearchAlbums,
            Capability::AlbumDetail,
            Capability::DigitalAlbumDetail,
            Capability::DigitalAlbumTracks,
            Capability::TrackAvailability,
            Capability::TrackDetail,
            Capability::ChartCatalog,
            Capability::ChartTracks,
            Capability::ChartHistoricalTracks,
        ])
    }

    async fn chart_catalog(
        &self,
        request: &tuneweave_core::ChartCatalogRequest,
    ) -> Result<tuneweave_core::ChartCatalog> {
        self.read_chart_catalogue(request).await
    }

    async fn chart_tracks(
        &self,
        id: &str,
        request: &tuneweave_core::ChartTrackListRequest,
    ) -> Result<Page<Track>> {
        self.read_chart_tracks(id, request).await
    }

    async fn set_track_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<tuneweave_core::SubscriptionResult> {
        self.set_favorite_track(id, subscribed, account).await
    }

    async fn account_albums(&self, request: &PageRequest) -> Result<Page<Album>> {
        self.ordinary_album_collections(None, request).await
    }

    async fn account_following_artists(
        &self,
        request: &PageRequest,
    ) -> Result<Page<tuneweave_core::Artist>> {
        self.read_following_artists(None, request).await
    }

    async fn set_artist_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<tuneweave_core::SubscriptionResult> {
        self.change_artist_subscription(id, subscribed, account)
            .await
    }

    async fn user_following_artists(
        &self,
        user_id: &str,
        request: &PageRequest,
    ) -> Result<Page<tuneweave_core::Artist>> {
        self.read_following_artists(Some(user_id), request).await
    }

    async fn account_purchased_tracks(
        &self,
        request: &PageRequest,
    ) -> Result<Page<tuneweave_core::PurchasedTrack>> {
        self.read_purchased_tracks(request).await
    }

    async fn account_purchased_albums(
        &self,
        request: &PageRequest,
    ) -> Result<Page<tuneweave_core::PurchasedAlbum>> {
        self.read_purchased_albums(request).await
    }
    async fn user_favorite_albums(&self, uid: &str, request: &PageRequest) -> Result<Page<Album>> {
        self.ordinary_album_collections(Some(uid), request).await
    }
    async fn account_digital_albums(&self, request: &PageRequest) -> Result<Page<DigitalAlbum>> {
        self.digital_album_collections(None, request).await
    }
    async fn user_favorite_digital_albums(
        &self,
        uid: &str,
        request: &PageRequest,
    ) -> Result<Page<DigitalAlbum>> {
        self.digital_album_collections(Some(uid), request).await
    }
    async fn set_album_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<tuneweave_core::SubscriptionResult> {
        self.change_album_collection(
            crate::client::albums::AlbumKind::Ordinary,
            id,
            subscribed,
            account,
        )
        .await
    }
    async fn set_album_subscriptions(
        &self,
        ids: &[String],
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<Vec<tuneweave_core::SubscriptionResult>> {
        self.change_album_collections(
            crate::client::albums::AlbumKind::Ordinary,
            ids,
            subscribed,
            account,
        )
        .await
    }
    async fn set_digital_album_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<tuneweave_core::SubscriptionResult> {
        self.change_album_collection(
            crate::client::albums::AlbumKind::Digital,
            id,
            subscribed,
            account,
        )
        .await
    }
    async fn set_digital_album_subscriptions(
        &self,
        ids: &[String],
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<Vec<tuneweave_core::SubscriptionResult>> {
        self.change_album_collections(
            crate::client::albums::AlbumKind::Digital,
            ids,
            subscribed,
            account,
        )
        .await
    }

    async fn create_playlist(
        &self,
        request: &tuneweave_core::PlaylistCreateRequest,
    ) -> Result<tuneweave_core::PlaylistMutationResult> {
        self.create_owned_playlist(request).await
    }
    async fn update_playlist(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistUpdateRequest,
    ) -> Result<tuneweave_core::PlaylistMutationResult> {
        if request.description.is_some() || request.tags.is_some() {
            return self.update_owned_playlist_metadata(id, request).await;
        }
        self.rename_owned_playlist(id, request).await
    }
    async fn update_playlist_cover(
        &self,
        id: &str,
        request: &tuneweave_core::ImageUploadRequest,
    ) -> Result<tuneweave_core::PlaylistCoverUpdateResult> {
        self.update_owned_playlist_cover(id, request).await
    }
    async fn audio_download_content(
        &self,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<tuneweave_core::AudioContent> {
        self.download_account_content(track, request).await
    }
    async fn upload_account_avatar(
        &self,
        request: &tuneweave_core::ImageUploadRequest,
    ) -> Result<tuneweave_core::ImageUploadResult> {
        self.upload_static_account_avatar(request).await
    }
    async fn reorder_playlist_tracks(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistTrackOrderRequest,
    ) -> Result<tuneweave_core::PlaylistTrackOrderResult> {
        self.reorder_owned_playlist_tracks(id, request).await
    }
    async fn playlist_track_occurrences(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<tuneweave_core::PlaylistTrackOccurrence>> {
        self.read_owned_playlist_occurrences(id, request).await
    }
    async fn reorder_playlist_occurrences(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistOccurrenceOrderRequest,
    ) -> Result<tuneweave_core::PlaylistOccurrenceOrderResult> {
        self.reorder_owned_playlist_occurrences(id, request).await
    }
    async fn delete_playlists(
        &self,
        request: &tuneweave_core::PlaylistDeleteRequest,
    ) -> Result<tuneweave_core::PlaylistDeleteResult> {
        self.delete_owned_playlists(request).await
    }
    async fn mutate_playlist_items(
        &self,
        id: &str,
        action: tuneweave_core::PlaylistItemMutationAction,
        request: &tuneweave_core::PlaylistItemMutationRequest,
    ) -> Result<tuneweave_core::PlaylistItemMutationResult> {
        self.write_owned_playlist_tracks(id, action, request).await
    }

    async fn set_playlist_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<tuneweave_core::SubscriptionResult> {
        self.set_collected_playlist(id, subscribed, account).await
    }

    async fn favorite_playlist(&self, account: Option<&str>) -> Result<Playlist> {
        Ok(self
            .read_account_playlist(None, None, account)
            .await?
            .playlist)
    }
    async fn favorite_tracks(&self, request: &PageRequest) -> Result<Page<Track>> {
        account_playlists::validate_page(request)?;
        Ok(self
            .read_account_playlist(None, None, request.account.as_deref())
            .await?
            .into_page(request))
    }
    async fn user_favorite_playlist(&self, uid: &str, account: Option<&str>) -> Result<Playlist> {
        Ok(self
            .read_account_playlist(None, Some(uid), account)
            .await?
            .playlist)
    }
    async fn user_favorite_tracks(&self, uid: &str, request: &PageRequest) -> Result<Page<Track>> {
        account_playlists::validate_page(request)?;
        Ok(self
            .read_account_playlist(None, Some(uid), request.account.as_deref())
            .await?
            .into_page(request))
    }
    async fn playlist_source(
        &self,
        id: &str,
        source_type: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        match source_type {
            "playlist" => self.playlist(id, account).await,
            "favorite_tracks" => self.user_favorite_playlist(id, account).await,
            "purchased_tracks" => self.purchased_tracks_source(id, account).await,
            "purchased_albums" => self.purchased_albums_source(id, account).await,
            "favorite_albums" => self.favorite_albums_source(id, account).await,
            _ => Err(TuneWeaveError::unsupported(
                Platform::Migu,
                Capability::PlaylistRead,
            )),
        }
    }
    async fn playlist_source_items(
        &self,
        id: &str,
        source_type: &str,
        request: &PageRequest,
    ) -> Result<Page<tuneweave_core::PlaylistPlayableItem>> {
        if source_type == "purchased_tracks" {
            return self.purchased_tracks_source_items(id, request).await;
        }
        if source_type == "purchased_albums" {
            return self.purchased_albums_source_items(id, request).await;
        }
        if source_type == "favorite_albums" {
            return self.favorite_albums_source_items(id, request).await;
        }
        let page = match source_type {
            "playlist" => self.playlist_tracks(id, request).await?,
            "favorite_tracks" => self.user_favorite_tracks(id, request).await?,
            _ => {
                return Err(TuneWeaveError::unsupported(
                    Platform::Migu,
                    Capability::PlaylistRead,
                ));
            }
        };
        Ok(Page {
            items: page
                .items
                .into_iter()
                .map(tuneweave_core::PlaylistPlayableItem::Track)
                .collect(),
            pagination: page.pagination,
        })
    }

    async fn account_playlists(&self, request: &PageRequest) -> Result<Page<Playlist>> {
        self.read_account_library(None, request, None).await
    }

    async fn user_created_playlists(
        &self,
        user_id: &str,
        request: &PageRequest,
    ) -> Result<Page<Playlist>> {
        self.read_account_library(
            Some(user_id),
            request,
            Some(crate::client::library::Section::Created),
        )
        .await
    }

    async fn user_favorite_playlists(
        &self,
        user_id: &str,
        request: &PageRequest,
    ) -> Result<Page<Playlist>> {
        self.read_account_library(
            Some(user_id),
            request,
            Some(crate::client::library::Section::Saved),
        )
        .await
    }

    async fn search(&self, query: &SearchQuery) -> Result<Page<Track>> {
        if query.account.is_some() || self.caller_credential.is_some() {
            return self.search_account_tracks(query).await;
        }
        self.require_public_source()?;
        validate_search_query(query)?;
        self.search_tracks_public(query, || Ok(())).await
    }

    async fn search_catalog(&self, query: &SearchQuery) -> Result<Page<SearchItem>> {
        if query.kind == SearchKind::Mv {
            return self.search_mvs(query).await;
        }
        if query.kind == SearchKind::Track {
            let page = self.search(query).await?;
            return Ok(Page {
                items: page.items.into_iter().map(SearchItem::Track).collect(),
                pagination: page.pagination,
            });
        }
        self.require_public_source()?;
        self.search_public_catalog(query).await
    }

    async fn album(&self, id: &str, account: Option<&str>) -> Result<Album> {
        self.require_public_source()?;
        albums::validate_source(id, account)?;
        self.client.album_metadata(id).await
    }

    async fn video(
        &self,
        id: &str,
        request: &tuneweave_core::VideoDetailRequest,
    ) -> Result<tuneweave_core::VideoDetail> {
        Ok(self.read_mv(id, request).await?.detail)
    }
    async fn video_stats(
        &self,
        id: &str,
        request: &tuneweave_core::VideoDetailRequest,
    ) -> Result<tuneweave_core::VideoStats> {
        Ok(self.read_mv(id, request).await?.stats)
    }
    async fn video_stream(
        &self,
        id: &str,
        request: &tuneweave_core::VideoStreamRequest,
    ) -> Result<tuneweave_core::VideoStream> {
        self.read_mv_stream(id, request).await
    }
    async fn migu_native_mv_stream(
        &self,
        id: &str,
        request: &tuneweave_core::MiguNativeMvStreamRequest,
    ) -> Result<tuneweave_core::VideoStream> {
        self.read_native_mv_stream(id, request).await
    }
    async fn video_streams(
        &self,
        ids: &[String],
        request: &tuneweave_core::VideoStreamRequest,
    ) -> Result<Vec<tuneweave_core::VideoStream>> {
        self.read_mv_streams(ids, request).await
    }
    async fn artist_videos(
        &self,
        id: &str,
        request: &tuneweave_core::ArtistVideoListRequest,
    ) -> Result<Page<tuneweave_core::Video>> {
        self.read_artist_mvs(id, request).await
    }

    async fn search_suggestions(
        &self,
        request: &tuneweave_core::SearchSuggestionRequest,
    ) -> Result<tuneweave_core::SearchSuggestionList> {
        self.read_pc_search_suggestions(request).await
    }

    async fn trending_searches(
        &self,
        request: &tuneweave_core::SearchTrendingRequest,
    ) -> Result<tuneweave_core::SearchTrendingList> {
        self.read_pc_search_trending(request).await
    }

    async fn artist_catalog(
        &self,
        request: &tuneweave_core::ArtistCatalogRequest,
    ) -> Result<tuneweave_core::ArtistCatalog> {
        self.read_artist_directory(request).await
    }

    async fn similar_artists(
        &self,
        id: &str,
        request: &tuneweave_core::SimilarArtistRequest,
    ) -> Result<tuneweave_core::SimilarArtistList> {
        self.read_similar_artists(id, request).await
    }

    async fn artist(&self, id: &str, account: Option<&str>) -> Result<tuneweave_core::Artist> {
        self.read_artist(id, account).await
    }
    async fn artist_overview(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<tuneweave_core::ArtistOverview> {
        self.read_artist_overview(id, account).await
    }
    async fn artist_tracks(
        &self,
        id: &str,
        request: &tuneweave_core::ArtistTrackListRequest,
    ) -> Result<Page<Track>> {
        self.read_artist_tracks(id, request).await
    }
    async fn artist_albums(&self, id: &str, request: &PageRequest) -> Result<Page<Album>> {
        self.read_artist_albums(id, request).await
    }
    async fn artist_digital_albums(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<DigitalAlbum>> {
        self.read_artist_digital_albums(id, request).await
    }

    async fn digital_album(&self, id: &str, account: Option<&str>) -> Result<DigitalAlbum> {
        self.require_public_source()?;
        albums::validate_source(id, account)?;
        self.client.digital_album_metadata(id).await
    }

    async fn album_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        self.require_public_source()?;
        self.read_album_tracks(id, request, false).await
    }

    async fn digital_album_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        self.require_public_source()?;
        self.read_album_tracks(id, request, true).await
    }

    async fn track(&self, id: &str, account: Option<&str>) -> Result<Track> {
        if account.is_some() || self.caller_credential.is_some() {
            return self.read_account_track(id, account).await;
        }
        self.require_public_source()?;
        if account.is_some() {
            return Err(migu_invalid_request(
                "Migu public track detail does not accept an account",
            ));
        }
        let content_id = parse_content_id(id)?;
        self.client.track_detail(content_id).await
    }

    async fn track_availability(
        &self,
        id: &str,
        request: &TrackAvailabilityRequest,
    ) -> Result<TrackAvailability> {
        if request.account.is_some() || self.caller_credential.is_some() {
            return self.account_track_availability(id, request).await;
        }
        self.require_public_source()?;
        validate_availability_request(request)?;
        let content_id = parse_content_id(id)?;
        self.client.track_availability(content_id, request).await
    }

    async fn lyrics(&self, id: &str, account: Option<&str>) -> Result<Lyrics> {
        self.require_public_source()?;
        if account.is_some() {
            return Err(migu_invalid_request(
                "Migu public lyrics do not accept an account",
            ));
        }
        let content_id = parse_content_id(id)?;
        self.client.lyrics(content_id).await
    }

    async fn lyrics_with_options(&self, id: &str, request: &LyricsRequest) -> Result<Lyrics> {
        self.require_public_source()?;
        validate_lyrics_request(request)?;
        let content_id = parse_content_id(id)?;
        self.client.lyrics(content_id).await
    }

    async fn stream(&self, track: &Track, request: &StreamRequest) -> Result<MediaStream> {
        if request.account.is_some() || self.caller_credential.is_some() {
            return self.stream_account_track(track, request).await;
        }
        self.require_public_source()?;
        self.client.stream(track, request).await
    }

    fn requires_download_authorization(&self, account: Option<&str>) -> bool {
        account.is_some() || self.caller_credential.is_some()
    }

    async fn download(&self, track: &Track, request: &StreamRequest) -> Result<MediaDownload> {
        if self.requires_download_authorization(request.account.as_deref()) {
            return self.download_account_track(track, request).await;
        }
        self.require_public_source()?;
        self.client.download(track, request).await
    }

    async fn playlist(&self, id: &str, account: Option<&str>) -> Result<Playlist> {
        if account.is_some() || self.caller_credential.is_some() {
            return Ok(self
                .read_account_playlist(Some(id), None, account)
                .await?
                .playlist);
        }
        self.require_public_source()?;
        if account.is_some() {
            return Err(migu_invalid_request(
                "Migu public playlists do not accept an account",
            ));
        }
        let playlist_id = parse_playlist_id(id)?;
        self.client.playlist_detail(playlist_id).await
    }

    async fn playlist_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        if request.account.is_some() || self.caller_credential.is_some() {
            account_playlists::validate_page(request)?;
            return Ok(self
                .read_account_playlist(Some(id), None, request.account.as_deref())
                .await?
                .into_page(request));
        }
        self.require_public_source()?;
        let playlist_id = parse_playlist_id(id)?;
        validate_playlist_page(request)?;
        let start_page = request.offset / UPSTREAM_PLAYLIST_PAGE_SIZE + 1;
        let first_skip = usize::try_from(request.offset % UPSTREAM_PLAYLIST_PAGE_SIZE)
            .map_err(|_| migu_invalid_request("Migu playlist offset is too large"))?;
        let requested = usize::try_from(request.limit)
            .map_err(|_| migu_invalid_request("Migu playlist limit is too large"))?;
        let required = u32::try_from(first_skip.saturating_add(requested)).unwrap_or(u32::MAX);
        let page_budget = required
            .saturating_add(UPSTREAM_PLAYLIST_PAGE_SIZE - 1)
            .checked_div(UPSTREAM_PLAYLIST_PAGE_SIZE)
            .unwrap_or(MAX_UPSTREAM_PLAYLIST_PAGES)
            .clamp(1, MAX_UPSTREAM_PLAYLIST_PAGES);

        let mut tracks = Vec::with_capacity(requested);
        let mut total = None;
        let mut publish_time = None;
        let mut fetched_pages = 0_u32;
        for page_index in 0..page_budget {
            let page_number = start_page.checked_add(page_index).ok_or_else(|| {
                migu_invalid_request("Migu playlist offset exceeds the upstream page range")
            })?;
            let page = self
                .client
                .playlist_tracks_page(playlist_id, page_number, UPSTREAM_PLAYLIST_PAGE_SIZE)
                .await?;
            fetched_pages = fetched_pages.saturating_add(1);
            if let Some(expected) = total {
                if page.total != expected {
                    return Err(migu_upstream_error(
                        "Migu playlist total changed during pagination",
                    ));
                }
            } else {
                total = Some(page.total);
            }
            if let (Some(expected), Some(actual)) =
                (publish_time.as_deref(), page.publish_time.as_deref())
                && expected != actual
            {
                return Err(migu_upstream_error(
                    "Migu playlist publication time changed during pagination",
                ));
            }
            if publish_time.is_none() {
                publish_time = page.publish_time;
            }

            let skip = if page_index == 0 { first_skip } else { 0 };
            for track in page.tracks.into_iter().skip(skip) {
                if tracks.len() == requested {
                    break;
                }
                tracks.push(track);
            }
            if tracks.len() == requested {
                break;
            }
            let returned = u64::try_from(tracks.len()).unwrap_or(u64::MAX);
            let consumed = u64::from(request.offset).saturating_add(returned);
            if consumed >= total.unwrap_or_default() {
                break;
            }
        }

        let total = total.unwrap_or_default();
        if total > u64::from(u32::MAX) {
            return Err(migu_upstream_error(
                "Migu playlist total exceeded the unified offset range",
            ));
        }
        let returned = u32::try_from(tracks.len()).unwrap_or(u32::MAX);
        let consumed = request.offset.saturating_add(returned);
        if tracks.len() < requested && u64::from(consumed) < total {
            return Err(migu_upstream_error(
                "Migu playlist pagination ended before the requested window",
            ));
        }
        for (index, track) in tracks.iter_mut().enumerate() {
            let position = u64::from(request.offset)
                .checked_add(u64::try_from(index).unwrap_or(u64::MAX))
                .ok_or_else(|| migu_upstream_error("Migu playlist position overflowed"))?;
            track
                .extensions
                .insert("playlist_position".to_owned(), json!(position));
        }
        let has_more = u64::from(consumed) < total;
        let mut extensions = Extensions::new();
        extensions.insert("backend".to_owned(), json!("playlist_song_v2"));
        extensions.insert(
            "upstream_page_size".to_owned(),
            json!(UPSTREAM_PLAYLIST_PAGE_SIZE),
        );
        extensions.insert("upstream_pages_fetched".to_owned(), json!(fetched_pages));
        if let Some(value) = publish_time {
            extensions.insert("publish_time".to_owned(), json!(value));
        }
        Ok(Page {
            items: tracks,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(total),
                next_offset: (has_more && returned > 0).then_some(consumed),
                has_more,
                extensions,
            },
        })
    }
}

fn validate_availability_request(request: &TrackAvailabilityRequest) -> Result<()> {
    if request.account.is_some() {
        return Err(migu_invalid_request(
            "Migu public listening rights do not accept an account",
        ));
    }
    if request.bitrate == 0 || request.bitrate > 10_000_000 {
        return Err(migu_invalid_request(
            "Migu availability bitrate must be between 1 and 10000000",
        ));
    }
    Ok(())
}

fn validate_lyrics_request(request: &LyricsRequest) -> Result<()> {
    if request.account.is_some() {
        return Err(migu_invalid_request(
            "Migu public lyrics do not accept an account",
        ));
    }
    if request.song_type.is_some() || request.singing_annotations {
        return Err(migu_invalid_request(
            "Migu lyrics do not accept song_type or singing annotations",
        ));
    }
    Ok(())
}

fn parse_content_id(id: &str) -> Result<&str> {
    if id.is_empty() || id.len() > 64 || !id.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        return Err(migu_invalid_request(
            "Migu track ID must be a canonical alphanumeric contentId",
        ));
    }
    Ok(id)
}

fn parse_playlist_id(id: &str) -> Result<&str> {
    let parsed = id.parse::<u64>().map_err(|_| {
        migu_invalid_request("Migu playlist ID must be a canonical positive musicListId")
    })?;
    if parsed == 0 || parsed.to_string() != id {
        return Err(migu_invalid_request(
            "Migu playlist ID must be a canonical positive musicListId",
        ));
    }
    Ok(id)
}

fn validate_playlist_page(request: &PageRequest) -> Result<()> {
    if request.account.is_some() {
        return Err(migu_invalid_request(
            "Migu public playlists do not accept an account",
        ));
    }
    if !(1..=100).contains(&request.limit) {
        return Err(migu_invalid_request(
            "Migu playlist limit must be between 1 and 100",
        ));
    }
    Ok(())
}

fn validate_search_query(query: &SearchQuery) -> Result<()> {
    if query.kind != SearchKind::Track {
        return Err(TuneWeaveError::unsupported(
            Platform::Migu,
            capability_for_search(query.kind),
        ));
    }
    validate_public_search_options(query)
}

fn validate_public_search_options(query: &SearchQuery) -> Result<()> {
    if query.variant != SearchVariant::Default {
        return Err(migu_invalid_request(
            "Migu public search only supports the default backend",
        ));
    }
    if query.account.is_some() {
        return Err(migu_invalid_request(
            "Migu public search does not accept an account",
        ));
    }
    if query.search_id.is_some() || query.highlight || !query.selectors.is_empty() {
        return Err(migu_invalid_request(
            "Migu public search does not accept search_id, highlight, or selectors",
        ));
    }
    if query.video_filters.is_some() {
        return Err(migu_invalid_request(
            "Migu public search does not accept video filters",
        ));
    }
    let keyword = query.query.trim();
    if keyword.is_empty() || keyword.len() > 512 || keyword.chars().any(char::is_control) {
        return Err(migu_invalid_request(
            "Migu search query must contain 1 to 512 non-control UTF-8 bytes",
        ));
    }
    if !(1..=100).contains(&query.limit) {
        return Err(migu_invalid_request(
            "Migu search limit must be between 1 and 100",
        ));
    }
    Ok(())
}

fn capability_for_search(kind: SearchKind) -> Capability {
    match kind {
        SearchKind::Track => Capability::SearchTracks,
        SearchKind::Album => Capability::SearchAlbums,
        SearchKind::Artist => Capability::SearchArtists,
        SearchKind::Playlist => Capability::SearchPlaylists,
        SearchKind::User => Capability::SearchUsers,
        SearchKind::Mv => Capability::SearchMvs,
        SearchKind::Lyric => Capability::SearchLyrics,
        SearchKind::RadioStation => Capability::SearchRadioStations,
        SearchKind::Podcast => Capability::SearchPodcasts,
        SearchKind::Video => Capability::SearchVideos,
        SearchKind::Mixed => Capability::SearchMixed,
        SearchKind::Voice => Capability::SearchVoices,
        SearchKind::Ringtone => Capability::SearchRingtones,
    }
}

fn migu_invalid_request(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Migu)
}

fn migu_upstream_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(tuneweave_core::ErrorCode::UpstreamError, message)
        .with_platform(Platform::Migu)
}

impl MiguProvider {
    async fn search_tracks_public(
        &self,
        query: &SearchQuery,
        check: impl Fn() -> Result<()> + Send + Sync,
    ) -> Result<Page<Track>> {
        let start_page = query.offset / UPSTREAM_PAGE_SIZE + 1;
        let first_skip = usize::try_from(query.offset % UPSTREAM_PAGE_SIZE)
            .map_err(|_| migu_invalid_request("Migu search offset is too large"))?;
        let requested = usize::try_from(query.limit)
            .map_err(|_| migu_invalid_request("Migu search limit is too large"))?;
        let required = u32::try_from(first_skip.saturating_add(requested)).unwrap_or(u32::MAX);
        let page_budget = required
            .saturating_add(UPSTREAM_PAGE_SIZE - 1)
            .checked_div(UPSTREAM_PAGE_SIZE)
            .unwrap_or(MAX_UPSTREAM_PAGES)
            .clamp(1, MAX_UPSTREAM_PAGES);

        let mut tracks = Vec::with_capacity(requested);
        let mut sequences = Vec::new();
        let mut conditions: Vec<MiguSearchCondition> = Vec::new();
        let mut fetched_pages = 0_u32;
        let mut has_more = false;
        for page_index in 0..page_budget {
            check()?;
            let page_number = start_page.checked_add(page_index).ok_or_else(|| {
                migu_invalid_request("Migu search offset exceeds the upstream page range")
            })?;
            let page = self
                .client
                .search_tracks_page(query.query.trim(), page_number)
                .await;
            check()?;
            let page = page?;
            fetched_pages = fetched_pages.saturating_add(1);
            if page.has_next && page.tracks.is_empty() {
                return Err(migu_upstream_error(
                    "Migu search reported another page without returning any tracks",
                ));
            }
            if let Some(sequence) = page.sequence {
                sequences.push(sequence);
            }
            if conditions.is_empty() {
                conditions = page.conditions;
            }
            let skip = if page_index == 0 { first_skip } else { 0 };
            let mut unconsumed = false;
            for track in page.tracks.into_iter().skip(skip) {
                if tracks.len() == requested {
                    unconsumed = true;
                    break;
                }
                tracks.push(track);
            }
            has_more = unconsumed || page.has_next;
            if tracks.len() == requested || !page.has_next {
                break;
            }
        }

        let returned = u32::try_from(tracks.len()).unwrap_or(u32::MAX);
        let consumed = query.offset.saturating_add(returned);
        let mut extensions = Extensions::new();
        extensions.insert("backend".to_owned(), json!("bmw_song_search_v1"));
        extensions.insert("upstream_page_size".to_owned(), json!(UPSTREAM_PAGE_SIZE));
        extensions.insert("upstream_pages_fetched".to_owned(), json!(fetched_pages));
        if !sequences.is_empty() {
            extensions.insert("upstream_sequences".to_owned(), json!(sequences));
        }
        if !conditions.is_empty() {
            extensions.insert("conditions".to_owned(), json!(conditions));
        }
        Ok(Page {
            items: tracks,
            pagination: PageMeta {
                limit: query.limit,
                offset: query.offset,
                total: None,
                next_offset: (has_more && returned > 0).then_some(consumed),
                has_more,
                extensions,
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search_query() -> SearchQuery {
        SearchQuery::tracks("反方向的钟", 30, 0)
    }

    #[test]
    fn provider_advertises_only_implemented_capabilities() {
        let provider = MiguProvider::new(MiguConfig::default()).expect("create Migu provider");
        assert_eq!(provider.platform(), Platform::Migu);
        assert_eq!(
            provider.capabilities(),
            BTreeSet::from([
                Capability::PasswordLogin,
                Capability::PhoneLogin,
                Capability::CredentialImport,
                Capability::SessionManagement,
                Capability::CallerManagedCredentials,
                Capability::AccountProfile,
                Capability::AccountAvatarWrite,
                Capability::UserProfileModern,
                Capability::AccountPurchasedTracks,
                Capability::AccountPurchasedAlbums,
                Capability::AccountPlaylists,
                Capability::AccountAlbums,
                Capability::AccountDigitalAlbums,
                Capability::AccountFollowingArtists,
                Capability::ArtistSubscriptionWrite,
                Capability::AlbumSubscriptionWrite,
                Capability::DigitalAlbumSubscriptionWrite,
                Capability::Favorites,
                Capability::TrackSubscriptionWrite,
                Capability::PlaylistSubscriptionWrite,
                Capability::PlaylistWrite,
                Capability::PlaylistOccurrenceRead,
                Capability::PlaylistOccurrenceWrite,
                Capability::UserMembership,
                Capability::UserMembershipClientInfo,
                Capability::AudioDownload,
                Capability::AudioStream,
                Capability::Lyrics,
                Capability::PlaylistRead,
                Capability::SearchTracks,
                Capability::SearchPlaylists,
                Capability::SearchArtists,
                Capability::SearchSuggestions,
                Capability::SearchTrending,
                Capability::ArtistCatalog,
                Capability::ArtistDetail,
                Capability::ArtistOverview,
                Capability::SimilarArtists,
                Capability::ArtistTracks,
                Capability::ArtistAlbums,
                Capability::ArtistDigitalAlbums,
                Capability::ArtistVideos,
                Capability::SearchMvs,
                Capability::VideoDetail,
                Capability::VideoStats,
                Capability::VideoStream,
                Capability::SearchAlbums,
                Capability::AlbumDetail,
                Capability::DigitalAlbumDetail,
                Capability::DigitalAlbumTracks,
                Capability::TrackAvailability,
                Capability::TrackDetail,
                Capability::ChartCatalog,
                Capability::ChartTracks,
                Capability::ChartHistoricalTracks,
            ])
        );
    }

    #[test]
    fn availability_rejects_accounts_and_invalid_bitrate_bounds() {
        assert!(validate_availability_request(&TrackAvailabilityRequest::default()).is_ok());
        assert!(validate_availability_request(&TrackAvailabilityRequest::new(1)).is_ok());
        assert!(validate_availability_request(&TrackAvailabilityRequest::new(10_000_000)).is_ok());
        assert!(validate_availability_request(&TrackAvailabilityRequest::new(0)).is_err());
        assert!(validate_availability_request(&TrackAvailabilityRequest::new(10_000_001)).is_err());
        let account = TrackAvailabilityRequest {
            account: Some("default".to_owned()),
            ..TrackAvailabilityRequest::default()
        };
        assert!(validate_availability_request(&account).is_err());
    }

    #[test]
    fn lyrics_accept_display_preferences_but_reject_foreign_protocol_options() {
        let rich = LyricsRequest {
            word_synced: true,
            translated: true,
            romanized: true,
            ..LyricsRequest::default()
        };
        assert!(validate_lyrics_request(&rich).is_ok());

        let mut account = rich.clone();
        account.account = Some("default".to_owned());
        assert!(validate_lyrics_request(&account).is_err());

        let mut song_type = rich.clone();
        song_type.song_type = Some(1);
        assert!(validate_lyrics_request(&song_type).is_err());

        let mut annotations = rich;
        annotations.singing_annotations = true;
        assert!(validate_lyrics_request(&annotations).is_err());
    }

    #[test]
    fn track_detail_requires_a_canonical_content_identity() {
        assert_eq!(
            parse_content_id("600908000007288315").expect("valid content ID"),
            "600908000007288315"
        );
        for id in [
            "",
            " 600908000007288315",
            "600908000007288315 ",
            "migu:600908000007288315",
            "id/path",
            "id?query",
            "你好",
        ] {
            assert!(parse_content_id(id).is_err(), "{id:?} must fail");
        }
    }

    #[test]
    fn public_playlists_require_canonical_ids_and_bounded_anonymous_pages() {
        assert_eq!(
            parse_playlist_id("231760782").expect("valid playlist ID"),
            "231760782"
        );
        for id in ["", "0", "01", "-1", "1.0", "migu:231760782", " playlist "] {
            assert!(parse_playlist_id(id).is_err(), "{id:?} must fail");
        }
        assert!(validate_playlist_page(&PageRequest::new(1, 0)).is_ok());
        assert!(validate_playlist_page(&PageRequest::new(100, 0)).is_ok());
        assert!(validate_playlist_page(&PageRequest::new(0, 0)).is_err());
        assert!(validate_playlist_page(&PageRequest::new(101, 0)).is_err());
        let account = PageRequest {
            account: Some("default".to_owned()),
            ..PageRequest::new(20, 0)
        };
        assert!(validate_playlist_page(&account).is_err());
    }

    #[test]
    fn public_search_rejects_unimplemented_or_silently_ignored_options() {
        let mut account = search_query();
        account.account = Some("default".to_owned());
        assert!(validate_search_query(&account).is_err());

        let mut variant = search_query();
        variant.variant = SearchVariant::Legacy;
        assert!(validate_search_query(&variant).is_err());

        let mut kind = search_query();
        kind.kind = SearchKind::Album;
        assert!(validate_search_query(&kind).is_err());

        let mut highlight = search_query();
        highlight.highlight = true;
        assert!(validate_search_query(&highlight).is_err());

        let mut excessive = search_query();
        excessive.limit = 101;
        assert!(validate_search_query(&excessive).is_err());
    }

    #[test]
    fn page_budget_covers_non_aligned_windows_without_becoming_unbounded() {
        for (offset, limit, expected) in [
            (0_u32, 1_u32, 1_u32),
            (19, 3, 2),
            (19, 100, 6),
            (20, 100, 5),
        ] {
            let skip = offset % UPSTREAM_PAGE_SIZE;
            let required = skip + limit;
            let budget = required
                .saturating_add(UPSTREAM_PAGE_SIZE - 1)
                .checked_div(UPSTREAM_PAGE_SIZE)
                .unwrap_or(MAX_UPSTREAM_PAGES)
                .clamp(1, MAX_UPSTREAM_PAGES);
            assert_eq!(budget, expected);
        }
    }

    #[tokio::test]
    #[ignore = "requires live Migu network access"]
    async fn live_provider_crosses_a_physical_page_boundary() {
        let provider = MiguProvider::new(MiguConfig::default()).expect("create Migu provider");
        let page = provider
            .search(&SearchQuery::tracks("周杰伦", 3, 19))
            .await
            .expect("live cross-page search");
        assert_eq!(page.items.len(), 3);
        assert_eq!(page.pagination.offset, 19);
        assert!(
            page.items
                .iter()
                .all(|track| track.resource_ref.platform() == Platform::Migu)
        );
    }

    #[tokio::test]
    #[ignore = "requires live Migu network access"]
    async fn live_public_playlist_supports_non_aligned_unified_offsets() {
        let provider = MiguProvider::new(MiguConfig::default()).expect("create Migu provider");
        let playlist = provider
            .playlist("231760782", None)
            .await
            .expect("live Migu playlist detail");
        assert_eq!(playlist.resource_ref.to_string(), "migu:231760782");

        let page = provider
            .playlist_tracks("231760782", &PageRequest::new(3, 49))
            .await
            .expect("live cross-page Migu playlist");
        assert_eq!(page.items.len(), 3);
        assert_eq!(page.pagination.offset, 49);
        assert_eq!(page.pagination.extensions["upstream_pages_fetched"], 2);
        assert_eq!(page.items[0].extensions["playlist_position"], 49);
        assert_eq!(page.items[2].extensions["playlist_position"], 51);
        assert!(
            page.items
                .iter()
                .all(|track| track.resource_ref.platform() == Platform::Migu)
        );
    }

    #[tokio::test]
    #[ignore = "requires live Migu network access"]
    async fn live_provider_returns_strict_public_track_detail() {
        let provider = MiguProvider::new(MiguConfig::default()).expect("create Migu provider");
        let track = provider
            .track("600908000007288315", None)
            .await
            .expect("live Migu track detail");
        assert_eq!(track.resource_ref.to_string(), "migu:600908000007288315");
        assert_eq!(track.extensions["backend"], "resourceinfo_v1");
        assert!(!track.available_qualities.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires live Migu network access"]
    async fn live_provider_prefers_word_synced_mrc_over_plain_lrc() {
        let provider = MiguProvider::new(MiguConfig::default()).expect("create Migu provider");
        let lyrics = provider
            .lyrics("600908000007288315", None)
            .await
            .expect("live Migu lyrics");
        assert_eq!(lyrics.format, "mrc");
        assert!(lyrics.word_synced.is_some());
        assert!(lyrics.plain.is_some());
    }

    #[tokio::test]
    #[ignore = "requires live Migu media access"]
    async fn live_public_media_distinguishes_full_playback_preview_and_download() {
        let provider = MiguProvider::new(MiguConfig::default()).expect("create Migu provider");

        let free = provider
            .track("600913000000358395", None)
            .await
            .expect("free Migu track");
        let free_rights = provider
            .track_availability("600913000000358395", &TrackAvailabilityRequest::default())
            .await
            .expect("free listening rights");
        assert!(free_rights.playable);
        assert_eq!(free_rights.extensions["limit_length"], false);
        let free_stream = provider
            .stream(&free, &StreamRequest::default())
            .await
            .expect("free public stream");
        assert_eq!(free_stream.requested_quality, tuneweave_core::Quality::Auto);
        assert_eq!(
            free_stream.actual_quality,
            tuneweave_core::Quality::Standard
        );
        assert_eq!(free_stream.trial, None);
        let free_download = provider
            .download(&free, &StreamRequest::default())
            .await
            .expect("free public download");
        assert!(free_download.available);
        assert!(free_download.url.is_some());

        let restricted = provider
            .track("600908000007288315", None)
            .await
            .expect("restricted Migu track");
        let restricted_rights = provider
            .track_availability("600908000007288315", &TrackAvailabilityRequest::default())
            .await
            .expect("restricted listening rights");
        assert!(!restricted_rights.playable);
        assert_eq!(restricted_rights.extensions["limit_length"], true);
        let preview = provider
            .stream(&restricted, &StreamRequest::default())
            .await
            .expect("restricted preview");
        assert_eq!(
            preview.trial,
            Some(tuneweave_core::TrialWindow {
                start_ms: 65_000,
                end_ms: 125_000
            })
        );
        assert_eq!(preview.actual_quality, tuneweave_core::Quality::Standard);
        let blocked_download = provider
            .download(&restricted, &StreamRequest::default())
            .await
            .expect("restricted download result");
        assert!(!blocked_download.available);
        assert!(blocked_download.url.is_none());
        assert_eq!(blocked_download.extensions["preview_url_withheld"], true);
    }
}
