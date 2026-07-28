use std::{collections::BTreeSet, fmt, sync::Arc};

use async_trait::async_trait;
use serde_json::json;
use tuneweave_core::{
    AccountCredentialStore, AccountProfile, ArtistSummary, AuthState, Capability, CreatorSummary,
    CredentialMode, ErrorCode, Extensions, MusicProvider, Page, PageMeta, PageRequest, Platform,
    Playlist, PlaylistPlayableItem, ProviderAuthResult, ProviderCredential, ProviderLogoutResult,
    ProviderQrPoll, ProviderQrStart, ResourceRef, Result, SearchItem, SearchKind, SearchQuery,
    SearchVariant, StoredAccountCredential, Track, TuneWeaveError, Video, VideoDetail,
    VideoDetailRequest, VideoPart, VideoPartListRequest, VideoResourceKind, VideoSearchDuration,
    VideoSearchFilters, VideoSearchOrder, VideoSubtitle, VideoSubtitleList, VideoSubtitleRequest,
};

use crate::BilibiliVideoIdentity;
use crate::client::{
    BilibiliClient, BilibiliCollectedPlaylist, BilibiliCollectedPlaylistKind,
    BilibiliCollectedPlaylistPage, BilibiliConfig, BilibiliCreatedFavoriteFolder,
    BilibiliCreatedFavoriteFolders, BilibiliCredential, BilibiliCredentialRefresh,
    BilibiliFavoriteFolder, BilibiliFavoriteMedia, BilibiliLogoutOutcome, BilibiliQrPoll,
    BilibiliSearchVideo, BilibiliSeasonArchive, BilibiliSessionStatus, BilibiliSpacePlaylist,
    BilibiliSpacePlaylistKind, BilibiliSpacePlaylistPage, BilibiliSubtitleCatalog,
    BilibiliVideoPart, BilibiliVideoSearchDuration, BilibiliVideoSearchFilters,
    BilibiliVideoSearchOrder, BilibiliVideoView,
};

const BILIBILI_CREDENTIAL_KIND: &str = "bilibili_cookie_v1";

#[derive(Clone)]
pub struct BilibiliProvider {
    client: BilibiliClient,
    credential_store: Option<Arc<dyn AccountCredentialStore>>,
    caller_credential: Option<BilibiliCredential>,
}

impl fmt::Debug for BilibiliProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BilibiliProvider")
            .field(
                "credential_store_configured",
                &self.credential_store.is_some(),
            )
            .field(
                "caller_credential_configured",
                &self.caller_credential.is_some(),
            )
            .finish_non_exhaustive()
    }
}

impl BilibiliProvider {
    pub fn new(config: BilibiliConfig) -> Result<Self> {
        let credential_store = config.credential_store.clone();
        Ok(Self {
            client: BilibiliClient::new(&config)?,
            credential_store,
            caller_credential: None,
        })
    }

    #[must_use]
    pub fn from_client(client: BilibiliClient) -> Self {
        Self {
            client,
            credential_store: None,
            caller_credential: None,
        }
    }
}

#[async_trait]
impl MusicProvider for BilibiliProvider {
    fn platform(&self) -> Platform {
        Platform::Bilibili
    }

