use std::{
    collections::BTreeSet,
    fmt,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use serde_json::json;
use tuneweave_core::{
    Artist, Capability, Extensions, Lyrics, LyricsRequest, MediaDownload, MediaStream,
    MusicProvider, Page, PageMeta, PageRequest, Platform, Playlist, PodcastCategoryRecommendations,
    PodcastListRequest, PodcastTaxonomy, PodcastTaxonomyRequest, Result, SearchItem, SearchKind,
    SearchQuery, SearchVariant, StreamRequest, Track, TrackAvailability, TrackAvailabilityRequest,
    TuneWeaveError,
};

use crate::client::{KuwoClient, KuwoConfig};
use crate::client::{KuwoNativeDeviceStore, native::credential::NativeCredential};

mod albums;
mod artists;
mod auth;
mod catalog;
mod charts;
mod discovery;
mod radio;
mod videos;

const UPSTREAM_PAGE_SIZE: u32 = 100;
const UPSTREAM_PLAYLIST_PAGE_SIZE: u32 = 100;

#[derive(Clone)]
pub struct KuwoProvider {
    client: KuwoClient,
    credential_store: Option<Arc<dyn tuneweave_core::AccountCredentialStore>>,
    device_store: Arc<KuwoNativeDeviceStore>,
    auth_registry: Arc<Mutex<auth::Registry>>,
    caller_credential: Option<Arc<Mutex<Option<NativeCredential>>>>,
}

impl fmt::Debug for KuwoProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KuwoProvider")
            .finish_non_exhaustive()
    }
}

impl KuwoProvider {
    pub fn new(config: KuwoConfig) -> Result<Self> {
        Ok(Self {
            client: KuwoClient::new(&config)?,
            credential_store: config.credential_store,
            device_store: Arc::new(KuwoNativeDeviceStore::new(config.device_path)),
            auth_registry: Arc::new(Mutex::new(auth::Registry::default())),
            caller_credential: None,
        })
    }

    #[must_use]
    pub fn from_client(client: KuwoClient) -> Self {
        Self {
            client,
            credential_store: None,
            device_store: Arc::new(KuwoNativeDeviceStore::default()),
            auth_registry: Arc::new(Mutex::new(auth::Registry::default())),
            caller_credential: None,
        }
    }
}

#[async_trait]
impl MusicProvider for KuwoProvider {
    fn platform(&self) -> Platform {
        Platform::Kuwo
    }

