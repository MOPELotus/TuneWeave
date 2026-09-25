use std::{
    collections::BTreeSet,
    fmt,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use serde_json::json;
use tuneweave_core::{
    AccountCredentialStore, AccountProfile, AuthState, Capability, CredentialMode, ErrorCode,
    Extensions, Lyrics, LyricsRequest, MediaDownload, MediaStream, MusicProvider, Page, PageMeta,
    PageRequest, Platform, PlaybackHistoryEntry, PlaybackHistoryRequest, Playlist,
    ProviderAuthResult, ProviderCredential, ProviderLogoutResult, ProviderQrPoll, ProviderQrStart,
    Result, SearchKind, SearchQuery, SearchVariant, StreamRequest, Track, TuneWeaveError,
};

use crate::client::{KugouClient, KugouConfig};
use crate::credential::AccountCredential as KugouCredential;

mod albums;
mod artist_videos;
mod artists;
mod catalog;
mod charts;
mod following;
mod history;
mod library;
mod lyrics;
mod media;
mod membership;
mod password;
mod purchases;
mod qr;
mod session;
mod sms;
mod video_streams;
mod videos;

#[cfg(test)]
mod search_default_tests;
#[cfg(test)]
mod search_suggestions_tests;
#[cfg(test)]
mod web_tests;

const UPSTREAM_PAGE_SIZE: u32 = 100;

#[derive(Clone)]
pub struct KugouProvider {
    client: KugouClient,
    credential_store: Option<Arc<dyn AccountCredentialStore>>,
    caller_credential: Option<Arc<Mutex<Option<KugouCredential>>>>,
    response_credential: Arc<Mutex<Option<ProviderCredential>>>,
    qr_transactions: Arc<Mutex<qr::Transactions>>,
}

impl fmt::Debug for KugouProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KugouProvider")
            .finish_non_exhaustive()
    }
}

impl KugouProvider {
    pub fn new(config: KugouConfig) -> Result<Self> {
        Ok(Self {
            client: KugouClient::new(&config)?,
            credential_store: config.credential_store,
            caller_credential: None,
            response_credential: Arc::default(),
            qr_transactions: Arc::default(),
        })
    }

    #[must_use]
    pub fn from_client(client: KugouClient) -> Self {
        Self {
            client,
            credential_store: None,
            caller_credential: None,
            response_credential: Arc::default(),
            qr_transactions: Arc::default(),
        }
    }
}

#[async_trait]
impl MusicProvider for KugouProvider {
    fn platform(&self) -> Platform {
        Platform::Kugou
    }