    fn name(&self) -> &'static str {
        "Bilibili"
    }

    fn with_caller_credential(
        &self,
        credential: &ProviderCredential,
    ) -> Result<Arc<dyn MusicProvider>> {
        Ok(Arc::new(self.caller_credential_scope(credential)?))
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::QrLogin,
            Capability::CallerManagedCredentials,
            Capability::AccountProfile,
            Capability::SessionManagement,
            Capability::SearchVideos,
            Capability::PlaylistRead,
            Capability::VideoDetail,
            Capability::VideoParts,
            Capability::VideoSubtitles,
        ])
    }

    async fn search_catalog(&self, query: &SearchQuery) -> Result<Page<SearchItem>> {
        if query.kind != SearchKind::Video {
            return Err(TuneWeaveError::unsupported(
                Platform::Bilibili,
                capability_for_search(query.kind),
            ));
        }
        if query.variant != SearchVariant::Default {
            return Err(bilibili_invalid_request(
                "Bilibili video search only supports the default backend",
            ));
        }
        if query.search_id.is_some() || !query.selectors.is_empty() {
            return Err(bilibili_invalid_request(
                "Bilibili video search does not accept search_id or selectors",
            ));
        }
        let keyword = query.query.trim();
        if keyword.is_empty() || keyword.len() > 512 || keyword.chars().any(char::is_control) {
            return Err(bilibili_invalid_request(
                "Bilibili video search keyword is invalid",
            ));
        }
        if !(1..=100).contains(&query.limit) {
            return Err(bilibili_invalid_request(
                "Bilibili video search limit must be between 1 and 100",
            ));
        }
        if query.offset >= 1_000 {
            return Err(bilibili_invalid_request(
                "Bilibili video search offset must be below 1000",
            ));
        }
        let filters = map_bilibili_video_search_filters(query.video_filters.as_ref())?;
        let credential = self.optional_request_credential(query.account.as_deref())?;
        const UPSTREAM_PAGE_SIZE: u32 = 20;
        let first_page = query.offset / UPSTREAM_PAGE_SIZE + 1;
        let first_skip = (query.offset % UPSTREAM_PAGE_SIZE) as usize;
        let mut current_page = first_page;
        let mut total = None;
        let mut page_count = None;
        let mut search_id = None;
        let mut items = Vec::with_capacity(query.limit as usize);

        while items.len() < query.limit as usize && current_page <= 50 {
            let page = self
                .client
                .search_videos_page(keyword, current_page, filters, credential.as_ref())
                .await?;
            if page.page_size != UPSTREAM_PAGE_SIZE {
                return Err(bilibili_data_error(
                    "Bilibili video search changed its page size",
                ));
            }
            if total.is_some_and(|total| total != page.total)
                || page_count.is_some_and(|count| count != page.page_count)
            {
                return Err(bilibili_data_error(
                    "Bilibili video search pagination changed during traversal",
                ));
            }
            total = Some(page.total);
            page_count = Some(page.page_count);
            search_id.get_or_insert(page.search_id);
            let skip = if current_page == first_page {
                first_skip
            } else {
                0
            };
            let page_item_count = page.videos.len();
            for video in page.videos.into_iter().skip(skip) {
                if items.len() == query.limit as usize {
                    break;
                }
                items.push(SearchItem::Video(map_bilibili_search_video(video)?));
            }
            let known_page_count = page_count.unwrap_or_default();
            if current_page >= known_page_count || page_item_count < UPSTREAM_PAGE_SIZE as usize {
                break;
            }
            current_page += 1;
        }

        let total = total.unwrap_or_default();
        let consumed = u64::from(query.offset).saturating_add(items.len() as u64);
        let has_more = consumed < total;
        let returned_count = u32::try_from(items.len()).unwrap_or(query.limit);
        let mut extensions = Extensions::new();
        if let Some(search_id) = search_id {
            extensions.insert("search_id".to_owned(), json!(search_id));
        }
        extensions.insert("upstream_page_size".to_owned(), json!(UPSTREAM_PAGE_SIZE));
        extensions.insert(
            "video_filters".to_owned(),
            json!(query.video_filters.clone().unwrap_or_default()),
        );
        if let Some(page_count) = page_count {
            extensions.insert("upstream_page_count".to_owned(), json!(page_count));
        }
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: query.limit,
                offset: query.offset,
                total: Some(total),
                next_offset: has_more.then(|| query.offset.saturating_add(returned_count)),
                has_more,
                extensions,
            },
        })
    }

    async fn video(&self, id: &str, request: &VideoDetailRequest) -> Result<VideoDetail> {
        if request.kind != VideoResourceKind::Video {
            return Err(bilibili_invalid_request(
                "Bilibili archive details require kind=video",
            ));
        }
        let identity = BilibiliVideoIdentity::parse(id)?;
        if matches!(
            identity,
            BilibiliVideoIdentity::Episode(_) | BilibiliVideoIdentity::Season(_)
        ) {
            return Err(bilibili_invalid_request(
                "Bilibili archive details require an AID or BVID",
            ));
        }
        let credential = self.optional_request_credential(request.account.as_deref())?;
        self.client
            .video_view(&identity, credential.as_ref())
            .await
            .and_then(map_bilibili_video_view)
    }

    async fn video_parts(
        &self,
        id: &str,
        request: &VideoPartListRequest,
    ) -> Result<Page<VideoPart>> {
        if request.kind != VideoResourceKind::Video {
            return Err(bilibili_invalid_request(
                "Bilibili archive parts require kind=video",
            ));
        }
        if !(1..=100).contains(&request.limit) {
            return Err(bilibili_invalid_request(
                "Bilibili video part limit must be between 1 and 100",
            ));
        }
        let identity = BilibiliVideoIdentity::parse(id)?;
        if matches!(
            identity,
            BilibiliVideoIdentity::Episode(_) | BilibiliVideoIdentity::Season(_)
        ) {
            return Err(bilibili_invalid_request(
                "Bilibili archive parts require an AID or BVID",
            ));
        }
        let credential = self.optional_request_credential(request.account.as_deref())?;
        let view = self
            .client
            .video_view(&identity, credential.as_ref())
            .await?;
        map_bilibili_video_parts(view.aid, view.bvid, view.parts, request)
    }

    async fn video_subtitles(
        &self,
        id: &str,
        request: &VideoSubtitleRequest,
    ) -> Result<VideoSubtitleList> {
        if request.kind != VideoResourceKind::Video {
            return Err(bilibili_invalid_request(
                "Bilibili archive subtitles require kind=video",
            ));
        }
        let identity = BilibiliVideoIdentity::parse(id)?;
        if matches!(
            identity,
            BilibiliVideoIdentity::Episode(_) | BilibiliVideoIdentity::Season(_)
        ) {
            return Err(bilibili_invalid_request(
                "Bilibili archive subtitles require an AID or BVID",
            ));
        }
        let cid = parse_bilibili_part_id(&request.part_id)?;
        let credential = self.optional_request_credential(request.account.as_deref())?;
        let view = self
            .client
            .video_view(&identity, credential.as_ref())
            .await?;
        if !view.parts.iter().any(|part| part.cid == cid) {
            return Err(bilibili_invalid_request(
                "Bilibili subtitle part does not belong to the requested video",
            ));
        }
        let catalog = self
            .client
            .video_subtitles(view.aid, &view.bvid, cid, credential.as_ref())
            .await?;
        map_bilibili_subtitle_catalog(catalog)
    }

    async fn playlist(&self, id: &str, account: Option<&str>) -> Result<Playlist> {
        let locator = parse_bilibili_playlist_locator(id)?;
        let credential = self.optional_request_credential(account)?;
        match locator {
            BilibiliPlaylistLocator::Season(season_id) => {
                let page = self
                    .client
                    .season_archives_page(season_id, 1, credential.as_ref())
                    .await?;
                let first_page_count = page.archives.len();
                let mut playlist = map_space_playlist(page.season)?;
                playlist
                    .extensions
                    .insert("detail_source".to_owned(), json!("season_archives"));
                playlist
                    .extensions
                    .insert("archive_page_size".to_owned(), json!(page.page_size));
                playlist.extensions.insert(
                    "first_page_archive_count".to_owned(),
                    json!(first_page_count),
                );
                Ok(playlist)
            }
            BilibiliPlaylistLocator::FavoriteFolder(media_id) => self
                .client
                .favorite_folder(media_id, credential.as_ref())
                .await
                .and_then(map_favorite_folder),
            BilibiliPlaylistLocator::Series(_) => Err(unsupported_bilibili_playlist_kind("series")),
        }
    }

    async fn playlist_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        let locator = parse_bilibili_playlist_locator(id)?;
        let credential = self.optional_request_credential(request.account.as_deref())?;
        match locator {
            BilibiliPlaylistLocator::Season(season_id) => {
                let (archives, pagination, owner_id) = self
                    .season_archive_window(season_id, request, credential.as_ref())
                    .await?;
                let items = archives
                    .into_iter()
                    .map(|archive| map_season_archive_track(archive, season_id, owner_id))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Page { items, pagination })
            }
            BilibiliPlaylistLocator::FavoriteFolder(media_id) => {
                let (medias, pagination) = self
                    .favorite_media_window(media_id, request, credential.as_ref())
                    .await?;
                let items = medias
                    .into_iter()
                    .map(|media| map_favorite_media_track(media, media_id))
                    .collect::<Result<Vec<_>>>()?;
                Ok(Page { items, pagination })
            }
            BilibiliPlaylistLocator::Series(_) => Err(unsupported_bilibili_playlist_kind("series")),
        }
    }

    async fn playlist_playable_items(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<PlaylistPlayableItem>> {
        let locator = parse_bilibili_playlist_locator(id)?;
        let credential = self.optional_request_credential(request.account.as_deref())?;
        match locator {
            BilibiliPlaylistLocator::Season(season_id) => {
                let (archives, pagination, owner_id) = self
                    .season_archive_window(season_id, request, credential.as_ref())
                    .await?;
                let items = archives
                    .into_iter()
                    .map(|archive| {
                        map_season_archive_video(archive, season_id, owner_id)
                            .map(PlaylistPlayableItem::Video)
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(Page { items, pagination })
            }
            BilibiliPlaylistLocator::FavoriteFolder(media_id) => {
                let (medias, pagination) = self
                    .favorite_media_window(media_id, request, credential.as_ref())
                    .await?;
                let items = medias
                    .into_iter()
                    .map(|media| {
                        map_favorite_media_video(media, media_id).map(PlaylistPlayableItem::Video)
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(Page { items, pagination })
            }
            BilibiliPlaylistLocator::Series(_) => Err(unsupported_bilibili_playlist_kind("series")),
        }
    }

    async fn playlist_source(
        &self,
        id: &str,
        source_type: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        match source_type {
            "playlist" => self.playlist(id, account).await,
            "season" => {
                let typed_id = bilibili_source_playlist_id(id, "season")?;
                self.playlist(&typed_id, account).await
            }
            "favorite_folder" => {
                let typed_id = bilibili_source_playlist_id(id, "favorite")?;
                self.playlist(&typed_id, account).await
            }
            _ => Err(unsupported_bilibili_playlist_source_type(source_type)),
        }
    }

    async fn playlist_source_items(
        &self,
        id: &str,
        source_type: &str,
        request: &PageRequest,
    ) -> Result<Page<PlaylistPlayableItem>> {
        match source_type {
            "playlist" => self.playlist_playable_items(id, request).await,
            "season" => {
                let typed_id = bilibili_source_playlist_id(id, "season")?;
                self.playlist_playable_items(&typed_id, request).await
            }
            "favorite_folder" => {
                let typed_id = bilibili_source_playlist_id(id, "favorite")?;
                self.playlist_playable_items(&typed_id, request).await
            }
            _ => Err(unsupported_bilibili_playlist_source_type(source_type)),
        }
    }

    async fn user_created_playlists(
        &self,
        user_id: &str,
        request: &tuneweave_core::PageRequest,
    ) -> Result<Page<Playlist>> {
        let owner_id = validate_bilibili_user_id(user_id)?;
        let credential = self.optional_request_credential(request.account.as_deref())?;
        let (folders, favorite_folders_hidden) = match self
            .client
            .created_favorite_folders(owner_id, credential.as_ref())
            .await
        {
            Ok(folders) => (folders, false),
            Err(error)
                if error.code == ErrorCode::PermissionDenied
                    && error
                        .details
                        .get("hidden")
                        .and_then(serde_json::Value::as_bool)
                        == Some(true) =>
            {
                (
                    BilibiliCreatedFavoriteFolders {
                        owner_id,
                        folders: Vec::new(),
                    },
                    true,
                )
            }
            Err(error) => return Err(error),
        };
        let limit = request.limit.clamp(1, 100);
        let favorite_folder_total = u32::try_from(folders.folders.len())
            .map_err(|_| bilibili_data_error("Bilibili favorite folder total overflowed"))?;
        let folder_from = usize::try_from(request.offset)
            .unwrap_or(usize::MAX)
            .min(folders.folders.len());
        let folder_to = folder_from
            .saturating_add(limit as usize)
            .min(folders.folders.len());
        let mut items = folders.folders[folder_from..folder_to]
            .iter()
            .cloned()
            .map(map_created_favorite_folder)
            .collect::<Result<Vec<_>>>()?;
        let remaining = limit.saturating_sub(
            u32::try_from(items.len())
                .map_err(|_| bilibili_data_error("Bilibili created playlist page overflowed"))?,
        );
        let space_offset = request.offset.saturating_sub(favorite_folder_total);
        let (space_items, space_total, space_pages_fetched) = self
            .space_playlist_window(owner_id, space_offset, remaining, credential.as_ref())
            .await?;
        items.extend(space_items);
        let total = u64::from(favorite_folder_total)
            .checked_add(space_total)
            .ok_or_else(|| bilibili_data_error("Bilibili created playlist total overflowed"))?;
        let returned = u32::try_from(items.len())
            .map_err(|_| bilibili_data_error("Bilibili created playlist page overflowed"))?;
        let next_offset = request
            .offset
            .checked_add(returned)
            .ok_or_else(|| bilibili_data_error("Bilibili created playlist offset overflowed"))?;
        let has_more = returned > 0 && u64::from(next_offset) < total;
        Ok(Page {
            items,
            pagination: PageMeta {
                limit,
                offset: request.offset,
                total: Some(total),
                next_offset: has_more.then_some(next_offset),
                has_more,
                extensions: Extensions::from([
                    ("library_scope".to_owned(), json!("user_created")),
                    ("user_mid".to_owned(), json!(owner_id)),
                    (
                        "favorite_folder_count".to_owned(),
                        json!(favorite_folder_total),
                    ),
                    (
                        "favorite_folders_hidden".to_owned(),
                        json!(favorite_folders_hidden),
                    ),
                    ("space_playlist_count".to_owned(), json!(space_total)),
                    ("space_pages_fetched".to_owned(), json!(space_pages_fetched)),
                    (
                        "directory_order".to_owned(),
                        json!(["favorite_folder", "season_or_series"]),
                    ),
                ]),
            },
        })
    }

    async fn user_favorite_playlists(
        &self,
        user_id: &str,
        request: &tuneweave_core::PageRequest,
    ) -> Result<Page<Playlist>> {
        const UPSTREAM_PAGE_SIZE: u32 = 70;
        let user_id = validate_bilibili_user_id(user_id)?;
        let credential = self.optional_request_credential(request.account.as_deref())?;
        let limit = request.limit.clamp(1, 100);
        let first_page = request.offset / UPSTREAM_PAGE_SIZE + 1;
        let first_skip = (request.offset % UPSTREAM_PAGE_SIZE) as usize;
        let mut current_page = first_page;
        let mut total = None;
        let mut fetched_pages = 0_u32;
        let mut identities = BTreeSet::new();
        let mut items = Vec::with_capacity(limit as usize);
        while items.len() < limit as usize {
            let page = self
                .client
                .collected_playlists_page(user_id, current_page, credential.as_ref())
                .await?;
            validate_collected_playlist_page(&page, current_page, total)?;
            total.get_or_insert(page.total);
            fetched_pages = fetched_pages.saturating_add(1);
            let skip = if current_page == first_page {
                first_skip
            } else {
                0
            };
            for playlist in page.playlists.into_iter().skip(skip) {
                if items.len() == limit as usize {
                    break;
                }
                if !identities.insert((playlist.kind, playlist.id)) {
                    return Err(bilibili_data_error(
                        "Bilibili collected playlist traversal returned a duplicate identity",
                    ));
                }
                items.push(map_collected_playlist(playlist)?);
            }
            if !page.has_more {
                break;
            }
            current_page = current_page.checked_add(1).ok_or_else(|| {
                bilibili_data_error("Bilibili collected playlist page overflowed")
            })?;
        }
        let total = total.unwrap_or_default();
        let returned = u32::try_from(items.len())
            .map_err(|_| bilibili_data_error("Bilibili collected playlist page overflowed"))?;
        let next_offset = request
            .offset
            .checked_add(returned)
            .ok_or_else(|| bilibili_data_error("Bilibili collected playlist offset overflowed"))?;
        let has_more = returned > 0 && u64::from(next_offset) < total;
        Ok(Page {
            items,
            pagination: PageMeta {
                limit,
                offset: request.offset,
                total: Some(total),
                next_offset: has_more.then_some(next_offset),
                has_more,
                extensions: Extensions::from([
                    ("library_scope".to_owned(), json!("user_favorite")),
                    ("user_mid".to_owned(), json!(user_id)),
                    ("upstream_page_size".to_owned(), json!(UPSTREAM_PAGE_SIZE)),
                    ("upstream_pages_fetched".to_owned(), json!(fetched_pages)),
                    ("includes_collected_seasons".to_owned(), json!(true)),
                ]),
            },
        })
    }

    async fn start_qr_login(&self, login_type: Option<&str>) -> Result<ProviderQrStart> {
        if let Some(login_type) = login_type.map(str::trim).filter(|value| !value.is_empty())
            && !matches!(login_type, "default" | "web" | "bilibili")
        {
            return Err(TuneWeaveError::invalid_request(format!(
                "unsupported Bilibili QR login type: {login_type}"
            ))
            .with_platform(Platform::Bilibili));
        }
        let start = self.client.create_qr_login().await?;
        Ok(ProviderQrStart {
            provider_transaction_id: start.qrcode_key,
            url: start.image_data_url.clone(),
            image_data_url: Some(start.image_data_url),
            expires_at: None,
        })
    }

    async fn poll_qr_login(
        &self,
        provider_transaction_id: &str,
        account: &str,
    ) -> Result<ProviderQrPoll> {
        self.poll_qr_login_with_mode(provider_transaction_id, account, CredentialMode::Server)
            .await
    }

    async fn poll_qr_login_with_mode(
        &self,
        provider_transaction_id: &str,
        account: &str,
        mode: CredentialMode,
    ) -> Result<ProviderQrPoll> {
        validate_bilibili_login_account(account, mode)?;
        match self.client.poll_qr_login(provider_transaction_id).await? {
            BilibiliQrPoll::Waiting => Ok(ProviderQrPoll {
                state: AuthState::Waiting,
                message: Some("waiting for Bilibili QR scan".to_owned()),
                profile: None,
                credential: None,
            }),
            BilibiliQrPoll::Scanned => Ok(ProviderQrPoll {
                state: AuthState::Scanned,
                message: Some("Bilibili QR scanned; waiting for confirmation".to_owned()),
                profile: None,
                credential: None,
            }),
            BilibiliQrPoll::Expired => Ok(ProviderQrPoll {
                state: AuthState::Expired,
                message: Some("Bilibili QR login expired".to_owned()),
                profile: None,
                credential: None,
            }),
            BilibiliQrPoll::Failed { code, message } => Ok(ProviderQrPoll {
                state: AuthState::Failed,
                message: Some(format!("{message} ({code})")),
                profile: None,
                credential: None,
            }),
            BilibiliQrPoll::Confirmed {
                credential,
                timestamp_ms,
            } => {
                let mut result = self.finish_authentication(account, &credential, mode)?;
                if let Some(timestamp_ms) = timestamp_ms {
                    result
                        .profile
                        .extensions
                        .insert("login_timestamp_ms".to_owned(), json!(timestamp_ms));
                }
                Ok(ProviderQrPoll {
                    state: AuthState::Confirmed,
                    message: Some("Bilibili account authenticated".to_owned()),
                    profile: Some(result.profile),
                    credential: result.credential,
                })
            }
        }
    }

    async fn session_profile(&self, account: &str) -> Result<AccountProfile> {
        let Some(credential) = self.selected_credential_optional(account)? else {
            return Ok(unauthenticated_bilibili_profile(account, None));
        };
        let status = self.client.session_status(Some(&credential)).await?;
        Ok(map_bilibili_session_profile(account, &credential, status))
    }

    async fn refresh_session(&self, account: &str) -> Result<AccountProfile> {
        Ok(self
            .refresh_session_with_ownership(account, None, CredentialMode::Server)
            .await?
            .profile)
    }

    async fn refresh_session_with_ownership(
        &self,
        account: &str,
        source_credential: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        let credential = bilibili_refresh_source(self, account, source_credential, mode)?;
        let BilibiliCredentialRefresh {
            credential,
            status,
            refreshed,
        } = self.client.refresh_credential(&credential).await?;
        let mut result = self.finish_authentication(account, &credential, mode)?;
        result.profile = map_bilibili_session_profile(account, &credential, status);
        result
            .profile
            .extensions
            .insert("refreshed".to_owned(), json!(refreshed));
        Ok(result)
    }

    async fn logout(&self, account: &str) -> Result<bool> {
        Ok(self
            .logout_with_ownership(account, None, CredentialMode::Server)
            .await?
            .removed)
    }

    async fn logout_with_ownership(
        &self,
        account: &str,
        source_credential: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<ProviderLogoutResult> {
        let caller_credential_discard_required = source_credential.is_some();
        let Some(credential) = bilibili_logout_source(self, account, source_credential, mode)?
        else {
            return Ok(ProviderLogoutResult {
                removed: false,
                caller_credential_discard_required,
            });
        };
        let outcome = self.client.logout(&credential).await?;
        let removed = if mode.persists_on_server() {
            self.remove_bilibili_credential(account).map_err(|_| {
                let upstream_state = match outcome {
                    BilibiliLogoutOutcome::LoggedOut => "logged_out",
                    BilibiliLogoutOutcome::CredentialExpired => "credential_expired",
                };
                TuneWeaveError::new(
                    ErrorCode::InternalError,
                    "Bilibili account was closed upstream but local credential removal failed",
                )
                .with_platform(Platform::Bilibili)
                .with_details(json!({ "upstream_state": upstream_state }))
            })?
        } else {
            false
        };
        Ok(ProviderLogoutResult {
            removed,
            caller_credential_discard_required,
        })
    }
}

impl BilibiliProvider {
    async fn favorite_media_window(
        &self,
        media_id: u64,
        request: &PageRequest,
        credential: Option<&BilibiliCredential>,
    ) -> Result<(Vec<BilibiliFavoriteMedia>, PageMeta)> {
        const UPSTREAM_PAGE_SIZE: u32 = 20;
        if !(1..=100).contains(&request.limit) {
            return Err(bilibili_invalid_request(
                "Bilibili playlist limit must be between 1 and 100",
            ));
        }
        let first_page = request.offset / UPSTREAM_PAGE_SIZE + 1;
        let first_skip = (request.offset % UPSTREAM_PAGE_SIZE) as usize;
        let mut current_page = first_page;
        let mut total = None;
        let mut owner_id = None;
        let mut fetched_pages = 0_u32;
        let mut identities = BTreeSet::new();
        let mut medias = Vec::with_capacity(request.limit as usize);
        let mut next_cursor = request.offset;
        while medias.len() < request.limit as usize {
            let page = self
                .client
                .favorite_media_page(media_id, current_page, credential)
                .await?;
            if page.page != current_page
                || page.page_size != UPSTREAM_PAGE_SIZE
                || page.folder.media_id != media_id
                || page.folder.media_count != page.total
                || total.is_some_and(|expected| expected != page.total)
                || owner_id.is_some_and(|expected| expected != page.folder.owner.id)
            {
                return Err(bilibili_data_error(
                    "Bilibili favorite media pagination changed during traversal",
                ));
            }
            total.get_or_insert(page.total);
            owner_id.get_or_insert(page.folder.owner.id);
            fetched_pages = fetched_pages.saturating_add(1);
            let skip = if current_page == first_page {
                first_skip
            } else {
                0
            };
            for (index, media) in page.medias.into_iter().enumerate().skip(skip) {
                if medias.len() == request.limit as usize {
                    break;
                }
                if !identities.insert(media.aid) {
                    return Err(bilibili_data_error(
                        "Bilibili favorite media traversal returned a duplicate identity",
                    ));
                }
                medias.push(media);
                let page_start = (current_page - 1).saturating_mul(UPSTREAM_PAGE_SIZE);
                next_cursor = page_start
                    .saturating_add(u32::try_from(index).unwrap_or(u32::MAX))
                    .saturating_add(1);
            }
            if medias.len() == request.limit as usize || !page.has_more {
                break;
            }
            next_cursor = current_page.saturating_mul(UPSTREAM_PAGE_SIZE);
            current_page = current_page
                .checked_add(1)
                .ok_or_else(|| bilibili_data_error("Bilibili favorite media page overflowed"))?;
        }
        let total = total.unwrap_or_default();
        let returned = u32::try_from(medias.len())
            .map_err(|_| bilibili_data_error("Bilibili favorite media page overflowed"))?;
        let has_more = returned > 0 && u64::from(next_cursor) < total;
        Ok((
            medias,
            PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(total),
                next_offset: has_more.then_some(next_cursor),
                has_more,
                extensions: Extensions::from([
                    ("collection_kind".to_owned(), json!("favorite_folder")),
                    ("media_id".to_owned(), json!(media_id)),
                    ("owner_mid".to_owned(), json!(owner_id)),
                    ("media_type".to_owned(), json!("video")),
                    ("upstream_page_size".to_owned(), json!(UPSTREAM_PAGE_SIZE)),
                    ("upstream_pages_fetched".to_owned(), json!(fetched_pages)),
                ]),
            },
        ))
    }

    async fn season_archive_window(
        &self,
        season_id: u64,
        request: &PageRequest,
        credential: Option<&BilibiliCredential>,
    ) -> Result<(Vec<BilibiliSeasonArchive>, PageMeta, u64)> {
        const UPSTREAM_PAGE_SIZE: u32 = 30;
        if !(1..=100).contains(&request.limit) {
            return Err(bilibili_invalid_request(
                "Bilibili playlist limit must be between 1 and 100",
            ));
        }
        let first_page = request.offset / UPSTREAM_PAGE_SIZE + 1;
        let first_skip = (request.offset % UPSTREAM_PAGE_SIZE) as usize;
        let mut current_page = first_page;
        let mut total = None;
        let mut owner_id = None;
        let mut fetched_pages = 0_u32;
        let mut identities = BTreeSet::new();
        let mut archives = Vec::with_capacity(request.limit as usize);

        while archives.len() < request.limit as usize {
            let page = self
                .client
                .season_archives_page(season_id, current_page, credential)
                .await?;
            if page.page != current_page
                || page.page_size != UPSTREAM_PAGE_SIZE
                || page.season.id != season_id
                || page.season.track_count != page.total
                || total.is_some_and(|expected| expected != page.total)
                || owner_id.is_some_and(|expected| expected != page.season.owner_id)
            {
                return Err(bilibili_data_error(
                    "Bilibili season archive pagination changed during traversal",
                ));
            }
            total.get_or_insert(page.total);
            owner_id.get_or_insert(page.season.owner_id);
            fetched_pages = fetched_pages.saturating_add(1);
            let skip = if current_page == first_page {
                first_skip
            } else {
                0
            };
            if request.offset < u32::try_from(page.total).unwrap_or(u32::MAX)
                && skip > page.archives.len()
            {
                return Err(bilibili_data_error(
                    "Bilibili season archive page did not cover the requested offset",
                ));
            }
            for archive in page.archives.into_iter().skip(skip) {
                if archives.len() == request.limit as usize {
                    break;
                }
                if !identities.insert((archive.aid, archive.bvid.clone())) {
                    return Err(bilibili_data_error(
                        "Bilibili season archive traversal returned a duplicate identity",
                    ));
                }
                archives.push(archive);
            }
            if archives.len() == request.limit as usize || !page.has_more {
                break;
            }
            current_page = current_page
                .checked_add(1)
                .ok_or_else(|| bilibili_data_error("Bilibili season archive page overflowed"))?;
        }

        let total = total.unwrap_or_default();
        let owner_id = owner_id
            .ok_or_else(|| bilibili_data_error("Bilibili season did not return an owner"))?;
        let returned = u32::try_from(archives.len())
            .map_err(|_| bilibili_data_error("Bilibili season archive page overflowed"))?;
        let next_offset = request
            .offset
            .checked_add(returned)
            .ok_or_else(|| bilibili_data_error("Bilibili season archive offset overflowed"))?;
        let has_more = returned > 0 && u64::from(next_offset) < total;
        Ok((
            archives,
            PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(total),
                next_offset: has_more.then_some(next_offset),
                has_more,
                extensions: Extensions::from([
                    ("collection_kind".to_owned(), json!("season")),
                    ("season_id".to_owned(), json!(season_id)),
                    ("owner_mid".to_owned(), json!(owner_id)),
                    ("media_type".to_owned(), json!("video")),
                    ("upstream_page_size".to_owned(), json!(UPSTREAM_PAGE_SIZE)),
                    ("upstream_pages_fetched".to_owned(), json!(fetched_pages)),
                ]),
            },
            owner_id,
        ))
    }

    async fn space_playlist_window(
        &self,
        user_id: u64,
        offset: u32,
        limit: u32,
        credential: Option<&BilibiliCredential>,
    ) -> Result<(Vec<Playlist>, u64, u32)> {
        const UPSTREAM_PAGE_SIZE: u32 = 20;
        let first_page = if limit == 0 {
            1
        } else {
            offset / UPSTREAM_PAGE_SIZE + 1
        };
        let first_skip = if limit == 0 {
            0
        } else {
            (offset % UPSTREAM_PAGE_SIZE) as usize
        };
        let mut current_page = first_page;
        let mut total = None;
        let mut fetched_pages = 0_u32;
        let mut identities = BTreeSet::new();
        let mut items = Vec::with_capacity(limit as usize);
        loop {
            let page = self
                .client
                .space_playlists_page(user_id, current_page, credential)
                .await?;
            validate_space_playlist_page(&page, current_page, total)?;
            total.get_or_insert(page.total);
            fetched_pages = fetched_pages.saturating_add(1);
            if limit == 0 {
                break;
            }
            let skip = if current_page == first_page {
                first_skip
            } else {
                0
            };
            for playlist in page.playlists.into_iter().skip(skip) {
                if items.len() == limit as usize {
                    break;
                }
                if !identities.insert((playlist.kind, playlist.id)) {
                    return Err(bilibili_data_error(
                        "Bilibili space playlist traversal returned a duplicate identity",
                    ));
                }
                items.push(map_space_playlist(playlist)?);
            }
            if items.len() == limit as usize || !page.has_more {
                break;
            }
            current_page = current_page
                .checked_add(1)
                .ok_or_else(|| bilibili_data_error("Bilibili space playlist page overflowed"))?;
        }
        Ok((items, total.unwrap_or_default(), fetched_pages))
    }

    fn caller_credential_scope(&self, credential: &ProviderCredential) -> Result<Self> {
        Ok(Self {
            client: self.client.clone(),
            credential_store: None,
            caller_credential: Some(parse_bilibili_caller_credential(credential)?),
        })
    }

    fn finish_authentication(
        &self,
        account: &str,
        credential: &BilibiliCredential,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        validate_bilibili_login_account(account, mode)?;
        let secret = serde_json::to_string(credential).map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "failed to serialize Bilibili account credential",
            )
            .with_platform(Platform::Bilibili)
        })?;
        let caller_credential = mode
            .returns_to_caller()
            .then(|| {
                ProviderCredential::new(Platform::Bilibili, BILIBILI_CREDENTIAL_KIND, &secret, None)
            })
            .transpose()?;
        if mode.persists_on_server() {
            let store = self.credential_store.as_ref().ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::InternalError,
                    "Bilibili account storage is not configured",
                )
                .with_platform(Platform::Bilibili)
            })?;
            store.put(&StoredAccountCredential::new(
                Platform::Bilibili,
                account,
                BILIBILI_CREDENTIAL_KIND,
                &secret,
            )?)?;
        }
        let mut profile = AccountProfile::authenticated(Platform::Bilibili, account);
        profile.user_id = Some(credential.user_id().to_owned());
        profile.extensions.insert(
            "credential_kind".to_owned(),
            json!(BILIBILI_CREDENTIAL_KIND),
        );
        Ok(ProviderAuthResult {
            profile,
            credential: caller_credential,
        })
    }

    fn selected_credential(&self, account: &str) -> Result<BilibiliCredential> {
        self.selected_credential_optional(account)?.ok_or_else(|| {
            bilibili_authentication_required(account, "Bilibili account was not found")
        })
    }

    fn selected_credential_optional(&self, account: &str) -> Result<Option<BilibiliCredential>> {
        if let Some(credential) = &self.caller_credential {
            if account == "default" {
                return Ok(Some(credential.clone()));
            }
            return Err(bilibili_authentication_required(
                account,
                "caller-managed Bilibili credentials do not expose server account aliases",
            ));
        }
        let Some(store) = self.credential_store.as_ref() else {
            return Ok(None);
        };
        let Some(stored) = store
            .load_platform(Platform::Bilibili)?
            .into_iter()
            .find(|credential| credential.account == account)
        else {
            return Ok(None);
        };
        if stored.kind != BILIBILI_CREDENTIAL_KIND {
            return Err(TuneWeaveError::new(
                ErrorCode::InternalError,
                "stored Bilibili credential has an unsupported kind",
            )
            .with_platform(Platform::Bilibili));
        }
        serde_json::from_str::<BilibiliCredential>(stored.secret())
            .map_err(|_| {
                TuneWeaveError::new(
                    ErrorCode::InternalError,
                    "stored Bilibili credential is malformed",
                )
                .with_platform(Platform::Bilibili)
            })?
            .normalize()
            .map_err(|_| {
                TuneWeaveError::new(
                    ErrorCode::InternalError,
                    "stored Bilibili credential is invalid",
                )
                .with_platform(Platform::Bilibili)
            })
            .map(Some)
    }

    fn optional_request_credential(
        &self,
        account: Option<&str>,
    ) -> Result<Option<BilibiliCredential>> {
        if let Some(account) = account {
            return self.selected_credential(account).map(Some);
        }
        Ok(self.caller_credential.clone())
    }

    fn remove_bilibili_credential(&self, account: &str) -> Result<bool> {
        let store = self.credential_store.as_ref().ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                "Bilibili account storage is not configured",
            )
            .with_platform(Platform::Bilibili)
        })?;
        store.remove(Platform::Bilibili, account.trim())
    }
}