    fn name(&self) -> &'static str {
        "Kuwo Music"
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::PasswordLogin,
            Capability::PhoneLogin,
            Capability::CallerManagedCredentials,
            Capability::SessionManagement,
            Capability::SessionRevocation,
            Capability::AccountProfile,
            Capability::AccountPlaylists,
            Capability::AccountFollowingArtists,
            Capability::ArtistSubscriptionWrite,
            Capability::AccountPlaylistSubmissions,
            Capability::PlaylistSubmissionWrite,
            Capability::PlaylistSubmissionRecordDelete,
            Capability::Favorites,
            Capability::TrackSubscriptionWrite,
            Capability::PlaylistSubscriptionWrite,
            Capability::PlaylistCollectionOrderWrite,
            Capability::UserMembership,
            Capability::UserMembershipClientInfo,
            Capability::UserProfileModern,
            Capability::AudioDownload,
            Capability::AudioStream,
            Capability::Lyrics,
            Capability::PlaylistRead,
            Capability::PlaylistCatalog,
            Capability::PlaylistWrite,
            Capability::PlaylistVisibilityWrite,
            Capability::SearchTracks,
            Capability::SearchAlbums,
            Capability::SearchArtists,
            Capability::SearchPlaylists,
            Capability::SearchMvs,
            Capability::SearchSuggestions,
            Capability::SearchTrending,
            Capability::ChartCatalog,
            Capability::ChartTracks,
            Capability::RadioTaxonomy,
            Capability::RadioStationList,
            Capability::RadioStationDetail,
            Capability::PodcastDetail,
            Capability::PodcastList,
            Capability::PodcastCategories,
            Capability::PodcastCategoryRecommendations,
            Capability::PodcastEpisodeList,
            Capability::PodcastEpisodeDetail,
            Capability::PodcastEpisodeStream,
            Capability::ArtistCatalog,
            Capability::ArtistList,
            Capability::ArtistDetail,
            Capability::ArtistOverview,
            Capability::ArtistTracks,
            Capability::ArtistAlbums,
            Capability::ArtistVideos,
            Capability::VideoDetail,
            Capability::VideoCatalog,
            Capability::VideoTaxonomy,
            Capability::VideoStats,
            Capability::VideoStream,
            Capability::AlbumDetail,
            Capability::AlbumList,
            Capability::TrackAvailability,
            Capability::TrackDetail,
        ])
    }

    fn with_caller_credential(
        &self,
        credential: &tuneweave_core::ProviderCredential,
    ) -> Result<Arc<dyn MusicProvider>> {
        Ok(Arc::new(self.caller_scope(credential)?))
    }
    async fn password_login(
        &self,
        request: &tuneweave_core::PasswordLoginRequest,
    ) -> Result<tuneweave_core::AccountProfile> {
        if request.backend == tuneweave_core::PasswordLoginBackend::Web {
            return Err(kuwo_invalid_request(
                "Kuwo Web password login requires the interactive image-challenge flow",
            ));
        }
        Ok(self
            .login_password(request, tuneweave_core::CredentialMode::Server)
            .await?
            .profile)
    }
    async fn password_login_with_mode(
        &self,
        request: &tuneweave_core::PasswordLoginRequest,
        mode: tuneweave_core::CredentialMode,
    ) -> Result<tuneweave_core::ProviderAuthResult> {
        if request.backend == tuneweave_core::PasswordLoginBackend::Web {
            return Err(kuwo_invalid_request(
                "Kuwo Web password login requires the interactive image-challenge flow",
            ));
        }
        self.login_password(request, mode).await
    }
    async fn begin_password_login(
        &self,
        request: &tuneweave_core::PasswordLoginRequest,
        mode: tuneweave_core::CredentialMode,
    ) -> Result<tuneweave_core::PasswordLoginProgress> {
        if request.backend == tuneweave_core::PasswordLoginBackend::Web {
            self.begin_web_password(request, mode).await
        } else {
            self.login_password(request, mode)
                .await
                .map(tuneweave_core::PasswordLoginProgress::Confirmed)
        }
    }
    async fn advance_password_login(
        &self,
        challenge: &tuneweave_core::ProviderPasswordChallenge,
        action: &tuneweave_core::PasswordChallengeAction,
    ) -> Result<tuneweave_core::PasswordLoginProgress> {
        self.advance_web_password(challenge, action).await
    }
    async fn revoke_session_with_ownership(
        &self,
        account: &str,
        source: Option<&tuneweave_core::ProviderCredential>,
        mode: tuneweave_core::CredentialMode,
    ) -> Result<tuneweave_core::ProviderSessionRevocationResult> {
        self.revoke_owned(account, source, mode, std::time::Duration::from_secs(45))
            .await
    }

    async fn session_profile(&self, account: &str) -> Result<tuneweave_core::AccountProfile> {
        self.read_session(account).await
    }
    async fn begin_auth_challenge(
        &self,
        request: &tuneweave_core::AuthChallengeRequest,
        mode: tuneweave_core::CredentialMode,
    ) -> Result<tuneweave_core::ProviderAuthChallenge> {
        self.begin_sms(request, mode).await
    }
    async fn complete_auth_challenge(
        &self,
        challenge: &tuneweave_core::ProviderAuthChallenge,
        code: &str,
    ) -> Result<tuneweave_core::ProviderAuthResult> {
        self.complete_sms(challenge, code).await
    }
    async fn auth_challenge_status(
        &self,
        challenge: &tuneweave_core::ProviderAuthChallenge,
    ) -> Result<tuneweave_core::AuthChallengeStatus> {
        self.sms_status(challenge)
    }
    async fn refresh_session(&self, account: &str) -> Result<tuneweave_core::AccountProfile> {
        Ok(self
            .refresh_owned(account, None, tuneweave_core::CredentialMode::Server)
            .await?
            .profile)
    }
    async fn refresh_session_with_ownership(
        &self,
        account: &str,
        source: Option<&tuneweave_core::ProviderCredential>,
        mode: tuneweave_core::CredentialMode,
    ) -> Result<tuneweave_core::ProviderAuthResult> {
        self.refresh_owned(account, source, mode).await
    }
    async fn logout(&self, account: &str) -> Result<bool> {
        Ok(self
            .logout_owned(account, None, tuneweave_core::CredentialMode::Server)?
            .removed)
    }
    async fn logout_with_ownership(
        &self,
        account: &str,
        source: Option<&tuneweave_core::ProviderCredential>,
        mode: tuneweave_core::CredentialMode,
    ) -> Result<tuneweave_core::ProviderLogoutResult> {
        self.logout_owned(account, source, mode)
    }
    async fn user_profile(
        &self,
        id: &str,
        backend: tuneweave_core::UserProfileBackend,
        account: Option<&str>,
    ) -> Result<tuneweave_core::UserProfile> {
        self.read_user_profile(id, backend, account).await
    }

    async fn user_membership(
        &self,
        id: Option<&str>,
        account: Option<&str>,
    ) -> Result<tuneweave_core::MembershipSummary> {
        self.read_membership(id, account).await
    }

    async fn account_playlists(&self, request: &PageRequest) -> Result<Page<Playlist>> {
        self.read_account_library(None, request, None).await
    }

    async fn account_following_artists(&self, request: &PageRequest) -> Result<Page<Artist>> {
        self.read_following_artists(None, request).await
    }
    async fn set_artist_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<tuneweave_core::SubscriptionResult> {
        self.set_following_artist(id, subscribed, account).await
    }

    async fn user_following_artists(
        &self,
        user_id: &str,
        request: &PageRequest,
    ) -> Result<Page<Artist>> {
        self.read_following_artists(Some(user_id), request).await
    }

    async fn delete_playlist_submission_records(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistSubmissionRecordDeleteRequest,
    ) -> Result<tuneweave_core::PlaylistSubmissionRecordDeleteResult> {
        self.delete_native_submission_records(id, request).await
    }

    async fn submit_playlist(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistSubmissionRequest,
    ) -> Result<tuneweave_core::PlaylistSubmissionResult> {
        self.submit_native_playlist(id, request).await
    }

    async fn account_playlist_submissions(
        &self,
        request: &PageRequest,
    ) -> Result<Page<tuneweave_core::PlaylistSubmission>> {
        self.read_playlist_submissions(request).await
    }

    async fn user_created_playlists(
        &self,
        user_id: &str,
        request: &PageRequest,
    ) -> Result<Page<Playlist>> {
        self.read_account_library(
            Some(user_id),
            request,
            Some(crate::client::native::library::Section::Created),
        )
        .await
    }

    async fn create_playlist(
        &self,
        request: &tuneweave_core::PlaylistCreateRequest,
    ) -> Result<tuneweave_core::PlaylistMutationResult> {
        self.create_native_playlist(request).await
    }

    async fn update_playlist(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistUpdateRequest,
    ) -> Result<tuneweave_core::PlaylistMutationResult> {
        self.update_native_playlist(id, request).await
    }

    async fn update_playlist_visibility(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistVisibilityUpdateRequest,
    ) -> Result<tuneweave_core::PlaylistMutationResult> {
        self.update_native_playlist_visibility(id, request).await
    }

    async fn update_playlist_cover(
        &self,
        id: &str,
        request: &tuneweave_core::ImageUploadRequest,
    ) -> Result<tuneweave_core::PlaylistCoverUpdateResult> {
        self.update_native_playlist_cover(id, request).await
    }

    async fn reorder_collected_playlists(
        &self,
        request: &tuneweave_core::PlaylistOrderRequest,
    ) -> Result<tuneweave_core::PlaylistOrderResult> {
        self.reorder_native_collected_playlists(request).await
    }
    async fn reorder_account_playlists(
        &self,
        request: &tuneweave_core::PlaylistOrderRequest,
    ) -> Result<tuneweave_core::PlaylistOrderResult> {
        self.reorder_native_library(request).await
    }

    async fn reorder_playlist_tracks(
        &self,
        id: &str,
        request: &tuneweave_core::PlaylistTrackOrderRequest,
    ) -> Result<tuneweave_core::PlaylistTrackOrderResult> {
        self.reorder_native_tracks(id, request).await
    }

    async fn mutate_playlist_items(
        &self,
        id: &str,
        action: tuneweave_core::PlaylistItemMutationAction,
        request: &tuneweave_core::PlaylistItemMutationRequest,
    ) -> Result<tuneweave_core::PlaylistItemMutationResult> {
        self.mutate_native_playlist_items(id, action, request).await
    }

    async fn delete_playlists(
        &self,
        request: &tuneweave_core::PlaylistDeleteRequest,
    ) -> Result<tuneweave_core::PlaylistDeleteResult> {
        self.delete_native_playlists(request).await
    }

    async fn favorite_playlist(&self, account: Option<&str>) -> Result<Playlist> {
        self.read_favorite_playlist(None, account).await
    }
    async fn set_playlist_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<tuneweave_core::SubscriptionResult> {
        self.set_native_playlist_subscription(id, subscribed, account)
            .await
    }
    async fn set_track_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<tuneweave_core::SubscriptionResult> {
        self.set_native_track_subscription(id, subscribed, account)
            .await
    }
    async fn favorite_tracks(&self, request: &PageRequest) -> Result<Page<Track>> {
        self.read_favorite_tracks(None, request).await
    }
    async fn user_favorite_playlist(&self, uid: &str, account: Option<&str>) -> Result<Playlist> {
        self.read_favorite_playlist(Some(uid), account).await
    }
    async fn user_favorite_tracks(&self, uid: &str, request: &PageRequest) -> Result<Page<Track>> {
        self.read_favorite_tracks(Some(uid), request).await
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
            _ => Err(kuwo_source_unsupported()),
        }
    }
    async fn playlist_source_items(
        &self,
        id: &str,
        source_type: &str,
        request: &PageRequest,
    ) -> Result<Page<tuneweave_core::PlaylistPlayableItem>> {
        let page = match source_type {
            "playlist" => self.playlist_tracks(id, request).await?,
            "favorite_tracks" => self.user_favorite_tracks(id, request).await?,
            _ => return Err(kuwo_source_unsupported()),
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

    async fn user_favorite_playlists(
        &self,
        user_id: &str,
        request: &PageRequest,
    ) -> Result<Page<Playlist>> {
        self.read_account_library(
            Some(user_id),
            request,
            Some(crate::client::native::library::Section::Saved),
        )
        .await
    }
    async fn user_membership_client_info(
        &self,
        id: Option<&str>,
        account: Option<&str>,
    ) -> Result<tuneweave_core::MembershipSummary> {
        self.read_membership(id, account).await
    }

    async fn search(&self, query: &SearchQuery) -> Result<Page<Track>> {
        self.require_public_scope()?;
        validate_search_query(query)?;
        let upstream_page = query.offset / UPSTREAM_PAGE_SIZE;
        let first_skip = usize::try_from(query.offset % UPSTREAM_PAGE_SIZE)
            .map_err(|_| kuwo_invalid_request("Kuwo search offset is too large"))?;
        let requested = usize::try_from(query.limit)
            .map_err(|_| kuwo_invalid_request("Kuwo search limit is too large"))?;

        let first = self
            .client
            .search_tracks_page(query.query.trim(), upstream_page, UPSTREAM_PAGE_SIZE)
            .await?;
        let total = first.total;
        let mut tracks = first
            .tracks
            .into_iter()
            .skip(first_skip)
            .collect::<Vec<_>>();
        let mut fetched_pages = 1_u32;
        let consumed_after_first =
            u64::from(query.offset).saturating_add(u64::try_from(tracks.len()).unwrap_or(u64::MAX));
        if tracks.len() < requested && consumed_after_first < total {
            let next_page = upstream_page.checked_add(1).ok_or_else(|| {
                kuwo_invalid_request("Kuwo search offset exceeds the upstream page range")
            })?;
            let second = self
                .client
                .search_tracks_page(query.query.trim(), next_page, UPSTREAM_PAGE_SIZE)
                .await?;
            if second.total != total {
                return Err(kuwo_upstream_error(
                    "Kuwo search total changed during pagination",
                ));
            }
            fetched_pages = fetched_pages.saturating_add(1);
            tracks.extend(second.tracks);
        }
        tracks.truncate(requested);

        let returned = u32::try_from(tracks.len()).unwrap_or(u32::MAX);
        let consumed = query.offset.saturating_add(returned);
        let has_more = u64::from(consumed) < total;
        let mut extensions = Extensions::new();
        extensions.insert(
            "backend".to_owned(),
            json!("current_web_search_music_by_keyword"),
        );
        extensions.insert("upstream_page_size".to_owned(), json!(UPSTREAM_PAGE_SIZE));
        extensions.insert("upstream_pages_fetched".to_owned(), json!(fetched_pages));
        Ok(Page {
            items: tracks,
            pagination: PageMeta {
                limit: query.limit,
                offset: query.offset,
                total: Some(total),
                next_offset: (has_more && returned > 0).then_some(consumed),
                has_more,
                extensions,
            },
        })
    }

    async fn search_catalog(&self, query: &SearchQuery) -> Result<Page<SearchItem>> {
        self.require_public_scope()?;
        if query.kind == SearchKind::Track {
            let page = self.search(query).await?;
            return Ok(Page {
                items: page.items.into_iter().map(SearchItem::Track).collect(),
                pagination: page.pagination,
            });
        }
        self.search_public_catalog(query).await
    }

    async fn search_suggestions(
        &self,
        request: &tuneweave_core::SearchSuggestionRequest,
    ) -> Result<tuneweave_core::SearchSuggestionList> {
        self.require_public_scope()?;
        self.client.search_suggestions(request).await
    }

    async fn trending_searches(
        &self,
        request: &tuneweave_core::SearchTrendingRequest,
    ) -> Result<tuneweave_core::SearchTrendingList> {
        self.require_public_scope()?;
        self.client.trending_searches(request).await
    }

    async fn radio_taxonomy(
        &self,
        request: &tuneweave_core::RadioTaxonomyRequest,
    ) -> Result<tuneweave_core::RadioTaxonomy> {
        self.require_public_scope()?;
        self.client.radio_taxonomy(request).await
    }

    async fn radio_stations(
        &self,
        request: &tuneweave_core::RadioStationListRequest,
    ) -> Result<Page<tuneweave_core::RadioStation>> {
        self.require_public_scope()?;
        self.client.radio_stations(request).await
    }

    async fn radio_station(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<tuneweave_core::RadioStation> {
        self.require_public_scope()?;
        self.client.radio_station(id, account).await
    }

    async fn podcast(&self, id: &str, account: Option<&str>) -> Result<tuneweave_core::Podcast> {
        self.require_public_scope()?;
        self.client.podcast(id, account).await
    }

    async fn podcast_categories(
        &self,
        request: &PodcastTaxonomyRequest,
    ) -> Result<PodcastTaxonomy> {
        self.require_public_scope()?;
        self.client.podcast_categories(request).await
    }

    async fn podcast_category_recommendations(
        &self,
        account: Option<&str>,
    ) -> Result<PodcastCategoryRecommendations> {
        self.require_public_scope()?;
        self.client.podcast_category_recommendations(account).await
    }

    async fn podcasts(
        &self,
        request: &PodcastListRequest,
    ) -> Result<Page<tuneweave_core::Podcast>> {
        self.require_public_scope()?;
        self.client.podcasts(request).await
    }

    async fn podcast_episodes(
        &self,
        id: &str,
        request: &tuneweave_core::PodcastEpisodeListRequest,
    ) -> Result<Page<tuneweave_core::PodcastEpisode>> {
        self.require_public_scope()?;
        self.client.podcast_episodes(id, request).await
    }

    async fn podcast_episode(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<tuneweave_core::PodcastEpisode> {
        self.require_public_scope()?;
        self.client.podcast_episode(id, account).await
    }

    async fn podcast_episode_stream(
        &self,
        id: &str,
        request: &StreamRequest,
    ) -> Result<tuneweave_core::PodcastEpisodeStream> {
        self.require_public_scope()?;
        self.client.podcast_episode_stream(id, request).await
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

    async fn artist_catalog(
        &self,
        request: &tuneweave_core::ArtistCatalogRequest,
    ) -> Result<tuneweave_core::ArtistCatalog> {
        self.require_public_scope()?;
        self.read_artist_catalog(request).await
    }

    async fn artist(&self, id: &str, account: Option<&str>) -> Result<tuneweave_core::Artist> {
        self.require_public_scope()?;
        self.read_artist(id, account).await
    }

    async fn artists(&self, request: &tuneweave_core::ArtistListRequest) -> Result<Page<Artist>> {
        self.require_public_scope()?;
        self.client.artists(request).await
    }

    async fn artist_overview(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<tuneweave_core::ArtistOverview> {
        self.require_public_scope()?;
        self.read_artist_overview(id, account).await
    }

    async fn artist_tracks(
        &self,
        id: &str,
        request: &tuneweave_core::ArtistTrackListRequest,
    ) -> Result<Page<Track>> {
        self.require_public_scope()?;
        self.read_artist_tracks(id, request).await
    }

    async fn artist_albums(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<tuneweave_core::Album>> {
        self.require_public_scope()?;
        self.read_artist_albums(id, request).await
    }

    async fn artist_videos(
        &self,
        id: &str,
        request: &tuneweave_core::ArtistVideoListRequest,
    ) -> Result<Page<tuneweave_core::Video>> {
        self.require_public_scope()?;
        self.read_artist_videos(id, request).await
    }

    async fn music_videos(
        &self,
        request: &tuneweave_core::MusicVideoListRequest,
    ) -> Result<Page<tuneweave_core::Video>> {
        self.require_public_scope()?;
        self.client.music_videos(request).await
    }

    async fn video_taxonomy(
        &self,
        request: &tuneweave_core::VideoTaxonomyRequest,
    ) -> Result<Page<tuneweave_core::VideoCatalogOption>> {
        self.require_public_scope()?;
        self.client.video_taxonomy(request).await
    }

    async fn video(
        &self,
        id: &str,
        request: &tuneweave_core::VideoDetailRequest,
    ) -> Result<tuneweave_core::VideoDetail> {
        self.require_public_scope()?;
        self.read_video(id, request).await
    }

    async fn video_stats(
        &self,
        id: &str,
        request: &tuneweave_core::VideoDetailRequest,
    ) -> Result<tuneweave_core::VideoStats> {
        self.require_public_scope()?;
        self.read_video_stats(id, request).await
    }

    async fn video_stream(
        &self,
        id: &str,
        request: &tuneweave_core::VideoStreamRequest,
    ) -> Result<tuneweave_core::VideoStream> {
        self.require_public_scope()?;
        self.read_video_streams(&[id.to_owned()], request)
            .await?
            .pop()
            .ok_or_else(|| kuwo_upstream_error("Kuwo MV playback omitted its result"))
    }

    async fn video_streams(
        &self,
        ids: &[String],
        request: &tuneweave_core::VideoStreamRequest,
    ) -> Result<Vec<tuneweave_core::VideoStream>> {
        self.require_public_scope()?;
        self.read_video_streams(ids, request).await
    }

    async fn albums(
        &self,
        request: &tuneweave_core::AlbumListRequest,
    ) -> Result<Page<tuneweave_core::Album>> {
        self.require_public_scope()?;
        self.client
            .hifi_albums_with_device_store(request, &self.device_store)
            .await
    }

    async fn album(&self, id: &str, account: Option<&str>) -> Result<tuneweave_core::Album> {
        self.require_public_scope()?;
        self.read_album(id, account).await
    }

    async fn album_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        self.require_public_scope()?;
        self.read_album_tracks(id, request).await
    }

    async fn track(&self, id: &str, account: Option<&str>) -> Result<Track> {
        if account.is_some() || self.caller_credential.is_some() {
            return self.read_native_media_track(id, account).await;
        }
        self.require_public_scope()?;
        if account.is_some() {
            return Err(kuwo_invalid_request(
                "Kuwo public track detail does not accept an account",
            ));
        }
        let music_id = parse_music_id(id)?;
        self.client.track_detail(music_id).await
    }

    async fn track_availability(
        &self,
        id: &str,
        request: &TrackAvailabilityRequest,
    ) -> Result<TrackAvailability> {
        if request.account.is_some() || self.caller_credential.is_some() {
            return self.read_native_availability(id, request).await;
        }
        self.require_public_scope()?;
        validate_availability_request(request)?;
        let music_id = parse_music_id(id)?;
        self.client.track_availability(music_id, request).await
    }

    async fn lyrics(&self, id: &str, account: Option<&str>) -> Result<Lyrics> {
        self.require_public_scope()?;
        if account.is_some() {
            return Err(kuwo_invalid_request(
                "Kuwo public lyrics do not accept an account",
            ));
        }
        let music_id = parse_music_id(id)?;
        self.client.lyrics(music_id).await
    }

    async fn lyrics_with_options(&self, id: &str, request: &LyricsRequest) -> Result<Lyrics> {
        self.require_public_scope()?;
        validate_lyrics_request(request)?;
        let music_id = parse_music_id(id)?;
        self.client
            .lyrics_with_options(music_id, request, &self.device_store)
            .await
    }

    async fn stream(&self, track: &Track, request: &StreamRequest) -> Result<MediaStream> {
        if request.account.is_some() || self.caller_credential.is_some() {
            return self.read_native_stream(track, request).await;
        }
        self.require_public_scope()?;
        self.client.stream(track, request).await
    }

    async fn audio_content(
        &self,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<tuneweave_core::AudioContent> {
        self.read_native_content(
            track,
            request,
            crate::client::native::media::Action::Play,
            crate::client::native::media::content::CONTENT_BUDGET,
        )
        .await
    }

    async fn audio_download_content(
        &self,
        track: &Track,
        request: &StreamRequest,
    ) -> Result<tuneweave_core::AudioContent> {
        self.read_native_content(
            track,
            request,
            crate::client::native::media::Action::Download,
            crate::client::native::media::content::CONTENT_BUDGET,
        )
        .await
    }

    fn requires_download_authorization(&self, account: Option<&str>) -> bool {
        account.is_some() || self.caller_credential.is_some()
    }

    async fn download(&self, track: &Track, request: &StreamRequest) -> Result<MediaDownload> {
        if request.account.is_some() || self.caller_credential.is_some() {
            return self.read_native_download(track, request).await;
        }
        self.require_public_scope()?;
        self.client.download(track, request).await
    }

    async fn playlist_catalog_taxonomy(
        &self,
        request: &tuneweave_core::PlaylistCatalogTaxonomyRequest,
    ) -> Result<tuneweave_core::PlaylistCatalogTaxonomy> {
        self.require_public_scope()?;
        self.client.playlist_catalog_taxonomy(request).await
    }

    async fn playlist_catalog(
        &self,
        request: &tuneweave_core::PlaylistCatalogRequest,
    ) -> Result<Page<Playlist>> {
        self.require_public_scope()?;
        self.client.playlist_catalog(request).await
    }

    async fn playlist(&self, id: &str, account: Option<&str>) -> Result<Playlist> {
        if account.is_some() || self.caller_credential.is_some() {
            return Ok(self.read_account_playlist(id, account).await?.playlist);
        }
        self.require_public_scope()?;
        if account.is_some() {
            return Err(kuwo_invalid_request(
                "Kuwo public playlists do not accept an account",
            ));
        }
        let playlist_id = parse_playlist_id(id)?;
        self.client.playlist_detail(playlist_id).await
    }

    async fn playlist_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        if request.account.is_some() || self.caller_credential.is_some() {
            return self.read_account_playlist_tracks(id, request).await;
        }
        self.require_public_scope()?;
        let playlist_id = parse_playlist_id(id)?;
        validate_playlist_page(request)?;
        let start_page = request.offset / UPSTREAM_PLAYLIST_PAGE_SIZE + 1;
        let first_skip = usize::try_from(request.offset % UPSTREAM_PLAYLIST_PAGE_SIZE)
            .map_err(|_| kuwo_invalid_request("Kuwo playlist offset is too large"))?;
        let requested = usize::try_from(request.limit)
            .map_err(|_| kuwo_invalid_request("Kuwo playlist limit is too large"))?;
        let first = self
            .client
            .playlist_page(playlist_id, start_page, UPSTREAM_PLAYLIST_PAGE_SIZE)
            .await?;
        let total = first.total;
        let playlist_ref = first.playlist.resource_ref;
        let mut tracks = first
            .tracks
            .into_iter()
            .skip(first_skip)
            .collect::<Vec<_>>();
        let mut fetched_pages = 1_u32;
        let consumed_after_first = u64::from(request.offset)
            .saturating_add(u64::try_from(tracks.len()).unwrap_or(u64::MAX));
        if tracks.len() < requested && consumed_after_first < total {
            let next_page = start_page.checked_add(1).ok_or_else(|| {
                kuwo_invalid_request("Kuwo playlist offset exceeds the upstream page range")
            })?;
            let second = self
                .client
                .playlist_page(playlist_id, next_page, UPSTREAM_PLAYLIST_PAGE_SIZE)
                .await?;
            if second.total != total || second.playlist.resource_ref != playlist_ref {
                return Err(kuwo_upstream_error(
                    "Kuwo playlist changed during pagination",
                ));
            }
            fetched_pages = fetched_pages.saturating_add(1);
            tracks.extend(second.tracks);
        }
        tracks.truncate(requested);

        let returned = u32::try_from(tracks.len()).unwrap_or(u32::MAX);
        let consumed = request.offset.saturating_add(returned);
        let has_more = u64::from(consumed) < total;
        let mut extensions = Extensions::new();
        extensions.insert("backend".to_owned(), json!("current_web_playlist_info"));
        extensions.insert(
            "upstream_page_size".to_owned(),
            json!(UPSTREAM_PLAYLIST_PAGE_SIZE),
        );
        extensions.insert("upstream_pages_fetched".to_owned(), json!(fetched_pages));
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
        return Err(kuwo_invalid_request(
            "Kuwo public listening rights do not accept an account",
        ));
    }
    if request.bitrate == 0 || request.bitrate > 10_000_000 {
        return Err(kuwo_invalid_request(
            "Kuwo availability bitrate must be between 1 and 10000000",
        ));
    }
    Ok(())
}

fn validate_lyrics_request(request: &LyricsRequest) -> Result<()> {
    if request.account.is_some() {
        return Err(kuwo_invalid_request(
            "Kuwo public lyrics do not accept an account",
        ));
    }
    if request.song_type.is_some() || request.singing_annotations {
        return Err(kuwo_invalid_request(
            "Kuwo lyrics do not accept song_type or singing annotations",
        ));
    }
    Ok(())
}

pub(crate) fn parse_music_id(id: &str) -> Result<&str> {
    let parsed = id
        .parse::<u64>()
        .map_err(|_| kuwo_invalid_request("Kuwo track ID must be a canonical positive music ID"))?;
    if parsed == 0 || parsed.to_string() != id {
        return Err(kuwo_invalid_request(
            "Kuwo track ID must be a canonical positive music ID",
        ));
    }
    Ok(id)
}

fn parse_playlist_id(id: &str) -> Result<&str> {
    let parsed = id
        .parse::<u64>()
        .map_err(|_| kuwo_invalid_request("Kuwo playlist ID must be a canonical positive PID"))?;
    if parsed == 0 || parsed.to_string() != id {
        return Err(kuwo_invalid_request(
            "Kuwo playlist ID must be a canonical positive PID",
        ));
    }
    Ok(id)
}

fn validate_playlist_page(request: &PageRequest) -> Result<()> {
    if request.account.is_some() {
        return Err(kuwo_invalid_request(
            "Kuwo public playlists do not accept an account",
        ));
    }
    if !(1..=100).contains(&request.limit) {
        return Err(kuwo_invalid_request(
            "Kuwo playlist limit must be between 1 and 100",
        ));
    }
    Ok(())
}

fn validate_search_query(query: &SearchQuery) -> Result<()> {
    if query.kind != SearchKind::Track {
        return Err(TuneWeaveError::unsupported(
            Platform::Kuwo,
            capability_for_search(query.kind),
        ));
    }
    validate_public_search_options(query)
}

fn validate_public_search_options(query: &SearchQuery) -> Result<()> {
    if query.variant != SearchVariant::Default {
        return Err(kuwo_invalid_request(
            "Kuwo public search only supports the default backend",
        ));
    }
    if query.account.is_some() {
        return Err(kuwo_invalid_request(
            "Kuwo public search does not accept an account",
        ));
    }
    if query.search_id.is_some() || query.highlight || !query.selectors.is_empty() {
        return Err(kuwo_invalid_request(
            "Kuwo public search does not accept search_id, highlight, or selectors",
        ));
    }
    if query.video_filters.is_some() {
        return Err(kuwo_invalid_request(
            "Kuwo public search does not accept video filters",
        ));
    }
    let keyword = query.query.trim();
    if keyword.is_empty() || keyword.len() > 512 || keyword.chars().any(char::is_control) {
        return Err(kuwo_invalid_request(
            "Kuwo search query must contain 1 to 512 non-control UTF-8 bytes",
        ));
    }
    if !(1..=100).contains(&query.limit) {
        return Err(kuwo_invalid_request(
            "Kuwo search limit must be between 1 and 100",
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

fn kuwo_invalid_request(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Kuwo)
}

fn kuwo_upstream_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(tuneweave_core::ErrorCode::UpstreamError, message)
        .with_platform(Platform::Kuwo)
}

fn kuwo_source_unsupported() -> TuneWeaveError {
    TuneWeaveError::new(
        tuneweave_core::ErrorCode::CapabilityNotSupported,
        "Kuwo playlist source type is not supported",
    )
    .with_platform(Platform::Kuwo)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search_query() -> SearchQuery {
        SearchQuery::tracks("反方向的钟", 30, 0)
    }

    #[test]
    fn provider_advertises_implemented_catalogue_and_account_capabilities() {
        let provider = KuwoProvider::new(KuwoConfig::default()).expect("create Kuwo provider");
        assert_eq!(provider.platform(), Platform::Kuwo);
        assert_eq!(
            provider.capabilities(),
            BTreeSet::from([
                Capability::PasswordLogin,
                Capability::PhoneLogin,
                Capability::CallerManagedCredentials,
                Capability::SessionManagement,
                Capability::SessionRevocation,
                Capability::AccountProfile,
                Capability::AccountPlaylists,
                Capability::AccountFollowingArtists,
                Capability::ArtistSubscriptionWrite,
                Capability::AccountPlaylistSubmissions,
                Capability::PlaylistSubmissionWrite,
                Capability::PlaylistSubmissionRecordDelete,
                Capability::Favorites,
                Capability::TrackSubscriptionWrite,
                Capability::PlaylistSubscriptionWrite,
                Capability::PlaylistCollectionOrderWrite,
                Capability::UserMembership,
                Capability::UserMembershipClientInfo,
                Capability::UserProfileModern,
                Capability::AudioDownload,
                Capability::AudioStream,
                Capability::Lyrics,
                Capability::PlaylistRead,
                Capability::PlaylistCatalog,
                Capability::PlaylistWrite,
                Capability::PlaylistVisibilityWrite,
                Capability::SearchTracks,
                Capability::SearchAlbums,
                Capability::SearchArtists,
                Capability::SearchPlaylists,
                Capability::SearchMvs,
                Capability::SearchSuggestions,
                Capability::SearchTrending,
                Capability::ChartCatalog,
                Capability::ChartTracks,
                Capability::RadioTaxonomy,
                Capability::RadioStationList,
                Capability::RadioStationDetail,
                Capability::PodcastDetail,
                Capability::PodcastList,
                Capability::PodcastCategories,
                Capability::PodcastCategoryRecommendations,
                Capability::PodcastEpisodeList,
                Capability::PodcastEpisodeDetail,
                Capability::PodcastEpisodeStream,
                Capability::ArtistCatalog,
                Capability::ArtistList,
                Capability::ArtistDetail,
                Capability::ArtistOverview,
                Capability::ArtistTracks,
                Capability::ArtistAlbums,
                Capability::ArtistVideos,
                Capability::VideoDetail,
                Capability::VideoCatalog,
                Capability::VideoTaxonomy,
                Capability::VideoStats,
                Capability::VideoStream,
                Capability::AlbumDetail,
                Capability::AlbumList,
                Capability::TrackAvailability,
                Capability::TrackDetail
            ])
        );
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
    fn track_detail_requires_a_canonical_public_identity_without_an_account() {
        assert_eq!(parse_music_id("228908").expect("valid music ID"), "228908");
        for invalid in ["", "0", "01", "-1", "MUSIC_228908", "abc"] {
            assert!(parse_music_id(invalid).is_err());
        }
    }

    #[test]
    fn availability_requires_a_bounded_bitrate_without_an_account() {
        assert!(validate_availability_request(&TrackAvailabilityRequest::default()).is_ok());
        assert!(validate_availability_request(&TrackAvailabilityRequest::new(1)).is_ok());
        assert!(validate_availability_request(&TrackAvailabilityRequest::new(10_000_000)).is_ok());
        assert!(validate_availability_request(&TrackAvailabilityRequest::new(0)).is_err());
        assert!(validate_availability_request(&TrackAvailabilityRequest::new(10_000_001)).is_err());

        let account = TrackAvailabilityRequest {
            bitrate: 128_000,
            account: Some("default".to_owned()),
        };
        assert!(validate_availability_request(&account).is_err());
    }

    #[test]
    fn public_playlist_inputs_require_canonical_ids_and_bounded_pages() {
        assert_eq!(
            parse_playlist_id("1082685104").expect("valid Kuwo playlist ID"),
            "1082685104"
        );
        for invalid in ["", "0", "01", "-1", "playlist_1082685104", "abc"] {
            assert!(parse_playlist_id(invalid).is_err());
        }
        assert!(validate_playlist_page(&PageRequest::new(100, 99)).is_ok());
        assert!(validate_playlist_page(&PageRequest::new(0, 0)).is_err());
        assert!(validate_playlist_page(&PageRequest::new(101, 0)).is_err());
        let account = PageRequest {
            limit: 30,
            offset: 0,
            account: Some("default".to_owned()),
        };
        assert!(validate_playlist_page(&account).is_err());
    }

    #[test]
    fn search_validation_rejects_foreign_options_before_network_access() {
        let valid = search_query();
        assert!(validate_search_query(&valid).is_ok());

        let mut account = valid.clone();
        account.account = Some("default".to_owned());
        assert!(validate_search_query(&account).is_err());

        let mut legacy = valid.clone();
        legacy.variant = SearchVariant::Legacy;
        assert!(validate_search_query(&legacy).is_err());

        let mut album = valid.clone();
        album.kind = SearchKind::Album;
        assert!(validate_search_query(&album).is_err());

        let mut selector = valid.clone();
        selector.highlight = true;
        assert!(validate_search_query(&selector).is_err());

        let mut empty = valid.clone();
        empty.query = " \t".to_owned();
        assert!(validate_search_query(&empty).is_err());

        let mut too_many = valid;
        too_many.limit = 101;
        assert!(validate_search_query(&too_many).is_err());
    }
}