    fn name(&self) -> &'static str {
        "KuGou Music"
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::QrLogin,
            Capability::PasswordLogin,
            Capability::PhoneLogin,
            Capability::SessionManagement,
            Capability::CallerManagedCredentials,
            Capability::AccountProfile,
            Capability::UserMembership,
            Capability::UserMembershipClientInfo,
            Capability::AccountPlaylists,
            Capability::AccountFollowingArtists,
            Capability::ArtistSubscriptionWrite,
            Capability::AccountPurchasedTracks,
            Capability::AccountPurchasedAlbums,
            Capability::UserProfileModern,
            Capability::AudioDownload,
            Capability::AudioStream,
            Capability::TrackAvailability,
            Capability::Lyrics,
            Capability::PlaylistRead,
            Capability::PlaylistOccurrenceRead,
            Capability::PlaylistOccurrenceWrite,
            Capability::PlaylistWrite,
            Capability::PlaylistVisibilityWrite,
            Capability::Favorites,
            Capability::TrackSubscriptionWrite,
            Capability::PlaylistSubscriptionWrite,
            Capability::SearchTracks,
            Capability::SearchAlbums,
            Capability::SearchArtists,
            Capability::SearchPlaylists,
            Capability::SearchMvs,
            Capability::SearchSuggestions,
            Capability::SearchDefault,
            Capability::VideoDetail,
            Capability::VideoStream,
            Capability::AlbumDetail,
            Capability::ArtistDetail,
            Capability::ArtistCatalog,
            Capability::ArtistList,
            Capability::ArtistOverview,
            Capability::ArtistStats,
            Capability::ArtistTracks,
            Capability::ArtistTopTracks,
            Capability::ArtistAlbums,
            Capability::ArtistVideos,
            Capability::ChartCatalog,
            Capability::ChartTracks,
            Capability::ChartHistoricalTracks,
            Capability::ChartPeriods,
            Capability::TrackDetail,
            Capability::ListeningHistory,
        ])
    }

    async fn begin_auth_challenge(
        &self,
        request: &tuneweave_core::AuthChallengeRequest,
        mode: CredentialMode,
    ) -> Result<tuneweave_core::ProviderAuthChallenge> {
        self.begin_sms(request, mode).await
    }

    async fn user_membership(
        &self,
        id: Option<&str>,
        account: Option<&str>,
    ) -> Result<tuneweave_core::MembershipSummary> {
        self.read_membership(id, account, std::time::Duration::from_secs(45))
            .await
    }

    async fn user_membership_client_info(
        &self,
        id: Option<&str>,
        account: Option<&str>,
    ) -> Result<tuneweave_core::MembershipSummary> {
        self.read_membership(id, account, std::time::Duration::from_secs(45))
            .await
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
        self.advance_sms(challenge, action).await
    }
    async fn complete_auth_challenge(
        &self,
        challenge: &tuneweave_core::ProviderAuthChallenge,
        code: &str,
    ) -> Result<ProviderAuthResult> {
        match self
            .advance_sms(
                challenge,
                &tuneweave_core::AuthChallengeAction::SubmitCode { code: code.into() },
            )
            .await?
        {
            tuneweave_core::AuthChallengeProgress::Confirmed(result) => Ok(result),
            tuneweave_core::AuthChallengeProgress::Pending(_) => Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "KuGou SMS login requires an additional challenge action",
            )
            .with_platform(Platform::Kugou)),
        }
    }

    async fn password_login(
        &self,
        request: &tuneweave_core::PasswordLoginRequest,
    ) -> Result<AccountProfile> {
        Ok(self
            .login_password(request, CredentialMode::Server)
            .await?
            .profile)
    }

    async fn begin_password_login(
        &self,
        request: &tuneweave_core::PasswordLoginRequest,
        mode: CredentialMode,
    ) -> Result<tuneweave_core::PasswordLoginProgress> {
        if request.backend == tuneweave_core::PasswordLoginBackend::Native {
            self.begin_native_password(request, mode).await
        } else {
            self.begin_web_password(request, mode).await
        }
    }

    async fn advance_password_login(
        &self,
        challenge: &tuneweave_core::ProviderPasswordChallenge,
        action: &tuneweave_core::PasswordChallengeAction,
    ) -> Result<tuneweave_core::PasswordLoginProgress> {
        if challenge.identity().backend == tuneweave_core::PasswordLoginBackend::Native {
            self.advance_native_password(challenge, action).await
        } else {
            self.advance_web_password(challenge, action).await
        }
    }

    async fn password_login_with_mode(
        &self,
        request: &tuneweave_core::PasswordLoginRequest,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        self.login_password(request, mode).await
    }

    async fn search(&self, query: &SearchQuery) -> Result<Page<Track>> {
        if query.account.is_some() || self.caller_credential.is_some() {
            return self.account_media_candidates(query).await;
        }
        self.require_public_source()?;
        validate_search_query(query)?;
        let upstream_page = query.offset / UPSTREAM_PAGE_SIZE + 1;
        let skip = usize::try_from(query.offset % UPSTREAM_PAGE_SIZE).unwrap_or(usize::MAX);
        let first = self
            .client
            .search_tracks_page(query.query.trim(), upstream_page, UPSTREAM_PAGE_SIZE)
            .await?;
        let total = first.total;
        let mut tracks = first.tracks.into_iter().skip(skip).collect::<Vec<_>>();
        let requested = usize::try_from(query.limit).unwrap_or(usize::MAX);
        let next_page_needed = tracks.len() < requested
            && u64::from(query.offset)
                .saturating_add(u64::try_from(tracks.len()).unwrap_or(u64::MAX))
                < total;
        if next_page_needed {
            let second = self
                .client
                .search_tracks_page(
                    query.query.trim(),
                    upstream_page.saturating_add(1),
                    UPSTREAM_PAGE_SIZE,
                )
                .await?;
            tracks.extend(second.tracks);
        }
        tracks.truncate(requested);
        let returned = u32::try_from(tracks.len()).unwrap_or(u32::MAX);
        let consumed = query.offset.saturating_add(returned);
        let has_more = u64::from(consumed) < total;
        let mut extensions = Extensions::new();
        extensions.insert("backend".to_owned(), json!("song_search_v2"));
        extensions.insert("upstream_page_size".to_owned(), json!(UPSTREAM_PAGE_SIZE));
        Ok(Page {
            items: tracks,
            pagination: PageMeta {
                limit: query.limit,
                offset: query.offset,
                total: Some(total),
                next_offset: has_more.then_some(consumed),
                has_more,
                extensions,
            },
        })
    }

    async fn search_catalog(
        &self,
        query: &SearchQuery,
    ) -> Result<Page<tuneweave_core::SearchItem>> {
        if query.kind == SearchKind::Track {
            let page = self.search(query).await?;
            return Ok(Page {
                items: page
                    .items
                    .into_iter()
                    .map(tuneweave_core::SearchItem::Track)
                    .collect(),
                pagination: page.pagination,
            });
        }
        self.search_public_catalog(query).await
    }

    async fn search_suggestions(
        &self,
        request: &tuneweave_core::SearchSuggestionRequest,
    ) -> Result<tuneweave_core::SearchSuggestionList> {
        self.require_public_source()?;
        self.client.search_suggestions(request).await
    }

    async fn default_search_keyword(
        &self,
        request: &tuneweave_core::SearchDefaultKeywordRequest,
    ) -> Result<tuneweave_core::SearchDefaultKeyword> {
        self.require_public_source()?;
        if request
            .account
            .as_deref()
            .is_some_and(|account| account != "default")
        {
            return Err(kugou_invalid_request(
                "KuGou default search keywords do not accept a named account",
            ));
        }
        // The shared service uses "default" for an omitted account selector.
        // This public Web endpoint never loads or refreshes that stored account.
        self.client
            .default_search_keyword(&tuneweave_core::SearchDefaultKeywordRequest { account: None })
            .await
    }

    async fn track(&self, id: &str, account: Option<&str>) -> Result<Track> {
        if account.is_some() || self.caller_credential.is_some() {
            return self.account_media_track(id, account).await;
        }
        self.require_public_source()?;
        if account.is_some() {
            return Err(kugou_invalid_request(
                "KuGou public track detail does not accept an account",
            ));
        }
        let album_audio_id = parse_album_audio_id(id)?;
        self.client.track_detail(album_audio_id).await
    }

    async fn album(&self, id: &str, account: Option<&str>) -> Result<tuneweave_core::Album> {
        self.read_album(id, account).await
    }

    async fn video(
        &self,
        id: &str,
        request: &tuneweave_core::VideoDetailRequest,
    ) -> Result<tuneweave_core::VideoDetail> {
        self.read_videos(&[id.to_owned()], request)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| {
                TuneWeaveError::new(ErrorCode::UpstreamError, "KuGou video response was empty")
                    .with_platform(Platform::Kugou)
            })
    }

    async fn videos(
        &self,
        ids: &[String],
        request: &tuneweave_core::VideoDetailRequest,
    ) -> Result<Vec<tuneweave_core::VideoDetail>> {
        self.read_videos(ids, request).await
    }

    async fn video_stream(
        &self,
        id: &str,
        request: &tuneweave_core::VideoStreamRequest,
    ) -> Result<tuneweave_core::VideoStream> {
        self.read_video_streams(&[id.to_owned()], request)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "KuGou video stream response was empty",
                )
                .with_platform(Platform::Kugou)
            })
    }

    async fn video_streams(
        &self,
        ids: &[String],
        request: &tuneweave_core::VideoStreamRequest,
    ) -> Result<Vec<tuneweave_core::VideoStream>> {
        self.read_video_streams(ids, request).await
    }

    async fn chart_catalog(
        &self,
        request: &tuneweave_core::ChartCatalogRequest,
    ) -> Result<tuneweave_core::ChartCatalog> {
        self.read_chart_catalogue(request).await
    }
    async fn chart_periods(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<tuneweave_core::ChartPeriodSummary>> {
        self.read_chart_periods(id, request).await
    }

    async fn chart_tracks(
        &self,
        id: &str,
        request: &tuneweave_core::ChartTrackListRequest,
    ) -> Result<Page<Track>> {
        self.read_chart_tracks(id, request).await
    }

    async fn album_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        self.read_album_tracks(id, request).await
    }

    async fn artists(
        &self,
        request: &tuneweave_core::ArtistListRequest,
    ) -> Result<Page<tuneweave_core::Artist>> {
        self.read_artists(request).await
    }

    async fn artist_catalog(
        &self,
        request: &tuneweave_core::ArtistCatalogRequest,
    ) -> Result<tuneweave_core::ArtistCatalog> {
        self.read_artist_catalog(request).await
    }

    async fn artist(&self, id: &str, account: Option<&str>) -> Result<tuneweave_core::Artist> {
        self.read_artist(id, account).await
    }
    async fn artist_videos(
        &self,
        id: &str,
        request: &tuneweave_core::ArtistVideoListRequest,
    ) -> Result<Page<tuneweave_core::Video>> {
        self.read_artist_videos(id, request).await
    }
    async fn artist_stats(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<tuneweave_core::ArtistStats> {
        self.read_artist_stats(id, account).await
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
    async fn artist_top_tracks(&self, id: &str, account: Option<&str>) -> Result<Page<Track>> {
        self.read_artist_top_tracks(id, account).await
    }
    async fn artist_albums(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<tuneweave_core::Album>> {
        self.read_artist_albums(id, request).await
    }

    async fn lyrics(&self, id: &str, account: Option<&str>) -> Result<Lyrics> {
        self.read_lyrics(id, account).await
    }

    async fn lyrics_with_options(&self, id: &str, request: &LyricsRequest) -> Result<Lyrics> {
        validate_lyrics_request(request)?;
        self.read_lyrics(id, request.account.as_deref()).await
    }

    async fn stream(&self, track: &Track, request: &StreamRequest) -> Result<MediaStream> {
        if request.account.is_some() || self.caller_credential.is_some() {
            return self
                .account_media_stream(track, request, crate::account::media::Behavior::Play)
                .await;
        }
        self.require_public_source()?;
        self.client.stream(track, request).await
    }

    async fn track_availability(
        &self,
        id: &str,
        request: &tuneweave_core::TrackAvailabilityRequest,
    ) -> Result<tuneweave_core::TrackAvailability> {
        self.account_media_availability(id, request).await
    }

    fn requires_download_authorization(&self, account: Option<&str>) -> bool {
        account.is_some() || self.caller_credential.is_some()
    }

    async fn download(&self, track: &Track, request: &StreamRequest) -> Result<MediaDownload> {
        if request.account.is_some() || self.caller_credential.is_some() {
            return self.account_media_download(track, request).await;
        }
        self.require_public_source()?;
        self.client.download(track, request).await
    }

    async fn playlist(&self, id: &str, account: Option<&str>) -> Result<Playlist> {
        if id.starts_with(crate::web::library::PREFIX) {
            return self.legacy_web_playlist_metadata(id, account).await;
        }
        if id.starts_with("cloudlist:") {
            return self.native_playlist_metadata(id, account).await;
        }
        self.require_public_source()?;
        if account.is_some() {
            return Err(kugou_invalid_request(
                "KuGou public playlists do not accept an account",
            ));
        }
        self.client.playlist_detail(id).await
    }

    async fn playlist_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        if id.starts_with(crate::web::library::PREFIX) {
            return self.legacy_web_playlist_tracks(id, request).await;
        }
        if id.starts_with("cloudlist:") {
            return self.native_playlist_tracks(id, request).await;
        }
        self.require_public_source()?;
        self.client.playlist_tracks(id, request).await
    }

    async fn account_purchased_tracks(
        &self,
        request: &PageRequest,
    ) -> Result<Page<tuneweave_core::PurchasedTrack>> {
        self.native_purchased_tracks(request).await
    }
    async fn account_purchased_albums(
        &self,
        request: &PageRequest,
    ) -> Result<Page<tuneweave_core::PurchasedAlbum>> {
        self.native_purchased_albums(request).await
    }

    async fn account_history(
        &self,
        request: &PlaybackHistoryRequest,
    ) -> Result<Page<PlaybackHistoryEntry>> {
        self.read_account_history(request).await
    }

    async fn account_playlists(&self, request: &PageRequest) -> Result<Page<Playlist>> {
        self.selected_account_playlists(request).await
    }

    async fn account_following_artists(
        &self,
        request: &PageRequest,
    ) -> Result<Page<tuneweave_core::Artist>> {
        self.read_followed_artists(None, request).await
    }

    async fn user_following_artists(
        &self,
        user_id: &str,
        request: &PageRequest,
    ) -> Result<Page<tuneweave_core::Artist>> {
        self.read_followed_artists(Some(user_id), request).await
    }

    async fn set_artist_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<tuneweave_core::SubscriptionResult> {
        self.native_set_artist_subscription(id, subscribed, account)
            .await
    }

    async fn favorite_playlist(&self, account: Option<&str>) -> Result<Playlist> {
        self.native_favorite_playlist(None, account).await
    }
    async fn favorite_tracks(&self, request: &PageRequest) -> Result<Page<Track>> {
        self.native_favorite_tracks(None, request).await
    }
    async fn user_favorite_playlist(&self, uid: &str, account: Option<&str>) -> Result<Playlist> {
        self.native_favorite_playlist(Some(uid), account).await
    }
    async fn user_favorite_tracks(&self, uid: &str, request: &PageRequest) -> Result<Page<Track>> {
        self.native_favorite_tracks(Some(uid), request).await
    }
    async fn playlist_source(
        &self,
        id: &str,
        source_type: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        if id.starts_with(crate::web::library::PREFIX) {
            if source_type == "playlist" {
                return self.legacy_web_playlist_source(id, account).await;
            }
            return Err(TuneWeaveError::unsupported(
                Platform::Kugou,
                Capability::PlaylistRead,
            ));
        }
        match source_type {
            "playlist" => self.playlist(id, account).await,
            "favorite_tracks" => self.user_favorite_playlist(id, account).await,
            "purchased_tracks" => self.purchased_tracks_source(id, account).await,
            "purchased_albums" => self.purchased_albums_source(id, account).await,
            _ => Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "KuGou playlist source type is not supported",
            )
            .with_platform(Platform::Kugou)),
        }
    }
    async fn playlist_source_items(
        &self,
        id: &str,
        source_type: &str,
        request: &PageRequest,
    ) -> Result<Page<tuneweave_core::PlaylistPlayableItem>> {
        if id.starts_with(crate::web::library::PREFIX) && source_type != "playlist" {
            return Err(TuneWeaveError::unsupported(
                Platform::Kugou,
                Capability::PlaylistRead,
            ));
        }
        if source_type == "purchased_tracks" {
            return self.purchased_tracks_source_items(id, request).await;
        }
        if source_type == "purchased_albums" {
            return self.purchased_albums_source_items(id, request).await;
        }
        let page = match source_type {
            "playlist" => self.playlist_tracks(id, request).await?,
            "favorite_tracks" => self.user_favorite_tracks(id, request).await?,
            _ => {
                return Err(TuneWeaveError::new(
                    ErrorCode::CapabilityNotSupported,
                    "KuGou playlist source type is not supported",
                )
                .with_platform(Platform::Kugou));
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
    async fn set_track_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<tuneweave_core::SubscriptionResult> {
        self.native_set_track_subscription(id, subscribed, account)
            .await
    }
    async fn mutate_playlist_items(
        &self,
        id: &str,
        action: tuneweave_core::PlaylistItemMutationAction,
        request: &tuneweave_core::PlaylistItemMutationRequest,
    ) -> Result<tuneweave_core::PlaylistItemMutationResult> {
        self.native_mutate_playlist_tracks(id, action, request)
            .await
    }

    async fn playlist_track_occurrences(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<tuneweave_core::PlaylistTrackOccurrence>> {
        if id.starts_with(crate::web::library::PREFIX) {
            return self.legacy_web_playlist_occurrences(id, request).await;
        }
        self.native_track_occurrences(id, request).await
    }
    async fn reorder_playlist_occurrences(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistOccurrenceOrderRequest,
    ) -> Result<tuneweave_core::PlaylistOccurrenceOrderResult> {
        if id.starts_with(crate::web::library::PREFIX) {
            return Err(TuneWeaveError::unsupported(
                Platform::Kugou,
                Capability::PlaylistOccurrenceWrite,
            ));
        }
        self.native_reorder_occurrences(id, request).await
    }
    async fn reorder_playlist_tracks(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistTrackOrderRequest,
    ) -> Result<tuneweave_core::PlaylistTrackOrderResult> {
        self.native_reorder_tracks(id, request).await
    }
    async fn reorder_account_playlists(
        &self,
        request: &tuneweave_core::PlaylistOrderRequest,
    ) -> Result<tuneweave_core::PlaylistOrderResult> {
        self.native_reorder_library(request).await
    }

    async fn create_playlist(
        &self,
        request: &tuneweave_core::PlaylistCreateRequest,
    ) -> Result<tuneweave_core::PlaylistMutationResult> {
        self.native_create_playlist(request).await
    }
    async fn update_playlist(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistUpdateRequest,
    ) -> Result<tuneweave_core::PlaylistMutationResult> {
        self.native_update_playlist(id, request).await
    }
    async fn update_playlist_visibility(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistVisibilityUpdateRequest,
    ) -> Result<tuneweave_core::PlaylistMutationResult> {
        self.native_update_playlist_visibility(id, request).await
    }
    async fn delete_playlists(
        &self,
        request: &tuneweave_core::PlaylistDeleteRequest,
    ) -> Result<tuneweave_core::PlaylistDeleteResult> {
        self.native_delete_playlists(request).await
    }
    async fn update_playlist_cover(
        &self,
        id: &str,
        request: &tuneweave_core::ImageUploadRequest,
    ) -> Result<tuneweave_core::PlaylistCoverUpdateResult> {
        self.native_update_playlist_cover(id, request).await
    }
    async fn set_playlist_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<tuneweave_core::SubscriptionResult> {
        self.native_set_playlist_subscription(id, subscribed, account)
            .await
    }

    async fn user_created_playlists(
        &self,
        uid: &str,
        request: &PageRequest,
    ) -> Result<Page<Playlist>> {
        self.native_account_playlists(Some(uid), Some(0), request)
            .await
    }

    async fn user_favorite_playlists(
        &self,
        uid: &str,
        request: &PageRequest,
    ) -> Result<Page<Playlist>> {
        self.native_account_playlists(Some(uid), Some(1), request)
            .await
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
            .map_err(|_| session::state_error())?
            .take())
    }
    async fn start_qr_login(&self, login_type: Option<&str>) -> Result<ProviderQrStart> {
        self.begin_qr(login_type, CredentialMode::Server).await
    }
    async fn start_qr_login_with_mode(
        &self,
        login_type: Option<&str>,
        mode: CredentialMode,
    ) -> Result<ProviderQrStart> {
        self.begin_qr(login_type, mode).await
    }
    async fn poll_qr_login(&self, id: &str, account: &str) -> Result<ProviderQrPoll> {
        self.poll_qr(id, account, CredentialMode::Server).await
    }
    async fn poll_qr_login_with_mode(
        &self,
        id: &str,
        account: &str,
        mode: CredentialMode,
    ) -> Result<ProviderQrPoll> {
        self.poll_qr(id, account, mode).await
    }
    async fn session_profile(&self, account: &str) -> Result<AccountProfile> {
        self.read_session(account).await
    }
    async fn user_profile(
        &self,
        id: &str,
        backend: tuneweave_core::UserProfileBackend,
        account: Option<&str>,
    ) -> Result<tuneweave_core::UserProfile> {
        self.read_user_profile(id, backend, account).await
    }
    async fn refresh_session(&self, account: &str) -> Result<AccountProfile> {
        Ok(self
            .refresh_owned(account, None, CredentialMode::Server)
            .await?
            .profile)
    }
    async fn refresh_session_with_ownership(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        self.refresh_owned(account, source, mode).await
    }
    async fn logout(&self, account: &str) -> Result<bool> {
        Ok(self
            .logout_owned(account, None, CredentialMode::Server)?
            .removed)
    }
    async fn logout_with_ownership(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<ProviderLogoutResult> {
        self.logout_owned(account, source, mode)
    }
}

fn validate_lyrics_request(request: &LyricsRequest) -> Result<()> {
    if request.song_type.is_some() || request.singing_annotations {
        return Err(kugou_invalid_request(
            "KuGou lyrics do not accept song_type or singing annotations",
        ));
    }
    Ok(())
}

fn parse_album_audio_id(id: &str) -> Result<u64> {
    let parsed = id.parse::<u64>().map_err(|_| {
        kugou_invalid_request("KuGou track ID must be a canonical positive album_audio_id")
    })?;
    if parsed == 0 || parsed.to_string() != id {
        return Err(kugou_invalid_request(
            "KuGou track ID must be a canonical positive album_audio_id",
        ));
    }
    Ok(parsed)
}

fn validate_search_query(query: &SearchQuery) -> Result<()> {
    if query.kind != SearchKind::Track {
        return Err(TuneWeaveError::unsupported(
            Platform::Kugou,
            capability_for_search(query.kind),
        ));
    }
    validate_search_options(query)
}

fn validate_search_options(query: &SearchQuery) -> Result<()> {
    if query.variant != SearchVariant::Default {
        return Err(kugou_invalid_request(
            "KuGou public search only supports the default backend",
        ));
    }
    if query.account.is_some() {
        return Err(kugou_invalid_request(
            "KuGou public search does not accept an account",
        ));
    }
    if query.search_id.is_some() || query.highlight || !query.selectors.is_empty() {
        return Err(kugou_invalid_request(
            "KuGou public search does not accept search_id, highlight, or selectors",
        ));
    }
    if query.video_filters.is_some() {
        return Err(kugou_invalid_request(
            "KuGou public search does not accept video filters",
        ));
    }
    let keyword = query.query.trim();
    if keyword.is_empty() || keyword.len() > 512 || keyword.chars().any(char::is_control) {
        return Err(kugou_invalid_request(
            "KuGou search query must contain 1 to 512 non-control UTF-8 bytes",
        ));
    }
    if !(1..=100).contains(&query.limit) {
        return Err(kugou_invalid_request(
            "KuGou search limit must be between 1 and 100",
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

fn kugou_invalid_request(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Kugou)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search_query() -> SearchQuery {
        SearchQuery::tracks("反方向的钟", 30, 0)
    }

    #[test]
    fn provider_advertises_only_implemented_public_and_native_account_capabilities() {
        let provider = KugouProvider::new(KugouConfig::default()).expect("create KuGou provider");
        assert_eq!(provider.platform(), Platform::Kugou);
        assert_eq!(
            provider.capabilities(),
            BTreeSet::from([
                Capability::QrLogin,
                Capability::PasswordLogin,
                Capability::PhoneLogin,
                Capability::SessionManagement,
                Capability::CallerManagedCredentials,
                Capability::AccountProfile,
                Capability::UserMembership,
                Capability::UserMembershipClientInfo,
                Capability::AccountPlaylists,
                Capability::AccountFollowingArtists,
                Capability::ArtistSubscriptionWrite,
                Capability::AccountPurchasedTracks,
                Capability::AccountPurchasedAlbums,
                Capability::UserProfileModern,
                Capability::AudioDownload,
                Capability::AudioStream,
                Capability::TrackAvailability,
                Capability::Lyrics,
                Capability::PlaylistRead,
                Capability::PlaylistOccurrenceRead,
                Capability::PlaylistOccurrenceWrite,
                Capability::PlaylistWrite,
                Capability::PlaylistVisibilityWrite,
                Capability::Favorites,
                Capability::TrackSubscriptionWrite,
                Capability::PlaylistSubscriptionWrite,
                Capability::SearchTracks,
                Capability::SearchAlbums,
                Capability::SearchArtists,
                Capability::SearchPlaylists,
                Capability::SearchMvs,
                Capability::SearchSuggestions,
                Capability::SearchDefault,
                Capability::VideoDetail,
                Capability::VideoStream,
                Capability::AlbumDetail,
                Capability::ArtistDetail,
                Capability::ArtistCatalog,
                Capability::ArtistList,
                Capability::ArtistOverview,
                Capability::ArtistStats,
                Capability::ArtistTracks,
                Capability::ArtistTopTracks,
                Capability::ArtistAlbums,
                Capability::ArtistVideos,
                Capability::ChartCatalog,
                Capability::ChartTracks,
                Capability::ChartHistoricalTracks,
                Capability::ChartPeriods,
                Capability::TrackDetail,
                Capability::ListeningHistory,
            ])
        );
    }

    #[test]
    fn track_detail_requires_a_canonical_album_audio_identity() {
        assert_eq!(
            parse_album_audio_id("32100650").expect("valid ID"),
            32100650
        );
        for id in ["", "0", "032100650", " 32100650", "+32100650", "-1", "hash"] {
            assert!(parse_album_audio_id(id).is_err(), "{id:?} must fail");
        }
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
        assert!(validate_lyrics_request(&account).is_ok());

        let mut song_type = rich.clone();
        song_type.song_type = Some(1);
        assert!(validate_lyrics_request(&song_type).is_err());

        let mut annotations = rich;
        annotations.singing_annotations = true;
        assert!(validate_lyrics_request(&annotations).is_err());
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
    }
}