fn validate_bilibili_login_account(account: &str, mode: CredentialMode) -> Result<()> {
    let account = account.trim();
    if account.is_empty() {
        return Err(
            TuneWeaveError::invalid_request("Bilibili account alias cannot be empty")
                .with_platform(Platform::Bilibili),
        );
    }
    if account.len() > 64 {
        return Err(TuneWeaveError::invalid_request(
            "Bilibili account alias cannot exceed 64 bytes",
        )
        .with_platform(Platform::Bilibili));
    }
    if mode == CredentialMode::Client && account != "default" {
        return Err(TuneWeaveError::invalid_request(
            "client credential mode does not accept a server account alias",
        )
        .with_platform(Platform::Bilibili));
    }
    Ok(())
}

fn map_bilibili_video_search_filters(
    filters: Option<&VideoSearchFilters>,
) -> Result<BilibiliVideoSearchFilters> {
    let filters = filters.cloned().unwrap_or_default();
    let order = match filters.order {
        VideoSearchOrder::Relevance => BilibiliVideoSearchOrder::Relevance,
        VideoSearchOrder::MostPlayed => BilibiliVideoSearchOrder::MostPlayed,
        VideoSearchOrder::Newest => BilibiliVideoSearchOrder::Newest,
        VideoSearchOrder::MostDanmaku => BilibiliVideoSearchOrder::MostDanmaku,
        VideoSearchOrder::MostFavorited => BilibiliVideoSearchOrder::MostFavorited,
        VideoSearchOrder::MostCommented => BilibiliVideoSearchOrder::MostCommented,
    };
    let duration = match filters.duration {
        VideoSearchDuration::Any => BilibiliVideoSearchDuration::Any,
        VideoSearchDuration::UnderTenMinutes => BilibiliVideoSearchDuration::UnderTenMinutes,
        VideoSearchDuration::TenToThirtyMinutes => BilibiliVideoSearchDuration::TenToThirtyMinutes,
        VideoSearchDuration::ThirtyToSixtyMinutes => {
            BilibiliVideoSearchDuration::ThirtyToSixtyMinutes
        }
        VideoSearchDuration::OverSixtyMinutes => BilibiliVideoSearchDuration::OverSixtyMinutes,
    };
    let category_id = filters
        .category_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "0")
        .map(|value| {
            value
                .parse::<u32>()
                .ok()
                .filter(|value| *value > 0)
                .ok_or_else(|| {
                    bilibili_invalid_request(
                        "Bilibili video search category ID must be a positive integer",
                    )
                })
        })
        .transpose()?;
    Ok(BilibiliVideoSearchFilters {
        order,
        duration,
        category_id,
    })
}

fn validate_bilibili_user_id(value: &str) -> Result<u64> {
    let value = value.trim();
    if value.is_empty() || value.len() > 20 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(bilibili_invalid_request(
            "Bilibili user ID must be a positive integer",
        ));
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| bilibili_invalid_request("Bilibili user ID must be a positive integer"))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BilibiliPlaylistLocator {
    Season(u64),
    FavoriteFolder(u64),
    Series(u64),
}

fn parse_bilibili_playlist_locator(value: &str) -> Result<BilibiliPlaylistLocator> {
    let value = value.trim();
    let (kind, id) = value.split_once(':').ok_or_else(|| {
        bilibili_invalid_request(
            "Bilibili playlist ID must include season, favorite, or series type",
        )
    })?;
    if id.is_empty()
        || id.len() > 20
        || !id.bytes().all(|byte| byte.is_ascii_digit())
        || id.starts_with('0')
    {
        return Err(bilibili_invalid_request(
            "Bilibili playlist ID must contain a positive numeric identity",
        ));
    }
    let id = id.parse::<u64>().ok().filter(|id| *id > 0).ok_or_else(|| {
        bilibili_invalid_request("Bilibili playlist ID must contain a positive numeric identity")
    })?;
    match kind {
        "season" => Ok(BilibiliPlaylistLocator::Season(id)),
        "favorite" => Ok(BilibiliPlaylistLocator::FavoriteFolder(id)),
        "series" => Ok(BilibiliPlaylistLocator::Series(id)),
        _ => Err(bilibili_invalid_request(
            "Bilibili playlist ID has an unsupported type",
        )),
    }
}

fn unsupported_bilibili_playlist_kind(kind: &str) -> TuneWeaveError {
    TuneWeaveError::unsupported(Platform::Bilibili, Capability::PlaylistRead)
        .with_details(json!({ "playlist_kind": kind }))
}

fn bilibili_source_playlist_id(id: &str, kind: &str) -> Result<String> {
    let typed_id = format!("{kind}:{}", id.trim());
    parse_bilibili_playlist_locator(&typed_id)?;
    Ok(typed_id)
}

fn unsupported_bilibili_playlist_source_type(source_type: &str) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::CapabilityNotSupported,
        format!("Bilibili does not support playlist source type {source_type}"),
    )
    .with_platform(Platform::Bilibili)
    .with_details(json!({ "source_type": source_type }))
}

fn bilibili_refresh_source(
    provider: &BilibiliProvider,
    account: &str,
    source_credential: Option<&ProviderCredential>,
    mode: CredentialMode,
) -> Result<BilibiliCredential> {
    validate_bilibili_login_account(account, mode)?;
    match source_credential {
        None if mode == CredentialMode::Client => Err(TuneWeaveError::invalid_request(
            "client credential refresh requires a caller credential",
        )
        .with_platform(Platform::Bilibili)),
        None => provider
            .selected_credential_optional(account)?
            .ok_or_else(|| {
                bilibili_authentication_required(account, "Bilibili account was not found")
            }),
        Some(_) if mode == CredentialMode::Server => Err(TuneWeaveError::invalid_request(
            "a caller credential cannot refresh a server-only session",
        )
        .with_platform(Platform::Bilibili)),
        Some(source) => {
            let caller = parse_bilibili_caller_credential(source)?;
            if mode == CredentialMode::Both {
                let stored = provider
                    .selected_credential_optional(account)?
                    .ok_or_else(|| {
                        bilibili_authentication_required(account, "Bilibili account was not found")
                    })?;
                ensure_matching_bilibili_identity(&caller, &stored)?;
            }
            Ok(caller)
        }
    }
}

fn bilibili_logout_source(
    provider: &BilibiliProvider,
    account: &str,
    source_credential: Option<&ProviderCredential>,
    mode: CredentialMode,
) -> Result<Option<BilibiliCredential>> {
    validate_bilibili_login_account(account, mode)?;
    match source_credential {
        None if mode != CredentialMode::Server => Err(TuneWeaveError::invalid_request(
            "caller-managed logout requires a caller credential",
        )
        .with_platform(Platform::Bilibili)),
        None => provider.selected_credential_optional(account),
        Some(_) if mode == CredentialMode::Server => Err(TuneWeaveError::invalid_request(
            "a caller credential cannot close a server-only session",
        )
        .with_platform(Platform::Bilibili)),
        Some(source) => {
            let caller = parse_bilibili_caller_credential(source)?;
            if mode == CredentialMode::Both {
                let stored = provider
                    .selected_credential_optional(account)?
                    .ok_or_else(|| {
                        bilibili_authentication_required(account, "Bilibili account was not found")
                    })?;
                ensure_matching_bilibili_identity(&caller, &stored)?;
            }
            Ok(Some(caller))
        }
    }
}

fn ensure_matching_bilibili_identity(
    caller: &BilibiliCredential,
    stored: &BilibiliCredential,
) -> Result<()> {
    if caller.user_id() != stored.user_id() {
        return Err(TuneWeaveError::invalid_request(
            "caller credential and server account refer to different Bilibili identities",
        )
        .with_platform(Platform::Bilibili));
    }
    Ok(())
}

fn parse_bilibili_caller_credential(credential: &ProviderCredential) -> Result<BilibiliCredential> {
    if credential.platform != Platform::Bilibili {
        return Err(TuneWeaveError::invalid_request(
            "caller credential platform does not match Bilibili",
        )
        .with_platform(Platform::Bilibili));
    }
    if credential.kind != BILIBILI_CREDENTIAL_KIND {
        return Err(TuneWeaveError::invalid_request(
            "caller credential kind is not supported by Bilibili",
        )
        .with_platform(Platform::Bilibili));
    }
    if credential.expires_at.is_some() {
        return Err(TuneWeaveError::invalid_request(
            "caller Bilibili credential expiry does not match its payload",
        )
        .with_platform(Platform::Bilibili));
    }
    serde_json::from_str::<BilibiliCredential>(credential.secret())
        .map_err(|_| {
            TuneWeaveError::invalid_request("caller Bilibili credential payload is malformed")
                .with_platform(Platform::Bilibili)
        })?
        .normalize()
        .map_err(|_| {
            TuneWeaveError::invalid_request("caller Bilibili credential payload is invalid")
                .with_platform(Platform::Bilibili)
        })
}

fn bilibili_authentication_required(account: &str, message: &str) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::AuthenticationRequired, message)
        .with_platform(Platform::Bilibili)
        .with_details(json!({ "account": account }))
}

fn unauthenticated_bilibili_profile(account: &str, user_id: Option<&str>) -> AccountProfile {
    let mut profile = AccountProfile::authenticated(Platform::Bilibili, account);
    profile.authenticated = false;
    profile.user_id = user_id.map(str::to_owned);
    profile
}

fn map_bilibili_session_profile(
    account: &str,
    credential: &BilibiliCredential,
    status: BilibiliSessionStatus,
) -> AccountProfile {
    let mut profile = if status.authenticated {
        AccountProfile::authenticated(Platform::Bilibili, account)
    } else {
        unauthenticated_bilibili_profile(account, Some(credential.user_id()))
    };
    profile.user_id = status
        .user_id
        .or_else(|| Some(credential.user_id().to_owned()));
    profile.nickname = status.nickname;
    profile.avatar_url = status.avatar_url;
    profile.extensions = status.extensions;
    profile
}

fn map_bilibili_search_video(item: BilibiliSearchVideo) -> Result<Video> {
    let identity = item.bvid.as_ref().map_or_else(
        || BilibiliVideoIdentity::Aid(item.aid),
        |bvid| BilibiliVideoIdentity::Bvid(bvid.clone()),
    );
    let id = identity.canonical_id();
    let resource_ref = identity.resource_ref()?;
    let creator_ref = ResourceRef::new(Platform::Bilibili, format!("user:{}", item.author_id))
        .map_err(|_| bilibili_data_error("Bilibili search creator identity was invalid"))?;
    let mut extensions = Extensions::from([
        ("aid".to_owned(), json!(item.aid)),
        ("duration_text".to_owned(), json!(item.duration_text)),
        ("tags".to_owned(), json!(item.tags)),
        ("hit_columns".to_owned(), json!(item.hit_columns)),
    ]);
    insert_optional(&mut extensions, "bvid", item.bvid);
    insert_optional(&mut extensions, "danmaku_count", item.danmaku_count);
    insert_optional(&mut extensions, "favorite_count", item.favorite_count);
    insert_optional(&mut extensions, "comment_count", item.comment_count);
    insert_optional(&mut extensions, "category_id", item.category_id);
    insert_optional(&mut extensions, "category_name", item.category_name);
    insert_optional(&mut extensions, "sent_at_unix", item.sent_at);
    insert_optional(&mut extensions, "paid", item.paid);
    insert_optional(&mut extensions, "collaborative", item.collaborative);
    insert_optional(&mut extensions, "rank_score", item.rank_score);
    Ok(Video {
        resource_ref,
        platform: Platform::Bilibili,
        id,
        title: item.title,
        creators: vec![CreatorSummary {
            resource_ref: Some(creator_ref),
            name: item.author,
            avatar_url: None,
        }],
        description: item.description,
        cover_url: Some(item.cover_url),
        duration_ms: item.duration_seconds.checked_mul(1_000),
        published_at: item.published_at.and_then(bilibili_unix_rfc3339),
        play_count: item.play_count,
        subscribed: None,
        extensions,
    })
}

fn map_bilibili_video_view(view: BilibiliVideoView) -> Result<VideoDetail> {
    let identity = BilibiliVideoIdentity::Bvid(view.bvid.clone());
    let resource_ref = identity.resource_ref()?;
    let owner_ref = ResourceRef::new(Platform::Bilibili, format!("user:{}", view.owner.id))
        .map_err(|_| bilibili_data_error("Bilibili video owner identity was invalid"))?;
    let duration_ms = view
        .duration_seconds
        .checked_mul(1_000)
        .ok_or_else(|| bilibili_data_error("Bilibili video duration overflowed"))?;
    let first_part = view
        .parts
        .first()
        .ok_or_else(|| bilibili_data_error("Bilibili video did not contain a first part"))?;
    let copyright = match view.copyright {
        1 => "original",
        2 => "repost",
        3 => "unspecified",
        _ => unreachable!("client accepted an unknown copyright value"),
    };
    let mut extensions = Extensions::from([
        ("aid".to_owned(), json!(view.aid)),
        ("bvid".to_owned(), json!(view.bvid)),
        ("state".to_owned(), json!(view.state)),
        ("copyright".to_owned(), json!(copyright)),
        ("category_id".to_owned(), json!(view.category_id)),
        ("dynamic_text".to_owned(), json!(view.dynamic_text)),
        ("part_count".to_owned(), json!(view.parts.len())),
        ("first_cid".to_owned(), json!(first_part.cid)),
        ("first_part_title".to_owned(), json!(first_part.title)),
        (
            "first_part_duration_seconds".to_owned(),
            json!(first_part.duration_seconds),
        ),
        ("first_part_source".to_owned(), json!(first_part.source)),
        ("first_part_width".to_owned(), json!(first_part.width)),
        ("first_part_height".to_owned(), json!(first_part.height)),
        ("first_part_rotated".to_owned(), json!(first_part.rotated)),
        ("danmaku_count".to_owned(), json!(view.stats.danmaku)),
        ("comment_count".to_owned(), json!(view.stats.reply)),
        ("favorite_count".to_owned(), json!(view.stats.favorite)),
        ("coin_count".to_owned(), json!(view.stats.coin)),
        ("share_count".to_owned(), json!(view.stats.share)),
        ("like_count".to_owned(), json!(view.stats.like)),
        ("current_rank".to_owned(), json!(view.stats.now_rank)),
        ("historic_rank".to_owned(), json!(view.stats.his_rank)),
        ("download_allowed".to_owned(), json!(view.rights.download)),
        ("movie".to_owned(), json!(view.rights.movie)),
        ("pay".to_owned(), json!(view.rights.pay)),
        ("high_bitrate".to_owned(), json!(view.rights.high_bitrate)),
        ("no_reprint".to_owned(), json!(view.rights.no_reprint)),
        ("ugc_pay".to_owned(), json!(view.rights.ugc_pay)),
        ("cooperation".to_owned(), json!(view.rights.cooperation)),
        ("interactive".to_owned(), json!(view.rights.interactive)),
        ("panoramic".to_owned(), json!(view.rights.panoramic)),
        ("no_share".to_owned(), json!(view.rights.no_share)),
        ("free_watch".to_owned(), json!(view.rights.free_watch)),
    ]);
    insert_optional(&mut extensions, "category_id_v2", view.category_id_v2);
    insert_optional(&mut extensions, "category_name", view.category_name);
    insert_optional(&mut extensions, "category_name_v2", view.category_name_v2);
    if view.created_at > 0 {
        extensions.insert("ctime".to_owned(), json!(view.created_at));
    }
    let video = Video {
        id: resource_ref.id().to_owned(),
        resource_ref,
        platform: Platform::Bilibili,
        title: view.title,
        creators: vec![CreatorSummary {
            resource_ref: Some(owner_ref),
            name: view.owner.name,
            avatar_url: view.owner.avatar_url,
        }],
        description: view.description,
        cover_url: Some(view.cover_url),
        duration_ms: Some(duration_ms),
        published_at: (view.published_at > 0)
            .then(|| bilibili_unix_rfc3339(view.published_at))
            .flatten(),
        play_count: Some(view.stats.view),
        subscribed: None,
        extensions,
    };
    Ok(VideoDetail {
        kind: VideoResourceKind::Video,
        video,
        resolutions: Vec::new(),
        extensions: Extensions::from([
            ("detail_source".to_owned(), json!("web_interface_view")),
            ("resolutions_require_playurl".to_owned(), json!(true)),
        ]),
    })
}

fn map_bilibili_video_parts(
    aid: u64,
    bvid: String,
    upstream_parts: Vec<BilibiliVideoPart>,
    request: &VideoPartListRequest,
) -> Result<Page<VideoPart>> {
    let video_ref = BilibiliVideoIdentity::Bvid(bvid.clone()).resource_ref()?;
    let total = u64::try_from(upstream_parts.len())
        .map_err(|_| bilibili_data_error("Bilibili video part count overflowed"))?;
    let mut parts = Vec::with_capacity(upstream_parts.len());
    for part in upstream_parts {
        let id = format!("cid:{}", part.cid);
        let resource_ref = ResourceRef::new(Platform::Bilibili, &id)
            .map_err(|_| bilibili_data_error("Bilibili video part identity was invalid"))?;
        let page = u32::try_from(part.page)
            .map_err(|_| bilibili_data_error("Bilibili video part page overflowed"))?;
        let width = u32::try_from(part.width)
            .map_err(|_| bilibili_data_error("Bilibili video part width overflowed"))?;
        let height = u32::try_from(part.height)
            .map_err(|_| bilibili_data_error("Bilibili video part height overflowed"))?;
        let duration_ms = part
            .duration_seconds
            .checked_mul(1_000)
            .ok_or_else(|| bilibili_data_error("Bilibili video part duration overflowed"))?;
        parts.push(VideoPart {
            resource_ref,
            video_ref: video_ref.clone(),
            platform: Platform::Bilibili,
            id,
            page,
            title: part.title,
            duration_ms: (duration_ms > 0).then_some(duration_ms),
            width: (width > 0).then_some(width),
            height: (height > 0).then_some(height),
            extensions: Extensions::from([
                ("aid".to_owned(), json!(aid)),
                ("bvid".to_owned(), json!(bvid)),
                ("cid".to_owned(), json!(part.cid)),
                ("source".to_owned(), json!(part.source)),
                ("rotated".to_owned(), json!(part.rotated)),
            ]),
        });
    }
    let start = usize::try_from(request.offset)
        .map_err(|_| bilibili_invalid_request("Bilibili video part offset overflowed"))?;
    let limit = usize::try_from(request.limit)
        .map_err(|_| bilibili_invalid_request("Bilibili video part limit overflowed"))?;
    let items = parts
        .into_iter()
        .skip(start)
        .take(limit)
        .collect::<Vec<_>>();
    let returned = u32::try_from(items.len())
        .map_err(|_| bilibili_data_error("Bilibili video part page overflowed"))?;
    let next_offset = request
        .offset
        .checked_add(returned)
        .ok_or_else(|| bilibili_data_error("Bilibili video part offset overflowed"))?;
    let has_more = returned > 0 && u64::from(next_offset) < total;
    Ok(Page {
        items,
        pagination: PageMeta {
            limit: request.limit,
            offset: request.offset,
            total: Some(total),
            next_offset: has_more.then_some(next_offset),
            has_more,
            extensions: Extensions::from([
                ("detail_source".to_owned(), json!("web_interface_view")),
                ("video_ref".to_owned(), json!(video_ref)),
            ]),
        },
    })
}

fn parse_bilibili_part_id(value: &str) -> Result<u64> {
    let value = value.trim();
    let Some(value) = value.strip_prefix("cid:") else {
        return Err(bilibili_invalid_request(
            "Bilibili subtitle part must use the cid:<id> identity",
        ));
    };
    if value.is_empty()
        || value.len() > 20
        || value.starts_with('0')
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(bilibili_invalid_request(
            "Bilibili subtitle CID must be a positive integer",
        ));
    }
    value
        .parse::<u64>()
        .ok()
        .filter(|cid| *cid > 0)
        .ok_or_else(|| bilibili_invalid_request("Bilibili subtitle CID must be a positive integer"))
}

fn map_bilibili_subtitle_catalog(catalog: BilibiliSubtitleCatalog) -> Result<VideoSubtitleList> {
    let video_ref = BilibiliVideoIdentity::Bvid(catalog.bvid.clone()).resource_ref()?;
    let part_id = format!("cid:{}", catalog.cid);
    let part_ref = ResourceRef::new(Platform::Bilibili, &part_id)
        .map_err(|_| bilibili_data_error("Bilibili subtitle part identity was invalid"))?;
    let mut items = Vec::with_capacity(catalog.subtitles.len());
    for subtitle in catalog.subtitles {
        let id = format!("subtitle:{}", subtitle.id_string);
        let resource_ref = ResourceRef::new(Platform::Bilibili, &id)
            .map_err(|_| bilibili_data_error("Bilibili subtitle identity was invalid"))?;
        items.push(VideoSubtitle {
            resource_ref,
            video_ref: video_ref.clone(),
            part_ref: part_ref.clone(),
            platform: Platform::Bilibili,
            id,
            language: subtitle.language,
            label: subtitle.label,
            format: "bilibili_json".to_owned(),
            locked: Some(subtitle.locked),
            extensions: Extensions::from([
                ("numeric_id".to_owned(), json!(subtitle.id)),
                ("id_string".to_owned(), json!(subtitle.id_string)),
                ("subtitle_type".to_owned(), json!(subtitle.subtitle_type)),
                ("ai_type".to_owned(), json!(subtitle.ai_type)),
                ("ai_status".to_owned(), json!(subtitle.ai_status)),
                ("content_available".to_owned(), json!(true)),
            ]),
        });
    }
    Ok(VideoSubtitleList {
        video_ref: video_ref.clone(),
        part_ref: part_ref.clone(),
        platform: Platform::Bilibili,
        requires_login: catalog.requires_login,
        can_submit: Some(catalog.can_submit),
        default_language: catalog.default_language,
        default_language_label: catalog.default_language_label,
        items,
        extensions: Extensions::from([
            ("aid".to_owned(), json!(catalog.aid)),
            ("bvid".to_owned(), json!(catalog.bvid)),
            ("cid".to_owned(), json!(catalog.cid)),
            ("video_ref".to_owned(), json!(video_ref)),
            ("part_ref".to_owned(), json!(part_ref)),
            ("catalog_source".to_owned(), json!("player_wbi_v2")),
        ]),
    })
}

fn map_season_archive_track(
    archive: BilibiliSeasonArchive,
    season_id: u64,
    owner_id: u64,
) -> Result<Track> {
    let resource_ref = bilibili_archive_ref(&archive.bvid)?;
    let owner_ref = bilibili_archive_owner_ref(owner_id)?;
    let duration_ms = archive
        .duration_seconds
        .checked_mul(1_000)
        .ok_or_else(|| bilibili_data_error("Bilibili season archive duration overflowed"))?;
    let mut extensions = season_archive_extensions(&archive, season_id, owner_id, &resource_ref);
    extensions.insert("normalized_from_video".to_owned(), json!(true));
    let id = resource_ref.id().to_owned();
    Ok(Track {
        resource_ref,
        platform: Platform::Bilibili,
        id,
        name: archive.title,
        aliases: Vec::new(),
        artists: vec![ArtistSummary {
            resource_ref: Some(owner_ref),
            name: owner_id.to_string(),
        }],
        album: None,
        duration_ms: Some(duration_ms),
        isrc: None,
        mv_ref: None,
        playable: Some(archive.state == 0),
        available_qualities: Vec::new(),
        extensions,
    })
}

fn map_season_archive_video(
    archive: BilibiliSeasonArchive,
    season_id: u64,
    owner_id: u64,
) -> Result<VideoDetail> {
    let resource_ref = bilibili_archive_ref(&archive.bvid)?;
    let owner_ref = bilibili_archive_owner_ref(owner_id)?;
    let duration_ms = archive
        .duration_seconds
        .checked_mul(1_000)
        .ok_or_else(|| bilibili_data_error("Bilibili season archive duration overflowed"))?;
    let extensions = season_archive_extensions(&archive, season_id, owner_id, &resource_ref);
    let id = resource_ref.id().to_owned();
    let video = Video {
        resource_ref,
        platform: Platform::Bilibili,
        id,
        title: archive.title,
        creators: vec![CreatorSummary {
            resource_ref: Some(owner_ref),
            name: owner_id.to_string(),
            avatar_url: None,
        }],
        description: String::new(),
        cover_url: Some(archive.cover_url),
        duration_ms: Some(duration_ms),
        published_at: (archive.published_at > 0)
            .then(|| bilibili_unix_rfc3339(archive.published_at))
            .flatten(),
        play_count: Some(archive.view_count),
        subscribed: None,
        extensions,
    };
    Ok(VideoDetail {
        kind: VideoResourceKind::Video,
        video,
        resolutions: Vec::new(),
        extensions: Extensions::from([
            ("detail_source".to_owned(), json!("season_archive")),
            ("summary_only".to_owned(), json!(true)),
            ("resolutions_resolved".to_owned(), json!(false)),
        ]),
    })
}

fn map_favorite_media_track(media: BilibiliFavoriteMedia, media_id: u64) -> Result<Track> {
    let resource_ref = favorite_media_ref(&media)?;
    let duration_ms = media
        .duration_seconds
        .checked_mul(1_000)
        .ok_or_else(|| bilibili_data_error("Bilibili favorite media duration overflowed"))?;
    let artists = media
        .owner
        .as_ref()
        .map(favorite_media_artist)
        .transpose()?
        .into_iter()
        .collect();
    let mut extensions = favorite_media_extensions(&media, media_id, &resource_ref);
    extensions.insert("normalized_from_video".to_owned(), json!(true));
    let id = resource_ref.id().to_owned();
    Ok(Track {
        resource_ref,
        platform: Platform::Bilibili,
        id,
        name: media.title,
        aliases: Vec::new(),
        artists,
        album: None,
        duration_ms: Some(duration_ms),
        isrc: None,
        mv_ref: None,
        playable: Some(!media.invalid),
        available_qualities: Vec::new(),
        extensions,
    })
}

fn map_favorite_media_video(media: BilibiliFavoriteMedia, media_id: u64) -> Result<VideoDetail> {
    let resource_ref = favorite_media_ref(&media)?;
    let duration_ms = media
        .duration_seconds
        .checked_mul(1_000)
        .ok_or_else(|| bilibili_data_error("Bilibili favorite media duration overflowed"))?;
    let creators = media
        .owner
        .as_ref()
        .map(favorite_media_creator)
        .transpose()?
        .into_iter()
        .collect();
    let extensions = favorite_media_extensions(&media, media_id, &resource_ref);
    let id = resource_ref.id().to_owned();
    let video = Video {
        resource_ref,
        platform: Platform::Bilibili,
        id,
        title: media.title,
        creators,
        description: media.description,
        cover_url: media.cover_url,
        duration_ms: Some(duration_ms),
        published_at: (media.published_at > 0)
            .then(|| bilibili_unix_rfc3339(media.published_at))
            .flatten(),
        play_count: Some(media.play_count),
        subscribed: None,
        extensions,
    };
    Ok(VideoDetail {
        kind: VideoResourceKind::Video,
        video,
        resolutions: Vec::new(),
        extensions: Extensions::from([
            ("detail_source".to_owned(), json!("favorite_media")),
            ("summary_only".to_owned(), json!(true)),
            ("resolutions_resolved".to_owned(), json!(false)),
        ]),
    })
}

fn favorite_media_ref(media: &BilibiliFavoriteMedia) -> Result<ResourceRef> {
    match &media.bvid {
        Some(bvid) => BilibiliVideoIdentity::Bvid(bvid.clone()).resource_ref(),
        None => BilibiliVideoIdentity::Aid(media.aid).resource_ref(),
    }
    .map_err(|_| bilibili_data_error("Bilibili favorite media identity was invalid"))
}

fn favorite_media_artist(
    owner: &crate::client::BilibiliCollectedPlaylistOwner,
) -> Result<ArtistSummary> {
    let resource_ref = ResourceRef::new(Platform::Bilibili, format!("user:{}", owner.id))
        .map_err(|_| bilibili_data_error("Bilibili favorite media owner was invalid"))?;
    Ok(ArtistSummary {
        resource_ref: Some(resource_ref),
        name: owner.name.clone(),
    })
}

fn favorite_media_creator(
    owner: &crate::client::BilibiliCollectedPlaylistOwner,
) -> Result<CreatorSummary> {
    let artist = favorite_media_artist(owner)?;
    Ok(CreatorSummary {
        resource_ref: artist.resource_ref,
        name: artist.name,
        avatar_url: owner.avatar_url.clone(),
    })
}

fn favorite_media_extensions(
    media: &BilibiliFavoriteMedia,
    media_id: u64,
    video_ref: &ResourceRef,
) -> Extensions {
    let mut extensions = Extensions::from([
        ("video_ref".to_owned(), json!(video_ref)),
        (
            "bilibili_playlist_kind".to_owned(),
            json!("favorite_folder"),
        ),
        ("media_id".to_owned(), json!(media_id)),
        ("aid".to_owned(), json!(media.aid)),
        ("invalid".to_owned(), json!(media.invalid)),
        ("part_count".to_owned(), json!(media.part_count)),
        ("collect_count".to_owned(), json!(media.collect_count)),
        ("play_count".to_owned(), json!(media.play_count)),
        ("danmaku_count".to_owned(), json!(media.danmaku_count)),
    ]);
    insert_optional(&mut extensions, "bvid", media.bvid.clone());
    if media.created_at > 0 {
        extensions.insert("ctime".to_owned(), json!(media.created_at));
    }
    if media.published_at > 0 {
        extensions.insert("pubtime".to_owned(), json!(media.published_at));
    }
    if media.favorited_at > 0 {
        extensions.insert("fav_time".to_owned(), json!(media.favorited_at));
    }
    extensions
}

fn bilibili_archive_ref(bvid: &str) -> Result<ResourceRef> {
    BilibiliVideoIdentity::Bvid(bvid.to_owned())
        .resource_ref()
        .map_err(|_| bilibili_data_error("Bilibili season archive identity was invalid"))
}

fn bilibili_archive_owner_ref(owner_id: u64) -> Result<ResourceRef> {
    ResourceRef::new(Platform::Bilibili, format!("user:{owner_id}"))
        .map_err(|_| bilibili_data_error("Bilibili season archive owner was invalid"))
}

fn season_archive_extensions(
    archive: &BilibiliSeasonArchive,
    season_id: u64,
    owner_id: u64,
    video_ref: &ResourceRef,
) -> Extensions {
    let mut extensions = Extensions::from([
        ("video_ref".to_owned(), json!(video_ref)),
        ("bilibili_playlist_kind".to_owned(), json!("season")),
        ("season_id".to_owned(), json!(season_id)),
        ("owner_mid".to_owned(), json!(owner_id)),
        ("creator_name_unavailable".to_owned(), json!(true)),
        ("aid".to_owned(), json!(archive.aid)),
        ("bvid".to_owned(), json!(archive.bvid)),
        ("interactive".to_owned(), json!(archive.interactive)),
        ("state".to_owned(), json!(archive.state)),
        ("paid".to_owned(), json!(archive.paid)),
        ("view_count".to_owned(), json!(archive.view_count)),
    ]);
    insert_optional(
        &mut extensions,
        "playback_position",
        archive.playback_position,
    );
    insert_optional(&mut extensions, "danmaku_count", archive.danmaku_count);
    if archive.created_at > 0 {
        extensions.insert("ctime".to_owned(), json!(archive.created_at));
    }
    if archive.published_at > 0 {
        extensions.insert("pubdate".to_owned(), json!(archive.published_at));
    }
    extensions
}

#[cfg(test)]
fn map_created_favorite_folder_page(
    response: BilibiliCreatedFavoriteFolders,
    offset: u32,
    limit: u32,
) -> Result<Page<Playlist>> {
    let total = u64::try_from(response.folders.len())
        .map_err(|_| bilibili_data_error("Bilibili favorite folder total overflowed"))?;
    let from = usize::try_from(offset)
        .unwrap_or(usize::MAX)
        .min(response.folders.len());
    let to = from
        .saturating_add(usize::try_from(limit).unwrap_or(usize::MAX))
        .min(response.folders.len());
    let items = response.folders[from..to]
        .iter()
        .cloned()
        .map(map_created_favorite_folder)
        .collect::<Result<Vec<_>>>()?;
    let returned = u32::try_from(items.len())
        .map_err(|_| bilibili_data_error("Bilibili favorite folder page overflowed"))?;
    let next_offset = offset
        .checked_add(returned)
        .ok_or_else(|| bilibili_data_error("Bilibili favorite folder offset overflowed"))?;
    let has_more = returned > 0 && u64::from(next_offset) < total;
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total: Some(total),
            next_offset: has_more.then_some(next_offset),
            has_more,
            extensions: Extensions::from([
                ("library_scope".to_owned(), json!("user_created")),
                ("user_mid".to_owned(), json!(response.owner_id)),
                ("upstream_count".to_owned(), json!(total)),
            ]),
        },
    })
}

fn map_created_favorite_folder(folder: BilibiliCreatedFavoriteFolder) -> Result<Playlist> {
    let resource_ref =
        ResourceRef::new(Platform::Bilibili, format!("favorite:{}", folder.media_id))
            .map_err(|_| bilibili_data_error("Bilibili favorite folder identity was invalid"))?;
    let is_private = folder.attributes & 1 != 0;
    let is_default = folder.attributes & 2 == 0;
    Ok(Playlist {
        resource_ref,
        platform: Platform::Bilibili,
        id: format!("favorite:{}", folder.media_id),
        name: folder.title,
        description: String::new(),
        cover_url: None,
        creator: None,
        track_count: Some(folder.media_count),
        tags: Vec::new(),
        subscribed: Some(false),
        created_at: None,
        updated_at: None,
        extensions: Extensions::from([
            ("source".to_owned(), json!("created")),
            ("media_id".to_owned(), json!(folder.media_id)),
            ("fid".to_owned(), json!(folder.folder_id)),
            ("owner_mid".to_owned(), json!(folder.owner_id)),
            (
                "owner_ref".to_owned(),
                json!(format!("bilibili:user:{}", folder.owner_id)),
            ),
            ("attr".to_owned(), json!(folder.attributes)),
            ("private".to_owned(), json!(is_private)),
            ("default".to_owned(), json!(is_default)),
            ("fav_state".to_owned(), json!(folder.favorite_state)),
            ("media_type".to_owned(), json!("video")),
            ("child_friendly".to_owned(), json!(folder.child_friendly)),
            (
                "child_friendly_description".to_owned(),
                json!(folder.child_friendly_description),
            ),
        ]),
    })
}

fn map_favorite_folder(folder: BilibiliFavoriteFolder) -> Result<Playlist> {
    let id = format!("favorite:{}", folder.media_id);
    let resource_ref = ResourceRef::new(Platform::Bilibili, &id)
        .map_err(|_| bilibili_data_error("Bilibili favorite folder identity was invalid"))?;
    let owner_ref = ResourceRef::new(Platform::Bilibili, format!("user:{}", folder.owner.id))
        .map_err(|_| bilibili_data_error("Bilibili favorite folder owner identity was invalid"))?;
    let creator = ArtistSummary {
        resource_ref: Some(owner_ref.clone()),
        name: folder.owner.name,
    };
    let mut extensions = Extensions::from([
        ("source".to_owned(), json!("detail")),
        ("collection_kind".to_owned(), json!("favorite_folder")),
        ("media_id".to_owned(), json!(folder.media_id)),
        ("fid".to_owned(), json!(folder.folder_id)),
        ("owner_mid".to_owned(), json!(folder.owner.id)),
        ("owner_ref".to_owned(), json!(owner_ref)),
        ("owner_followed".to_owned(), json!(folder.owner.followed)),
        ("owner_vip_type".to_owned(), json!(folder.owner.vip_type)),
        (
            "owner_vip_status".to_owned(),
            json!(folder.owner.vip_status),
        ),
        ("attr".to_owned(), json!(folder.attributes)),
        ("private".to_owned(), json!(folder.attributes & 1 != 0)),
        ("default".to_owned(), json!(folder.attributes & 2 == 0)),
        ("cover_type".to_owned(), json!(folder.cover_type)),
        ("invalid".to_owned(), json!(folder.invalid)),
        ("fav_state".to_owned(), json!(folder.favorite_state)),
        ("like_state".to_owned(), json!(folder.like_state)),
        ("pinned".to_owned(), json!(folder.pinned)),
        ("collect_count".to_owned(), json!(folder.counts.collect)),
        ("play_count".to_owned(), json!(folder.counts.play)),
        ("thumb_up_count".to_owned(), json!(folder.counts.thumb_up)),
        ("share_count".to_owned(), json!(folder.counts.share)),
        ("media_type".to_owned(), json!("video")),
        ("child_friendly".to_owned(), json!(folder.child_friendly)),
        (
            "child_friendly_description".to_owned(),
            json!(folder.child_friendly_description),
        ),
    ]);
    insert_optional(&mut extensions, "owner_avatar_url", folder.owner.avatar_url);
    Ok(Playlist {
        resource_ref,
        platform: Platform::Bilibili,
        id,
        name: folder.title,
        description: folder.description,
        cover_url: folder.cover_url,
        creator: Some(creator),
        track_count: Some(folder.media_count),
        tags: Vec::new(),
        subscribed: Some(folder.favorite_state),
        created_at: (folder.created_at > 0)
            .then(|| bilibili_unix_rfc3339(folder.created_at))
            .flatten(),
        updated_at: (folder.updated_at > 0)
            .then(|| bilibili_unix_rfc3339(folder.updated_at))
            .flatten(),
        extensions,
    })
}

fn validate_collected_playlist_page(
    page: &BilibiliCollectedPlaylistPage,
    requested_page: u32,
    expected_total: Option<u64>,
) -> Result<()> {
    if page.page != requested_page
        || page.page_size != 70
        || page.playlists.len() > page.page_size as usize
        || expected_total.is_some_and(|total| total != page.total)
    {
        return Err(bilibili_data_error(
            "Bilibili collected playlist pagination changed during traversal",
        ));
    }
    Ok(())
}

fn map_collected_playlist(playlist: BilibiliCollectedPlaylist) -> Result<Playlist> {
    let (kind, id, resource_ref) = match playlist.kind {
        BilibiliCollectedPlaylistKind::FavoriteFolder => {
            let id = format!("favorite:{}", playlist.id);
            let resource_ref = ResourceRef::new(Platform::Bilibili, &id).map_err(|_| {
                bilibili_data_error("Bilibili collected favorite folder identity was invalid")
            })?;
            ("favorite_folder", id, resource_ref)
        }
        BilibiliCollectedPlaylistKind::Season => {
            let id = format!("season:{}", playlist.id);
            let resource_ref = ResourceRef::new(Platform::Bilibili, &id).map_err(|_| {
                bilibili_data_error("Bilibili collected season identity was invalid")
            })?;
            ("season", id, resource_ref)
        }
    };
    let mut extensions = Extensions::from([
        ("source".to_owned(), json!("favorite")),
        ("collection_kind".to_owned(), json!(kind)),
        ("upstream_id".to_owned(), json!(playlist.id)),
        ("attr".to_owned(), json!(playlist.attributes)),
        (
            "attr_description".to_owned(),
            json!(playlist.attribute_description),
        ),
        ("private".to_owned(), json!(playlist.attributes & 1 != 0)),
        ("default".to_owned(), json!(playlist.attributes & 2 == 0)),
        ("cover_type".to_owned(), json!(playlist.cover_type)),
        ("invalid".to_owned(), json!(playlist.invalid)),
        ("fav_state".to_owned(), json!(playlist.favorite_state)),
        ("media_type".to_owned(), json!("video")),
        ("child_friendly".to_owned(), json!(playlist.child_friendly)),
        (
            "child_friendly_description".to_owned(),
            json!(playlist.child_friendly_description),
        ),
    ]);
    insert_optional(&mut extensions, "fid", playlist.folder_id);
    insert_optional(&mut extensions, "view_count", playlist.view_count);
    insert_optional(&mut extensions, "pinned", playlist.pinned);
    insert_optional(&mut extensions, "deep_link", playlist.deep_link);
    insert_optional(&mut extensions, "bvid", playlist.bvid);
    let creator = playlist
        .owner
        .map(|owner| {
            let resource_ref = ResourceRef::new(Platform::Bilibili, format!("user:{}", owner.id))
                .map_err(|_| {
                bilibili_data_error("Bilibili collected playlist owner was invalid")
            })?;
            extensions.insert("owner_mid".to_owned(), json!(owner.id));
            extensions.insert("owner_ref".to_owned(), json!(resource_ref));
            insert_optional(&mut extensions, "owner_avatar_url", owner.avatar_url);
            Ok::<_, TuneWeaveError>(ArtistSummary {
                resource_ref: Some(resource_ref),
                name: owner.name,
            })
        })
        .transpose()?;
    Ok(Playlist {
        resource_ref,
        platform: Platform::Bilibili,
        id,
        name: playlist.title,
        description: playlist.description,
        cover_url: playlist.cover_url,
        creator,
        track_count: Some(playlist.media_count),
        tags: Vec::new(),
        subscribed: Some(playlist.favorite_state),
        created_at: (playlist.created_at > 0)
            .then(|| bilibili_unix_rfc3339(playlist.created_at))
            .flatten(),
        updated_at: (playlist.updated_at > 0)
            .then(|| bilibili_unix_rfc3339(playlist.updated_at))
            .flatten(),
        extensions,
    })
}

fn validate_space_playlist_page(
    page: &BilibiliSpacePlaylistPage,
    requested_page: u32,
    expected_total: Option<u64>,
) -> Result<()> {
    if page.page != requested_page
        || page.page_size != 20
        || page.playlists.len() > page.page_size as usize
        || expected_total.is_some_and(|total| total != page.total)
    {
        return Err(bilibili_data_error(
            "Bilibili space playlist pagination changed during traversal",
        ));
    }
    Ok(())
}

fn map_space_playlist(playlist: BilibiliSpacePlaylist) -> Result<Playlist> {
    let (kind, id) = match playlist.kind {
        BilibiliSpacePlaylistKind::Season => ("season", format!("season:{}", playlist.id)),
        BilibiliSpacePlaylistKind::Series => ("series", format!("series:{}", playlist.id)),
    };
    let resource_ref = ResourceRef::new(Platform::Bilibili, &id)
        .map_err(|_| bilibili_data_error("Bilibili space playlist identity was invalid"))?;
    let owner_ref = ResourceRef::new(Platform::Bilibili, format!("user:{}", playlist.owner_id))
        .map_err(|_| bilibili_data_error("Bilibili space playlist owner was invalid"))?;
    let mut extensions = Extensions::from([
        ("source".to_owned(), json!("created")),
        ("collection_kind".to_owned(), json!(kind)),
        ("upstream_id".to_owned(), json!(playlist.id)),
        ("owner_mid".to_owned(), json!(playlist.owner_id)),
        ("owner_ref".to_owned(), json!(owner_ref)),
        ("category".to_owned(), json!(playlist.category)),
        ("recent_aids".to_owned(), json!(playlist.recent_aids)),
        ("preview_aids".to_owned(), json!(playlist.preview_aids)),
        ("media_type".to_owned(), json!("video")),
    ]);
    insert_optional(&mut extensions, "display_title", playlist.display_title);
    insert_optional(&mut extensions, "state", playlist.state);
    insert_optional(&mut extensions, "creator_mode", playlist.creator_mode);
    if playlist.published_at > 0 {
        extensions.insert("ptime".to_owned(), json!(playlist.published_at));
        if let Some(timestamp) = bilibili_unix_rfc3339(playlist.published_at) {
            extensions.insert("published_at".to_owned(), json!(timestamp));
        }
    }
    let created_at = [playlist.created_at, playlist.published_at]
        .into_iter()
        .find(|timestamp| *timestamp > 0)
        .and_then(bilibili_unix_rfc3339);
    let updated_at = (playlist.updated_at > 0)
        .then(|| bilibili_unix_rfc3339(playlist.updated_at))
        .flatten();
    Ok(Playlist {
        resource_ref,
        platform: Platform::Bilibili,
        id,
        name: playlist.name,
        description: playlist.description,
        cover_url: playlist.cover_url,
        creator: None,
        track_count: Some(playlist.track_count),
        tags: playlist.keywords,
        subscribed: Some(false),
        created_at,
        updated_at,
        extensions,
    })
}

fn insert_optional<T: serde::Serialize>(extensions: &mut Extensions, key: &str, value: Option<T>) {
    if let Some(value) = value
        && let Ok(value) = serde_json::to_value(value)
    {
        extensions.insert(key.to_owned(), value);
    }
}

const fn capability_for_search(kind: SearchKind) -> Capability {
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

fn bilibili_unix_rfc3339(timestamp: u64) -> Option<String> {
    let days = i64::try_from(timestamp / 86_400).ok()?;
    let seconds = timestamp % 86_400;
    let z = days.checked_add(719_468)?;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    if !(0..=9_999).contains(&year) {
        return None;
    }
    let hour = seconds / 3_600;
    let minute = (seconds % 3_600) / 60;
    let second = seconds % 60;
    Some(format!(
        "{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z"
    ))
}

fn bilibili_invalid_request(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Bilibili)
}

fn bilibili_data_error(message: impl Into<String>) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::UpstreamError, message)
        .with_platform(Platform::Bilibili)
        .retryable(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct RecordingCredentialStore {
        credentials: Mutex<Vec<StoredAccountCredential>>,
    }

    impl AccountCredentialStore for RecordingCredentialStore {
        fn load_platform(&self, platform: Platform) -> Result<Vec<StoredAccountCredential>> {
            Ok(self
                .credentials
                .lock()
                .expect("credential store lock")
                .iter()
                .filter(|credential| credential.platform == platform)
                .cloned()
                .collect())
        }

        fn put(&self, credential: &StoredAccountCredential) -> Result<()> {
            let mut credentials = self.credentials.lock().expect("credential store lock");
            credentials.retain(|stored| {
                stored.platform != credential.platform || stored.account != credential.account
            });
            credentials.push(credential.clone());
            Ok(())
        }

        fn remove(&self, platform: Platform, account: &str) -> Result<bool> {
            let mut credentials = self.credentials.lock().expect("credential store lock");
            let before = credentials.len();
            credentials.retain(|stored| stored.platform != platform || stored.account != account);
            Ok(before != credentials.len())
        }
    }

    fn sample_credential() -> BilibiliCredential {
        BilibiliCredential {
            dede_user_id: "47275982".to_owned(),
            dede_user_id_ck_md5: "0123456789abcdef".to_owned(),
            sessdata: "private%2Csession".to_owned(),
            bili_jct: "0123456789abcdef0123456789abcdef".to_owned(),
            sid: Some("private-sid".to_owned()),
            refresh_token: "private-refresh".to_owned(),
        }
        .normalize()
        .expect("sample credential")
    }

    fn caller_material(credential: &BilibiliCredential) -> ProviderCredential {
        ProviderCredential::new(
            Platform::Bilibili,
            BILIBILI_CREDENTIAL_KIND,
            serde_json::to_string(credential).expect("credential JSON"),
            None,
        )
        .expect("caller credential")
    }

    #[test]
    fn qr_login_supports_server_client_and_both_credential_ownership() {
        let credential = sample_credential();
        let client_only = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let client_result = client_only
            .finish_authentication("default", &credential, CredentialMode::Client)
            .expect("client-owned login");
        assert_eq!(client_result.profile.user_id.as_deref(), Some("47275982"));
        let caller = client_result.credential.expect("caller credential");
        assert_eq!(caller.kind, BILIBILI_CREDENTIAL_KIND);
        assert_eq!(
            client_only
                .finish_authentication("named", &credential, CredentialMode::Client)
                .expect_err("client mode rejects aliases")
                .code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            client_only
                .finish_authentication("default", &credential, CredentialMode::Server)
                .expect_err("server mode requires storage")
                .code,
            ErrorCode::InternalError
        );

        let store = Arc::new(RecordingCredentialStore::default());
        let both = BilibiliProvider::new(BilibiliConfig {
            credential_store: Some(store.clone()),
            ..BilibiliConfig::default()
        })
        .expect("provider with storage");
        let both_result = both
            .finish_authentication("personal", &credential, CredentialMode::Both)
            .expect("both-owned login");
        let caller = both_result.credential.expect("both caller credential");
        let stored = store
            .load_platform(Platform::Bilibili)
            .expect("stored credentials");
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].account, "personal");
        assert_eq!(stored[0].secret(), caller.secret());
        assert!(!format!("{both:?}").contains("private"));
    }

    #[test]
    fn caller_credentials_are_strongly_validated_and_isolated_from_aliases() {
        let credential = sample_credential();
        let secret = serde_json::to_string(&credential).expect("credential JSON");
        let material =
            ProviderCredential::new(Platform::Bilibili, BILIBILI_CREDENTIAL_KIND, secret, None)
                .expect("provider credential");
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let scoped = provider
            .caller_credential_scope(&material)
            .expect("caller scope");
        assert_eq!(
            scoped
                .selected_credential("default")
                .expect("caller credential"),
            credential
        );
        assert_eq!(
            scoped
                .selected_credential("server-alias")
                .expect_err("caller scope must isolate aliases")
                .code,
            ErrorCode::AuthenticationRequired
        );

        for invalid in [
            ProviderCredential::new(Platform::Qq, BILIBILI_CREDENTIAL_KIND, "{}", None)
                .expect("wrong platform material"),
            ProviderCredential::new(Platform::Bilibili, "cookie", "{}", None)
                .expect("wrong kind material"),
            ProviderCredential::new(Platform::Bilibili, BILIBILI_CREDENTIAL_KIND, "{}", Some(1))
                .expect("wrong expiry material"),
        ] {
            let error = provider
                .caller_credential_scope(&invalid)
                .expect_err("invalid caller credential must fail");
            assert_eq!(error.code, ErrorCode::InvalidRequest);
        }
    }

    #[tokio::test]
    async fn missing_account_session_is_anonymous_without_network_access() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let profile = provider
            .session_profile("missing")
            .await
            .expect("missing account profile");
        assert!(!profile.authenticated);
        assert_eq!(profile.account, "missing");
        assert!(profile.user_id.is_none());
        let capabilities = provider.capabilities();
        assert!(capabilities.contains(&Capability::AccountProfile));
        assert!(capabilities.contains(&Capability::SessionManagement));
    }

    #[test]
    fn session_management_sources_enforce_ownership_and_identity() {
        let store = Arc::new(RecordingCredentialStore::default());
        let provider = BilibiliProvider::new(BilibiliConfig {
            credential_store: Some(store.clone()),
            ..BilibiliConfig::default()
        })
        .expect("provider");
        let credential = sample_credential();
        provider
            .finish_authentication("personal", &credential, CredentialMode::Server)
            .expect("store credential");

        assert_eq!(
            bilibili_refresh_source(&provider, "personal", None, CredentialMode::Server)
                .expect("server refresh source"),
            credential
        );
        assert_eq!(
            bilibili_logout_source(&provider, "missing", None, CredentialMode::Server)
                .expect("missing server logout source"),
            None
        );
        assert_eq!(
            bilibili_refresh_source(&provider, "default", None, CredentialMode::Client)
                .expect_err("client refresh requires a credential")
                .code,
            ErrorCode::InvalidRequest
        );

        let caller = caller_material(&credential);
        assert_eq!(
            bilibili_refresh_source(&provider, "personal", Some(&caller), CredentialMode::Both,)
                .expect("matching both refresh source"),
            credential
        );
        assert_eq!(
            bilibili_logout_source(&provider, "personal", Some(&caller), CredentialMode::Server,)
                .expect_err("server mode rejects caller logout")
                .code,
            ErrorCode::InvalidRequest
        );

        let mut other = credential.clone();
        other.dede_user_id = "999".to_owned();
        let other = caller_material(&other);
        let mismatch =
            bilibili_refresh_source(&provider, "personal", Some(&other), CredentialMode::Both)
                .expect_err("both mode rejects a different identity");
        assert_eq!(mismatch.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn session_mapping_keeps_profile_summary_and_typed_extensions() {
        let credential = sample_credential();
        let status = BilibiliSessionStatus {
            authenticated: true,
            user_id: Some("47275982".to_owned()),
            nickname: Some("Lotus".to_owned()),
            avatar_url: Some("https://i0.hdslb.com/bfs/face/avatar.jpg".to_owned()),
            extensions: std::collections::BTreeMap::from([
                ("platform_code".to_owned(), json!(0)),
                ("nav".to_owned(), json!({ "vip_status": 1 })),
            ]),
        };
        let profile = map_bilibili_session_profile("personal", &credential, status);
        assert!(profile.authenticated);
        assert_eq!(profile.user_id.as_deref(), Some("47275982"));
        assert_eq!(profile.nickname.as_deref(), Some("Lotus"));
        assert_eq!(profile.extensions["nav"]["vip_status"], 1);
    }

    #[test]
    fn video_search_mapping_uses_stable_video_and_creator_references() {
        let video = map_bilibili_search_video(BilibiliSearchVideo {
            aid: 78_977_417,
            bvid: Some("BV1KJ411C7Un".to_owned()),
            title: "初音未来".to_owned(),
            author: "MitchieM".to_owned(),
            author_id: 5_669_526,
            description: "音乐视频".to_owned(),
            cover_url: "https://i1.hdslb.com/bfs/archive/cover.jpg".to_owned(),
            duration_seconds: 242,
            duration_text: "4:02".to_owned(),
            play_count: Some(2_915_520),
            danmaku_count: Some(14_572),
            favorite_count: Some(114_102),
            comment_count: Some(6_124),
            published_at: Some(1_579_877_678),
            sent_at: Some(1_593_099_008),
            category_id: Some("30".to_owned()),
            category_name: Some("VOCALOID·UTAU".to_owned()),
            tags: vec!["音乐".to_owned()],
            hit_columns: vec!["title".to_owned()],
            paid: Some(false),
            collaborative: Some(true),
            rank_score: Some(109_020_056),
        })
        .expect("mapped video");
        assert_eq!(video.resource_ref.to_string(), "bilibili:bvid:BV1KJ411C7Un");
        assert_eq!(video.id, "bvid:BV1KJ411C7Un");
        assert_eq!(video.duration_ms, Some(242_000));
        assert_eq!(video.published_at.as_deref(), Some("2020-01-24T14:54:38Z"));
        assert_eq!(
            video.creators[0]
                .resource_ref
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("bilibili:user:5669526")
        );
        assert_eq!(video.extensions["collaborative"], true);
    }

    #[test]
    fn video_search_filters_map_every_unified_branch_and_validate_categories() {
        let filters = map_bilibili_video_search_filters(Some(&VideoSearchFilters {
            order: VideoSearchOrder::MostCommented,
            duration: VideoSearchDuration::OverSixtyMinutes,
            category_id: Some("193".to_owned()),
        }))
        .expect("mapped filters");
        assert_eq!(filters.order, BilibiliVideoSearchOrder::MostCommented);
        assert_eq!(
            filters.duration,
            BilibiliVideoSearchDuration::OverSixtyMinutes
        );
        assert_eq!(filters.category_id, Some(193));

        let defaults = map_bilibili_video_search_filters(None).expect("default filters");
        assert_eq!(defaults, BilibiliVideoSearchFilters::default());

        let invalid = map_bilibili_video_search_filters(Some(&VideoSearchFilters {
            category_id: Some("music".to_owned()),
            ..VideoSearchFilters::default()
        }))
        .expect_err("invalid category");
        assert_eq!(invalid.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn created_favorite_folder_mapping_preserves_ids_attributes_and_pagination() {
        let page = map_created_favorite_folder_page(
            BilibiliCreatedFavoriteFolders {
                owner_id: 7_792_521,
                folders: vec![
                    BilibiliCreatedFavoriteFolder {
                        media_id: 44_233_921,
                        folder_id: 442_339,
                        owner_id: 7_792_521,
                        attributes: 0,
                        title: "默认收藏夹".to_owned(),
                        favorite_state: false,
                        media_count: 178,
                        child_friendly: false,
                        child_friendly_description: String::new(),
                    },
                    BilibiliCreatedFavoriteFolder {
                        media_id: 90_210_021,
                        folder_id: 902_100,
                        owner_id: 7_792_521,
                        attributes: 3,
                        title: "私有音乐".to_owned(),
                        favorite_state: true,
                        media_count: 12,
                        child_friendly: true,
                        child_friendly_description: "适合青少年".to_owned(),
                    },
                ],
            },
            1,
            1,
        )
        .expect("mapped favorite folder page");
        assert_eq!(page.items.len(), 1);
        assert_eq!(
            page.items[0].resource_ref.to_string(),
            "bilibili:favorite:90210021"
        );
        assert_eq!(page.items[0].id, "favorite:90210021");
        assert_eq!(page.items[0].track_count, Some(12));
        assert_eq!(page.items[0].subscribed, Some(false));
        assert_eq!(page.items[0].extensions["fid"], 902_100);
        assert_eq!(page.items[0].extensions["owner_mid"], 7_792_521);
        assert_eq!(
            page.items[0].extensions["owner_ref"],
            "bilibili:user:7792521"
        );
        assert_eq!(page.items[0].extensions["private"], true);
        assert_eq!(page.items[0].extensions["default"], false);
        assert_eq!(page.pagination.total, Some(2));
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.extensions["library_scope"], "user_created");
    }

    #[test]
    fn collected_playlist_mapping_preserves_kind_owner_state_and_metadata() {
        let playlist = map_collected_playlist(BilibiliCollectedPlaylist {
            kind: BilibiliCollectedPlaylistKind::Season,
            id: 4_641_954,
            folder_id: None,
            owner: Some(crate::client::BilibiliCollectedPlaylistOwner {
                id: 1_868_902_080,
                name: "哔哩哔哩拜年纪".to_owned(),
                avatar_url: Some("https://i0.hdslb.com/bfs/face/avatar.jpg".to_owned()),
            }),
            attributes: 0,
            attribute_description: String::new(),
            title: "2025哔哩哔哩拜年纪".to_owned(),
            cover_url: Some("https://archive.biliimg.com/bfs/archive/season.jpg".to_owned()),
            description: "视频合集".to_owned(),
            cover_type: 0,
            created_at: 0,
            updated_at: 1_738_078_200,
            invalid: false,
            favorite_state: true,
            media_count: 46,
            view_count: Some(74_688_312),
            pinned: Some(false),
            deep_link: Some("bilibili://video/113884295860962?is_from_ugc_season=1".to_owned()),
            bvid: None,
            child_friendly: false,
            child_friendly_description: String::new(),
        })
        .expect("mapped collected season");
        assert_eq!(playlist.resource_ref.to_string(), "bilibili:season:4641954");
        assert_eq!(playlist.id, "season:4641954");
        assert_eq!(playlist.subscribed, Some(true));
        assert_eq!(playlist.track_count, Some(46));
        assert_eq!(
            playlist
                .creator
                .as_ref()
                .and_then(|creator| creator.resource_ref.as_ref())
                .map(ToString::to_string)
                .as_deref(),
            Some("bilibili:user:1868902080")
        );
        assert_eq!(playlist.updated_at.as_deref(), Some("2025-01-28T15:30:00Z"));
        assert_eq!(playlist.extensions["collection_kind"], "season");
        assert_eq!(playlist.extensions["view_count"], 74_688_312);
        assert_eq!(
            playlist.extensions["owner_avatar_url"],
            "https://i0.hdslb.com/bfs/face/avatar.jpg"
        );

        let favorite = map_collected_playlist(BilibiliCollectedPlaylist {
            kind: BilibiliCollectedPlaylistKind::FavoriteFolder,
            id: 49_630_708,
            folder_id: Some(496_307),
            owner: None,
            attributes: 22,
            attribute_description: String::new(),
            title: "失效收藏夹".to_owned(),
            cover_url: None,
            description: String::new(),
            cover_type: 2,
            created_at: 0,
            updated_at: 0,
            invalid: true,
            favorite_state: false,
            media_count: 0,
            view_count: None,
            pinned: None,
            deep_link: None,
            bvid: None,
            child_friendly: false,
            child_friendly_description: String::new(),
        })
        .expect("mapped collected favorite folder");
        assert_eq!(
            favorite.resource_ref.to_string(),
            "bilibili:favorite:49630708"
        );
        assert_eq!(favorite.extensions["fid"], 496_307);
        assert_eq!(favorite.extensions["invalid"], true);
    }

    #[test]
    fn space_playlist_mapping_preserves_season_and_series_identity() {
        let season = map_space_playlist(BilibiliSpacePlaylist {
            kind: BilibiliSpacePlaylistKind::Season,
            id: 587_216,
            owner_id: 37_737_161,
            name: "合集·拾枝杂谈".to_owned(),
            display_title: Some("拾枝杂谈".to_owned()),
            description: "公开视频合集".to_owned(),
            cover_url: Some("https://archive.biliimg.com/bfs/archive/season.jpg".to_owned()),
            category: 0,
            track_count: 10,
            created_at: 0,
            published_at: 1_694_682_652,
            updated_at: 0,
            state: None,
            creator_mode: None,
            keywords: Vec::new(),
            recent_aids: vec![343_807_541],
            preview_aids: vec![343_807_541],
        })
        .expect("mapped space season");
        assert_eq!(season.resource_ref.to_string(), "bilibili:season:587216");
        assert_eq!(season.id, "season:587216");
        assert_eq!(season.subscribed, Some(false));
        assert_eq!(season.extensions["owner_ref"], "bilibili:user:37737161");
        assert_eq!(season.extensions["collection_kind"], "season");
        assert_eq!(season.extensions["display_title"], "拾枝杂谈");
        assert_eq!(season.extensions["recent_aids"][0], 343_807_541);

        let series = map_space_playlist(BilibiliSpacePlaylist {
            kind: BilibiliSpacePlaylistKind::Series,
            id: 3_908_327,
            owner_id: 37_737_161,
            name: "Kotlin开心路线".to_owned(),
            display_title: None,
            description: "Kotlin 学习路线".to_owned(),
            cover_url: Some("https://i0.hdslb.com/bfs/archive/series.jpg".to_owned()),
            category: 1,
            track_count: 3,
            created_at: 1_705_401_630,
            published_at: 0,
            updated_at: 1_705_925_782,
            state: Some(2),
            creator_mode: Some("auto".to_owned()),
            keywords: vec!["Kotlin".to_owned(), "构建".to_owned()],
            recent_aids: vec![284_063_097],
            preview_aids: vec![284_063_097],
        })
        .expect("mapped space series");
        assert_eq!(series.resource_ref.to_string(), "bilibili:series:3908327");
        assert_eq!(series.tags, ["Kotlin", "构建"]);
        assert_eq!(series.extensions["state"], 2);
        assert_eq!(series.extensions["creator_mode"], "auto");
    }

    #[test]
    fn playlist_locator_requires_explicit_nonzero_typed_identity() {
        assert_eq!(
            parse_bilibili_playlist_locator("season:3629748").expect("season locator"),
            BilibiliPlaylistLocator::Season(3_629_748)
        );
        assert_eq!(
            parse_bilibili_playlist_locator("favorite:2883236382")
                .expect("favorite folder locator"),
            BilibiliPlaylistLocator::FavoriteFolder(2_883_236_382)
        );
        assert_eq!(
            parse_bilibili_playlist_locator("series:3908327").expect("series locator"),
            BilibiliPlaylistLocator::Series(3_908_327)
        );
        for invalid in [
            "",
            "3629748",
            "season:0",
            "season:03629748",
            "season:-1",
            "playlist:1",
            "season:1:2",
        ] {
            let error =
                parse_bilibili_playlist_locator(invalid).expect_err("invalid playlist locator");
            assert_eq!(error.code, ErrorCode::InvalidRequest);
        }
    }

    #[tokio::test]
    async fn playlist_sources_require_supported_types_and_plain_numeric_ids() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let unsupported = provider
            .playlist_source("1", "user_directory", None)
            .await
            .expect_err("unsupported source type");
        assert_eq!(unsupported.code, ErrorCode::CapabilityNotSupported);
        assert_eq!(unsupported.details["source_type"], "user_directory");
        for (id, source_type) in [
            ("season:3629748", "season"),
            ("03629748", "season"),
            ("favorite:2883236382", "favorite_folder"),
            ("0", "favorite_folder"),
        ] {
            let error = provider
                .playlist_source(id, source_type, None)
                .await
                .expect_err("invalid source ID");
            assert_eq!(error.code, ErrorCode::InvalidRequest);
        }
    }

    #[test]
    fn season_archive_maps_to_typed_video_and_track_compatibility_view() {
        let archive = BilibiliSeasonArchive {
            aid: 170_001,
            bvid: "BV17x411w7KC".to_owned(),
            title: "测试视频".to_owned(),
            cover_url: "https://i0.hdslb.com/bfs/archive/test.jpg".to_owned(),
            duration_seconds: 185,
            created_at: 1_500_000_000,
            published_at: 1_500_000_100,
            interactive: true,
            playback_position: Some(42),
            state: 0,
            paid: false,
            view_count: 123_456,
            danmaku_count: Some(789),
        };
        let track =
            map_season_archive_track(archive.clone(), 3_629_748, 327_961_371).expect("track");
        assert_eq!(track.resource_ref.to_string(), "bilibili:bvid:BV17x411w7KC");
        assert_eq!(track.duration_ms, Some(185_000));
        assert_eq!(
            track.artists[0].resource_ref.as_ref().unwrap().to_string(),
            "bilibili:user:327961371"
        );
        assert_eq!(track.extensions["video_ref"], "bilibili:bvid:BV17x411w7KC");
        assert_eq!(track.extensions["normalized_from_video"], true);
        assert_eq!(track.extensions["playback_position"], 42);

        let detail =
            map_season_archive_video(archive, 3_629_748, 327_961_371).expect("video detail");
        assert_eq!(detail.kind, VideoResourceKind::Video);
        assert_eq!(
            detail.video.resource_ref.to_string(),
            "bilibili:bvid:BV17x411w7KC"
        );
        assert_eq!(detail.video.play_count, Some(123_456));
        assert_eq!(detail.video.extensions["aid"], 170_001);
        assert_eq!(detail.video.extensions["interactive"], true);
        assert_eq!(detail.extensions["summary_only"], true);
        assert!(detail.resolutions.is_empty());
    }

    #[test]
    fn favorite_folder_detail_maps_complete_playlist_metadata() {
        let playlist = map_favorite_folder(BilibiliFavoriteFolder {
            media_id: 2_883_236_382,
            folder_id: 28_832_363,
            owner: crate::client::BilibiliFavoriteFolderOwner {
                id: 47_275_982,
                name: "荷花-Lotus".to_owned(),
                avatar_url: Some("https://i2.hdslb.com/bfs/face/avatar.jpg".to_owned()),
                followed: false,
                vip_type: 1,
                vip_status: false,
            },
            attributes: 22,
            title: "相声".to_owned(),
            cover_url: Some("https://i2.hdslb.com/bfs/archive/folder.jpg".to_owned()),
            cover_type: 2,
            description: "公开收藏夹".to_owned(),
            created_at: 1_705_401_630,
            updated_at: 1_705_925_782,
            invalid: false,
            favorite_state: true,
            like_state: false,
            media_count: 99,
            pinned: true,
            child_friendly: false,
            child_friendly_description: String::new(),
            counts: crate::client::BilibiliFavoriteFolderCounts {
                collect: 3,
                play: 2_059,
                thumb_up: 7,
                share: 11,
            },
        })
        .expect("mapped favorite folder");
        assert_eq!(
            playlist.resource_ref.to_string(),
            "bilibili:favorite:2883236382"
        );
        assert_eq!(playlist.creator.as_ref().unwrap().name, "荷花-Lotus");
        assert_eq!(playlist.track_count, Some(99));
        assert_eq!(playlist.subscribed, Some(true));
        assert_eq!(playlist.extensions["fid"], 28_832_363);
        assert_eq!(playlist.extensions["private"], false);
        assert_eq!(playlist.extensions["default"], false);
        assert_eq!(playlist.extensions["play_count"], 2_059);
    }

    #[tokio::test]
    async fn playlist_detail_validates_kind_and_account_before_network_access() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let unsupported = provider
            .playlist("series:3908327", None)
            .await
            .expect_err("series detail is not implemented yet");
        assert_eq!(unsupported.code, ErrorCode::CapabilityNotSupported);
        assert_eq!(unsupported.details["playlist_kind"], "series");
        let missing = provider
            .playlist("favorite:2883236382", Some("missing"))
            .await
            .expect_err("missing selected account");
        assert_eq!(missing.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn playlist_items_validate_kind_limit_and_account_before_network_access() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let unsupported = provider
            .playlist_playable_items("series:3908327", &tuneweave_core::PageRequest::new(30, 0))
            .await
            .expect_err("series items are not implemented yet");
        assert_eq!(unsupported.code, ErrorCode::CapabilityNotSupported);
        assert_eq!(unsupported.details["playlist_kind"], "series");

        let invalid_limit = provider
            .playlist_tracks(
                "favorite:2883236382",
                &tuneweave_core::PageRequest::new(0, 0),
            )
            .await
            .expect_err("zero limit");
        assert_eq!(invalid_limit.code, ErrorCode::InvalidRequest);

        let missing = provider
            .playlist_playable_items(
                "season:3629748",
                &tuneweave_core::PageRequest {
                    limit: 5,
                    offset: 28,
                    account: Some("missing".to_owned()),
                },
            )
            .await
            .expect_err("missing selected account");
        assert_eq!(missing.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn created_favorite_folders_validate_identity_and_account_before_network_access() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        for invalid in ["", "0", "-1", "user:7792521", "abc"] {
            let error = provider
                .user_created_playlists(invalid, &tuneweave_core::PageRequest::new(30, 0))
                .await
                .expect_err("invalid user ID");
            assert_eq!(error.code, ErrorCode::InvalidRequest);
        }
        let error = provider
            .user_created_playlists(
                "7792521",
                &tuneweave_core::PageRequest {
                    limit: 30,
                    offset: 0,
                    account: Some("missing".to_owned()),
                },
            )
            .await
            .expect_err("missing selected account");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
        assert!(provider.capabilities().contains(&Capability::PlaylistRead));
    }

    #[tokio::test]
    async fn collected_playlists_validate_identity_and_account_before_network_access() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let invalid = provider
            .user_favorite_playlists("user:293793435", &tuneweave_core::PageRequest::new(30, 0))
            .await
            .expect_err("typed user prefix is invalid in a typed user route");
        assert_eq!(invalid.code, ErrorCode::InvalidRequest);
        let missing = provider
            .user_favorite_playlists(
                "293793435",
                &tuneweave_core::PageRequest {
                    limit: 30,
                    offset: 0,
                    account: Some("missing".to_owned()),
                },
            )
            .await
            .expect_err("missing selected account");
        assert_eq!(missing.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn video_search_rejects_unsupported_inputs_before_network_access() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let mut query = SearchQuery {
            query: "music".to_owned(),
            kind: SearchKind::Track,
            variant: SearchVariant::Default,
            limit: 20,
            offset: 0,
            account: None,
            search_id: None,
            highlight: false,
            selectors: Vec::new(),
            video_filters: None,
        };
        assert_eq!(
            provider
                .search_catalog(&query)
                .await
                .expect_err("track search is unsupported")
                .code,
            ErrorCode::CapabilityNotSupported
        );
        query.kind = SearchKind::Video;
        query.offset = 1_000;
        assert_eq!(
            provider
                .search_catalog(&query)
                .await
                .expect_err("large offset must fail")
                .code,
            ErrorCode::InvalidRequest
        );
        query.offset = 0;
        query.account = Some("missing".to_owned());
        assert_eq!(
            provider
                .search_catalog(&query)
                .await
                .expect_err("missing selected account must fail")
                .code,
            ErrorCode::AuthenticationRequired
        );
        assert!(provider.capabilities().contains(&Capability::SearchVideos));
    }

    #[tokio::test]
    async fn video_detail_rejects_wrong_kind_episode_and_missing_account_before_network() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let wrong_kind = provider
            .video(
                "bvid:BV117411r7R1",
                &VideoDetailRequest::new(VideoResourceKind::Mv),
            )
            .await
            .expect_err("MV kind");
        assert_eq!(wrong_kind.code, ErrorCode::InvalidRequest);
        let episode = provider
            .video("ep:123", &VideoDetailRequest::new(VideoResourceKind::Video))
            .await
            .expect_err("episode identity");
        assert_eq!(episode.code, ErrorCode::InvalidRequest);
        let missing = provider
            .video(
                "bvid:BV117411r7R1",
                &VideoDetailRequest {
                    kind: VideoResourceKind::Video,
                    account: Some("missing".to_owned()),
                },
            )
            .await
            .expect_err("missing account");
        assert_eq!(missing.code, ErrorCode::AuthenticationRequired);
        assert!(provider.capabilities().contains(&Capability::VideoDetail));

        let wrong_part_kind = provider
            .video_parts(
                "bvid:BV117411r7R1",
                &VideoPartListRequest::new(VideoResourceKind::Mv, 30, 0),
            )
            .await
            .expect_err("part MV kind");
        assert_eq!(wrong_part_kind.code, ErrorCode::InvalidRequest);
        let invalid_part_limit = provider
            .video_parts(
                "bvid:BV117411r7R1",
                &VideoPartListRequest::new(VideoResourceKind::Video, 0, 0),
            )
            .await
            .expect_err("zero part limit");
        assert_eq!(invalid_part_limit.code, ErrorCode::InvalidRequest);
        let episode_parts = provider
            .video_parts(
                "ep:123",
                &VideoPartListRequest::new(VideoResourceKind::Video, 30, 0),
            )
            .await
            .expect_err("episode parts");
        assert_eq!(episode_parts.code, ErrorCode::InvalidRequest);
        let missing_part_account = provider
            .video_parts(
                "bvid:BV117411r7R1",
                &VideoPartListRequest {
                    kind: VideoResourceKind::Video,
                    limit: 30,
                    offset: 0,
                    account: Some("missing".to_owned()),
                },
            )
            .await
            .expect_err("missing part account");
        assert_eq!(missing_part_account.code, ErrorCode::AuthenticationRequired);
        assert!(provider.capabilities().contains(&Capability::VideoParts));

        let wrong_subtitle_kind = provider
            .video_subtitles(
                "bvid:BV117411r7R1",
                &VideoSubtitleRequest::new(VideoResourceKind::Mv, "cid:146044693"),
            )
            .await
            .expect_err("subtitle MV kind");
        assert_eq!(wrong_subtitle_kind.code, ErrorCode::InvalidRequest);
        let invalid_subtitle_part = provider
            .video_subtitles(
                "bvid:BV117411r7R1",
                &VideoSubtitleRequest::new(VideoResourceKind::Video, "page:1"),
            )
            .await
            .expect_err("invalid subtitle part");
        assert_eq!(invalid_subtitle_part.code, ErrorCode::InvalidRequest);
        let missing_subtitle_account = provider
            .video_subtitles(
                "bvid:BV117411r7R1",
                &VideoSubtitleRequest {
                    kind: VideoResourceKind::Video,
                    part_id: "cid:146044693".to_owned(),
                    account: Some("missing".to_owned()),
                },
            )
            .await
            .expect_err("missing subtitle account");
        assert_eq!(
            missing_subtitle_account.code,
            ErrorCode::AuthenticationRequired
        );
        assert!(
            provider
                .capabilities()
                .contains(&Capability::VideoSubtitles)
        );
    }

    #[test]
    fn video_parts_keep_cid_identity_and_apply_a_local_window() {
        let page = map_bilibili_video_parts(
            170_001,
            "BV17x411w7KC".to_owned(),
            vec![
                BilibiliVideoPart {
                    cid: 279_786,
                    page: 1,
                    source: "vupload".to_owned(),
                    title: "第一 P".to_owned(),
                    duration_seconds: 120,
                    width: 1920,
                    height: 1080,
                    rotated: false,
                },
                BilibiliVideoPart {
                    cid: 279_787,
                    page: 2,
                    source: "vupload".to_owned(),
                    title: "第二 P".to_owned(),
                    duration_seconds: 0,
                    width: 0,
                    height: 0,
                    rotated: false,
                },
                BilibiliVideoPart {
                    cid: 279_788,
                    page: 3,
                    source: String::new(),
                    title: "第三 P".to_owned(),
                    duration_seconds: 180,
                    width: 1080,
                    height: 1920,
                    rotated: true,
                },
            ],
            &VideoPartListRequest::new(VideoResourceKind::Video, 1, 1),
        )
        .expect("mapped video part window");

        assert_eq!(page.items.len(), 1);
        assert_eq!(
            page.items[0].resource_ref.to_string(),
            "bilibili:cid:279787"
        );
        assert_eq!(
            page.items[0].video_ref.to_string(),
            "bilibili:bvid:BV17x411w7KC"
        );
        assert_eq!(page.items[0].page, 2);
        assert_eq!(page.items[0].duration_ms, None);
        assert_eq!(page.items[0].width, None);
        assert_eq!(page.items[0].height, None);
        assert_eq!(page.pagination.total, Some(3));
        assert_eq!(page.pagination.next_offset, Some(2));
        assert!(page.pagination.has_more);
        assert_eq!(
            page.pagination.extensions["video_ref"],
            "bilibili:bvid:BV17x411w7KC"
        );
    }

    #[test]
    fn subtitle_catalog_uses_stable_refs_and_keeps_platform_codes_typed() {
        let catalog = map_bilibili_subtitle_catalog(BilibiliSubtitleCatalog {
            aid: 60_977_932,
            bvid: "BV1Jt411P77c".to_owned(),
            cid: 106_101_299,
            requires_login: false,
            can_submit: true,
            default_language: Some("zh-CN".to_owned()),
            default_language_label: Some("中文（中国）".to_owned()),
            subtitles: vec![crate::client::BilibiliSubtitle {
                id: 13_643_112_644_608_002,
                id_string: "13643112644608002".to_owned(),
                language: "zh-Hans".to_owned(),
                label: "中文（简体）".to_owned(),
                locked: true,
                resource_url: url::Url::parse(
                    "https://aisubtitle.hdslb.com/bfs/subtitle/example.json?auth_key=redacted",
                )
                .expect("subtitle URL"),
                subtitle_type: 0,
                ai_type: 1,
                ai_status: 2,
            }],
        })
        .expect("mapped subtitle catalog");

        assert_eq!(catalog.video_ref.to_string(), "bilibili:bvid:BV1Jt411P77c");
        assert_eq!(catalog.part_ref.to_string(), "bilibili:cid:106101299");
        assert_eq!(catalog.items.len(), 1);
        assert_eq!(
            catalog.items[0].resource_ref.to_string(),
            "bilibili:subtitle:13643112644608002"
        );
        assert_eq!(catalog.items[0].language, "zh-Hans");
        assert_eq!(catalog.items[0].format, "bilibili_json");
        assert_eq!(catalog.items[0].extensions["ai_type"], 1);
        assert_eq!(catalog.items[0].extensions["ai_status"], 2);
        assert_eq!(catalog.items[0].extensions["content_available"], true);
        assert!(
            !serde_json::to_string(&catalog)
                .expect("serialize subtitle catalog")
                .contains("auth_key")
        );
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili Passport access"]
    async fn live_provider_creates_a_qr_image_without_exposing_the_poll_key() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let start = provider
            .start_qr_login(None)
            .await
            .expect("provider QR start");
        assert_eq!(start.provider_transaction_id.len(), 32);
        assert!(start.url.starts_with("data:image/svg+xml;base64,"));
        assert_eq!(start.image_data_url.as_deref(), Some(start.url.as_str()));
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili favorite folder access"]
    async fn live_provider_keeps_public_favorite_folders_in_created_directory() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let page = provider
            .user_created_playlists("7792521", &tuneweave_core::PageRequest::new(30, 0))
            .await
            .expect("public created favorite folders");
        assert!(!page.items.is_empty());
        assert_eq!(page.pagination.extensions["user_mid"], 7_792_521);
        assert!(
            page.items
                .iter()
                .any(|playlist| playlist.id.starts_with("favorite:"))
        );
        assert!(page.pagination.extensions["favorite_folder_count"].as_u64() > Some(0));
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili collected playlist access"]
    async fn live_provider_paginates_public_collected_playlists() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let page = provider
            .user_favorite_playlists("293793435", &tuneweave_core::PageRequest::new(10, 5))
            .await
            .expect("public collected playlists");
        assert_eq!(page.items.len(), 10);
        assert_eq!(page.pagination.offset, 5);
        assert!(page.pagination.has_more);
        assert_eq!(page.pagination.extensions["user_mid"], 293_793_435);
        assert!(
            page.items
                .iter()
                .all(|playlist| playlist.subscribed.is_some())
        );
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili space playlist access"]
    async fn live_created_directory_merges_folders_seasons_and_series() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let page = provider
            .user_created_playlists("37737161", &tuneweave_core::PageRequest::new(30, 0))
            .await
            .expect("merged created playlist directory");
        assert!(
            page.items
                .iter()
                .any(|playlist| playlist.id.starts_with("season:"))
        );
        assert!(
            page.items
                .iter()
                .any(|playlist| playlist.id.starts_with("series:"))
        );
        assert!(page.pagination.extensions["space_playlist_count"].as_u64() > Some(0));
        assert_eq!(
            page.pagination.extensions["directory_order"],
            json!(["favorite_folder", "season_or_series"])
        );
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili season access"]
    async fn live_playlist_detail_maps_public_season() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let playlist = provider
            .playlist("season:3629748", None)
            .await
            .expect("live public season detail");
        assert_eq!(playlist.resource_ref.to_string(), "bilibili:season:3629748");
        assert_eq!(playlist.track_count, Some(617));
        assert_eq!(playlist.extensions["owner_mid"], 327_961_371);
        assert_eq!(playlist.extensions["detail_source"], "season_archives");
        assert_eq!(playlist.extensions["first_page_archive_count"], 30);
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili favorite folder access"]
    async fn live_playlist_detail_maps_public_favorite_folder() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let playlist = provider
            .playlist("favorite:2883236382", None)
            .await
            .expect("live public favorite folder detail");
        assert_eq!(
            playlist.resource_ref.to_string(),
            "bilibili:favorite:2883236382"
        );
        assert_eq!(playlist.creator.as_ref().unwrap().name, "荷花-Lotus");
        assert_eq!(playlist.track_count, Some(99));
        assert_eq!(playlist.extensions["owner_mid"], 47_275_982);
        assert_eq!(playlist.extensions["fid"], 28_832_363);
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili favorite folder access"]
    async fn live_favorite_video_page_crosses_upstream_boundary() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let page = provider
            .playlist_playable_items(
                "favorite:2883236382",
                &tuneweave_core::PageRequest::new(5, 18),
            )
            .await
            .expect("live public favorite videos");
        assert_eq!(page.items.len(), 5);
        assert_eq!(page.pagination.offset, 18);
        assert_eq!(page.pagination.total, Some(99));
        assert_eq!(page.pagination.next_offset, Some(23));
        assert_eq!(page.pagination.extensions["upstream_pages_fetched"], 2);
        assert!(page.items.iter().all(|item| {
            matches!(
            item,
            PlaylistPlayableItem::Video(detail)
                if detail.video.resource_ref.platform() == Platform::Bilibili
                    && detail.video.extensions["media_id"] == 2_883_236_382_u64
            )
        }));

        let gap_page = provider
            .playlist_playable_items(
                "favorite:2883236382",
                &tuneweave_core::PageRequest::new(5, 39),
            )
            .await
            .expect("page after an upstream pagination gap");
        assert_eq!(gap_page.items.len(), 5);
        assert_eq!(gap_page.pagination.next_offset, Some(45));
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili playlist access"]
    async fn live_uni_sources_traverse_complete_season_and_favorite_folder() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        for (id, source_type, expected_total) in [
            ("3629748", "season", 617_u64),
            ("2883236382", "favorite_folder", 99_u64),
        ] {
            let playlist = provider
                .playlist_source(id, source_type, None)
                .await
                .expect("live playlist source");
            assert_eq!(playlist.track_count, Some(expected_total));
            let mut offset = 0;
            let mut returned = 0_u64;
            loop {
                let page = provider
                    .playlist_source_items(
                        id,
                        source_type,
                        &tuneweave_core::PageRequest::new(100, offset),
                    )
                    .await
                    .expect("live playlist source items");
                assert!(
                    page.items
                        .iter()
                        .all(|item| matches!(item, PlaylistPlayableItem::Video(_)))
                );
                returned += page.items.len() as u64;
                if !page.pagination.has_more {
                    break;
                }
                offset = page
                    .pagination
                    .next_offset
                    .expect("continuing source page has next offset");
            }
            assert!(returned <= expected_total);
            assert!(returned > 0);
            if source_type == "season" {
                assert_eq!(returned, expected_total);
            } else {
                assert_eq!(returned, 98);
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili video access"]
    async fn live_video_detail_resolves_aid_and_bvid_to_one_canonical_video() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let request = VideoDetailRequest::new(VideoResourceKind::Video);
        let by_bvid = provider
            .video("bvid:BV117411r7R1", &request)
            .await
            .expect("video detail by BVID");
        let by_aid = provider
            .video("aid:85440373", &request)
            .await
            .expect("video detail by AID");
        assert_eq!(by_aid.video.resource_ref, by_bvid.video.resource_ref);
        assert_eq!(
            by_bvid.video.resource_ref.to_string(),
            "bilibili:bvid:BV117411r7R1"
        );
        assert_eq!(by_bvid.video.extensions["aid"], 85_440_373);
        assert_eq!(by_bvid.video.extensions["first_cid"], 146_044_693);
        assert_eq!(by_bvid.video.extensions["part_count"], 1);
        assert!(
            by_bvid
                .video
                .cover_url
                .as_deref()
                .unwrap()
                .starts_with("https://")
        );
        assert!(!by_bvid.video.creators[0].name.is_empty());
        assert_eq!(
            by_bvid.video.creators[0]
                .resource_ref
                .as_ref()
                .unwrap()
                .platform(),
            Platform::Bilibili
        );
        assert!(by_bvid.resolutions.is_empty());
        assert_eq!(by_bvid.extensions["resolutions_require_playurl"], true);
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili video access"]
    async fn live_video_parts_preserve_all_pages_and_support_offset_windows() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let request = VideoPartListRequest::new(VideoResourceKind::Video, 3, 4);
        let page = provider
            .video_parts("bvid:BV17x411w7KC", &request)
            .await
            .expect("multi-part video directory");
        assert_eq!(page.items.len(), 3);
        assert_eq!(page.pagination.total, Some(10));
        assert_eq!(page.pagination.offset, 4);
        assert_eq!(page.pagination.next_offset, Some(7));
        assert!(page.pagination.has_more);
        for (index, part) in page.items.iter().enumerate() {
            assert_eq!(part.page, u32::try_from(index).unwrap() + 5);
            assert!(part.resource_ref.id().starts_with("cid:"));
            assert_eq!(part.video_ref.to_string(), "bilibili:bvid:BV17x411w7KC");
            assert_eq!(part.extensions["aid"], 170_001);
        }
    }

    #[tokio::test]
    #[ignore = "requires live Bilibili season access"]
    async fn live_season_video_page_crosses_upstream_boundary() {
        let provider = BilibiliProvider::new(BilibiliConfig::default()).expect("provider");
        let page = provider
            .playlist_playable_items("season:3629748", &tuneweave_core::PageRequest::new(5, 28))
            .await
            .expect("live public season videos");
        assert_eq!(page.items.len(), 5);
        assert_eq!(page.pagination.offset, 28);
        assert_eq!(page.pagination.total, Some(617));
        assert_eq!(page.pagination.next_offset, Some(33));
        assert!(page.pagination.has_more);
        assert_eq!(page.pagination.extensions["upstream_pages_fetched"], 2);
        assert!(page.items.iter().all(|item| {
            matches!(
                item,
                PlaylistPlayableItem::Video(detail)
                    if detail.video.resource_ref.id().starts_with("bvid:BV")
                        && detail.video.extensions["video_ref"] == detail.video.resource_ref.to_string()
            )
        }));
    }
}
