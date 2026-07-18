use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    io::Cursor,
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use lofty::{file::TaggedFileExt, probe::Probe, tag::Accessor};
use md5::{Digest, Md5};
use percent_encoding::{NON_ALPHANUMERIC, utf8_percent_encode};
use qrcode::{QrCode, render::svg};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tuneweave_core::{
    AccountCredentialStore, AccountProfile, Album, AlbumListRequest, AlbumStats, AlbumSummary,
    Artist, ArtistArea, ArtistBiographySection, ArtistCategory, ArtistChart, ArtistChartArea,
    ArtistChartEntry, ArtistChartRequest, ArtistContentCount, ArtistListRequest, ArtistOverview,
    ArtistStats, ArtistSummary, ArtistTrackListRequest, ArtistTrackOrder, ArtistUpdatesRequest,
    ArtistVideoListRequest, ArtistWorkKind, ArtistWorkUpdate, ArtistWorksRequest, AudioRecognition,
    AudioRecognitionMatch, AudioRecognitionRequest, AuthChallengeRequest, AuthChallengeValidation,
    AuthPrincipalStatus, AuthPrincipalStatusRequest, AuthState, Banner, BannerCatalog,
    BannerClient, BannerListRequest, BannerTargetKind, Capability, ChallengeMethod, Chart,
    ChartCatalog, ChartCatalogRequest, ChartCatalogView, ChartGroup, ChartTrackPreview,
    CloudImportRequest, CloudImportResult, CloudLyricsRequest, CloudMatchRequest, CloudMatchResult,
    CloudTrack, CloudTrackDeleteRequest, CloudTrackDeleteResult, CloudTrackDetailRequest,
    CloudUploadCompleteRequest, CloudUploadRequest, CloudUploadResult, CloudUploadTicket,
    CloudUploadTicketRequest, Comment, CommentDeleteRequest, CommentListRequest, CommentListView,
    CommentMutationAction, CommentMutationResult, CommentPage, CommentReaction,
    CommentReactionKind, CommentReactionListRequest, CommentReactionMutationRequest,
    CommentReactionMutationResult, CommentReactionPage, CommentReplyReference,
    CommentReportRequest, CommentReportResult, CommentSort, CommentTarget, CommentTargetKind,
    CommentThreadStats, CommentThreadStatsBatch, CommentThreadStatsRequest, CommentWriteRequest,
    CountryCallingCode, CountryCallingCodeGroup, CountryCallingCodeListRequest, CreatorSummary,
    DigitalAlbum, DigitalAlbumChartEntry, DigitalAlbumChartKind, DigitalAlbumChartPeriod,
    DigitalAlbumChartRequest, DigitalAlbumListRequest, DimensionChart, DimensionChartRequest,
    DimensionChartTrackEntry, DimensionChartTrackSnapshot, ErrorCode, Extensions,
    ImageUploadRequest, ImageUploadResult, LocalTrackMatchRequest, LocalTrackMatchResult,
    LyricContributor, Lyrics, MediaDownload, MediaStream, MembershipSummary, Money, MusicProvider,
    Page, PageMeta, PageRequest, ParseResourceRefError, PasswordFormat, PasswordLoginRequest,
    Platform, PlatformApiRequest, PlatformBatchRequest, PlaybackHistoryEntry,
    PlaybackHistoryPeriod, PlaybackHistoryRequest, Playlist, PlaylistCoverUpdateResult,
    PlaylistCreateRequest, PlaylistDeleteRequest, PlaylistDeleteResult, PlaylistItemKind,
    PlaylistItemMutationAction, PlaylistItemMutationRequest, PlaylistItemMutationResult,
    PlaylistKind, PlaylistMetadataUpdateVariant, PlaylistMutationAction, PlaylistMutationResult,
    PlaylistOrderRequest, PlaylistOrderResult, PlaylistTrackOrderRequest, PlaylistTrackOrderResult,
    PlaylistUpdateRequest, PlaylistVisibility, Podcast, PodcastCatalog, PodcastCategory,
    PodcastChartEntry, PodcastChartKind, PodcastChartRequest, PodcastCreatorChartEntry,
    PodcastCreatorChartKind, PodcastCreatorChartRequest, PodcastEpisode, PodcastEpisodeChartEntry,
    PodcastEpisodeChartKind, PodcastEpisodeChartRequest, PodcastEpisodeListRequest,
    PodcastEpisodeLyrics, PodcastEpisodeStream, PodcastListRequest, PodcastTaxonomy, PrincipalType,
    ProviderQrPoll, ProviderQrStart, Quality, RadioCatalogOption, RadioStation, RadioStationCursor,
    RadioStationListRequest, RadioTaxonomy, RadioTaxonomyRequest, RecommendationRequest,
    ResolutionStatus, ResourceRef, Result, SearchDefaultKeyword, SearchDefaultKeywordRequest,
    SearchItem, SearchKind, SearchMultiMatch, SearchMultiMatchRequest, SearchMultiMatchSection,
    SearchOpaqueItem, SearchQuery, SearchSuggestion, SearchSuggestionClient, SearchSuggestionList,
    SearchSuggestionRequest, SearchTrendingDetail, SearchTrendingEntry, SearchTrendingList,
    SearchTrendingRequest, SearchVariant, StoredAccountCredential, StreamBatch, StreamOutcome,
    StreamRequest, StreamVariant, SubscriptionResult, Track, TrackAvailability,
    TrackAvailabilityRequest, TrackEntitlement, TrialWindow, TuneWeaveError, User, Video,
    VideoDetail, VideoDetailRequest, VideoKind, VideoResolution, VideoResourceKind, VideoStats,
    VideoStream, VideoStreamRequest,
};
use url::Url;

use crate::{
    NeteaseAccountSummary, NeteaseCaptchaVerification, NeteaseClient, NeteaseConfig,
    NeteaseLoginResult, NeteaseQrCheck, NeteaseQrLogin, NeteaseQrState, NeteaseSessionStatus,
    dto::{
        AlbumDetail, AlbumEntitlementsEnvelope, AlbumEnvelope, AlbumListEnvelope,
        AlbumStatsEnvelope, ArtistAlbumsEnvelope, ArtistChartEnvelope, ArtistDescriptionEnvelope,
        ArtistDetailEnvelope, ArtistDynamicEnvelope, ArtistFanProfile, ArtistFansEnvelope,
        ArtistFollowCountEnvelope, ArtistListEnvelope, ArtistListItem, ArtistMvItem,
        ArtistMvsEnvelope, ArtistNewTracksEnvelope, ArtistNewTracksPlayAllEnvelope,
        ArtistNewVideoItem, ArtistNewVideosEnvelope, ArtistNewWorksEnvelope,
        ArtistOverviewEnvelope, ArtistSublistEnvelope, ArtistTopTracksEnvelope,
        ArtistTracksEnvelope, ArtistVideoCreator, ArtistVideoRecord, ArtistVideosEnvelope,
        AudioMatchEnvelope, AudioQuality, BannerEnvelope, BroadcastTaxonomyEnvelope,
        ChartCatalogEnvelope, ChartGroupItem, ChartItem, ChartRankPreviewItem,
        ChartTextPreviewItem, CloudTracksEnvelope, CloudUploadAllocationEnvelope,
        CloudUploadServersEnvelope, CloudVideoDetailEnvelope, CloudVideoUrlEnvelope,
        DigitalAlbumChartEnvelope, DigitalAlbumChartItem, DigitalAlbumEnvelope,
        DigitalAlbumListEnvelope, DigitalAlbumListItem, DimensionChartDetailEnvelope,
        DimensionChartTrackItem, DimensionChartTracksEnvelope, ImageUploadAllocationEnvelope,
        LikedTracksEnvelope, LyricText, LyricUser, LyricsEnvelope, MvDetailEnvelope, MvUrlEnvelope,
        PlayHistoryEnvelope, PlayHistoryRecord, PlaylistDetail, PlaylistEnvelope, Privilege,
        RecommendationReason, RecommendedPlaylistsEnvelope, RecommendedTracksEnvelope,
        SearchEnvelope, Song, StreamData, StreamEnvelope, SubscribedAlbumsEnvelope,
        TrackEntitlementData, TrackEnvelope, UserPlaylistsEnvelope, VideoCreatorItem,
        VideoStatsEnvelope, VideoUrlItem,
    },
};

const CLOUD_UPLOAD_BUCKET: &str = "jd-musicrep-privatecloud-audio-public";
const NETEASE_CREDENTIAL_KIND: &str = "cookie";

#[derive(Clone)]
pub struct NeteaseProvider {
    client: NeteaseClient,
    accounts: Arc<RwLock<BTreeMap<String, NeteaseClient>>>,
    credential_store: Option<Arc<dyn AccountCredentialStore>>,
}

impl NeteaseProvider {
    pub fn new(config: NeteaseConfig) -> Result<Self> {
        let credential_store = config.credential_store.clone();
        let provider = Self {
            client: NeteaseClient::new(config)?,
            accounts: Arc::new(RwLock::new(BTreeMap::new())),
            credential_store,
        };
        provider.restore_sessions()?;
        Ok(provider)
    }

    #[must_use]
    pub fn from_client(client: NeteaseClient) -> Self {
        Self {
            client,
            accounts: Arc::new(RwLock::new(BTreeMap::new())),
            credential_store: None,
        }
    }

    pub async fn create_qr_login(&self) -> Result<NeteaseQrLogin> {
        self.client.create_qr_login().await
    }

    pub async fn check_qr_login(&self, key: &str, account: &str) -> Result<NeteaseQrCheck> {
        let account = normalize_account_label(Some(account))?.to_owned();
        let check = self.client.check_qr_login(key).await?;
        if check.state == NeteaseQrState::Confirmed {
            let cookie = check.session_cookie().ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "NetEase confirmed QR login without returning MUSIC_U",
                )
                .with_platform(Platform::Netease)
            })?;
            self.install_session(&account, cookie.to_owned())?;
        }
        Ok(check)
    }

    pub async fn send_phone_captcha(&self, phone: &str, country_code: &str) -> Result<()> {
        self.client.send_phone_captcha(phone, country_code).await
    }

    pub async fn verify_phone_captcha(
        &self,
        phone: &str,
        country_code: &str,
        captcha: &str,
    ) -> Result<NeteaseCaptchaVerification> {
        self.client
            .verify_phone_captcha(phone, country_code, captcha)
            .await
    }

    pub async fn login_with_email_password(
        &self,
        account: &str,
        email: &str,
        password: &str,
    ) -> Result<NeteaseAccountSummary> {
        let account = normalize_account_label(Some(account))?;
        let login = self
            .client
            .login_with_email_password(email, password)
            .await?;
        self.persist_login(account, login)
    }

    pub async fn login_with_email_md5(
        &self,
        account: &str,
        email: &str,
        password_md5: &str,
    ) -> Result<NeteaseAccountSummary> {
        let account = normalize_account_label(Some(account))?;
        let login = self
            .client
            .login_with_email_md5(email, password_md5)
            .await?;
        self.persist_login(account, login)
    }

    pub async fn login_with_phone_password(
        &self,
        account: &str,
        phone: &str,
        country_code: &str,
        password: &str,
    ) -> Result<NeteaseAccountSummary> {
        let account = normalize_account_label(Some(account))?;
        let login = self
            .client
            .login_with_phone_password(phone, country_code, password)
            .await?;
        self.persist_login(account, login)
    }

    pub async fn login_with_phone_password_md5(
        &self,
        account: &str,
        phone: &str,
        country_code: &str,
        password_md5: &str,
    ) -> Result<NeteaseAccountSummary> {
        let account = normalize_account_label(Some(account))?;
        let login = self
            .client
            .login_with_phone_password_md5(phone, country_code, password_md5)
            .await?;
        self.persist_login(account, login)
    }

    pub async fn login_with_phone_captcha(
        &self,
        account: &str,
        phone: &str,
        country_code: &str,
        captcha: &str,
    ) -> Result<NeteaseAccountSummary> {
        let account = normalize_account_label(Some(account))?;
        let login = self
            .client
            .login_with_phone_captcha(phone, country_code, captcha)
            .await?;
        self.persist_login(account, login)
    }

    pub async fn logout_account(&self, account: &str) -> Result<bool> {
        let account = normalize_account_label(Some(account))?.to_owned();
        let client = self.client_for(Some(&account))?;
        let remote_logout = client.logout().await;
        let removed = self.remove_session(&account)?;
        remote_logout
            .map_err(|error| error.with_details(json!({ "local_session_removed": removed })))?;
        Ok(removed)
    }

    fn persist_login(
        &self,
        account: &str,
        login: NeteaseLoginResult,
    ) -> Result<NeteaseAccountSummary> {
        let summary = login.account.clone();
        self.install_session(account, login.into_session_cookie())?;
        Ok(summary)
    }

    fn client_for(&self, account: Option<&str>) -> Result<NeteaseClient> {
        let account = normalize_account_label(account)?;
        let accounts = self.accounts.read().map_err(|_| account_store_error())?;
        if let Some(client) = accounts.get(account) {
            return Ok(client.clone());
        }
        if account == "default" {
            return Ok(self.client.clone());
        }
        Err(TuneWeaveError::new(
            ErrorCode::AuthenticationRequired,
            format!("NetEase account alias {account} is not logged in"),
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "account": account })))
    }

    async fn upload_image_blob(
        &self,
        request: &ImageUploadRequest,
        operation: &str,
    ) -> Result<(NeteaseClient, ImageUploadAllocationEnvelope, Value)> {
        let (filename, content_type) = validate_image_upload(request)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, operation)?;
        let allocation_response = client.allocate_image_upload(filename).await?;
        ensure_account_access(&client, &allocation_response.body, operation)?;
        let allocation: ImageUploadAllocationEnvelope = parse_body(allocation_response.body)?;
        validate_image_allocation(&allocation)?;
        let upload_response = client
            .upload_image(
                &allocation.result.object_key,
                &allocation.result.token,
                content_type,
                &request.data,
            )
            .await?;
        ensure_success(&upload_response.body)?;
        Ok((client, allocation, upload_response.body))
    }

    fn install_session(&self, account: &str, cookie: String) -> Result<()> {
        let account = normalize_account_label(Some(account))?.to_owned();
        validate_session_cookie(&cookie)?;
        if let Some(store) = &self.credential_store {
            store.put(&StoredAccountCredential::new(
                Platform::Netease,
                &account,
                NETEASE_CREDENTIAL_KIND,
                &cookie,
            )?)?;
        }
        self.install_session_in_memory(account, cookie)
    }

    fn install_session_in_memory(&self, account: String, cookie: String) -> Result<()> {
        validate_session_cookie(&cookie)?;
        self.accounts
            .write()
            .map_err(|_| account_store_error())?
            .insert(account, self.client.with_cookie(cookie));
        Ok(())
    }

    fn restore_sessions(&self) -> Result<()> {
        let Some(store) = &self.credential_store else {
            return Ok(());
        };
        for credential in store.load_platform(Platform::Netease)? {
            if credential.kind != NETEASE_CREDENTIAL_KIND {
                return Err(TuneWeaveError::new(
                    ErrorCode::InternalError,
                    format!(
                        "unsupported NetEase account credential kind: {}",
                        credential.kind
                    ),
                )
                .with_platform(Platform::Netease));
            }
            let account = normalize_account_label(Some(&credential.account))?.to_owned();
            self.install_session_in_memory(account, credential.into_secret())?;
        }
        Ok(())
    }

    fn remove_session(&self, account: &str) -> Result<bool> {
        let account = normalize_account_label(Some(account))?;
        let persisted = self
            .credential_store
            .as_ref()
            .map_or(Ok(false), |store| store.remove(Platform::Netease, account))?;
        let mut accounts = self.accounts.write().map_err(|_| account_store_error())?;
        let removed = accounts.remove(account).is_some();
        if account == "default" {
            let had_default = persisted || removed || self.client.is_authenticated();
            accounts.insert(account.to_owned(), self.client.without_cookie());
            return Ok(had_default);
        }
        Ok(persisted || removed)
    }

    async fn playlist_detail(&self, client: &NeteaseClient, id: u64) -> Result<PlaylistDetail> {
        let response = client
            .request_eapi(
                "/api/v6/playlist/detail",
                json!({
                    "id": id,
                    "n": 100_000,
                    "s": 8
                }),
            )
            .await?;
        ensure_success(&response.body)?;
        let response: PlaylistEnvelope = parse_body(response.body)?;
        response.playlist.ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::ResourceNotFound,
                "NetEase playlist was not found",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "id": id }))
        })
    }
}

#[async_trait]
impl MusicProvider for NeteaseProvider {
    fn platform(&self) -> Platform {
        Platform::Netease
    }

    fn name(&self) -> &'static str {
        "NetEase Cloud Music"
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::SearchTracks,
            Capability::SearchAlbums,
            Capability::SearchArtists,
            Capability::SearchPlaylists,
            Capability::SearchUsers,
            Capability::SearchMvs,
            Capability::SearchLyrics,
            Capability::SearchPodcasts,
            Capability::SearchVideos,
            Capability::SearchMixed,
            Capability::SearchVoices,
            Capability::SearchDefault,
            Capability::SearchTrending,
            Capability::SearchSuggestions,
            Capability::SearchMultiMatch,
            Capability::SearchLocalTrackMatch,
            Capability::UserMembership,
            Capability::AudioRecognition,
            Capability::Banners,
            Capability::RadioTaxonomy,
            Capability::RadioStationDetail,
            Capability::RadioStationList,
            Capability::RadioStationSubscriptionWrite,
            Capability::PodcastCategories,
            Capability::PodcastList,
            Capability::PodcastCharts,
            Capability::PodcastCreatorCharts,
            Capability::PodcastDetail,
            Capability::PodcastWorkbenchDetail,
            Capability::PodcastSubscriptionWrite,
            Capability::PodcastEpisodeList,
            Capability::PodcastEpisodeCharts,
            Capability::PodcastEpisodeDetail,
            Capability::PodcastEpisodeWorkbenchDetail,
            Capability::PodcastEpisodeStream,
            Capability::PodcastEpisodeLyrics,
            Capability::TrackDetail,
            Capability::TrackAvailability,
            Capability::AlbumDetail,
            Capability::AlbumList,
            Capability::AlbumStats,
            Capability::AlbumTrackEntitlements,
            Capability::AlbumSubscriptionWrite,
            Capability::DigitalAlbumDetail,
            Capability::DigitalAlbumList,
            Capability::DigitalAlbumCharts,
            Capability::ChartCatalog,
            Capability::ArtistCharts,
            Capability::DimensionCharts,
            Capability::ArtistDetail,
            Capability::ArtistOverview,
            Capability::ArtistStats,
            Capability::ArtistList,
            Capability::ArtistAlbums,
            Capability::ArtistFans,
            Capability::ArtistVideos,
            Capability::ArtistTracks,
            Capability::ArtistTopTracks,
            Capability::ArtistSubscriptionWrite,
            Capability::PlaylistRead,
            Capability::PlaylistWrite,
            Capability::Lyrics,
            Capability::AudioStream,
            Capability::AudioStreamBatch,
            Capability::AudioDownload,
            Capability::VideoDetail,
            Capability::VideoStats,
            Capability::VideoStream,
            Capability::QrLogin,
            Capability::PasswordLogin,
            Capability::PhoneLogin,
            Capability::CountryCallingCodes,
            Capability::ChallengeValidation,
            Capability::PrincipalStatus,
            Capability::SessionManagement,
            Capability::AccountProfile,
            Capability::AccountPlaylists,
            Capability::AccountAlbums,
            Capability::AccountRadioStations,
            Capability::AccountPodcasts,
            Capability::AccountFollowingArtists,
            Capability::AccountArtistNewVideos,
            Capability::AccountArtistNewTracks,
            Capability::AccountArtistNewWorks,
            Capability::AccountArtistNewTracksPlayAll,
            Capability::AccountAvatarWrite,
            Capability::AccountCloudUpload,
            Capability::AccountCloudDirectUpload,
            Capability::AccountCloudImport,
            Capability::AccountCloudLyrics,
            Capability::AccountCloudMatch,
            Capability::AccountCloudRead,
            Capability::AccountCloudDelete,
            Capability::AccountCloudDownload,
            Capability::Favorites,
            Capability::ListeningHistory,
            Capability::Recommendations,
            Capability::CommentWrite,
            Capability::CommentsRead,
            Capability::CommentReactionsRead,
            Capability::CommentReactionsWrite,
            Capability::CommentReportsWrite,
            Capability::CommentThreadStats,
            Capability::PlatformApi,
            Capability::PlatformBatch,
        ])
    }

    async fn search(&self, query: &SearchQuery) -> Result<Page<Track>> {
        if query.kind != SearchKind::Track {
            return Err(TuneWeaveError::unsupported(
                Platform::Netease,
                capability_for_search(query.kind),
            ));
        }
        let keyword = query.query.trim();
        if keyword.is_empty() {
            return Err(TuneWeaveError::invalid_request(
                "search query cannot be empty",
            ));
        }
        let limit = query.limit.clamp(1, 100);
        let client = self.client_for(query.account.as_deref())?;
        let response = client
            .request_eapi(
                "/api/search/get",
                json!({
                    "s": keyword,
                    "type": 1,
                    "limit": limit,
                    "offset": query.offset
                }),
            )
            .await?;
        ensure_success(&response.body)?;
        let response: SearchEnvelope = parse_body(response.body)?;
        let count = response.result.songs.len() as u32;
        let next_offset = query.offset.saturating_add(count);
        let has_more = u64::from(next_offset) < response.result.song_count;
        let items = response
            .result
            .songs
            .into_iter()
            .map(|song| map_song(song, None))
            .collect::<Result<Vec<_>>>()?;

        Ok(Page {
            items,
            pagination: PageMeta {
                limit,
                offset: query.offset,
                total: Some(response.result.song_count),
                next_offset: has_more.then_some(next_offset),
                has_more,
                extensions: Default::default(),
            },
        })
    }

    async fn search_catalog(&self, query: &SearchQuery) -> Result<Page<SearchItem>> {
        let keyword = query.query.trim();
        if keyword.is_empty() {
            return Err(
                TuneWeaveError::invalid_request("search query cannot be empty")
                    .with_platform(Platform::Netease),
            );
        }
        if query.kind == SearchKind::RadioStation {
            return Err(TuneWeaveError::unsupported(
                Platform::Netease,
                Capability::SearchRadioStations,
            ));
        }
        let limit = query.limit.clamp(1, 100);
        let client = self.client_for(query.account.as_deref())?;
        let (path, payload, variant) = netease_catalog_search_request(query, keyword, limit);
        let response = client.request_eapi(path, payload).await?;
        ensure_success(&response.body)?;
        let mut page = map_cloud_search_response(query.kind, limit, query.offset, response.body)?;
        page.pagination
            .extensions
            .insert("variant".to_owned(), json!(variant));
        page.pagination
            .extensions
            .insert("request_path".to_owned(), json!(path));
        Ok(page)
    }

    async fn default_search_keyword(
        &self,
        request: &SearchDefaultKeywordRequest,
    ) -> Result<SearchDefaultKeyword> {
        let client = self.client_for(request.account.as_deref())?;
        let (path, payload) = netease_default_search_keyword_request();
        let response = client.request_eapi(path, payload).await?;
        ensure_success(&response.body)?;
        map_netease_default_search_keyword(response.body)
    }

    async fn trending_searches(
        &self,
        request: &SearchTrendingRequest,
    ) -> Result<SearchTrendingList> {
        let client = self.client_for(request.account.as_deref())?;
        let (path, payload, use_weapi) = netease_trending_search_request(request.detail);
        let response = if use_weapi {
            client.request_weapi(path, payload).await?
        } else {
            client.request_eapi(path, payload).await?
        };
        ensure_success(&response.body)?;
        map_netease_trending_searches(request.detail, response.body)
    }

    async fn search_suggestions(
        &self,
        request: &SearchSuggestionRequest,
    ) -> Result<SearchSuggestionList> {
        let query = request.query.trim();
        if query.is_empty() {
            return Err(
                TuneWeaveError::invalid_request("search suggestion query cannot be empty")
                    .with_platform(Platform::Netease),
            );
        }
        let client = self.client_for(request.account.as_deref())?;
        let (path, payload, use_weapi) = netease_search_suggestion_request(request.client, query);
        let response = if use_weapi {
            client.request_weapi(path, payload).await?
        } else {
            client.request_eapi(path, payload).await?
        };
        ensure_success(&response.body)?;
        map_netease_search_suggestions(request.client, query, response.body)
    }

    async fn search_multi_match(
        &self,
        request: &SearchMultiMatchRequest,
    ) -> Result<SearchMultiMatch> {
        let query = request.query.trim();
        if query.is_empty() {
            return Err(TuneWeaveError::invalid_request(
                "multi-match search query cannot be empty",
            )
            .with_platform(Platform::Netease));
        }
        if request.kind == SearchKind::RadioStation {
            return Err(TuneWeaveError::unsupported(
                Platform::Netease,
                Capability::SearchRadioStations,
            ));
        }
        let client = self.client_for(request.account.as_deref())?;
        let (path, payload) = netease_search_multi_match_request(request.kind, query);
        let response = client.request_weapi(path, payload).await?;
        ensure_success(&response.body)?;
        map_netease_search_multi_match(query, request.kind, response.body)
    }

    async fn match_local_track(
        &self,
        request: &LocalTrackMatchRequest,
    ) -> Result<LocalTrackMatchResult> {
        let (path, payload, md5) = netease_local_track_match_request(request)?;
        let client = self.client_for(request.account.as_deref())?;
        let response = client.request_api(path, payload).await?;
        ensure_success(&response.body)?;
        map_netease_local_track_match(&md5, response.body)
    }

    async fn user_membership(
        &self,
        id: Option<&str>,
        account: Option<&str>,
    ) -> Result<MembershipSummary> {
        let id = id
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(|id| parse_numeric_id("user", id))
            .transpose()?;
        let client = self.client_for(account)?;
        let (path, payload) = netease_user_membership_request(id);
        let response = client.request_weapi(path, payload).await?;
        ensure_success(&response.body)?;
        map_netease_user_membership(id, response.body)
    }

    async fn recognize_audio(&self, request: &AudioRecognitionRequest) -> Result<AudioRecognition> {
        let fingerprint = request.fingerprint.trim();
        if fingerprint.is_empty() {
            return Err(
                TuneWeaveError::invalid_request("audio fingerprint cannot be empty")
                    .with_platform(Platform::Netease),
            );
        }
        if fingerprint.len() > 131_072 {
            return Err(TuneWeaveError::invalid_request(
                "audio fingerprint cannot exceed 131072 bytes",
            )
            .with_platform(Platform::Netease));
        }
        if !(1..=300).contains(&request.duration_seconds) {
            return Err(TuneWeaveError::invalid_request(
                "audio fingerprint duration must be between 1 and 300 seconds",
            )
            .with_platform(Platform::Netease));
        }
        let client = self.client_for(request.account.as_deref())?;
        let response = client
            .match_audio(fingerprint, request.duration_seconds)
            .await?;
        ensure_success(&response.body)?;
        let raw_response = response.body.clone();
        let response: AudioMatchEnvelope = parse_body(response.body)?;
        map_audio_recognition(response, raw_response)
    }

    async fn banners(&self, request: &BannerListRequest) -> Result<Vec<Banner>> {
        let (path, payload, protocol) = netease_banner_request(request)?;
        let client = self.client_for(request.account.as_deref())?;
        let response = match protocol {
            NeteaseBannerProtocol::Eapi => client.request_eapi(path, payload).await?,
            NeteaseBannerProtocol::Weapi => client.request_weapi(path, payload).await?,
        };
        ensure_success(&response.body)?;
        match request.catalog {
            BannerCatalog::Music => {
                let response: BannerEnvelope = parse_body(response.body)?;
                response
                    .banners
                    .into_iter()
                    .map(|banner| map_banner(banner, request.client))
                    .collect()
            }
            BannerCatalog::Podcast => {
                let body = response.body;
                let banners = body
                    .get("data")
                    .and_then(Value::as_array)
                    .cloned()
                    .ok_or_else(|| {
                        TuneWeaveError::new(
                            ErrorCode::UpstreamError,
                            "NetEase podcast banner response did not contain its data array",
                        )
                        .with_platform(Platform::Netease)
                        .with_details(json!({ "response": body.clone() }))
                    })?;
                banners.into_iter().map(map_podcast_banner).collect()
            }
        }
    }

    async fn radio_taxonomy(&self, request: &RadioTaxonomyRequest) -> Result<RadioTaxonomy> {
        let client = self.client_for(request.account.as_deref())?;
        let response = client
            .request_eapi("/api/voice/broadcast/category/region/get", json!({}))
            .await?;
        ensure_success(&response.body)?;
        let raw_response = response.body.clone();
        let response: BroadcastTaxonomyEnvelope = parse_body(response.body)?;
        let categories = response
            .data
            .categories
            .into_iter()
            .map(|option| map_radio_catalog_option(option, "category"))
            .collect::<Result<Vec<_>>>()?;
        let regions = response
            .data
            .regions
            .into_iter()
            .map(|option| map_radio_catalog_option(option, "region"))
            .collect::<Result<Vec<_>>>()?;
        let mut extensions = Extensions::new();
        extensions.insert("response".to_owned(), raw_response);
        Ok(RadioTaxonomy {
            categories,
            regions,
            extensions,
        })
    }

    async fn radio_station(&self, id: &str, account: Option<&str>) -> Result<RadioStation> {
        let id = parse_numeric_id("broadcast station", id)?;
        let client = self.client_for(account)?;
        let response = client
            .request_eapi(
                "/api/voice/broadcast/channel/currentinfo",
                json!({ "channelId": id }),
            )
            .await?;
        ensure_success(&response.body)?;
        map_radio_station_response(response.body)
    }

    async fn radio_stations(
        &self,
        request: &RadioStationListRequest,
    ) -> Result<Page<RadioStation>> {
        let payload = netease_radio_station_list_payload(request)?;
        let client = self.client_for(request.account.as_deref())?;
        let response = client
            .request_eapi("/api/voice/broadcast/channel/list", payload)
            .await?;
        ensure_success(&response.body)?;
        map_radio_station_list_response(response.body, request)
    }

    async fn set_radio_station_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        let id = parse_numeric_id("broadcast station", id)?;
        let client = self.client_for(account)?;
        let response = client
            .request_eapi(
                "/api/content/interact/collect",
                netease_radio_station_subscription_payload(id, subscribed),
            )
            .await?;
        ensure_account_access(&client, &response.body, "broadcast station subscription")?;
        map_radio_station_subscription_result(id, subscribed, response.body)
    }

    async fn podcast_categories(&self, account: Option<&str>) -> Result<PodcastTaxonomy> {
        let client = self.client_for(account)?;
        let (path, payload) = netease_podcast_categories_request();
        let response = client.request_weapi(path, payload).await?;
        map_netease_podcast_categories(response.body)
    }

    async fn podcasts(&self, request: &PodcastListRequest) -> Result<Page<Podcast>> {
        let (path, payload) = netease_podcast_catalog_request(request)?;
        let client = self.client_for(request.account.as_deref())?;
        let response = client.request_weapi(path, payload).await?;
        map_netease_podcast_catalog_response(response.body, request)
    }

    async fn podcast(&self, id: &str, account: Option<&str>) -> Result<Podcast> {
        let id = parse_numeric_id("podcast", id)?;
        let client = self.client_for(account)?;
        let (path, payload) = netease_podcast_request(id);
        let response = client.request_weapi(path, payload).await?;
        map_netease_podcast_response(response.body)
    }

    async fn podcast_workbench(&self, id: &str, account: Option<&str>) -> Result<Podcast> {
        let id = parse_numeric_id("podcast voice list", id)?;
        let client = self.client_for(account)?;
        require_authenticated_client(&client, "podcast voice list detail")?;
        let (path, payload) = netease_podcast_workbench_request(id);
        let response = client.request_eapi(path, payload).await?;
        map_netease_podcast_workbench_response(response.body)
    }

    async fn podcast_chart(
        &self,
        request: &PodcastChartRequest,
    ) -> Result<Page<PodcastChartEntry>> {
        let (path, payload) = netease_podcast_chart_request(request)?;
        let client = self.client_for(request.account.as_deref())?;
        let response = client.request_weapi(path, payload).await?;
        map_netease_podcast_chart(response.body, request)
    }

    async fn podcast_creator_chart(
        &self,
        request: &PodcastCreatorChartRequest,
    ) -> Result<Page<PodcastCreatorChartEntry>> {
        let (path, payload) = netease_podcast_creator_chart_request(request)?;
        let client = self.client_for(request.account.as_deref())?;
        let response = client.request_weapi(path, payload).await?;
        map_netease_podcast_creator_chart(response.body, request)
    }

    async fn set_podcast_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        let id = parse_numeric_id("podcast", id)?;
        let client = self.client_for(account)?;
        let (path, payload) = netease_podcast_subscription_request(id, subscribed);
        let response = client.request_weapi(path, payload).await?;
        ensure_account_access(&client, &response.body, "podcast subscription")?;
        map_podcast_subscription_result(id, subscribed, response.body)
    }

    async fn podcast_episodes(
        &self,
        id: &str,
        request: &PodcastEpisodeListRequest,
    ) -> Result<Page<PodcastEpisode>> {
        let id = parse_numeric_id("podcast", id)?;
        let limit = request.limit.clamp(1, 100);
        let client = self.client_for(request.account.as_deref())?;
        let (path, payload) = netease_podcast_episodes_request(id, request, limit);
        let response = client.request_weapi(path, payload).await?;
        map_netease_podcast_episodes(response.body, limit, request.offset)
    }

    async fn podcast_episode_chart(
        &self,
        request: &PodcastEpisodeChartRequest,
    ) -> Result<Page<PodcastEpisodeChartEntry>> {
        let (path, payload) = netease_podcast_episode_chart_request(request)?;
        let client = self.client_for(request.account.as_deref())?;
        let response = client.request_weapi(path, payload).await?;
        map_netease_podcast_episode_chart(response.body, request)
    }

    async fn podcast_episode(&self, id: &str, account: Option<&str>) -> Result<PodcastEpisode> {
        let id = parse_numeric_id("podcast episode", id)?;
        let client = self.client_for(account)?;
        let (path, payload) = netease_podcast_episode_request(id);
        let response = client.request_weapi(path, payload).await?;
        map_netease_podcast_episode_response(response.body)
    }

    async fn podcast_episode_workbench(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<PodcastEpisode> {
        let id = parse_numeric_id("podcast voice", id)?;
        let client = self.client_for(account)?;
        require_authenticated_client(&client, "podcast voice detail")?;
        let (path, payload) = netease_podcast_episode_workbench_request(id);
        let response = client.request_eapi(path, payload).await?;
        map_netease_podcast_episode_workbench_response(response.body)
    }

    async fn podcast_episode_stream(
        &self,
        id: &str,
        request: &StreamRequest,
    ) -> Result<PodcastEpisodeStream> {
        let episode = self.podcast_episode(id, request.account.as_deref()).await?;
        let stream = {
            let audio = episode.audio.as_ref().ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "NetEase podcast episode did not expose a playable audio track",
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "episode_ref": episode.resource_ref.to_string() }))
            })?;
            self.stream(audio, request).await?
        };
        map_netease_podcast_episode_stream(episode, stream)
    }

    async fn podcast_episode_lyrics(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<PodcastEpisodeLyrics> {
        let id = parse_numeric_id("podcast episode", id)?;
        let episode = self.podcast_episode(&id.to_string(), account).await?;
        let client = self.client_for(account)?;
        let (path, payload) = netease_podcast_episode_lyrics_request(id);
        let response = client.request_eapi(path, payload).await?;
        ensure_success(&response.body)?;
        let document = match response.body.get("data") {
            Some(Value::Null) => None,
            Some(data) if data.is_object() => {
                let url = data
                    .get("lyricUrl")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|url| !url.is_empty())
                    .ok_or_else(|| podcast_lyrics_error("lyric URL", &response.body))?;
                Some(client.fetch_voice_lyric_document(url).await?)
            }
            _ => return Err(podcast_lyrics_error("data object or null", &response.body)),
        };
        map_netease_podcast_episode_lyrics(episode, response.body, document)
    }

    async fn track(&self, id: &str, account: Option<&str>) -> Result<Track> {
        let id = parse_numeric_id("track", id)?;
        let client = self.client_for(account)?;
        let response = client
            .request_eapi(
                "/api/v3/song/detail",
                json!({
                    "c": format!(r#"[{{"id":{id}}}]"#)
                }),
            )
            .await?;
        ensure_success(&response.body)?;
        let response: TrackEnvelope = parse_body(response.body)?;
        let mut privileges = response
            .privileges
            .into_iter()
            .map(|privilege| (privilege.id, privilege))
            .collect::<HashMap<_, _>>();
        let song = response.songs.into_iter().next().ok_or_else(|| {
            TuneWeaveError::new(ErrorCode::ResourceNotFound, "NetEase track was not found")
                .with_platform(Platform::Netease)
                .with_details(json!({ "id": id }))
        })?;
        let privilege = privileges.remove(&song.id);
        map_song(song, privilege)
    }

    async fn track_availability(
        &self,
        id: &str,
        request: &TrackAvailabilityRequest,
    ) -> Result<TrackAvailability> {
        let id = parse_numeric_id("track", id)?;
        let client = self.client_for(request.account.as_deref())?;
        let response = client
            .request_weapi(
                "/api/song/enhance/player/url",
                json!({
                    "ids": format!("[{id}]"),
                    "br": request.bitrate
                }),
            )
            .await?;
        map_track_availability(id, request.bitrate, response.body)
    }

    async fn album(&self, id: &str, account: Option<&str>) -> Result<Album> {
        let id = parse_numeric_id("album", id)?;
        let client = self.client_for(account)?;
        let response = fetch_album_content(&client, id).await?;
        map_album(response.album)
    }

    async fn album_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        let id = parse_numeric_id("album", id)?;
        let client = self.client_for(request.account.as_deref())?;
        let response = fetch_album_content(&client, id).await?;
        let limit = request.limit.clamp(1, 100);
        let (songs, pagination) = select_page(response.songs, limit, request.offset);
        let items = songs
            .into_iter()
            .map(|song| map_song(song, None))
            .collect::<Result<Vec<_>>>()?;
        Ok(Page { items, pagination })
    }

    async fn albums(&self, request: &AlbumListRequest) -> Result<Page<Album>> {
        let limit = request.limit.clamp(1, 100);
        let catalog = AlbumCatalog::parse(request.catalog.as_deref())?;
        let client = self.client_for(request.account.as_deref())?;
        let (path, payload) = match catalog {
            AlbumCatalog::New => (
                "/api/album/new",
                json!({
                    "limit": limit,
                    "offset": request.offset,
                    "total": true,
                    "area": normalize_album_area(request.area.as_deref())?
                }),
            ),
            AlbumCatalog::Newest => {
                if request
                    .area
                    .as_deref()
                    .map(str::trim)
                    .is_some_and(|area| !area.is_empty())
                {
                    return Err(TuneWeaveError::invalid_request(
                        "area is not supported for the NetEase newest album catalog",
                    )
                    .with_platform(Platform::Netease));
                }
                ("/api/discovery/newAlbum", json!({}))
            }
        };
        let response = client.request_weapi(path, payload).await?;
        ensure_success(&response.body)?;
        let response: AlbumListEnvelope = parse_body(response.body)?;
        let items = response
            .albums
            .into_iter()
            .map(map_album_list_item)
            .collect::<Result<Vec<_>>>()?;
        if catalog == AlbumCatalog::Newest {
            let (items, pagination) = select_page(items, limit, request.offset);
            return Ok(Page { items, pagination });
        }
        let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let next_offset = request.offset.saturating_add(consumed);
        let has_more = response
            .total
            .map_or(consumed == limit, |total| u64::from(next_offset) < total);
        Ok(Page {
            items,
            pagination: PageMeta {
                limit,
                offset: request.offset,
                total: response.total,
                next_offset: has_more.then_some(next_offset),
                has_more,
                extensions: Default::default(),
            },
        })
    }

    async fn album_stats(&self, id: &str, account: Option<&str>) -> Result<AlbumStats> {
        let id = parse_numeric_id("album", id)?;
        let client = self.client_for(account)?;
        let response = client
            .request_weapi("/api/album/detail/dynamic", json!({ "id": id }))
            .await?;
        ensure_success(&response.body)?;
        let response: AlbumStatsEnvelope = parse_body(response.body)?;
        map_album_stats(id, response)
    }

    async fn album_track_entitlements(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<TrackEntitlement>> {
        let id = parse_numeric_id("album", id)?;
        let client = self.client_for(request.account.as_deref())?;
        let response = client
            .request_eapi("/api/album/privilege", json!({ "id": id }))
            .await?;
        ensure_success(&response.body)?;
        let response: AlbumEntitlementsEnvelope = parse_body(response.body)?;
        let limit = request.limit.clamp(1, 100);
        let (items, pagination) = select_page(response.data, limit, request.offset);
        let items = items
            .into_iter()
            .map(map_track_entitlement)
            .collect::<Result<Vec<_>>>()?;
        Ok(Page { items, pagination })
    }

    async fn set_album_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        let id = parse_numeric_id("album", id)?;
        let (path, payload) = netease_album_subscription_request(id, subscribed);
        let client = self.client_for(account)?;
        let response = client.request_weapi(path, payload).await?;
        ensure_success(&response.body)?;
        map_album_subscription_result(id, subscribed, response.body)
    }

    async fn digital_album(&self, id: &str, account: Option<&str>) -> Result<DigitalAlbum> {
        let id = parse_numeric_id("digital album", id)?;
        let client = self.client_for(account)?;
        let response = client
            .request_weapi("/api/vipmall/albumproduct/detail", json!({ "id": id }))
            .await?;
        ensure_success(&response.body)?;
        let raw = response.body;
        let response: DigitalAlbumEnvelope = parse_body(raw.clone())?;
        map_digital_album(response, &raw, id)
    }

    async fn digital_albums(
        &self,
        request: &DigitalAlbumListRequest,
    ) -> Result<Page<DigitalAlbum>> {
        let limit = request.limit.clamp(1, 100);
        let catalog = DigitalAlbumCatalog::parse(request.catalog.as_deref())?;
        let area = normalize_digital_album_area(catalog, request.area.as_deref())?;
        let client = self.client_for(request.account.as_deref())?;
        let mut payload = json!({
            "limit": limit,
            "offset": request.offset,
            "total": true,
            "area": area
        });
        let kind = request
            .kind
            .as_deref()
            .map(str::trim)
            .filter(|kind| !kind.is_empty());
        if catalog == DigitalAlbumCatalog::Style && kind.is_some() {
            return Err(TuneWeaveError::invalid_request(
                "type is not supported for the NetEase style catalog",
            )
            .with_platform(Platform::Netease));
        }
        if let Some(kind) = kind {
            payload["type"] = Value::String(kind.to_owned());
        }
        let response = client.request_weapi(catalog.endpoint(), payload).await?;
        ensure_success(&response.body)?;
        let response: DigitalAlbumListEnvelope = parse_body(response.body)?;
        let products = match catalog {
            DigitalAlbumCatalog::Latest => response.products,
            DigitalAlbumCatalog::Style => response.album_products,
        };
        let items = products
            .into_iter()
            .map(map_digital_album_list_item)
            .collect::<Result<Vec<_>>>()?;
        let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
        let has_more = response.has_next_page.unwrap_or(consumed == limit);
        Ok(Page {
            items,
            pagination: PageMeta {
                limit,
                offset: request.offset,
                total: None,
                next_offset: has_more.then_some(request.offset.saturating_add(consumed)),
                has_more,
                extensions: Default::default(),
            },
        })
    }

    async fn digital_album_chart(
        &self,
        request: &DigitalAlbumChartRequest,
    ) -> Result<Page<DigitalAlbumChartEntry>> {
        let limit = request.limit.clamp(1, 100);
        let (path, payload) = netease_digital_album_chart_request(request)?;
        let client = self.client_for(request.account.as_deref())?;
        let response = client.request_weapi(&path, payload).await?;
        ensure_success(&response.body)?;
        let response: DigitalAlbumChartEnvelope = parse_body(response.body)?;
        let items = response
            .products
            .into_iter()
            .enumerate()
            .map(|(index, raw)| {
                let position = u32::try_from(index).unwrap_or(u32::MAX);
                map_digital_album_chart_entry(raw, position)
            })
            .collect::<Result<Vec<_>>>()?;
        let (items, pagination) = select_page(items, limit, request.offset);
        Ok(Page { items, pagination })
    }

    async fn chart_catalog(&self, request: &ChartCatalogRequest) -> Result<ChartCatalog> {
        let client = self.client_for(request.account.as_deref())?;
        let response = match request.view {
            ChartCatalogView::Overview => client.request_eapi("/api/toplist", json!({})).await?,
            ChartCatalogView::Summary => {
                client
                    .request_weapi("/api/toplist/detail", json!({}))
                    .await?
            }
            ChartCatalogView::Modern => {
                client
                    .request_weapi("/api/toplist/detail/v2", json!({}))
                    .await?
            }
        };
        ensure_success(&response.body)?;
        let raw_response = response.body.clone();
        let response: ChartCatalogEnvelope = parse_body(response.body)?;
        map_chart_catalog(response, request.view, raw_response)
    }

    async fn artist_chart(&self, request: &ArtistChartRequest) -> Result<ArtistChart> {
        let client = self.client_for(request.account.as_deref())?;
        let response = client
            .request_weapi(
                "/api/toplist/artist",
                json!({
                    "type": netease_artist_chart_area(request.area),
                    "limit": 100,
                    "offset": 0,
                    "total": true
                }),
            )
            .await?;
        ensure_success(&response.body)?;
        let raw_response = response.body.clone();
        let response: ArtistChartEnvelope = parse_body(response.body)?;
        map_artist_chart(response, request.area, raw_response)
    }

    async fn dimension_chart(&self, request: &DimensionChartRequest) -> Result<DimensionChart> {
        let payload = netease_dimension_chart_payload(request)?;
        let client = self.client_for(request.account.as_deref())?;
        let response = client.request_eapi("/api/chart/detail", payload).await?;
        ensure_success(&response.body)?;
        let raw_response = response.body.clone();
        let response: DimensionChartDetailEnvelope = parse_body(response.body)?;
        map_dimension_chart(response, request, raw_response)
    }

    async fn dimension_chart_tracks(
        &self,
        request: &DimensionChartRequest,
    ) -> Result<DimensionChartTrackSnapshot> {
        let payload = netease_dimension_chart_payload(request)?;
        let client = self.client_for(request.account.as_deref())?;
        let response = client
            .request_eapi("/api/chart/song/detail", payload)
            .await?;
        ensure_success(&response.body)?;
        let raw_response = response.body.clone();
        let response: DimensionChartTracksEnvelope = parse_body(response.body)?;
        map_dimension_chart_tracks(response, request, raw_response)
    }

    async fn artist(&self, id: &str, account: Option<&str>) -> Result<Artist> {
        let id = parse_numeric_id("artist", id)?;
        let client = self.client_for(account)?;
        let detail_response = client
            .request_eapi("/api/artist/head/info/get", json!({ "id": id }))
            .await?;
        ensure_success(&detail_response.body)?;
        let detail_raw = detail_response.body;
        let detail: ArtistDetailEnvelope = parse_body(detail_raw.clone())?;

        let description_response = client
            .request_weapi("/api/artist/introduction", json!({ "id": id }))
            .await?;
        ensure_success(&description_response.body)?;
        let description_raw = description_response.body;
        let description: ArtistDescriptionEnvelope = parse_body(description_raw.clone())?;

        map_artist(detail, description, detail_raw, description_raw)
    }

    async fn artist_overview(&self, id: &str, account: Option<&str>) -> Result<ArtistOverview> {
        let id = parse_numeric_id("artist", id)?;
        let client = self.client_for(account)?;
        let response = client
            .request_weapi(&format!("/api/v1/artist/{id}"), json!({}))
            .await?;
        ensure_success(&response.body)?;
        let raw_response = response.body.clone();
        let response: ArtistOverviewEnvelope = parse_body(response.body)?;
        map_artist_overview(response, raw_response)
    }

    async fn artist_stats(&self, id: &str, account: Option<&str>) -> Result<ArtistStats> {
        let id = parse_numeric_id("artist", id)?;
        let client = self.client_for(account)?;
        let response = client
            .request_eapi("/api/artist/detail/dynamic", json!({ "id": id }))
            .await?;
        ensure_success(&response.body)?;
        let raw = response.body;
        let response: ArtistDynamicEnvelope = parse_body(raw.clone())?;
        let follow_count_response = client
            .request_weapi("/api/artist/follow/count/get", json!({ "id": id }))
            .await?;
        ensure_success(&follow_count_response.body)?;
        let follow_count_raw = follow_count_response.body;
        let follow_count: ArtistFollowCountEnvelope = parse_body(follow_count_raw.clone())?;
        map_artist_stats(id, response, raw, follow_count, follow_count_raw)
    }

    async fn artists(&self, request: &ArtistListRequest) -> Result<Page<Artist>> {
        let limit = request.limit.clamp(1, 100);
        let client = self.client_for(request.account.as_deref())?;
        let mut payload = json!({
            "limit": limit,
            "offset": request.offset,
            "total": true,
            "type": netease_artist_category(request.category),
            "area": netease_artist_area(request.area)
        });
        if let Some(initial) = netease_artist_initial(request.initial.as_deref())? {
            payload["initial"] = Value::from(initial);
        }
        let response = client.request_weapi("/api/v1/artist/list", payload).await?;
        ensure_success(&response.body)?;
        let response: ArtistListEnvelope = parse_body(response.body)?;
        map_artist_list_response(response, limit, request.offset)
    }

    async fn artist_albums(&self, id: &str, request: &PageRequest) -> Result<Page<Album>> {
        let id = parse_numeric_id("artist", id)?;
        let limit = request.limit.clamp(1, 100);
        let client = self.client_for(request.account.as_deref())?;
        let response = client
            .request_weapi(
                &format!("/api/artist/albums/{id}"),
                json!({
                    "limit": limit,
                    "offset": request.offset,
                    "total": true
                }),
            )
            .await?;
        ensure_success(&response.body)?;
        let response: ArtistAlbumsEnvelope = parse_body(response.body)?;
        map_artist_albums_response(response, limit, request.offset)
    }

    async fn artist_fans(&self, id: &str, request: &PageRequest) -> Result<Page<User>> {
        let id = parse_numeric_id("artist", id)?;
        let limit = request.limit.clamp(1, 100);
        let client = self.client_for(request.account.as_deref())?;
        let response = client
            .request_weapi(
                "/api/artist/fans/get",
                json!({
                    "id": id,
                    "limit": limit,
                    "offset": request.offset
                }),
            )
            .await?;
        ensure_success(&response.body)?;
        let response: ArtistFansEnvelope = parse_body(response.body)?;
        map_artist_fans_response(response, limit, request.offset)
    }

    async fn artist_videos(
        &self,
        id: &str,
        request: &ArtistVideoListRequest,
    ) -> Result<Page<Video>> {
        let id = parse_numeric_id("artist", id)?;
        let limit = request.limit.clamp(1, 100);
        let client = self.client_for(request.account.as_deref())?;
        match request.kind {
            VideoKind::Mv => {
                if request.cursor.is_some() || request.order.is_some() {
                    return Err(TuneWeaveError::invalid_request(
                        "cursor and order are not supported for the NetEase MV catalog",
                    )
                    .with_platform(Platform::Netease));
                }
                let response = client
                    .request_weapi(
                        "/api/artist/mvs",
                        json!({
                            "artistId": id,
                            "limit": limit,
                            "offset": request.offset,
                            "total": true
                        }),
                    )
                    .await?;
                ensure_success(&response.body)?;
                let response: ArtistMvsEnvelope = parse_body(response.body)?;
                map_artist_mvs_response(response, limit, request.offset)
            }
            VideoKind::All => {
                let cursor = request
                    .cursor
                    .as_deref()
                    .map_or_else(|| json!(request.offset), |cursor| json!(cursor));
                let order = request
                    .order
                    .as_deref()
                    .map_or_else(|| json!(0), |order| json!(order));
                let response = client
                    .request_weapi(
                        "/api/mlog/artist/video",
                        json!({
                            "artistId": id,
                            "page": json!({ "size": limit, "cursor": cursor }).to_string(),
                            "tab": 0,
                            "order": order
                        }),
                    )
                    .await?;
                ensure_success(&response.body)?;
                let response: ArtistVideosEnvelope = parse_body(response.body)?;
                map_artist_videos_response(response, limit, request.offset)
            }
        }
    }

    async fn video(&self, id: &str, request: &VideoDetailRequest) -> Result<VideoDetail> {
        let id = validated_video_id(id, request.kind)?;
        let client = self.client_for(request.account.as_deref())?;
        match request.kind {
            VideoResourceKind::Mv => {
                let response = client
                    .request_weapi("/api/v1/mv/detail", json!({ "id": id }))
                    .await?;
                ensure_success(&response.body)?;
                let raw_response = response.body.clone();
                let response: MvDetailEnvelope = parse_body(response.body)?;
                map_mv_detail(response, raw_response)
            }
            VideoResourceKind::Video => {
                let response = client
                    .request_weapi("/api/cloudvideo/v1/video/detail", json!({ "id": id }))
                    .await?;
                ensure_success(&response.body)?;
                let raw_response = response.body.clone();
                let response: CloudVideoDetailEnvelope = parse_body(response.body)?;
                map_cloud_video_detail(response, raw_response)
            }
        }
    }

    async fn video_stats(&self, id: &str, request: &VideoDetailRequest) -> Result<VideoStats> {
        let id = validated_video_id(id, request.kind)?;
        let thread_id = match request.kind {
            VideoResourceKind::Mv => format!("R_MV_5_{id}"),
            VideoResourceKind::Video => format!("R_VI_62_{id}"),
        };
        let client = self.client_for(request.account.as_deref())?;
        let response = client
            .request_weapi(
                "/api/comment/commentthread/info",
                json!({ "threadid": thread_id, "composeliked": true }),
            )
            .await?;
        ensure_success(&response.body)?;
        let raw_response = response.body.clone();
        let response: VideoStatsEnvelope = parse_body(response.body)?;
        map_video_stats(&id, request.kind, response, raw_response)
    }

    async fn video_stream(&self, id: &str, request: &VideoStreamRequest) -> Result<VideoStream> {
        let id = validated_video_id(id, request.kind)?;
        let resolution = validated_video_resolution(request.resolution)?;
        let client = self.client_for(request.account.as_deref())?;
        match request.kind {
            VideoResourceKind::Mv => {
                let response = client
                    .request_weapi(
                        "/api/song/enhance/play/mv/url",
                        json!({ "id": id, "r": resolution }),
                    )
                    .await?;
                ensure_success(&response.body)?;
                let raw_response = response.body.clone();
                let response: MvUrlEnvelope = parse_body(response.body)?;
                map_video_stream(&id, request, response.data, raw_response)
            }
            VideoResourceKind::Video => {
                let ids = serde_json::to_string(&[id.as_str()]).map_err(|error| {
                    TuneWeaveError::new(
                        ErrorCode::InternalError,
                        format!("failed to serialize NetEase video ids: {error}"),
                    )
                    .with_platform(Platform::Netease)
                })?;
                let response = client
                    .request_weapi(
                        "/api/cloudvideo/playurl",
                        json!({ "ids": ids, "resolution": resolution }),
                    )
                    .await?;
                ensure_success(&response.body)?;
                let raw_response = response.body.clone();
                let response: CloudVideoUrlEnvelope = parse_body(response.body)?;
                let item = response.urls.into_iter().next().unwrap_or(VideoUrlItem {
                    id: Some(Value::String(id.clone())),
                    url: None,
                    size: None,
                    validity: None,
                    expi: None,
                    r: None,
                    resolution: None,
                    code: None,
                    fee: None,
                    mv_fee: None,
                    msg: None,
                    md5: None,
                });
                map_video_stream(&id, request, item, raw_response)
            }
        }
    }

    async fn artist_tracks(
        &self,
        id: &str,
        request: &ArtistTrackListRequest,
    ) -> Result<Page<Track>> {
        let id = parse_numeric_id("artist", id)?;
        let limit = request.limit.clamp(1, 100);
        let client = self.client_for(request.account.as_deref())?;
        let order = match request.order {
            ArtistTrackOrder::Hot => "hot",
            ArtistTrackOrder::Time => "time",
        };
        let response = client
            .request_api(
                "/api/v1/artist/songs",
                json!({
                    "id": id,
                    "private_cloud": "true",
                    "work_type": 1,
                    "order": order,
                    "offset": request.offset,
                    "limit": limit
                }),
            )
            .await?;
        ensure_success(&response.body)?;
        let response: ArtistTracksEnvelope = parse_body(response.body)?;
        map_artist_tracks_response(response, limit, request.offset)
    }

    async fn artist_top_tracks(&self, id: &str, account: Option<&str>) -> Result<Page<Track>> {
        let id = parse_numeric_id("artist", id)?;
        let client = self.client_for(account)?;
        let response = client
            .request_weapi("/api/artist/top/song", json!({ "id": id }))
            .await?;
        ensure_success(&response.body)?;
        let raw_response = response.body.clone();
        let response: ArtistTopTracksEnvelope = parse_body(response.body)?;
        map_artist_top_tracks_response(response, raw_response)
    }

    async fn set_artist_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        let id = parse_numeric_id("artist", id)?;
        let (path, payload) = netease_artist_subscription_request(id, subscribed);
        let client = self.client_for(account)?;
        let response = client.request_weapi(path, payload).await?;
        ensure_success(&response.body)?;
        map_artist_subscription_result(id, subscribed, response.body)
    }

    async fn playlist(&self, id: &str, account: Option<&str>) -> Result<Playlist> {
        let id = parse_numeric_id("playlist", id)?;
        let client = self.client_for(account)?;
        map_playlist(self.playlist_detail(&client, id).await?)
    }

    async fn playlist_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        let id = parse_numeric_id("playlist", id)?;
        let client = self.client_for(request.account.as_deref())?;
        let playlist = self.playlist_detail(&client, id).await?;
        let total = playlist.track_ids.len() as u64;
        let limit = request.limit.clamp(1, 100);
        let offset = request.offset;
        let selected_ids = playlist
            .track_ids
            .into_iter()
            .skip(offset as usize)
            .take(limit as usize)
            .map(|track| track.id)
            .collect::<Vec<_>>();
        let items = fetch_tracks_by_ids(&client, &selected_ids).await?;
        let consumed = selected_ids.len() as u32;
        let next_offset = offset.saturating_add(consumed);
        let has_more = u64::from(next_offset) < total;

        Ok(Page {
            items,
            pagination: PageMeta {
                limit,
                offset,
                total: Some(total),
                next_offset: has_more.then_some(next_offset),
                has_more,
                extensions: Default::default(),
            },
        })
    }

    async fn create_playlist(
        &self,
        request: &PlaylistCreateRequest,
    ) -> Result<PlaylistMutationResult> {
        let payload = netease_playlist_create_payload(request)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "playlist creation")?;
        let response = client
            .request_weapi("/api/playlist/create", payload)
            .await?;
        ensure_account_access(&client, &response.body, "playlist creation")?;
        map_netease_playlist_create_result(response.body)
    }

    async fn update_playlist(
        &self,
        id: &str,
        request: &PlaylistUpdateRequest,
    ) -> Result<PlaylistMutationResult> {
        let id = parse_numeric_id("playlist", id)?;
        let calls = netease_playlist_update_calls(id, request)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "playlist update")?;
        let mut responses = Vec::with_capacity(calls.len());
        for (path, payload) in calls {
            let response = client.request_api(path, payload).await?;
            ensure_account_access(&client, &response.body, "playlist update")?;
            responses.push(json!({ "path": path, "body": response.body }));
        }
        map_netease_playlist_mutation_result(
            id,
            PlaylistMutationAction::Update,
            json!({ "responses": responses }),
        )
    }

    async fn delete_playlists(
        &self,
        request: &PlaylistDeleteRequest,
    ) -> Result<PlaylistDeleteResult> {
        let ids = netease_playlist_ids("playlist deletion", &request.playlist_refs)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "playlist deletion")?;
        let response = client
            .request_weapi(
                "/api/playlist/remove",
                json!({ "ids": format!("[{}]", join_numeric_ids(&ids)) }),
            )
            .await?;
        ensure_account_access(&client, &response.body, "playlist deletion")?;
        Ok(PlaylistDeleteResult {
            playlist_refs: request.playlist_refs.clone(),
            extensions: Extensions::from([("response".to_owned(), response.body)]),
        })
    }

    async fn mutate_playlist_items(
        &self,
        id: &str,
        action: PlaylistItemMutationAction,
        request: &PlaylistItemMutationRequest,
    ) -> Result<PlaylistItemMutationResult> {
        let playlist_id = parse_numeric_id("playlist", id)?;
        let item_ids = netease_playlist_item_ids(request)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "playlist item update")?;
        match request.kind {
            PlaylistItemKind::Track => {
                let payload =
                    netease_playlist_track_mutation_payload(playlist_id, action, &item_ids);
                let initial = client
                    .request_api("/api/playlist/manipulate/tracks", payload)
                    .await?;
                if json_i64(&initial.body["code"]) == Some(512) {
                    let mut duplicated = item_ids.clone();
                    duplicated.extend(item_ids.iter().cloned());
                    let retry = client
                        .request_api(
                            "/api/playlist/manipulate/tracks",
                            netease_playlist_track_mutation_payload(
                                playlist_id,
                                action,
                                &duplicated,
                            ),
                        )
                        .await?;
                    ensure_account_access(&client, &retry.body, "playlist track update")?;
                    return map_netease_playlist_item_mutation_result(
                        playlist_id,
                        action,
                        request,
                        retry.body,
                        Some(initial.body),
                    );
                }
                ensure_account_access(&client, &initial.body, "playlist track update")?;
                map_netease_playlist_item_mutation_result(
                    playlist_id,
                    action,
                    request,
                    initial.body,
                    None,
                )
            }
            PlaylistItemKind::Video => {
                let (path, payload) =
                    netease_playlist_video_mutation_request(playlist_id, action, &item_ids);
                let response = client.request_weapi(path, payload).await?;
                ensure_account_access(&client, &response.body, "video playlist update")?;
                map_netease_playlist_item_mutation_result(
                    playlist_id,
                    action,
                    request,
                    response.body,
                    None,
                )
            }
        }
    }

    async fn reorder_playlist_tracks(
        &self,
        id: &str,
        request: &PlaylistTrackOrderRequest,
    ) -> Result<PlaylistTrackOrderResult> {
        let playlist_id = parse_numeric_id("playlist", id)?;
        let track_ids = netease_playlist_track_ids("playlist track order", &request.track_refs)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "playlist track order update")?;
        let (path, payload) = netease_playlist_track_order_request(playlist_id, &track_ids);
        let response = client.request_api(path, payload).await?;
        ensure_account_access(&client, &response.body, "playlist track order update")?;
        map_netease_playlist_track_order_result(
            playlist_id,
            request.track_refs.clone(),
            response.body,
        )
    }

    async fn reorder_account_playlists(
        &self,
        request: &PlaylistOrderRequest,
    ) -> Result<PlaylistOrderResult> {
        let ids = netease_playlist_ids("playlist order", &request.playlist_refs)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "playlist order update")?;
        let (path, payload) = netease_account_playlist_order_request(&ids);
        let response = client.request_weapi(path, payload).await?;
        ensure_account_access(&client, &response.body, "playlist order update")?;
        Ok(PlaylistOrderResult {
            playlist_refs: request.playlist_refs.clone(),
            extensions: Extensions::from([("response".to_owned(), response.body)]),
        })
    }

    async fn update_playlist_cover(
        &self,
        id: &str,
        request: &ImageUploadRequest,
    ) -> Result<PlaylistCoverUpdateResult> {
        let id = parse_numeric_id("playlist", id)?;
        let (client, allocation, upload_response) = self
            .upload_image_blob(request, "playlist cover update")
            .await?;
        let (path, payload) =
            netease_playlist_cover_update_request(id, &allocation.result.document_id);
        let update_response = client.request_weapi(path, payload).await?;
        ensure_account_access(&client, &update_response.body, "playlist cover update")?;
        let response = update_response.body;
        let image =
            map_image_upload_result(request, allocation, upload_response, response.clone())?;
        let playlist_ref =
            ResourceRef::new(Platform::Netease, id.to_string()).map_err(|error| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    format!("NetEase returned an invalid playlist id: {error}"),
                )
                .with_platform(Platform::Netease)
            })?;
        Ok(PlaylistCoverUpdateResult {
            playlist_ref,
            image,
            extensions: Extensions::from([("response".to_owned(), response)]),
        })
    }

    async fn account_playlists(&self, request: &PageRequest) -> Result<Page<Playlist>> {
        let account = request.account.as_deref().unwrap_or("default");
        let client = self.client_for(Some(account))?;
        let user_id = authenticated_user_id(&client, account).await?;
        let limit = request.limit.clamp(1, 100);
        let response = client
            .request_weapi(
                "/api/user/playlist",
                json!({
                    "uid": user_id,
                    "limit": limit,
                    "offset": request.offset,
                    "includeVideo": true
                }),
            )
            .await?;
        ensure_success(&response.body)?;
        let response: UserPlaylistsEnvelope = parse_body(response.body)?;
        map_user_playlists(response, limit, request.offset)
    }

    async fn account_albums(&self, request: &PageRequest) -> Result<Page<Album>> {
        let account = request.account.as_deref().unwrap_or("default");
        let client = self.client_for(Some(account))?;
        let limit = request.limit.clamp(1, 100);
        let response = client
            .request_weapi(
                "/api/album/sublist",
                json!({
                    "limit": limit,
                    "offset": request.offset,
                    "total": true
                }),
            )
            .await?;
        ensure_success(&response.body)?;
        map_subscribed_albums_response(response.body, request, limit)
    }

    async fn account_radio_stations(&self, request: &PageRequest) -> Result<Page<RadioStation>> {
        let account = request.account.as_deref().unwrap_or("default");
        let client = self.client_for(Some(account))?;
        let limit = request.limit.max(1);
        let response = client
            .request_eapi(
                "/api/content/channel/collect/list",
                netease_radio_collection_payload(limit, request.offset),
            )
            .await?;
        ensure_account_access(&client, &response.body, "broadcast station collection")?;
        map_radio_collection_response(response.body, limit, request.offset)
    }

    async fn account_podcasts(&self, request: &PageRequest) -> Result<Page<Podcast>> {
        let account = request.account.as_deref().unwrap_or("default");
        let client = self.client_for(Some(account))?;
        let limit = request.limit.clamp(1, 100);
        let (path, payload) = netease_subscribed_podcasts_request(limit, request.offset);
        let response = client.request_weapi(path, payload).await?;
        ensure_account_access(&client, &response.body, "subscribed podcast catalog")?;
        map_netease_subscribed_podcasts(response.body, limit, request.offset)
    }

    async fn account_following_artists(&self, request: &PageRequest) -> Result<Page<Artist>> {
        let account = request.account.as_deref().unwrap_or("default");
        let client = self.client_for(Some(account))?;
        let limit = request.limit.clamp(1, 100);
        let response = client
            .request_weapi(
                "/api/artist/sublist",
                json!({
                    "limit": limit,
                    "offset": request.offset,
                    "total": true
                }),
            )
            .await?;
        ensure_account_access(&client, &response.body, "followed artist catalog")?;
        let response: ArtistSublistEnvelope = parse_body(response.body)?;
        map_artist_sublist_response(response, limit, request.offset)
    }

    async fn account_artist_new_videos(
        &self,
        request: &ArtistUpdatesRequest,
    ) -> Result<Page<Video>> {
        let account = request.account.as_deref().unwrap_or("default");
        let client = self.client_for(Some(account))?;
        let limit = request.limit.clamp(1, 100);
        let before_ms = match request.before_ms {
            Some(before_ms) => before_ms,
            None => unix_millis_now()?,
        };
        let response = client
            .request_weapi(
                "/api/sub/artist/new/works/mv/list",
                json!({
                    "limit": limit,
                    "startTimestamp": before_ms
                }),
            )
            .await?;
        ensure_account_access(&client, &response.body, "followed artist new videos")?;
        let raw_response = response.body.clone();
        let response: ArtistNewVideosEnvelope = parse_body(response.body)?;
        map_artist_new_videos_response(response, raw_response, limit, before_ms)
    }

    async fn account_artist_new_tracks(
        &self,
        request: &ArtistUpdatesRequest,
    ) -> Result<Page<Track>> {
        let account = request.account.as_deref().unwrap_or("default");
        let client = self.client_for(Some(account))?;
        let limit = request.limit.clamp(1, 100);
        let before_ms = match request.before_ms {
            Some(before_ms) => before_ms,
            None => unix_millis_now()?,
        };
        let response = client
            .request_weapi(
                "/api/sub/artist/new/works/song/list",
                json!({
                    "limit": limit,
                    "startTimestamp": before_ms
                }),
            )
            .await?;
        ensure_account_access(&client, &response.body, "followed artist new tracks")?;
        let raw_response = response.body.clone();
        let response: ArtistNewTracksEnvelope = parse_body(response.body)?;
        map_artist_new_tracks_response(response, raw_response, limit, before_ms)
    }

    async fn account_artist_new_works(
        &self,
        request: &ArtistWorksRequest,
    ) -> Result<Page<ArtistWorkUpdate>> {
        let account = request.account.as_deref().unwrap_or("default");
        let client = self.client_for(Some(account))?;
        let limit = request.limit.clamp(1, 100);
        let before_ms = match request.before_ms {
            Some(before_ms) => before_ms,
            None => unix_millis_now()?,
        };
        let response = client
            .request_eapi(
                "/api/sub/artist/new/works/song-mv/list/v2",
                json!({
                    "startTimestamp": before_ms,
                    "sourceType": request.source_type,
                    "limit": limit,
                    "firstRequest": request.first_request
                }),
            )
            .await?;
        ensure_account_access(&client, &response.body, "followed artist new works")?;
        let raw_response = response.body.clone();
        let response: ArtistNewWorksEnvelope = parse_body(response.body)?;
        map_artist_new_works_response(response, raw_response, request, limit, before_ms)
    }

    async fn account_artist_new_tracks_play_all(
        &self,
        account: Option<&str>,
    ) -> Result<Page<Track>> {
        let account = account.unwrap_or("default");
        let client = self.client_for(Some(account))?;
        let response = client
            .request_eapi("/api/sub/artist/new/works/song/playall", json!({}))
            .await?;
        ensure_account_access(
            &client,
            &response.body,
            "followed artist new tracks play-all",
        )?;
        let raw_response = response.body.clone();
        let response: ArtistNewTracksPlayAllEnvelope = parse_body(response.body)?;
        map_artist_new_tracks_play_all_response(response, raw_response)
    }

    async fn favorite_tracks(&self, request: &PageRequest) -> Result<Page<Track>> {
        let account = request.account.as_deref().unwrap_or("default");
        let client = self.client_for(Some(account))?;
        let user_id = authenticated_user_id(&client, account).await?;
        fetch_favorite_tracks(&client, &user_id, request).await
    }

    async fn user_favorite_tracks(
        &self,
        user_id: &str,
        request: &PageRequest,
    ) -> Result<Page<Track>> {
        let user_id = parse_numeric_id("user", user_id)?.to_string();
        let client = self.client_for(request.account.as_deref())?;
        fetch_favorite_tracks(&client, &user_id, request).await
    }

    async fn account_history(
        &self,
        request: &PlaybackHistoryRequest,
    ) -> Result<Page<PlaybackHistoryEntry>> {
        let account = request.account.as_deref().unwrap_or("default");
        let client = self.client_for(Some(account))?;
        let user_id = authenticated_user_id(&client, account).await?;
        fetch_play_history(&client, &user_id, request).await
    }

    async fn user_history(
        &self,
        user_id: &str,
        request: &PlaybackHistoryRequest,
    ) -> Result<Page<PlaybackHistoryEntry>> {
        let user_id = parse_numeric_id("user", user_id)?.to_string();
        let client = self.client_for(request.account.as_deref())?;
        fetch_play_history(&client, &user_id, request).await
    }

    async fn recommended_tracks(&self, request: &RecommendationRequest) -> Result<Page<Track>> {
        let account = request.account.as_deref().unwrap_or("default");
        let client = self.client_for(Some(account))?;
        let response = client
            .request_weapi(
                "/api/v3/discovery/recommend/songs",
                json!({ "afresh": request.refresh }),
            )
            .await?;
        ensure_account_access(&client, &response.body, "daily track recommendations")?;
        let response: RecommendedTracksEnvelope = parse_body(response.body)?;
        map_recommended_tracks(response, request.limit, request.offset)
    }

    async fn recommended_playlists(
        &self,
        request: &RecommendationRequest,
    ) -> Result<Page<Playlist>> {
        let account = request.account.as_deref().unwrap_or("default");
        let client = self.client_for(Some(account))?;
        let response = client
            .request_weapi("/api/v1/discovery/recommend/resource", json!({}))
            .await?;
        ensure_account_access(&client, &response.body, "daily playlist recommendations")?;
        let response: RecommendedPlaylistsEnvelope = parse_body(response.body)?;
        map_recommended_playlists(response, request.limit, request.offset)
    }

    async fn lyrics(&self, id: &str, account: Option<&str>) -> Result<Lyrics> {
        let id = parse_numeric_id("track", id)?;
        let client = self.client_for(account)?;
        let response = client
            .request_eapi(
                "/api/song/lyric/v1",
                json!({
                    "id": id,
                    "cp": false,
                    "tv": 0,
                    "lv": 0,
                    "rv": 0,
                    "kv": 0,
                    "yv": 0,
                    "ytv": 0,
                    "yrv": 0
                }),
            )
            .await?;
        ensure_success(&response.body)?;
        let response: LyricsEnvelope = parse_body(response.body)?;
        map_lyrics(&id.to_string(), response)
    }

    async fn stream(&self, track: &Track, request: &StreamRequest) -> Result<MediaStream> {
        let client = self.client_for(request.account.as_deref())?;
        let batch = request_netease_streams(&client, std::slice::from_ref(track), request).await?;
        let outcome = batch.outcomes.into_iter().next().ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase returned an empty stream batch",
            )
            .with_platform(Platform::Netease)
            .with_details(json!(batch.extensions))
        })?;
        if let Some(stream) = outcome.stream {
            return Ok(stream);
        }
        Err(TuneWeaveError::new(
            outcome.error_code.unwrap_or(ErrorCode::UpstreamError),
            outcome
                .error
                .unwrap_or_else(|| "NetEase did not return a playable stream".to_owned()),
        )
        .with_platform(Platform::Netease)
        .with_details(json!(outcome.extensions)))
    }

    async fn streams(&self, tracks: &[Track], request: &StreamRequest) -> Result<StreamBatch> {
        if tracks.is_empty() {
            return Ok(StreamBatch {
                outcomes: Vec::new(),
                extensions: Extensions::new(),
            });
        }
        let client = self.client_for(request.account.as_deref())?;
        request_netease_streams(&client, tracks, request).await
    }

    async fn download(&self, track: &Track, request: &StreamRequest) -> Result<MediaDownload> {
        let id = validate_netease_stream_track(track)?;
        let client = self.client_for(request.account.as_deref())?;
        let (variant, path, payload, requested_level) = netease_download_request(id, request);
        let response = client.request_eapi(path, payload).await?;
        map_netease_download(
            track,
            request,
            variant,
            path,
            requested_level,
            response.body,
        )
    }

    async fn start_qr_login(&self, login_type: Option<&str>) -> Result<ProviderQrStart> {
        if let Some(login_type) = login_type.map(str::trim).filter(|value| !value.is_empty())
            && !matches!(login_type, "default" | "netease" | "pc")
        {
            return Err(TuneWeaveError::invalid_request(format!(
                "unsupported NetEase QR login type: {login_type}"
            ))
            .with_platform(Platform::Netease));
        }
        let login = NeteaseProvider::create_qr_login(self).await?;
        Ok(ProviderQrStart {
            provider_transaction_id: login.key,
            image_data_url: Some(qr_image_data_url(&login.url)?),
            url: login.url,
            expires_at: None,
        })
    }

    async fn poll_qr_login(
        &self,
        provider_transaction_id: &str,
        account: &str,
    ) -> Result<ProviderQrPoll> {
        let check = NeteaseProvider::check_qr_login(self, provider_transaction_id, account).await?;
        let state = match check.state {
            NeteaseQrState::Waiting => AuthState::Waiting,
            NeteaseQrState::Scanned => AuthState::Scanned,
            NeteaseQrState::Confirmed => AuthState::Confirmed,
            NeteaseQrState::Expired => AuthState::Expired,
            NeteaseQrState::Failed => AuthState::Failed,
        };
        Ok(ProviderQrPoll {
            state,
            message: check.message,
            profile: (state == AuthState::Confirmed)
                .then(|| AccountProfile::authenticated(Platform::Netease, account)),
        })
    }

    async fn password_login(&self, request: &PasswordLoginRequest) -> Result<AccountProfile> {
        let country_code = request.country_code.as_deref().unwrap_or("86");
        let summary = match (request.principal_type, request.password_format) {
            (PrincipalType::Email, PasswordFormat::Plain) => {
                NeteaseProvider::login_with_email_password(
                    self,
                    &request.account,
                    &request.principal,
                    &request.password,
                )
                .await?
            }
            (PrincipalType::Email, PasswordFormat::Md5) => {
                NeteaseProvider::login_with_email_md5(
                    self,
                    &request.account,
                    &request.principal,
                    &request.password,
                )
                .await?
            }
            (PrincipalType::Phone, PasswordFormat::Plain) => {
                NeteaseProvider::login_with_phone_password(
                    self,
                    &request.account,
                    &request.principal,
                    country_code,
                    &request.password,
                )
                .await?
            }
            (PrincipalType::Phone, PasswordFormat::Md5) => {
                NeteaseProvider::login_with_phone_password_md5(
                    self,
                    &request.account,
                    &request.principal,
                    country_code,
                    &request.password,
                )
                .await?
            }
            (PrincipalType::Username, _) => {
                return Err(TuneWeaveError::invalid_request(
                    "NetEase password login supports email or phone principals",
                )
                .with_platform(Platform::Netease));
            }
        };
        Ok(map_account_profile(&request.account, summary))
    }

    async fn start_auth_challenge(&self, request: &AuthChallengeRequest) -> Result<()> {
        match request.method {
            ChallengeMethod::Sms => {
                NeteaseProvider::send_phone_captcha(
                    self,
                    &request.principal,
                    request.country_code.as_deref().unwrap_or("86"),
                )
                .await
            }
        }
    }

    async fn validate_auth_challenge(
        &self,
        request: &AuthChallengeRequest,
        code: &str,
    ) -> Result<AuthChallengeValidation> {
        let verification = match request.method {
            ChallengeMethod::Sms => {
                NeteaseProvider::verify_phone_captcha(
                    self,
                    &request.principal,
                    request.country_code.as_deref().unwrap_or("86"),
                    code,
                )
                .await?
            }
        };
        let mut extensions = Extensions::new();
        extensions.insert("response".to_owned(), verification.response);
        Ok(AuthChallengeValidation {
            method: request.method,
            valid: verification.valid,
            platform_code: Some(verification.code.to_string()),
            message: verification.message,
            extensions,
        })
    }

    async fn country_calling_codes(
        &self,
        request: &CountryCallingCodeListRequest,
    ) -> Result<Vec<CountryCallingCodeGroup>> {
        let client = self.client_for(request.account.as_deref())?;
        let response = client
            .request_eapi("/api/lbs/countries/v1", json!({}))
            .await?;
        ensure_success(&response.body)?;
        map_netease_country_calling_codes(response.body)
    }

    async fn auth_principal_status(
        &self,
        request: &AuthPrincipalStatusRequest,
    ) -> Result<AuthPrincipalStatus> {
        if request.principal_type != PrincipalType::Phone {
            return Err(TuneWeaveError::invalid_request(
                "NetEase principal status only supports phone numbers",
            )
            .with_platform(Platform::Netease));
        }
        let client = self.client_for(Some(&request.account))?;
        let status = client
            .cellphone_status(
                &request.principal,
                request.country_code.as_deref().unwrap_or("86"),
            )
            .await?;
        let mut extensions = Extensions::new();
        extensions.insert("response".to_owned(), status.response);
        Ok(AuthPrincipalStatus {
            principal_type: request.principal_type,
            exists: status.exists,
            has_password: status.has_password,
            display_name: status.nickname,
            avatar_url: status.avatar_url,
            platform_code: Some(status.code.to_string()),
            extensions,
        })
    }

    async fn verify_auth_challenge(
        &self,
        request: &AuthChallengeRequest,
        code: &str,
    ) -> Result<AccountProfile> {
        let summary = match request.method {
            ChallengeMethod::Sms => {
                NeteaseProvider::login_with_phone_captcha(
                    self,
                    &request.account,
                    &request.principal,
                    request.country_code.as_deref().unwrap_or("86"),
                    code,
                )
                .await?
            }
        };
        Ok(map_account_profile(&request.account, summary))
    }

    async fn logout(&self, account: &str) -> Result<bool> {
        NeteaseProvider::logout_account(self, account).await
    }

    async fn session_profile(&self, account: &str) -> Result<AccountProfile> {
        let account = normalize_account_label(Some(account))?.to_owned();
        let client = match self.client_for(Some(&account)) {
            Ok(client) => client,
            Err(error) if error.code == ErrorCode::AuthenticationRequired => {
                let mut profile = AccountProfile::authenticated(Platform::Netease, account);
                profile.authenticated = false;
                return Ok(profile);
            }
            Err(error) => return Err(error),
        };
        let status = client.session_status().await?;
        Ok(map_session_profile(&account, status))
    }

    async fn refresh_session(&self, account: &str) -> Result<AccountProfile> {
        let account = normalize_account_label(Some(account))?.to_owned();
        let client = self.client_for(Some(&account))?;
        let refresh = client.refresh_session().await?;
        self.install_session(&account, refresh.into_session_cookie())?;
        let status = self.client_for(Some(&account))?.session_status().await?;
        Ok(map_session_profile(&account, status))
    }

    async fn upload_account_avatar(
        &self,
        request: &ImageUploadRequest,
    ) -> Result<ImageUploadResult> {
        let (client, allocation, upload_response) =
            self.upload_image_blob(request, "avatar upload").await?;
        let update_response = client
            .update_account_avatar(allocation.result.document_id.clone())
            .await?;
        ensure_account_access(&client, &update_response.body, "avatar upload")?;
        map_image_upload_result(request, allocation, upload_response, update_response.body)
    }

    async fn upload_cloud_track(&self, request: &CloudUploadRequest) -> Result<CloudUploadResult> {
        if request.data.is_empty() {
            return Err(
                TuneWeaveError::invalid_request("cloud audio body must not be empty")
                    .with_platform(Platform::Netease),
            );
        }
        if request.bitrate == 0 {
            return Err(TuneWeaveError::invalid_request(
                "cloud audio bitrate must be greater than zero",
            )
            .with_platform(Platform::Netease));
        }
        let file_size = u64::try_from(request.data.len()).map_err(|_| {
            TuneWeaveError::invalid_request("cloud audio body is too large")
                .with_platform(Platform::Netease)
        })?;
        let md5 = cloud_audio_md5(&request.data);
        let descriptor =
            cloud_upload_descriptor(&md5, &request.filename, Some(request.content_type.as_str()))?;
        let tagged_metadata = read_cloud_audio_metadata(&request.data);
        let (song_name, artist, album) =
            resolve_cloud_audio_metadata(request, &descriptor, &tagged_metadata)?;
        let ticket = self
            .cloud_upload_ticket(&CloudUploadTicketRequest {
                md5: md5.clone(),
                file_size,
                filename: descriptor.filename.clone(),
                bitrate: request.bitrate,
                content_type: Some(descriptor.content_type.clone()),
                account: request.account.clone(),
            })
            .await?;
        let provisional_track_id = ticket.provisional_track_id.clone().ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase cloud upload check did not return a song id",
            )
            .with_platform(Platform::Netease)
        })?;
        let upload_response = if ticket.upload_required {
            if ticket.upload_method != "POST" {
                return Err(TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "NetEase returned an unsupported cloud upload method",
                )
                .with_platform(Platform::Netease));
            }
            let token = ticket.upload_headers.get("x-nos-token").ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "NetEase cloud upload ticket did not contain an upload token",
                )
                .with_platform(Platform::Netease)
            })?;
            let content_type = ticket
                .upload_headers
                .get("Content-Type")
                .map_or(descriptor.content_type.as_str(), String::as_str);
            let client = self.client_for(request.account.as_deref())?;
            Some(
                client
                    .upload_cloud_audio(
                        &ticket.upload_url,
                        token,
                        &md5,
                        content_type,
                        &request.data,
                    )
                    .await?
                    .body,
            )
        } else {
            None
        };
        let mut result = self
            .complete_cloud_upload(&CloudUploadCompleteRequest {
                provisional_track_id,
                resource_id: ticket.resource_id.clone(),
                md5: md5.clone(),
                filename: descriptor.filename,
                song_name: Some(song_name.clone()),
                artist: Some(artist.clone()),
                album: Some(album.clone()),
                bitrate: request.bitrate,
                account: request.account.clone(),
            })
            .await?;
        result.upload_required = Some(ticket.upload_required);
        result.uploaded = Some(ticket.upload_required);
        result.extensions.insert(
            "proxy_upload".to_owned(),
            json!({
                "md5": md5,
                "file_size": file_size,
                "content_type": descriptor.content_type,
                "song_name": song_name,
                "artist": artist,
                "album": album,
                "tagged_metadata": tagged_metadata,
                "ticket": ticket.extensions
            }),
        );
        if let Some(upload_response) = upload_response {
            result
                .extensions
                .insert("upload_response".to_owned(), upload_response);
        }
        Ok(result)
    }

    async fn cloud_upload_ticket(
        &self,
        request: &CloudUploadTicketRequest,
    ) -> Result<CloudUploadTicket> {
        let descriptor = validate_cloud_upload_ticket_request(request)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "cloud upload")?;

        let check_response = client
            .request_eapi(
                "/api/cloud/upload/check",
                json!({
                    "bitrate": request.bitrate.to_string(),
                    "ext": "",
                    "length": request.file_size,
                    "md5": descriptor.md5,
                    "songId": "0",
                    "version": 1
                }),
            )
            .await?;
        ensure_account_access(&client, &check_response.body, "cloud upload")?;
        let check_response = check_response.body;

        let allocation_response = client
            .request_weapi(
                "/api/nos/token/alloc",
                json!({
                    "bucket": CLOUD_UPLOAD_BUCKET,
                    "ext": descriptor.extension,
                    "filename": descriptor.allocation_filename,
                    "local": false,
                    "nos_product": 3,
                    "type": "audio",
                    "md5": descriptor.md5
                }),
            )
            .await?;
        ensure_account_access(&client, &allocation_response.body, "cloud upload token")?;
        let allocation: CloudUploadAllocationEnvelope = parse_body(allocation_response.body)?;
        validate_cloud_upload_allocation(&allocation)?;

        let servers_response = client.cloud_upload_servers(CLOUD_UPLOAD_BUCKET).await?;
        let servers_raw = servers_response.body.clone();
        let servers: CloudUploadServersEnvelope = parse_body(servers_response.body)?;
        let server = servers.upload.first().ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase did not return a cloud upload server",
            )
            .with_platform(Platform::Netease)
        })?;
        let upload_url =
            build_cloud_upload_url(server, CLOUD_UPLOAD_BUCKET, &allocation.result.object_key)?;
        let resource_id = json_scalar_string(&allocation.result.resource_id).ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase cloud upload allocation did not contain a resource id",
            )
            .with_platform(Platform::Netease)
        })?;
        let upload_required = check_response
            .get("needUpload")
            .and_then(json_bool)
            .unwrap_or(true);
        let provisional_track_id = check_response.get("songId").and_then(json_scalar_string);
        let mut upload_headers = BTreeMap::new();
        upload_headers.insert("Content-Length".to_owned(), request.file_size.to_string());
        upload_headers.insert("Content-MD5".to_owned(), descriptor.md5.clone());
        upload_headers.insert("Content-Type".to_owned(), descriptor.content_type.clone());
        upload_headers.insert("x-nos-token".to_owned(), allocation.result.token.clone());
        let mut extensions = Extensions::new();
        extensions.insert("check_response".to_owned(), check_response);
        extensions.insert(
            "allocation".to_owned(),
            json!({
                "bucket": CLOUD_UPLOAD_BUCKET,
                "object_key": allocation.result.object_key,
                "resource_id": resource_id,
                "extension": descriptor.extension,
                "filename": descriptor.allocation_filename
            }),
        );
        extensions.insert("upload_servers_response".to_owned(), servers_raw);
        Ok(CloudUploadTicket {
            upload_required,
            provisional_track_id,
            resource_id,
            upload_method: "POST".to_owned(),
            upload_url,
            upload_headers,
            extensions,
        })
    }

    async fn complete_cloud_upload(
        &self,
        request: &CloudUploadCompleteRequest,
    ) -> Result<CloudUploadResult> {
        let descriptor = validate_cloud_upload_complete_request(request)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "cloud upload completion")?;
        let info_response = client
            .request_eapi(
                "/api/upload/cloud/info/v2",
                json!({
                    "md5": descriptor.md5,
                    "songid": descriptor.provisional_track_id,
                    "filename": descriptor.filename,
                    "song": descriptor.song_name,
                    "album": descriptor.album,
                    "artist": descriptor.artist,
                    "bitrate": request.bitrate.to_string(),
                    "resourceId": descriptor.resource_id
                }),
            )
            .await?;
        ensure_account_access(&client, &info_response.body, "cloud upload completion")?;
        let info_response = info_response.body;
        let track_id = info_response
            .get("songId")
            .and_then(json_scalar_string)
            .unwrap_or_else(|| descriptor.provisional_track_id.to_owned());
        let publish_response = client
            .request_eapi("/api/cloud/pub/v2", json!({ "songid": track_id }))
            .await?;
        ensure_account_access(&client, &publish_response.body, "cloud publication")?;
        map_cloud_upload_result(track_id, None, None, info_response, publish_response.body)
    }

    async fn import_cloud_track(&self, request: &CloudImportRequest) -> Result<CloudImportResult> {
        let descriptor = validate_cloud_import_request(request)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "cloud import")?;
        let check_response = client
            .request_eapi(
                "/api/cloud/upload/check/v2",
                cloud_import_check_payload(&descriptor, request.file_size),
            )
            .await?;
        ensure_account_access(&client, &check_response.body, "cloud import check")?;
        let check_response = check_response.body;
        let check_item = check_response
            .get("data")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "NetEase cloud import check did not return a result",
                )
                .with_platform(Platform::Netease)
            })?;
        let checked_track_id = check_item
            .get("songId")
            .and_then(json_scalar_string)
            .ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "NetEase cloud import check did not return a song id",
                )
                .with_platform(Platform::Netease)
            })?;
        let upload_status = check_item.get("upload").and_then(json_i64);
        let import_response = client
            .request_eapi(
                "/api/cloud/user/song/import",
                cloud_import_payload(&descriptor, &checked_track_id),
            )
            .await?;
        ensure_account_access(&client, &import_response.body, "cloud import")?;
        map_cloud_import_result(
            &checked_track_id,
            upload_status,
            check_response,
            import_response.body,
        )
    }

    async fn cloud_lyrics(&self, request: &CloudLyricsRequest) -> Result<Lyrics> {
        let user_id = required_cloud_value("user_id", &request.user_id)?;
        let track_id = required_cloud_value("track_id", &request.track_id)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "cloud lyrics")?;
        let response = client
            .request_eapi(
                "/api/cloud/lyric/get",
                cloud_lyrics_payload(&user_id, &track_id),
            )
            .await?;
        ensure_account_access(&client, &response.body, "cloud lyrics")?;
        let raw = response.body.clone();
        let response: LyricsEnvelope = parse_body(response.body)?;
        let mut lyrics = map_lyrics(&track_id, response)?;
        lyrics
            .extensions
            .insert("cloud_user_id".to_owned(), json!(user_id));
        lyrics.extensions.insert("cloud_response".to_owned(), raw);
        Ok(lyrics)
    }

    async fn match_cloud_track(&self, request: &CloudMatchRequest) -> Result<CloudMatchResult> {
        let user_id = required_cloud_value("user_id", &request.user_id)?;
        let cloud_track_id = required_cloud_value("cloud_track_id", &request.cloud_track_id)?;
        let target_track_id = request
            .target_track_id
            .as_deref()
            .map(|value| required_cloud_value("target_track_id", value))
            .transpose()?
            .unwrap_or_else(|| "0".to_owned());
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "cloud track matching")?;
        let response = client
            .request_weapi(
                "/api/cloud/user/song/match",
                cloud_match_payload(&user_id, &cloud_track_id, &target_track_id),
            )
            .await?;
        ensure_account_access(&client, &response.body, "cloud track matching")?;
        map_cloud_match_result(&cloud_track_id, &target_track_id, &user_id, response.body)
    }

    async fn cloud_tracks(&self, request: &PageRequest) -> Result<Page<CloudTrack>> {
        let limit = request.limit.clamp(1, 100);
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "cloud library")?;
        let (path, payload) = netease_cloud_tracks_request(limit, request.offset);
        let response = client.request_weapi(path, payload).await?;
        ensure_account_access(&client, &response.body, "cloud library")?;
        map_netease_cloud_tracks(limit, request.offset, response.body)
    }

    async fn cloud_track_details(
        &self,
        request: &CloudTrackDetailRequest,
    ) -> Result<Vec<CloudTrack>> {
        let ids = validate_cloud_track_refs(&request.track_refs)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "cloud track details")?;
        let (path, payload) = netease_cloud_track_details_request(&ids);
        let response = client.request_weapi(path, payload).await?;
        ensure_account_access(&client, &response.body, "cloud track details")?;
        map_netease_cloud_track_details(response.body)
    }

    async fn delete_cloud_tracks(
        &self,
        request: &CloudTrackDeleteRequest,
    ) -> Result<CloudTrackDeleteResult> {
        let ids = validate_cloud_track_refs(&request.track_refs)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "cloud track deletion")?;
        let (path, payload) = netease_cloud_track_delete_request(&ids);
        let response = client.request_weapi(path, payload).await?;
        ensure_account_access(&client, &response.body, "cloud track deletion")?;
        let mut extensions = Extensions::new();
        extensions.insert("response".to_owned(), response.body);
        Ok(CloudTrackDeleteResult {
            track_refs: request.track_refs.clone(),
            deleted: true,
            extensions,
        })
    }

    async fn download_cloud_track(&self, id: &str, account: Option<&str>) -> Result<MediaDownload> {
        let id = required_cloud_value("track_id", id)?;
        let client = self.client_for(account)?;
        require_authenticated_client(&client, "cloud track download")?;
        let (path, payload) = netease_cloud_track_download_request(&id);
        let response = client.request_eapi(path, payload).await?;
        ensure_account_access(&client, &response.body, "cloud track download")?;
        map_netease_cloud_track_download(&id, path, response.body)
    }

    async fn post_comment(&self, request: &CommentWriteRequest) -> Result<CommentMutationResult> {
        let (path, payload, action) = netease_comment_write_request(request)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "comment writing")?;
        let response = client.request_weapi(path, payload).await?;
        ensure_account_access(&client, &response.body, "comment writing")?;
        map_comment_mutation_result(&request.target, action, None, response.body)
    }

    async fn delete_comment(
        &self,
        request: &CommentDeleteRequest,
    ) -> Result<CommentMutationResult> {
        let (path, payload, comment_id) = netease_comment_delete_request(request)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "comment deletion")?;
        let response = client.request_weapi(path, payload).await?;
        ensure_account_access(&client, &response.body, "comment deletion")?;
        map_comment_mutation_result(
            &request.target,
            CommentMutationAction::Delete,
            Some(&comment_id),
            response.body,
        )
    }

    async fn comments(&self, request: &CommentListRequest) -> Result<CommentPage> {
        let (path, payload, mode) = netease_comment_list_request(request)?;
        let client = self.client_for(request.account.as_deref())?;
        let response = client.request_weapi(&path, payload).await?;
        ensure_success(&response.body)?;
        map_netease_comment_page(request, mode, response.body)
    }

    async fn comment_reactions(
        &self,
        request: &CommentReactionListRequest,
    ) -> Result<CommentReactionPage> {
        let (path, payload) = netease_comment_reaction_list_request(request)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "comment reaction listing")?;
        let response = client.request_eapi(path, payload).await?;
        ensure_account_access(&client, &response.body, "comment reaction listing")?;
        map_netease_comment_reaction_page(request, response.body)
    }

    async fn set_comment_reaction(
        &self,
        request: &CommentReactionMutationRequest,
    ) -> Result<CommentReactionMutationResult> {
        let (path, payload) = netease_comment_reaction_mutation_request(request)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "comment reaction writing")?;
        let response = client.request_weapi(path, payload).await?;
        ensure_account_access(&client, &response.body, "comment reaction writing")?;
        map_netease_comment_reaction_mutation(request, response.body)
    }

    async fn report_comment(&self, request: &CommentReportRequest) -> Result<CommentReportResult> {
        let (path, payload) = netease_comment_report_request(request)?;
        let client = self.client_for(request.account.as_deref())?;
        require_authenticated_client(&client, "comment reporting")?;
        let response = client.request_eapi(path, payload).await?;
        ensure_account_access(&client, &response.body, "comment reporting")?;
        map_netease_comment_report(request, response.body)
    }

    async fn comment_thread_stats(
        &self,
        request: &CommentThreadStatsRequest,
    ) -> Result<CommentThreadStatsBatch> {
        let (path, payload) = netease_comment_thread_stats_request(request)?;
        let client = self.client_for(request.account.as_deref())?;
        let response = client.request_weapi(path, payload).await?;
        ensure_success(&response.body)?;
        map_netease_comment_thread_stats(request, response.body)
    }

    async fn platform_api(&self, request: &PlatformApiRequest) -> Result<Value> {
        let uri = validate_platform_api_request(request)?;
        let protocol = NeteaseApiProtocol::parse(request.protocol.as_deref())?;
        let client = self.client_for(request.account.as_deref())?;
        let response = match protocol {
            NeteaseApiProtocol::Eapi => client.request_eapi(uri, request.data.clone()).await?,
            NeteaseApiProtocol::Weapi => client.request_weapi(uri, request.data.clone()).await?,
            NeteaseApiProtocol::Api => client.request_api(uri, request.data.clone()).await?,
            NeteaseApiProtocol::Linuxapi => {
                client.request_linuxapi(uri, request.data.clone()).await?
            }
            NeteaseApiProtocol::Xeapi => client.request_xeapi(uri, request.data.clone()).await?,
        };
        ensure_platform_api_success(&response.body)?;
        Ok(response.body)
    }

    async fn platform_batch(&self, request: &PlatformBatchRequest) -> Result<Value> {
        validate_platform_batch_request(request)?;
        let protocol = NeteaseApiProtocol::parse(request.protocol.as_deref())?;
        let client = self.client_for(request.account.as_deref())?;
        let data = serialize_netease_batch_requests(request);
        let response = match protocol {
            NeteaseApiProtocol::Eapi => client.request_eapi("/api/batch", data).await?,
            NeteaseApiProtocol::Weapi => client.request_weapi("/api/batch", data).await?,
            NeteaseApiProtocol::Api => client.request_api("/api/batch", data).await?,
            NeteaseApiProtocol::Linuxapi => client.request_linuxapi("/api/batch", data).await?,
            NeteaseApiProtocol::Xeapi => client.request_xeapi("/api/batch", data).await?,
        };
        ensure_platform_api_success(&response.body)?;
        Ok(response.body)
    }
}

fn qr_image_data_url(url: &str) -> Result<String> {
    let code = QrCode::new(url.as_bytes()).map_err(|_| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase QR login URL could not be encoded as an image",
        )
        .with_platform(Platform::Netease)
    })?;
    let image = code
        .render::<svg::Color>()
        .min_dimensions(320, 320)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build();
    Ok(format!(
        "data:image/svg+xml;base64,{}",
        BASE64.encode(image.as_bytes())
    ))
}

fn netease_comment_write_request(
    request: &CommentWriteRequest,
) -> Result<(&'static str, Value, CommentMutationAction)> {
    let content = request.content.trim();
    if content.is_empty() {
        return Err(
            TuneWeaveError::invalid_request("comment content cannot be empty")
                .with_platform(Platform::Netease),
        );
    }
    let thread_id = netease_comment_thread_id(&request.target)?;
    let mut payload = json!({
        "threadId": thread_id,
        "content": request.content
    });
    if let Some(reply_to) = request.reply_to.as_deref() {
        let comment_id = required_comment_id("reply_to", reply_to)?;
        payload["commentId"] = json!(comment_id);
        Ok((
            "/api/resource/comments/reply",
            payload,
            CommentMutationAction::Reply,
        ))
    } else {
        Ok((
            "/api/resource/comments/add",
            payload,
            CommentMutationAction::Create,
        ))
    }
}

fn netease_comment_delete_request(
    request: &CommentDeleteRequest,
) -> Result<(&'static str, Value, String)> {
    let thread_id = netease_comment_thread_id(&request.target)?;
    let comment_id = required_comment_id("comment_id", &request.comment_id)?;
    Ok((
        "/api/resource/comments/delete",
        json!({
            "threadId": thread_id,
            "commentId": comment_id
        }),
        comment_id,
    ))
}

fn netease_comment_thread_id(target: &CommentTarget) -> Result<String> {
    if target.resource_ref.platform() != Platform::Netease {
        return Err(TuneWeaveError::invalid_request(
            "NetEase comment targets must use a netease resource reference",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "ref": target.resource_ref })));
    }
    let id = target.resource_ref.id();
    if target.kind == CommentTargetKind::Event {
        if !id.starts_with("A_EV_2_") {
            return Err(TuneWeaveError::invalid_request(
                "NetEase event comment targets must use the complete A_EV_2_ thread id",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "ref": target.resource_ref })));
        }
        return Ok(id.to_owned());
    }
    let prefix = match target.kind {
        CommentTargetKind::Track => "R_SO_4_",
        CommentTargetKind::Mv => "R_MV_5_",
        CommentTargetKind::Playlist => "A_PL_0_",
        CommentTargetKind::Album => "R_AL_3_",
        CommentTargetKind::RadioEpisode => "A_DJ_1_",
        CommentTargetKind::Video => "R_VI_62_",
        CommentTargetKind::RadioStation => "A_DR_14_",
        CommentTargetKind::Event => unreachable!("event targets return above"),
    };
    Ok(format!("{prefix}{id}"))
}

fn required_comment_id(field: &str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(
            TuneWeaveError::invalid_request(format!("{field} cannot be empty"))
                .with_platform(Platform::Netease),
        );
    }
    Ok(value.to_owned())
}

fn netease_comment_reaction_list_request(
    request: &CommentReactionListRequest,
) -> Result<(&'static str, Value)> {
    if request.kind != CommentReactionKind::Hug {
        return Err(TuneWeaveError::invalid_request(
            "NetEase only exposes hug reaction directories",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "kind": request.kind, "allowed": ["hug"] })));
    }
    if request.target_user_ref.platform() != Platform::Netease {
        return Err(TuneWeaveError::invalid_request(
            "NetEase comment reaction users must use a netease resource reference",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "target_user_ref": request.target_user_ref })));
    }
    if !(1..=100).contains(&request.limit) {
        return Err(TuneWeaveError::invalid_request(
            "comment reaction limit must be between 1 and 100",
        )
        .with_platform(Platform::Netease));
    }
    if request.page == 0 {
        return Err(TuneWeaveError::invalid_request(
            "comment reaction page must be greater than zero",
        )
        .with_platform(Platform::Netease));
    }
    let thread_id = netease_comment_thread_id(&request.target)?;
    let comment_id = required_comment_id("comment_id", &request.comment_id)?;
    let target_user_id = required_comment_id("target_user_ref", request.target_user_ref.id())?;
    let cursor = comment_reaction_cursor("cursor", request.cursor.as_deref())?;
    let id_cursor = comment_reaction_cursor("id_cursor", request.id_cursor.as_deref())?;
    Ok((
        "/api/v2/resource/comments/hug/list",
        json!({
            "targetUserId": target_user_id,
            "commentId": comment_id,
            "cursor": cursor,
            "threadId": thread_id,
            "pageNo": request.page,
            "idCursor": id_cursor,
            "pageSize": request.limit
        }),
    ))
}

fn comment_reaction_cursor(field: &str, value: Option<&str>) -> Result<String> {
    value.map_or_else(
        || Ok("-1".to_owned()),
        |value| required_comment_id(field, value),
    )
}

fn netease_comment_reaction_mutation_request(
    request: &CommentReactionMutationRequest,
) -> Result<(&'static str, Value)> {
    if request.kind != CommentReactionKind::Like {
        return Err(TuneWeaveError::invalid_request(
            "NetEase comment like protocol only accepts like reactions",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "kind": request.kind, "allowed": ["like"] })));
    }
    if request.target_user_ref.is_some() {
        return Err(TuneWeaveError::invalid_request(
            "NetEase comment likes do not accept target_user_ref",
        )
        .with_platform(Platform::Netease));
    }
    let thread_id = netease_comment_thread_id(&request.target)?;
    let comment_id = required_comment_id("comment_id", &request.comment_id)?;
    let path = if request.active {
        "/api/v1/comment/like"
    } else {
        "/api/v1/comment/unlike"
    };
    Ok((
        path,
        json!({
            "threadId": thread_id,
            "commentId": comment_id
        }),
    ))
}

fn netease_comment_report_request(request: &CommentReportRequest) -> Result<(&'static str, Value)> {
    if request.target.kind != CommentTargetKind::Track {
        return Err(TuneWeaveError::invalid_request(
            "NetEase comment reports only accept track comment targets",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "kind": request.target.kind, "allowed": ["track"] })));
    }
    let thread_id = netease_comment_thread_id(&request.target)?;
    let comment_id = required_comment_id("comment_id", &request.comment_id)?;
    if request.reason.trim().is_empty() {
        return Err(
            TuneWeaveError::invalid_request("comment report reason cannot be empty")
                .with_platform(Platform::Netease),
        );
    }
    Ok((
        "/api/report/reportcomment",
        json!({
            "threadId": thread_id,
            "commentId": comment_id,
            "reason": request.reason
        }),
    ))
}

fn netease_comment_thread_stats_request(
    request: &CommentThreadStatsRequest,
) -> Result<(&'static str, Value)> {
    for reference in &request.resource_refs {
        if reference.platform() != Platform::Netease {
            return Err(TuneWeaveError::invalid_request(
                "NetEase comment stats resources must use netease references",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "ref": reference })));
        }
    }
    let resource_ids = request
        .resource_refs
        .iter()
        .map(ResourceRef::id)
        .collect::<Vec<_>>();
    Ok((
        "/api/resource/commentInfo/list",
        json!({
            "resourceType": netease_comment_resource_type(request.kind),
            "resourceIds": json!(resource_ids).to_string()
        }),
    ))
}

fn netease_comment_resource_type(kind: CommentTargetKind) -> &'static str {
    match kind {
        CommentTargetKind::Track => "4",
        CommentTargetKind::Mv => "5",
        CommentTargetKind::Playlist => "0",
        CommentTargetKind::Album => "3",
        CommentTargetKind::RadioEpisode => "1",
        CommentTargetKind::Video => "62",
        CommentTargetKind::Event => "2",
        CommentTargetKind::RadioStation => "14",
    }
}

fn map_comment_mutation_result(
    target: &CommentTarget,
    action: CommentMutationAction,
    requested_comment_id: Option<&str>,
    response: Value,
) -> Result<CommentMutationResult> {
    let comment_id = requested_comment_id.map(str::to_owned).or_else(|| {
        [
            response.pointer("/comment/commentId"),
            response.pointer("/data/commentId"),
            response.get("commentId"),
        ]
        .into_iter()
        .flatten()
        .find_map(usable_resource_id)
    });
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), response);
    Ok(CommentMutationResult {
        target: target.clone(),
        comment_id,
        action,
        extensions,
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NeteaseCommentListMode {
    Legacy,
    Modern,
    Hot,
    Floor,
}

fn netease_comment_list_request(
    request: &CommentListRequest,
) -> Result<(String, Value, NeteaseCommentListMode)> {
    if request.limit == 0 {
        return Err(
            TuneWeaveError::invalid_request("comment limit must be greater than zero")
                .with_platform(Platform::Netease),
        );
    }
    let thread_id = netease_comment_thread_id(&request.target)?;
    match request.view {
        CommentListView::All => {
            if request.parent_comment_id.is_some() {
                return Err(comment_request_conflict(
                    "parent_comment_id requires view=replies",
                ));
            }
            if let Some(sort) = request.sort {
                if request.before_time_ms.is_some() {
                    return Err(comment_request_conflict(
                        "before_time_ms is not used by sorted comments",
                    ));
                }
                let page_no = comment_page_no(request)?;
                let page_offset = page_no.saturating_sub(1).saturating_mul(request.limit);
                let (sort_type, cursor) = match sort {
                    CommentSort::Recommended => (99, json!(page_offset)),
                    CommentSort::Hot => (2, json!(format!("normalHot#{page_offset}"))),
                    CommentSort::Time => (
                        3,
                        json!(
                            request
                                .cursor
                                .as_deref()
                                .map(str::trim)
                                .filter(|cursor| !cursor.is_empty())
                                .unwrap_or("0")
                        ),
                    ),
                };
                if sort != CommentSort::Time && request.cursor.is_some() {
                    return Err(comment_request_conflict(
                        "cursor is only accepted with sort=time",
                    ));
                }
                return Ok((
                    "/api/v2/resource/comments".to_owned(),
                    json!({
                        "threadId": thread_id,
                        "pageNo": page_no,
                        "showInner": request.include_replies,
                        "pageSize": request.limit,
                        "cursor": cursor,
                        "sortType": sort_type
                    }),
                    NeteaseCommentListMode::Modern,
                ));
            }
            if request.page.is_some() || request.cursor.is_some() {
                return Err(comment_request_conflict(
                    "page and cursor require an explicit comment sort",
                ));
            }
            let mut payload = json!({
                "limit": request.limit,
                "offset": request.offset,
                "beforeTime": request.before_time_ms.unwrap_or(0)
            });
            if request.target.kind != CommentTargetKind::Event {
                payload["rid"] = json!(request.target.resource_ref.id());
            }
            Ok((
                format!("/api/v1/resource/comments/{thread_id}"),
                payload,
                NeteaseCommentListMode::Legacy,
            ))
        }
        CommentListView::Hot => {
            if request.sort.is_some()
                || request.page.is_some()
                || request.cursor.is_some()
                || request.parent_comment_id.is_some()
            {
                return Err(comment_request_conflict(
                    "view=hot does not accept sort, page, cursor, or parent_comment_id",
                ));
            }
            Ok((
                format!("/api/v1/resource/hotcomments/{thread_id}"),
                json!({
                    "rid": request.target.resource_ref.id(),
                    "limit": request.limit,
                    "offset": request.offset,
                    "beforeTime": request.before_time_ms.unwrap_or(0)
                }),
                NeteaseCommentListMode::Hot,
            ))
        }
        CommentListView::Replies => {
            if request.sort.is_some() || request.page.is_some() || request.cursor.is_some() {
                return Err(comment_request_conflict(
                    "view=replies does not accept sort, page, or cursor",
                ));
            }
            let parent_comment_id = request
                .parent_comment_id
                .as_deref()
                .ok_or_else(|| {
                    comment_request_conflict("parent_comment_id is required for view=replies")
                })
                .and_then(|id| required_comment_id("parent_comment_id", id))?;
            let time = request
                .before_time_ms
                .map(i64::try_from)
                .transpose()
                .map_err(|_| {
                    comment_request_conflict("before_time_ms exceeds the signed platform range")
                })?
                .unwrap_or(-1);
            Ok((
                "/api/resource/comment/floor/get".to_owned(),
                json!({
                    "parentCommentId": parent_comment_id,
                    "threadId": thread_id,
                    "time": time,
                    "limit": request.limit
                }),
                NeteaseCommentListMode::Floor,
            ))
        }
    }
}

fn comment_page_no(request: &CommentListRequest) -> Result<u32> {
    let page = request
        .page
        .unwrap_or_else(|| (request.offset / request.limit).saturating_add(1));
    if page == 0 {
        return Err(comment_request_conflict(
            "comment page must be greater than zero",
        ));
    }
    Ok(page)
}

fn comment_request_conflict(message: &str) -> TuneWeaveError {
    TuneWeaveError::invalid_request(message).with_platform(Platform::Netease)
}

fn map_netease_comment_page(
    request: &CommentListRequest,
    mode: NeteaseCommentListMode,
    response: Value,
) -> Result<CommentPage> {
    match mode {
        NeteaseCommentListMode::Legacy => map_legacy_comment_page(request, response),
        NeteaseCommentListMode::Modern => map_modern_comment_page(request, response),
        NeteaseCommentListMode::Hot => map_hot_comment_page(request, response),
        NeteaseCommentListMode::Floor => map_floor_comment_page(request, response),
    }
}

fn map_legacy_comment_page(request: &CommentListRequest, response: Value) -> Result<CommentPage> {
    let comments = map_comment_array(&response, "comments")?;
    let hot_comments = map_comment_array(&response, "hotComments")?;
    let top_comments = map_comment_array(&response, "topComments")?;
    let total = response.get("total").and_then(json_u64);
    let consumed = u32::try_from(comments.len()).unwrap_or(u32::MAX);
    let next_offset = request.offset.saturating_add(consumed);
    let has_more = response
        .get("more")
        .and_then(json_bool)
        .or_else(|| total.map(|total| u64::from(next_offset) < total))
        .unwrap_or(consumed >= request.limit);
    let mut pagination_extensions = Extensions::new();
    pagination_extensions.insert("mode".to_owned(), json!("legacy"));
    pagination_extensions.insert("returned_count".to_owned(), json!(comments.len()));
    pagination_extensions.insert(
        "limit_applied".to_owned(),
        json!(comments.len() <= request.limit as usize),
    );
    insert_extension(
        &mut pagination_extensions,
        "next_before_time_ms",
        comments.last().and_then(|comment| comment.created_at_ms),
    );
    insert_extension(
        &mut pagination_extensions,
        "more_hot",
        response.get("moreHot").and_then(json_bool),
    );
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), response);
    Ok(CommentPage {
        target: request.target.clone(),
        comments,
        hot_comments,
        top_comments,
        current_comment: None,
        pagination: PageMeta {
            limit: request.limit,
            offset: request.offset,
            total,
            next_offset: (has_more && consumed > 0).then_some(next_offset),
            has_more,
            extensions: pagination_extensions,
        },
        extensions,
    })
}

fn map_modern_comment_page(request: &CommentListRequest, response: Value) -> Result<CommentPage> {
    let data = response.get("data").unwrap_or(&Value::Null);
    let comments = map_comment_array(data, "comments")?;
    let hot_comments = map_comment_array(data, "hotComments")?;
    let top_comments = map_comment_array(data, "topComments")?;
    let current_comment = map_optional_comment(data.get("currentComment"))?;
    let total = data.get("totalCount").and_then(json_u64);
    let has_more = data.get("hasMore").and_then(json_bool).unwrap_or(false);
    let page = comment_page_no(request)?;
    let offset = page.saturating_sub(1).saturating_mul(request.limit);
    let mut pagination_extensions = Extensions::new();
    pagination_extensions.insert("mode".to_owned(), json!("modern"));
    pagination_extensions.insert("page".to_owned(), json!(page));
    pagination_extensions.insert("requested_offset".to_owned(), json!(request.offset));
    pagination_extensions.insert("returned_count".to_owned(), json!(comments.len()));
    pagination_extensions.insert(
        "limit_applied".to_owned(),
        json!(comments.len() <= request.limit as usize),
    );
    insert_extension(
        &mut pagination_extensions,
        "next_cursor",
        data.get("cursor").and_then(json_scalar_string),
    );
    insert_extension(
        &mut pagination_extensions,
        "platform_sort_type",
        data.get("sortType").and_then(json_i64),
    );
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), response);
    Ok(CommentPage {
        target: request.target.clone(),
        comments,
        hot_comments,
        top_comments,
        current_comment,
        pagination: PageMeta {
            limit: request.limit,
            offset,
            total,
            next_offset: has_more.then_some(offset.saturating_add(request.limit)),
            has_more,
            extensions: pagination_extensions,
        },
        extensions,
    })
}

fn map_hot_comment_page(request: &CommentListRequest, response: Value) -> Result<CommentPage> {
    let hot_comments = map_comment_array(&response, "hotComments")?;
    let top_comments = map_comment_array(&response, "topComments")?;
    let total = response.get("total").and_then(json_u64);
    let consumed = u32::try_from(hot_comments.len()).unwrap_or(u32::MAX);
    let next_offset = request.offset.saturating_add(consumed);
    let has_more = response
        .get("hasMore")
        .and_then(json_bool)
        .or_else(|| total.map(|total| u64::from(next_offset) < total))
        .unwrap_or(consumed >= request.limit);
    let mut pagination_extensions = Extensions::new();
    pagination_extensions.insert("mode".to_owned(), json!("hot"));
    pagination_extensions.insert("returned_count".to_owned(), json!(hot_comments.len()));
    pagination_extensions.insert(
        "limit_applied".to_owned(),
        json!(hot_comments.len() <= request.limit as usize),
    );
    insert_extension(
        &mut pagination_extensions,
        "next_before_time_ms",
        hot_comments
            .last()
            .and_then(|comment| comment.created_at_ms),
    );
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), response);
    Ok(CommentPage {
        target: request.target.clone(),
        comments: Vec::new(),
        hot_comments,
        top_comments,
        current_comment: None,
        pagination: PageMeta {
            limit: request.limit,
            offset: request.offset,
            total,
            next_offset: (has_more && consumed > 0).then_some(next_offset),
            has_more,
            extensions: pagination_extensions,
        },
        extensions,
    })
}

fn map_floor_comment_page(request: &CommentListRequest, response: Value) -> Result<CommentPage> {
    let data = response.get("data").unwrap_or(&Value::Null);
    let comments = map_comment_array(data, "comments")?;
    let top_comments = map_comment_array(data, "bestComments")?;
    let current_comment = map_optional_comment(data.get("currentComment"))?;
    let total = data.get("totalCount").and_then(json_u64);
    let has_more = data.get("hasMore").and_then(json_bool).unwrap_or(false);
    let mut pagination_extensions = Extensions::new();
    pagination_extensions.insert("mode".to_owned(), json!("floor"));
    pagination_extensions.insert("requested_offset".to_owned(), json!(request.offset));
    pagination_extensions.insert("offset_applied".to_owned(), json!(false));
    pagination_extensions.insert("returned_count".to_owned(), json!(comments.len()));
    insert_extension(
        &mut pagination_extensions,
        "next_before_time_ms",
        data.get("time").and_then(json_u64).filter(|time| *time > 0),
    );
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), response);
    Ok(CommentPage {
        target: request.target.clone(),
        comments,
        hot_comments: Vec::new(),
        top_comments,
        current_comment,
        pagination: PageMeta {
            limit: request.limit,
            offset: 0,
            total,
            next_offset: None,
            has_more,
            extensions: pagination_extensions,
        },
        extensions,
    })
}

fn map_comment_array(container: &Value, field: &str) -> Result<Vec<Comment>> {
    container
        .get(field)
        .and_then(Value::as_array)
        .map(|comments| comments.iter().cloned().map(map_netease_comment).collect())
        .unwrap_or_else(|| Ok(Vec::new()))
}

fn map_optional_comment(raw: Option<&Value>) -> Result<Option<Comment>> {
    raw.filter(|raw| raw.as_object().is_some_and(|object| !object.is_empty()))
        .cloned()
        .map(map_netease_comment)
        .transpose()
}

fn map_netease_comment(raw: Value) -> Result<Comment> {
    let id = [raw.get("commentId"), raw.get("id")]
        .into_iter()
        .flatten()
        .find_map(usable_resource_id)
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase comment is missing a usable id",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "comment": raw }))
        })?;
    let content = raw
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let parent_comment_id = raw
        .get("parentCommentId")
        .and_then(json_scalar_string)
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty() && id != "0" && id != "-1");
    let replied_to = raw
        .get("beReplied")
        .and_then(Value::as_array)
        .map(|replies| {
            replies
                .iter()
                .cloned()
                .map(map_comment_reply_reference)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let ip_location = raw
        .pointer("/ipLocation/location")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|location| !location.is_empty())
        .map(str::to_owned);
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw.clone());
    Ok(Comment {
        platform: Platform::Netease,
        id,
        content,
        author: raw.get("user").and_then(map_netease_comment_user),
        created_at_ms: raw.get("time").and_then(json_u64),
        created_at_text: raw
            .get("timeStr")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned),
        liked: raw.get("liked").and_then(json_bool),
        like_count: raw.get("likedCount").and_then(json_u64),
        parent_comment_id,
        reply_count: raw.get("replyCount").and_then(json_u64),
        replied_to,
        ip_location,
        extensions,
    })
}

fn map_comment_reply_reference(raw: Value) -> CommentReplyReference {
    let comment_id = ["beRepliedCommentId", "commentId", "id"]
        .into_iter()
        .find_map(|field| raw.get(field).and_then(usable_resource_id));
    let content = raw
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let author = raw.get("user").and_then(map_netease_comment_user);
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw);
    CommentReplyReference {
        comment_id,
        content,
        author,
        extensions,
    }
}

fn map_netease_comment_user(raw: &Value) -> Option<User> {
    let id = [raw.get("userId"), raw.get("id")]
        .into_iter()
        .flatten()
        .find_map(usable_resource_id)?;
    let resource_ref = ResourceRef::new(Platform::Netease, id.clone()).ok()?;
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw.clone());
    Some(User {
        resource_ref,
        platform: Platform::Netease,
        id,
        name: radio_text_field(raw, &["nickname", "userName", "name"]).unwrap_or_default(),
        avatar_url: raw
            .get("avatarUrl")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(str::to_owned),
        signature: raw
            .get("signature")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|signature| !signature.is_empty())
            .map(str::to_owned),
        followed: raw.get("followed").and_then(json_bool),
        mutual: raw.get("mutual").and_then(json_bool),
        extensions,
    })
}

fn map_netease_comment_reaction_page(
    request: &CommentReactionListRequest,
    response: Value,
) -> Result<CommentReactionPage> {
    let data = netease_comment_reaction_data(&response)?;
    let raw_reactions = data
        .get("hugComments")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase comment hug response is missing hugComments",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "response": response }))
        })?;
    let reactions = raw_reactions
        .iter()
        .cloned()
        .map(map_netease_comment_hug)
        .collect::<Result<Vec<_>>>()?;
    let current_comment = map_optional_comment(data.get("currentComment"))?;
    let total = data
        .get("hugTotalCounts")
        .or_else(|| data.get("total"))
        .or_else(|| data.get("count"))
        .and_then(json_u64);
    let has_more = data.get("hasMore").and_then(json_bool).unwrap_or(false);
    let offset = request.page.saturating_sub(1).saturating_mul(request.limit);
    let consumed = u32::try_from(reactions.len()).unwrap_or(u32::MAX);
    let mut pagination_extensions = Extensions::new();
    pagination_extensions.insert("mode".to_owned(), json!("reaction_hug"));
    pagination_extensions.insert("page".to_owned(), json!(request.page));
    pagination_extensions.insert("returned_count".to_owned(), json!(reactions.len()));
    pagination_extensions.insert(
        "limit_applied".to_owned(),
        json!(reactions.len() <= request.limit as usize),
    );
    insert_extension(
        &mut pagination_extensions,
        "requested_cursor",
        request.cursor.clone(),
    );
    insert_extension(
        &mut pagination_extensions,
        "requested_id_cursor",
        request.id_cursor.clone(),
    );
    insert_extension(
        &mut pagination_extensions,
        "next_cursor",
        data.get("cursor").and_then(json_scalar_string),
    );
    insert_extension(
        &mut pagination_extensions,
        "next_id_cursor",
        data.get("idCursor").and_then(json_scalar_string),
    );
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), response);
    Ok(CommentReactionPage {
        target: request.target.clone(),
        comment_id: request.comment_id.trim().to_owned(),
        target_user_ref: request.target_user_ref.clone(),
        kind: CommentReactionKind::Hug,
        reactions,
        current_comment,
        pagination: PageMeta {
            limit: request.limit,
            offset,
            total,
            next_offset: (has_more && consumed > 0).then_some(offset.saturating_add(consumed)),
            has_more,
            extensions: pagination_extensions,
        },
        extensions,
    })
}

fn netease_comment_reaction_data(response: &Value) -> Result<&Value> {
    let mut candidate = response;
    for _ in 0..=2 {
        if candidate.get("hugComments").is_some() {
            return Ok(candidate);
        }
        let Some(data) = candidate.get("data").filter(|data| data.is_object()) else {
            break;
        };
        candidate = data;
    }
    Err(TuneWeaveError::new(
        ErrorCode::UpstreamError,
        "NetEase comment hug response is missing its data container",
    )
    .with_platform(Platform::Netease)
    .with_details(json!({ "response": response })))
}

fn map_netease_comment_hug(raw: Value) -> Result<CommentReaction> {
    let user = raw
        .get("user")
        .and_then(map_netease_comment_user)
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase comment hug is missing a usable user",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "hug": raw }))
        })?;
    let content = raw
        .get("hugContent")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw);
    Ok(CommentReaction {
        kind: CommentReactionKind::Hug,
        user,
        content,
        extensions,
    })
}

fn map_netease_comment_reaction_mutation(
    request: &CommentReactionMutationRequest,
    response: Value,
) -> Result<CommentReactionMutationResult> {
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), response);
    Ok(CommentReactionMutationResult {
        target: request.target.clone(),
        comment_id: request.comment_id.trim().to_owned(),
        kind: request.kind,
        active: request.active,
        target_user_ref: request.target_user_ref.clone(),
        extensions,
    })
}

fn map_netease_comment_report(
    request: &CommentReportRequest,
    response: Value,
) -> Result<CommentReportResult> {
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), response);
    Ok(CommentReportResult {
        target: request.target.clone(),
        comment_id: request.comment_id.trim().to_owned(),
        reason: request.reason.clone(),
        submitted: true,
        extensions,
    })
}

fn map_netease_comment_thread_stats(
    request: &CommentThreadStatsRequest,
    response: Value,
) -> Result<CommentThreadStatsBatch> {
    let raw_stats = response
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase comment stats response is missing its data array",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "response": response }))
        })?;
    let stats = raw_stats
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, raw)| {
            map_netease_comment_thread_stat(
                request.kind,
                request.resource_refs.get(index).cloned(),
                raw,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let mut extensions = Extensions::new();
    extensions.insert(
        "resource_type".to_owned(),
        json!(netease_comment_resource_type(request.kind)),
    );
    extensions.insert("returned_count".to_owned(), json!(stats.len()));
    extensions.insert("response".to_owned(), response);
    Ok(CommentThreadStatsBatch {
        kind: request.kind,
        requested_refs: request.resource_refs.clone(),
        stats,
        extensions,
    })
}

fn map_netease_comment_thread_stat(
    kind: CommentTargetKind,
    requested_ref: Option<ResourceRef>,
    raw: Value,
) -> Result<CommentThreadStats> {
    let target = netease_comment_stats_target(kind, &raw)?;
    let latest_liked_users = map_comment_stats_users(&raw)?;
    let comments = map_comment_array(&raw, "comments")?;
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw.clone());
    Ok(CommentThreadStats {
        target,
        requested_ref,
        liked: raw.get("liked").and_then(json_bool),
        like_count: raw.get("likedCount").and_then(json_u64),
        comment_count: raw.get("commentCount").and_then(json_u64),
        comment_count_text: raw
            .get("commentCountDesc")
            .and_then(json_scalar_string)
            .filter(|value| !value.trim().is_empty()),
        share_count: raw.get("shareCount").and_then(json_u64),
        comment_upgraded: raw.get("commentUpgraded").and_then(json_bool),
        musician_comment_count: raw.get("musicianSaidCount").and_then(json_u64),
        latest_liked_users,
        comments,
        extensions,
    })
}

fn netease_comment_stats_target(kind: CommentTargetKind, raw: &Value) -> Result<CommentTarget> {
    let thread_id = raw
        .get("threadId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase comment stats item is missing threadId",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "stats": raw }))
        })?;
    let prefix = match kind {
        CommentTargetKind::Track => "R_SO_4_",
        CommentTargetKind::Mv => "R_MV_5_",
        CommentTargetKind::Playlist => "A_PL_0_",
        CommentTargetKind::Album => "R_AL_3_",
        CommentTargetKind::RadioEpisode => "A_DJ_1_",
        CommentTargetKind::Video => "R_VI_62_",
        CommentTargetKind::Event => "A_EV_2_",
        CommentTargetKind::RadioStation => "A_DR_14_",
    };
    let suffix = thread_id.strip_prefix(prefix).ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase comment stats returned a mismatched thread type",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "kind": kind, "thread_id": thread_id }))
    })?;
    if suffix.is_empty() {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase comment stats returned an empty thread resource id",
        )
        .with_platform(Platform::Netease));
    }
    let id = if kind == CommentTargetKind::Event {
        thread_id
    } else {
        suffix
    };
    let resource_ref = ResourceRef::new(Platform::Netease, id).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase comment stats returned an invalid resource id: {error}"),
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "thread_id": thread_id }))
    })?;
    Ok(CommentTarget::new(resource_ref, kind))
}

fn map_comment_stats_users(raw: &Value) -> Result<Vec<User>> {
    let Some(value) = raw.get("latestLikedUsers") else {
        return Ok(Vec::new());
    };
    if value.is_null() {
        return Ok(Vec::new());
    }
    let users = value.as_array().ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase comment stats latestLikedUsers is not an array",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "stats": raw }))
    })?;
    users
        .iter()
        .map(|user| {
            map_netease_comment_user(user).ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "NetEase comment stats contains an invalid liked user",
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "user": user }))
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NeteaseApiProtocol {
    Eapi,
    Weapi,
    Api,
    Linuxapi,
    Xeapi,
}

impl NeteaseApiProtocol {
    fn parse(value: Option<&str>) -> Result<Self> {
        match value.map(str::trim).filter(|value| !value.is_empty()) {
            None | Some("eapi") => Ok(Self::Eapi),
            Some("weapi") => Ok(Self::Weapi),
            Some("api") => Ok(Self::Api),
            Some("linuxapi") => Ok(Self::Linuxapi),
            Some("xeapi") => Ok(Self::Xeapi),
            Some(value) => Err(TuneWeaveError::invalid_request(format!(
                "unsupported NetEase API protocol: {value}"
            ))
            .with_platform(Platform::Netease)
            .with_details(json!({
                "protocol": value,
                "supported": ["eapi", "weapi", "api", "linuxapi", "xeapi"]
            }))),
        }
    }
}

fn validate_platform_api_request(request: &PlatformApiRequest) -> Result<&str> {
    let uri = request.uri.as_str();
    validate_netease_api_uri(uri)?;
    let data = request.data.as_object().ok_or_else(|| {
        TuneWeaveError::invalid_request("NetEase extension API data must be a JSON object")
            .with_platform(Platform::Netease)
    })?;
    if data.contains_key("cookie") {
        return Err(TuneWeaveError::invalid_request(
            "NetEase extension API does not accept Cookie data; select a stored account alias",
        )
        .with_platform(Platform::Netease));
    }
    Ok(uri)
}

fn validate_platform_batch_request(request: &PlatformBatchRequest) -> Result<()> {
    if request.requests.is_empty() {
        return Err(TuneWeaveError::invalid_request(
            "NetEase batch requires at least one /api/... request",
        )
        .with_platform(Platform::Netease));
    }
    for uri in request.requests.keys() {
        validate_netease_api_uri(uri)?;
    }
    Ok(())
}

fn serialize_netease_batch_requests(request: &PlatformBatchRequest) -> Value {
    let mut data = request
        .requests
        .iter()
        .map(|(uri, data)| {
            let data = match data {
                Value::String(data) => data.clone(),
                data => data.to_string(),
            };
            (uri.clone(), Value::String(data))
        })
        .collect::<serde_json::Map<_, _>>();
    data.insert("e_r".to_owned(), Value::Bool(request.encrypted_response));
    Value::Object(data)
}

fn validate_netease_api_uri(uri: &str) -> Result<()> {
    if uri.trim() != uri {
        return Err(TuneWeaveError::invalid_request(
            "NetEase API uri cannot contain surrounding whitespace",
        )
        .with_platform(Platform::Netease));
    }
    if !uri.starts_with("/api/") || uri.len() == "/api/".len() {
        return Err(TuneWeaveError::invalid_request(
            "NetEase API uri must start with /api/ and name an endpoint",
        )
        .with_platform(Platform::Netease));
    }
    if uri.contains(['\r', '\n', '#', '\\']) || uri.contains("://") {
        return Err(TuneWeaveError::invalid_request(
            "NetEase API uri contains a forbidden character",
        )
        .with_platform(Platform::Netease));
    }
    let path = uri.split_once('?').map_or(uri, |(path, _)| path);
    if path
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(TuneWeaveError::invalid_request(
            "NetEase API uri cannot contain dot path segments",
        )
        .with_platform(Platform::Netease));
    }
    Ok(())
}

fn ensure_platform_api_success(body: &Value) -> Result<()> {
    let code = body["code"]
        .as_i64()
        .or_else(|| body["code"].as_str().and_then(|code| code.parse().ok()));
    if code.is_none_or(|code| matches!(code, 200 | 201 | 302 | 400 | 502 | 800..=803)) {
        return Ok(());
    }
    ensure_success(body)
}

async fn authenticated_user_id(client: &NeteaseClient, account: &str) -> Result<String> {
    let status = client.session_status().await?;
    if !status.authenticated {
        return Err(TuneWeaveError::new(
            ErrorCode::AuthenticationRequired,
            format!("NetEase account alias {account} is not logged in"),
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "account": account })));
    }
    usable_owned_resource_id(status.account.user_id)
        .or_else(|| usable_owned_resource_id(status.account.id))
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase account status did not contain a user id",
            )
            .with_platform(Platform::Netease)
        })
}

async fn fetch_album_content(client: &NeteaseClient, id: u64) -> Result<AlbumEnvelope> {
    let response = client
        .request_weapi(&format!("/api/v1/album/{id}"), json!({}))
        .await?;
    ensure_success(&response.body)?;
    parse_body(response.body)
}

async fn fetch_tracks_by_ids(client: &NeteaseClient, ids: &[u64]) -> Result<Vec<Track>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let request_tracks =
        Value::Array(ids.iter().map(|id| json!({ "id": id })).collect()).to_string();
    let response = client
        .request_eapi("/api/v3/song/detail", json!({ "c": request_tracks }))
        .await?;
    ensure_success(&response.body)?;
    let response: TrackEnvelope = parse_body(response.body)?;
    let mut songs = response
        .songs
        .into_iter()
        .map(|song| (song.id, song))
        .collect::<HashMap<_, _>>();
    let mut privileges = response
        .privileges
        .into_iter()
        .map(|privilege| (privilege.id, privilege))
        .collect::<HashMap<_, _>>();
    ids.iter()
        .filter_map(|id| {
            songs
                .remove(id)
                .map(|song| map_song(song, privileges.remove(id)))
        })
        .collect()
}

async fn fetch_favorite_tracks(
    client: &NeteaseClient,
    user_id: &str,
    request: &PageRequest,
) -> Result<Page<Track>> {
    let response = client
        .request_eapi("/api/song/like/get", json!({ "uid": user_id }))
        .await?;
    ensure_success(&response.body)?;
    let response: LikedTracksEnvelope = parse_body(response.body)?;
    let limit = request.limit.clamp(1, 100);
    let (selected_ids, pagination) = select_page(response.ids, limit, request.offset);
    let items = fetch_tracks_by_ids(client, &selected_ids).await?;
    Ok(Page { items, pagination })
}

async fn fetch_play_history(
    client: &NeteaseClient,
    user_id: &str,
    request: &PlaybackHistoryRequest,
) -> Result<Page<PlaybackHistoryEntry>> {
    let history_type = match request.period {
        PlaybackHistoryPeriod::AllTime => 0,
        PlaybackHistoryPeriod::Week => 1,
    };
    let response = client
        .request_weapi(
            "/api/v1/play/record",
            json!({ "uid": user_id, "type": history_type }),
        )
        .await?;
    ensure_account_access(client, &response.body, "play history")?;
    let response: PlayHistoryEnvelope = parse_body(response.body)?;
    let records = match request.period {
        PlaybackHistoryPeriod::AllTime => response.all_data,
        PlaybackHistoryPeriod::Week => response.week_data,
    };
    let limit = request.limit.clamp(1, 100);
    let (records, pagination) = select_page(records, limit, request.offset);
    let items = records
        .into_iter()
        .map(map_play_history_record)
        .collect::<Result<Vec<_>>>()?;
    Ok(Page { items, pagination })
}

fn map_play_history_record(record: PlayHistoryRecord) -> Result<PlaybackHistoryEntry> {
    Ok(PlaybackHistoryEntry {
        track: map_song(record.song, None)?,
        play_count: record.play_count,
        score: record.score,
        last_played_at: None,
        extensions: Extensions::new(),
    })
}

fn map_recommended_tracks(
    response: RecommendedTracksEnvelope,
    limit: u32,
    offset: u32,
) -> Result<Page<Track>> {
    let mut reasons = response
        .data
        .recommend_reasons
        .into_iter()
        .map(|reason| (reason.song_id, reason))
        .collect::<HashMap<_, _>>();
    let tracks = response
        .data
        .daily_songs
        .into_iter()
        .map(|song| {
            let song_id = song.id;
            let mut track = map_song(song, None)?;
            if let Some(reason) = reasons.remove(&song_id) {
                insert_recommendation_reason(&mut track.extensions, reason);
            }
            Ok(track)
        })
        .collect::<Result<Vec<_>>>()?;
    let limit = limit.clamp(1, 100);
    let (items, pagination) = select_page(tracks, limit, offset);
    Ok(Page { items, pagination })
}

fn insert_recommendation_reason(extensions: &mut Extensions, reason: RecommendationReason) {
    extensions.insert(
        "recommendation".to_owned(),
        json!({
            "reason": reason.reason,
            "reason_id": reason.reason_id,
            "target_url": reason.target_url,
        }),
    );
}

fn map_recommended_playlists(
    response: RecommendedPlaylistsEnvelope,
    limit: u32,
    offset: u32,
) -> Result<Page<Playlist>> {
    let playlists = response
        .recommend
        .into_iter()
        .map(map_playlist)
        .collect::<Result<Vec<_>>>()?;
    let limit = limit.clamp(1, 100);
    let (items, pagination) = select_page(playlists, limit, offset);
    Ok(Page { items, pagination })
}

fn map_audio_recognition(
    response: AudioMatchEnvelope,
    raw_response: Value,
) -> Result<AudioRecognition> {
    let query_id = response.data.query_id.as_ref().and_then(json_scalar_string);
    let matches = response
        .data
        .result
        .unwrap_or_default()
        .into_iter()
        .map(|raw| {
            let song_raw = raw.get("song").cloned().ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "NetEase audio match result is missing song",
                )
                .with_platform(Platform::Netease)
            })?;
            let song: Song = parse_body(song_raw.clone())?;
            let mut track = map_song(song, None)?;
            track
                .extensions
                .insert("audio_recognition_song".to_owned(), song_raw);
            let start_time_ms = ["startTime", "start_time"].into_iter().find_map(|field| {
                raw.get(field).and_then(|value| {
                    value
                        .as_u64()
                        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
                })
            });
            let mut extensions = Extensions::new();
            extensions.insert("match".to_owned(), raw);
            Ok(AudioRecognitionMatch {
                track,
                start_time_ms,
                extensions,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut extensions = Extensions::new();
    insert_extension(&mut extensions, "type", response.data.kind);
    insert_extension(&mut extensions, "mv", response.data.mv);
    insert_extension(&mut extensions, "module_list", response.data.module_list);
    extensions.insert("response".to_owned(), raw_response);
    Ok(AudioRecognition {
        matches,
        query_id,
        no_match_reason: response.data.no_match_reason,
        extensions,
    })
}

fn netease_banner_client(client: BannerClient) -> &'static str {
    match client {
        BannerClient::Pc => "pc",
        BannerClient::Android => "android",
        BannerClient::Iphone => "iphone",
        BannerClient::Ipad => "ipad",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum NeteaseBannerProtocol {
    Eapi,
    Weapi,
}

fn netease_banner_request(
    request: &BannerListRequest,
) -> Result<(&'static str, Value, NeteaseBannerProtocol)> {
    match request.catalog {
        BannerCatalog::Music => Ok((
            "/api/v2/banner/get",
            json!({ "clientType": netease_banner_client(request.client) }),
            NeteaseBannerProtocol::Eapi,
        )),
        BannerCatalog::Podcast if request.client == BannerClient::Pc => Ok((
            "/api/djradio/banner/get",
            json!({}),
            NeteaseBannerProtocol::Weapi,
        )),
        BannerCatalog::Podcast => Err(TuneWeaveError::invalid_request(
            "the NetEase podcast banner catalog does not support client selection",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "catalog": request.catalog, "client": request.client }))),
    }
}

fn map_banner(raw: Value, client: BannerClient) -> Result<Banner> {
    map_banner_with_context(raw, BannerCatalog::Music, Some(client))
}

fn map_podcast_banner(raw: Value) -> Result<Banner> {
    map_banner_with_context(raw, BannerCatalog::Podcast, None)
}

fn map_banner_with_context(
    raw: Value,
    catalog: BannerCatalog,
    client: Option<BannerClient>,
) -> Result<Banner> {
    let image_url =
        radio_text_field(&raw, &["bigImageUrl", "pic", "imageUrl"]).ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase banner did not contain an image URL",
            )
            .with_platform(Platform::Netease)
        })?;
    let target_type = raw.get("targetType").and_then(json_i64);
    let target_kind = match target_type {
        Some(1) => BannerTargetKind::Track,
        Some(10) => BannerTargetKind::Album,
        Some(100) => BannerTargetKind::Artist,
        Some(1_000) => BannerTargetKind::Playlist,
        Some(1_004) => BannerTargetKind::Video,
        Some(60_001) => BannerTargetKind::PodcastEpisode,
        Some(3_000) => BannerTargetKind::Web,
        _ => BannerTargetKind::Unknown,
    };
    let target_ref = raw
        .get("targetId")
        .and_then(json_scalar_string)
        .filter(|id| id != "0")
        .map(|id| ResourceRef::new(Platform::Netease, id))
        .transpose()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase banner returned an invalid target id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let mut extensions = Extensions::new();
    extensions.insert("catalog".to_owned(), json!(catalog));
    insert_extension(&mut extensions, "client", client);
    extensions.insert("banner".to_owned(), raw.clone());
    Ok(Banner {
        id: ["bannerId", "adid"]
            .into_iter()
            .find_map(|field| raw.get(field).and_then(json_scalar_string)),
        title: radio_text_field(&raw, &["mainTitle", "typeTitle"]),
        image_url,
        target_ref,
        target_kind,
        url: raw
            .get("url")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|url| !url.is_empty())
            .map(str::to_owned),
        exclusive: raw.get("exclusive").and_then(json_bool),
        extensions,
    })
}

fn map_radio_catalog_option(raw: Value, kind: &str) -> Result<RadioCatalogOption> {
    let id = raw.get("id").and_then(json_scalar_string).ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase broadcast {kind} did not contain an id"),
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "option": raw.clone() }))
    })?;
    let name = raw
        .get("name")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase broadcast {kind} did not contain a name"),
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "option": raw.clone() }))
        })?;
    let mut extensions = Extensions::new();
    extensions.insert("broadcast_option".to_owned(), raw);
    Ok(RadioCatalogOption {
        id,
        name,
        extensions,
    })
}

fn netease_radio_station_subscription_payload(id: u64, subscribed: bool) -> Value {
    json!({
        "contentType": "BROADCAST",
        "contentId": id.to_string(),
        "cancelCollect": if subscribed { "false" } else { "true" }
    })
}

fn netease_podcast_categories_request() -> (&'static str, Value) {
    ("/api/djradio/category/get", json!({}))
}

fn netease_podcast_catalog_request(request: &PodcastListRequest) -> Result<(&'static str, Value)> {
    if !(1..=100).contains(&request.limit) {
        return Err(TuneWeaveError::invalid_request(
            "podcast catalog limit must be between 1 and 100",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "limit": request.limit })));
    }
    if request.page.is_some() && request.catalog != PodcastCatalog::TodayPreferred {
        return Err(TuneWeaveError::invalid_request(format!(
            "the NetEase {} podcast catalog does not accept page",
            podcast_catalog_name(request.catalog)
        ))
        .with_platform(Platform::Netease)
        .with_details(json!({ "catalog": request.catalog, "page": request.page })));
    }
    match request.catalog {
        PodcastCatalog::Featured => {
            if request.category_id.is_some() {
                return Err(TuneWeaveError::invalid_request(
                    "the NetEase featured podcast catalog does not accept category_id",
                )
                .with_platform(Platform::Netease)
                .with_details(
                    json!({ "catalog": request.catalog, "category_id": request.category_id }),
                ));
            }
            if request.offset != 0 {
                return Err(TuneWeaveError::invalid_request(
                    "the NetEase featured podcast catalog is a fixed snapshot and requires offset=0",
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "catalog": request.catalog, "offset": request.offset })));
            }
            Ok(("/api/djradio/recommend/v1", json!({})))
        }
        PodcastCatalog::Hot => {
            if request.category_id.is_some() {
                return Err(TuneWeaveError::invalid_request(
                    "the NetEase hot podcast catalog does not accept category_id",
                )
                .with_platform(Platform::Netease)
                .with_details(
                    json!({ "catalog": request.catalog, "category_id": request.category_id }),
                ));
            }
            Ok((
                "/api/djradio/hot/v1",
                json!({ "limit": request.limit, "offset": request.offset }),
            ))
        }
        PodcastCatalog::CategoryFeatured => {
            let category_id = request.category_id.as_deref().ok_or_else(|| {
                TuneWeaveError::invalid_request(
                    "the NetEase category featured podcast catalog requires category_id",
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "catalog": request.catalog }))
            })?;
            let category_id = parse_numeric_id("podcast category", category_id)?.to_string();
            if request.offset != 0 {
                return Err(TuneWeaveError::invalid_request(
                    "the NetEase category featured podcast catalog is a fixed snapshot and requires offset=0",
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "catalog": request.catalog, "offset": request.offset })));
            }
            Ok(("/api/djradio/recommend", json!({ "cateId": category_id })))
        }
        PodcastCatalog::CategoryHot => {
            let category_id = request.category_id.as_deref().ok_or_else(|| {
                TuneWeaveError::invalid_request(
                    "the NetEase category hot podcast catalog requires category_id",
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "catalog": request.catalog }))
            })?;
            let category_id = parse_numeric_id("podcast category", category_id)?.to_string();
            Ok((
                "/api/djradio/hot",
                json!({
                    "cateId": category_id,
                    "limit": request.limit,
                    "offset": request.offset
                }),
            ))
        }
        PodcastCatalog::Personalized => {
            if request.category_id.is_some() {
                return Err(TuneWeaveError::invalid_request(
                    "the NetEase personalized podcast catalog does not accept category_id",
                )
                .with_platform(Platform::Netease)
                .with_details(
                    json!({ "catalog": request.catalog, "category_id": request.category_id }),
                ));
            }
            if request.offset != 0 {
                return Err(TuneWeaveError::invalid_request(
                    "the NetEase personalized podcast catalog is a head recommendation and requires offset=0",
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "catalog": request.catalog, "offset": request.offset })));
            }
            Ok((
                "/api/djradio/personalize/rcmd",
                json!({ "limit": request.limit }),
            ))
        }
        PodcastCatalog::TodayPreferred => {
            if request.category_id.is_some() {
                return Err(TuneWeaveError::invalid_request(
                    "the NetEase today preferred podcast catalog does not accept category_id",
                )
                .with_platform(Platform::Netease)
                .with_details(
                    json!({ "catalog": request.catalog, "category_id": request.category_id }),
                ));
            }
            if request.offset != 0 {
                return Err(TuneWeaveError::invalid_request(
                    "the NetEase today preferred podcast catalog uses page and requires offset=0",
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "catalog": request.catalog, "offset": request.offset })));
            }
            Ok((
                "/api/djradio/home/today/perfered",
                json!({ "page": request.page.unwrap_or(0) }),
            ))
        }
        PodcastCatalog::Paid => {
            if request.category_id.is_some() {
                return Err(TuneWeaveError::invalid_request(
                    "the NetEase paid podcast catalog does not accept category_id",
                )
                .with_platform(Platform::Netease)
                .with_details(
                    json!({ "catalog": request.catalog, "category_id": request.category_id }),
                ));
            }
            Ok((
                "/api/djradio/home/paygift/list",
                json!({
                    "limit": request.limit,
                    "offset": request.offset,
                    "_nmclfl": 1
                }),
            ))
        }
    }
}

const fn podcast_catalog_name(catalog: PodcastCatalog) -> &'static str {
    match catalog {
        PodcastCatalog::Featured => "featured",
        PodcastCatalog::Hot => "hot",
        PodcastCatalog::CategoryFeatured => "category_featured",
        PodcastCatalog::CategoryHot => "category_hot",
        PodcastCatalog::Personalized => "personalized",
        PodcastCatalog::TodayPreferred => "today_preferred",
        PodcastCatalog::Paid => "paid",
    }
}

fn netease_podcast_request(id: u64) -> (&'static str, Value) {
    ("/api/djradio/v2/get", json!({ "id": id }))
}

fn netease_podcast_chart_request(request: &PodcastChartRequest) -> Result<(&'static str, Value)> {
    if !(1..=100).contains(&request.limit) {
        return Err(TuneWeaveError::invalid_request(
            "podcast chart limit must be between 1 and 100",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "limit": request.limit })));
    }
    match request.kind {
        PodcastChartKind::New => Ok((
            "/api/djradio/toplist",
            json!({ "limit": request.limit, "offset": request.offset, "type": "0" }),
        )),
        PodcastChartKind::Hot => Ok((
            "/api/djradio/toplist",
            json!({ "limit": request.limit, "offset": request.offset, "type": 1 }),
        )),
        PodcastChartKind::Paid => {
            if request.offset != 0 {
                return Err(TuneWeaveError::invalid_request(
                    "the NetEase paid podcast chart does not support offset",
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "kind": request.kind, "offset": request.offset })));
            }
            Ok((
                "/api/djradio/toplist/pay",
                json!({ "limit": request.limit }),
            ))
        }
    }
}

fn netease_podcast_creator_chart_request(
    request: &PodcastCreatorChartRequest,
) -> Result<(&'static str, Value)> {
    if !(1..=100).contains(&request.limit) {
        return Err(TuneWeaveError::invalid_request(
            "podcast creator chart limit must be between 1 and 100",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "limit": request.limit })));
    }
    match request.kind {
        PodcastCreatorChartKind::Newcomer => Ok((
            "/api/dj/toplist/newcomer",
            json!({ "limit": request.limit, "offset": request.offset }),
        )),
        PodcastCreatorChartKind::Popular | PodcastCreatorChartKind::Trending24Hours => {
            if request.offset != 0 {
                return Err(TuneWeaveError::invalid_request(
                    "this NetEase podcast creator chart does not support offset",
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "kind": request.kind, "offset": request.offset })));
            }
            Ok((
                match request.kind {
                    PodcastCreatorChartKind::Popular => "/api/dj/toplist/popular",
                    PodcastCreatorChartKind::Trending24Hours => "/api/dj/toplist/hours",
                    PodcastCreatorChartKind::Newcomer => unreachable!(),
                },
                json!({ "limit": request.limit }),
            ))
        }
    }
}

fn netease_podcast_subscription_request(id: u64, subscribed: bool) -> (&'static str, Value) {
    (
        if subscribed {
            "/api/djradio/sub"
        } else {
            "/api/djradio/unsub"
        },
        json!({ "id": id }),
    )
}

fn netease_podcast_workbench_request(id: u64) -> (&'static str, Value) {
    ("/api/voice/workbench/voicelist/detail", json!({ "id": id }))
}

fn netease_subscribed_podcasts_request(limit: u32, offset: u32) -> (&'static str, Value) {
    (
        "/api/djradio/get/subed",
        json!({ "limit": limit, "offset": offset, "total": true }),
    )
}

fn netease_podcast_episodes_request(
    id: u64,
    request: &PodcastEpisodeListRequest,
    limit: u32,
) -> (&'static str, Value) {
    (
        "/api/dj/program/byradio",
        json!({
            "radioId": id,
            "limit": limit,
            "offset": request.offset,
            "asc": request.ascending
        }),
    )
}

fn netease_podcast_episode_chart_request(
    request: &PodcastEpisodeChartRequest,
) -> Result<(&'static str, Value)> {
    if !(1..=100).contains(&request.limit) {
        return Err(TuneWeaveError::invalid_request(
            "podcast episode chart limit must be between 1 and 100",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "limit": request.limit })));
    }
    match request.kind {
        PodcastEpisodeChartKind::Popular => Ok((
            "/api/program/toplist/v1",
            json!({ "limit": request.limit, "offset": request.offset }),
        )),
        PodcastEpisodeChartKind::Trending24Hours => {
            if request.offset != 0 {
                return Err(TuneWeaveError::invalid_request(
                    "the NetEase 24-hour podcast episode chart does not support offset",
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "kind": request.kind, "offset": request.offset })));
            }
            Ok((
                "/api/djprogram/toplist/hours",
                json!({ "limit": request.limit }),
            ))
        }
    }
}

fn netease_podcast_episode_request(id: u64) -> (&'static str, Value) {
    ("/api/dj/program/detail", json!({ "id": id }))
}

fn netease_podcast_episode_workbench_request(id: u64) -> (&'static str, Value) {
    ("/api/voice/workbench/voice/detail", json!({ "id": id }))
}

fn netease_podcast_episode_lyrics_request(id: u64) -> (&'static str, Value) {
    ("/api/voice/lyric/get", json!({ "programId": id }))
}

fn netease_radio_station_list_payload(request: &RadioStationListRequest) -> Result<Value> {
    let category_id = request
        .category_id
        .as_deref()
        .map(|id| parse_numeric_id("broadcast category", id).map(|id| id.to_string()))
        .transpose()?
        .unwrap_or_else(|| "0".to_owned());
    let region_id = request
        .region_id
        .as_deref()
        .map(|id| parse_numeric_id("broadcast region", id).map(|id| id.to_string()))
        .transpose()?
        .unwrap_or_else(|| "0".to_owned());
    let (last_id, score) = match &request.cursor {
        Some(cursor) => (
            parse_numeric_id("broadcast station cursor", &cursor.id)?.to_string(),
            cursor.score.to_string(),
        ),
        None => ("0".to_owned(), "-1".to_owned()),
    };
    Ok(json!({
        "categoryId": category_id,
        "regionId": region_id,
        "limit": request.limit.max(1).to_string(),
        "lastId": last_id,
        "score": score
    }))
}

fn map_radio_station_list_response(
    body: Value,
    request: &RadioStationListRequest,
) -> Result<Page<RadioStation>> {
    let data = body
        .get("data")
        .filter(|value| value.is_object())
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase broadcast station catalog did not contain data",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "response": body.clone() }))
        })?;
    let raw_items = data
        .get("list")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase broadcast station catalog did not contain a list",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "response": body.clone() }))
        })?;
    let total = data.get("total").and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    });
    let upstream_has_more = data.get("hasMore").and_then(json_bool).unwrap_or(false);
    let next_cursor = if upstream_has_more {
        let last = raw_items.last().ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase broadcast station catalog has more items but no cursor source",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "response": body.clone() }))
        })?;
        let id = radio_scalar_field(last, &["id", "channelId"])
            .ok_or_else(|| radio_station_cursor_error("id", last))?;
        let score = last
            .get("score")
            .and_then(json_i64)
            .ok_or_else(|| radio_station_cursor_error("score", last))?;
        Some(RadioStationCursor { id, score })
    } else {
        None
    };
    let items = raw_items
        .into_iter()
        .map(|raw| {
            let mut station = map_radio_station_fields(&raw, &raw, None)?;
            station
                .extensions
                .insert("broadcast_station".to_owned(), raw);
            Ok(station)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut extensions = Extensions::new();
    if let Some(cursor) = &next_cursor {
        extensions.insert("next_cursor".to_owned(), json!(cursor));
    }
    extensions.insert("requested_offset".to_owned(), json!(request.offset));
    extensions.insert("offset_applied".to_owned(), json!(false));
    extensions.insert("response".to_owned(), body);
    Ok(Page {
        items,
        pagination: PageMeta {
            limit: request.limit.max(1),
            offset: 0,
            total,
            next_offset: None,
            has_more: next_cursor.is_some(),
            extensions,
        },
    })
}

fn radio_station_cursor_error(field: &str, raw: &Value) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("NetEase broadcast station cursor did not contain a {field}"),
    )
    .with_platform(Platform::Netease)
    .with_details(json!({ "station": raw }))
}

fn netease_radio_collection_payload(limit: u32, offset: u32) -> Value {
    json!({
        "contentType": "BROADCAST",
        "limit": limit.to_string(),
        "offset": offset.to_string(),
        "timeReverseOrder": "true",
        "startDate": "4762584922000"
    })
}

fn map_radio_collection_response(
    body: Value,
    limit: u32,
    offset: u32,
) -> Result<Page<RadioStation>> {
    let raw_items = radio_collection_items(&body).ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase broadcast station collection did not contain a list",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "response": body.clone() }))
    })?;
    let items = raw_items
        .into_iter()
        .map(map_collected_radio_station)
        .collect::<Result<Vec<_>>>()?;
    let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let next_offset = offset.saturating_add(consumed);
    let total = radio_collection_scalar(&body, &["total", "count"]).and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    });
    let explicit_more = radio_collection_scalar(&body, &["hasMore", "more"]).and_then(json_bool);
    let has_more = if consumed == 0 {
        false
    } else if let Some(has_more) = explicit_more {
        has_more
    } else if let Some(total) = total {
        u64::from(next_offset) < total
    } else {
        consumed >= limit
    };
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), body);
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total,
            next_offset: has_more.then_some(next_offset),
            has_more,
            extensions,
        },
    })
}

fn radio_collection_items(body: &Value) -> Option<Vec<Value>> {
    let data = body.get("data").unwrap_or(&Value::Null);
    let nested_data = data.get("data").unwrap_or(&Value::Null);
    let mut first_present = None;
    for container in [data, nested_data, body] {
        if let Some(items) = container.as_array() {
            if first_present.is_none() {
                first_present = Some(items);
            }
            if !items.is_empty() {
                return Some(items.clone());
            }
        }
        for field in ["list", "items", "records", "contents", "channels"] {
            if let Some(items) = container.get(field).and_then(Value::as_array) {
                if first_present.is_none() {
                    first_present = Some(items);
                }
                if !items.is_empty() {
                    return Some(items.clone());
                }
            }
        }
    }
    first_present.cloned()
}

fn preferred_array_field<'a>(container: &'a Value, fields: &[&str]) -> Option<&'a Vec<Value>> {
    let mut first_present = None;
    for field in fields {
        let Some(values) = container.get(*field).and_then(Value::as_array) else {
            continue;
        };
        if first_present.is_none() {
            first_present = Some(values);
        }
        if !values.is_empty() {
            return Some(values);
        }
    }
    first_present
}

fn radio_collection_scalar<'a>(body: &'a Value, fields: &[&str]) -> Option<&'a Value> {
    let data = body.get("data").unwrap_or(&Value::Null);
    let nested_data = data.get("data").unwrap_or(&Value::Null);
    [data, nested_data, body].into_iter().find_map(|container| {
        fields
            .iter()
            .find_map(|field| container.get(field).filter(|value| !value.is_null()))
    })
}

fn map_collected_radio_station(raw: Value) -> Result<RadioStation> {
    let station_raw = embedded_radio_station(&raw);
    let mut station = map_radio_station_fields(&station_raw, &raw, Some(true))?;
    station.extensions.insert("collection_item".to_owned(), raw);
    station
        .extensions
        .insert("broadcast_station".to_owned(), station_raw);
    Ok(station)
}

fn map_radio_station_response(body: Value) -> Result<RadioStation> {
    let raw = body
        .get("data")
        .filter(|value| value.is_object())
        .unwrap_or(&body)
        .clone();
    let station_raw = embedded_radio_station(&raw);
    let mut station = map_radio_station_fields(&station_raw, &raw, None)?;
    station.extensions.insert("current_info".to_owned(), raw);
    station.extensions.insert("response".to_owned(), body);
    Ok(station)
}

fn map_radio_station_fields(
    station_raw: &Value,
    fallback_raw: &Value,
    default_subscribed: Option<bool>,
) -> Result<RadioStation> {
    let id = radio_scalar_field(station_raw, &["id", "channelId", "contentId"])
        .or_else(|| radio_scalar_field(fallback_raw, &["id", "channelId", "contentId"]))
        .ok_or_else(|| radio_station_item_error("id", fallback_raw))?;
    let name = radio_text_field(
        station_raw,
        &["name", "channelName", "contentName", "title"],
    )
    .or_else(|| {
        radio_text_field(
            fallback_raw,
            &["name", "channelName", "contentName", "title"],
        )
    })
    .ok_or_else(|| radio_station_item_error("name", fallback_raw))?;
    let reference = ResourceRef::new(Platform::Netease, &id).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid broadcast station id: {error}"),
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "item": fallback_raw }))
    })?;
    let mut station = RadioStation::new(reference, name);
    station.description = radio_text_field(station_raw, &["description", "desc"])
        .or_else(|| radio_text_field(fallback_raw, &["description", "desc"]))
        .unwrap_or_default();
    station.cover_url = radio_text_field(
        station_raw,
        &["coverUrl", "channelCoverUrl", "picUrl", "imageUrl"],
    )
    .or_else(|| {
        radio_text_field(
            fallback_raw,
            &["coverUrl", "channelCoverUrl", "picUrl", "imageUrl"],
        )
    });
    station.category = radio_text_field(station_raw, &["categoryName", "category"])
        .or_else(|| radio_text_field(fallback_raw, &["categoryName", "category"]));
    station.region = radio_text_field(station_raw, &["regionName", "region"])
        .or_else(|| radio_text_field(fallback_raw, &["regionName", "region"]));
    station.stream_url = radio_text_field(station_raw, &["playUrl", "streamUrl"])
        .or_else(|| radio_text_field(fallback_raw, &["playUrl", "streamUrl"]));
    station.current_program = radio_text_field(station_raw, &["programName", "currentProgram"])
        .or_else(|| radio_text_field(fallback_raw, &["programName", "currentProgram"]));
    station.subscribed = radio_bool_field(station_raw, &["subed", "subscribed", "collected"])
        .or_else(|| radio_bool_field(fallback_raw, &["subed", "subscribed", "collected"]))
        .or(default_subscribed);
    Ok(station)
}

fn embedded_radio_station(raw: &Value) -> Value {
    let mut current = raw.clone();
    for _ in 0..2 {
        let Some(next) = [
            "content",
            "resource",
            "channel",
            "broadcast",
            "contentJson",
            "resourceJson",
            "data",
        ]
        .into_iter()
        .filter_map(|field| current.get(field))
        .find_map(|candidate| match candidate {
            Value::Object(object) if !object.is_empty() => Some(candidate.clone()),
            Value::String(candidate) => serde_json::from_str::<Value>(candidate)
                .ok()
                .filter(|value| value.as_object().is_some_and(|object| !object.is_empty())),
            _ => None,
        }) else {
            break;
        };
        current = next;
    }
    current
}

fn radio_scalar_field(raw: &Value, fields: &[&str]) -> Option<String> {
    fields
        .iter()
        .find_map(|field| raw.get(field).and_then(json_scalar_string))
}

fn radio_text_field(raw: &Value, fields: &[&str]) -> Option<String> {
    fields.iter().find_map(|field| {
        raw.get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

fn radio_bool_field(raw: &Value, fields: &[&str]) -> Option<bool> {
    fields
        .iter()
        .find_map(|field| raw.get(field).and_then(json_bool))
}

fn radio_station_item_error(field: &str, raw: &Value) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("NetEase broadcast station item did not contain a {field}"),
    )
    .with_platform(Platform::Netease)
    .with_details(json!({ "item": raw }))
}

fn map_netease_podcast_categories(body: Value) -> Result<PodcastTaxonomy> {
    ensure_success(&body)?;
    let raw_categories = body
        .get("categories")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| podcast_item_error("categories array", &body))?;
    let categories = raw_categories
        .into_iter()
        .map(map_netease_podcast_category)
        .collect::<Result<Vec<_>>>()?;
    Ok(PodcastTaxonomy {
        categories,
        extensions: Extensions::from([("response".to_owned(), body)]),
    })
}

fn map_netease_podcast_category(raw: Value) -> Result<PodcastCategory> {
    let id = raw
        .get("id")
        .and_then(json_scalar_string)
        .ok_or_else(|| podcast_item_error("category id", &raw))?;
    let name = radio_text_field(&raw, &["name"])
        .ok_or_else(|| podcast_item_error("category name", &raw))?;
    let icon_url = radio_text_field(
        &raw,
        &[
            "picWebUrl",
            "pic96x96Url",
            "pic84x84IdUrl",
            "pic56x56Url",
            "picUWPUrl",
            "picIPadUrl",
            "picMacUrl",
            "picPCWhiteUrl",
            "picPCBlackUrl",
        ],
    );
    Ok(PodcastCategory {
        id,
        name,
        icon_url,
        extensions: Extensions::from([("category".to_owned(), raw)]),
    })
}

fn map_netease_podcast_catalog_response(
    body: Value,
    request: &PodcastListRequest,
) -> Result<Page<Podcast>> {
    ensure_success(&body)?;
    let (raw_items, container) = match request.catalog {
        PodcastCatalog::Personalized | PodcastCatalog::TodayPreferred => {
            (body.get("data"), "data array")
        }
        PodcastCatalog::Paid => (
            body.get("data").and_then(|data| data.get("list")),
            "data.list array",
        ),
        _ => (body.get("djRadios"), "djRadios array"),
    };
    let raw_items = raw_items
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| podcast_item_error(container, &body))?;
    let items = raw_items
        .into_iter()
        .map(map_netease_podcast)
        .collect::<Result<Vec<_>>>()?;
    let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let (total, next_offset, has_more, limit_applied) = match request.catalog {
        PodcastCatalog::Featured => (
            Some(u64::try_from(items.len()).unwrap_or(u64::MAX)),
            None,
            false,
            false,
        ),
        PodcastCatalog::CategoryFeatured => {
            let has_more = consumed > 0 && body.get("hasMore").and_then(json_bool).unwrap_or(false);
            (None, None, has_more, false)
        }
        PodcastCatalog::Hot => {
            let total = body.get("count").and_then(json_u64);
            let candidate_offset = request.offset.saturating_add(consumed);
            let upstream_has_more = body.get("hasMore").and_then(json_bool);
            let has_more = consumed > 0
                && upstream_has_more.unwrap_or_else(|| {
                    total.is_some_and(|total| u64::from(candidate_offset) < total)
                });
            (total, has_more.then_some(candidate_offset), has_more, true)
        }
        PodcastCatalog::CategoryHot => {
            let total = body.get("count").and_then(json_u64);
            let candidate_offset = request.offset.saturating_add(request.limit);
            let upstream_has_more = body.get("hasMore").and_then(json_bool);
            let has_more = consumed > 0
                && upstream_has_more.unwrap_or_else(|| {
                    total.is_some_and(|total| u64::from(candidate_offset) < total)
                });
            (
                total,
                has_more.then_some(candidate_offset),
                has_more,
                consumed <= request.limit,
            )
        }
        PodcastCatalog::Personalized => (None, None, false, true),
        PodcastCatalog::TodayPreferred => (None, None, false, false),
        PodcastCatalog::Paid => {
            let candidate_offset = request.offset.saturating_add(consumed);
            let has_more = consumed > 0
                && body
                    .get("data")
                    .and_then(|data| data.get("hasMore"))
                    .and_then(json_bool)
                    .unwrap_or(false);
            (
                None,
                has_more.then_some(candidate_offset),
                has_more,
                consumed <= request.limit,
            )
        }
    };
    let mut extensions = Extensions::from([
        ("catalog".to_owned(), json!(request.catalog)),
        ("returned_count".to_owned(), json!(items.len())),
        ("limit_applied".to_owned(), json!(limit_applied)),
    ]);
    insert_extension(
        &mut extensions,
        "category_id",
        request.category_id.as_deref(),
    );
    if request.catalog == PodcastCatalog::CategoryFeatured {
        extensions.insert("continuation_supported".to_owned(), json!(false));
    }
    if request.catalog == PodcastCatalog::TodayPreferred {
        extensions.insert("page".to_owned(), json!(request.page.unwrap_or(0)));
        extensions.insert("page_control_supported".to_owned(), json!(true));
    }
    extensions.insert("response".to_owned(), body);
    Ok(Page {
        items,
        pagination: PageMeta {
            limit: request.limit,
            offset: request.offset,
            total,
            next_offset,
            has_more,
            extensions,
        },
    })
}

fn map_netease_podcast_response(body: Value) -> Result<Podcast> {
    ensure_success(&body)?;
    let raw = body
        .get("data")
        .filter(|data| data.is_object())
        .cloned()
        .ok_or_else(|| podcast_item_error("podcast data", &body))?;
    let mut podcast = map_netease_podcast(raw)?;
    podcast.extensions.insert("response".to_owned(), body);
    Ok(podcast)
}

fn map_netease_podcast_workbench_response(body: Value) -> Result<Podcast> {
    ensure_success(&body)?;
    let mut first_mapping_error = None;
    for raw in [
        body.pointer("/data/voiceList"),
        body.pointer("/data/voicelist"),
        body.get("data"),
        body.get("voiceList"),
        body.get("voicelist"),
    ]
    .into_iter()
    .flatten()
    .filter(|value| value.as_object().is_some_and(|object| !object.is_empty()))
    {
        match map_netease_podcast(raw.clone()) {
            Ok(mut podcast) => {
                podcast
                    .extensions
                    .insert("workbench_response".to_owned(), body.clone());
                return Ok(podcast);
            }
            Err(error) if first_mapping_error.is_none() => first_mapping_error = Some(error),
            Err(_) => {}
        }
    }
    Err(first_mapping_error
        .unwrap_or_else(|| podcast_item_error("workbench voice list data", &body)))
}

fn map_netease_podcast_chart(
    body: Value,
    request: &PodcastChartRequest,
) -> Result<Page<PodcastChartEntry>> {
    ensure_success(&body)?;
    let (raw_items, total, update_time_ms, offset_submitted) = match request.kind {
        PodcastChartKind::New | PodcastChartKind::Hot => (
            body.get("toplist").and_then(Value::as_array).cloned(),
            None,
            body.get("updateTime").and_then(json_u64),
            true,
        ),
        PodcastChartKind::Paid => {
            let data = body
                .get("data")
                .filter(|data| data.is_object())
                .ok_or_else(|| podcast_item_error("paid podcast chart data", &body))?;
            (
                data.get("list").and_then(Value::as_array).cloned(),
                data.get("total").and_then(json_u64),
                data.get("updateTime").and_then(json_u64),
                false,
            )
        }
    };
    let raw_items = raw_items.ok_or_else(|| podcast_item_error("podcast chart list", &body))?;
    let entries = raw_items
        .into_iter()
        .map(|raw| map_netease_podcast_chart_entry(raw, request.kind))
        .collect::<Result<Vec<_>>>()?;
    let mut extensions = Extensions::from([
        ("kind".to_owned(), json!(request.kind)),
        ("returned_count".to_owned(), json!(entries.len())),
        (
            "limit_applied".to_owned(),
            json!(entries.len() <= request.limit as usize),
        ),
        ("requested_offset".to_owned(), json!(request.offset)),
        ("offset_submitted".to_owned(), json!(offset_submitted)),
        ("offset_applied".to_owned(), json!(false)),
        ("offset_control_supported".to_owned(), json!(false)),
        ("continuation_supported".to_owned(), json!(false)),
    ]);
    insert_extension(&mut extensions, "update_time_ms", update_time_ms);
    if let Some(updated_at) = update_time_ms.and_then(|value| unix_rfc3339(value / 1_000)) {
        extensions.insert("updated_at".to_owned(), json!(updated_at));
    }
    extensions.insert("response".to_owned(), body);
    Ok(Page {
        items: entries,
        pagination: PageMeta {
            limit: request.limit,
            offset: 0,
            total,
            next_offset: None,
            has_more: false,
            extensions,
        },
    })
}

fn map_netease_podcast_chart_entry(
    raw: Value,
    kind: PodcastChartKind,
) -> Result<PodcastChartEntry> {
    let rank = raw
        .get("rank")
        .and_then(json_u64)
        .and_then(|rank| u32::try_from(rank).ok())
        .filter(|rank| *rank > 0)
        .ok_or_else(|| podcast_item_error("positive podcast chart rank", &raw))?;
    let mut podcast = map_netease_podcast(raw.clone())?;
    if kind == PodcastChartKind::Paid {
        podcast.paid = Some(true);
    }
    Ok(PodcastChartEntry {
        rank,
        previous_rank: raw.get("lastRank").and_then(json_i64),
        score: raw.get("score").and_then(json_u64),
        podcast,
        extensions: Extensions::from([("chart_entry".to_owned(), raw)]),
    })
}

fn map_netease_podcast_creator_chart(
    body: Value,
    request: &PodcastCreatorChartRequest,
) -> Result<Page<PodcastCreatorChartEntry>> {
    ensure_success(&body)?;
    let data = body
        .get("data")
        .filter(|data| data.is_object())
        .ok_or_else(|| podcast_item_error("podcast creator chart data", &body))?;
    let raw_items = data
        .get("list")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| podcast_item_error("podcast creator chart list", &body))?;
    let entries = raw_items
        .into_iter()
        .map(map_netease_podcast_creator_chart_entry)
        .collect::<Result<Vec<_>>>()?;
    let total = data.get("total").and_then(json_u64);
    let update_time_ms = data.get("updateTime").and_then(json_u64);
    let offset_submitted = request.kind == PodcastCreatorChartKind::Newcomer;
    let mut extensions = Extensions::from([
        ("kind".to_owned(), json!(request.kind)),
        ("returned_count".to_owned(), json!(entries.len())),
        (
            "limit_applied".to_owned(),
            json!(entries.len() <= request.limit as usize),
        ),
        ("requested_offset".to_owned(), json!(request.offset)),
        ("offset_submitted".to_owned(), json!(offset_submitted)),
        ("offset_applied".to_owned(), json!(false)),
        ("offset_control_supported".to_owned(), json!(false)),
        ("continuation_supported".to_owned(), json!(false)),
    ]);
    insert_extension(&mut extensions, "update_time_ms", update_time_ms);
    if let Some(updated_at) = update_time_ms.and_then(|value| unix_rfc3339(value / 1_000)) {
        extensions.insert("updated_at".to_owned(), json!(updated_at));
    }
    extensions.insert("response".to_owned(), body);
    Ok(Page {
        items: entries,
        pagination: PageMeta {
            limit: request.limit,
            offset: 0,
            total,
            next_offset: None,
            has_more: false,
            extensions,
        },
    })
}

fn map_netease_podcast_creator_chart_entry(raw: Value) -> Result<PodcastCreatorChartEntry> {
    let id = raw
        .get("id")
        .and_then(usable_resource_id)
        .ok_or_else(|| podcast_item_error("podcast creator id", &raw))?;
    let name = radio_text_field(&raw, &["nickName", "nickname", "name"])
        .ok_or_else(|| podcast_item_error("podcast creator name", &raw))?;
    let resource_ref = ResourceRef::new(Platform::Netease, &id).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid podcast creator id: {error}"),
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "item": raw }))
    })?;
    let rank = raw
        .get("rank")
        .and_then(json_u64)
        .and_then(|rank| u32::try_from(rank).ok())
        .filter(|rank| *rank > 0)
        .ok_or_else(|| podcast_item_error("positive podcast creator chart rank", &raw))?;
    let creator = User {
        resource_ref,
        platform: Platform::Netease,
        id,
        name,
        avatar_url: radio_text_field(&raw, &["avatarUrl"]),
        signature: radio_text_field(&raw, &["signature"]),
        followed: raw.get("followed").and_then(json_bool),
        mutual: raw.get("mutual").and_then(json_bool),
        extensions: Extensions::from([("creator".to_owned(), raw.clone())]),
    };
    Ok(PodcastCreatorChartEntry {
        rank,
        previous_rank: raw.get("lastRank").and_then(json_i64),
        score: raw.get("score").and_then(json_u64),
        follower_count: raw.get("userFollowedCount").and_then(json_u64),
        creator,
        extensions: Extensions::from([("chart_entry".to_owned(), raw)]),
    })
}

fn map_netease_subscribed_podcasts(body: Value, limit: u32, offset: u32) -> Result<Page<Podcast>> {
    ensure_success(&body)?;
    let raw_items = body
        .get("djRadios")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| podcast_item_error("subscribed podcast list", &body))?;
    let mut items = raw_items
        .into_iter()
        .map(map_netease_podcast)
        .collect::<Result<Vec<_>>>()?;
    for podcast in &mut items {
        podcast.subscribed = Some(true);
    }
    let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let candidate_offset = offset.saturating_add(consumed);
    let total = body.get("count").and_then(json_u64);
    let explicit_more = body
        .get("hasMore")
        .and_then(json_bool)
        .or_else(|| body.get("more").and_then(json_bool));
    let has_more = consumed > 0
        && explicit_more.unwrap_or_else(|| {
            total.map_or(consumed >= limit, |total| {
                u64::from(candidate_offset) < total
            })
        });
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total,
            next_offset: has_more.then_some(candidate_offset),
            has_more,
            extensions: Extensions::from([("response".to_owned(), body)]),
        },
    })
}

fn map_podcast_subscription_result(
    id: u64,
    subscribed: bool,
    response: Value,
) -> Result<SubscriptionResult> {
    let resource_ref = ResourceRef::new(Platform::Netease, id.to_string()).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid podcast id: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    Ok(SubscriptionResult {
        resource_ref,
        subscribed,
        extensions: Extensions::from([("response".to_owned(), response)]),
    })
}

fn map_netease_podcast(raw: Value) -> Result<Podcast> {
    let id = ["id", "voiceListId", "radioId"]
        .into_iter()
        .find_map(|field| raw.get(field).and_then(usable_resource_id))
        .ok_or_else(|| podcast_item_error("id", &raw))?;
    let name = radio_text_field(&raw, &["name", "title"])
        .ok_or_else(|| podcast_item_error("name", &raw))?;
    let reference = ResourceRef::new(Platform::Netease, &id).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid podcast id: {error}"),
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "item": raw }))
    })?;
    let mut podcast = Podcast::new(reference, name);
    podcast.description = radio_text_field(&raw, &["desc", "description"]).unwrap_or_default();
    podcast.cover_url = radio_text_field(&raw, &["picUrl", "coverUrl", "coverImgUrl"]);
    podcast.creator = match map_podcast_creator(raw.get("dj"))? {
        Some(creator) => Some(creator),
        None => match map_podcast_creator(raw.get("creator"))? {
            Some(creator) => Some(creator),
            None => radio_text_field(&raw, &["creatorName"]).map(|name| CreatorSummary {
                resource_ref: None,
                name,
                avatar_url: None,
            }),
        },
    };
    podcast.category = radio_text_field(&raw, &["category", "categoryName"]);
    podcast.secondary_category = radio_text_field(&raw, &["secondCategory", "secondCategoryName"]);
    podcast.episode_count =
        podcast_u64_field(&raw, &["programCount", "episodeCount", "voiceCount"]);
    podcast.subscriber_count = podcast_u64_field(&raw, &["subCount", "subscriberCount"]);
    podcast.play_count = podcast_u64_field(&raw, &["playCount", "listenerCount"]);
    podcast.subscribed = radio_bool_field(&raw, &["subed", "subscribed"]);
    podcast.paid = podcast_paid_field(&raw, &["radioFeeType", "feeScope", "voiceFeeType"]);
    podcast.purchased = radio_bool_field(&raw, &["buyed", "purchased"]);
    podcast.price = podcast_price(&raw);
    podcast.created_at = podcast_positive_u64_field(&raw, &["createTime"])
        .and_then(|milliseconds| unix_rfc3339(milliseconds / 1_000));
    podcast.extensions.insert("podcast".to_owned(), raw);
    Ok(podcast)
}

fn map_netease_podcast_episodes(
    body: Value,
    limit: u32,
    offset: u32,
) -> Result<Page<PodcastEpisode>> {
    ensure_success(&body)?;
    let raw_items = body
        .get("programs")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| podcast_item_error("programs array", &body))?;
    let items = raw_items
        .into_iter()
        .map(map_netease_podcast_episode)
        .collect::<Result<Vec<_>>>()?;
    let total = body.get("count").and_then(json_u64);
    let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let next_offset = offset.saturating_add(consumed);
    let upstream_more = body.get("more").and_then(json_bool);
    let has_more = upstream_more
        .unwrap_or_else(|| total.is_some_and(|total| u64::from(next_offset) < total))
        && consumed > 0;
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total,
            next_offset: has_more.then_some(next_offset),
            has_more,
            extensions: Extensions::from([("response".to_owned(), body)]),
        },
    })
}

fn map_netease_podcast_episode_chart(
    body: Value,
    request: &PodcastEpisodeChartRequest,
) -> Result<Page<PodcastEpisodeChartEntry>> {
    ensure_success(&body)?;
    let (raw_items, total, update_time_ms, offset_submitted) = match request.kind {
        PodcastEpisodeChartKind::Popular => (
            body.get("toplist").and_then(Value::as_array).cloned(),
            None,
            body.get("updateTime").and_then(json_u64),
            true,
        ),
        PodcastEpisodeChartKind::Trending24Hours => {
            let data = body
                .get("data")
                .filter(|data| data.is_object())
                .ok_or_else(|| podcast_item_error("24-hour chart data", &body))?;
            (
                data.get("list").and_then(Value::as_array).cloned(),
                data.get("total").and_then(json_u64),
                data.get("updateTime").and_then(json_u64),
                false,
            )
        }
    };
    let raw_items = raw_items.ok_or_else(|| podcast_item_error("episode chart list", &body))?;
    let entries = raw_items
        .into_iter()
        .map(map_netease_podcast_episode_chart_entry)
        .collect::<Result<Vec<_>>>()?;
    let mut extensions = Extensions::from([
        ("kind".to_owned(), json!(request.kind)),
        ("returned_count".to_owned(), json!(entries.len())),
        (
            "limit_applied".to_owned(),
            json!(entries.len() <= request.limit as usize),
        ),
        ("requested_offset".to_owned(), json!(request.offset)),
        ("offset_submitted".to_owned(), json!(offset_submitted)),
        ("offset_applied".to_owned(), json!(false)),
        ("offset_control_supported".to_owned(), json!(false)),
        ("continuation_supported".to_owned(), json!(false)),
    ]);
    insert_extension(&mut extensions, "update_time_ms", update_time_ms);
    if let Some(updated_at) = update_time_ms.and_then(|value| unix_rfc3339(value / 1_000)) {
        extensions.insert("updated_at".to_owned(), json!(updated_at));
    }
    extensions.insert("response".to_owned(), body);
    Ok(Page {
        items: entries,
        pagination: PageMeta {
            limit: request.limit,
            offset: 0,
            total,
            next_offset: None,
            has_more: false,
            extensions,
        },
    })
}

fn map_netease_podcast_episode_chart_entry(raw: Value) -> Result<PodcastEpisodeChartEntry> {
    let program = raw
        .get("program")
        .filter(|program| program.is_object())
        .cloned()
        .ok_or_else(|| podcast_item_error("episode chart program", &raw))?;
    let rank = raw
        .get("rank")
        .and_then(json_u64)
        .and_then(|rank| u32::try_from(rank).ok())
        .filter(|rank| *rank > 0)
        .ok_or_else(|| podcast_item_error("positive episode chart rank", &raw))?;
    let mut episode = map_netease_podcast_episode(program)?;
    episode.paid = raw
        .get("programFeeType")
        .and_then(json_i64)
        .map(|fee_type| fee_type > 0)
        .or(episode.paid);
    Ok(PodcastEpisodeChartEntry {
        rank,
        previous_rank: raw.get("lastRank").and_then(json_i64),
        score: raw.get("score").and_then(json_u64),
        episode,
        extensions: Extensions::from([("chart_entry".to_owned(), raw)]),
    })
}

fn map_netease_podcast_episode_response(body: Value) -> Result<PodcastEpisode> {
    ensure_success(&body)?;
    let raw = body
        .get("program")
        .filter(|program| program.is_object())
        .cloned()
        .ok_or_else(|| podcast_item_error("program", &body))?;
    let mut episode = map_netease_podcast_episode(raw)?;
    episode.extensions.insert("response".to_owned(), body);
    Ok(episode)
}

fn map_netease_podcast_episode_workbench_response(body: Value) -> Result<PodcastEpisode> {
    ensure_success(&body)?;
    let mut first_mapping_error = None;
    for raw in [
        body.pointer("/data/voice"),
        body.get("data"),
        body.get("voice"),
    ]
    .into_iter()
    .flatten()
    .filter(|value| value.as_object().is_some_and(|object| !object.is_empty()))
    {
        match map_netease_podcast_episode(raw.clone()) {
            Ok(mut episode) => {
                episode
                    .extensions
                    .insert("workbench_response".to_owned(), body.clone());
                return Ok(episode);
            }
            Err(error) if first_mapping_error.is_none() => first_mapping_error = Some(error),
            Err(_) => {}
        }
    }
    Err(first_mapping_error.unwrap_or_else(|| podcast_item_error("workbench voice data", &body)))
}

fn map_netease_podcast_episode_stream(
    episode: PodcastEpisode,
    stream: MediaStream,
) -> Result<PodcastEpisodeStream> {
    let episode_ref = episode.resource_ref.clone();
    let audio_ref = episode
        .audio
        .as_ref()
        .map(|audio| audio.resource_ref.clone())
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase podcast episode did not expose a playable audio track",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "episode_ref": episode_ref }))
        })?;
    if audio_ref.platform() != Platform::Netease {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase podcast episode exposed an audio track from another platform",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({
            "episode_ref": episode_ref,
            "audio_ref": audio_ref
        })));
    }
    Ok(PodcastEpisodeStream {
        episode_ref,
        audio_ref,
        stream,
        extensions: Extensions::from([("episode".to_owned(), json!(episode))]),
    })
}

fn map_netease_podcast_episode_lyrics(
    episode: PodcastEpisode,
    response: Value,
    document: Option<Value>,
) -> Result<PodcastEpisodeLyrics> {
    ensure_success(&response)?;
    let episode_ref = episode.resource_ref.clone();
    let audio_ref = episode
        .audio
        .as_ref()
        .map(|audio| audio.resource_ref.clone());
    let track_ref = audio_ref.clone().unwrap_or_else(|| episode_ref.clone());
    let mut extensions = Extensions::from([
        ("episode".to_owned(), json!(episode)),
        ("response".to_owned(), response.clone()),
    ]);
    let Some(document) = document else {
        return Ok(PodcastEpisodeLyrics {
            episode_ref,
            audio_ref,
            lyrics: Lyrics {
                track_ref,
                plain: None,
                translated: None,
                romanized: None,
                word_synced: None,
                format: "plain".to_owned(),
                contributors: Vec::new(),
                extensions: Extensions::from([("available".to_owned(), json!(false))]),
            },
            extensions,
        });
    };

    let sentences = document
        .get("sents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut lines = Vec::with_capacity(sentences.len());
    for sentence in sentences {
        let begin = sentence
            .get("beg")
            .and_then(json_u64)
            .ok_or_else(|| podcast_lyrics_error("sentence start time", &document))?;
        if let Some(end) = sentence.get("end").and_then(json_u64)
            && end < begin
        {
            return Err(podcast_lyrics_error(
                "sentence end time after its start",
                &document,
            ));
        }
        let text = sentence
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| podcast_lyrics_error("sentence text", &document))?
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !text.is_empty() {
            lines.push(format!("{}{}", voice_lrc_timestamp(begin), text));
        }
    }
    let plain = (!lines.is_empty()).then(|| lines.join("\n"));
    let word_synced = serde_json::to_string(&document).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase voice lyric document could not be serialized: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    let mut lyric_extensions = Extensions::from([
        ("available".to_owned(), json!(true)),
        ("sentence_count".to_owned(), json!(sentences.len())),
        ("word_synced_format".to_owned(), json!("netease_voice_json")),
    ]);
    if let Some(duration) = document.get("duration").and_then(json_u64) {
        lyric_extensions.insert("duration_ms".to_owned(), json!(duration));
    }
    if let Some(source) = response.pointer("/data/source")
        && !source.is_null()
    {
        lyric_extensions.insert("source".to_owned(), source.clone());
    }
    extensions.insert("transcript_schema".to_owned(), json!("netease_voice_json"));
    Ok(PodcastEpisodeLyrics {
        episode_ref,
        audio_ref,
        lyrics: Lyrics {
            track_ref,
            plain,
            translated: None,
            romanized: None,
            word_synced: Some(word_synced),
            format: "netease_voice_json".to_owned(),
            contributors: Vec::new(),
            extensions: lyric_extensions,
        },
        extensions,
    })
}

fn voice_lrc_timestamp(milliseconds: u64) -> String {
    let minutes = milliseconds / 60_000;
    let seconds = (milliseconds % 60_000) / 1_000;
    let fraction = milliseconds % 1_000;
    format!("[{minutes:02}:{seconds:02}.{fraction:03}]")
}

fn podcast_lyrics_error(field: &str, raw: &Value) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("NetEase voice lyric response did not contain a valid {field}"),
    )
    .with_platform(Platform::Netease)
    .with_details(json!({ "response": raw }))
}

fn map_netease_podcast_episode(raw: Value) -> Result<PodcastEpisode> {
    let id = ["id", "voiceId", "programId"]
        .into_iter()
        .find_map(|field| raw.get(field).and_then(usable_resource_id))
        .ok_or_else(|| podcast_item_error("program id", &raw))?;
    let name = radio_text_field(&raw, &["name", "title", "songName"])
        .ok_or_else(|| podcast_item_error("program name", &raw))?;
    let reference = ResourceRef::new(Platform::Netease, &id).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid podcast episode id: {error}"),
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "item": raw }))
    })?;
    let podcast_ref = raw
        .get("radio")
        .and_then(|radio| radio.get("id"))
        .and_then(usable_resource_id)
        .or_else(|| {
            ["radioId", "voiceListId"]
                .into_iter()
                .find_map(|field| raw.get(field).and_then(usable_resource_id))
        })
        .map(|id| ResourceRef::new(Platform::Netease, id))
        .transpose()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid episode podcast id: {error}"),
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "item": raw }))
        })?;
    let main_track_id = ["mainTrackId", "songId", "trackId"]
        .into_iter()
        .find_map(|field| raw.get(field).and_then(usable_resource_id));
    let mut audio = raw
        .get("mainSong")
        .filter(|song| song.is_object())
        .cloned()
        .map(parse_body::<Song>)
        .transpose()?
        .map(|song| map_song(song, None))
        .transpose()?;
    if let (Some(main_track_id), Some(audio)) = (main_track_id.as_deref(), audio.as_ref())
        && audio.id != main_track_id
    {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase podcast episode contains conflicting audio track ids",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({
            "episode_id": id,
            "main_track_id": main_track_id,
            "song_id": audio.id
        })));
    }
    if audio.is_none()
        && let Some(main_track_id) = main_track_id
    {
        let audio_ref = ResourceRef::new(Platform::Netease, main_track_id).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid episode audio id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
        let mut fallback = Track::new(audio_ref, name.clone());
        fallback.duration_ms = podcast_positive_u64_field(&raw, &["duration", "durationMs"]);
        audio = Some(fallback);
    }

    let mut episode = PodcastEpisode::new(reference, name);
    episode.podcast_ref = podcast_ref;
    episode.description =
        radio_text_field(&raw, &["description", "programDesc", "desc"]).unwrap_or_default();
    episode.cover_url = radio_text_field(&raw, &["coverUrl", "blurCoverUrl", "coverImgUrl"]);
    episode.creator = match map_podcast_creator(raw.get("dj"))? {
        Some(creator) => Some(creator),
        None => map_podcast_creator(raw.get("creator"))?,
    };
    episode.duration_ms = podcast_positive_u64_field(&raw, &["duration", "durationMs"])
        .or_else(|| audio.as_ref().and_then(|audio| audio.duration_ms));
    episode.audio = audio;
    episode.published_at =
        podcast_positive_u64_field(&raw, &["createTime", "scheduledPublishTime", "publishTime"])
            .and_then(|milliseconds| unix_rfc3339(milliseconds / 1_000));
    episode.serial_number = podcast_u64_field(&raw, &["serialNum", "seqNo", "orderNo"]);
    episode.listener_count = podcast_u64_field(&raw, &["listenerCount", "playCount"]);
    episode.liked_count = podcast_u64_field(&raw, &["likedCount"]);
    episode.comment_count = podcast_u64_field(&raw, &["commentCount"]);
    episode.share_count = podcast_u64_field(&raw, &["shareCount"]);
    episode.subscribed = radio_bool_field(&raw, &["subscribed"]);
    episode.has_lyrics = radio_bool_field(&raw, &["existLyric"]);
    episode.paid = podcast_paid_field(&raw, &["programFeeType", "feeScope", "voiceFeeType"]);
    episode.purchased = radio_bool_field(&raw, &["buyed", "buyedNew", "purchased"]);
    episode.extensions.insert("program".to_owned(), raw);
    Ok(episode)
}

fn map_podcast_creator(raw: Option<&Value>) -> Result<Option<CreatorSummary>> {
    let Some(raw) = raw.filter(|raw| raw.is_object()) else {
        return Ok(None);
    };
    let Some(name) = radio_text_field(raw, &["nickname", "name"]) else {
        return Ok(None);
    };
    let resource_ref = raw
        .get("userId")
        .and_then(usable_resource_id)
        .or_else(|| raw.get("id").and_then(usable_resource_id))
        .map(|id| ResourceRef::new(Platform::Netease, id))
        .transpose()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid podcast creator id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    Ok(Some(CreatorSummary {
        resource_ref,
        name,
        avatar_url: radio_text_field(raw, &["avatarUrl"]),
    }))
}

fn podcast_u64_field(raw: &Value, fields: &[&str]) -> Option<u64> {
    fields
        .iter()
        .find_map(|field| raw.get(field).and_then(json_u64))
}

fn podcast_positive_u64_field(raw: &Value, fields: &[&str]) -> Option<u64> {
    fields.iter().find_map(|field| {
        raw.get(*field)
            .and_then(json_u64)
            .filter(|value| *value > 0)
    })
}

fn podcast_paid_field(raw: &Value, fields: &[&str]) -> Option<bool> {
    fields
        .iter()
        .find_map(|field| raw.get(field).and_then(json_i64))
        .map(|fee_type| fee_type > 0)
}

fn podcast_price(raw: &Value) -> Option<Money> {
    podcast_u64_field(raw, &["discountPrice", "originalPrice"]).map(|cents| Money {
        amount: cents as f64 / 100.0,
        currency: "CNY".to_owned(),
    })
}

fn podcast_item_error(field: &str, raw: &Value) -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::UpstreamError,
        format!("NetEase podcast response did not contain {field}"),
    )
    .with_platform(Platform::Netease)
    .with_details(json!({ "response": raw }))
}

fn json_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn json_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn json_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::Number(value) => value.as_i64().map(|value| value != 0),
        Value::String(value) if value.eq_ignore_ascii_case("true") || value == "1" => Some(true),
        Value::String(value) if value.eq_ignore_ascii_case("false") || value == "0" => Some(false),
        _ => None,
    }
}

fn validate_image_upload(request: &ImageUploadRequest) -> Result<(&str, &str)> {
    let filename = request.filename.trim();
    if filename.is_empty() {
        return Err(
            TuneWeaveError::invalid_request("image filename cannot be empty")
                .with_platform(Platform::Netease),
        );
    }
    if filename.len() > 255 || filename.chars().any(char::is_control) {
        return Err(TuneWeaveError::invalid_request(
            "image filename must be at most 255 bytes and contain no control characters",
        )
        .with_platform(Platform::Netease));
    }
    let content_type = request.content_type.trim();
    if !content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim().to_ascii_lowercase().starts_with("image/"))
    {
        return Err(TuneWeaveError::invalid_request(
            "image content type must use the image media type",
        )
        .with_platform(Platform::Netease));
    }
    if request.data.is_empty() {
        return Err(
            TuneWeaveError::invalid_request("image data cannot be empty")
                .with_platform(Platform::Netease),
        );
    }
    if request.image_size == Some(0) {
        return Err(
            TuneWeaveError::invalid_request("image_size must be greater than zero")
                .with_platform(Platform::Netease),
        );
    }
    Ok((filename, content_type))
}

#[derive(Debug)]
struct CloudUploadDescriptor {
    md5: String,
    filename: String,
    allocation_filename: String,
    extension: String,
    content_type: String,
}

#[derive(Debug)]
struct CloudUploadCompleteDescriptor {
    provisional_track_id: String,
    resource_id: String,
    md5: String,
    filename: String,
    song_name: String,
    artist: String,
    album: String,
}

#[derive(Debug)]
struct CloudImportDescriptor {
    md5: String,
    source_track_id: String,
    bitrate_kbps: u64,
    file_type: String,
    song_name: String,
    artist: String,
    album: String,
}

#[derive(Clone, Debug, Default, Serialize)]
struct CloudAudioMetadata {
    song_name: Option<String>,
    artist: Option<String>,
    album: Option<String>,
}

fn read_cloud_audio_metadata(data: &[u8]) -> CloudAudioMetadata {
    let Ok(probe) = Probe::new(Cursor::new(data)).guess_file_type() else {
        return CloudAudioMetadata::default();
    };
    let Ok(tagged_file) = probe.read() else {
        return CloudAudioMetadata::default();
    };
    let primary_tag = tagged_file.primary_tag();
    let fallback_tag = tagged_file.first_tag();
    cloud_audio_metadata_from_tags(primary_tag, fallback_tag)
}

fn cloud_audio_metadata_from_tags(
    primary_tag: Option<&lofty::tag::Tag>,
    fallback_tag: Option<&lofty::tag::Tag>,
) -> CloudAudioMetadata {
    CloudAudioMetadata {
        song_name: primary_tag
            .and_then(|tag| clean_cloud_tag_value(tag.title().as_deref()))
            .or_else(|| fallback_tag.and_then(|tag| clean_cloud_tag_value(tag.title().as_deref()))),
        artist: primary_tag
            .and_then(|tag| clean_cloud_tag_value(tag.artist().as_deref()))
            .or_else(|| {
                fallback_tag.and_then(|tag| clean_cloud_tag_value(tag.artist().as_deref()))
            }),
        album: primary_tag
            .and_then(|tag| clean_cloud_tag_value(tag.album().as_deref()))
            .or_else(|| fallback_tag.and_then(|tag| clean_cloud_tag_value(tag.album().as_deref()))),
    }
}

fn cloud_audio_md5(data: &[u8]) -> String {
    hex::encode(Md5::digest(data))
}

fn clean_cloud_tag_value(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| {
            !value.is_empty() && value.len() <= 1_024 && !value.chars().any(char::is_control)
        })
        .map(str::to_owned)
}

fn resolve_cloud_audio_metadata(
    request: &CloudUploadRequest,
    descriptor: &CloudUploadDescriptor,
    tagged: &CloudAudioMetadata,
) -> Result<(String, String, String)> {
    let song_name = validate_optional_cloud_metadata("song_name", request.song_name.as_deref())?
        .or_else(|| tagged.song_name.clone())
        .unwrap_or_else(|| descriptor.allocation_filename.clone());
    let artist = validate_optional_cloud_metadata("artist", request.artist.as_deref())?
        .or_else(|| tagged.artist.clone())
        .unwrap_or_else(|| "未知艺术家".to_owned());
    let album = validate_optional_cloud_metadata("album", request.album.as_deref())?
        .or_else(|| tagged.album.clone())
        .unwrap_or_else(|| "未知专辑".to_owned());
    Ok((song_name, artist, album))
}

fn validate_cloud_upload_ticket_request(
    request: &CloudUploadTicketRequest,
) -> Result<CloudUploadDescriptor> {
    if request.file_size == 0 {
        return Err(TuneWeaveError::invalid_request(
            "cloud audio file_size must be greater than zero",
        )
        .with_platform(Platform::Netease));
    }
    if request.bitrate == 0 {
        return Err(TuneWeaveError::invalid_request(
            "cloud audio bitrate must be greater than zero",
        )
        .with_platform(Platform::Netease));
    }
    cloud_upload_descriptor(
        &request.md5,
        &request.filename,
        request.content_type.as_deref(),
    )
}

fn validate_cloud_upload_complete_request(
    request: &CloudUploadCompleteRequest,
) -> Result<CloudUploadCompleteDescriptor> {
    if request.bitrate == 0 {
        return Err(TuneWeaveError::invalid_request(
            "cloud audio bitrate must be greater than zero",
        )
        .with_platform(Platform::Netease));
    }
    let provisional_track_id =
        required_cloud_value("provisional_track_id", &request.provisional_track_id)?;
    let resource_id = required_cloud_value("resource_id", &request.resource_id)?;
    let descriptor = cloud_upload_descriptor(&request.md5, &request.filename, None)?;
    let fallback_name = cloud_filename_stem(&descriptor.filename);
    Ok(CloudUploadCompleteDescriptor {
        provisional_track_id,
        resource_id,
        md5: descriptor.md5,
        filename: descriptor.filename,
        song_name: validate_optional_cloud_metadata("song_name", request.song_name.as_deref())?
            .unwrap_or(fallback_name),
        artist: validate_optional_cloud_metadata("artist", request.artist.as_deref())?
            .unwrap_or_else(|| "未知艺术家".to_owned()),
        album: validate_optional_cloud_metadata("album", request.album.as_deref())?
            .unwrap_or_else(|| "未知专辑".to_owned()),
    })
}

fn validate_cloud_import_request(request: &CloudImportRequest) -> Result<CloudImportDescriptor> {
    if request.file_size == 0 {
        return Err(TuneWeaveError::invalid_request(
            "cloud import file_size must be greater than zero",
        )
        .with_platform(Platform::Netease));
    }
    let bitrate_kbps = request.bitrate / 1_000;
    if bitrate_kbps == 0 {
        return Err(TuneWeaveError::invalid_request(
            "cloud import bitrate must be at least 1000 bit/s",
        )
        .with_platform(Platform::Netease));
    }
    let source_track_id = request
        .source_track_id
        .as_deref()
        .map(|value| required_cloud_value("source_track_id", value))
        .transpose()?
        .unwrap_or_else(|| "-2".to_owned());
    let source_id = source_track_id.parse::<i64>().map_err(|_| {
        TuneWeaveError::invalid_request(
            "cloud import source_track_id must be a positive NetEase id or -2",
        )
        .with_platform(Platform::Netease)
    })?;
    if source_id != -2 && source_id <= 0 {
        return Err(TuneWeaveError::invalid_request(
            "cloud import source_track_id must be a positive NetEase id or -2",
        )
        .with_platform(Platform::Netease));
    }
    let file_type = request
        .file_type
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if file_type.is_empty()
        || file_type.len() > 10
        || !file_type.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(TuneWeaveError::invalid_request(
            "cloud import file_type must be a 1 to 10 character alphanumeric extension",
        )
        .with_platform(Platform::Netease));
    }
    let song_name = required_cloud_value("song_name", &request.song_name)?;
    if song_name.len() > 255
        || song_name
            .chars()
            .any(|character| matches!(character, '/' | '\\'))
    {
        return Err(TuneWeaveError::invalid_request(
            "cloud import song_name must be a safe filename stem of at most 255 bytes",
        )
        .with_platform(Platform::Netease));
    }
    Ok(CloudImportDescriptor {
        md5: normalize_cloud_md5(&request.md5)?,
        source_track_id,
        bitrate_kbps,
        file_type,
        song_name,
        artist: cloud_import_metadata("artist", &request.artist, "未知")?,
        album: cloud_import_metadata("album", &request.album, "未知")?,
    })
}

fn cloud_import_metadata(name: &str, value: &str, fallback: &str) -> Result<String> {
    let value = value.trim();
    let value = if value.is_empty() { fallback } else { value };
    if value.len() > 1_024 || value.chars().any(char::is_control) {
        return Err(TuneWeaveError::invalid_request(format!(
            "cloud import {name} must not exceed 1024 bytes or contain control characters"
        ))
        .with_platform(Platform::Netease));
    }
    Ok(value.to_owned())
}

fn cloud_import_check_payload(descriptor: &CloudImportDescriptor, file_size: u64) -> Value {
    json!({
        "uploadType": 0,
        "songs": json!([{
            "md5": descriptor.md5,
            "songId": descriptor.source_track_id,
            "bitrate": descriptor.bitrate_kbps,
            "fileSize": file_size
        }]).to_string()
    })
}

fn cloud_import_payload(descriptor: &CloudImportDescriptor, checked_track_id: &str) -> Value {
    json!({
        "uploadType": 0,
        "songs": json!([{
            "songId": checked_track_id,
            "bitrate": descriptor.bitrate_kbps,
            "song": descriptor.song_name,
            "artist": descriptor.artist,
            "album": descriptor.album,
            "fileName": format!("{}.{}", descriptor.song_name, descriptor.file_type)
        }]).to_string()
    })
}

fn cloud_lyrics_payload(user_id: &str, track_id: &str) -> Value {
    json!({
        "userId": user_id,
        "songId": track_id,
        "lv": -1,
        "kv": -1
    })
}

fn cloud_match_payload(user_id: &str, cloud_track_id: &str, target_track_id: &str) -> Value {
    json!({
        "userId": user_id,
        "songId": cloud_track_id,
        "adjustSongId": target_track_id
    })
}

fn netease_cloud_tracks_request(limit: u32, offset: u32) -> (&'static str, Value) {
    (
        "/api/v1/cloud/get",
        json!({
            "limit": limit,
            "offset": offset
        }),
    )
}

fn netease_cloud_track_details_request(ids: &[String]) -> (&'static str, Value) {
    ("/api/v1/cloud/get/byids", json!({ "songIds": ids }))
}

fn netease_cloud_track_delete_request(ids: &[String]) -> (&'static str, Value) {
    ("/api/cloud/del", json!({ "songIds": [ids.join(",")] }))
}

fn netease_cloud_track_download_request(id: &str) -> (&'static str, Value) {
    ("/api/cloud/dowonload", json!({ "songId": id }))
}

fn validate_cloud_track_refs(track_refs: &[ResourceRef]) -> Result<Vec<String>> {
    if track_refs.is_empty() {
        return Err(TuneWeaveError::invalid_request(
            "cloud track_refs must contain at least one reference",
        )
        .with_platform(Platform::Netease));
    }
    track_refs
        .iter()
        .map(|track_ref| {
            if track_ref.platform() != Platform::Netease {
                return Err(TuneWeaveError::invalid_request(
                    "NetEase cloud operations only accept NetEase track references",
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "track_ref": track_ref })));
            }
            required_cloud_value("track_id", track_ref.id())
        })
        .collect()
}

fn map_netease_cloud_tracks(limit: u32, offset: u32, response: Value) -> Result<Page<CloudTrack>> {
    ensure_success(&response)?;
    let raw_response = response.clone();
    let CloudTracksEnvelope {
        data,
        count,
        has_more,
        size,
        max_size,
        upgrade_sign,
    } = parse_body(response)?;
    let data = data.ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase cloud library response is missing its data array",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "response": raw_response }))
    })?;
    let items = data
        .into_iter()
        .map(map_netease_cloud_track)
        .collect::<Result<Vec<_>>>()?;
    let total = count.as_ref().and_then(json_u64);
    let consumed = items.len() as u32;
    let next_offset = offset.saturating_add(consumed);
    let has_more = has_more
        .as_ref()
        .and_then(json_bool)
        .unwrap_or_else(|| total.is_some_and(|total| u64::from(next_offset) < total));
    let mut extensions = Extensions::new();
    insert_extension(&mut extensions, "storage_size", size);
    insert_extension(&mut extensions, "storage_max_size", max_size);
    insert_extension(&mut extensions, "upgrade_sign", upgrade_sign);
    extensions.insert("response".to_owned(), raw_response);
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total,
            next_offset: (has_more && consumed > 0).then_some(next_offset),
            has_more,
            extensions,
        },
    })
}

fn map_netease_cloud_track_details(response: Value) -> Result<Vec<CloudTrack>> {
    ensure_success(&response)?;
    let raw_response = response.clone();
    let envelope: CloudTracksEnvelope = parse_body(response)?;
    envelope
        .data
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase cloud track details response is missing its data array",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "response": raw_response }))
        })?
        .into_iter()
        .map(map_netease_cloud_track)
        .collect()
}

fn map_netease_cloud_track(raw: Value) -> Result<CloudTrack> {
    let song_value = [raw.get("simpleSong"), raw.get("song")]
        .into_iter()
        .flatten()
        .find(|song| song.is_object())
        .cloned()
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase cloud track is missing its song metadata",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "item": raw }))
        })?;
    let song: Song = parse_body(song_value.clone())?;
    let original_track_id = song.id.to_string();
    let mut track = map_song(song, None)?;
    let cloud_track_id = raw
        .get("songId")
        .and_then(usable_resource_id)
        .unwrap_or_else(|| original_track_id.clone());
    let cloud_track_ref =
        ResourceRef::new(Platform::Netease, cloud_track_id.clone()).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid cloud track id: {error}"),
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "item": raw }))
        })?;
    let matched_track_id = [
        raw.get("matchedId"),
        raw.get("matchId"),
        raw.get("asId"),
        song_value.get("s_id"),
        song_value.get("matchedId"),
        song_value.get("matchId"),
    ]
    .into_iter()
    .flatten()
    .find_map(|value| usable_resource_id(value).filter(|id| id != &cloud_track_id))
    .or_else(|| (original_track_id != cloud_track_id).then_some(original_track_id));
    let matched_track_ref = matched_track_id
        .map(|id| ResourceRef::new(Platform::Netease, id))
        .transpose()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid matched cloud track id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;

    if track.name.trim().is_empty()
        && let Some(name) = cloud_text_field(&raw, &["songName", "name"])
    {
        track.name = name;
    }
    if track.artists.is_empty()
        && let Some(artist) = cloud_text_field(&raw, &["artist", "artistName"])
    {
        track.artists.push(ArtistSummary {
            resource_ref: None,
            name: artist,
        });
    }
    let cover_url = cloud_text_field(&raw, &["cover", "coverUrl", "picUrl"]);
    if let Some(album) = track.album.as_mut() {
        if album.name.trim().is_empty()
            && let Some(album_name) = cloud_text_field(&raw, &["album", "albumName"])
        {
            album.name = album_name;
        }
        if album.cover_url.is_none() {
            album.cover_url.clone_from(&cover_url);
        }
    } else if let Some(album_name) = cloud_text_field(&raw, &["album", "albumName"]) {
        track.album = Some(AlbumSummary {
            resource_ref: None,
            name: album_name,
            cover_url: cover_url.clone(),
        });
    }
    track.resource_ref = cloud_track_ref.clone();
    track.platform = Platform::Netease;
    track.id.clone_from(&cloud_track_id);

    let filename = cloud_text_field(&raw, &["fileName", "filename"]);
    let file_type = cloud_text_field(&raw, &["fileType", "type"])
        .and_then(normalize_cloud_file_type)
        .or_else(|| filename.as_deref().and_then(cloud_file_type_from_name));
    let added_at = raw
        .get("addTime")
        .and_then(json_u64)
        .filter(|timestamp| *timestamp > 0)
        .and_then(|timestamp| unix_rfc3339(timestamp / 1_000));
    let mut extensions = Extensions::new();
    extensions.insert("cloud_item".to_owned(), raw.clone());
    Ok(CloudTrack {
        cloud_track_ref,
        track,
        filename,
        file_size: cloud_u64_field(&raw, &["fileSize", "size"]),
        file_type,
        bitrate: cloud_u64_field(&raw, &["bitrate", "br"]),
        md5: cloud_text_field(&raw, &["md5"]),
        added_at,
        matched_track_ref,
        extensions,
    })
}

fn map_netease_cloud_track_download(
    id: &str,
    path: &str,
    response: Value,
) -> Result<MediaDownload> {
    ensure_success(&response)?;
    let raw_response = response.clone();
    let item = if let Some(data) = response.get("data") {
        if let Some(items) = data.as_array() {
            items
                .iter()
                .find(|item| cloud_download_item_id(item).as_deref() == Some(id))
                .or_else(|| (items.len() == 1).then(|| items.first()).flatten())
                .cloned()
                .ok_or_else(|| {
                    TuneWeaveError::new(
                        ErrorCode::UpstreamError,
                        "NetEase omitted the requested cloud download result",
                    )
                    .with_platform(Platform::Netease)
                    .with_details(json!({ "id": id, "response": raw_response }))
                })?
        } else if data.is_object() {
            data.clone()
        } else {
            return Err(TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase cloud download data has an unsupported shape",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "id": id, "response": raw_response })));
        }
    } else if response.get("url").is_some() || response.get("downloadUrl").is_some() {
        response.clone()
    } else {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase cloud download response is missing data",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "response": response })));
    };
    if let Some(returned_id) = cloud_download_item_id(&item)
        && returned_id != id
    {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase returned a cloud download result for the wrong track",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "expected": id, "actual": returned_id })));
    }
    let track_ref = ResourceRef::new(Platform::Netease, id.to_owned()).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid cloud download track id: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    let url = cloud_text_field(&item, &["downloadUrl", "url"]);
    let bitrate = cloud_u64_field(&item, &["br", "bitrate"]);
    let format = cloud_text_field(&item, &["fileType", "type"])
        .and_then(normalize_cloud_file_type)
        .or_else(|| {
            cloud_text_field(&item, &["fileName", "filename", "name"])
                .as_deref()
                .and_then(cloud_file_type_from_name)
        });
    let codec = cloud_text_field(&item, &["encodeType", "codec"]).or_else(|| format.clone());
    let expires_at = cloud_u64_field(&item, &["expi"])
        .filter(|expires_in_seconds| *expires_in_seconds > 0)
        .and_then(expiration_rfc3339);
    let platform_code =
        cloud_i64_field(&item, &["code"]).or_else(|| raw_response.get("code").and_then(json_i64));
    let message = cloud_text_field(&item, &["message", "msg"])
        .or_else(|| cloud_text_field(&raw_response, &["message", "msg"]));
    let mut extensions = Extensions::new();
    extensions.insert("request_path".to_owned(), json!(path));
    extensions.insert("response_item".to_owned(), item.clone());
    extensions.insert("response".to_owned(), raw_response);
    Ok(MediaDownload {
        track_ref,
        platform: Platform::Netease,
        available: url.is_some(),
        url,
        headers: BTreeMap::new(),
        expires_at,
        format,
        codec,
        bitrate,
        size: cloud_u64_field(&item, &["size", "fileSize"]),
        duration_ms: cloud_u64_field(&item, &["time", "duration"]),
        requested_quality: Quality::Auto,
        actual_quality: stream_quality(cloud_text_field(&item, &["level"]).as_deref(), bitrate),
        platform_code,
        fee: cloud_i64_field(&item, &["fee"]),
        message,
        extensions,
    })
}

fn cloud_download_item_id(item: &Value) -> Option<String> {
    ["songId", "id"]
        .into_iter()
        .find_map(|field| item.get(field).and_then(json_scalar_string))
}

fn cloud_text_field(raw: &Value, fields: &[&str]) -> Option<String> {
    fields.iter().find_map(|field| {
        raw.get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    })
}

fn cloud_u64_field(raw: &Value, fields: &[&str]) -> Option<u64> {
    fields
        .iter()
        .find_map(|field| raw.get(field).and_then(json_u64))
}

fn cloud_i64_field(raw: &Value, fields: &[&str]) -> Option<i64> {
    fields
        .iter()
        .find_map(|field| raw.get(field).and_then(json_i64))
}

fn normalize_cloud_file_type(file_type: String) -> Option<String> {
    let file_type = file_type
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    (!file_type.is_empty()).then_some(file_type)
}

fn cloud_file_type_from_name(filename: &str) -> Option<String> {
    filename
        .rsplit_once('.')
        .map(|(_, extension)| extension.to_owned())
        .and_then(normalize_cloud_file_type)
}

fn cloud_upload_descriptor(
    md5: &str,
    filename: &str,
    content_type: Option<&str>,
) -> Result<CloudUploadDescriptor> {
    let md5 = normalize_cloud_md5(md5)?;
    let filename = filename.trim();
    if filename.is_empty()
        || filename.len() > 255
        || filename
            .chars()
            .any(|character| character.is_control() || matches!(character, '/' | '\\'))
    {
        return Err(TuneWeaveError::invalid_request(
            "cloud audio filename must be a safe basename of at most 255 bytes",
        )
        .with_platform(Platform::Netease));
    }
    let extension = filename
        .rsplit_once('.')
        .map_or("mp3", |(_, extension)| extension)
        .trim()
        .to_ascii_lowercase();
    let extension = if extension.is_empty()
        || extension.len() > 10
        || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        "mp3".to_owned()
    } else {
        extension
    };
    let allocation_filename = cloud_filename_stem(filename)
        .chars()
        .filter(|character| !character.is_whitespace())
        .map(|character| if character == '.' { '_' } else { character })
        .collect::<String>();
    let allocation_filename = if allocation_filename.is_empty() {
        "unknown".to_owned()
    } else {
        allocation_filename
    };
    let content_type = content_type
        .map(str::trim)
        .filter(|content_type| !content_type.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| cloud_audio_content_type(&extension).to_owned());
    if content_type.chars().any(char::is_control) {
        return Err(
            TuneWeaveError::invalid_request("cloud audio content type is invalid")
                .with_platform(Platform::Netease),
        );
    }
    Ok(CloudUploadDescriptor {
        md5,
        filename: filename.to_owned(),
        allocation_filename,
        extension,
        content_type,
    })
}

fn normalize_cloud_md5(md5: &str) -> Result<String> {
    let md5 = md5.trim().to_ascii_lowercase();
    if md5.len() != 32 || !md5.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(TuneWeaveError::invalid_request(
            "cloud audio md5 must contain exactly 32 hexadecimal characters",
        )
        .with_platform(Platform::Netease));
    }
    Ok(md5)
}

fn required_cloud_value(name: &str, value: &str) -> Result<String> {
    let value = value.trim();
    if value.is_empty() || value.len() > 1_024 || value.chars().any(char::is_control) {
        return Err(TuneWeaveError::invalid_request(format!(
            "cloud {name} must be 1 to 1024 bytes and contain no control characters"
        ))
        .with_platform(Platform::Netease));
    }
    Ok(value.to_owned())
}

fn validate_optional_cloud_metadata(name: &str, value: Option<&str>) -> Result<Option<String>> {
    let value = value.map(str::trim).filter(|value| !value.is_empty());
    let Some(value) = value else {
        return Ok(None);
    };
    if value.len() > 1_024 || value.chars().any(char::is_control) {
        return Err(TuneWeaveError::invalid_request(format!(
            "cloud audio {name} must not exceed 1024 bytes or contain control characters"
        ))
        .with_platform(Platform::Netease));
    }
    Ok(Some(value.to_owned()))
}

fn cloud_filename_stem(filename: &str) -> String {
    filename
        .rsplit_once('.')
        .map_or(filename, |(stem, _)| stem)
        .to_owned()
}

fn cloud_audio_content_type(extension: &str) -> &'static str {
    match extension {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "m4a" | "mp4" => "audio/mp4",
        "ogg" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "aac" => "audio/aac",
        _ => "application/octet-stream",
    }
}

fn validate_cloud_upload_allocation(allocation: &CloudUploadAllocationEnvelope) -> Result<()> {
    if allocation.result.object_key.trim().is_empty()
        || allocation.result.token.trim().is_empty()
        || json_scalar_string(&allocation.result.resource_id).is_none()
    {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase cloud upload allocation is incomplete",
        )
        .with_platform(Platform::Netease));
    }
    Ok(())
}

fn build_cloud_upload_url(server: &str, bucket: &str, object_key: &str) -> Result<String> {
    let url = Url::parse(server).map_err(|_| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase returned an invalid cloud upload server",
        )
        .with_platform(Platform::Netease)
    })?;
    let host = url.host_str().unwrap_or_default();
    if !matches!(url.scheme(), "http" | "https")
        || !host.ends_with(".127.net")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !matches!(url.path(), "" | "/")
        || object_key.trim().is_empty()
    {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase cloud upload server is outside the allowed NOS destination",
        )
        .with_platform(Platform::Netease));
    }
    let object_key = utf8_percent_encode(object_key, NON_ALPHANUMERIC);
    Ok(format!(
        "{}/{bucket}/{object_key}?offset=0&complete=true&version=1.0",
        url.origin().ascii_serialization()
    ))
}

fn require_authenticated_client(client: &NeteaseClient, operation: &str) -> Result<()> {
    if client.is_authenticated() {
        return Ok(());
    }
    Err(TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        format!("NetEase {operation} requires a logged-in session"),
    )
    .with_platform(Platform::Netease))
}

fn map_cloud_upload_result(
    track_id: String,
    upload_required: Option<bool>,
    uploaded: Option<bool>,
    info_response: Value,
    publish_response: Value,
) -> Result<CloudUploadResult> {
    let track_ref = ResourceRef::new(Platform::Netease, track_id).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid cloud track id: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    let mut extensions = Extensions::new();
    extensions.insert("info_response".to_owned(), info_response);
    extensions.insert("publish_response".to_owned(), publish_response);
    Ok(CloudUploadResult {
        track_ref: Some(track_ref),
        upload_required,
        uploaded,
        published: true,
        extensions,
    })
}

fn map_cloud_import_result(
    checked_track_id: &str,
    upload_status: Option<i64>,
    check_response: Value,
    import_response: Value,
) -> Result<CloudImportResult> {
    let track_id = [
        import_response.get("songId"),
        import_response.pointer("/data/songId"),
        import_response.pointer("/data/0/songId"),
        import_response.pointer("/result/songId"),
    ]
    .into_iter()
    .flatten()
    .find_map(usable_resource_id)
    .unwrap_or_else(|| checked_track_id.to_owned());
    let track_ref = ResourceRef::new(Platform::Netease, track_id).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid imported cloud track id: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    let already_present = match upload_status {
        Some(1) => Some(true),
        Some(0 | 2) => Some(false),
        _ => None,
    };
    let mut extensions = Extensions::new();
    if let Some(upload_status) = upload_status {
        extensions.insert("upload_status".to_owned(), json!(upload_status));
    }
    extensions.insert("check_response".to_owned(), check_response);
    extensions.insert("import_response".to_owned(), import_response);
    Ok(CloudImportResult {
        track_ref: Some(track_ref),
        imported: true,
        already_present,
        extensions,
    })
}

fn map_cloud_match_result(
    cloud_track_id: &str,
    target_track_id: &str,
    user_id: &str,
    response: Value,
) -> Result<CloudMatchResult> {
    let cloud_track_ref =
        ResourceRef::new(Platform::Netease, cloud_track_id.to_owned()).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid cloud track id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let target_track_ref = (target_track_id != "0")
        .then(|| ResourceRef::new(Platform::Netease, target_track_id.to_owned()))
        .transpose()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid cloud match target id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let matched = target_track_ref.is_some();
    let mut extensions = Extensions::new();
    extensions.insert("cloud_user_id".to_owned(), json!(user_id));
    extensions.insert("response".to_owned(), response);
    Ok(CloudMatchResult {
        cloud_track_ref,
        target_track_ref,
        matched,
        extensions,
    })
}

fn validate_image_allocation(response: &ImageUploadAllocationEnvelope) -> Result<()> {
    if response.result.object_key.trim().is_empty()
        || response.result.token.trim().is_empty()
        || json_scalar_string(&response.result.document_id)
            .as_deref()
            .is_none_or(str::is_empty)
    {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase image upload allocation is incomplete",
        )
        .with_platform(Platform::Netease));
    }
    Ok(())
}

fn map_image_upload_result(
    request: &ImageUploadRequest,
    allocation: ImageUploadAllocationEnvelope,
    upload_response: Value,
    update_response: Value,
) -> Result<ImageUploadResult> {
    let image_id = json_scalar_string(&allocation.result.document_id).ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase image upload allocation did not contain an image id",
        )
        .with_platform(Platform::Netease)
    })?;
    let url_pre = format!("https://p1.music.126.net/{}", allocation.result.object_key);
    let url = ["/data/url", "/url", "/data/avatarUrl", "/avatarUrl"]
        .into_iter()
        .find_map(|pointer| {
            update_response
                .pointer(pointer)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|url| !url.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| Some(url_pre.clone()));
    let mut extensions = Extensions::new();
    extensions.insert("url_pre".to_owned(), json!(url_pre));
    extensions.insert(
        "allocation".to_owned(),
        json!({
            "object_key": allocation.result.object_key,
            "document_id": allocation.result.document_id
        }),
    );
    extensions.insert("upload_response".to_owned(), upload_response);
    extensions.insert("response".to_owned(), update_response);
    if request.image_size.is_some() || request.crop_x.is_some() || request.crop_y.is_some() {
        extensions.insert(
            "reference_crop_parameters".to_owned(),
            json!({
                "image_size": request.image_size,
                "crop_x": request.crop_x,
                "crop_y": request.crop_y,
                "applied": false
            }),
        );
    }
    Ok(ImageUploadResult {
        url,
        image_id: Some(image_id),
        extensions,
    })
}

fn ensure_account_access(client: &NeteaseClient, body: &Value, operation: &str) -> Result<()> {
    match ensure_success(body) {
        Err(error) if error.code == ErrorCode::PermissionDenied && !client.is_authenticated() => {
            Err(TuneWeaveError::new(
                ErrorCode::AuthenticationRequired,
                format!("NetEase {operation} requires a logged-in session"),
            )
            .with_platform(Platform::Netease)
            .with_details(error.details))
        }
        result => result,
    }
}

fn select_page<T>(items: Vec<T>, limit: u32, offset: u32) -> (Vec<T>, PageMeta) {
    let total = items.len() as u64;
    let selected = items
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect::<Vec<_>>();
    let consumed = selected.len() as u32;
    let next_offset = offset.saturating_add(consumed);
    let has_more = u64::from(next_offset) < total;
    (
        selected,
        PageMeta {
            limit,
            offset,
            total: Some(total),
            next_offset: has_more.then_some(next_offset),
            has_more,
            extensions: Default::default(),
        },
    )
}

fn map_account_profile(account: &str, summary: NeteaseAccountSummary) -> AccountProfile {
    let mut profile = AccountProfile::authenticated(Platform::Netease, account);
    profile.user_id =
        usable_owned_resource_id(summary.user_id).or_else(|| usable_owned_resource_id(summary.id));
    profile.nickname = summary.nickname;
    profile.avatar_url = summary.avatar_url;
    profile
}

fn map_netease_country_calling_codes(response: Value) -> Result<Vec<CountryCallingCodeGroup>> {
    let raw_groups = response
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase country calling codes response is missing its data array",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "response": response }))
        })?;
    let mut catalog_response = response.clone();
    if let Some(catalog) = catalog_response.as_object_mut() {
        catalog.remove("data");
    }
    raw_groups
        .into_iter()
        .map(|raw_group| {
            let label = required_country_calling_code_field(&raw_group, "label", "group")?;
            let raw_entries = raw_group
                .get("countryList")
                .and_then(Value::as_array)
                .cloned()
                .ok_or_else(|| {
                    TuneWeaveError::new(
                        ErrorCode::UpstreamError,
                        "NetEase country calling code group is missing countryList",
                    )
                    .with_platform(Platform::Netease)
                    .with_details(json!({ "group": raw_group }))
                })?;
            let entries = raw_entries
                .into_iter()
                .map(|raw_entry| {
                    let calling_code =
                        required_country_calling_code_field(&raw_entry, "code", "entry")?;
                    let region_code =
                        required_country_calling_code_field(&raw_entry, "locale", "entry")?;
                    let name = required_country_calling_code_field(&raw_entry, "zh", "entry")?;
                    let english_name =
                        required_country_calling_code_field(&raw_entry, "en", "entry")?;
                    Ok(CountryCallingCode {
                        calling_code,
                        region_code,
                        name,
                        english_name,
                        extensions: Extensions::from([("response".to_owned(), raw_entry)]),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(CountryCallingCodeGroup {
                label,
                entries,
                extensions: Extensions::from([
                    ("response".to_owned(), raw_group),
                    ("catalog_response".to_owned(), catalog_response.clone()),
                ]),
            })
        })
        .collect()
}

fn required_country_calling_code_field(raw: &Value, field: &str, scope: &str) -> Result<String> {
    raw.get(field)
        .and_then(json_scalar_string)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase country calling code {scope} is missing {field}"),
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ scope: raw }))
        })
}

fn map_user_playlists(
    response: UserPlaylistsEnvelope,
    limit: u32,
    offset: u32,
) -> Result<Page<Playlist>> {
    let items = response
        .playlist
        .into_iter()
        .map(map_playlist)
        .collect::<Result<Vec<_>>>()?;
    let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let next_offset = offset.saturating_add(consumed);
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total: None,
            next_offset: response.more.then_some(next_offset),
            has_more: response.more,
            extensions: Default::default(),
        },
    })
}

fn map_subscribed_albums_response(
    raw: Value,
    request: &PageRequest,
    limit: u32,
) -> Result<Page<Album>> {
    let response: SubscribedAlbumsEnvelope = parse_body(raw.clone())?;
    let items = response
        .data
        .into_iter()
        .map(|raw| {
            let mut album = map_album_list_item(raw.clone())?;
            album.extensions.remove("catalog_item");
            album.extensions.insert("subscription_item".to_owned(), raw);
            Ok(album)
        })
        .collect::<Result<Vec<_>>>()?;
    let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let next_offset = request.offset.saturating_add(consumed);
    let has_more = response.has_more.unwrap_or_else(|| {
        response
            .count
            .map_or(consumed == limit, |total| u64::from(next_offset) < total)
    });
    let mut metadata = raw;
    if let Some(object) = metadata.as_object_mut() {
        object.remove("data");
    }
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), metadata);
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset: request.offset,
            total: response.count,
            next_offset: has_more.then_some(next_offset),
            has_more,
            extensions,
        },
    })
}

fn map_session_profile(account: &str, status: NeteaseSessionStatus) -> AccountProfile {
    let mut profile = map_account_profile(account, status.account);
    profile.authenticated = status.authenticated;
    profile
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AlbumCatalog {
    New,
    Newest,
}

impl AlbumCatalog {
    fn parse(value: Option<&str>) -> Result<Self> {
        match value.unwrap_or("new").trim().to_ascii_lowercase().as_str() {
            "new" => Ok(Self::New),
            "newest" => Ok(Self::Newest),
            value => Err(TuneWeaveError::invalid_request(format!(
                "unsupported album catalog: {value}"
            ))
            .with_platform(Platform::Netease)
            .with_details(json!({ "allowed": ["new", "newest"] }))),
        }
    }
}

fn normalize_album_area(area: Option<&str>) -> Result<String> {
    let area = area.unwrap_or("ALL").trim().to_ascii_uppercase();
    let normalized = match area.as_str() {
        "ALL" => Some("ALL"),
        "ZH" | "Z_H" => Some("ZH"),
        "EA" | "E_A" => Some("EA"),
        "KR" => Some("KR"),
        "JP" => Some("JP"),
        _ => None,
    };
    normalized.map(str::to_owned).ok_or_else(|| {
        TuneWeaveError::invalid_request("NetEase album area is not supported")
            .with_platform(Platform::Netease)
            .with_details(json!({
                "area": area,
                "allowed": ["ALL", "ZH", "EA", "KR", "JP"]
            }))
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DigitalAlbumCatalog {
    Latest,
    Style,
}

impl DigitalAlbumCatalog {
    fn parse(value: Option<&str>) -> Result<Self> {
        match value
            .unwrap_or("latest")
            .trim()
            .to_ascii_lowercase()
            .as_str()
        {
            "latest" => Ok(Self::Latest),
            "style" => Ok(Self::Style),
            value => Err(TuneWeaveError::invalid_request(format!(
                "unsupported digital album catalog: {value}"
            ))
            .with_platform(Platform::Netease)
            .with_details(json!({ "allowed": ["latest", "style"] }))),
        }
    }

    const fn endpoint(self) -> &'static str {
        match self {
            Self::Latest => "/api/vipmall/albumproduct/list",
            Self::Style => "/api/vipmall/appalbum/album/style",
        }
    }
}

fn normalize_digital_album_area(
    catalog: DigitalAlbumCatalog,
    area: Option<&str>,
) -> Result<String> {
    let area = area
        .unwrap_or(match catalog {
            DigitalAlbumCatalog::Latest => "ALL",
            DigitalAlbumCatalog::Style => "Z_H",
        })
        .trim()
        .to_ascii_uppercase();
    let normalized = match (catalog, area.as_str()) {
        (DigitalAlbumCatalog::Latest, "ALL") => Some("ALL"),
        (DigitalAlbumCatalog::Latest, "ZH" | "Z_H") => Some("ZH"),
        (DigitalAlbumCatalog::Latest, "EA" | "E_A") => Some("EA"),
        (DigitalAlbumCatalog::Style, "ZH" | "Z_H") => Some("Z_H"),
        (DigitalAlbumCatalog::Style, "EA" | "E_A") => Some("E_A"),
        (_, "KR") => Some("KR"),
        (_, "JP") => Some("JP"),
        _ => None,
    };
    normalized.map(str::to_owned).ok_or_else(|| {
        TuneWeaveError::invalid_request("NetEase digital album area is not supported")
            .with_platform(Platform::Netease)
            .with_details(json!({ "area": area, "catalog": format!("{catalog:?}") }))
    })
}

fn netease_album_subscription_request(id: u64, subscribed: bool) -> (&'static str, Value) {
    let path = if subscribed {
        "/api/album/sub"
    } else {
        "/api/album/unsub"
    };
    (path, json!({ "id": id }))
}

fn netease_artist_subscription_request(id: u64, subscribed: bool) -> (&'static str, Value) {
    let path = if subscribed {
        "/api/artist/sub"
    } else {
        "/api/artist/unsub"
    };
    (
        path,
        json!({
            "artistId": id,
            "artistIds": format!("[{id}]")
        }),
    )
}

fn netease_playlist_create_payload(request: &PlaylistCreateRequest) -> Result<Value> {
    let name = validate_playlist_name(&request.name)?;
    let privacy = match request.visibility {
        PlaylistVisibility::Public => "0",
        PlaylistVisibility::Private => "10",
    };
    let kind = match request.kind {
        PlaylistKind::Normal => "NORMAL",
        PlaylistKind::Video => "VIDEO",
        PlaylistKind::Shared => "SHARED",
    };
    Ok(json!({
        "name": name,
        "privacy": privacy,
        "type": kind
    }))
}

fn netease_playlist_cover_update_request(id: u64, image_id: &Value) -> (&'static str, Value) {
    (
        "/api/playlist/cover/update",
        json!({
            "id": id,
            "coverImgId": image_id
        }),
    )
}

fn netease_playlist_update_calls(
    id: u64,
    request: &PlaylistUpdateRequest,
) -> Result<Vec<(&'static str, Value)>> {
    let name = request
        .name
        .as_deref()
        .map(validate_playlist_name)
        .transpose()?;
    let description = request.description.clone();
    let tags = request
        .tags
        .as_deref()
        .map(normalize_playlist_tags)
        .transpose()?;
    if name.is_none() && description.is_none() && tags.is_none() {
        return Err(TuneWeaveError::invalid_request(
            "playlist update requires name, description, or tags",
        )
        .with_platform(Platform::Netease));
    }

    let variant = match request.variant {
        PlaylistMetadataUpdateVariant::Default
            if name.is_some() && description.is_some() && tags.is_some() =>
        {
            PlaylistMetadataUpdateVariant::Batch
        }
        PlaylistMetadataUpdateVariant::Default => PlaylistMetadataUpdateVariant::Individual,
        variant => variant,
    };
    if variant == PlaylistMetadataUpdateVariant::Batch {
        let (Some(name), Some(description), Some(tags)) = (name, description, tags) else {
            return Err(TuneWeaveError::invalid_request(
                "batch playlist update requires name, description, and tags",
            )
            .with_platform(Platform::Netease));
        };
        return Ok(vec![(
            "/api/batch",
            json!({
                "/api/playlist/desc/update": json!({ "id": id, "desc": description }).to_string(),
                "/api/playlist/tags/update": json!({ "id": id, "tags": tags }).to_string(),
                "/api/playlist/update/name": json!({ "id": id, "name": name }).to_string()
            }),
        )]);
    }

    let mut calls = Vec::with_capacity(3);
    if let Some(name) = name {
        calls.push((
            "/api/playlist/update/name",
            json!({ "id": id, "name": name }),
        ));
    }
    if let Some(description) = description {
        calls.push((
            "/api/playlist/desc/update",
            json!({ "id": id, "desc": description }),
        ));
    }
    if let Some(tags) = tags {
        calls.push((
            "/api/playlist/tags/update",
            json!({ "id": id, "tags": tags }),
        ));
    }
    Ok(calls)
}

fn validate_playlist_name(name: &str) -> Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(
            TuneWeaveError::invalid_request("playlist name cannot be empty")
                .with_platform(Platform::Netease),
        );
    }
    if name.len() > 255 || name.chars().any(char::is_control) {
        return Err(TuneWeaveError::invalid_request(
            "playlist name must be at most 255 bytes and contain no control characters",
        )
        .with_platform(Platform::Netease));
    }
    Ok(name.to_owned())
}

fn normalize_playlist_tags(tags: &[String]) -> Result<String> {
    let mut normalized = Vec::with_capacity(tags.len());
    for tag in tags {
        let tag = tag.trim();
        if tag.is_empty() || tag.contains(';') || tag.chars().any(char::is_control) {
            return Err(TuneWeaveError::invalid_request(
                "playlist tags must be non-empty visible values without semicolons",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "tag": tag })));
        }
        normalized.push(tag);
    }
    Ok(normalized.join(";"))
}

fn netease_playlist_ids(resource: &str, references: &[ResourceRef]) -> Result<Vec<u64>> {
    if references.is_empty() {
        return Err(TuneWeaveError::invalid_request(format!(
            "NetEase {resource} requires at least one playlist reference"
        ))
        .with_platform(Platform::Netease));
    }
    references
        .iter()
        .map(|reference| {
            if reference.platform() != Platform::Netease {
                return Err(TuneWeaveError::invalid_request(format!(
                    "NetEase {resource} only accepts netease playlist references"
                ))
                .with_platform(Platform::Netease)
                .with_details(json!({ "ref": reference })));
            }
            parse_numeric_id("playlist", reference.id())
        })
        .collect()
}

fn netease_playlist_track_ids(resource: &str, references: &[ResourceRef]) -> Result<Vec<u64>> {
    if references.is_empty() {
        return Err(TuneWeaveError::invalid_request(format!(
            "NetEase {resource} requires at least one track reference"
        ))
        .with_platform(Platform::Netease));
    }
    references
        .iter()
        .map(|reference| {
            if reference.platform() != Platform::Netease {
                return Err(TuneWeaveError::invalid_request(format!(
                    "NetEase {resource} only accepts netease track references"
                ))
                .with_platform(Platform::Netease)
                .with_details(json!({ "ref": reference })));
            }
            parse_numeric_id("track", reference.id())
        })
        .collect()
}

fn netease_playlist_item_ids(request: &PlaylistItemMutationRequest) -> Result<Vec<String>> {
    if request.item_refs.is_empty() {
        return Err(TuneWeaveError::invalid_request(
            "NetEase playlist item update requires at least one item reference",
        )
        .with_platform(Platform::Netease));
    }
    request
        .item_refs
        .iter()
        .map(|reference| {
            if reference.platform() != Platform::Netease {
                return Err(TuneWeaveError::invalid_request(
                    "NetEase playlist items must use netease references",
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "ref": reference, "kind": request.kind })));
            }
            match request.kind {
                PlaylistItemKind::Track => {
                    parse_numeric_id("track", reference.id()).map(|id| id.to_string())
                }
                PlaylistItemKind::Video => {
                    let id = reference.id().trim();
                    if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
                        return Err(TuneWeaveError::invalid_request(
                            "NetEase playlist video ids must be between 1 and 256 visible characters",
                        )
                        .with_platform(Platform::Netease)
                        .with_details(json!({ "ref": reference })));
                    }
                    Ok(id.to_owned())
                }
            }
        })
        .collect()
}

fn netease_playlist_track_mutation_payload(
    playlist_id: u64,
    action: PlaylistItemMutationAction,
    item_ids: &[String],
) -> Value {
    let operation = match action {
        PlaylistItemMutationAction::Add => "add",
        PlaylistItemMutationAction::Remove => "del",
    };
    json!({
        "op": operation,
        "pid": playlist_id,
        "trackIds": json!(item_ids).to_string(),
        "imme": "true"
    })
}

fn netease_playlist_video_mutation_request(
    playlist_id: u64,
    action: PlaylistItemMutationAction,
    item_ids: &[String],
) -> (&'static str, Value) {
    let path = match action {
        PlaylistItemMutationAction::Add => "/api/playlist/track/add",
        PlaylistItemMutationAction::Remove => "/api/playlist/track/delete",
    };
    let tracks = item_ids
        .iter()
        .map(|id| json!({ "type": 3, "id": id }))
        .collect::<Vec<_>>();
    (
        path,
        json!({
            "id": playlist_id,
            "tracks": json!(tracks).to_string()
        }),
    )
}

fn netease_playlist_track_order_request(
    playlist_id: u64,
    track_ids: &[u64],
) -> (&'static str, Value) {
    (
        "/api/playlist/manipulate/tracks",
        json!({
            "pid": playlist_id,
            "trackIds": format!("[{}]", join_numeric_ids(track_ids)),
            "op": "update"
        }),
    )
}

fn netease_account_playlist_order_request(ids: &[u64]) -> (&'static str, Value) {
    (
        "/api/playlist/order/update",
        json!({ "ids": format!("[{}]", join_numeric_ids(ids)) }),
    )
}

fn join_numeric_ids(ids: &[u64]) -> String {
    ids.iter().map(u64::to_string).collect::<Vec<_>>().join(",")
}

fn netease_digital_album_chart_request(
    request: &DigitalAlbumChartRequest,
) -> Result<(String, Value)> {
    let period = match request.period {
        DigitalAlbumChartPeriod::Daily => "daily",
        DigitalAlbumChartPeriod::Week => "week",
        DigitalAlbumChartPeriod::Year => "year",
        DigitalAlbumChartPeriod::Total => "total",
    };
    if request.year.is_some() && request.period != DigitalAlbumChartPeriod::Year {
        return Err(TuneWeaveError::invalid_request(
            "year is only supported for the NetEase yearly digital album chart",
        )
        .with_platform(Platform::Netease));
    }
    let album_type = match request.kind {
        DigitalAlbumChartKind::Album => 0,
        DigitalAlbumChartKind::Single => 1,
    };
    let mut payload = json!({ "albumType": album_type });
    if let Some(year) = request.year {
        payload["year"] = json!(year);
    }
    Ok((
        format!("/api/feealbum/songsaleboard/{period}/type"),
        payload,
    ))
}

const fn netease_artist_chart_area(area: ArtistChartArea) -> u8 {
    match area {
        ArtistChartArea::Chinese => 1,
        ArtistChartArea::Western => 2,
        ArtistChartArea::Korean => 3,
        ArtistChartArea::Japanese => 4,
    }
}

fn netease_dimension_chart_payload(request: &DimensionChartRequest) -> Result<Value> {
    let (chart_code, target_id, target_type) = validated_dimension_chart_parts(request)?;
    Ok(json!({
        "chartCode": chart_code,
        "targetId": target_id,
        "targetType": target_type
    }))
}

fn validated_dimension_chart_parts(request: &DimensionChartRequest) -> Result<(&str, &str, &str)> {
    Ok((
        required_dimension_chart_value("chart_code", &request.chart_code)?,
        required_dimension_chart_value("target_id", &request.target_id)?,
        required_dimension_chart_value("target_type", &request.target_type)?,
    ))
}

fn required_dimension_chart_value<'a>(name: &str, value: &'a str) -> Result<&'a str> {
    let value = value.trim();
    if value.is_empty() {
        return Err(TuneWeaveError::invalid_request(format!(
            "dimension chart {name} cannot be empty"
        ))
        .with_platform(Platform::Netease));
    }
    Ok(value)
}

fn normalize_account_label(account: Option<&str>) -> Result<&str> {
    let account = account.unwrap_or("default").trim();
    let account = if account.is_empty() {
        "default"
    } else {
        account
    };
    if account.len() > 64 {
        return Err(
            TuneWeaveError::invalid_request("account alias cannot exceed 64 bytes")
                .with_platform(Platform::Netease),
        );
    }
    Ok(account)
}

fn account_store_error() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::InternalError,
        "NetEase account store lock is poisoned",
    )
    .with_platform(Platform::Netease)
}

fn validate_session_cookie(cookie: &str) -> Result<()> {
    if crate::client::has_authenticated_cookie(Some(cookie)) {
        return Ok(());
    }
    Err(TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "NetEase session cookie does not contain MUSIC_U",
    )
    .with_platform(Platform::Netease))
}

async fn request_netease_streams(
    client: &NeteaseClient,
    tracks: &[Track],
    request: &StreamRequest,
) -> Result<StreamBatch> {
    let ids = tracks
        .iter()
        .map(validate_netease_stream_track)
        .collect::<Result<Vec<_>>>()?;
    let (variant, path, payload, level) = netease_stream_request(&ids, request);
    let response = match variant {
        StreamVariant::Legacy => client.request_api(path, payload).await?,
        StreamVariant::Modern => client.request_xeapi(path, payload).await?,
        StreamVariant::Default => unreachable!("default stream variant is resolved above"),
    };
    map_netease_stream_batch(
        tracks,
        request,
        client.is_authenticated(),
        variant,
        path,
        level,
        response.body,
    )
}

fn map_netease_stream_batch(
    tracks: &[Track],
    request: &StreamRequest,
    authenticated: bool,
    variant: StreamVariant,
    path: &str,
    level: Option<&str>,
    response: Value,
) -> Result<StreamBatch> {
    ensure_success(&response)?;
    let ids = tracks
        .iter()
        .map(validate_netease_stream_track)
        .collect::<Result<Vec<_>>>()?;
    let raw_response = response.clone();
    let raw_items = response
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase stream response is missing its data array",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "response": response }))
        })?;
    let envelope: StreamEnvelope = parse_body(response)?;
    let outcomes = tracks
        .iter()
        .zip(ids)
        .map(|(track, id)| {
            let raw = raw_items
                .iter()
                .find(|item| item.get("id").and_then(json_u64) == Some(id))
                .cloned();
            let Some(stream) = envelope.data.iter().find(|stream| stream.id == id).cloned() else {
                return stream_outcome_error(
                    track,
                    TuneWeaveError::new(
                        ErrorCode::UpstreamError,
                        "NetEase omitted a requested stream result",
                    )
                    .with_platform(Platform::Netease)
                    .with_details(json!({ "id": id })),
                    raw,
                );
            };
            match map_stream(track, request, stream, authenticated) {
                Ok(stream) => StreamOutcome {
                    track_ref: track.resource_ref.clone(),
                    status: ResolutionStatus::Success,
                    stream: Some(stream),
                    error_code: None,
                    error: None,
                    extensions: raw
                        .map(|raw| Extensions::from([("response_item".to_owned(), raw)]))
                        .unwrap_or_default(),
                },
                Err(error) => stream_outcome_error(track, error, raw),
            }
        })
        .collect();
    let mut extensions = Extensions::from([
        ("variant".to_owned(), json!(variant)),
        ("request_path".to_owned(), json!(path)),
        ("response".to_owned(), raw_response),
    ]);
    if let Some(level) = level {
        extensions.insert("level".to_owned(), json!(level));
    }
    Ok(StreamBatch {
        outcomes,
        extensions,
    })
}

fn validate_netease_stream_track(track: &Track) -> Result<u64> {
    if track.platform != Platform::Netease
        || track.resource_ref.platform() != Platform::Netease
        || track.resource_ref.id() != track.id
    {
        return Err(TuneWeaveError::invalid_request(
            "NetEase provider can only resolve consistent NetEase tracks",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "track_ref": track.resource_ref })));
    }
    parse_numeric_id("track", &track.id)
}

fn netease_stream_request(
    ids: &[u64],
    request: &StreamRequest,
) -> (StreamVariant, &'static str, Value, Option<&'static str>) {
    match request.variant {
        StreamVariant::Legacy => (
            StreamVariant::Legacy,
            "/api/song/enhance/player/url",
            json!({
                "ids": Value::Array(ids.iter().map(|id| json!(id.to_string())).collect())
                    .to_string(),
                "br": request.bitrate.unwrap_or_else(|| requested_bitrate(request.quality))
            }),
            None,
        ),
        StreamVariant::Default | StreamVariant::Modern => {
            let level = netease_stream_level(request.quality);
            let mut payload = json!({
                "ids": format!(
                    "[{}]",
                    ids.iter().map(u64::to_string).collect::<Vec<_>>().join(",")
                ),
                "level": level,
                "encodeType": "flac"
            });
            if level == "sky" {
                payload["immerseType"] = json!("c51");
            }
            (
                StreamVariant::Modern,
                "/api/song/enhance/player/url/v1",
                payload,
                Some(level),
            )
        }
    }
}

fn netease_download_request(
    id: u64,
    request: &StreamRequest,
) -> (StreamVariant, &'static str, Value, Option<&'static str>) {
    match request.variant {
        StreamVariant::Legacy => (
            StreamVariant::Legacy,
            "/api/song/enhance/download/url",
            json!({
                "id": id.to_string(),
                "br": request
                    .bitrate
                    .unwrap_or_else(|| requested_bitrate(request.quality))
            }),
            None,
        ),
        StreamVariant::Default | StreamVariant::Modern => {
            let level = netease_stream_level(request.quality);
            (
                StreamVariant::Modern,
                "/api/song/enhance/download/url/v1",
                json!({
                    "id": id.to_string(),
                    "immerseType": "c51",
                    "level": level
                }),
                Some(level),
            )
        }
    }
}

fn map_netease_download(
    track: &Track,
    request: &StreamRequest,
    variant: StreamVariant,
    path: &str,
    requested_level: Option<&str>,
    response: Value,
) -> Result<MediaDownload> {
    ensure_success(&response)?;
    let id = validate_netease_stream_track(track)?;
    let raw_response = response.clone();
    let data = response.get("data").ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase download response is missing data",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "response": response }))
    })?;
    let item = if let Some(items) = data.as_array() {
        items
            .iter()
            .find(|item| item.get("id").and_then(json_u64) == Some(id))
            .cloned()
            .ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "NetEase omitted a requested download result",
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "id": id, "response": raw_response }))
            })?
    } else {
        data.clone()
    };
    let download: StreamData = parse_body(item.clone())?;
    if download.id != id {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase returned a download result for the wrong track",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "expected": id, "actual": download.id })));
    }
    let url = download
        .url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned);
    let actual_quality = stream_quality(download.level.as_deref(), download.br);
    let mut extensions = Extensions::from([
        ("variant".to_owned(), json!(variant)),
        ("request_path".to_owned(), json!(path)),
        ("response_item".to_owned(), item),
        ("response".to_owned(), raw_response),
    ]);
    if let Some(level) = requested_level {
        extensions.insert("requested_level".to_owned(), json!(level));
    }
    let format = normalized_string(download.kind);
    let codec = normalized_string(download.encode_type).or_else(|| format.clone());
    Ok(MediaDownload {
        track_ref: track.resource_ref.clone(),
        platform: Platform::Netease,
        available: url.is_some(),
        url,
        headers: BTreeMap::new(),
        expires_at: download
            .expi
            .filter(|expires_in_seconds| *expires_in_seconds > 0)
            .and_then(expiration_rfc3339),
        format,
        codec,
        bitrate: download.br,
        size: download.size,
        duration_ms: download
            .time
            .filter(|duration| *duration > 0)
            .or_else(|| track.duration_ms.filter(|duration| *duration > 0)),
        requested_quality: request.quality,
        actual_quality,
        platform_code: download.code,
        fee: download.fee,
        message: download.message,
        extensions,
    })
}

const fn netease_stream_level(quality: Quality) -> &'static str {
    match quality {
        Quality::Auto | Quality::High => "exhigh",
        Quality::Low | Quality::Standard => "standard",
        Quality::Higher => "higher",
        Quality::Lossless => "lossless",
        Quality::Hires => "hires",
        Quality::Surround => "jyeffect",
        Quality::Spatial => "sky",
        Quality::Dolby => "dolby",
        Quality::Master => "jymaster",
    }
}

fn stream_outcome_error(track: &Track, error: TuneWeaveError, raw: Option<Value>) -> StreamOutcome {
    let mut extensions = Extensions::from([("details".to_owned(), error.details)]);
    if let Some(raw) = raw {
        extensions.insert("response_item".to_owned(), raw);
    }
    StreamOutcome {
        track_ref: track.resource_ref.clone(),
        status: netease_stream_error_status(error.code),
        stream: None,
        error_code: Some(error.code),
        error: Some(error.message),
        extensions,
    }
}

const fn netease_stream_error_status(code: ErrorCode) -> ResolutionStatus {
    match code {
        ErrorCode::AuthenticationRequired => ResolutionStatus::AuthenticationRequired,
        ErrorCode::PermissionDenied => ResolutionStatus::PermissionDenied,
        ErrorCode::MatchRejected => ResolutionStatus::NoMatch,
        ErrorCode::CapabilityNotSupported
        | ErrorCode::PlatformUnavailable
        | ErrorCode::ResourceNotFound => ResolutionStatus::Unavailable,
        ErrorCode::InvalidRequest
        | ErrorCode::Conflict
        | ErrorCode::RateLimited
        | ErrorCode::UpstreamError
        | ErrorCode::UpstreamTimeout
        | ErrorCode::InternalError => ResolutionStatus::UpstreamError,
    }
}

fn map_stream(
    track: &Track,
    request: &StreamRequest,
    stream: StreamData,
    authenticated: bool,
) -> Result<MediaStream> {
    let url = stream
        .url
        .as_deref()
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| stream_unavailable(&stream, authenticated))?;
    let actual_quality = stream_quality(stream.level.as_deref(), stream.br);
    let trial = stream.free_trial_info.and_then(|trial| {
        let start_ms = trial.start?.checked_mul(1_000)?;
        let end_ms = trial.end?.checked_mul(1_000)?;
        (end_ms > start_ms).then_some(TrialWindow { start_ms, end_ms })
    });
    let format = normalized_string(stream.kind);
    let codec = normalized_string(stream.encode_type).or_else(|| format.clone());

    Ok(MediaStream {
        url,
        backup_urls: Vec::new(),
        headers: BTreeMap::new(),
        expires_at: stream
            .expi
            .filter(|expires_in_seconds| *expires_in_seconds > 0)
            .and_then(expiration_rfc3339),
        format,
        codec,
        bitrate: stream.br,
        size: stream.size,
        duration_ms: stream
            .time
            .filter(|duration| *duration > 0)
            .or_else(|| track.duration_ms.filter(|duration| *duration > 0)),
        requested_quality: request.quality,
        actual_quality,
        trial,
        origin_track: Some(track.resource_ref.clone()),
        resolved_track: track.resource_ref.clone(),
        resolved_platform: Platform::Netease,
        match_score: Some(1.0),
        attempts: Vec::new(),
    })
}

fn map_track_availability(
    id: u64,
    requested_bitrate: u64,
    response: Value,
) -> Result<TrackAvailability> {
    ensure_success(&response)?;
    let mut safe_response = response.clone();
    sanitize_player_urls(&mut safe_response);
    let response: StreamEnvelope = parse_body(response)?;
    let stream = response
        .data
        .into_iter()
        .find(|stream| stream.id == id)
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase omitted the requested availability result",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "id": id }))
        })?;
    let playable = stream.code == Some(200);
    let track_ref = ResourceRef::new(Platform::Netease, id.to_string()).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid availability track id: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), safe_response);
    Ok(TrackAvailability {
        track_ref,
        playable,
        requested_bitrate,
        actual_bitrate: stream.br.filter(|bitrate| *bitrate > 0),
        platform_code: stream.code,
        message: if playable {
            "ok".to_owned()
        } else {
            "亲爱的,暂无版权".to_owned()
        },
        extensions,
    })
}

fn sanitize_player_urls(response: &mut Value) {
    if let Some(items) = response.get_mut("data").and_then(Value::as_array_mut) {
        for item in items {
            if let Some(item) = item.as_object_mut() {
                item.insert("url".to_owned(), Value::Null);
            }
        }
    }
}

fn stream_unavailable(stream: &StreamData, authenticated: bool) -> TuneWeaveError {
    let code = if !authenticated && stream.fee.is_some_and(|fee| fee > 0) {
        ErrorCode::AuthenticationRequired
    } else if stream.code == Some(404) {
        ErrorCode::ResourceNotFound
    } else {
        ErrorCode::PermissionDenied
    };
    TuneWeaveError::new(code, "NetEase did not return a playable stream")
        .with_platform(Platform::Netease)
        .with_details(json!({
            "id": stream.id,
            "upstream_code": stream.code,
            "fee": stream.fee,
            "level": stream.level
        }))
}

fn requested_bitrate(quality: Quality) -> u64 {
    match quality {
        Quality::Low | Quality::Standard => 128_000,
        Quality::Higher => 192_000,
        Quality::High => 320_000,
        Quality::Auto
        | Quality::Lossless
        | Quality::Hires
        | Quality::Surround
        | Quality::Spatial
        | Quality::Dolby
        | Quality::Master => 999_000,
    }
}

fn stream_quality(level: Option<&str>, bitrate: Option<u64>) -> Quality {
    match level.unwrap_or_default().to_ascii_lowercase().as_str() {
        "standard" => Quality::Standard,
        "higher" => Quality::Higher,
        "exhigh" => Quality::High,
        "lossless" => Quality::Lossless,
        "hires" => Quality::Hires,
        "jyeffect" => Quality::Surround,
        "sky" => Quality::Spatial,
        "dolby" => Quality::Dolby,
        "jymaster" => Quality::Master,
        _ => bitrate.map_or(Quality::Auto, quality_for_bitrate),
    }
}

fn optional_quality(level: Option<&str>, bitrate: Option<u64>) -> Option<Quality> {
    let quality = stream_quality(level, bitrate);
    (quality != Quality::Auto).then_some(quality)
}

fn quality_for_bitrate(bitrate: u64) -> Quality {
    match bitrate {
        0 => Quality::Auto,
        1..=96_000 => Quality::Low,
        96_001..=128_000 => Quality::Standard,
        128_001..=256_000 => Quality::Higher,
        256_001..=500_000 => Quality::High,
        500_001..=1_500_000 => Quality::Lossless,
        1_500_001.. => Quality::Hires,
    }
}

const fn quality_rank(quality: Quality) -> u8 {
    match quality {
        Quality::Auto => 0,
        Quality::Low => 1,
        Quality::Standard => 2,
        Quality::Higher => 3,
        Quality::High => 4,
        Quality::Lossless => 5,
        Quality::Hires => 6,
        Quality::Surround => 7,
        Quality::Spatial => 8,
        Quality::Dolby => 9,
        Quality::Master => 10,
    }
}

fn expiration_rfc3339(expires_in_seconds: u64) -> Option<String> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    unix_rfc3339(now.checked_add(expires_in_seconds)?)
}

fn unix_rfc3339(timestamp: u64) -> Option<String> {
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

fn unix_millis_now() -> Result<u64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::InternalError,
                format!("system clock is before the Unix epoch: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    u64::try_from(duration.as_millis()).map_err(|_| {
        TuneWeaveError::new(
            ErrorCode::InternalError,
            "current Unix timestamp does not fit in 64 bits",
        )
        .with_platform(Platform::Netease)
    })
}

fn map_lyrics(id: &str, lyrics: LyricsEnvelope) -> Result<Lyrics> {
    let track_ref = ResourceRef::new(Platform::Netease, id.to_owned()).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid lyrics track id: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    let plain = lyric_text(lyrics.lrc.as_ref());
    let translated = lyric_text(lyrics.tlyric.as_ref());
    let romanized = lyric_text(lyrics.romalrc.as_ref());
    let word_synced = lyric_text(lyrics.yrc.as_ref());
    let format = primary_lyric_format(word_synced.is_some(), plain.is_some()).to_owned();
    let mut contributors = Vec::new();
    if let Some(contributor) = map_lyric_user("lyrics", lyrics.lyric_user)? {
        contributors.push(contributor);
    }
    if let Some(contributor) = map_lyric_user("translation", lyrics.trans_user)? {
        contributors.push(contributor);
    }
    let mut extensions = Extensions::new();
    insert_extension(&mut extensions, "pure_music", lyrics.pure_music);
    insert_lyric_extension(
        &mut extensions,
        "word_synced_translated",
        lyrics.ytlrc.as_ref(),
    );
    insert_lyric_extension(
        &mut extensions,
        "word_synced_romanized",
        lyrics.yromalrc.as_ref(),
    );
    insert_lyric_version(&mut extensions, "plain_version", lyrics.lrc.as_ref());
    insert_lyric_version(
        &mut extensions,
        "translated_version",
        lyrics.tlyric.as_ref(),
    );
    insert_lyric_version(
        &mut extensions,
        "romanized_version",
        lyrics.romalrc.as_ref(),
    );
    insert_lyric_version(&mut extensions, "word_synced_version", lyrics.yrc.as_ref());

    Ok(Lyrics {
        track_ref,
        plain,
        translated,
        romanized,
        word_synced,
        format,
        contributors,
        extensions,
    })
}

const fn primary_lyric_format(has_word_synced: bool, has_plain_lrc: bool) -> &'static str {
    match (has_word_synced, has_plain_lrc) {
        (true, _) => "yrc",
        (false, true) => "lrc",
        (false, false) => "plain",
    }
}

fn lyric_text(lyrics: Option<&LyricText>) -> Option<String> {
    lyrics
        .and_then(|lyrics| lyrics.lyric.as_deref())
        .map(str::trim)
        .filter(|lyrics| !lyrics.is_empty())
        .map(str::to_owned)
}

fn map_lyric_user(role: &str, user: Option<LyricUser>) -> Result<Option<LyricContributor>> {
    let Some(user) = user else {
        return Ok(None);
    };
    let Some(name) = user
        .nickname
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
    else {
        return Ok(None);
    };
    let resource_ref = user
        .user_id
        .filter(|id| *id > 0)
        .or_else(|| user.userid.filter(|id| *id > 0))
        .or_else(|| user.id.filter(|id| *id > 0))
        .map(|id| ResourceRef::new(Platform::Netease, id.to_string()))
        .transpose()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid lyric contributor id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    Ok(Some(LyricContributor {
        role: role.to_owned(),
        resource_ref,
        name,
    }))
}

fn insert_lyric_extension(extensions: &mut Extensions, name: &str, lyrics: Option<&LyricText>) {
    if let Some(lyrics) = lyric_text(lyrics) {
        extensions.insert(name.to_owned(), json!(lyrics));
    }
}

fn insert_lyric_version(extensions: &mut Extensions, name: &str, lyrics: Option<&LyricText>) {
    if let Some(version) = lyrics.and_then(|lyrics| lyrics.version) {
        extensions.insert(name.to_owned(), json!(version));
    }
}

fn map_playlist(playlist: PlaylistDetail) -> Result<Playlist> {
    let resource_ref =
        ResourceRef::new(Platform::Netease, playlist.id.to_string()).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid playlist id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let creator = playlist
        .creator
        .map(
            |creator| -> std::result::Result<ArtistSummary, ParseResourceRefError> {
                Ok(ArtistSummary {
                    resource_ref: Some(ResourceRef::new(
                        Platform::Netease,
                        creator.user_id.to_string(),
                    )?),
                    name: creator.nickname,
                })
            },
        )
        .transpose()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid playlist creator id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let mut extensions = Extensions::new();
    insert_extension(&mut extensions, "create_time_ms", playlist.create_time);
    insert_extension(&mut extensions, "update_time_ms", playlist.update_time);
    insert_extension(&mut extensions, "privacy", playlist.privacy);
    insert_extension(&mut extensions, "special_type", playlist.special_type);
    insert_extension(&mut extensions, "play_count", playlist.play_count);
    insert_extension(&mut extensions, "copywriter", playlist.copywriter);
    insert_extension(&mut extensions, "algorithm", playlist.alg);

    Ok(Playlist {
        resource_ref,
        platform: Platform::Netease,
        id: playlist.id.to_string(),
        name: playlist.name,
        description: playlist.description.unwrap_or_default(),
        cover_url: playlist.cover_img_url,
        creator,
        track_count: playlist
            .track_count
            .or(Some(playlist.track_ids.len() as u64)),
        tags: playlist.tags,
        subscribed: playlist.subscribed,
        created_at: None,
        updated_at: None,
        extensions,
    })
}

fn map_netease_playlist_create_result(response: Value) -> Result<PlaylistMutationResult> {
    let id = ["/playlist/id", "/data/id", "/id"]
        .into_iter()
        .find_map(|pointer| {
            response
                .pointer(pointer)
                .and_then(json_u64)
                .filter(|id| *id > 0)
        })
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase playlist creation response is missing the new playlist id",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "response": response }))
        })?;
    let playlist = response
        .get("playlist")
        .cloned()
        .and_then(|playlist| serde_json::from_value::<PlaylistDetail>(playlist).ok())
        .map(map_playlist)
        .transpose()?;
    if playlist
        .as_ref()
        .is_some_and(|playlist| playlist.id != id.to_string())
    {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase playlist creation response contains conflicting playlist ids",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "response": response, "id": id })));
    }
    let mut result =
        map_netease_playlist_mutation_result(id, PlaylistMutationAction::Create, response)?;
    result.playlist = playlist;
    Ok(result)
}

fn map_netease_playlist_mutation_result(
    id: u64,
    action: PlaylistMutationAction,
    response: Value,
) -> Result<PlaylistMutationResult> {
    let playlist_ref = ResourceRef::new(Platform::Netease, id.to_string()).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid playlist id: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    Ok(PlaylistMutationResult {
        playlist_ref,
        action,
        playlist: None,
        extensions: Extensions::from([("response".to_owned(), response)]),
    })
}

fn map_netease_playlist_item_mutation_result(
    playlist_id: u64,
    action: PlaylistItemMutationAction,
    request: &PlaylistItemMutationRequest,
    response: Value,
    initial_response: Option<Value>,
) -> Result<PlaylistItemMutationResult> {
    let playlist_ref =
        ResourceRef::new(Platform::Netease, playlist_id.to_string()).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid playlist id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let snapshot_id = ["/snapshotId", "/snapshot_id", "/data/snapshotId"]
        .into_iter()
        .find_map(|pointer| response.pointer(pointer).and_then(usable_resource_id));
    let cloud_track_count = ["/cloudCount", "/cloudTrackCount", "/data/cloudCount"]
        .into_iter()
        .find_map(|pointer| response.pointer(pointer).and_then(json_u64));
    let mut extensions = Extensions::from([("response".to_owned(), response)]);
    if let Some(initial_response) = initial_response {
        extensions.insert("initial_response".to_owned(), initial_response);
        extensions.insert("retried_code_512".to_owned(), Value::Bool(true));
    }
    Ok(PlaylistItemMutationResult {
        playlist_ref,
        item_refs: request.item_refs.clone(),
        kind: request.kind,
        action,
        snapshot_id,
        cloud_track_count,
        extensions,
    })
}

fn map_netease_playlist_track_order_result(
    playlist_id: u64,
    track_refs: Vec<ResourceRef>,
    response: Value,
) -> Result<PlaylistTrackOrderResult> {
    let playlist_ref =
        ResourceRef::new(Platform::Netease, playlist_id.to_string()).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid playlist id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let snapshot_id = ["/snapshotId", "/snapshot_id", "/data/snapshotId"]
        .into_iter()
        .find_map(|pointer| response.pointer(pointer).and_then(usable_resource_id));
    Ok(PlaylistTrackOrderResult {
        playlist_ref,
        track_refs,
        snapshot_id,
        extensions: Extensions::from([("response".to_owned(), response)]),
    })
}

fn map_album(album: AlbumDetail) -> Result<Album> {
    let resource_ref =
        ResourceRef::new(Platform::Netease, album.id.to_string()).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid album id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let artists = album
        .artists
        .into_iter()
        .map(
            |artist| -> std::result::Result<ArtistSummary, ParseResourceRefError> {
                Ok(ArtistSummary {
                    resource_ref: Some(ResourceRef::new(Platform::Netease, artist.id.to_string())?),
                    name: artist.name,
                })
            },
        )
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid album artist id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let mut extensions = Extensions::new();
    insert_extension(&mut extensions, "sub_type", album.sub_type);
    insert_extension(&mut extensions, "paid", album.paid);
    insert_extension(&mut extensions, "on_sale", album.on_sale);
    insert_extension(&mut extensions, "mark", album.mark);
    insert_extension(&mut extensions, "publish_time_ms", album.publish_time);
    Ok(Album {
        resource_ref,
        platform: Platform::Netease,
        id: album.id.to_string(),
        name: album.name,
        aliases: album.alia,
        artists,
        description: album.description.unwrap_or_default(),
        cover_url: album.pic_url,
        published_at: album
            .publish_time
            .and_then(|milliseconds| unix_rfc3339(milliseconds / 1_000)),
        track_count: album.size,
        company: album.company,
        kind: album.kind,
        extensions,
    })
}

fn map_album_list_item(raw: Value) -> Result<Album> {
    let album: AlbumDetail = parse_body(raw.clone())?;
    let mut album = map_album(album)?;
    album.extensions.insert("catalog_item".to_owned(), raw);
    Ok(album)
}

const fn netease_artist_category(category: ArtistCategory) -> i64 {
    match category {
        ArtistCategory::All => -1,
        ArtistCategory::Male => 1,
        ArtistCategory::Female => 2,
        ArtistCategory::Group => 3,
    }
}

const fn netease_artist_area(area: ArtistArea) -> i64 {
    match area {
        ArtistArea::All => -1,
        ArtistArea::Chinese => 7,
        ArtistArea::Western => 96,
        ArtistArea::Japanese => 8,
        ArtistArea::Korean => 16,
        ArtistArea::Other => 0,
    }
}

fn netease_artist_initial(initial: Option<&str>) -> Result<Option<i64>> {
    let Some(initial) = initial.map(str::trim).filter(|initial| !initial.is_empty()) else {
        return Ok(None);
    };
    match initial.to_ascii_lowercase().as_str() {
        "hot" | "-1" => return Ok(Some(-1)),
        "other" | "#" | "0" => return Ok(Some(0)),
        _ => {}
    }
    let bytes = initial.as_bytes();
    if bytes.len() == 1 && bytes[0].is_ascii_alphabetic() {
        return Ok(Some(i64::from(bytes[0].to_ascii_uppercase())));
    }
    Err(
        TuneWeaveError::invalid_request("initial must be one ASCII letter, hot, or other")
            .with_platform(Platform::Netease)
            .with_details(json!({ "initial": initial })),
    )
}

fn map_artist_list_response(
    response: ArtistListEnvelope,
    limit: u32,
    offset: u32,
) -> Result<Page<Artist>> {
    let items = response
        .artists
        .into_iter()
        .map(map_artist_list_item)
        .collect::<Result<Vec<_>>>()?;
    let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let next_offset = offset.saturating_add(consumed);
    let has_more = response.more.unwrap_or(consumed == limit);
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total: None,
            next_offset: (has_more && consumed > 0).then_some(next_offset),
            has_more,
            extensions: Extensions::new(),
        },
    })
}

fn map_artist_list_item(raw: Value) -> Result<Artist> {
    let item: ArtistListItem = parse_body(raw.clone())?;
    let resource_ref =
        ResourceRef::new(Platform::Netease, item.id.to_string()).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid artist id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let mut aliases = Vec::new();
    for alias in item
        .alias
        .into_iter()
        .chain(item.translated_names)
        .chain(item.trans)
    {
        let alias = alias.trim();
        if !alias.is_empty() && alias != item.name && !aliases.iter().any(|item| item == alias) {
            aliases.push(alias.to_owned());
        }
    }
    let mut extensions = Extensions::new();
    extensions.insert("catalog_item".to_owned(), raw);
    Ok(Artist {
        resource_ref,
        platform: Platform::Netease,
        id: item.id.to_string(),
        name: item.name,
        aliases,
        description: item.brief_description.unwrap_or_default(),
        biography_sections: Vec::new(),
        avatar_url: item.avatar_url,
        cover_url: item.cover_url,
        album_count: item.album_count,
        track_count: item.track_count,
        mv_count: item.mv_count,
        video_count: None,
        identities: Vec::new(),
        extensions,
    })
}

fn map_artist_albums_response(
    response: ArtistAlbumsEnvelope,
    limit: u32,
    offset: u32,
) -> Result<Page<Album>> {
    let items = response
        .albums
        .into_iter()
        .map(map_artist_album_item)
        .collect::<Result<Vec<_>>>()?;
    let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let next_offset = offset.saturating_add(consumed);
    let has_more = response.more.unwrap_or(consumed == limit);
    let mut extensions = Extensions::new();
    insert_extension(&mut extensions, "artist", response.artist);
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total: response.total,
            next_offset: (has_more && consumed > 0).then_some(next_offset),
            has_more,
            extensions,
        },
    })
}

fn map_artist_tracks_response(
    response: ArtistTracksEnvelope,
    limit: u32,
    offset: u32,
) -> Result<Page<Track>> {
    let total = response.total;
    let items = response
        .songs
        .into_iter()
        .map(|raw| {
            let song: Song = parse_body(raw.clone())?;
            let mut track = map_song(song, None)?;
            track.extensions.insert("artist_track".to_owned(), raw);
            Ok(track)
        })
        .collect::<Result<Vec<_>>>()?;
    let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let next_offset = offset.saturating_add(consumed);
    let has_more = response
        .more
        .unwrap_or_else(|| total.map_or(consumed == limit, |total| u64::from(next_offset) < total));
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total,
            next_offset: (has_more && consumed > 0).then_some(next_offset),
            has_more,
            extensions: Extensions::new(),
        },
    })
}

fn map_artist_sublist_response(
    response: ArtistSublistEnvelope,
    limit: u32,
    offset: u32,
) -> Result<Page<Artist>> {
    let total = response.count;
    let items = response
        .data
        .into_iter()
        .map(|raw| {
            let mut artist = map_artist_list_item(raw.clone())?;
            artist.extensions.remove("catalog_item");
            artist.extensions.insert("following_item".to_owned(), raw);
            Ok(artist)
        })
        .collect::<Result<Vec<_>>>()?;
    let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let next_offset = offset.saturating_add(consumed);
    let has_more = response
        .has_more
        .unwrap_or_else(|| total.map_or(consumed == limit, |total| u64::from(next_offset) < total));
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total,
            next_offset: (has_more && consumed > 0).then_some(next_offset),
            has_more,
            extensions: Extensions::new(),
        },
    })
}

fn map_artist_top_tracks_response(
    response: ArtistTopTracksEnvelope,
    raw_response: Value,
) -> Result<Page<Track>> {
    let mut privileges = response
        .privileges
        .into_iter()
        .map(|privilege| (privilege.id, privilege))
        .collect::<HashMap<_, _>>();
    let items = response
        .songs
        .into_iter()
        .map(|raw| {
            let song: Song = parse_body(raw.clone())?;
            let privilege = privileges.remove(&song.id);
            let mut track = map_song(song, privilege)?;
            track.extensions.insert("artist_top_track".to_owned(), raw);
            Ok(track)
        })
        .collect::<Result<Vec<_>>>()?;
    let total = items.len() as u64;
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw_response);
    Ok(Page {
        items,
        pagination: PageMeta {
            limit: 50,
            offset: 0,
            total: Some(total),
            next_offset: None,
            has_more: false,
            extensions,
        },
    })
}

fn map_artist_fans_response(
    response: ArtistFansEnvelope,
    limit: u32,
    offset: u32,
) -> Result<Page<User>> {
    let items = response
        .data
        .into_iter()
        .map(map_artist_fan)
        .collect::<Result<Vec<_>>>()?;
    let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let next_offset = offset.saturating_add(consumed);
    let has_more = response.has_more.unwrap_or(consumed == limit);
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total: response.total,
            next_offset: (has_more && consumed > 0).then_some(next_offset),
            has_more,
            extensions: Extensions::new(),
        },
    })
}

fn map_artist_fan(raw: Value) -> Result<User> {
    let profile_raw = raw.get("userProfile").cloned().ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase artist fan item is missing userProfile",
        )
        .with_platform(Platform::Netease)
    })?;
    let profile: ArtistFanProfile = parse_body(profile_raw)?;
    let resource_ref =
        ResourceRef::new(Platform::Netease, profile.user_id.to_string()).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid fan user id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let mut extensions = Extensions::new();
    extensions.insert("fan".to_owned(), raw);
    Ok(User {
        resource_ref,
        platform: Platform::Netease,
        id: profile.user_id.to_string(),
        name: profile.nickname,
        avatar_url: profile.avatar_url,
        signature: profile.signature,
        followed: profile.followed,
        mutual: profile.mutual,
        extensions,
    })
}

fn map_artist_mvs_response(
    response: ArtistMvsEnvelope,
    limit: u32,
    offset: u32,
) -> Result<Page<Video>> {
    let items = response
        .mvs
        .into_iter()
        .map(map_artist_mv)
        .collect::<Result<Vec<_>>>()?;
    let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let next_offset = offset.saturating_add(consumed);
    let has_more = response.has_more.unwrap_or(consumed == limit);
    let mut extensions = Extensions::new();
    insert_extension(&mut extensions, "time", response.time);
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total: None,
            next_offset: (has_more && consumed > 0).then_some(next_offset),
            has_more,
            extensions,
        },
    })
}

fn map_artist_mv(raw: Value) -> Result<Video> {
    let item: ArtistMvItem = parse_body(raw.clone())?;
    let resource_ref =
        ResourceRef::new(Platform::Netease, item.id.to_string()).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid MV id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let artist_name = item.artist_name.clone();
    let mut creators = item
        .artists
        .into_iter()
        .map(map_artist_mv_creator)
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if creators.is_empty()
        && let Some(creator) = item.artist
        && let Some(creator) = map_artist_mv_creator(creator)?
    {
        creators.push(creator);
    }
    if creators.is_empty()
        && let Some((id, name)) = item.artist_id.zip(artist_name)
        && !name.trim().is_empty()
    {
        creators.push(CreatorSummary {
            resource_ref: Some(ResourceRef::new(Platform::Netease, id.to_string()).map_err(
                |error| {
                    TuneWeaveError::new(
                        ErrorCode::UpstreamError,
                        format!("NetEase returned an invalid MV artist id: {error}"),
                    )
                    .with_platform(Platform::Netease)
                },
            )?),
            name,
            avatar_url: None,
        });
    }
    let mut extensions = Extensions::new();
    extensions.insert("mv".to_owned(), raw);
    Ok(Video {
        resource_ref,
        platform: Platform::Netease,
        id: item.id.to_string(),
        title: item.name,
        creators,
        description: String::new(),
        cover_url: normalized_string(item.image_16x9_url)
            .or_else(|| normalized_string(item.imgurl)),
        duration_ms: item.duration,
        published_at: item.published_at,
        play_count: item.play_count,
        subscribed: item.subed,
        extensions,
    })
}

fn map_artist_mv_creator(creator: crate::dto::ArtistMvCreator) -> Result<Option<CreatorSummary>> {
    if creator.name.trim().is_empty() {
        return Ok(None);
    }
    let creator_ref =
        ResourceRef::new(Platform::Netease, creator.id.to_string()).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid MV artist id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    Ok(Some(CreatorSummary {
        resource_ref: Some(creator_ref),
        name: creator.name,
        avatar_url: normalized_string(creator.avatar_url),
    }))
}

fn map_artist_videos_response(
    response: ArtistVideosEnvelope,
    limit: u32,
    offset: u32,
) -> Result<Page<Video>> {
    let page = response.data.page;
    let items = response
        .data
        .records
        .into_iter()
        .map(map_artist_video)
        .collect::<Result<Vec<_>>>()?;
    let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let next_offset = offset.saturating_add(consumed);
    let has_more = page.more.unwrap_or(consumed == limit);
    let mut extensions = Extensions::new();
    insert_extension(
        &mut extensions,
        "next_cursor",
        page.cursor.as_ref().and_then(json_scalar_string),
    );
    insert_extension(&mut extensions, "page_size", page.size);
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total: None,
            next_offset: (has_more && consumed > 0).then_some(next_offset),
            has_more,
            extensions,
        },
    })
}

fn map_artist_video(raw: Value) -> Result<Video> {
    let item: ArtistVideoRecord = parse_body(raw.clone())?;
    let id = item
        .resource
        .base
        .id
        .as_ref()
        .and_then(usable_resource_id)
        .or_else(|| item.id.as_ref().and_then(usable_resource_id))
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase artist video item is missing a usable id",
            )
            .with_platform(Platform::Netease)
        })?;
    let resource_ref = ResourceRef::new(Platform::Netease, id.clone()).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid artist video id: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    let base = item.resource.base;
    let extension = item.resource.extension;
    let mut creators = extension
        .as_ref()
        .map(|extension| {
            extension
                .artists
                .iter()
                .filter_map(map_artist_video_creator)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if creators.is_empty()
        && let Some(user) = item.resource.user_profile.as_ref()
        && !user.nickname.trim().is_empty()
    {
        creators.push(CreatorSummary {
            resource_ref: user
                .user_id
                .as_ref()
                .and_then(json_scalar_string)
                .and_then(|id| ResourceRef::new(Platform::Netease, id).ok()),
            name: user.nickname.clone(),
            avatar_url: user.avatar_url.clone(),
        });
    }
    if creators.is_empty()
        && let Some(name) = extension
            .as_ref()
            .and_then(|extension| extension.artist_name.as_deref())
            .map(str::trim)
            .filter(|name| !name.is_empty())
    {
        creators.push(CreatorSummary {
            resource_ref: None,
            name: name.to_owned(),
            avatar_url: None,
        });
    }
    let mut extensions = Extensions::new();
    extensions.insert("artist_video".to_owned(), raw);
    Ok(Video {
        resource_ref,
        platform: Platform::Netease,
        id,
        title: normalized_string(base.text)
            .or_else(|| normalized_string(base.original_title))
            .unwrap_or_default(),
        creators,
        description: base.desc.unwrap_or_default(),
        cover_url: base.cover_url,
        duration_ms: base.duration,
        published_at: base
            .published_at_ms
            .filter(|milliseconds| *milliseconds > 0)
            .and_then(|milliseconds| unix_rfc3339(milliseconds / 1_000)),
        play_count: extension.and_then(|extension| extension.play_count),
        subscribed: None,
        extensions,
    })
}

fn map_mv_detail(response: MvDetailEnvelope, raw_response: Value) -> Result<VideoDetail> {
    let data = response.data;
    let id = data.id.to_string();
    let resource_ref = video_resource_ref(&id, "MV")?;
    let fallback_creator = data.artist_id.zip(data.artist_name.clone());
    let creators = map_video_creators(data.artists, fallback_creator);
    let resolutions = data
        .brs
        .into_iter()
        .filter_map(map_video_resolution)
        .collect();
    let detail_raw = raw_response.get("data").cloned().unwrap_or(Value::Null);
    let mut video_extensions = Extensions::new();
    video_extensions.insert("detail".to_owned(), detail_raw);
    let video = Video {
        resource_ref,
        platform: Platform::Netease,
        id,
        title: data.name,
        creators,
        description: normalized_string(data.desc)
            .or_else(|| normalized_string(data.brief_description))
            .unwrap_or_default(),
        cover_url: normalized_string(data.cover),
        duration_ms: data.duration,
        published_at: normalized_string(data.published_at),
        play_count: data.play_count,
        subscribed: response.subed,
        extensions: video_extensions,
    };
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw_response);
    Ok(VideoDetail {
        kind: VideoResourceKind::Mv,
        video,
        resolutions,
        extensions,
    })
}

fn map_cloud_video_detail(
    response: CloudVideoDetailEnvelope,
    raw_response: Value,
) -> Result<VideoDetail> {
    let data = response.data;
    let id = json_scalar_string(&data.vid).ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase video detail is missing a usable video id",
        )
        .with_platform(Platform::Netease)
    })?;
    let resource_ref = video_resource_ref(&id, "video")?;
    let creators = data
        .creator
        .into_iter()
        .filter_map(map_video_creator)
        .collect();
    let resolutions = data
        .resolutions
        .into_iter()
        .filter_map(map_video_resolution)
        .collect();
    let detail_raw = raw_response.get("data").cloned().unwrap_or(Value::Null);
    let mut video_extensions = Extensions::new();
    video_extensions.insert("detail".to_owned(), detail_raw);
    let video = Video {
        resource_ref,
        platform: Platform::Netease,
        id,
        title: data.title,
        creators,
        description: data.description.unwrap_or_default(),
        cover_url: normalized_string(data.cover_url),
        duration_ms: data.duration_ms,
        published_at: data
            .published_at
            .filter(|milliseconds| *milliseconds > 0)
            .and_then(|milliseconds| unix_rfc3339(milliseconds / 1_000)),
        play_count: data.play_count,
        subscribed: data.subed,
        extensions: video_extensions,
    };
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw_response);
    Ok(VideoDetail {
        kind: VideoResourceKind::Video,
        video,
        resolutions,
        extensions,
    })
}

fn map_video_creators(
    creators: Vec<VideoCreatorItem>,
    fallback: Option<(u64, String)>,
) -> Vec<CreatorSummary> {
    let mut creators = creators
        .into_iter()
        .filter_map(map_video_creator)
        .collect::<Vec<_>>();
    if creators.is_empty()
        && let Some((id, name)) = fallback
        && !name.trim().is_empty()
    {
        creators.push(CreatorSummary {
            resource_ref: ResourceRef::new(Platform::Netease, id.to_string()).ok(),
            name,
            avatar_url: None,
        });
    }
    creators
}

fn map_video_creator(creator: VideoCreatorItem) -> Option<CreatorSummary> {
    let name = creator.name.trim();
    if name.is_empty() {
        return None;
    }
    Some(CreatorSummary {
        resource_ref: creator
            .id
            .as_ref()
            .and_then(json_scalar_string)
            .and_then(|id| ResourceRef::new(Platform::Netease, id).ok()),
        name: name.to_owned(),
        avatar_url: normalized_string(creator.avatar_url),
    })
}

fn map_video_resolution(raw: Value) -> Option<VideoResolution> {
    let resolution = ["br", "resolution", "height"]
        .into_iter()
        .find_map(|key| raw.get(key).and_then(scalar_u64).filter(|value| *value > 0))
        .and_then(|value| u32::try_from(value).ok())?;
    let format = radio_text_field(&raw, &["format", "container", "type"])
        .map(|value| value.to_ascii_lowercase());
    let mut extensions = Extensions::new();
    extensions.insert("resolution".to_owned(), raw.clone());
    Some(VideoResolution {
        resolution,
        width: raw
            .get("width")
            .and_then(scalar_u64)
            .and_then(|value| u32::try_from(value).ok()),
        height: raw
            .get("height")
            .and_then(scalar_u64)
            .and_then(|value| u32::try_from(value).ok()),
        size: raw.get("size").and_then(scalar_u64),
        format,
        extensions,
    })
}

fn map_video_stats(
    id: &str,
    kind: VideoResourceKind,
    response: VideoStatsEnvelope,
    raw_response: Value,
) -> Result<VideoStats> {
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw_response);
    Ok(VideoStats {
        video_ref: video_resource_ref(id, "video stats")?,
        kind,
        liked: response.liked,
        like_count: response.liked_count,
        comment_count: response.comment_count,
        share_count: response.share_count,
        extensions,
    })
}

fn map_video_stream(
    id: &str,
    request: &VideoStreamRequest,
    item: VideoUrlItem,
    raw_response: Value,
) -> Result<VideoStream> {
    if let Some(response_id) = item.id.as_ref().and_then(json_scalar_string)
        && response_id != id
    {
        return Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase returned a video stream for a different id",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "requested_id": id, "response_id": response_id })));
    }
    let url = normalized_string(item.url);
    let platform_code = item
        .code
        .or_else(|| raw_response.get("code").and_then(Value::as_i64));
    let available = url.is_some() && platform_code.is_none_or(|code| code == 200);
    let actual_resolution = item
        .r
        .filter(|resolution| *resolution > 0)
        .or_else(|| item.resolution.filter(|resolution| *resolution > 0));
    let format = url.as_deref().and_then(video_url_format);
    let mut extensions = Extensions::new();
    extensions.insert("kind".to_owned(), json!(request.kind));
    insert_extension(&mut extensions, "md5", normalized_string(item.md5));
    extensions.insert("response".to_owned(), raw_response);
    Ok(VideoStream {
        video_ref: video_resource_ref(id, "video stream")?,
        platform: Platform::Netease,
        available,
        url,
        backup_urls: Vec::new(),
        headers: BTreeMap::new(),
        expires_at: item
            .expi
            .filter(|seconds| *seconds > 0)
            .or_else(|| item.validity.filter(|seconds| *seconds > 0))
            .and_then(expiration_rfc3339),
        format,
        codec: None,
        width: None,
        height: actual_resolution,
        size: item.size,
        duration_ms: None,
        requested_resolution: request.resolution,
        actual_resolution,
        platform_code,
        fee: item.fee.or(item.mv_fee),
        message: normalized_string(item.msg),
        extensions,
    })
}

fn video_resource_ref(id: &str, kind: &str) -> Result<ResourceRef> {
    ResourceRef::new(Platform::Netease, id).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid {kind} id: {error}"),
        )
        .with_platform(Platform::Netease)
    })
}

fn video_url_format(url: &str) -> Option<String> {
    let path = Url::parse(url).ok()?.path().to_ascii_lowercase();
    ["mp4", "m3u8", "flv", "webm"]
        .into_iter()
        .find(|format| path.ends_with(&format!(".{format}")))
        .map(str::to_owned)
}

fn map_artist_new_videos_response(
    response: ArtistNewVideosEnvelope,
    raw_response: Value,
    limit: u32,
    before_ms: u64,
) -> Result<Page<Video>> {
    let next_before_ms = response
        .data
        .new_works
        .last()
        .and_then(artist_update_timestamp);
    let items = response
        .data
        .new_works
        .into_iter()
        .map(map_artist_new_video)
        .collect::<Result<Vec<_>>>()?;
    let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let has_more = response.data.has_more.unwrap_or(consumed == limit);
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw_response);
    extensions.insert("before_ms".to_owned(), json!(before_ms));
    insert_extension(&mut extensions, "next_before_ms", next_before_ms);
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset: 0,
            total: None,
            next_offset: None,
            has_more,
            extensions,
        },
    })
}

fn map_artist_new_tracks_response(
    response: ArtistNewTracksEnvelope,
    raw_response: Value,
    limit: u32,
    before_ms: u64,
) -> Result<Page<Track>> {
    let work_block_count = response.data.new_works.len();
    let next_before_ms = response
        .data
        .new_works
        .last()
        .and_then(artist_update_timestamp);
    let items = response
        .data
        .new_works
        .into_iter()
        .map(map_artist_new_track_block)
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    let consumed_blocks = u32::try_from(work_block_count).unwrap_or(u32::MAX);
    let has_more = response.data.has_more.unwrap_or(consumed_blocks == limit);
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw_response);
    extensions.insert("before_ms".to_owned(), json!(before_ms));
    extensions.insert("limit_unit".to_owned(), json!("work_blocks"));
    extensions.insert("work_block_count".to_owned(), json!(work_block_count));
    insert_extension(&mut extensions, "next_before_ms", next_before_ms);
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset: 0,
            total: response.data.new_song_count,
            next_offset: None,
            has_more,
            extensions,
        },
    })
}

fn map_artist_new_track_block(raw: Value) -> Result<Vec<Track>> {
    if raw.get("id").is_some() {
        let song: Song = parse_body(raw.clone())?;
        let mut track = map_song(song, None)?;
        track.extensions.insert("artist_new_track".to_owned(), raw);
        return Ok(vec![track]);
    }

    let songs =
        artist_work_resources(&raw, &["songLists", "songList", "songs"]).ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase followed artist track block is missing its song list",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "block": raw }))
        })?;
    songs
        .iter()
        .cloned()
        .map(|song_raw| {
            let song: Song = parse_body(song_raw.clone())?;
            let mut track = map_song(song, None)?;
            track
                .extensions
                .insert("artist_new_track".to_owned(), song_raw);
            track
                .extensions
                .insert("artist_new_track_block".to_owned(), raw.clone());
            Ok(track)
        })
        .collect()
}

fn map_artist_new_works_response(
    response: ArtistNewWorksEnvelope,
    raw_response: Value,
    request: &ArtistWorksRequest,
    limit: u32,
    before_ms: u64,
) -> Result<Page<ArtistWorkUpdate>> {
    let next_before_ms = response
        .data
        .new_works
        .last()
        .and_then(artist_update_timestamp);
    let items = response
        .data
        .new_works
        .into_iter()
        .map(|raw| map_artist_work_update(raw, request.source_type))
        .collect::<Result<Vec<_>>>()?;
    let consumed = u32::try_from(items.len()).unwrap_or(u32::MAX);
    let has_more = response.data.has_more.unwrap_or(consumed == limit);
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw_response);
    extensions.insert("before_ms".to_owned(), json!(before_ms));
    extensions.insert("source_type".to_owned(), json!(request.source_type));
    extensions.insert("first_request".to_owned(), json!(request.first_request));
    insert_extension(&mut extensions, "next_before_ms", next_before_ms);
    insert_extension(
        &mut extensions,
        "latest_visit_time",
        response.data.latest_visit_time,
    );
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset: 0,
            total: None,
            next_offset: None,
            has_more,
            extensions,
        },
    })
}

fn map_artist_new_tracks_play_all_response(
    response: ArtistNewTracksPlayAllEnvelope,
    raw_response: Value,
) -> Result<Page<Track>> {
    let items = response
        .data
        .songs
        .into_iter()
        .map(|raw| {
            let song: Song = parse_body(raw.clone())?;
            let mut track = map_song(song, None)?;
            track
                .extensions
                .insert("artist_new_track_play_all".to_owned(), raw);
            Ok(track)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw_response);
    Ok(Page {
        items,
        pagination: PageMeta {
            limit: 50,
            offset: 0,
            total: response.data.count,
            next_offset: None,
            has_more: false,
            extensions,
        },
    })
}

fn map_artist_work_update(raw: Value, default_source_type: u32) -> Result<ArtistWorkUpdate> {
    let source_type = raw["sourceType"]
        .as_u64()
        .and_then(|value| u32::try_from(value).ok())
        .unwrap_or(default_source_type);
    let info = raw.get("info").unwrap_or(&Value::Null);
    let block_title = info.get("blockTitle").unwrap_or(&Value::Null);
    let tracks = artist_work_resources(info, &["songLists", "songList", "songs"])
        .map(|songs| {
            songs
                .iter()
                .cloned()
                .map(|raw| {
                    let song: Song = parse_body(raw.clone())?;
                    let mut track = map_song(song, None)?;
                    track.extensions.insert("artist_work_track".to_owned(), raw);
                    Ok(track)
                })
                .collect::<Result<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_default();
    let videos = artist_work_resources(
        info,
        &[
            "mvLists",
            "mvList",
            "mvs",
            "videoLists",
            "videoList",
            "videos",
        ],
    )
    .map(|videos| {
        videos
            .iter()
            .cloned()
            .map(map_artist_new_video)
            .collect::<Result<Vec<_>>>()
    })
    .transpose()?
    .unwrap_or_default();
    let block_type = info["blockType"]
        .as_str()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let kind = artist_work_kind(!tracks.is_empty(), !videos.is_empty(), &block_type);
    let artist_name = block_title["artistName"]
        .as_str()
        .map(str::trim)
        .filter(|name| !name.is_empty());
    let artist = artist_name.map(|name| ArtistSummary {
        resource_ref: block_title
            .get("artistId")
            .and_then(json_scalar_string)
            .and_then(|id| ResourceRef::new(Platform::Netease, id).ok()),
        name: name.to_owned(),
    });
    let published_at = block_title["publishDate"]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| raw.get("publishTime").and_then(netease_published_at));
    let title = block_title["resourceName"]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let cover_url = radio_text_field(block_title, &["resourcePicUrl", "imgUrl"]);
    let mut extensions = Extensions::new();
    extensions.insert("artist_work".to_owned(), raw);
    Ok(ArtistWorkUpdate {
        source_type,
        kind,
        published_at,
        artist,
        title,
        cover_url,
        tracks,
        videos,
        extensions,
    })
}

fn artist_work_resources<'a>(info: &'a Value, keys: &[&str]) -> Option<&'a Vec<Value>> {
    preferred_array_field(info, keys)
}

fn artist_work_kind(has_tracks: bool, has_videos: bool, block_type: &str) -> ArtistWorkKind {
    match (has_tracks, has_videos) {
        (true, true) => ArtistWorkKind::Mixed,
        (true, false) => ArtistWorkKind::Track,
        (false, true) => ArtistWorkKind::Video,
        (false, false) => {
            let track_hint = block_type.contains("song") || block_type.contains("track");
            let video_hint = block_type.contains("mv") || block_type.contains("video");
            match (track_hint, video_hint) {
                (true, true) => ArtistWorkKind::Mixed,
                (true, false) => ArtistWorkKind::Track,
                (false, true) => ArtistWorkKind::Video,
                (false, false) => ArtistWorkKind::Unknown,
            }
        }
    }
}

fn map_artist_new_video(raw: Value) -> Result<Video> {
    let item: ArtistNewVideoItem = parse_body(raw.clone())?;
    let id = item
        .mv_id
        .as_ref()
        .and_then(usable_resource_id)
        .or_else(|| item.id.as_ref().and_then(usable_resource_id))
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase followed artist video item is missing a usable id",
            )
            .with_platform(Platform::Netease)
        })?;
    let resource_ref = ResourceRef::new(Platform::Netease, id.clone()).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid followed artist video id: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    let mut creators = item
        .artists
        .iter()
        .filter_map(map_artist_video_creator)
        .collect::<Vec<_>>();
    if creators.is_empty()
        && let Some(name) = item
            .artist_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
    {
        creators.push(CreatorSummary {
            resource_ref: item
                .artist_id
                .as_ref()
                .and_then(json_scalar_string)
                .and_then(|id| ResourceRef::new(Platform::Netease, id).ok()),
            name: name.to_owned(),
            avatar_url: item.artist_image_url.clone(),
        });
    }
    let published_at = normalized_string(item.published_date)
        .or_else(|| item.published_at.as_ref().and_then(netease_published_at));
    let mut extensions = Extensions::new();
    extensions.insert("artist_new_video".to_owned(), raw);
    Ok(Video {
        resource_ref,
        platform: Platform::Netease,
        id,
        title: normalized_string(item.mv_name)
            .or_else(|| normalized_string(item.name))
            .unwrap_or_default(),
        creators,
        description: normalized_string(item.desc)
            .or_else(|| normalized_string(item.brief_description))
            .unwrap_or_default(),
        cover_url: normalized_string(item.mv_cover_url).or_else(|| normalized_string(item.cover)),
        duration_ms: item.duration,
        published_at,
        play_count: item.play_count,
        subscribed: None,
        extensions,
    })
}

fn artist_update_timestamp(raw: &Value) -> Option<u64> {
    raw.get("publishTime").and_then(json_timestamp_millis)
}

fn json_timestamp_millis(value: &Value) -> Option<u64> {
    let timestamp = value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))?;
    if timestamp < 100_000_000_000 {
        timestamp.checked_mul(1_000)
    } else {
        Some(timestamp)
    }
}

fn netease_published_at(value: &Value) -> Option<String> {
    if let Some(milliseconds) = json_timestamp_millis(value) {
        return unix_rfc3339(milliseconds / 1_000);
    }
    value
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn map_artist_video_creator(creator: &ArtistVideoCreator) -> Option<CreatorSummary> {
    let name = creator.name.trim();
    if name.is_empty() {
        return None;
    }
    Some(CreatorSummary {
        resource_ref: creator
            .id
            .as_ref()
            .and_then(json_scalar_string)
            .and_then(|id| ResourceRef::new(Platform::Netease, id).ok()),
        name: name.to_owned(),
        avatar_url: creator.avatar_url.clone(),
    })
}

fn json_scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => {
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_owned())
        }
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn usable_resource_id(value: &Value) -> Option<String> {
    json_scalar_string(value)
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty() && id != "0")
}

fn usable_owned_resource_id(value: Option<String>) -> Option<String> {
    value
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty() && id != "0")
}

fn map_artist(
    detail: ArtistDetailEnvelope,
    description: ArtistDescriptionEnvelope,
    detail_raw: Value,
    description_raw: Value,
) -> Result<Artist> {
    let artist = detail.data.artist;
    let resource_ref =
        ResourceRef::new(Platform::Netease, artist.id.to_string()).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid artist id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let mut aliases = Vec::new();
    for alias in artist.alias.into_iter().chain(artist.translated_names) {
        let alias = alias.trim();
        if !alias.is_empty() && alias != artist.name && !aliases.iter().any(|item| item == alias) {
            aliases.push(alias.to_owned());
        }
    }
    let description_text = description
        .brief_description
        .as_deref()
        .map(str::trim)
        .filter(|description| !description.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            artist
                .brief_description
                .as_deref()
                .map(str::trim)
                .filter(|description| !description.is_empty())
                .map(str::to_owned)
        })
        .unwrap_or_default();
    let biography_sections = description
        .introduction
        .into_iter()
        .filter_map(|section| {
            let title = section.title.trim().to_owned();
            let text = section.text.trim().to_owned();
            (!title.is_empty() || !text.is_empty())
                .then_some(ArtistBiographySection { title, text })
        })
        .collect();
    let mut extensions = Extensions::new();
    extensions.insert("detail_response".to_owned(), detail_raw);
    extensions.insert("description_response".to_owned(), description_raw);
    Ok(Artist {
        resource_ref,
        platform: Platform::Netease,
        id: artist.id.to_string(),
        name: artist.name,
        aliases,
        description: description_text,
        biography_sections,
        avatar_url: artist.avatar,
        cover_url: artist.cover,
        album_count: artist.album_count,
        track_count: artist.track_count,
        mv_count: artist.mv_count,
        video_count: detail.data.video_count,
        identities: artist.identities,
        extensions,
    })
}

fn map_artist_overview(
    response: ArtistOverviewEnvelope,
    raw_response: Value,
) -> Result<ArtistOverview> {
    let mut artist = map_artist_list_item(response.artist.clone())?;
    artist.extensions.remove("catalog_item");
    artist
        .extensions
        .insert("overview_artist".to_owned(), response.artist);
    let featured_tracks = response
        .hot_songs
        .into_iter()
        .map(|raw| {
            let song: Song = parse_body(raw.clone())?;
            let mut track = map_song(song, None)?;
            track.extensions.insert("overview_track".to_owned(), raw);
            Ok(track)
        })
        .collect::<Result<Vec<_>>>()?;
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw_response);
    Ok(ArtistOverview {
        artist,
        featured_tracks,
        has_more_tracks: response.more.unwrap_or(false),
        extensions,
    })
}

fn map_artist_stats(
    id: u64,
    stats: ArtistDynamicEnvelope,
    raw: Value,
    follow_count: ArtistFollowCountEnvelope,
    follow_count_raw: Value,
) -> Result<ArtistStats> {
    let artist_ref = ResourceRef::new(Platform::Netease, id.to_string()).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid artist id: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    let video_counts = stats
        .video_counts
        .into_iter()
        .map(|count| ArtistContentCount {
            category: Some(count.cat.to_string()),
            count: count.num,
            extensions: Extensions::new(),
        })
        .collect();
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw);
    extensions.insert("follow_count_response".to_owned(), follow_count_raw);
    Ok(ArtistStats {
        artist_ref,
        followed: follow_count
            .data
            .is_following
            .or(follow_count.data.follow)
            .or(stats.followed),
        follower_count: follow_count.data.follower_count,
        video_counts,
        online_concert_count: stats.concert.and_then(|concert| concert.online_count),
        extensions,
    })
}

fn map_artist_album_item(raw: Value) -> Result<Album> {
    let album: AlbumDetail = parse_body(raw.clone())?;
    let mut album = map_album(album)?;
    album.extensions.insert("artist_album_item".to_owned(), raw);
    Ok(album)
}

fn map_album_stats(id: u64, stats: AlbumStatsEnvelope) -> Result<AlbumStats> {
    let album_ref = ResourceRef::new(Platform::Netease, id.to_string()).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid album id: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    let mut extensions = Extensions::new();
    insert_extension(&mut extensions, "subscribed_at_ms", stats.subscribed_at);
    insert_extension(&mut extensions, "album_game_info", stats.album_game_info);
    Ok(AlbumStats {
        album_ref,
        subscribed: stats.subscribed,
        subscriber_count: stats.subscriber_count,
        comment_count: stats.comment_count,
        share_count: stats.share_count,
        like_count: stats.like_count,
        on_sale: stats.on_sale,
        subscribed_at: stats
            .subscribed_at
            .filter(|milliseconds| *milliseconds > 0)
            .and_then(|milliseconds| unix_rfc3339(milliseconds / 1_000)),
        extensions,
    })
}

fn map_track_entitlement(raw: Value) -> Result<TrackEntitlement> {
    let entitlement: TrackEntitlementData = parse_body(raw.clone())?;
    let track_ref =
        ResourceRef::new(Platform::Netease, entitlement.id.to_string()).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid track id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let mut available_qualities = Vec::new();
    for quality in entitlement
        .charge_info
        .iter()
        .filter_map(|charge| charge.rate)
        .map(quality_for_bitrate)
    {
        if !available_qualities.contains(&quality) {
            available_qualities.push(quality);
        }
    }
    available_qualities.sort_by_key(|quality| quality_rank(*quality));
    let mut extensions = Extensions::new();
    extensions.insert("privilege".to_owned(), raw);
    Ok(TrackEntitlement {
        track_ref,
        playable: entitlement
            .st
            .map(|status| status >= 0 && entitlement.pl.unwrap_or(0) > 0),
        downloadable: entitlement
            .st
            .map(|status| status >= 0 && entitlement.dl.unwrap_or(0) > 0),
        play_bitrate: entitlement.pl,
        download_bitrate: entitlement.dl,
        max_play_bitrate: entitlement
            .play_max_bitrate
            .filter(|bitrate| *bitrate > 0)
            .or_else(|| entitlement.maxbr.filter(|bitrate| *bitrate > 0)),
        max_download_bitrate: entitlement.download_max_bitrate,
        play_quality: optional_quality(entitlement.play_level.as_deref(), entitlement.pl),
        download_quality: optional_quality(entitlement.download_level.as_deref(), entitlement.dl),
        available_qualities,
        fee: entitlement.fee,
        paid: entitlement.payed.map(|paid| paid > 0),
        extensions,
    })
}

fn map_digital_album(
    response: DigitalAlbumEnvelope,
    raw: &Value,
    requested_id: u64,
) -> Result<DigitalAlbum> {
    let album = response.album.ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::ResourceNotFound,
            "NetEase digital album was not found",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "id": requested_id }))
    })?;
    let resource_ref =
        ResourceRef::new(Platform::Netease, album.album_id.to_string()).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid digital album id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let artist_name =
        normalized_string(album.artist_names).or_else(|| normalized_string(album.artist_name));
    let artists = match (album.artist_id, artist_name) {
        (id, Some(name)) if !name.trim().is_empty() => vec![ArtistSummary {
            resource_ref: id
                .map(|id| ResourceRef::new(Platform::Netease, id.to_string()))
                .transpose()
                .map_err(|error| {
                    TuneWeaveError::new(
                        ErrorCode::UpstreamError,
                        format!("NetEase returned an invalid digital album artist id: {error}"),
                    )
                    .with_platform(Platform::Netease)
                })?,
            name,
        }],
        _ => Vec::new(),
    };
    let product = response.product;
    let description = product
        .as_ref()
        .map(|product| {
            product
                .descr
                .iter()
                .map(|item| item.resource.trim())
                .filter(|resource| !resource.is_empty() && *resource != "</br>")
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();
    let mut extensions = Extensions::new();
    insert_extension(&mut extensions, "bought_count", response.bought_count);
    for key in [
        "album",
        "product",
        "board",
        "style",
        "singleSongProductId",
        "visitorId",
    ] {
        if let Some(value) = raw.get(key) {
            extensions.insert(key.to_owned(), value.clone());
        }
    }
    Ok(DigitalAlbum {
        resource_ref,
        platform: Platform::Netease,
        id: album.album_id.to_string(),
        name: album.album_name,
        artists,
        description,
        cover_url: album.cover_url,
        published_at: product
            .as_ref()
            .and_then(|product| product.publish_time)
            .and_then(|milliseconds| unix_rfc3339(milliseconds / 1_000)),
        price: product
            .as_ref()
            .and_then(|product| product.price)
            .map(|amount| Money {
                amount,
                currency: "CNY".to_owned(),
            }),
        is_free: product.as_ref().and_then(|product| product.is_free),
        purchasable: response.can_buy,
        purchased: response.has_album,
        sale_count: product.as_ref().and_then(|product| product.sale_count),
        track_count: None,
        tags: product.map_or_else(Vec::new, |product| product.tags),
        extensions,
    })
}

fn map_digital_album_list_item(raw: Value) -> Result<DigitalAlbum> {
    let item: DigitalAlbumListItem = parse_body(raw.clone())?;
    let resource_ref =
        ResourceRef::new(Platform::Netease, item.album_id.to_string()).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid digital album id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let artists = item
        .artist_name
        .filter(|name| !name.trim().is_empty())
        .map(|name| {
            vec![ArtistSummary {
                resource_ref: None,
                name,
            }]
        })
        .unwrap_or_default();
    let mut extensions = Extensions::new();
    extensions.insert("product".to_owned(), raw);
    Ok(DigitalAlbum {
        resource_ref,
        platform: Platform::Netease,
        id: item.album_id.to_string(),
        name: item.album_name,
        artists,
        description: String::new(),
        cover_url: item.cover_url,
        published_at: item
            .publish_time
            .and_then(|milliseconds| unix_rfc3339(milliseconds / 1_000)),
        price: item.price.map(|amount| Money {
            amount,
            currency: "CNY".to_owned(),
        }),
        is_free: item.price.map(|price| price == 0.0),
        purchasable: None,
        purchased: None,
        sale_count: item.sale_count,
        track_count: None,
        tags: Vec::new(),
        extensions,
    })
}

fn map_digital_album_chart_entry(raw: Value, position: u32) -> Result<DigitalAlbumChartEntry> {
    let item: DigitalAlbumChartItem = parse_body(raw.clone())?;
    let rank = item.rank.unwrap_or(position).saturating_add(1);
    let mut extensions = Extensions::new();
    insert_extension(&mut extensions, "upstream_rank", item.rank);
    insert_extension(&mut extensions, "album_type", item.album_type);
    Ok(DigitalAlbumChartEntry {
        rank,
        rank_change: item.rank_change,
        product: map_digital_album_list_item(raw)?,
        extensions,
    })
}

fn map_chart_catalog(
    response: ChartCatalogEnvelope,
    view: ChartCatalogView,
    raw_response: Value,
) -> Result<ChartCatalog> {
    let groups = match view {
        ChartCatalogView::Modern => response
            .data
            .into_iter()
            .map(map_chart_group)
            .collect::<Result<Vec<_>>>()?,
        ChartCatalogView::Overview | ChartCatalogView::Summary => {
            let charts = response
                .list
                .into_iter()
                .map(map_chart)
                .collect::<Result<Vec<_>>>()?;
            vec![ChartGroup {
                code: None,
                name: "all".to_owned(),
                display_type: None,
                target_url: None,
                charts,
                extensions: Extensions::new(),
            }]
        }
    };
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw_response);
    Ok(ChartCatalog {
        platform: Platform::Netease,
        view,
        groups,
        extensions,
    })
}

fn map_chart_group(raw: Value) -> Result<ChartGroup> {
    let group: ChartGroupItem = parse_body(raw.clone())?;
    let charts = group
        .list
        .into_iter()
        .map(map_chart)
        .collect::<Result<Vec<_>>>()?;
    let mut extensions = Extensions::new();
    extensions.insert("group".to_owned(), raw);
    Ok(ChartGroup {
        code: normalized_string(group.category_code),
        name: group.name,
        display_type: normalized_string(group.display_type)
            .or_else(|| normalized_string(group.front_display_type)),
        target_url: normalized_string(group.target_url),
        charts,
        extensions,
    })
}

fn map_chart(raw: Value) -> Result<Chart> {
    let item: ChartItem = parse_body(raw.clone())?;
    let id = item.id.and_then(scalar_string);
    let target_kind = normalized_string(item.target_type).map(|value| value.to_ascii_lowercase());
    let has_track_target = target_kind
        .as_deref()
        .is_none_or(|target| target.eq_ignore_ascii_case("playlist"));
    let resource_ref = if has_track_target {
        id.as_deref()
            .filter(|id| *id != "0")
            .map(|id| chart_resource_ref(id, "chart"))
            .transpose()?
    } else {
        None
    };
    let previews = map_chart_previews(item.track_rank_list, item.tracks)?;
    let cover_url = [
        item.cover_url,
        item.cover_img_url,
        item.new_first_cover_url,
        item.first_cover_url,
    ]
    .into_iter()
    .find_map(normalized_string);
    let target_url =
        normalized_string(item.target_url).or_else(|| normalized_string(item.front_target_url));
    let mut extensions = Extensions::new();
    extensions.insert("chart".to_owned(), raw);
    Ok(Chart {
        resource_ref,
        platform: Platform::Netease,
        id,
        name: item.name,
        description: item.description.unwrap_or_default(),
        cover_url,
        update_frequency: normalized_string(item.update_frequency),
        updated_at_ms: item.update_time,
        track_count: item.track_count,
        play_count: item.play_count,
        subscribed: item.subscribed,
        playable: item.can_play,
        target_kind,
        target_url,
        previews,
        extensions,
    })
}

fn map_chart_previews(
    ranked: Option<Vec<Value>>,
    textual: Option<Vec<Value>>,
) -> Result<Vec<ChartTrackPreview>> {
    let ranked = ranked.unwrap_or_default();
    if !ranked.is_empty() {
        return ranked.into_iter().map(map_chart_rank_preview).collect();
    }
    textual
        .unwrap_or_default()
        .into_iter()
        .enumerate()
        .map(|(index, raw)| map_chart_text_preview(raw, index))
        .collect()
}

fn map_chart_rank_preview(raw: Value) -> Result<ChartTrackPreview> {
    let item: ChartRankPreviewItem = parse_body(raw.clone())?;
    let rank = item.rank;
    let previous_rank = item.last_rank.filter(|rank| *rank > 0);
    let rank_change = rank
        .zip(previous_rank)
        .map(|(rank, previous)| i64::from(previous) - i64::from(rank));
    let track_ref = item
        .track_id
        .and_then(scalar_string)
        .filter(|id| id != "0")
        .map(|id| chart_resource_ref(&id, "chart preview track"))
        .transpose()?;
    let mut extensions = Extensions::new();
    extensions.insert("preview".to_owned(), raw);
    Ok(ChartTrackPreview {
        rank,
        previous_rank,
        rank_change,
        track_ref,
        name: normalized_string(item.song_name)
            .or_else(|| normalized_string(item.item_name))
            .unwrap_or_default(),
        byline: normalized_string(item.artist_name),
        cover_url: normalized_string(item.cover_url),
        extensions,
    })
}

fn map_chart_text_preview(raw: Value, index: usize) -> Result<ChartTrackPreview> {
    let item: ChartTextPreviewItem = parse_body(raw.clone())?;
    let mut extensions = Extensions::new();
    extensions.insert("preview".to_owned(), raw);
    Ok(ChartTrackPreview {
        rank: Some(u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1)),
        previous_rank: None,
        rank_change: None,
        track_ref: None,
        name: normalized_string(item.first).unwrap_or_default(),
        byline: normalized_string(item.second),
        cover_url: None,
        extensions,
    })
}

fn map_artist_chart(
    response: ArtistChartEnvelope,
    area: ArtistChartArea,
    raw_response: Value,
) -> Result<ArtistChart> {
    let updated_at_ms = response.list.update_time;
    let entries = response
        .list
        .artists
        .into_iter()
        .enumerate()
        .map(|(index, raw)| map_artist_chart_entry(raw, index))
        .collect::<Result<Vec<_>>>()?;
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw_response);
    Ok(ArtistChart {
        platform: Platform::Netease,
        area,
        updated_at_ms,
        entries,
        extensions,
    })
}

fn map_artist_chart_entry(raw: Value, index: usize) -> Result<ArtistChartEntry> {
    let rank = u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1);
    let previous_rank = raw
        .get("lastRank")
        .and_then(Value::as_u64)
        .and_then(|rank| u32::try_from(rank).ok())
        .filter(|rank| *rank > 0);
    let rank_change = previous_rank.map(|previous| i64::from(previous) - i64::from(rank));
    let score = raw.get("score").and_then(|value| {
        value
            .as_u64()
            .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
    });
    let mut artist = map_artist_list_item(raw.clone())?;
    artist.extensions.remove("catalog_item");
    artist
        .extensions
        .insert("artist_chart_item".to_owned(), raw.clone());
    let mut extensions = Extensions::new();
    extensions.insert("entry".to_owned(), raw);
    Ok(ArtistChartEntry {
        rank,
        previous_rank,
        rank_change,
        score,
        artist,
        extensions,
    })
}

fn chart_resource_ref(id: &str, kind: &str) -> Result<ResourceRef> {
    ResourceRef::new(Platform::Netease, id).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid {kind} id: {error}"),
        )
        .with_platform(Platform::Netease)
    })
}

fn normalized_string(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn map_dimension_chart(
    response: DimensionChartDetailEnvelope,
    request: &DimensionChartRequest,
    raw_response: Value,
) -> Result<DimensionChart> {
    let (requested_code, target_id, target_type) = validated_dimension_chart_parts(request)?;
    let data = response.data;
    let chart_code = data
        .chart_code
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| requested_code.to_owned());
    let (id, resource_ref) =
        dimension_chart_reference(data.chart_id, &chart_code, target_id, target_type)?;
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw_response);
    Ok(DimensionChart {
        resource_ref,
        platform: Platform::Netease,
        id,
        chart_code,
        target_id: target_id.to_owned(),
        target_type: target_type.to_owned(),
        name: data.name.unwrap_or_default(),
        description: data.description.unwrap_or_default(),
        cover_url: data.cover_url,
        updated_at_ms: data.update_time,
        play_count: data.play_count,
        share_count: data.share_count,
        comment_count: data.comment_count,
        supports_comments: data.support_comment,
        extensions,
    })
}

fn map_dimension_chart_tracks(
    response: DimensionChartTracksEnvelope,
    request: &DimensionChartRequest,
    raw_response: Value,
) -> Result<DimensionChartTrackSnapshot> {
    let (requested_code, target_id, target_type) = validated_dimension_chart_parts(request)?;
    let data = response.data;
    let chart_code = data
        .chart_code
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| requested_code.to_owned());
    let (_, chart_ref) =
        dimension_chart_reference(data.chart_id, &chart_code, target_id, target_type)?;
    let entries = data
        .charts
        .into_iter()
        .enumerate()
        .map(|(index, raw)| map_dimension_chart_track_entry(raw, index))
        .collect::<Result<Vec<_>>>()?;
    let groups = data
        .group_name_map
        .into_iter()
        .filter_map(|(key, value)| scalar_string(value).map(|value| (key, value)))
        .collect();
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw_response);
    Ok(DimensionChartTrackSnapshot {
        chart_ref,
        chart_code,
        target_id: target_id.to_owned(),
        target_type: target_type.to_owned(),
        entries,
        period_label: data.period_update_time_text.and_then(scalar_string),
        groups,
        extensions,
    })
}

fn map_dimension_chart_track_entry(raw: Value, index: usize) -> Result<DimensionChartTrackEntry> {
    let item: DimensionChartTrackItem = parse_body(raw.clone())?;
    let rank = u32::try_from(index).unwrap_or(u32::MAX).saturating_add(1);
    let previous_rank = item
        .last_rank
        .and_then(|rank| u32::try_from(rank).ok())
        .filter(|rank| *rank > 0);
    let rank_change = previous_rank.map(|previous| i64::from(previous) - i64::from(rank));
    let track = map_song(item.song_data, item.privilege)?;
    let mut extensions = Extensions::new();
    extensions.insert("entry".to_owned(), raw);
    Ok(DimensionChartTrackEntry {
        rank,
        previous_rank,
        rank_change,
        track,
        reason: item.reason.filter(|reason| !reason.trim().is_empty()),
        reason_id: item.reason_id.and_then(scalar_string),
        score: item.score.as_ref().and_then(scalar_f64),
        ratio: item.ratio.as_ref().and_then(scalar_f64),
        collected: item.collect,
        extensions,
    })
}

fn dimension_chart_reference(
    chart_id: Option<String>,
    chart_code: &str,
    target_id: &str,
    target_type: &str,
) -> Result<(String, ResourceRef)> {
    let id = chart_id
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| format!("{chart_code}#{target_id}@{target_type}#"));
    let resource_ref = ResourceRef::new(Platform::Netease, id.clone()).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid dimension chart id: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    Ok((id, resource_ref))
}

fn scalar_string(value: Value) -> Option<String> {
    match value {
        Value::String(value) => (!value.trim().is_empty()).then_some(value),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn scalar_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn scalar_u64(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_str()
            .and_then(|value| value.parse().ok())
            .or_else(|| {
                let value = value.as_f64()?;
                (value.is_finite() && value >= 0.0 && value.fract() == 0.0).then_some(value as u64)
            })
    })
}

fn map_album_subscription_result(
    id: u64,
    subscribed: bool,
    response: Value,
) -> Result<SubscriptionResult> {
    let resource_ref = ResourceRef::new(Platform::Netease, id.to_string()).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid album id: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), response);
    Ok(SubscriptionResult {
        resource_ref,
        subscribed,
        extensions,
    })
}

fn map_artist_subscription_result(
    id: u64,
    subscribed: bool,
    response: Value,
) -> Result<SubscriptionResult> {
    let resource_ref = ResourceRef::new(Platform::Netease, id.to_string()).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid artist id: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), response);
    Ok(SubscriptionResult {
        resource_ref,
        subscribed,
        extensions,
    })
}

fn map_radio_station_subscription_result(
    id: u64,
    subscribed: bool,
    response: Value,
) -> Result<SubscriptionResult> {
    let resource_ref = ResourceRef::new(Platform::Netease, id.to_string()).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid broadcast station id: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), response);
    Ok(SubscriptionResult {
        resource_ref,
        subscribed,
        extensions,
    })
}

fn insert_extension<T: serde::Serialize>(
    extensions: &mut Extensions,
    name: &str,
    value: Option<T>,
) {
    if let Some(value) = value.and_then(|value| serde_json::to_value(value).ok()) {
        extensions.insert(name.to_owned(), value);
    }
}

fn map_song(song: Song, outer_privilege: Option<Privilege>) -> Result<Track> {
    let resource_ref =
        ResourceRef::new(Platform::Netease, song.id.to_string()).map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid track id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let available_qualities = map_qualities(&song);
    let artists = song
        .ar
        .into_iter()
        .filter(|artist| !artist.name.trim().is_empty())
        .map(
            |artist| -> std::result::Result<ArtistSummary, ParseResourceRefError> {
                Ok(ArtistSummary {
                    resource_ref: Some(ResourceRef::new(Platform::Netease, artist.id.to_string())?),
                    name: artist.name,
                })
            },
        )
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid artist id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let album = song
        .al
        .map(
            |album| -> std::result::Result<AlbumSummary, ParseResourceRefError> {
                Ok(AlbumSummary {
                    resource_ref: (album.id > 0)
                        .then(|| ResourceRef::new(Platform::Netease, album.id.to_string()))
                        .transpose()?,
                    name: album.name,
                    cover_url: album.pic_url,
                })
            },
        )
        .transpose()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid album id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let mv_ref = song
        .mv
        .filter(|id| *id > 0)
        .map(|id| ResourceRef::new(Platform::Netease, id.to_string()))
        .transpose()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid MV id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    let privilege = outer_privilege.or(song.privilege);
    let playable = privilege
        .as_ref()
        .map(|privilege| privilege.st >= 0 && privilege.pl > 0)
        .or_else(|| song.st.map(|status| status >= 0));
    let mut extensions = Extensions::new();
    if let Some(fee) = song.fee {
        extensions.insert("fee".to_owned(), json!(fee));
    }
    if let Some(mark) = song.mark {
        extensions.insert("mark".to_owned(), json!(mark));
    }
    if let Some(privilege) = privilege {
        extensions.insert(
            "privilege".to_owned(),
            json!({
                "fee": privilege.fee,
                "max_bitrate": privilege.maxbr,
                "play_bitrate": privilege.pl,
                "status": privilege.st
            }),
        );
    }

    Ok(Track {
        resource_ref,
        platform: Platform::Netease,
        id: song.id.to_string(),
        name: song.name,
        aliases: song.alia,
        artists,
        album,
        duration_ms: song.dt,
        isrc: None,
        mv_ref,
        playable,
        available_qualities,
        extensions,
    })
}

fn map_qualities(song: &Song) -> Vec<Quality> {
    let mut qualities = Vec::new();
    if has_audio(&song.l) || has_audio(&song.m) || has_audio(&song.h) {
        qualities.push(Quality::Standard);
    }
    if has_audio(&song.m) || has_audio(&song.h) {
        qualities.push(Quality::Higher);
    }
    if has_audio(&song.h) {
        qualities.push(Quality::High);
    }
    if has_audio(&song.sq) {
        qualities.push(Quality::Lossless);
    }
    if has_audio(&song.hr) {
        qualities.push(Quality::Hires);
    }
    qualities
}

fn has_audio(quality: &Option<AudioQuality>) -> bool {
    quality
        .as_ref()
        .is_some_and(|quality| quality.br.unwrap_or(1) > 0)
}

fn parse_numeric_id(resource: &str, id: &str) -> Result<u64> {
    id.parse().map_err(|_| {
        TuneWeaveError::invalid_request(format!(
            "NetEase {resource} id must be an unsigned integer"
        ))
        .with_platform(Platform::Netease)
        .with_details(json!({ "resource": resource, "id": id }))
    })
}

fn validated_video_id(id: &str, kind: VideoResourceKind) -> Result<String> {
    let id = id.trim();
    if id.is_empty() || id.len() > 256 || id.chars().any(char::is_control) {
        return Err(TuneWeaveError::invalid_request(
            "NetEase video id must be between 1 and 256 visible characters",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "id": id, "kind": kind })));
    }
    match kind {
        VideoResourceKind::Mv => parse_numeric_id("MV", id).map(|id| id.to_string()),
        VideoResourceKind::Video => Ok(id.to_owned()),
    }
}

fn validated_video_resolution(resolution: u32) -> Result<u32> {
    if (1..=4_320).contains(&resolution) {
        return Ok(resolution);
    }
    Err(
        TuneWeaveError::invalid_request("video resolution must be between 1 and 4320")
            .with_platform(Platform::Netease)
            .with_details(json!({ "resolution": resolution })),
    )
}

const fn netease_cloud_search_type(kind: SearchKind) -> u32 {
    match kind {
        SearchKind::Track => 1,
        SearchKind::Album => 10,
        SearchKind::Artist => 100,
        SearchKind::Playlist => 1_000,
        SearchKind::User => 1_002,
        SearchKind::Mv => 1_004,
        SearchKind::Lyric => 1_006,
        SearchKind::RadioStation => 1_009,
        SearchKind::Podcast => 1_009,
        SearchKind::Video => 1_014,
        SearchKind::Mixed => 1_018,
        SearchKind::Voice => 2_000,
    }
}

fn netease_default_search_keyword_request() -> (&'static str, Value) {
    ("/api/search/defaultkeyword/get", json!({}))
}

fn map_netease_default_search_keyword(response: Value) -> Result<SearchDefaultKeyword> {
    let data = response
        .get("data")
        .filter(|data| data.is_object())
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase default search response is missing its data object",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "response": response }))
        })?;
    let keyword = data
        .get("realkeyword")
        .and_then(json_scalar_string)
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase default search response is missing realkeyword",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "response": response }))
        })?;
    let display_text = [
        data.get("showKeyword"),
        data.pointer("/styleKeyword/keyWord"),
    ]
    .into_iter()
    .flatten()
    .find_map(|value| {
        json_scalar_string(value)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
    .unwrap_or_else(|| keyword.clone());
    let kind = data
        .get("searchType")
        .and_then(json_u64)
        .and_then(netease_search_kind_from_type);
    let image_url = data
        .get("imageUrl")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    Ok(SearchDefaultKeyword {
        keyword,
        display_text,
        kind,
        image_url,
        extensions: Extensions::from([("response".to_owned(), response)]),
    })
}

const fn netease_search_kind_from_type(value: u64) -> Option<SearchKind> {
    match value {
        1 => Some(SearchKind::Track),
        10 => Some(SearchKind::Album),
        100 => Some(SearchKind::Artist),
        1_000 => Some(SearchKind::Playlist),
        1_002 => Some(SearchKind::User),
        1_004 => Some(SearchKind::Mv),
        1_006 => Some(SearchKind::Lyric),
        1_009 => Some(SearchKind::Podcast),
        1_014 => Some(SearchKind::Video),
        1_018 => Some(SearchKind::Mixed),
        2_000 => Some(SearchKind::Voice),
        _ => None,
    }
}

fn netease_trending_search_request(detail: SearchTrendingDetail) -> (&'static str, Value, bool) {
    match detail {
        SearchTrendingDetail::Brief => ("/api/search/hot", json!({ "type": 1111 }), false),
        SearchTrendingDetail::Full => ("/api/hotsearchlist/get", json!({}), true),
    }
}

fn map_netease_trending_searches(
    detail: SearchTrendingDetail,
    response: Value,
) -> Result<SearchTrendingList> {
    let raw_entries = match detail {
        SearchTrendingDetail::Brief => response.pointer("/result/hots"),
        SearchTrendingDetail::Full => response.get("data"),
    }
    .and_then(Value::as_array)
    .cloned()
    .ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase trending search response is missing its entries array",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "detail": detail, "response": response }))
    })?;
    let entries = raw_entries
        .into_iter()
        .enumerate()
        .map(|(index, raw)| {
            let keyword_field = match detail {
                SearchTrendingDetail::Brief => "first",
                SearchTrendingDetail::Full => "searchWord",
            };
            let keyword = raw
                .get(keyword_field)
                .and_then(json_scalar_string)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    TuneWeaveError::new(
                        ErrorCode::UpstreamError,
                        "NetEase trending search entry is missing its keyword",
                    )
                    .with_platform(Platform::Netease)
                    .with_details(json!({ "detail": detail, "entry": raw }))
                })?;
            let description_field = match detail {
                SearchTrendingDetail::Brief => "third",
                SearchTrendingDetail::Full => "content",
            };
            let description = raw
                .get(description_field)
                .and_then(json_scalar_string)
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty());
            let score = (detail == SearchTrendingDetail::Full)
                .then(|| raw.get("score").and_then(json_u64))
                .flatten();
            let icon_type = raw.get("iconType").and_then(json_i64);
            let icon_url = search_trending_url(&raw, "iconUrl");
            let target_url = search_trending_url(&raw, "url");
            Ok(SearchTrendingEntry {
                rank: u32::try_from(index.saturating_add(1)).unwrap_or(u32::MAX),
                keyword,
                description,
                score,
                icon_type,
                icon_url,
                target_url,
                extensions: Extensions::from([("response".to_owned(), raw)]),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(SearchTrendingList {
        detail,
        entries,
        extensions: Extensions::from([("response".to_owned(), response)]),
    })
}

fn search_trending_url(raw: &Value, field: &str) -> Option<String> {
    raw.get(field)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

fn netease_search_suggestion_request(
    client: SearchSuggestionClient,
    query: &str,
) -> (&'static str, Value, bool) {
    match client {
        SearchSuggestionClient::Web => ("/api/search/suggest/web", json!({ "s": query }), true),
        SearchSuggestionClient::Mobile => {
            ("/api/search/suggest/keyword", json!({ "s": query }), true)
        }
        SearchSuggestionClient::Pc => (
            "/api/search/pc/suggest/keyword/get",
            json!({ "keyword": query }),
            false,
        ),
    }
}

fn map_netease_search_suggestions(
    client: SearchSuggestionClient,
    query: &str,
    response: Value,
) -> Result<SearchSuggestionList> {
    let (suggestions, recommendations) = match client {
        SearchSuggestionClient::Web => {
            let result = search_suggestion_container(&response, "result", client)?;
            (map_netease_web_search_suggestions(result)?, Vec::new())
        }
        SearchSuggestionClient::Mobile => {
            let result = search_suggestion_container(&response, "result", client)?;
            let suggestions = optional_search_suggestion_array(result, "allMatch", client)?
                .into_iter()
                .map(|raw| map_netease_keyword_suggestion(raw, None))
                .collect::<Result<Vec<_>>>()?;
            (suggestions, Vec::new())
        }
        SearchSuggestionClient::Pc => {
            let data = search_suggestion_container(&response, "data", client)?;
            let suggestions = optional_search_suggestion_array(data, "suggests", client)?
                .into_iter()
                .map(|raw| map_netease_keyword_suggestion(raw, None))
                .collect::<Result<Vec<_>>>()?;
            let recommendations = optional_search_suggestion_array(data, "recs", client)?
                .into_iter()
                .map(|raw| map_netease_keyword_suggestion(raw, None))
                .collect::<Result<Vec<_>>>()?;
            (suggestions, recommendations)
        }
    };
    Ok(SearchSuggestionList {
        query: query.to_owned(),
        client,
        suggestions,
        recommendations,
        extensions: Extensions::from([("response".to_owned(), response)]),
    })
}

fn search_suggestion_container<'a>(
    response: &'a Value,
    field: &str,
    client: SearchSuggestionClient,
) -> Result<&'a Value> {
    response
        .get(field)
        .filter(|value| value.is_object())
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase {client:?} search suggestions are missing {field}"),
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "response": response }))
        })
}

fn optional_search_suggestion_array(
    container: &Value,
    field: &str,
    client: SearchSuggestionClient,
) -> Result<Vec<Value>> {
    match container.get(field) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(values)) => Ok(values.clone()),
        Some(value) => Err(TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase {client:?} search suggestion {field} is not an array"),
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "value": value }))),
    }
}

fn map_netease_web_search_suggestions(result: &Value) -> Result<Vec<SearchSuggestion>> {
    let mut sections = result
        .get("order")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter_map(web_search_suggestion_section)
        .collect::<Vec<_>>();
    for section in [
        ("songs", SearchKind::Track),
        ("albums", SearchKind::Album),
        ("artists", SearchKind::Artist),
        ("playlists", SearchKind::Playlist),
        ("userprofiles", SearchKind::User),
        ("mvs", SearchKind::Mv),
        ("djRadios", SearchKind::Podcast),
        ("videos", SearchKind::Video),
    ] {
        if result.get(section.0).is_some() && !sections.contains(&section) {
            sections.push(section);
        }
    }
    let mut suggestions = Vec::new();
    for (field, kind) in sections {
        for raw in optional_search_suggestion_array(result, field, SearchSuggestionClient::Web)? {
            let resource = map_cloud_search_item(kind, raw.clone());
            let keyword = search_suggestion_keyword(&raw)
                .or_else(|| search_item_keyword(&resource))
                .ok_or_else(|| {
                    TuneWeaveError::new(
                        ErrorCode::UpstreamError,
                        "NetEase web search suggestion is missing a display keyword",
                    )
                    .with_platform(Platform::Netease)
                    .with_details(json!({ "section": field, "entry": raw }))
                })?;
            suggestions.push(SearchSuggestion {
                keyword,
                kind: Some(kind),
                display_text: None,
                icon_url: None,
                resource: Some(resource),
                extensions: Extensions::from([
                    ("section".to_owned(), json!(field)),
                    ("response".to_owned(), raw),
                ]),
            });
        }
    }
    Ok(suggestions)
}

fn web_search_suggestion_section(value: &str) -> Option<(&'static str, SearchKind)> {
    match value {
        "song" | "songs" => Some(("songs", SearchKind::Track)),
        "album" | "albums" => Some(("albums", SearchKind::Album)),
        "artist" | "artists" => Some(("artists", SearchKind::Artist)),
        "playlist" | "playlists" => Some(("playlists", SearchKind::Playlist)),
        "user" | "users" | "userprofiles" => Some(("userprofiles", SearchKind::User)),
        "mv" | "mvs" => Some(("mvs", SearchKind::Mv)),
        "radio" | "djRadio" | "djRadios" => Some(("djRadios", SearchKind::Podcast)),
        "video" | "videos" => Some(("videos", SearchKind::Video)),
        _ => None,
    }
}

fn map_netease_keyword_suggestion(
    raw: Value,
    resource: Option<SearchItem>,
) -> Result<SearchSuggestion> {
    let keyword = search_suggestion_keyword(&raw).ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase keyword search suggestion is missing its keyword",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "entry": raw }))
    })?;
    let kind = ["type", "resourceType"].into_iter().find_map(|field| {
        raw.get(field)
            .and_then(json_u64)
            .and_then(netease_search_kind_from_type)
    });
    let display_text = ["showText", "feature"].into_iter().find_map(|field| {
        raw.get(field)
            .and_then(json_scalar_string)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    });
    let icon_url = search_trending_url(&raw, "iconUrl");
    Ok(SearchSuggestion {
        keyword,
        kind,
        display_text,
        icon_url,
        resource,
        extensions: Extensions::from([("response".to_owned(), raw)]),
    })
}

fn search_suggestion_keyword(raw: &Value) -> Option<String> {
    [
        "keyword",
        "searchWord",
        "query",
        "name",
        "nickname",
        "title",
    ]
    .into_iter()
    .find_map(|field| {
        raw.get(field)
            .and_then(json_scalar_string)
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    })
}

fn search_item_keyword(item: &SearchItem) -> Option<String> {
    let value = match item {
        SearchItem::Track(track) => Some(track.name.as_str()),
        SearchItem::Album(album) => Some(album.name.as_str()),
        SearchItem::Artist(artist) => Some(artist.name.as_str()),
        SearchItem::Playlist(playlist) => Some(playlist.name.as_str()),
        SearchItem::User(user) => Some(user.name.as_str()),
        SearchItem::Video(video) => Some(video.title.as_str()),
        SearchItem::RadioStation(station) => Some(station.name.as_str()),
        SearchItem::Podcast(podcast) => Some(podcast.name.as_str()),
        SearchItem::Opaque(item) => item.title.as_deref(),
    }?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_owned())
}

fn netease_search_multi_match_request(kind: SearchKind, query: &str) -> (&'static str, Value) {
    (
        "/api/search/suggest/multimatch",
        json!({
            "type": netease_cloud_search_type(kind),
            "s": query
        }),
    )
}

fn map_netease_search_multi_match(
    query: &str,
    requested_kind: SearchKind,
    response: Value,
) -> Result<SearchMultiMatch> {
    let result = response
        .get("result")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase multi-match search response is missing its result object",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "response": response }))
        })?;
    let mut section_names = Vec::new();
    let order = match result.get("orders") {
        None | Some(Value::Null) => result.get("order"),
        order => order,
    };
    if let Some(order) = order {
        let order = order.as_array().ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase multi-match search section order is not an array",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "order": order }))
        })?;
        for section in order {
            let section = section
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    TuneWeaveError::new(
                        ErrorCode::UpstreamError,
                        "NetEase multi-match search section order contains an invalid name",
                    )
                    .with_platform(Platform::Netease)
                    .with_details(json!({ "section": section }))
                })?;
            if !section_names.iter().any(|known| known == section) {
                section_names.push(section.to_owned());
            }
        }
    }
    for (section, value) in result {
        if !matches!(section.as_str(), "orders" | "order")
            && value.is_array()
            && !section_names.iter().any(|known| known == section)
        {
            section_names.push(section.clone());
        }
    }

    let mut sections = Vec::with_capacity(section_names.len());
    for (index, section) in section_names.into_iter().enumerate() {
        let raw_items = match result.get(&section) {
            None | Some(Value::Null) => Vec::new(),
            Some(Value::Array(items)) => items.clone(),
            Some(value) => {
                return Err(TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    format!("NetEase multi-match search section {section} is not an array"),
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "section": section, "value": value })));
            }
        };
        let kind = netease_multi_match_section_kind(&section);
        let items = raw_items
            .iter()
            .cloned()
            .map(|raw| map_netease_multi_match_item(&section, kind, raw))
            .collect();
        sections.push(SearchMultiMatchSection {
            section,
            kind,
            items,
            extensions: Extensions::from([
                ("order_index".to_owned(), json!(index)),
                ("returned_count".to_owned(), json!(raw_items.len())),
            ]),
        });
    }

    Ok(SearchMultiMatch {
        query: query.to_owned(),
        requested_kind,
        sections,
        extensions: Extensions::from([
            (
                "platform_type".to_owned(),
                json!(netease_cloud_search_type(requested_kind)),
            ),
            ("response".to_owned(), response),
        ]),
    })
}

fn netease_multi_match_section_kind(section: &str) -> Option<SearchKind> {
    match section {
        "song" | "songs" => Some(SearchKind::Track),
        "album" | "albums" => Some(SearchKind::Album),
        "artist" | "artists" => Some(SearchKind::Artist),
        "playlist" | "playlists" => Some(SearchKind::Playlist),
        "user" | "users" | "userprofile" | "userprofiles" => Some(SearchKind::User),
        "mv" | "mvs" => Some(SearchKind::Mv),
        "djRadio" | "djRadios" | "radio" | "radios" => Some(SearchKind::Podcast),
        "new_mlog" | "video" | "videos" => Some(SearchKind::Video),
        "voice" | "voices" | "resources" => Some(SearchKind::Voice),
        _ => None,
    }
}

fn map_netease_multi_match_item(section: &str, kind: Option<SearchKind>, raw: Value) -> SearchItem {
    if section == "new_mlog" {
        let record = raw
            .get("baseInfo")
            .filter(|value| value.as_object().is_some_and(|object| !object.is_empty()))
            .cloned()
            .unwrap_or_else(|| raw.clone());
        if let Ok(mut video) = map_artist_video(record) {
            video.extensions.insert("multi_match_item".to_owned(), raw);
            return SearchItem::Video(video);
        }
    }
    if let Some(kind) = kind {
        return map_cloud_search_item(kind, raw);
    }
    let SearchItem::Opaque(mut item) = opaque_cloud_search_item(SearchKind::Mixed, raw, None)
    else {
        unreachable!("mixed search items always use the opaque representation")
    };
    item.kind = section.to_owned();
    SearchItem::Opaque(item)
}

fn netease_local_track_match_request(
    request: &LocalTrackMatchRequest,
) -> Result<(&'static str, Value, String)> {
    let md5 = normalize_local_match_md5(&request.md5)?;
    let duration_seconds = request.duration_ms as f64 / 1_000.0;
    let songs = json!([{
        "title": request.title,
        "album": request.album,
        "artist": request.artist,
        "duration": duration_seconds,
        "persistId": md5
    }]);
    Ok((
        "/api/search/match/new",
        json!({ "songs": songs.to_string() }),
        md5,
    ))
}

fn normalize_local_match_md5(md5: &str) -> Result<String> {
    let md5 = md5.trim().to_ascii_lowercase();
    if md5.len() != 32 || !md5.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(TuneWeaveError::invalid_request(
            "local track md5 must contain exactly 32 hexadecimal characters",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "md5": md5 })));
    }
    Ok(md5)
}

fn map_netease_local_track_match(md5: &str, response: Value) -> Result<LocalTrackMatchResult> {
    let result = response
        .get("result")
        .filter(|value| value.is_object())
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase local track match response is missing its result object",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "response": response }))
        })?;
    let raw_ids = result.get("ids").and_then(Value::as_array).ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase local track match response is missing its ids array",
        )
        .with_platform(Platform::Netease)
        .with_details(json!({ "response": response }))
    })?;
    let matched_ids = raw_ids
        .iter()
        .map(|value| {
            json_scalar_string(value).ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "NetEase local track match response contains an invalid matched id",
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "id": value }))
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let raw_songs = result
        .get("songs")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase local track match response is missing its songs array",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "response": response }))
        })?;
    let matches = raw_songs
        .into_iter()
        .map(|raw| {
            let song = parse_body::<Song>(raw.clone())?;
            let mut track = map_song(song, None)?;
            track.extensions.insert("local_match_item".to_owned(), raw);
            Ok(track)
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(LocalTrackMatchResult {
        md5: md5.to_owned(),
        matches,
        extensions: Extensions::from([
            ("matched_ids".to_owned(), json!(matched_ids)),
            ("response".to_owned(), response),
        ]),
    })
}

fn netease_user_membership_request(id: Option<u64>) -> (&'static str, Value) {
    (
        "/api/music-vip-membership/front/vip/info",
        json!({ "userId": id.map(|id| id.to_string()).unwrap_or_default() }),
    )
}

fn map_netease_user_membership(id: Option<u64>, response: Value) -> Result<MembershipSummary> {
    let data = response
        .get("data")
        .filter(|value| value.is_object())
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase user membership response is missing its data object",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "response": response }))
        })?;
    let level = data
        .get("redVipLevel")
        .and_then(json_u64)
        .map(u32::try_from)
        .transpose()
        .map_err(|_| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase user membership level exceeds the supported range",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "level": data.get("redVipLevel") }))
        })?;
    let annual_count = data.get("redVipAnnualCount").and_then(json_i64);
    let icon_url = data
        .get("redVipLevelIcon")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let user_ref = id
        .map(|id| ResourceRef::new(Platform::Netease, id.to_string()))
        .transpose()
        .map_err(|error| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                format!("NetEase returned an invalid membership user id: {error}"),
            )
            .with_platform(Platform::Netease)
        })?;
    Ok(MembershipSummary {
        user_ref,
        level,
        active: None,
        annual_count,
        expires_at: None,
        icon_url,
        extensions: Extensions::from([("response".to_owned(), response)]),
    })
}

fn netease_catalog_search_request(
    query: &SearchQuery,
    keyword: &str,
    limit: u32,
) -> (&'static str, Value, SearchVariant) {
    if query.kind == SearchKind::Podcast && query.variant == SearchVariant::Default {
        return (
            "/api/search/voicelist/get",
            json!({
                "keyword": keyword,
                "scene": "normal",
                "limit": limit,
                "offset": query.offset,
                "e_r": true
            }),
            SearchVariant::Default,
        );
    }
    let variant = match query.variant {
        SearchVariant::Default => SearchVariant::Cloud,
        variant => variant,
    };
    match (variant, query.kind) {
        (SearchVariant::Legacy, SearchKind::Voice) => (
            "/api/search/voice/get",
            json!({
                "keyword": keyword,
                "scene": "normal",
                "limit": limit,
                "offset": query.offset
            }),
            variant,
        ),
        (SearchVariant::Legacy, _) => (
            "/api/search/get",
            json!({
                "s": keyword,
                "type": netease_cloud_search_type(query.kind),
                "limit": limit,
                "offset": query.offset
            }),
            variant,
        ),
        (SearchVariant::Cloud, _) => (
            "/api/cloudsearch/pc",
            json!({
                "s": keyword,
                "type": netease_cloud_search_type(query.kind),
                "limit": limit,
                "offset": query.offset,
                "total": true
            }),
            variant,
        ),
        (SearchVariant::Default, _) => unreachable!("default search variant is resolved above"),
    }
}

fn cloud_search_shape(kind: SearchKind) -> (&'static [&'static str], &'static [&'static str]) {
    match kind {
        SearchKind::Track | SearchKind::Lyric => (&["songs"], &["songCount"]),
        SearchKind::Album => (&["albums"], &["albumCount"]),
        SearchKind::Artist => (&["artists"], &["artistCount"]),
        SearchKind::Playlist => (&["playlists"], &["playlistCount"]),
        SearchKind::User => (&["userprofiles"], &["userprofileCount"]),
        SearchKind::Mv => (&["mvs"], &["mvCount"]),
        SearchKind::RadioStation => (&["djRadios"], &["djRadiosCount"]),
        SearchKind::Podcast => (&["resources", "djRadios"], &["totalCount", "djRadiosCount"]),
        SearchKind::Video => (&["videos"], &["videoCount"]),
        SearchKind::Mixed => (&[], &[]),
        SearchKind::Voice => (
            &["voices", "resources"],
            &["voiceCount", "resourceCount", "totalCount"],
        ),
    }
}

fn preferred_cloud_search_result<'a>(
    kind: SearchKind,
    body: &'a Value,
    item_keys: &[&str],
) -> Option<&'a Value> {
    if !matches!(kind, SearchKind::Voice | SearchKind::Podcast) {
        return body.get("result");
    }

    let mut first_present = None;
    for candidate in [body.get("result"), body.get("data")].into_iter().flatten() {
        if first_present.is_none() {
            first_present = Some(candidate);
        }
        if preferred_array_field(candidate, item_keys).is_some_and(|items| !items.is_empty()) {
            return Some(candidate);
        }
    }
    first_present
}

fn map_cloud_search_response(
    kind: SearchKind,
    limit: u32,
    offset: u32,
    body: Value,
) -> Result<Page<SearchItem>> {
    let (item_keys, count_keys) = cloud_search_shape(kind);
    let result = preferred_cloud_search_result(kind, &body, item_keys)
        .cloned()
        .unwrap_or_else(|| json!({}));
    let raw_items = preferred_array_field(&result, item_keys)
        .cloned()
        .unwrap_or_default();
    let total = count_keys
        .iter()
        .find_map(|key| result.get(*key).and_then(json_u64));
    let had_item_array = item_keys
        .iter()
        .any(|key| result.get(*key).is_some_and(Value::is_array));
    let mut items = raw_items
        .iter()
        .cloned()
        .map(|raw| map_cloud_search_item(kind, raw))
        .collect::<Vec<_>>();
    let opaque_result = items.is_empty()
        && !had_item_array
        && result.as_object().is_some_and(|result| !result.is_empty());
    if opaque_result {
        items.push(opaque_cloud_search_item(kind, result.clone(), None));
    }

    let consumed = u32::try_from(raw_items.len()).unwrap_or(u32::MAX);
    let next_offset = offset.saturating_add(consumed);
    let has_more = if opaque_result {
        false
    } else if let Some(has_more) = result.get("hasMore").and_then(json_bool) {
        consumed > 0 && has_more
    } else if let Some(total) = total {
        u64::from(next_offset) < total
    } else {
        consumed > 0 && consumed >= limit
    };
    let mut extensions = Extensions::new();
    extensions.insert("kind".to_owned(), json!(kind));
    extensions.insert(
        "platform_type".to_owned(),
        json!(netease_cloud_search_type(kind)),
    );
    extensions.insert("returned_count".to_owned(), json!(raw_items.len()));
    extensions.insert(
        "limit_applied".to_owned(),
        json!(!had_item_array || raw_items.len() <= limit as usize),
    );
    extensions.insert("response".to_owned(), body);
    Ok(Page {
        items,
        pagination: PageMeta {
            limit,
            offset,
            total,
            next_offset: (has_more && consumed > 0).then_some(next_offset),
            has_more,
            extensions,
        },
    })
}

fn map_cloud_search_item(kind: SearchKind, raw: Value) -> SearchItem {
    let mapped = match kind {
        SearchKind::Track | SearchKind::Lyric => {
            let song = parse_body::<Song>(raw.clone());
            song.and_then(|song| map_song(song, None)).map(|mut track| {
                track
                    .extensions
                    .insert("search_item".to_owned(), raw.clone());
                SearchItem::Track(track)
            })
        }
        SearchKind::Album => map_album_list_item(raw.clone()).map(SearchItem::Album),
        SearchKind::Artist => map_artist_list_item(raw.clone()).map(SearchItem::Artist),
        SearchKind::Playlist => parse_body::<PlaylistDetail>(raw.clone())
            .and_then(map_playlist)
            .map(|mut playlist| {
                playlist
                    .extensions
                    .insert("search_item".to_owned(), raw.clone());
                SearchItem::Playlist(playlist)
            }),
        SearchKind::User => {
            map_artist_fan(json!({ "userProfile": raw.clone() })).map(|mut user| {
                user.extensions
                    .insert("search_item".to_owned(), raw.clone());
                SearchItem::User(user)
            })
        }
        SearchKind::Mv => map_artist_mv(raw.clone()).map(SearchItem::Video),
        SearchKind::RadioStation => {
            map_radio_station_fields(&raw, &raw, None).map(|mut station| {
                station
                    .extensions
                    .insert("search_item".to_owned(), raw.clone());
                SearchItem::RadioStation(station)
            })
        }
        SearchKind::Podcast => {
            let source = raw
                .get("baseInfo")
                .filter(|value| value.as_object().is_some_and(|object| !object.is_empty()))
                .cloned()
                .unwrap_or_else(|| raw.clone());
            map_netease_podcast(source).map(|mut podcast| {
                podcast
                    .extensions
                    .insert("search_item".to_owned(), raw.clone());
                SearchItem::Podcast(podcast)
            })
        }
        SearchKind::Video => map_cloud_search_video(raw.clone()).map(SearchItem::Video),
        SearchKind::Mixed | SearchKind::Voice => {
            return opaque_cloud_search_item(kind, raw, None);
        }
    };
    mapped.unwrap_or_else(|error| opaque_cloud_search_item(kind, raw, Some(error.message)))
}

fn cloud_search_item_source(raw: &Value) -> &Value {
    let mut best = raw;
    let mut best_score = cloud_search_item_source_score(raw);
    for field in ["baseInfo", "data", "resource", "content"] {
        let Some(candidate) = raw.get(field).filter(|value| value.is_object()) else {
            continue;
        };
        let score = cloud_search_item_source_score(candidate);
        if score > best_score {
            best = candidate;
            best_score = score;
        }
    }
    best
}

fn cloud_search_item_source_score(source: &Value) -> usize {
    let has_id = ["vid", "id", "resourceId", "userId", "djId"]
        .into_iter()
        .any(|field| {
            source
                .get(field)
                .and_then(json_scalar_string)
                .is_some_and(|value| !value.trim().is_empty() && value != "0")
        });
    let has_title = ["title", "name", "nickname"]
        .into_iter()
        .any(|field| radio_text_field(source, &[field]).is_some());
    let creator_count = preferred_array_field(source, &["creators", "artists", "creator"])
        .map_or(0, Vec::len)
        .min(4);
    let metadata_count = [
        "coverUrl",
        "cover",
        "picUrl",
        "durationms",
        "durationMs",
        "duration",
        "playTime",
        "playCount",
        "description",
        "desc",
    ]
    .into_iter()
    .filter(|field| source.get(*field).is_some_and(|value| !value.is_null()))
    .count();
    usize::from(has_id) * 16 + usize::from(has_title) * 16 + creator_count * 2 + metadata_count
}

fn map_cloud_search_video(raw: Value) -> Result<Video> {
    let source = cloud_search_item_source(&raw);
    let id = ["vid", "id"]
        .into_iter()
        .find_map(|field| source.get(field).and_then(usable_resource_id))
        .ok_or_else(|| {
            TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "NetEase video search item did not contain an id",
            )
            .with_platform(Platform::Netease)
        })?;
    let title = radio_text_field(source, &["title", "name"]).ok_or_else(|| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            "NetEase video search item did not contain a title",
        )
        .with_platform(Platform::Netease)
    })?;
    let resource_ref = ResourceRef::new(Platform::Netease, &id).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("NetEase returned an invalid video search id: {error}"),
        )
        .with_platform(Platform::Netease)
    })?;
    let creators = preferred_array_field(source, &["creators", "artists", "creator"])
        .into_iter()
        .flatten()
        .filter_map(|creator| {
            let name = radio_text_field(creator, &["userName", "name", "nickname"])?;
            let creator_ref = ["userId", "id"]
                .into_iter()
                .find_map(|field| creator.get(field).and_then(usable_resource_id))
                .and_then(|id| ResourceRef::new(Platform::Netease, id).ok());
            Some(CreatorSummary {
                resource_ref: creator_ref,
                name,
                avatar_url: radio_text_field(creator, &["avatarUrl", "img1v1Url"]),
            })
        })
        .collect();
    let duration_ms = ["durationms", "durationMs", "duration"]
        .into_iter()
        .find_map(|field| {
            source
                .get(field)
                .and_then(json_u64)
                .filter(|duration| *duration > 0)
        });
    let play_count = ["playTime", "playCount"]
        .into_iter()
        .find_map(|field| source.get(field).and_then(json_u64));
    let published_at = source
        .get("publishTime")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let description = radio_text_field(source, &["description", "desc"]).unwrap_or_default();
    let cover_url = radio_text_field(source, &["coverUrl", "cover", "picUrl"]);
    let subscribed = radio_bool_field(source, &["subed", "subscribed"]);
    let mut extensions = Extensions::new();
    extensions.insert("search_item".to_owned(), raw);
    Ok(Video {
        resource_ref,
        platform: Platform::Netease,
        id,
        title,
        creators,
        description,
        cover_url,
        duration_ms,
        published_at,
        play_count,
        subscribed,
        extensions,
    })
}

fn opaque_cloud_search_item(
    kind: SearchKind,
    raw: Value,
    mapping_error: Option<String>,
) -> SearchItem {
    let source = cloud_search_item_source(&raw);
    let id = ["id", "vid", "userId", "resourceId", "djId"]
        .into_iter()
        .find_map(|field| source.get(field).and_then(usable_resource_id))
        .or_else(|| {
            ["id", "vid", "userId", "resourceId", "djId"]
                .into_iter()
                .find_map(|field| raw.get(field).and_then(usable_resource_id))
        });
    let title = ["name", "title", "nickname"]
        .into_iter()
        .find_map(|field| {
            source
                .get(field)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
        .or_else(|| {
            ["name", "title", "nickname"].into_iter().find_map(|field| {
                raw.get(field)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
            })
        });
    let mut extensions = Extensions::new();
    extensions.insert("response".to_owned(), raw);
    if let Some(mapping_error) = mapping_error {
        extensions.insert("mapping_error".to_owned(), json!(mapping_error));
    }
    SearchItem::Opaque(SearchOpaqueItem {
        platform: Platform::Netease,
        kind: serde_json::to_value(kind)
            .ok()
            .and_then(|kind| kind.as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".to_owned()),
        id,
        title,
        extensions,
    })
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
    }
}

fn ensure_success(body: &Value) -> Result<()> {
    let code = body["code"]
        .as_i64()
        .or_else(|| body["code"].as_str().and_then(|code| code.parse().ok()))
        .unwrap_or(500);
    if code == 200 {
        return Ok(());
    }
    let message = ["message", "msg"]
        .into_iter()
        .find_map(|field| {
            body[field]
                .as_str()
                .map(str::trim)
                .filter(|message| !message.is_empty())
        })
        .unwrap_or("NetEase request failed");
    let error_code = match code {
        301 | 401 => ErrorCode::AuthenticationRequired,
        -2 | 403 => ErrorCode::PermissionDenied,
        404 => ErrorCode::ResourceNotFound,
        429 => ErrorCode::RateLimited,
        _ => ErrorCode::UpstreamError,
    };
    Err(TuneWeaveError::new(error_code, message)
        .with_platform(Platform::Netease)
        .retryable(code == 429 || code >= 500)
        .with_details(json!({ "upstream_code": code })))
}

fn parse_body<T: DeserializeOwned>(body: Value) -> Result<T> {
    serde_json::from_value(body).map_err(|error| {
        TuneWeaveError::new(
            ErrorCode::UpstreamError,
            format!("failed to parse NetEase response: {error}"),
        )
        .with_platform(Platform::Netease)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qr_login_image_is_a_self_contained_svg_data_url() {
        let data_url = qr_image_data_url("orpheus://orpheus/pub/login?codekey=test-key")
            .expect("encode QR image");
        let encoded = data_url
            .strip_prefix("data:image/svg+xml;base64,")
            .expect("SVG data URL");
        let image = BASE64.decode(encoded).expect("base64 SVG");
        let image = String::from_utf8(image).expect("UTF-8 SVG");
        assert!(image.starts_with("<?xml"));
        assert!(image.contains("<svg"));
        assert!(image.contains("width=\""));
        assert!(image.contains("height=\""));
        assert!(!image.contains("test-key"));
    }

    static TEST_CREDENTIAL_DIRECTORY_SEQUENCE: std::sync::atomic::AtomicU64 =
        std::sync::atomic::AtomicU64::new(1);

    struct TestCredentialDirectory(std::path::PathBuf);

    impl TestCredentialDirectory {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "tuneweave-netease-credential-store-{}-{}",
                std::process::id(),
                TEST_CREDENTIAL_DIRECTORY_SEQUENCE
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).expect("create credential test directory");
            Self(path)
        }

        fn store(&self) -> Arc<dyn AccountCredentialStore> {
            Arc::new(tuneweave_core::FileAccountCredentialStore::new(&self.0))
        }
    }

    impl Drop for TestCredentialDirectory {
        fn drop(&mut self) {
            if self.0.starts_with(std::env::temp_dir()) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    fn fixture_song() -> Song {
        serde_json::from_value(json!({
            "id": 123,
            "name": "反方向的钟",
            "alia": ["Clockwise"],
            "ar": [{"id": 6452, "name": "周杰伦"}],
            "al": {"id": 456, "name": "Jay", "picUrl": "https://example.test/cover.jpg"},
            "dt": 258000,
            "mv": 0,
            "fee": 1,
            "st": 0,
            "mark": 8192,
            "privilege": {"id": 123, "st": 0, "fee": 1, "pl": 320000, "maxbr": 999000},
            "l": {"br": 128000},
            "m": {"br": 192000},
            "h": {"br": 320000},
            "sq": {"br": 999000},
            "hr": null
        }))
        .expect("valid fixture")
    }

    fn fixture_podcast_program(id: u64, podcast_id: u64, audio_id: u64) -> Value {
        json!({
            "id": id,
            "name": "一期节目",
            "description": "节目介绍",
            "coverUrl": "https://example.test/episode.jpg",
            "duration": 258_000,
            "createTime": 1_704_067_200_000_u64,
            "serialNum": 42,
            "listenerCount": 1_234,
            "likedCount": 12,
            "commentCount": 3,
            "shareCount": 4,
            "subscribed": false,
            "existLyric": true,
            "programFeeType": 0,
            "buyed": false,
            "mainTrackId": audio_id,
            "mainSong": {
                "id": audio_id,
                "name": "一期节目音频",
                "alias": [],
                "artists": [{"id": 32_953_014, "name": "主播"}],
                "album": {
                    "id": 0,
                    "name": "代码时间",
                    "picUrl": "https://example.test/episode.jpg"
                },
                "duration": 258_000,
                "status": 0,
                "lMusic": {"bitrate": 128_000}
            },
            "dj": {
                "userId": 32_953_014,
                "nickname": "主播",
                "avatarUrl": "https://example.test/avatar.jpg"
            },
            "radio": {"id": podcast_id, "name": "代码时间"}
        })
    }

    fn fixture_podcast_radio(id: u64, name: &str) -> Value {
        json!({
            "id": id,
            "name": name,
            "desc": "播客介绍",
            "picUrl": "https://example.test/podcast.jpg",
            "dj": {
                "userId": 32_953_014,
                "nickname": "主播",
                "avatarUrl": "https://example.test/avatar.jpg"
            },
            "category": "音乐播客",
            "categoryId": 2,
            "copywriter": "热门推荐语",
            "createTime": 1_704_067_200_000_u64,
            "feeScope": 0,
            "playCount": 98_765,
            "programCount": 120,
            "radioFeeType": 0,
            "rcmdtext": "推荐理由",
            "subCount": 4_567,
            "futureField": {"kept": true}
        })
    }

    fn fixture_cloud_item(cloud_id: &str, source_id: u64) -> Value {
        json!({
            "songId": cloud_id,
            "songName": "反方向的钟（云盘）",
            "artist": "周杰伦",
            "album": "Jay",
            "cover": "https://example.test/cloud-cover.jpg",
            "fileName": "反方向的钟.flac",
            "fileSize": "50412168",
            "fileType": ".FLAC",
            "bitrate": "999000",
            "md5": "d02b8ab79d91c01167ba31e349fe5275",
            "addTime": 1704067200000_u64,
            "simpleSong": {
                "id": source_id,
                "name": "反方向的钟",
                "ar": [],
                "al": null,
                "dt": 258000,
                "st": 0,
                "sq": {"br": 999000}
            },
            "futureField": {"kept": true}
        })
    }

    #[test]
    fn maps_netease_song_to_unified_track() {
        let track = map_song(fixture_song(), None).expect("map song");
        assert_eq!(track.resource_ref.to_string(), "netease:123");
        assert_eq!(track.name, "反方向的钟");
        assert_eq!(track.artists[0].name, "周杰伦");
        assert_eq!(track.duration_ms, Some(258000));
        assert_eq!(
            track.available_qualities,
            vec![
                Quality::Standard,
                Quality::Higher,
                Quality::High,
                Quality::Lossless
            ]
        );
        assert_eq!(track.playable, Some(true));
        assert_eq!(track.extensions["fee"], 1);
    }

    #[test]
    fn song_quality_mapping_keeps_the_192k_tier_distinct_from_320k() {
        for (quality_fields, expected) in [
            (json!({"l": {"br": 128_000}}), vec![Quality::Standard]),
            (
                json!({"m": {"br": 192_000}}),
                vec![Quality::Standard, Quality::Higher],
            ),
            (
                json!({"h": {"br": 320_000}}),
                vec![Quality::Standard, Quality::Higher, Quality::High],
            ),
            (json!({"sq": {"br": 999_000}}), vec![Quality::Lossless]),
            (json!({"hr": {"br": 1_999_000}}), vec![Quality::Hires]),
        ] {
            let mut fixture = json!({"id": 1, "name": "quality fixture"});
            fixture
                .as_object_mut()
                .expect("song fixture object")
                .extend(quality_fields.as_object().expect("quality fields").clone());
            let song: Song = serde_json::from_value(fixture).expect("valid quality fixture");
            assert_eq!(map_qualities(&song), expected);
        }
    }

    #[test]
    fn default_search_keyword_matches_reference_protocol_and_response_shape() {
        let (path, payload) = netease_default_search_keyword_request();
        assert_eq!(path, "/api/search/defaultkeyword/get");
        assert_eq!(payload, json!({}));

        let prompt = map_netease_default_search_keyword(json!({
            "code": 200,
            "data": {
                "realkeyword": "周旋",
                "showKeyword": "🔥周旋 最近很火哦",
                "searchType": 1,
                "imageUrl": "https://example.test/search.png",
                "alg": "dq_0"
            },
            "message": null
        }))
        .expect("map default search keyword");
        assert_eq!(prompt.keyword, "周旋");
        assert_eq!(prompt.display_text, "🔥周旋 最近很火哦");
        assert_eq!(prompt.kind, Some(SearchKind::Track));
        assert_eq!(
            prompt.image_url.as_deref(),
            Some("https://example.test/search.png")
        );
        assert_eq!(prompt.extensions["response"]["data"]["alg"], "dq_0");

        let fallback = map_netease_default_search_keyword(json!({
            "code": 200,
            "data": {
                "realkeyword": "未知类型",
                "showKeyword": " ",
                "searchType": 9999,
                "styleKeyword": {"keyWord": "展示未知类型"}
            }
        }))
        .expect("map fallback display text");
        assert_eq!(fallback.display_text, "展示未知类型");
        assert_eq!(fallback.kind, None);
    }

    #[test]
    fn default_search_keyword_rejects_missing_data_and_keyword() {
        for response in [
            json!({"code": 200}),
            json!({"code": 200, "data": {"showKeyword": "只有展示词"}}),
        ] {
            assert_eq!(
                map_netease_default_search_keyword(response)
                    .expect_err("malformed default search keyword")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn trending_searches_match_brief_eapi_and_full_weapi_protocols() {
        let (path, payload, use_weapi) =
            netease_trending_search_request(SearchTrendingDetail::Brief);
        assert_eq!(path, "/api/search/hot");
        assert_eq!(payload, json!({"type": 1111}));
        assert!(!use_weapi);

        let brief = map_netease_trending_searches(
            SearchTrendingDetail::Brief,
            json!({
                "code": 200,
                "result": {"hots": [
                    {"first": "薛之谦", "second": 1, "third": null, "iconType": 1},
                    {"first": "周旋", "second": 1, "third": "热门", "iconType": 1}
                ]}
            }),
        )
        .expect("map brief trending searches");
        assert_eq!(brief.detail, SearchTrendingDetail::Brief);
        assert_eq!(brief.entries.len(), 2);
        assert_eq!(brief.entries[0].rank, 1);
        assert_eq!(brief.entries[0].keyword, "薛之谦");
        assert_eq!(brief.entries[0].score, None);
        assert_eq!(brief.entries[1].description.as_deref(), Some("热门"));
        assert_eq!(brief.extensions["response"]["code"], 200);

        let (path, payload, use_weapi) =
            netease_trending_search_request(SearchTrendingDetail::Full);
        assert_eq!(path, "/api/hotsearchlist/get");
        assert_eq!(payload, json!({}));
        assert!(use_weapi);

        let full = map_netease_trending_searches(
            SearchTrendingDetail::Full,
            json!({
                "code": 200,
                "data": [{
                    "searchWord": "薛之谦",
                    "score": 107509,
                    "content": "歌手热搜",
                    "iconType": 4,
                    "iconUrl": "https://example.test/hot.png",
                    "url": "https://example.test/search"
                }]
            }),
        )
        .expect("map full trending searches");
        assert_eq!(full.detail, SearchTrendingDetail::Full);
        assert_eq!(full.entries[0].score, Some(107_509));
        assert_eq!(full.entries[0].description.as_deref(), Some("歌手热搜"));
        assert_eq!(full.entries[0].icon_type, Some(4));
        assert_eq!(
            full.entries[0].icon_url.as_deref(),
            Some("https://example.test/hot.png")
        );
        assert_eq!(
            full.entries[0].target_url.as_deref(),
            Some("https://example.test/search")
        );
    }

    #[test]
    fn trending_searches_reject_missing_arrays_and_keywords() {
        for (detail, response) in [
            (SearchTrendingDetail::Brief, json!({"code": 200})),
            (
                SearchTrendingDetail::Full,
                json!({"code": 200, "data": [{"score": 1}]}),
            ),
        ] {
            assert_eq!(
                map_netease_trending_searches(detail, response)
                    .expect_err("malformed trending search response")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn search_suggestion_clients_match_all_reference_protocol_branches() {
        for (client, path, payload, use_weapi) in [
            (
                SearchSuggestionClient::Web,
                "/api/search/suggest/web",
                json!({"s": "海阔天空"}),
                true,
            ),
            (
                SearchSuggestionClient::Mobile,
                "/api/search/suggest/keyword",
                json!({"s": "海阔天空"}),
                true,
            ),
            (
                SearchSuggestionClient::Pc,
                "/api/search/pc/suggest/keyword/get",
                json!({"keyword": "海阔天空"}),
                false,
            ),
        ] {
            assert_eq!(
                netease_search_suggestion_request(client, "海阔天空"),
                (path, payload, use_weapi),
                "{client:?}"
            );
        }
    }

    #[test]
    fn maps_web_mobile_and_pc_search_suggestion_shapes_without_losing_resources() {
        let web = map_netease_search_suggestions(
            SearchSuggestionClient::Web,
            "海阔天空",
            json!({
                "code": 200,
                "result": {
                    "order": ["albums", "djRadios"],
                    "albums": [{
                        "id": 34209,
                        "name": "海阔天空",
                        "artists": [{"id": 11127, "name": "Beyond"}],
                        "picUrl": "https://example.test/album.jpg",
                        "size": 10
                    }],
                    "djRadios": [{
                        "id": 336355127,
                        "name": "代码时间",
                        "desc": "面向开发者的播客",
                        "picUrl": "https://example.test/podcast.jpg",
                        "dj": {"userId": 32953014, "nickname": "主播"},
                        "category": "科技",
                        "programCount": 120
                    }]
                }
            }),
        )
        .expect("map web suggestions");
        assert_eq!(web.client, SearchSuggestionClient::Web);
        assert_eq!(web.query, "海阔天空");
        assert_eq!(web.suggestions.len(), 2);
        assert_eq!(web.suggestions[0].keyword, "海阔天空");
        assert_eq!(web.suggestions[0].kind, Some(SearchKind::Album));
        assert!(matches!(
            web.suggestions[0].resource.as_ref(),
            Some(SearchItem::Album(_))
        ));
        assert_eq!(web.suggestions[1].keyword, "代码时间");
        assert_eq!(web.suggestions[1].kind, Some(SearchKind::Podcast));
        let Some(SearchItem::Podcast(podcast)) = web.suggestions[1].resource.as_ref() else {
            panic!("djRadios search suggestions must map to podcasts");
        };
        assert_eq!(podcast.resource_ref.to_string(), "netease:336355127");
        assert!(web.recommendations.is_empty());

        let mobile = map_netease_search_suggestions(
            SearchSuggestionClient::Mobile,
            "海阔天空",
            json!({
                "code": 200,
                "result": {"allMatch": [
                    {"keyword": "海阔天空", "type": 0, "resourceType": 10, "feature": ""},
                    {"keyword": "海阔天空尾奏", "type": 1},
                    {"keyword": "代码时间", "type": 0, "resourceType": 1009}
                ]}
            }),
        )
        .expect("map mobile suggestions");
        assert_eq!(mobile.suggestions.len(), 3);
        assert_eq!(mobile.suggestions[0].kind, Some(SearchKind::Album));
        assert!(mobile.suggestions[0].resource.is_none());
        assert_eq!(mobile.suggestions[2].kind, Some(SearchKind::Podcast));

        let pc = map_netease_search_suggestions(
            SearchSuggestionClient::Pc,
            "海阔天空",
            json!({
                "code": 200,
                "data": {
                    "suggests": [{
                        "keyword": "海阔天空",
                        "showText": "歌曲",
                        "iconUrl": "https://example.test/icon.png"
                    }],
                    "recs": [{"keyword": "海阔天空 Beyond"}],
                    "recTitle": "相关搜索"
                }
            }),
        )
        .expect("map PC suggestions");
        assert_eq!(pc.suggestions.len(), 1);
        assert_eq!(pc.recommendations.len(), 1);
        assert_eq!(pc.suggestions[0].display_text.as_deref(), Some("歌曲"));
        assert_eq!(
            pc.suggestions[0].icon_url.as_deref(),
            Some("https://example.test/icon.png")
        );
        assert_eq!(pc.recommendations[0].keyword, "海阔天空 Beyond");
        assert_eq!(pc.extensions["response"]["data"]["recTitle"], "相关搜索");
    }

    #[test]
    fn search_suggestions_reject_missing_containers_wrong_arrays_and_keywords() {
        for (client, response) in [
            (SearchSuggestionClient::Web, json!({"code": 200})),
            (
                SearchSuggestionClient::Mobile,
                json!({"code": 200, "result": {"allMatch": {}}}),
            ),
            (
                SearchSuggestionClient::Pc,
                json!({"code": 200, "data": {"suggests": [{}]}}),
            ),
        ] {
            assert_eq!(
                map_netease_search_suggestions(client, "海阔天空", response)
                    .expect_err("malformed search suggestions")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn multi_match_search_matches_reference_protocol_and_preserves_ordered_types() {
        let (path, payload) = netease_search_multi_match_request(SearchKind::Track, "海阔天空");
        assert_eq!(path, "/api/search/suggest/multimatch");
        assert_eq!(payload, json!({"type": 1, "s": "海阔天空"}));

        let result = map_netease_search_multi_match(
            "海阔天空",
            SearchKind::Track,
            json!({
                "code": 200,
                "result": {
                    "orders": null,
                    "order": ["artist", "new_mlog", "playlist", "djRadios"],
                    "artist": [{"id": 11127, "name": "Beyond"}],
                    "new_mlog": [{
                        "resourceId": "5501497",
                        "baseInfo": {
                            "id": "5501497",
                            "resource": {
                                "mlogBaseData": {
                                    "id": "5501497",
                                    "text": "海阔天空 Ver.2",
                                    "coverUrl": "https://example.test/video.jpg",
                                    "duration": 330230,
                                    "pubTime": 1496769827329_u64
                                },
                                "mlogExtVO": {
                                    "artists": [{"id": 11127, "name": "Beyond"}],
                                    "playCount": 3321987
                                }
                            }
                        }
                    }],
                    "playlist": [{"id": 151235962, "name": "粤语经典老歌"}],
                    "djRadios": [{
                        "id": 336355127,
                        "name": "代码时间",
                        "desc": "面向开发者的播客",
                        "dj": {"userId": 32953014, "nickname": "主播"}
                    }],
                    "mystery": [{"id": " ", "resourceId": "opaque-1", "title": "未知匹配"}]
                }
            }),
        )
        .expect("map multi-match search");

        assert_eq!(result.query, "海阔天空");
        assert_eq!(result.requested_kind, SearchKind::Track);
        assert_eq!(result.sections.len(), 5);
        assert_eq!(result.sections[0].section, "artist");
        assert_eq!(result.sections[0].kind, Some(SearchKind::Artist));
        assert!(matches!(result.sections[0].items[0], SearchItem::Artist(_)));
        assert_eq!(result.sections[1].section, "new_mlog");
        assert_eq!(result.sections[1].kind, Some(SearchKind::Video));
        let SearchItem::Video(video) = &result.sections[1].items[0] else {
            panic!("new_mlog should map to a video");
        };
        assert_eq!(video.resource_ref.to_string(), "netease:5501497");
        assert_eq!(video.title, "海阔天空 Ver.2");
        assert_eq!(video.creators[0].name, "Beyond");
        assert_eq!(result.sections[2].section, "playlist");
        assert!(matches!(
            result.sections[2].items[0],
            SearchItem::Playlist(_)
        ));
        assert_eq!(result.sections[3].section, "djRadios");
        assert_eq!(result.sections[3].kind, Some(SearchKind::Podcast));
        let SearchItem::Podcast(podcast) = &result.sections[3].items[0] else {
            panic!("djRadios multi-match sections must map to podcasts");
        };
        assert_eq!(podcast.resource_ref.to_string(), "netease:336355127");
        assert_eq!(result.sections[4].section, "mystery");
        assert_eq!(result.sections[4].kind, None);
        let SearchItem::Opaque(item) = &result.sections[4].items[0] else {
            panic!("unknown sections should remain opaque");
        };
        assert_eq!(item.kind, "mystery");
        assert_eq!(item.id.as_deref(), Some("opaque-1"));
        assert_eq!(result.extensions["platform_type"], 1);
        assert_eq!(result.extensions["response"]["code"], 200);
    }

    #[test]
    fn multi_match_search_rejects_malformed_result_order_and_sections() {
        for response in [
            json!({"code": 200}),
            json!({"code": 200, "result": {"orders": {}}}),
            json!({"code": 200, "result": {"orders": null, "order": {}}}),
            json!({"code": 200, "result": {"orders": [1]}}),
            json!({"code": 200, "result": {"orders": ["artist"], "artist": {}}}),
        ] {
            assert_eq!(
                map_netease_search_multi_match("test", SearchKind::Track, response)
                    .expect_err("malformed multi-match search")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn local_track_match_uses_reference_raw_api_payload_and_maps_candidates() {
        let request = LocalTrackMatchRequest {
            title: "富士山下".to_owned(),
            album: String::new(),
            artist: "陈奕迅".to_owned(),
            duration_ms: 259_210,
            md5: "BD708D006912A09D827F02E754CF8E56".to_owned(),
            account: None,
        };
        let (path, payload, md5) =
            netease_local_track_match_request(&request).expect("local match request");
        assert_eq!(path, "/api/search/match/new");
        assert_eq!(md5, "bd708d006912a09d827f02e754cf8e56");
        let songs: Value =
            serde_json::from_str(payload["songs"].as_str().expect("serialized songs payload"))
                .expect("valid songs JSON");
        assert_eq!(songs[0]["title"], "富士山下");
        assert_eq!(songs[0]["album"], "");
        assert_eq!(songs[0]["artist"], "陈奕迅");
        assert_eq!(songs[0]["duration"], 259.21);
        assert_eq!(songs[0]["persistId"], md5);

        let result = map_netease_local_track_match(
            &md5,
            json!({
                "code": 200,
                "result": {
                    "ids": [md5],
                    "songs": [{
                        "id": 65766,
                        "name": "富士山下",
                        "artists": [{"id": 2116, "name": "陈奕迅"}],
                        "album": {
                            "id": 6451,
                            "name": "What's Going On…?",
                            "picUrl": "https://example.test/album.jpg"
                        },
                        "duration": 258902,
                        "mvid": 303140,
                        "fee": 1,
                        "status": 0,
                        "lMusic": {"bitrate": 128000},
                        "hMusic": {"bitrate": 320000}
                    }]
                }
            }),
        )
        .expect("map local track match");
        assert_eq!(result.md5, md5);
        assert_eq!(result.matches.len(), 1);
        assert_eq!(result.matches[0].resource_ref.to_string(), "netease:65766");
        assert_eq!(result.matches[0].name, "富士山下");
        assert_eq!(result.matches[0].artists[0].name, "陈奕迅");
        assert_eq!(result.matches[0].duration_ms, Some(258_902));
        assert_eq!(result.extensions["matched_ids"][0], md5);
        assert_eq!(result.extensions["response"]["code"], 200);
    }

    #[test]
    fn local_track_match_preserves_no_match_and_rejects_invalid_inputs_or_responses() {
        let no_match = map_netease_local_track_match(
            "00000000000000000000000000000000",
            json!({"code": 200, "result": {"ids": [], "songs": []}}),
        )
        .expect("map no-match response");
        assert!(no_match.matches.is_empty());
        assert_eq!(no_match.extensions["matched_ids"], json!([]));

        for md5 in ["", "not-md5", "0123456789abcdef0123456789abcdeg"] {
            let request = LocalTrackMatchRequest {
                title: String::new(),
                album: String::new(),
                artist: String::new(),
                duration_ms: 0,
                md5: md5.to_owned(),
                account: None,
            };
            assert_eq!(
                netease_local_track_match_request(&request)
                    .expect_err("invalid md5")
                    .code,
                ErrorCode::InvalidRequest
            );
        }

        for response in [
            json!({"code": 200}),
            json!({"code": 200, "result": {"songs": []}}),
            json!({"code": 200, "result": {"ids": [], "songs": {}}}),
            json!({"code": 200, "result": {"ids": [{}], "songs": []}}),
        ] {
            assert_eq!(
                map_netease_local_track_match("00000000000000000000000000000000", response)
                    .expect_err("malformed local track match response")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn user_membership_matches_public_and_current_account_request_branches() {
        assert_eq!(
            netease_user_membership_request(Some(32_953_014)),
            (
                "/api/music-vip-membership/front/vip/info",
                json!({"userId": "32953014"})
            )
        );
        assert_eq!(
            netease_user_membership_request(None),
            (
                "/api/music-vip-membership/front/vip/info",
                json!({"userId": ""})
            )
        );

        let membership = map_netease_user_membership(
            Some(32_953_014),
            json!({
                "code": 200,
                "data": {
                    "redVipAnnualCount": -1,
                    "redVipDynamicIconUrl": null,
                    "redVipDynamicIconUrl2": null,
                    "redVipLevel": 7,
                    "redVipLevelIcon": "https://example.test/red-vip.png"
                },
                "message": "成功"
            }),
        )
        .expect("map public user membership");
        assert_eq!(
            membership
                .user_ref
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("netease:32953014")
        );
        assert_eq!(membership.level, Some(7));
        assert_eq!(membership.annual_count, Some(-1));
        assert_eq!(
            membership.icon_url.as_deref(),
            Some("https://example.test/red-vip.png")
        );
        assert_eq!(membership.active, None);
        assert_eq!(membership.expires_at, None);
        assert_eq!(membership.extensions["response"]["message"], "成功");

        let current =
            map_netease_user_membership(None, json!({"code": 200, "data": {"redVipLevel": 0}}))
                .expect("map current account membership");
        assert!(current.user_ref.is_none());
        assert_eq!(current.level, Some(0));
    }

    #[test]
    fn user_membership_rejects_missing_data_and_out_of_range_levels() {
        for response in [
            json!({"code": 200}),
            json!({"code": 200, "data": []}),
            json!({"code": 200, "data": {"redVipLevel": 4294967296_u64}}),
        ] {
            assert_eq!(
                map_netease_user_membership(Some(1), response)
                    .expect_err("malformed user membership")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn catalog_search_variants_match_both_reference_protocols() {
        let mut query = SearchQuery::tracks("周杰伦", 2, 3);
        query.variant = SearchVariant::Legacy;
        for kind in [
            SearchKind::Track,
            SearchKind::Album,
            SearchKind::Artist,
            SearchKind::Playlist,
            SearchKind::User,
            SearchKind::Mv,
            SearchKind::Lyric,
            SearchKind::Podcast,
            SearchKind::Video,
            SearchKind::Mixed,
        ] {
            query.kind = kind;
            let (path, payload, variant) = netease_catalog_search_request(&query, "周杰伦", 2);
            assert_eq!(path, "/api/search/get", "{kind:?}");
            assert_eq!(payload["s"], "周杰伦", "{kind:?}");
            assert_eq!(payload["type"], netease_cloud_search_type(kind), "{kind:?}");
            assert_eq!(payload["limit"], 2, "{kind:?}");
            assert_eq!(payload["offset"], 3, "{kind:?}");
            assert!(payload.get("total").is_none(), "{kind:?}");
            assert_eq!(variant, SearchVariant::Legacy, "{kind:?}");
        }

        query.kind = SearchKind::Voice;
        let (path, payload, variant) = netease_catalog_search_request(&query, "周杰伦", 2);
        assert_eq!(path, "/api/search/voice/get");
        assert_eq!(payload["keyword"], "周杰伦");
        assert_eq!(payload["scene"], "normal");
        assert_eq!(payload["limit"], 2);
        assert_eq!(payload["offset"], 3);
        assert!(payload.get("s").is_none());
        assert!(payload.get("type").is_none());
        assert_eq!(variant, SearchVariant::Legacy);

        query.kind = SearchKind::Podcast;
        query.variant = SearchVariant::Default;
        let (path, payload, variant) = netease_catalog_search_request(&query, "故事", 2);
        assert_eq!(path, "/api/search/voicelist/get");
        assert_eq!(payload["keyword"], "故事");
        assert_eq!(payload["scene"], "normal");
        assert_eq!(payload["limit"], 2);
        assert_eq!(payload["offset"], 3);
        assert_eq!(payload["e_r"], true);
        assert!(payload.get("s").is_none());
        assert!(payload.get("type").is_none());
        assert_eq!(variant, SearchVariant::Default);

        query.variant = SearchVariant::Cloud;
        let (path, payload, variant) = netease_catalog_search_request(&query, "故事", 2);
        assert_eq!(path, "/api/cloudsearch/pc");
        assert_eq!(payload["s"], "故事");
        assert_eq!(payload["type"], 1_009);
        assert_eq!(payload["total"], true);
        assert_eq!(variant, SearchVariant::Cloud);

        query.kind = SearchKind::Voice;
        for requested_variant in [SearchVariant::Default, SearchVariant::Cloud] {
            query.variant = requested_variant;
            let (path, payload, resolved_variant) =
                netease_catalog_search_request(&query, "周杰伦", 2);
            assert_eq!(path, "/api/cloudsearch/pc");
            assert_eq!(payload["s"], "周杰伦");
            assert_eq!(payload["type"], 2_000);
            assert_eq!(payload["total"], true);
            assert_eq!(resolved_variant, SearchVariant::Cloud);
        }
    }

    #[tokio::test]
    async fn catalog_search_does_not_mislabel_podcasts_as_live_radio_stations() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let error = MusicProvider::search_catalog(
            &provider,
            &SearchQuery {
                query: "故事".to_owned(),
                kind: SearchKind::RadioStation,
                variant: SearchVariant::Default,
                limit: 10,
                offset: 0,
                account: None,
            },
        )
        .await
        .expect_err("NetEase has no live broadcast search endpoint");
        assert_eq!(error.code, ErrorCode::CapabilityNotSupported);
        assert_eq!(error.platform, Some(Platform::Netease));
        assert_eq!(error.details["capability"], "search_radio_stations");

        let error = MusicProvider::search_multi_match(
            &provider,
            &SearchMultiMatchRequest {
                query: "故事".to_owned(),
                kind: SearchKind::RadioStation,
                account: None,
            },
        )
        .await
        .expect_err("multi-match must not relabel podcast matches as live broadcasts");
        assert_eq!(error.code, ErrorCode::CapabilityNotSupported);
        assert_eq!(error.platform, Some(Platform::Netease));
        assert_eq!(error.details["capability"], "search_radio_stations");
    }

    #[test]
    fn cloudsearch_maps_every_typed_reference_branch() {
        let song = json!({
            "id": 185809,
            "name": "反方向的钟",
            "artists": [{"id": 6452, "name": "周杰伦"}],
            "album": {"id": 18915, "name": "Jay", "picUrl": "https://example.test/album.jpg"},
            "duration": 258000,
            "status": 0,
            "lyrics": ["穿梭时间的画面的钟"]
        });
        let cases = [
            (
                SearchKind::Track,
                "songs",
                "songCount",
                song.clone(),
                "track",
                "netease:185809",
            ),
            (
                SearchKind::Album,
                "albums",
                "albumCount",
                json!({
                    "id": 18915,
                    "name": "Jay",
                    "artists": [{"id": 6452, "name": "周杰伦"}],
                    "picUrl": "https://example.test/album.jpg",
                    "size": 10
                }),
                "album",
                "netease:18915",
            ),
            (
                SearchKind::Artist,
                "artists",
                "artistCount",
                json!({
                    "id": 6452,
                    "name": "周杰伦",
                    "alias": ["Jay Chou"],
                    "img1v1Url": "https://example.test/artist.jpg"
                }),
                "artist",
                "netease:6452",
            ),
            (
                SearchKind::Playlist,
                "playlists",
                "playlistCount",
                json!({
                    "id": 3778678,
                    "name": "云音乐热歌榜",
                    "creator": {"userId": 1, "nickname": "网易云音乐"},
                    "coverImgUrl": "https://example.test/playlist.jpg",
                    "trackCount": 200
                }),
                "playlist",
                "netease:3778678",
            ),
            (
                SearchKind::User,
                "userprofiles",
                "userprofileCount",
                json!({
                    "userId": 6298206519_u64,
                    "nickname": "轻手揍人丸",
                    "avatarUrl": "https://example.test/avatar.jpg",
                    "followed": false,
                    "mutual": false
                }),
                "user",
                "netease:6298206519",
            ),
            (
                SearchKind::Mv,
                "mvs",
                "mvCount",
                json!({
                    "id": 22695250,
                    "name": "任性",
                    "artistName": "周杰伦",
                    "cover": "https://example.test/mv.jpg",
                    "duration": 266000,
                    "playCount": 100726
                }),
                "video",
                "netease:22695250",
            ),
            (
                SearchKind::Lyric,
                "songs",
                "songCount",
                song,
                "track",
                "netease:185809",
            ),
            (
                SearchKind::Podcast,
                "djRadios",
                "djRadiosCount",
                json!({
                    "id": 336355127,
                    "name": "代码时间",
                    "desc": "面向开发者的播客",
                    "picUrl": "https://example.test/podcast.jpg",
                    "dj": {"userId": 32953014, "nickname": "主播"},
                    "category": "科技",
                    "programCount": 120
                }),
                "podcast",
                "netease:336355127",
            ),
            (
                SearchKind::Video,
                "videos",
                "videoCount",
                json!({
                    "vid": "video-1",
                    "title": "周杰伦现场",
                    "coverUrl": "https://example.test/video.jpg",
                    "durationms": 120000,
                    "playTime": 1000,
                    "creator": [{"userId": 6452, "userName": "周杰伦"}]
                }),
                "video",
                "netease:video-1",
            ),
        ];

        for (kind, item_key, count_key, item, expected_type, expected_ref) in cases {
            let mut body = json!({"code": 200, "result": {}});
            body["result"][item_key] = json!([item]);
            body["result"][count_key] = json!(3);
            let page =
                map_cloud_search_response(kind, 1, 0, body).expect("map typed cloud search branch");
            assert_eq!(page.items.len(), 1, "{kind:?}");
            let value = serde_json::to_value(&page.items[0]).expect("serialize search item");
            assert_eq!(value["type"], expected_type, "{kind:?}");
            assert_eq!(value["data"]["ref"], expected_ref, "{kind:?}");
            assert_eq!(page.pagination.total, Some(3), "{kind:?}");
            assert_eq!(page.pagination.next_offset, Some(1), "{kind:?}");
            assert!(page.pagination.has_more, "{kind:?}");
            assert_eq!(
                page.pagination.extensions["platform_type"],
                netease_cloud_search_type(kind),
                "{kind:?}"
            );
            assert_eq!(page.pagination.extensions["response"]["code"], 200);
        }
    }

    #[test]
    fn dedicated_podcast_search_unwraps_base_info_and_preserves_ranking_metadata() {
        let page = map_cloud_search_response(
            SearchKind::Podcast,
            3,
            0,
            json!({
                "code": 200,
                "data": {
                    "resources": [{
                        "alg": "voice_search",
                        "reason": "标题命中",
                        "baseInfo": {
                            "id": 1212051560,
                            "name": "宝宝巴士-十万个为什么百科故事",
                            "desc": "儿童百科故事",
                            "picUrl": "https://example.test/story.jpg",
                            "dj": {
                                "userId": 32953014,
                                "nickname": "宝宝巴士"
                            },
                            "category": "亲子",
                            "programCount": 88,
                            "subCount": 1234,
                            "playCount": 5678,
                            "radioFeeType": 0
                        }
                    }],
                    "totalCount": 349,
                    "hasMore": true
                }
            }),
        )
        .expect("map dedicated podcast search");

        let SearchItem::Podcast(podcast) = &page.items[0] else {
            panic!("dedicated podcast search must stay typed");
        };
        assert_eq!(podcast.resource_ref.to_string(), "netease:1212051560");
        assert_eq!(podcast.name, "宝宝巴士-十万个为什么百科故事");
        assert_eq!(
            podcast
                .creator
                .as_ref()
                .map(|creator| creator.name.as_str()),
            Some("宝宝巴士")
        );
        assert_eq!(podcast.episode_count, Some(88));
        assert_eq!(podcast.subscriber_count, Some(1234));
        assert_eq!(podcast.play_count, Some(5678));
        assert_eq!(podcast.extensions["search_item"]["alg"], "voice_search");
        assert_eq!(podcast.extensions["search_item"]["reason"], "标题命中");
        assert_eq!(page.pagination.total, Some(349));
        assert_eq!(page.pagination.next_offset, Some(1));
        assert!(page.pagination.has_more);
        assert_eq!(page.pagination.extensions["returned_count"], 1);
        assert_eq!(page.pagination.extensions["response"]["code"], 200);
    }

    #[test]
    fn cloudsearch_preserves_mixed_voice_fallback_and_platform_pagination_behavior() {
        let mixed = map_cloud_search_response(
            SearchKind::Mixed,
            30,
            0,
            json!({
                "code": 200,
                "result": {"order": ["song"], "song": {"more": true}}
            }),
        )
        .expect("map mixed cloud search");
        assert_eq!(mixed.items.len(), 1);
        let SearchItem::Opaque(mixed) = &mixed.items[0] else {
            panic!("mixed result must remain opaque");
        };
        assert_eq!(mixed.kind, "mixed");
        assert_eq!(mixed.extensions["response"]["order"][0], "song");

        let voice = map_cloud_search_response(
            SearchKind::Voice,
            1,
            0,
            json!({
                "code": 200,
                "result": {
                    "voiceCount": 2,
                    "resourceCount": 99,
                    "voices": [{"id": "voice-1", "title": "声音节目"}],
                    "resources": [{"id": "legacy-voice", "title": "旧摘要"}]
                }
            }),
        )
        .expect("map voice cloud search");
        let SearchItem::Opaque(voice_item) = &voice.items[0] else {
            panic!("voice result must remain opaque");
        };
        assert_eq!(voice_item.kind, "voice");
        assert_eq!(voice_item.id.as_deref(), Some("voice-1"));
        assert_eq!(voice.pagination.total, Some(2));
        assert!(voice.pagination.has_more);

        let legacy_voice = map_cloud_search_response(
            SearchKind::Voice,
            1,
            0,
            json!({
                "code": 200,
                "result": {"voices": []},
                "data": {
                    "totalCount": 2,
                    "hasMore": true,
                    "resources": [{"id": "voice-2", "title": "旧版声音节目"}]
                }
            }),
        )
        .expect("map legacy voice search");
        let SearchItem::Opaque(legacy_voice_item) = &legacy_voice.items[0] else {
            panic!("legacy voice result must remain opaque");
        };
        assert_eq!(legacy_voice_item.id.as_deref(), Some("voice-2"));
        assert_eq!(legacy_voice.pagination.total, Some(2));
        assert!(legacy_voice.pagination.has_more);

        let malformed = map_cloud_search_response(
            SearchKind::Album,
            1,
            0,
            json!({
                "code": 200,
                "result": {"albumCount": 1, "albums": [{"unexpected": true}]}
            }),
        )
        .expect("preserve malformed upstream item");
        let SearchItem::Opaque(malformed) = &malformed.items[0] else {
            panic!("unmappable item must remain opaque");
        };
        assert_eq!(malformed.kind, "album");
        assert!(malformed.extensions["mapping_error"].is_string());
    }

    #[test]
    fn cloudsearch_video_prefers_the_richest_nested_resource_and_nonempty_creators() {
        let page = map_cloud_search_response(
            SearchKind::Video,
            1,
            0,
            json!({
                "code": 200,
                "result": {
                    "videoCount": 1,
                    "videos": [{
                        "id": "wrapper-summary",
                        "title": "摘要标题",
                        "data": {},
                        "resource": {
                            "vid": " ",
                            "id": "video-rich",
                            "title": "完整视频",
                            "coverUrl": "https://example.test/rich.jpg",
                            "durationms": 0,
                            "duration": 330230,
                            "creators": [],
                            "artists": [
                                {"userId": 0, "id": 6452, "name": "周杰伦"},
                                {"id": 13193, "name": "五月天"}
                            ],
                            "creator": [{"id": 1, "name": "单人摘要"}]
                        }
                    }]
                }
            }),
        )
        .expect("map richest nested video search resource");

        let SearchItem::Video(video) = &page.items[0] else {
            panic!("video result must remain typed");
        };
        assert_eq!(video.resource_ref.to_string(), "netease:video-rich");
        assert_eq!(video.title, "完整视频");
        assert_eq!(video.duration_ms, Some(330230));
        assert_eq!(video.creators.len(), 2);
        assert_eq!(
            video.creators[0]
                .resource_ref
                .as_ref()
                .expect("creator reference")
                .to_string(),
            "netease:6452"
        );
        assert_eq!(video.creators[1].name, "五月天");
    }

    #[test]
    fn cloudsearch_type_codes_and_capabilities_cover_the_complete_reference_enum() {
        let cases = [
            (SearchKind::Track, 1, Capability::SearchTracks),
            (SearchKind::Album, 10, Capability::SearchAlbums),
            (SearchKind::Artist, 100, Capability::SearchArtists),
            (SearchKind::Playlist, 1_000, Capability::SearchPlaylists),
            (SearchKind::User, 1_002, Capability::SearchUsers),
            (SearchKind::Mv, 1_004, Capability::SearchMvs),
            (SearchKind::Lyric, 1_006, Capability::SearchLyrics),
            (
                SearchKind::RadioStation,
                1_009,
                Capability::SearchRadioStations,
            ),
            (SearchKind::Podcast, 1_009, Capability::SearchPodcasts),
            (SearchKind::Video, 1_014, Capability::SearchVideos),
            (SearchKind::Mixed, 1_018, Capability::SearchMixed),
            (SearchKind::Voice, 2_000, Capability::SearchVoices),
        ];
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let capabilities = provider.capabilities();
        for (kind, platform_type, capability) in cases {
            assert_eq!(netease_cloud_search_type(kind), platform_type);
            assert_eq!(capability_for_search(kind), capability);
            if kind == SearchKind::RadioStation {
                assert!(!capabilities.contains(&capability), "{capability:?}");
            } else {
                assert!(capabilities.contains(&capability), "{capability:?}");
            }
        }
        assert!(capabilities.contains(&Capability::SearchDefault));
        assert!(capabilities.contains(&Capability::SearchTrending));
        assert!(capabilities.contains(&Capability::SearchSuggestions));
        assert!(capabilities.contains(&Capability::SearchMultiMatch));
        assert!(capabilities.contains(&Capability::SearchLocalTrackMatch));
        assert!(capabilities.contains(&Capability::UserMembership));
    }

    #[test]
    fn comment_writes_map_every_reference_resource_type_and_action() {
        let cases = [
            (CommentTargetKind::Track, "185809", "R_SO_4_185809"),
            (CommentTargetKind::Mv, "5436712", "R_MV_5_5436712"),
            (CommentTargetKind::Playlist, "705123491", "A_PL_0_705123491"),
            (CommentTargetKind::Album, "32311", "R_AL_3_32311"),
            (
                CommentTargetKind::RadioEpisode,
                "794062371",
                "A_DJ_1_794062371",
            ),
            (
                CommentTargetKind::Video,
                "89ADDE33C0AAE8EC14B99F6750DB954D",
                "R_VI_62_89ADDE33C0AAE8EC14B99F6750DB954D",
            ),
            (
                CommentTargetKind::Event,
                "A_EV_2_6559519868_32953014",
                "A_EV_2_6559519868_32953014",
            ),
            (CommentTargetKind::RadioStation, "362", "A_DR_14_362"),
        ];
        for (kind, id, thread_id) in cases {
            let target = CommentTarget::new(
                ResourceRef::new(Platform::Netease, id).expect("valid comment target"),
                kind,
            );
            let create = CommentWriteRequest {
                target: target.clone(),
                content: "  保留内容空格  ".to_owned(),
                reply_to: None,
                account: Some("personal".to_owned()),
            };
            let (path, payload, action) =
                netease_comment_write_request(&create).expect("build create request");
            assert_eq!(path, "/api/resource/comments/add", "{kind:?}");
            assert_eq!(action, CommentMutationAction::Create, "{kind:?}");
            assert_eq!(payload["threadId"], thread_id, "{kind:?}");
            assert_eq!(payload["content"], "  保留内容空格  ", "{kind:?}");
            assert!(payload.get("commentId").is_none(), "{kind:?}");

            let reply = CommentWriteRequest {
                reply_to: Some("1438569889".to_owned()),
                ..create
            };
            let (path, payload, action) =
                netease_comment_write_request(&reply).expect("build reply request");
            assert_eq!(path, "/api/resource/comments/reply", "{kind:?}");
            assert_eq!(action, CommentMutationAction::Reply, "{kind:?}");
            assert_eq!(payload["threadId"], thread_id, "{kind:?}");
            assert_eq!(payload["commentId"], "1438569889", "{kind:?}");

            let delete = CommentDeleteRequest {
                target,
                comment_id: "1535550516319".to_owned(),
                account: Some("personal".to_owned()),
            };
            let (path, payload, comment_id) =
                netease_comment_delete_request(&delete).expect("build delete request");
            assert_eq!(path, "/api/resource/comments/delete", "{kind:?}");
            assert_eq!(payload["threadId"], thread_id, "{kind:?}");
            assert_eq!(payload["commentId"], "1535550516319", "{kind:?}");
            assert_eq!(comment_id, "1535550516319", "{kind:?}");
        }
    }

    #[test]
    fn comment_writes_validate_targets_fields_and_preserve_results() {
        let target = CommentTarget::new(
            ResourceRef::new(Platform::Netease, "185809").expect("valid comment target"),
            CommentTargetKind::Track,
        );
        let result = map_comment_mutation_result(
            &target,
            CommentMutationAction::Create,
            None,
            json!({"code": 200, "comment": {"commentId": 1535550516319_u64}}),
        )
        .expect("map comment result");
        assert_eq!(result.comment_id.as_deref(), Some("1535550516319"));
        assert_eq!(result.action, CommentMutationAction::Create);
        assert_eq!(result.extensions["response"]["code"], 200);

        let fallback_result = map_comment_mutation_result(
            &target,
            CommentMutationAction::Create,
            None,
            json!({
                "code": 200,
                "comment": {"commentId": 0},
                "data": {"commentId": "1535550516320"}
            }),
        )
        .expect("skip unusable primary comment id");
        assert_eq!(fallback_result.comment_id.as_deref(), Some("1535550516320"));

        let invalid_platform = CommentWriteRequest {
            target: CommentTarget::new(
                ResourceRef::new(Platform::Qq, "185809").expect("valid QQ reference"),
                CommentTargetKind::Track,
            ),
            content: "test".to_owned(),
            reply_to: None,
            account: None,
        };
        assert_eq!(
            netease_comment_write_request(&invalid_platform)
                .expect_err("foreign target")
                .code,
            ErrorCode::InvalidRequest
        );

        let invalid_event = CommentWriteRequest {
            target: CommentTarget::new(
                ResourceRef::new(Platform::Netease, "6559519868").expect("valid reference"),
                CommentTargetKind::Event,
            ),
            content: "test".to_owned(),
            reply_to: None,
            account: None,
        };
        assert_eq!(
            netease_comment_write_request(&invalid_event)
                .expect_err("incomplete event thread")
                .code,
            ErrorCode::InvalidRequest
        );

        for (content, reply_to) in [("", None), ("test", Some("  "))] {
            let invalid = CommentWriteRequest {
                target: target.clone(),
                content: content.to_owned(),
                reply_to: reply_to.map(str::to_owned),
                account: None,
            };
            assert_eq!(
                netease_comment_write_request(&invalid)
                    .expect_err("invalid comment field")
                    .code,
                ErrorCode::InvalidRequest
            );
        }
        let invalid_delete = CommentDeleteRequest {
            target,
            comment_id: " ".to_owned(),
            account: None,
        };
        assert_eq!(
            netease_comment_delete_request(&invalid_delete)
                .expect_err("empty comment id")
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[tokio::test]
    async fn comment_writes_require_authentication_before_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let target = CommentTarget::new(
            ResourceRef::new(Platform::Netease, "185809").expect("valid comment target"),
            CommentTargetKind::Track,
        );
        let create = MusicProvider::post_comment(
            &provider,
            &CommentWriteRequest {
                target: target.clone(),
                content: "test".to_owned(),
                reply_to: None,
                account: None,
            },
        )
        .await
        .expect_err("anonymous create must fail");
        assert_eq!(create.code, ErrorCode::AuthenticationRequired);

        let delete = MusicProvider::delete_comment(
            &provider,
            &CommentDeleteRequest {
                target,
                comment_id: "1535550516319".to_owned(),
                account: None,
            },
        )
        .await
        .expect_err("anonymous delete must fail");
        assert_eq!(delete.code, ErrorCode::AuthenticationRequired);
    }

    #[test]
    fn comment_hug_list_requests_preserve_every_resource_prefix_and_reference_parameter() {
        let cases = [
            (CommentTargetKind::Track, "863481066", "R_SO_4_863481066"),
            (CommentTargetKind::Mv, "5436712", "R_MV_5_5436712"),
            (CommentTargetKind::Playlist, "705123491", "A_PL_0_705123491"),
            (CommentTargetKind::Album, "32311", "R_AL_3_32311"),
            (
                CommentTargetKind::RadioEpisode,
                "794062371",
                "A_DJ_1_794062371",
            ),
            (
                CommentTargetKind::Video,
                "89ADDE33C0AAE8EC14B99F6750DB954D",
                "R_VI_62_89ADDE33C0AAE8EC14B99F6750DB954D",
            ),
            (
                CommentTargetKind::Event,
                "A_EV_2_6559519868_32953014",
                "A_EV_2_6559519868_32953014",
            ),
            (CommentTargetKind::RadioStation, "362", "A_DR_14_362"),
        ];
        for (kind, id, thread_id) in cases {
            let mut request = CommentReactionListRequest::new(
                CommentTarget::new(
                    ResourceRef::new(Platform::Netease, id).expect("valid comment target"),
                    kind,
                ),
                "1167145843".to_owned(),
                ResourceRef::new(Platform::Netease, "285516405").expect("valid target user"),
                CommentReactionKind::Hug,
                2,
            );
            request.page = 3;
            request.cursor = Some("04-八月-2020 17:46:25:000".to_owned());
            request.id_cursor = Some("362576849".to_owned());
            let (path, payload) = netease_comment_reaction_list_request(&request)
                .expect("build comment hug list request");
            assert_eq!(path, "/api/v2/resource/comments/hug/list", "{kind:?}");
            assert_eq!(payload["targetUserId"], "285516405", "{kind:?}");
            assert_eq!(payload["commentId"], "1167145843", "{kind:?}");
            assert_eq!(payload["threadId"], thread_id, "{kind:?}");
            assert_eq!(payload["pageNo"], 3, "{kind:?}");
            assert_eq!(payload["pageSize"], 2, "{kind:?}");
            assert_eq!(payload["cursor"], "04-八月-2020 17:46:25:000", "{kind:?}");
            assert_eq!(payload["idCursor"], "362576849", "{kind:?}");
        }

        let request = CommentReactionListRequest::new(
            CommentTarget::new(
                ResourceRef::new(Platform::Netease, "863481066").expect("valid comment target"),
                CommentTargetKind::Track,
            ),
            "1167145843".to_owned(),
            ResourceRef::new(Platform::Netease, "285516405").expect("valid target user"),
            CommentReactionKind::Hug,
            100,
        );
        let (_, payload) =
            netease_comment_reaction_list_request(&request).expect("build default request");
        assert_eq!(payload["pageNo"], 1);
        assert_eq!(payload["pageSize"], 100);
        assert_eq!(payload["cursor"], "-1");
        assert_eq!(payload["idCursor"], "-1");
    }

    #[test]
    fn comment_hug_list_requests_reject_unsupported_or_invalid_inputs() {
        let base = CommentReactionListRequest::new(
            CommentTarget::new(
                ResourceRef::new(Platform::Netease, "863481066").expect("valid comment target"),
                CommentTargetKind::Track,
            ),
            "1167145843".to_owned(),
            ResourceRef::new(Platform::Netease, "285516405").expect("valid target user"),
            CommentReactionKind::Hug,
            100,
        );
        let mut cases = Vec::new();
        cases.push(CommentReactionListRequest {
            kind: CommentReactionKind::Like,
            ..base.clone()
        });
        cases.push(CommentReactionListRequest {
            target_user_ref: ResourceRef::new(Platform::Qq, "285516405")
                .expect("valid foreign user"),
            ..base.clone()
        });
        cases.push(CommentReactionListRequest {
            limit: 0,
            ..base.clone()
        });
        cases.push(CommentReactionListRequest {
            limit: 101,
            ..base.clone()
        });
        cases.push(CommentReactionListRequest {
            page: 0,
            ..base.clone()
        });
        cases.push(CommentReactionListRequest {
            comment_id: " ".to_owned(),
            ..base.clone()
        });
        cases.push(CommentReactionListRequest {
            cursor: Some(" ".to_owned()),
            ..base.clone()
        });
        cases.push(CommentReactionListRequest {
            id_cursor: Some(" ".to_owned()),
            ..base
        });

        for request in cases {
            assert_eq!(
                netease_comment_reaction_list_request(&request)
                    .expect_err("invalid reaction request")
                    .code,
                ErrorCode::InvalidRequest
            );
        }
    }

    #[test]
    fn maps_comment_hug_lists_with_users_current_comment_totals_and_dual_cursors() {
        let mut request = CommentReactionListRequest::new(
            CommentTarget::new(
                ResourceRef::new(Platform::Netease, "863481066").expect("valid comment target"),
                CommentTargetKind::Track,
            ),
            "1167145843".to_owned(),
            ResourceRef::new(Platform::Netease, "285516405").expect("valid target user"),
            CommentReactionKind::Hug,
            2,
        );
        request.page = 2;
        request.cursor = Some("previous-cursor".to_owned());
        request.id_cursor = Some("100".to_owned());
        let response = json!({
            "code": 200,
            "data": {
                "code": 200,
                "data": {
                    "currentComment": fixture_comment(1_167_145_843, "原评论"),
                    "hugComments": [
                        {
                            "user": {
                                "userId": 2_121_989_064_u64,
                                "nickname": "清梦初仄",
                                "avatarUrl": "https://example.test/hugger.jpg",
                                "followed": false,
                                "isHug": true
                            },
                            "hugContent": "给了 Puddin_of_Harley_Quinn 一个抱抱"
                        },
                        {
                            "user": {"userId": 1_598_024_192_u64, "nickname": "李一窝_"},
                            "hugContent": "第二个抱抱",
                            "futureField": true
                        }
                    ],
                    "hasMore": true,
                    "cursor": "04-八月-2020 17:46:25:000",
                    "idCursor": 362_576_849,
                    "hugTotalCounts": 150
                }
            },
            "message": ""
        });
        let page =
            map_netease_comment_reaction_page(&request, response).expect("map comment hug list");
        assert_eq!(page.kind, CommentReactionKind::Hug);
        assert_eq!(page.target.resource_ref.to_string(), "netease:863481066");
        assert_eq!(page.comment_id, "1167145843");
        assert_eq!(page.target_user_ref.to_string(), "netease:285516405");
        assert_eq!(page.reactions.len(), 2);
        assert_eq!(page.reactions[0].user.id, "2121989064");
        assert_eq!(page.reactions[0].user.name, "清梦初仄");
        assert_eq!(
            page.reactions[0].content.as_deref(),
            Some("给了 Puddin_of_Harley_Quinn 一个抱抱")
        );
        assert_eq!(
            page.reactions[1].extensions["response"]["futureField"],
            true
        );
        assert_eq!(
            page.current_comment
                .as_ref()
                .map(|comment| comment.id.as_str()),
            Some("1167145843")
        );
        assert_eq!(page.pagination.limit, 2);
        assert_eq!(page.pagination.offset, 2);
        assert_eq!(page.pagination.total, Some(150));
        assert_eq!(page.pagination.next_offset, Some(4));
        assert!(page.pagination.has_more);
        assert_eq!(page.pagination.extensions["mode"], "reaction_hug");
        assert_eq!(
            page.pagination.extensions["next_cursor"],
            "04-八月-2020 17:46:25:000"
        );
        assert_eq!(page.pagination.extensions["next_id_cursor"], "362576849");
        assert_eq!(
            page.pagination.extensions["requested_cursor"],
            "previous-cursor"
        );
        assert_eq!(page.extensions["response"]["code"], 200);
    }

    #[test]
    fn comment_hug_list_mapping_rejects_missing_arrays_and_users() {
        let request = CommentReactionListRequest::new(
            CommentTarget::new(
                ResourceRef::new(Platform::Netease, "863481066").expect("valid comment target"),
                CommentTargetKind::Track,
            ),
            "1167145843".to_owned(),
            ResourceRef::new(Platform::Netease, "285516405").expect("valid target user"),
            CommentReactionKind::Hug,
            2,
        );
        for response in [
            json!({"code": 200, "data": {"hasMore": false}}),
            json!({"code": 200, "data": {"hugComments": [{"hugContent": "无用户"}]}}),
        ] {
            assert_eq!(
                map_netease_comment_reaction_page(&request, response)
                    .expect_err("malformed hug response")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[tokio::test]
    async fn comment_hug_lists_require_authentication_before_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let request = CommentReactionListRequest::new(
            CommentTarget::new(
                ResourceRef::new(Platform::Netease, "863481066").expect("valid comment target"),
                CommentTargetKind::Track,
            ),
            "1167145843".to_owned(),
            ResourceRef::new(Platform::Netease, "285516405").expect("valid target user"),
            CommentReactionKind::Hug,
            2,
        );
        let error = MusicProvider::comment_reactions(&provider, &request)
            .await
            .expect_err("anonymous hug list must fail before network access");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[test]
    fn comment_like_requests_cover_every_resource_and_both_action_branches() {
        let cases = [
            (CommentTargetKind::Track, "29178366", "R_SO_4_29178366"),
            (CommentTargetKind::Mv, "5436712", "R_MV_5_5436712"),
            (CommentTargetKind::Playlist, "705123491", "A_PL_0_705123491"),
            (CommentTargetKind::Album, "32311", "R_AL_3_32311"),
            (
                CommentTargetKind::RadioEpisode,
                "794062371",
                "A_DJ_1_794062371",
            ),
            (
                CommentTargetKind::Video,
                "89ADDE33C0AAE8EC14B99F6750DB954D",
                "R_VI_62_89ADDE33C0AAE8EC14B99F6750DB954D",
            ),
            (
                CommentTargetKind::Event,
                "A_EV_2_6559519868_32953014",
                "A_EV_2_6559519868_32953014",
            ),
            (CommentTargetKind::RadioStation, "362", "A_DR_14_362"),
        ];
        for (kind, id, thread_id) in cases {
            for (active, expected_path) in [
                (true, "/api/v1/comment/like"),
                (false, "/api/v1/comment/unlike"),
            ] {
                let request = CommentReactionMutationRequest {
                    target: CommentTarget::new(
                        ResourceRef::new(Platform::Netease, id).expect("valid reaction target"),
                        kind,
                    ),
                    comment_id: "12840183".to_owned(),
                    kind: CommentReactionKind::Like,
                    active,
                    target_user_ref: None,
                    account: Some("personal".to_owned()),
                };
                let (path, payload) = netease_comment_reaction_mutation_request(&request)
                    .expect("build comment like request");
                assert_eq!(path, expected_path, "{kind:?} {active}");
                assert_eq!(payload["threadId"], thread_id, "{kind:?} {active}");
                assert_eq!(payload["commentId"], "12840183", "{kind:?} {active}");

                let result = map_netease_comment_reaction_mutation(
                    &request,
                    json!({"code": 200, "data": {}}),
                )
                .expect("map comment like result");
                assert_eq!(result.target, request.target);
                assert_eq!(result.comment_id, "12840183");
                assert_eq!(result.kind, CommentReactionKind::Like);
                assert_eq!(result.active, active);
                assert_eq!(result.extensions["response"]["code"], 200);
            }
        }
    }

    #[test]
    fn comment_like_requests_reject_wrong_reactions_users_fields_and_targets() {
        let base = CommentReactionMutationRequest {
            target: CommentTarget::new(
                ResourceRef::new(Platform::Netease, "29178366").expect("valid reaction target"),
                CommentTargetKind::Track,
            ),
            comment_id: "12840183".to_owned(),
            kind: CommentReactionKind::Like,
            active: true,
            target_user_ref: None,
            account: None,
        };
        let cases = [
            CommentReactionMutationRequest {
                kind: CommentReactionKind::Hug,
                ..base.clone()
            },
            CommentReactionMutationRequest {
                target_user_ref: Some(
                    ResourceRef::new(Platform::Netease, "285516405").expect("valid target user"),
                ),
                ..base.clone()
            },
            CommentReactionMutationRequest {
                comment_id: " ".to_owned(),
                ..base.clone()
            },
            CommentReactionMutationRequest {
                target: CommentTarget::new(
                    ResourceRef::new(Platform::Qq, "29178366").expect("valid foreign target"),
                    CommentTargetKind::Track,
                ),
                ..base.clone()
            },
            CommentReactionMutationRequest {
                target: CommentTarget::new(
                    ResourceRef::new(Platform::Netease, "6559519868")
                        .expect("valid incomplete event target"),
                    CommentTargetKind::Event,
                ),
                ..base
            },
        ];
        for request in cases {
            assert_eq!(
                netease_comment_reaction_mutation_request(&request)
                    .expect_err("invalid like request")
                    .code,
                ErrorCode::InvalidRequest
            );
        }
    }

    #[tokio::test]
    async fn comment_likes_require_authentication_before_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for active in [true, false] {
            let request = CommentReactionMutationRequest {
                target: CommentTarget::new(
                    ResourceRef::new(Platform::Netease, "29178366").expect("valid reaction target"),
                    CommentTargetKind::Track,
                ),
                comment_id: "12840183".to_owned(),
                kind: CommentReactionKind::Like,
                active,
                target_user_ref: None,
                account: None,
            };
            let error = MusicProvider::set_comment_reaction(&provider, &request)
                .await
                .expect_err("anonymous comment reaction must fail before network access");
            assert_eq!(error.code, ErrorCode::AuthenticationRequired);
        }
    }

    #[test]
    fn comment_report_request_matches_reference_song_only_eapi_payload() {
        let request = CommentReportRequest {
            target: CommentTarget::new(
                ResourceRef::new(Platform::Netease, "2058263032").expect("valid report target"),
                CommentTargetKind::Track,
            ),
            comment_id: "123456789".to_owned(),
            reason: "人身攻击".to_owned(),
            account: Some("personal".to_owned()),
        };
        let (path, payload) =
            netease_comment_report_request(&request).expect("build comment report request");
        assert_eq!(path, "/api/report/reportcomment");
        assert_eq!(payload["threadId"], "R_SO_4_2058263032");
        assert_eq!(payload["commentId"], "123456789");
        assert_eq!(payload["reason"], "人身攻击");

        let result = map_netease_comment_report(&request, json!({"code": 200}))
            .expect("map comment report response");
        assert_eq!(result.target, request.target);
        assert_eq!(result.comment_id, "123456789");
        assert_eq!(result.reason, "人身攻击");
        assert!(result.submitted);
        assert_eq!(result.extensions["response"]["code"], 200);
    }

    #[test]
    fn comment_report_rejects_non_track_foreign_and_empty_fields() {
        let base = CommentReportRequest {
            target: CommentTarget::new(
                ResourceRef::new(Platform::Netease, "2058263032").expect("valid report target"),
                CommentTargetKind::Track,
            ),
            comment_id: "123456789".to_owned(),
            reason: "人身攻击".to_owned(),
            account: None,
        };
        let cases = [
            CommentReportRequest {
                target: CommentTarget::new(
                    ResourceRef::new(Platform::Netease, "705123491")
                        .expect("valid playlist reference"),
                    CommentTargetKind::Playlist,
                ),
                ..base.clone()
            },
            CommentReportRequest {
                target: CommentTarget::new(
                    ResourceRef::new(Platform::Qq, "2058263032").expect("valid foreign reference"),
                    CommentTargetKind::Track,
                ),
                ..base.clone()
            },
            CommentReportRequest {
                comment_id: " ".to_owned(),
                ..base.clone()
            },
            CommentReportRequest {
                reason: " \t".to_owned(),
                ..base
            },
        ];
        for request in cases {
            assert_eq!(
                netease_comment_report_request(&request)
                    .expect_err("invalid comment report")
                    .code,
                ErrorCode::InvalidRequest
            );
        }
    }

    #[tokio::test]
    async fn comment_reports_require_authentication_before_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let request = CommentReportRequest {
            target: CommentTarget::new(
                ResourceRef::new(Platform::Netease, "2058263032").expect("valid report target"),
                CommentTargetKind::Track,
            ),
            comment_id: "123456789".to_owned(),
            reason: "人身攻击".to_owned(),
            account: None,
        };
        let error = MusicProvider::report_comment(&provider, &request)
            .await
            .expect_err("anonymous comment report must fail before network access");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[test]
    fn comment_thread_stats_requests_cover_all_internal_types_and_empty_batches() {
        let cases = [
            (CommentTargetKind::Track, "185809", "4"),
            (CommentTargetKind::Mv, "5436712", "5"),
            (CommentTargetKind::Playlist, "705123491", "0"),
            (CommentTargetKind::Album, "32311", "3"),
            (CommentTargetKind::RadioEpisode, "794062371", "1"),
            (
                CommentTargetKind::Video,
                "89ADDE33C0AAE8EC14B99F6750DB954D",
                "62",
            ),
            (CommentTargetKind::Event, "6559519868", "2"),
            (CommentTargetKind::RadioStation, "362", "14"),
        ];
        for (kind, id, resource_type) in cases {
            let request = CommentThreadStatsRequest {
                kind,
                resource_refs: vec![
                    ResourceRef::new(Platform::Netease, id).expect("valid stats resource"),
                ],
                account: Some("personal".to_owned()),
            };
            let (path, payload) = netease_comment_thread_stats_request(&request)
                .expect("build comment stats request");
            assert_eq!(path, "/api/resource/commentInfo/list", "{kind:?}");
            assert_eq!(payload["resourceType"], resource_type, "{kind:?}");
            assert_eq!(payload["resourceIds"], format!("[\"{id}\"]"), "{kind:?}");
        }

        let empty = CommentThreadStatsRequest {
            kind: CommentTargetKind::Track,
            resource_refs: Vec::new(),
            account: None,
        };
        let (_, payload) =
            netease_comment_thread_stats_request(&empty).expect("build empty stats request");
        assert_eq!(payload["resourceIds"], "[]");

        let foreign = CommentThreadStatsRequest {
            kind: CommentTargetKind::Track,
            resource_refs: vec![
                ResourceRef::new(Platform::Qq, "185809").expect("valid foreign resource"),
            ],
            account: None,
        };
        assert_eq!(
            netease_comment_thread_stats_request(&foreign)
                .expect_err("foreign stats request")
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn maps_comment_thread_stats_counts_users_comments_and_canonical_video_ids() {
        let requested = ResourceRef::new(Platform::Netease, "89ADDE33C0AAE8EC14B99F6750DB954D")
            .expect("valid requested video");
        let request = CommentThreadStatsRequest {
            kind: CommentTargetKind::Video,
            resource_refs: vec![requested.clone()],
            account: None,
        };
        let batch = map_netease_comment_thread_stats(
            &request,
            json!({
                "code": 200,
                "data": [{
                    "latestLikedUsers": [{
                        "userId": 2121989064_u64,
                        "nickname": "清梦初仄",
                        "avatarUrl": "https://example.test/avatar.jpg",
                        "followed": false
                    }],
                    "liked": false,
                    "comments": [fixture_comment(3160990055, "最近评论")],
                    "resourceType": 62,
                    "resourceId": 2335163,
                    "commentUpgraded": false,
                    "musicianSaidCount": 1,
                    "commentCountDesc": "1000+",
                    "likedCount": 36,
                    "commentCount": 1123,
                    "shareCount": 27153,
                    "threadId": "R_VI_62_2335163",
                    "futureField": {"kept": true}
                }]
            }),
        )
        .expect("map comment stats");
        assert_eq!(batch.kind, CommentTargetKind::Video);
        assert_eq!(batch.requested_refs, vec![requested.clone()]);
        assert_eq!(batch.stats.len(), 1);
        let stats = &batch.stats[0];
        assert_eq!(stats.requested_ref.as_ref(), Some(&requested));
        assert_eq!(stats.target.resource_ref.to_string(), "netease:2335163");
        assert_eq!(stats.target.kind, CommentTargetKind::Video);
        assert_eq!(stats.liked, Some(false));
        assert_eq!(stats.like_count, Some(36));
        assert_eq!(stats.comment_count, Some(1123));
        assert_eq!(stats.comment_count_text.as_deref(), Some("1000+"));
        assert_eq!(stats.share_count, Some(27153));
        assert_eq!(stats.comment_upgraded, Some(false));
        assert_eq!(stats.musician_comment_count, Some(1));
        assert_eq!(stats.latest_liked_users[0].id, "2121989064");
        assert_eq!(stats.comments[0].id, "3160990055");
        assert_eq!(stats.extensions["response"]["futureField"]["kept"], true);
        assert_eq!(batch.extensions["resource_type"], "62");
        assert_eq!(batch.extensions["response"]["code"], 200);
    }

    #[test]
    fn maps_event_stats_to_complete_threads_and_preserves_empty_batches() {
        let request = CommentThreadStatsRequest {
            kind: CommentTargetKind::Event,
            resource_refs: vec![
                ResourceRef::new(Platform::Netease, "6559519868").expect("valid event resource"),
            ],
            account: None,
        };
        let batch = map_netease_comment_thread_stats(
            &request,
            json!({
                "code": 200,
                "data": [{
                    "resourceType": 2,
                    "resourceId": 6559519868_u64,
                    "commentCount": 0,
                    "threadId": "A_EV_2_6559519868_0"
                }]
            }),
        )
        .expect("map event stats");
        assert_eq!(
            batch.stats[0].target.resource_ref.to_string(),
            "netease:A_EV_2_6559519868_0"
        );
        assert_eq!(batch.stats[0].comment_count, Some(0));

        let empty_request = CommentThreadStatsRequest {
            kind: CommentTargetKind::Track,
            resource_refs: Vec::new(),
            account: None,
        };
        let empty =
            map_netease_comment_thread_stats(&empty_request, json!({"code": 200, "data": []}))
                .expect("map empty stats");
        assert!(empty.stats.is_empty());
        assert!(empty.requested_refs.is_empty());
    }

    #[test]
    fn comment_thread_stats_reject_malformed_arrays_threads_and_users() {
        let request = CommentThreadStatsRequest {
            kind: CommentTargetKind::Track,
            resource_refs: vec![
                ResourceRef::new(Platform::Netease, "185809").expect("valid resource"),
            ],
            account: None,
        };
        for response in [
            json!({"code": 200}),
            json!({"code": 200, "data": [{"threadId": "R_MV_5_185809"}]}),
            json!({
                "code": 200,
                "data": [{"threadId": "R_SO_4_185809", "latestLikedUsers": [{}]}]
            }),
        ] {
            assert_eq!(
                map_netease_comment_thread_stats(&request, response)
                    .expect_err("malformed stats response")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_comment_thread_stats_cover_every_reference_resource_type() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let cases = [
            (CommentTargetKind::Track, "185809"),
            (CommentTargetKind::Mv, "5436712"),
            (CommentTargetKind::Playlist, "705123491"),
            (CommentTargetKind::Album, "32311"),
            (CommentTargetKind::RadioEpisode, "794062371"),
            (CommentTargetKind::Video, "89ADDE33C0AAE8EC14B99F6750DB954D"),
            (CommentTargetKind::Event, "6559519868"),
            (CommentTargetKind::RadioStation, "362"),
        ];
        for (kind, id) in cases {
            let request = CommentThreadStatsRequest {
                kind,
                resource_refs: vec![
                    ResourceRef::new(Platform::Netease, id).expect("valid live resource"),
                ],
                account: None,
            };
            let batch = MusicProvider::comment_thread_stats(&provider, &request)
                .await
                .unwrap_or_else(|error| panic!("{kind:?} stats failed: {error}"));
            assert_eq!(batch.extensions["response"]["code"], 200, "{kind:?}");
            assert_eq!(batch.stats.len(), 1, "{kind:?}");
            assert_eq!(batch.stats[0].target.kind, kind, "{kind:?}");
        }
    }

    fn fixture_comment(id: u64, content: &str) -> Value {
        json!({
            "commentId": id,
            "content": content,
            "time": 1_582_035_919_432_u64,
            "timeStr": "2020-02-18",
            "liked": false,
            "likedCount": 5_646,
            "parentCommentId": 0,
            "replyCount": 2,
            "ipLocation": {"location": "上海"},
            "user": {
                "userId": 278_612_322,
                "nickname": "阿良0321",
                "avatarUrl": "https://example.test/avatar.jpg",
                "followed": false,
                "mutual": false
            },
            "beReplied": [{
                "beRepliedCommentId": 100,
                "content": "原评论",
                "user": {"userId": 200, "nickname": "被回复者"}
            }],
            "richContent": "保留平台富文本"
        })
    }

    #[test]
    fn comment_list_requests_cover_every_resource_and_public_view_protocol() {
        let cases = [
            (CommentTargetKind::Track, "185809", "R_SO_4_185809", true),
            (CommentTargetKind::Mv, "5436712", "R_MV_5_5436712", true),
            (
                CommentTargetKind::Playlist,
                "705123491",
                "A_PL_0_705123491",
                true,
            ),
            (CommentTargetKind::Album, "32311", "R_AL_3_32311", true),
            (
                CommentTargetKind::RadioEpisode,
                "794062371",
                "A_DJ_1_794062371",
                true,
            ),
            (
                CommentTargetKind::Video,
                "89ADDE33C0AAE8EC14B99F6750DB954D",
                "R_VI_62_89ADDE33C0AAE8EC14B99F6750DB954D",
                true,
            ),
            (
                CommentTargetKind::Event,
                "A_EV_2_6559519868_32953014",
                "A_EV_2_6559519868_32953014",
                false,
            ),
            (CommentTargetKind::RadioStation, "362", "A_DR_14_362", true),
        ];
        for (kind, id, thread_id, has_rid) in cases {
            let target = CommentTarget::new(
                ResourceRef::new(Platform::Netease, id).expect("valid comment target"),
                kind,
            );
            let mut request = CommentListRequest::new(target, 20);
            request.offset = 40;
            request.before_time_ms = Some(1_600_000_000_000);
            let (path, payload, mode) =
                netease_comment_list_request(&request).expect("build legacy comments request");
            assert_eq!(mode, NeteaseCommentListMode::Legacy, "{kind:?}");
            assert_eq!(
                path,
                format!("/api/v1/resource/comments/{thread_id}"),
                "{kind:?}"
            );
            assert_eq!(payload["limit"], 20, "{kind:?}");
            assert_eq!(payload["offset"], 40, "{kind:?}");
            assert_eq!(payload["beforeTime"], 1_600_000_000_000_u64, "{kind:?}");
            assert_eq!(payload.get("rid").is_some(), has_rid, "{kind:?}");
            if has_rid {
                assert_eq!(payload["rid"], id, "{kind:?}");
            }
        }

        let target = CommentTarget::new(
            ResourceRef::new(Platform::Netease, "185809").expect("valid track target"),
            CommentTargetKind::Track,
        );
        for (sort, expected_type, expected_cursor) in [
            (CommentSort::Recommended, 99, json!(20)),
            (CommentSort::Hot, 2, json!("normalHot#20")),
            (CommentSort::Time, 3, json!("1582035919432")),
        ] {
            let mut request = CommentListRequest::new(target.clone(), 20);
            request.sort = Some(sort);
            request.page = Some(2);
            request.include_replies = false;
            if sort == CommentSort::Time {
                request.cursor = Some("1582035919432".to_owned());
            }
            let (path, payload, mode) =
                netease_comment_list_request(&request).expect("build modern comments request");
            assert_eq!(path, "/api/v2/resource/comments");
            assert_eq!(mode, NeteaseCommentListMode::Modern);
            assert_eq!(payload["threadId"], "R_SO_4_185809");
            assert_eq!(payload["pageNo"], 2);
            assert_eq!(payload["pageSize"], 20);
            assert_eq!(payload["showInner"], false);
            assert_eq!(payload["sortType"], expected_type);
            assert_eq!(payload["cursor"], expected_cursor);
        }

        let mut hot = CommentListRequest::new(target.clone(), 5);
        hot.view = CommentListView::Hot;
        hot.offset = 10;
        let (path, payload, mode) =
            netease_comment_list_request(&hot).expect("build hot comments request");
        assert_eq!(path, "/api/v1/resource/hotcomments/R_SO_4_185809");
        assert_eq!(payload["rid"], "185809");
        assert_eq!(mode, NeteaseCommentListMode::Hot);

        let mut replies = CommentListRequest::new(target, 10);
        replies.view = CommentListView::Replies;
        replies.parent_comment_id = Some("3160990055".to_owned());
        replies.before_time_ms = Some(1_582_035_919_432);
        replies.offset = 20;
        let (path, payload, mode) =
            netease_comment_list_request(&replies).expect("build floor comments request");
        assert_eq!(path, "/api/resource/comment/floor/get");
        assert_eq!(payload["parentCommentId"], "3160990055");
        assert_eq!(payload["threadId"], "R_SO_4_185809");
        assert_eq!(payload["time"], 1_582_035_919_432_i64);
        assert!(payload.get("offset").is_none());
        assert_eq!(mode, NeteaseCommentListMode::Floor);
    }

    #[test]
    fn comment_list_requests_reject_conflicting_and_missing_fields() {
        let target = CommentTarget::new(
            ResourceRef::new(Platform::Netease, "185809").expect("valid track target"),
            CommentTargetKind::Track,
        );
        let mut cases = Vec::new();
        let mut zero_limit = CommentListRequest::new(target.clone(), 0);
        zero_limit.view = CommentListView::All;
        cases.push(zero_limit);
        let mut missing_parent = CommentListRequest::new(target.clone(), 20);
        missing_parent.view = CommentListView::Replies;
        cases.push(missing_parent);
        let mut cursor_without_sort = CommentListRequest::new(target.clone(), 20);
        cursor_without_sort.cursor = Some("100".to_owned());
        cases.push(cursor_without_sort);
        let mut wrong_cursor_sort = CommentListRequest::new(target.clone(), 20);
        wrong_cursor_sort.sort = Some(CommentSort::Hot);
        wrong_cursor_sort.cursor = Some("100".to_owned());
        cases.push(wrong_cursor_sort);
        let mut hot_with_sort = CommentListRequest::new(target.clone(), 20);
        hot_with_sort.view = CommentListView::Hot;
        hot_with_sort.sort = Some(CommentSort::Recommended);
        cases.push(hot_with_sort);
        let mut zero_page = CommentListRequest::new(target, 20);
        zero_page.sort = Some(CommentSort::Recommended);
        zero_page.page = Some(0);
        cases.push(zero_page);

        for request in cases {
            assert_eq!(
                netease_comment_list_request(&request)
                    .expect_err("invalid comment request")
                    .code,
                ErrorCode::InvalidRequest
            );
        }
    }

    #[test]
    fn maps_legacy_comment_lists_without_losing_hot_top_or_reply_data() {
        let target = CommentTarget::new(
            ResourceRef::new(Platform::Netease, "185809").expect("valid track target"),
            CommentTargetKind::Track,
        );
        let mut request = CommentListRequest::new(target, 2);
        request.offset = 4;
        let page = map_netease_comment_page(
            &request,
            NeteaseCommentListMode::Legacy,
            json!({
                "code": 200,
                "total": 68_334,
                "more": true,
                "moreHot": true,
                "comments": [fixture_comment(3_160_990_055, "普通评论")],
                "hotComments": [fixture_comment(200, "热门评论")],
                "topComments": [fixture_comment(300, "置顶评论")]
            }),
        )
        .expect("map legacy comments");
        assert_eq!(page.comments.len(), 1);
        assert_eq!(page.hot_comments.len(), 1);
        assert_eq!(page.top_comments.len(), 1);
        let comment = &page.comments[0];
        assert_eq!(comment.id, "3160990055");
        assert_eq!(comment.content, "普通评论");
        assert_eq!(comment.author.as_ref().expect("author").name, "阿良0321");
        assert_eq!(
            comment
                .author
                .as_ref()
                .expect("author")
                .resource_ref
                .to_string(),
            "netease:278612322"
        );
        assert_eq!(comment.like_count, Some(5_646));
        assert_eq!(comment.parent_comment_id, None);
        assert_eq!(comment.reply_count, Some(2));
        assert_eq!(comment.replied_to[0].comment_id.as_deref(), Some("100"));
        assert_eq!(
            comment.replied_to[0]
                .author
                .as_ref()
                .expect("reply author")
                .name,
            "被回复者"
        );
        assert_eq!(comment.ip_location.as_deref(), Some("上海"));
        assert_eq!(
            comment.extensions["response"]["richContent"],
            "保留平台富文本"
        );
        assert_eq!(page.pagination.total, Some(68_334));
        assert_eq!(page.pagination.next_offset, Some(5));
        assert!(page.pagination.has_more);
        assert_eq!(page.pagination.extensions["mode"], "legacy");
        assert_eq!(page.pagination.extensions["limit_applied"], true);
        assert_eq!(
            page.pagination.extensions["next_before_time_ms"],
            1_582_035_919_432_u64
        );
        assert_eq!(page.extensions["response"]["code"], 200);
    }

    #[test]
    fn comment_mapping_skips_empty_primary_ids_and_names() {
        let comment = map_netease_comment(json!({
            "commentId": null,
            "id": "3160990055",
            "content": "兼容字段",
            "user": {
                "userId": " ",
                "id": "278612322",
                "nickname": " ",
                "userName": "有效昵称"
            },
            "beReplied": [{
                "beRepliedCommentId": "",
                "commentId": 0,
                "id": "100",
                "content": "原评论",
                "user": {"userId": null, "id": "200", "nickname": "被回复者"}
            }]
        }))
        .expect("map compatible comment aliases");

        assert_eq!(comment.id, "3160990055");
        let author = comment.author.expect("comment author");
        assert_eq!(author.resource_ref.to_string(), "netease:278612322");
        assert_eq!(author.name, "有效昵称");
        assert_eq!(comment.replied_to[0].comment_id.as_deref(), Some("100"));
        assert_eq!(
            comment.replied_to[0]
                .author
                .as_ref()
                .expect("reply author")
                .resource_ref
                .to_string(),
            "netease:200"
        );
    }

    #[test]
    fn maps_modern_hot_and_floor_comment_pagination_honestly() {
        let target = CommentTarget::new(
            ResourceRef::new(Platform::Netease, "185809").expect("valid track target"),
            CommentTargetKind::Track,
        );
        let mut modern_request = CommentListRequest::new(target.clone(), 2);
        modern_request.sort = Some(CommentSort::Recommended);
        modern_request.page = Some(2);
        let modern = map_netease_comment_page(
            &modern_request,
            NeteaseCommentListMode::Modern,
            json!({
                "code": 200,
                "data": {
                    "comments": [
                        fixture_comment(1, "一"),
                        fixture_comment(2, "二"),
                        fixture_comment(3, "三")
                    ],
                    "currentComment": fixture_comment(4, "当前"),
                    "totalCount": 68_334,
                    "hasMore": true,
                    "cursor": 1_581_222_127_578_u64,
                    "sortType": 99
                }
            }),
        )
        .expect("map modern comments");
        assert_eq!(modern.comments.len(), 3);
        assert_eq!(modern.current_comment.as_ref().expect("current").id, "4");
        assert_eq!(modern.pagination.offset, 2);
        assert_eq!(modern.pagination.next_offset, Some(4));
        assert_eq!(modern.pagination.extensions["next_cursor"], "1581222127578");
        assert_eq!(modern.pagination.extensions["limit_applied"], false);

        let mut hot_request = CommentListRequest::new(target.clone(), 2);
        hot_request.view = CommentListView::Hot;
        let hot = map_netease_comment_page(
            &hot_request,
            NeteaseCommentListMode::Hot,
            json!({
                "code": 200,
                "total": 408,
                "hasMore": true,
                "hotComments": [fixture_comment(10, "热评")],
                "topComments": [fixture_comment(11, "置顶")]
            }),
        )
        .expect("map hot comments");
        assert!(hot.comments.is_empty());
        assert_eq!(hot.hot_comments[0].id, "10");
        assert_eq!(hot.pagination.total, Some(408));
        assert_eq!(hot.pagination.next_offset, Some(1));

        let mut floor_request = CommentListRequest::new(target, 2);
        floor_request.view = CommentListView::Replies;
        floor_request.parent_comment_id = Some("3160990055".to_owned());
        floor_request.offset = 20;
        let floor = map_netease_comment_page(
            &floor_request,
            NeteaseCommentListMode::Floor,
            json!({
                "code": 200,
                "data": {
                    "comments": [fixture_comment(20, "楼层回复")],
                    "bestComments": [fixture_comment(21, "最佳回复")],
                    "currentComment": fixture_comment(22, "当前回复"),
                    "totalCount": 3,
                    "hasMore": true,
                    "time": 1_580_000_000_000_u64
                }
            }),
        )
        .expect("map floor comments");
        assert_eq!(floor.comments[0].id, "20");
        assert_eq!(floor.top_comments[0].id, "21");
        assert_eq!(floor.current_comment.as_ref().expect("current").id, "22");
        assert_eq!(floor.pagination.offset, 0);
        assert_eq!(floor.pagination.extensions["requested_offset"], 20);
        assert_eq!(floor.pagination.extensions["offset_applied"], false);
        assert_eq!(
            floor.pagination.extensions["next_before_time_ms"],
            1_580_000_000_000_u64
        );
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_comments_cover_reference_resources_views_and_sorts() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let targets = [
            (CommentTargetKind::Track, "185809"),
            (CommentTargetKind::Album, "32311"),
            (CommentTargetKind::Playlist, "705123491"),
            (CommentTargetKind::Mv, "5436712"),
            (CommentTargetKind::RadioEpisode, "794062371"),
            (CommentTargetKind::Video, "89ADDE33C0AAE8EC14B99F6750DB954D"),
            (CommentTargetKind::Event, "A_EV_2_6559519868_32953014"),
            (CommentTargetKind::RadioStation, "362"),
        ];
        let mut track_page = None;
        for (kind, id) in targets {
            let request = CommentListRequest::new(
                CommentTarget::new(
                    ResourceRef::new(Platform::Netease, id).expect("valid comment target"),
                    kind,
                ),
                1,
            );
            let page = MusicProvider::comments(&provider, &request)
                .await
                .unwrap_or_else(|error| panic!("{kind:?} comments failed: {error}"));
            assert_eq!(page.extensions["response"]["code"], 200, "{kind:?}");
            if kind == CommentTargetKind::Track {
                track_page = Some(page);
            }
        }

        let track_target = CommentTarget::new(
            ResourceRef::new(Platform::Netease, "185809").expect("valid track target"),
            CommentTargetKind::Track,
        );
        for sort in [
            CommentSort::Recommended,
            CommentSort::Hot,
            CommentSort::Time,
        ] {
            let mut request = CommentListRequest::new(track_target.clone(), 2);
            request.sort = Some(sort);
            let page = MusicProvider::comments(&provider, &request)
                .await
                .unwrap_or_else(|error| panic!("{sort:?} comments failed: {error}"));
            assert_eq!(page.extensions["response"]["code"], 200, "{sort:?}");
            assert_eq!(page.pagination.extensions["mode"], "modern");
        }

        let mut hot_request = CommentListRequest::new(track_target.clone(), 2);
        hot_request.view = CommentListView::Hot;
        let hot = MusicProvider::comments(&provider, &hot_request)
            .await
            .expect("live hot comments");
        assert_eq!(hot.extensions["response"]["code"], 200);
        assert_eq!(hot.pagination.extensions["mode"], "hot");

        let parent_comment_id = track_page
            .and_then(|page| page.comments.into_iter().next())
            .map(|comment| comment.id)
            .expect("live track comments include a floor parent");
        let mut floor_request = CommentListRequest::new(track_target, 2);
        floor_request.view = CommentListView::Replies;
        floor_request.parent_comment_id = Some(parent_comment_id);
        let floor = MusicProvider::comments(&provider, &floor_request)
            .await
            .expect("live floor comments");
        assert_eq!(floor.extensions["response"]["code"], 200);
        assert_eq!(floor.pagination.extensions["mode"], "floor");
    }

    #[test]
    fn maps_legacy_search_song_shape() {
        let song = serde_json::from_value(json!({
            "id": 123,
            "name": "反方向的钟",
            "alias": ["Clockwise"],
            "artists": [{"id": 6452, "name": "周杰伦"}],
            "album": {"id": 456, "name": "Jay", "picUrl": "https://example.test/cover.jpg"},
            "duration": 258000,
            "mvid": 789,
            "fee": 1,
            "status": 0,
            "hMusic": {"bitrate": 320000},
            "sqMusic": {"bitrate": 999000}
        }))
        .expect("valid legacy search fixture");

        let track = map_song(song, None).expect("map legacy search song");
        assert_eq!(track.artists[0].name, "周杰伦");
        assert_eq!(track.album.expect("album").name, "Jay");
        assert_eq!(track.duration_ms, Some(258000));
        assert_eq!(track.mv_ref.expect("MV").to_string(), "netease:789");
        assert_eq!(track.playable, Some(true));
        assert_eq!(
            track.available_qualities,
            vec![
                Quality::Standard,
                Quality::Higher,
                Quality::High,
                Quality::Lossless
            ]
        );
    }

    #[test]
    fn maps_audio_recognition_results_and_preserves_match_metadata() {
        let raw = json!({
            "code": 200,
            "data": {
                "type": 0,
                "queryId": "query-1",
                "noMatchReason": 10,
                "result": [
                    {
                        "song": {
                            "id": 185809,
                            "name": "晴天",
                            "artists": [{"id": 6452, "name": "周杰伦"}],
                            "album": {
                                "id": 18905,
                                "name": "叶惠美",
                                "picUrl": "https://example.test/cover.jpg"
                            },
                            "duration": 269000,
                            "mvid": 186001,
                            "status": 0
                        },
                        "startTime": null,
                        "start_time": "1500",
                        "score": 0.97
                    }
                ],
                "mv": {"id": 186001},
                "moduleList": ["song"]
            }
        });
        let response: AudioMatchEnvelope =
            serde_json::from_value(raw.clone()).expect("audio match fixture");

        let recognition =
            map_audio_recognition(response, raw).expect("map audio recognition result");

        assert_eq!(recognition.query_id.as_deref(), Some("query-1"));
        assert_eq!(recognition.no_match_reason, Some(10));
        assert_eq!(recognition.matches.len(), 1);
        assert_eq!(
            recognition.matches[0].track.resource_ref.to_string(),
            "netease:185809"
        );
        assert_eq!(recognition.matches[0].track.artists[0].name, "周杰伦");
        assert_eq!(recognition.matches[0].start_time_ms, Some(1500));
        assert_eq!(recognition.matches[0].extensions["match"]["score"], 0.97);
        assert_eq!(
            recognition.matches[0].track.extensions["audio_recognition_song"]["mvid"],
            186001
        );
        assert_eq!(recognition.extensions["type"], 0);
        assert_eq!(recognition.extensions["module_list"][0], "song");
        assert_eq!(recognition.extensions["response"]["code"], 200);
    }

    #[test]
    fn maps_pc_and_mobile_banners_without_losing_target_semantics() {
        let pc = map_banner(
            json!({
                "bigImageUrl": "https://example.test/banner-large.png",
                "imageUrl": "https://example.test/banner.png",
                "targetId": 384808686,
                "targetType": 10,
                "typeTitle": "新碟首发",
                "mainTitle": "首发专辑标题",
                "url": "https://music.163.com/album?id=384808686",
                "s_ctrp": "trace-metadata"
            }),
            BannerClient::Pc,
        )
        .expect("map PC banner");
        assert_eq!(pc.id, None);
        assert_eq!(pc.image_url, "https://example.test/banner-large.png");
        assert_eq!(pc.title.as_deref(), Some("首发专辑标题"));
        assert_eq!(pc.target_kind, BannerTargetKind::Album);
        assert_eq!(
            pc.target_ref.expect("album target").to_string(),
            "netease:384808686"
        );
        assert_eq!(pc.extensions["client"], "pc");
        assert_eq!(pc.extensions["catalog"], "music");
        assert_eq!(pc.extensions["banner"]["s_ctrp"], "trace-metadata");

        let mobile = map_banner(
            json!({
                "bannerId": "1717750403848278",
                "pic": "https://example.test/mobile.jpg",
                "targetId": "0",
                "targetType": "3000",
                "typeTitle": "独家策划",
                "url": "https://example.test/event",
                "exclusive": "false",
                "monitorClickList": []
            }),
            BannerClient::Iphone,
        )
        .expect("map mobile banner");
        assert_eq!(mobile.id.as_deref(), Some("1717750403848278"));
        assert_eq!(mobile.target_ref, None);
        assert_eq!(mobile.target_kind, BannerTargetKind::Web);
        assert_eq!(mobile.exclusive, Some(false));
        assert_eq!(mobile.extensions["client"], "iphone");

        let fallback = map_banner(
            json!({
                "bigImageUrl": "  ",
                "pic": "https://example.test/fallback.jpg",
                "mainTitle": "",
                "typeTitle": "有效摘要"
            }),
            BannerClient::Android,
        )
        .expect("skip blank preferred banner fields");
        assert_eq!(fallback.image_url, "https://example.test/fallback.jpg");
        assert_eq!(fallback.title.as_deref(), Some("有效摘要"));
    }

    #[test]
    fn maps_podcast_banners_without_faking_music_targets() {
        let banner = map_podcast_banner(json!({
            "exclusive": false,
            "pic": "https://example.test/podcast-banner.jpg",
            "targetId": 3723949603_i64,
            "targetType": 60001,
            "typeTitle": "脱口秀",
            "url": "orpheus://program/3723949603",
            "futureField": { "kept": true }
        }))
        .expect("map podcast banner");

        assert_eq!(banner.id, None);
        assert_eq!(banner.title.as_deref(), Some("脱口秀"));
        assert_eq!(
            banner
                .target_ref
                .expect("podcast episode target")
                .to_string(),
            "netease:3723949603"
        );
        assert_eq!(banner.target_kind, BannerTargetKind::PodcastEpisode);
        assert_eq!(banner.url.as_deref(), Some("orpheus://program/3723949603"));
        assert_eq!(banner.exclusive, Some(false));
        assert_eq!(banner.extensions["catalog"], "podcast");
        assert!(!banner.extensions.contains_key("client"));
        assert_eq!(banner.extensions["banner"]["futureField"]["kept"], true);
    }

    #[test]
    fn maps_broadcast_categories_and_regions_without_numeric_id_leakage() {
        let category = map_radio_catalog_option(json!({ "id": 1, "name": "音乐台" }), "category")
            .expect("map category");
        assert_eq!(category.id, "1");
        assert_eq!(category.name, "音乐台");
        assert_eq!(category.extensions["broadcast_option"]["id"], 1);

        let region = map_radio_catalog_option(
            json!({ "id": "407", "name": " 网络台 ", "future": true }),
            "region",
        )
        .expect("map region");
        assert_eq!(region.id, "407");
        assert_eq!(region.name, "网络台");
        assert_eq!(region.extensions["broadcast_option"]["future"], true);

        assert_eq!(
            map_radio_catalog_option(json!({ "id": 1, "name": "" }), "category")
                .expect_err("missing name")
                .code,
            ErrorCode::UpstreamError
        );
    }

    #[test]
    fn podcast_requests_match_reference_modules() {
        assert_eq!(
            netease_podcast_categories_request(),
            ("/api/djradio/category/get", json!({}))
        );
        let mut hot_request = PodcastListRequest::new(PodcastCatalog::Hot, 20, 40);
        hot_request.account = Some("podcast-user".to_owned());
        assert_eq!(
            netease_podcast_catalog_request(&hot_request).expect("hot podcast request"),
            ("/api/djradio/hot/v1", json!({"limit": 20, "offset": 40}))
        );
        let featured_request = PodcastListRequest::new(PodcastCatalog::Featured, 30, 0);
        assert_eq!(
            netease_podcast_catalog_request(&featured_request).expect("featured podcast request"),
            ("/api/djradio/recommend/v1", json!({}))
        );
        let personalized_request = PodcastListRequest::new(PodcastCatalog::Personalized, 6, 0);
        assert_eq!(
            netease_podcast_catalog_request(&personalized_request)
                .expect("personalized podcast request"),
            ("/api/djradio/personalize/rcmd", json!({"limit": 6}))
        );
        let mut category_hot_request = PodcastListRequest::new(PodcastCatalog::CategoryHot, 30, 60);
        category_hot_request.category_id = Some("2".to_owned());
        assert_eq!(
            netease_podcast_catalog_request(&category_hot_request)
                .expect("category hot podcast request"),
            (
                "/api/djradio/hot",
                json!({"cateId": "2", "limit": 30, "offset": 60})
            )
        );
        let mut category_featured_request =
            PodcastListRequest::new(PodcastCatalog::CategoryFeatured, 30, 0);
        category_featured_request.category_id = Some("2".to_owned());
        assert_eq!(
            netease_podcast_catalog_request(&category_featured_request)
                .expect("category featured podcast request"),
            ("/api/djradio/recommend", json!({"cateId": "2"}))
        );
        let mut today_request = PodcastListRequest::new(PodcastCatalog::TodayPreferred, 30, 0);
        today_request.page = Some(2);
        assert_eq!(
            netease_podcast_catalog_request(&today_request)
                .expect("today preferred podcast request"),
            ("/api/djradio/home/today/perfered", json!({"page": 2}))
        );
        let paid_request = PodcastListRequest::new(PodcastCatalog::Paid, 3, 6);
        assert_eq!(
            netease_podcast_catalog_request(&paid_request).expect("paid podcast request"),
            (
                "/api/djradio/home/paygift/list",
                json!({"limit": 3, "offset": 6, "_nmclfl": 1})
            )
        );
        assert_eq!(
            netease_podcast_request(336_355_127),
            ("/api/djradio/v2/get", json!({"id": 336_355_127}))
        );
        assert_eq!(
            netease_podcast_workbench_request(336_355_127),
            (
                "/api/voice/workbench/voicelist/detail",
                json!({"id": 336_355_127})
            )
        );
        assert_eq!(
            netease_podcast_chart_request(
                &PodcastChartRequest::new(PodcastChartKind::New, 20, 40,)
            )
            .expect("new podcast chart request"),
            (
                "/api/djradio/toplist",
                json!({"limit": 20, "offset": 40, "type": "0"})
            )
        );
        assert_eq!(
            netease_podcast_chart_request(
                &PodcastChartRequest::new(PodcastChartKind::Hot, 30, 60,)
            )
            .expect("hot podcast chart request"),
            (
                "/api/djradio/toplist",
                json!({"limit": 30, "offset": 60, "type": 1})
            )
        );
        assert_eq!(
            netease_podcast_chart_request(
                &PodcastChartRequest::new(PodcastChartKind::Paid, 10, 0,)
            )
            .expect("paid podcast chart request"),
            ("/api/djradio/toplist/pay", json!({"limit": 10}))
        );
        for invalid in [
            PodcastChartRequest::new(PodcastChartKind::New, 0, 0),
            PodcastChartRequest::new(PodcastChartKind::Hot, 101, 0),
            PodcastChartRequest::new(PodcastChartKind::Paid, 20, 1),
        ] {
            assert_eq!(
                netease_podcast_chart_request(&invalid)
                    .expect_err("invalid podcast chart request")
                    .code,
                ErrorCode::InvalidRequest
            );
        }
        assert_eq!(
            netease_podcast_creator_chart_request(&PodcastCreatorChartRequest::new(
                PodcastCreatorChartKind::Newcomer,
                20,
                40,
            ))
            .expect("newcomer podcast creator chart request"),
            (
                "/api/dj/toplist/newcomer",
                json!({"limit": 20, "offset": 40})
            )
        );
        assert_eq!(
            netease_podcast_creator_chart_request(&PodcastCreatorChartRequest::new(
                PodcastCreatorChartKind::Popular,
                30,
                0,
            ))
            .expect("popular podcast creator chart request"),
            ("/api/dj/toplist/popular", json!({"limit": 30}))
        );
        assert_eq!(
            netease_podcast_creator_chart_request(&PodcastCreatorChartRequest::new(
                PodcastCreatorChartKind::Trending24Hours,
                10,
                0,
            ))
            .expect("24-hour podcast creator chart request"),
            ("/api/dj/toplist/hours", json!({"limit": 10}))
        );
        for invalid in [
            PodcastCreatorChartRequest::new(PodcastCreatorChartKind::Newcomer, 0, 0),
            PodcastCreatorChartRequest::new(PodcastCreatorChartKind::Newcomer, 101, 0),
            PodcastCreatorChartRequest::new(PodcastCreatorChartKind::Popular, 20, 1),
            PodcastCreatorChartRequest::new(PodcastCreatorChartKind::Trending24Hours, 20, 1),
        ] {
            assert_eq!(
                netease_podcast_creator_chart_request(&invalid)
                    .expect_err("invalid podcast creator chart request")
                    .code,
                ErrorCode::InvalidRequest
            );
        }
        assert_eq!(
            netease_podcast_subscription_request(336_355_127, true),
            ("/api/djradio/sub", json!({"id": 336_355_127}))
        );
        assert_eq!(
            netease_podcast_subscription_request(336_355_127, false),
            ("/api/djradio/unsub", json!({"id": 336_355_127}))
        );
        assert_eq!(
            netease_subscribed_podcasts_request(30, 60),
            (
                "/api/djradio/get/subed",
                json!({"limit": 30, "offset": 60, "total": true})
            )
        );

        let mut request = PodcastEpisodeListRequest::new(25, 50);
        request.ascending = true;
        request.account = Some("podcast-user".to_owned());
        assert_eq!(
            netease_podcast_episodes_request(336_355_127, &request, 25),
            (
                "/api/dj/program/byradio",
                json!({
                    "radioId": 336_355_127,
                    "limit": 25,
                    "offset": 50,
                    "asc": true
                })
            )
        );
        assert_eq!(
            netease_podcast_episode_request(1_367_665_101),
            ("/api/dj/program/detail", json!({"id": 1_367_665_101}))
        );
        assert_eq!(
            netease_podcast_episode_workbench_request(2_058_695_201),
            (
                "/api/voice/workbench/voice/detail",
                json!({"id": 2_058_695_201})
            )
        );
        assert_eq!(
            netease_podcast_episode_lyrics_request(1_367_665_101),
            ("/api/voice/lyric/get", json!({"programId": 1_367_665_101}))
        );
        assert_eq!(
            netease_podcast_episode_chart_request(&PodcastEpisodeChartRequest::new(
                PodcastEpisodeChartKind::Popular,
                20,
                40,
            ))
            .expect("popular episode chart request"),
            (
                "/api/program/toplist/v1",
                json!({"limit": 20, "offset": 40})
            )
        );
        assert_eq!(
            netease_podcast_episode_chart_request(&PodcastEpisodeChartRequest::new(
                PodcastEpisodeChartKind::Trending24Hours,
                30,
                0,
            ))
            .expect("24-hour episode chart request"),
            ("/api/djprogram/toplist/hours", json!({"limit": 30}))
        );
        for invalid in [
            PodcastEpisodeChartRequest::new(PodcastEpisodeChartKind::Popular, 0, 0),
            PodcastEpisodeChartRequest::new(PodcastEpisodeChartKind::Popular, 101, 0),
            PodcastEpisodeChartRequest::new(PodcastEpisodeChartKind::Trending24Hours, 20, 1),
        ] {
            assert_eq!(
                netease_podcast_episode_chart_request(&invalid)
                    .expect_err("invalid episode chart request")
                    .code,
                ErrorCode::InvalidRequest
            );
        }

        for invalid in [
            PodcastListRequest::new(PodcastCatalog::Hot, 0, 0),
            PodcastListRequest::new(PodcastCatalog::Hot, 101, 0),
            PodcastListRequest::new(PodcastCatalog::Featured, 20, 1),
            PodcastListRequest::new(PodcastCatalog::Personalized, 20, 1),
            PodcastListRequest::new(PodcastCatalog::CategoryHot, 20, 0),
            PodcastListRequest::new(PodcastCatalog::CategoryFeatured, 20, 0),
        ] {
            assert_eq!(
                netease_podcast_catalog_request(&invalid)
                    .expect_err("invalid podcast catalog request")
                    .code,
                ErrorCode::InvalidRequest
            );
        }
        let mut hot_with_category = PodcastListRequest::new(PodcastCatalog::Hot, 20, 0);
        hot_with_category.category_id = Some("2".to_owned());
        assert_eq!(
            netease_podcast_catalog_request(&hot_with_category)
                .expect_err("hot catalog rejects category")
                .code,
            ErrorCode::InvalidRequest
        );
        let mut category_featured = PodcastListRequest::new(PodcastCatalog::Featured, 20, 0);
        category_featured.category_id = Some("2".to_owned());
        assert_eq!(
            netease_podcast_catalog_request(&category_featured)
                .expect_err("featured catalog rejects category")
                .code,
            ErrorCode::InvalidRequest
        );
        let mut category_personalized =
            PodcastListRequest::new(PodcastCatalog::Personalized, 20, 0);
        category_personalized.category_id = Some("2".to_owned());
        assert_eq!(
            netease_podcast_catalog_request(&category_personalized)
                .expect_err("personalized catalog rejects category")
                .code,
            ErrorCode::InvalidRequest
        );
        let mut invalid_category_hot = PodcastListRequest::new(PodcastCatalog::CategoryHot, 20, 0);
        invalid_category_hot.category_id = Some("music".to_owned());
        assert_eq!(
            netease_podcast_catalog_request(&invalid_category_hot)
                .expect_err("category hot catalog rejects non-numeric category")
                .code,
            ErrorCode::InvalidRequest
        );
        for (category_id, offset) in [(Some("music"), 0), (Some("2"), 1)] {
            let mut invalid_category_featured =
                PodcastListRequest::new(PodcastCatalog::CategoryFeatured, 20, offset);
            invalid_category_featured.category_id = category_id.map(str::to_owned);
            assert_eq!(
                netease_podcast_catalog_request(&invalid_category_featured)
                    .expect_err("category featured catalog rejects invalid controls")
                    .code,
                ErrorCode::InvalidRequest
            );
        }
        let mut hot_with_page = PodcastListRequest::new(PodcastCatalog::Hot, 20, 0);
        hot_with_page.page = Some(2);
        assert_eq!(
            netease_podcast_catalog_request(&hot_with_page)
                .expect_err("hot catalog rejects page")
                .code,
            ErrorCode::InvalidRequest
        );
        for (category_id, offset) in [(Some("2"), 0), (None, 1)] {
            let mut invalid_today =
                PodcastListRequest::new(PodcastCatalog::TodayPreferred, 20, offset);
            invalid_today.category_id = category_id.map(str::to_owned);
            assert_eq!(
                netease_podcast_catalog_request(&invalid_today)
                    .expect_err("today preferred catalog rejects invalid controls")
                    .code,
                ErrorCode::InvalidRequest
            );
        }
        let mut paid_with_category = PodcastListRequest::new(PodcastCatalog::Paid, 20, 0);
        paid_with_category.category_id = Some("2".to_owned());
        assert_eq!(
            netease_podcast_catalog_request(&paid_with_category)
                .expect_err("paid catalog rejects category")
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn maps_podcast_categories_with_stable_ids_icons_and_complete_extensions() {
        let taxonomy = map_netease_podcast_categories(json!({
            "code": 200,
            "categories": [
                {
                    "id": 2,
                    "name": " 音乐播客 ",
                    "picWebUrl": "https://example.test/web.png",
                    "pic96x96Url": "https://example.test/96.png",
                    "picPCBlackUrl": "https://example.test/black.png",
                    "futureField": {"kept": true}
                },
                {
                    "id": "10001",
                    "name": "有声书",
                    "picWebUrl": "",
                    "pic56x56Url": "https://example.test/56.png"
                }
            ],
            "futureTopLevel": "preserved"
        }))
        .expect("map podcast taxonomy");

        assert_eq!(taxonomy.categories.len(), 2);
        assert_eq!(taxonomy.categories[0].id, "2");
        assert_eq!(taxonomy.categories[0].name, "音乐播客");
        assert_eq!(
            taxonomy.categories[0].icon_url.as_deref(),
            Some("https://example.test/web.png")
        );
        assert_eq!(
            taxonomy.categories[0].extensions["category"]["futureField"]["kept"],
            true
        );
        assert_eq!(taxonomy.categories[1].id, "10001");
        assert_eq!(
            taxonomy.categories[1].icon_url.as_deref(),
            Some("https://example.test/56.png")
        );
        assert_eq!(
            taxonomy.extensions["response"]["futureTopLevel"],
            "preserved"
        );

        for invalid in [
            json!({"code": 200}),
            json!({"code": 200, "categories": [{"name": "缺少 ID"}]}),
            json!({"code": 200, "categories": [{"id": 1, "name": "  "}]}),
        ] {
            assert_eq!(
                map_netease_podcast_categories(invalid)
                    .expect_err("invalid podcast taxonomy")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn maps_podcast_catalog_variants_without_losing_fields_or_pagination() {
        let request = PodcastListRequest::new(PodcastCatalog::Hot, 2, 4);
        let page = map_netease_podcast_catalog_response(
            json!({
                "code": 200,
                "djRadios": [
                    fixture_podcast_radio(336_355_127, "代码时间"),
                    fixture_podcast_radio(350_080_795, "音乐电台")
                ],
                "count": 50,
                "hasMore": true,
                "futureTopLevel": "preserved"
            }),
            &request,
        )
        .expect("map hot podcast catalog");

        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].resource_ref.to_string(), "netease:336355127");
        assert_eq!(page.items[0].name, "代码时间");
        assert_eq!(page.items[0].category.as_deref(), Some("音乐播客"));
        assert_eq!(page.items[0].episode_count, Some(120));
        assert_eq!(page.items[0].play_count, Some(98_765));
        assert_eq!(page.items[0].subscriber_count, Some(4_567));
        assert_eq!(
            page.items[0].extensions["podcast"]["futureField"]["kept"],
            true
        );
        assert_eq!(page.pagination.limit, 2);
        assert_eq!(page.pagination.offset, 4);
        assert_eq!(page.pagination.total, Some(50));
        assert_eq!(page.pagination.next_offset, Some(6));
        assert!(page.pagination.has_more);
        assert_eq!(page.pagination.extensions["catalog"], "hot");
        assert_eq!(page.pagination.extensions["returned_count"], 2);
        assert_eq!(page.pagination.extensions["limit_applied"], true);
        assert_eq!(
            page.pagination.extensions["response"]["futureTopLevel"],
            "preserved"
        );

        let featured_request = PodcastListRequest::new(PodcastCatalog::Featured, 2, 0);
        let featured = map_netease_podcast_catalog_response(
            json!({
                "code": 200,
                "name": "精选电台 - 谈情说爱",
                "djRadios": [
                    fixture_podcast_radio(336_355_127, "代码时间"),
                    fixture_podcast_radio(350_080_795, "音乐电台"),
                    fixture_podcast_radio(792_544_462, "四只烤翅")
                ]
            }),
            &featured_request,
        )
        .expect("map featured podcast snapshot");
        assert_eq!(featured.items.len(), 3);
        assert_eq!(featured.pagination.limit, 2);
        assert_eq!(featured.pagination.offset, 0);
        assert_eq!(featured.pagination.total, Some(3));
        assert_eq!(featured.pagination.next_offset, None);
        assert!(!featured.pagination.has_more);
        assert_eq!(featured.pagination.extensions["catalog"], "featured");
        assert_eq!(featured.pagination.extensions["returned_count"], 3);
        assert_eq!(featured.pagination.extensions["limit_applied"], false);
        assert_eq!(
            featured.pagination.extensions["response"]["name"],
            "精选电台 - 谈情说爱"
        );

        let personalized_request = PodcastListRequest::new(PodcastCatalog::Personalized, 2, 0);
        let personalized = map_netease_podcast_catalog_response(
            json!({
                "code": 200,
                "data": [
                    fixture_podcast_radio(792_734_685, "张家旺"),
                    fixture_podcast_radio(794_532_838, "火羊瞌睡了")
                ]
            }),
            &personalized_request,
        )
        .expect("map personalized podcast recommendations");
        assert_eq!(personalized.items.len(), 2);
        assert_eq!(personalized.pagination.limit, 2);
        assert_eq!(personalized.pagination.offset, 0);
        assert_eq!(personalized.pagination.total, None);
        assert_eq!(personalized.pagination.next_offset, None);
        assert!(!personalized.pagination.has_more);
        assert_eq!(
            personalized.pagination.extensions["catalog"],
            "personalized"
        );
        assert_eq!(personalized.pagination.extensions["returned_count"], 2);
        assert_eq!(personalized.pagination.extensions["limit_applied"], true);

        let mut category_hot_request = PodcastListRequest::new(PodcastCatalog::CategoryHot, 2, 4);
        category_hot_request.category_id = Some("2".to_owned());
        let category_hot = map_netease_podcast_catalog_response(
            json!({
                "code": 200,
                "count": 1000,
                "hasMore": true,
                "djRadios": [
                    fixture_podcast_radio(988_389_326, "分类置顶一"),
                    fixture_podcast_radio(795_678_845, "分类置顶二"),
                    fixture_podcast_radio(978_246_340, "分类置顶三")
                ]
            }),
            &category_hot_request,
        )
        .expect("map category hot podcasts");
        assert_eq!(category_hot.items.len(), 3);
        assert_eq!(category_hot.pagination.limit, 2);
        assert_eq!(category_hot.pagination.offset, 4);
        assert_eq!(category_hot.pagination.total, Some(1000));
        assert_eq!(category_hot.pagination.next_offset, Some(6));
        assert!(category_hot.pagination.has_more);
        assert_eq!(
            category_hot.pagination.extensions["catalog"],
            "category_hot"
        );
        assert_eq!(category_hot.pagination.extensions["category_id"], "2");
        assert_eq!(category_hot.pagination.extensions["returned_count"], 3);
        assert_eq!(category_hot.pagination.extensions["limit_applied"], false);

        let mut category_featured_request =
            PodcastListRequest::new(PodcastCatalog::CategoryFeatured, 2, 0);
        category_featured_request.category_id = Some("2".to_owned());
        let category_featured = map_netease_podcast_catalog_response(
            json!({
                "code": 200,
                "hasMore": true,
                "djRadios": [
                    fixture_podcast_radio(1_218_133_742, "小螺号"),
                    fixture_podcast_radio(1_211_583_483, "A座B座"),
                    fixture_podcast_radio(1_211_455_786, "小Lin说")
                ]
            }),
            &category_featured_request,
        )
        .expect("map category featured podcasts");
        assert_eq!(category_featured.items.len(), 3);
        assert_eq!(category_featured.pagination.limit, 2);
        assert_eq!(category_featured.pagination.offset, 0);
        assert_eq!(category_featured.pagination.total, None);
        assert_eq!(category_featured.pagination.next_offset, None);
        assert!(category_featured.pagination.has_more);
        assert_eq!(
            category_featured.pagination.extensions["catalog"],
            "category_featured"
        );
        assert_eq!(category_featured.pagination.extensions["category_id"], "2");
        assert_eq!(category_featured.pagination.extensions["returned_count"], 3);
        assert_eq!(
            category_featured.pagination.extensions["limit_applied"],
            false
        );
        assert_eq!(
            category_featured.pagination.extensions["continuation_supported"],
            false
        );

        let mut today_request = PodcastListRequest::new(PodcastCatalog::TodayPreferred, 30, 0);
        today_request.page = Some(2);
        let today = map_netease_podcast_catalog_response(
            json!({
                "code": 200,
                "data": [
                    fixture_podcast_radio(1_230_174_483, "今日播客一"),
                    fixture_podcast_radio(1_225_587_488, "今日播客二")
                ],
                "msg": null
            }),
            &today_request,
        )
        .expect("map today preferred podcasts");
        assert_eq!(today.items.len(), 2);
        assert_eq!(today.pagination.limit, 30);
        assert_eq!(today.pagination.offset, 0);
        assert_eq!(today.pagination.total, None);
        assert_eq!(today.pagination.next_offset, None);
        assert!(!today.pagination.has_more);
        assert_eq!(today.pagination.extensions["catalog"], "today_preferred");
        assert_eq!(today.pagination.extensions["page"], 2);
        assert_eq!(today.pagination.extensions["page_control_supported"], true);
        assert_eq!(today.pagination.extensions["returned_count"], 2);
        assert_eq!(today.pagination.extensions["limit_applied"], false);

        let paid_request = PodcastListRequest::new(PodcastCatalog::Paid, 2, 4);
        let paid = map_netease_podcast_catalog_response(
            json!({
                "code": 200,
                "data": {
                    "hasMore": true,
                    "list": [
                        {
                            "id": 795_086_635,
                            "name": "广播剧《青梅屿》",
                            "picUrl": "https://example.test/paid.jpg",
                            "programCount": 14,
                            "radioFeeType": 2,
                            "feeScope": 1,
                            "originalPrice": 1290,
                            "discountPrice": null,
                            "futureField": "preserved"
                        },
                        {
                            "id": 795_568_115,
                            "name": "折扣播客",
                            "radioFeeType": 2,
                            "originalPrice": 990,
                            "discountPrice": 490
                        }
                    ]
                },
                "msg": null
            }),
            &paid_request,
        )
        .expect("map paid podcasts");
        assert_eq!(paid.items.len(), 2);
        assert_eq!(paid.items[0].paid, Some(true));
        assert_eq!(
            paid.items[0].price.as_ref().map(|price| price.amount),
            Some(12.9)
        );
        assert_eq!(
            paid.items[0]
                .price
                .as_ref()
                .map(|price| price.currency.as_str()),
            Some("CNY")
        );
        assert_eq!(
            paid.items[1].price.as_ref().map(|price| price.amount),
            Some(4.9)
        );
        assert_eq!(
            paid.items[0].extensions["podcast"]["futureField"],
            "preserved"
        );
        assert_eq!(paid.pagination.limit, 2);
        assert_eq!(paid.pagination.offset, 4);
        assert_eq!(paid.pagination.total, None);
        assert_eq!(paid.pagination.next_offset, Some(6));
        assert!(paid.pagination.has_more);
        assert_eq!(paid.pagination.extensions["catalog"], "paid");
        assert_eq!(paid.pagination.extensions["returned_count"], 2);
        assert_eq!(paid.pagination.extensions["limit_applied"], true);

        let empty = map_netease_podcast_catalog_response(
            json!({"code": 200, "djRadios": [], "hasMore": true}),
            &PodcastListRequest::new(PodcastCatalog::Hot, 20, 80),
        )
        .expect("empty hot podcast catalog");
        assert!(!empty.pagination.has_more);
        assert_eq!(empty.pagination.next_offset, None);

        for invalid in [
            json!({"code": 200}),
            json!({"code": 200, "djRadios": [{"name": "缺少 ID"}]}),
        ] {
            assert_eq!(
                map_netease_podcast_catalog_response(invalid, &request)
                    .expect_err("invalid podcast catalog response")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn maps_podcast_detail_and_episode_catalog_without_collapsing_audio_identity() {
        let podcast = map_netease_podcast_response(json!({
            "code": 200,
            "data": {
                "id": 336_355_127,
                "name": "代码时间",
                "desc": "面向开发者的播客",
                "picUrl": "https://example.test/podcast.jpg",
                "dj": {
                    "userId": null,
                    "id": 32_953_014,
                    "nickname": " ",
                    "name": "主播",
                    "avatarUrl": "https://example.test/avatar.jpg"
                },
                "category": "科技",
                "secondCategory": "互联网",
                "programCount": 120,
                "subCount": 4_567,
                "playCount": 98_765,
                "subed": true,
                "radioFeeType": 0,
                "buyed": false,
                "createTime": 1_704_067_200_000_u64,
                "futureField": "preserved"
            }
        }))
        .expect("map podcast detail");
        assert_eq!(podcast.resource_ref.to_string(), "netease:336355127");
        assert_eq!(podcast.description, "面向开发者的播客");
        let creator = podcast.creator.expect("podcast creator");
        assert_eq!(creator.name, "主播");
        assert_eq!(
            creator
                .resource_ref
                .expect("podcast creator reference")
                .to_string(),
            "netease:32953014"
        );
        assert_eq!(podcast.category.as_deref(), Some("科技"));
        assert_eq!(podcast.secondary_category.as_deref(), Some("互联网"));
        assert_eq!(podcast.episode_count, Some(120));
        assert_eq!(podcast.subscriber_count, Some(4_567));
        assert_eq!(podcast.play_count, Some(98_765));
        assert_eq!(podcast.subscribed, Some(true));
        assert_eq!(podcast.paid, Some(false));
        assert_eq!(podcast.purchased, Some(false));
        assert_eq!(podcast.extensions["podcast"]["futureField"], "preserved");
        assert_eq!(podcast.extensions["response"]["code"], 200);

        let workbench = map_netease_podcast_workbench_response(json!({
            "code": 200,
            "data": {
                "id": 0,
                "name": "低优先级包装",
                "voiceList": {
                    "voiceListId": 336_355_127,
                    "name": "代码时间工作台",
                    "description": "创作者播客介绍",
                    "coverImgUrl": "https://example.test/workbench-podcast.jpg",
                    "categoryName": "科技",
                    "voiceCount": 120,
                    "playCount": 98_765,
                    "voiceFeeType": 0,
                    "creatorName": "低信息摘要",
                    "creator": {
                        "id": 32_953_014,
                        "name": "完整主播",
                        "avatarUrl": "https://example.test/avatar.jpg"
                    },
                    "displayStatus": "ONLINE",
                    "futureField": {"preserved": true}
                }
            }
        }))
        .expect("map podcast workbench detail");
        assert_eq!(workbench.resource_ref.to_string(), "netease:336355127");
        assert_eq!(workbench.name, "代码时间工作台");
        assert_eq!(workbench.description, "创作者播客介绍");
        assert_eq!(workbench.category.as_deref(), Some("科技"));
        assert_eq!(workbench.episode_count, Some(120));
        assert_eq!(workbench.play_count, Some(98_765));
        assert_eq!(workbench.paid, Some(false));
        let creator = workbench.creator.as_ref().expect("workbench creator");
        assert_eq!(creator.name, "完整主播");
        assert_eq!(
            creator
                .resource_ref
                .as_ref()
                .expect("workbench creator reference")
                .to_string(),
            "netease:32953014"
        );
        assert_eq!(workbench.extensions["podcast"]["displayStatus"], "ONLINE");
        assert_eq!(
            workbench.extensions["podcast"]["futureField"]["preserved"],
            true
        );
        assert_eq!(workbench.extensions["workbench_response"]["code"], 200);

        let fallback = map_netease_podcast_workbench_response(json!({
            "code": 200,
            "data": {
                "voiceList": {"name": "无效嵌套摘要"},
                "radioId": 336_355_128,
                "name": "有效直接详情"
            }
        }))
        .expect("fall back after invalid preferred voice list wrapper");
        assert_eq!(fallback.resource_ref.to_string(), "netease:336355128");
        assert_eq!(fallback.name, "有效直接详情");

        for invalid in [
            json!({"code": 200}),
            json!({"code": 200, "data": {}}),
            json!({"code": 200, "data": {"voiceList": {"name": "缺少 ID"}}}),
        ] {
            assert_eq!(
                map_netease_podcast_workbench_response(invalid)
                    .expect_err("invalid podcast workbench response")
                    .code,
                ErrorCode::UpstreamError
            );
        }

        let program = fixture_podcast_program(1_367_665_101, 336_355_127, 2_603_965_162);
        let page = map_netease_podcast_episodes(
            json!({
                "code": 200,
                "count": 12,
                "more": true,
                "programs": [program.clone()]
            }),
            1,
            5,
        )
        .expect("map podcast episode catalog");
        assert_eq!(page.items.len(), 1);
        let episode = &page.items[0];
        assert_eq!(episode.resource_ref.to_string(), "netease:1367665101");
        assert_eq!(
            episode
                .podcast_ref
                .as_ref()
                .expect("podcast reference")
                .to_string(),
            "netease:336355127"
        );
        let audio = episode.audio.as_ref().expect("episode audio track");
        assert_eq!(audio.resource_ref.to_string(), "netease:2603965162");
        assert_ne!(episode.resource_ref, audio.resource_ref);
        assert_eq!(audio.name, "一期节目音频");
        assert_eq!(episode.duration_ms, Some(258_000));
        assert_eq!(episode.serial_number, Some(42));
        assert_eq!(episode.listener_count, Some(1_234));
        assert_eq!(episode.liked_count, Some(12));
        assert_eq!(episode.comment_count, Some(3));
        assert_eq!(episode.share_count, Some(4));
        assert_eq!(episode.subscribed, Some(false));
        assert_eq!(episode.has_lyrics, Some(true));
        assert_eq!(episode.paid, Some(false));
        assert_eq!(episode.purchased, Some(false));
        assert_eq!(episode.extensions["program"]["id"], 1_367_665_101_u64);
        assert_eq!(page.pagination.total, Some(12));
        assert_eq!(page.pagination.next_offset, Some(6));
        assert!(page.pagination.has_more);
        assert_eq!(page.pagination.extensions["response"]["code"], 200);

        let episode = map_netease_podcast_episode_response(json!({
            "code": 200,
            "program": program
        }))
        .expect("map podcast episode detail");
        assert_eq!(episode.resource_ref.to_string(), "netease:1367665101");
        assert_eq!(episode.extensions["response"]["code"], 200);

        let request = StreamRequest::default();
        let stream_data: StreamData = serde_json::from_value(json!({
            "id": 2_603_965_162_u64,
            "url": "https://example.test/podcast.mp3",
            "br": 128_000,
            "size": 4_096,
            "code": 200,
            "expi": 1_200,
            "type": "mp3",
            "level": "standard",
            "encodeType": "mp3",
            "time": 258_000,
            "fee": 0
        }))
        .expect("valid podcast stream fixture");
        let stream = map_stream(
            episode.audio.as_ref().expect("episode audio"),
            &request,
            stream_data,
            false,
        )
        .expect("map podcast audio stream");
        let episode_stream =
            map_netease_podcast_episode_stream(episode, stream).expect("map episode stream");
        assert_eq!(episode_stream.episode_ref.to_string(), "netease:1367665101");
        assert_eq!(episode_stream.audio_ref.to_string(), "netease:2603965162");
        assert_eq!(
            episode_stream
                .stream
                .origin_track
                .expect("stream origin track")
                .to_string(),
            "netease:2603965162"
        );
        assert_eq!(episode_stream.extensions["episode"]["id"], "1367665101");

        let mut incomplete_summary =
            fixture_podcast_program(1_367_665_102, 336_355_127, 2_603_965_162);
        incomplete_summary["duration"] = json!(0);
        incomplete_summary["createTime"] = json!(0);
        incomplete_summary["scheduledPublishTime"] = json!(1_704_067_200_000_u64);
        let recovered = map_netease_podcast_episode(incomplete_summary)
            .expect("recover podcast timing from richer fallback fields");
        assert_eq!(recovered.duration_ms, Some(258_000));
        assert_eq!(recovered.published_at, unix_rfc3339(1_704_067_200));
    }

    #[test]
    fn maps_workbench_voice_detail_aliases_without_lower_priority_overrides() {
        let episode = map_netease_podcast_episode_workbench_response(json!({
            "code": 200,
            "data": {
                "id": 0,
                "name": "低优先级包装",
                "voice": {
                    "voiceId": 2_058_695_201_u64,
                    "songName": "工作台声音",
                    "desc": "工作台介绍",
                    "coverImgUrl": "https://example.test/workbench.jpg",
                    "radioId": 336_355_127,
                    "songId": 1_336_048_748,
                    "durationMs": 61_250,
                    "publishTime": 1_704_067_200_000_u64,
                    "orderNo": 7,
                    "playCount": 456,
                    "voiceFeeType": 1,
                    "creator": {
                        "id": 32_953_014,
                        "name": "主播",
                        "avatarUrl": "https://example.test/avatar.jpg"
                    },
                    "displayStatus": "ONLINE",
                    "futureField": {"preserved": true}
                }
            }
        }))
        .expect("map workbench voice detail");

        assert_eq!(episode.resource_ref.to_string(), "netease:2058695201");
        assert_eq!(episode.name, "工作台声音");
        assert_eq!(
            episode
                .podcast_ref
                .as_ref()
                .expect("workbench podcast reference")
                .to_string(),
            "netease:336355127"
        );
        assert_eq!(
            episode
                .audio
                .as_ref()
                .expect("workbench audio reference")
                .resource_ref
                .to_string(),
            "netease:1336048748"
        );
        assert_eq!(episode.duration_ms, Some(61_250));
        assert_eq!(episode.serial_number, Some(7));
        assert_eq!(episode.listener_count, Some(456));
        assert_eq!(episode.paid, Some(true));
        assert_eq!(
            episode
                .creator
                .as_ref()
                .map(|creator| creator.name.as_str()),
            Some("主播")
        );
        assert_eq!(episode.extensions["program"]["displayStatus"], "ONLINE");
        assert_eq!(
            episode.extensions["program"]["futureField"]["preserved"],
            true
        );
        assert_eq!(episode.extensions["workbench_response"]["code"], 200);

        let fallback = map_netease_podcast_episode_workbench_response(json!({
            "code": 200,
            "data": {
                "voice": {"songName": "无效嵌套摘要"},
                "voiceId": 2_058_695_202_u64,
                "songName": "有效直接详情",
                "radioId": 336_355_127,
                "songId": 1_336_048_749
            }
        }))
        .expect("fall back after an invalid preferred wrapper");
        assert_eq!(fallback.resource_ref.to_string(), "netease:2058695202");
        assert_eq!(fallback.name, "有效直接详情");

        for invalid in [
            json!({"code": 200}),
            json!({"code": 200, "data": {}}),
            json!({"code": 200, "data": {"voice": {"songName": "缺少 ID"}}}),
        ] {
            assert_eq!(
                map_netease_podcast_episode_workbench_response(invalid)
                    .expect_err("invalid workbench voice response")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn maps_subscribed_podcasts_and_mutations_with_library_semantics() {
        let mut first = fixture_podcast_radio(336_355_127, "代码时间");
        first["subed"] = json!(false);
        let page = map_netease_subscribed_podcasts(
            json!({
                "code": 200,
                "count": 10,
                "hasMore": null,
                "more": true,
                "djRadios": [
                    first,
                    fixture_podcast_radio(350_080_795, "音乐电台")
                ],
                "futureField": "preserved"
            }),
            2,
            4,
        )
        .expect("map subscribed podcasts");
        assert_eq!(page.items.len(), 2);
        assert!(
            page.items
                .iter()
                .all(|podcast| podcast.subscribed == Some(true))
        );
        assert_eq!(page.pagination.limit, 2);
        assert_eq!(page.pagination.offset, 4);
        assert_eq!(page.pagination.total, Some(10));
        assert_eq!(page.pagination.next_offset, Some(6));
        assert!(page.pagination.has_more);
        assert_eq!(
            page.pagination.extensions["response"]["futureField"],
            "preserved"
        );

        let subscribed = map_podcast_subscription_result(
            336_355_127,
            true,
            json!({"code": 200, "futureField": true}),
        )
        .expect("map podcast subscribe result");
        assert_eq!(subscribed.resource_ref.to_string(), "netease:336355127");
        assert!(subscribed.subscribed);
        assert_eq!(subscribed.extensions["response"]["futureField"], true);
        let unsubscribed =
            map_podcast_subscription_result(336_355_127, false, json!({"code": 200}))
                .expect("map podcast unsubscribe result");
        assert!(!unsubscribed.subscribed);

        let empty = map_netease_subscribed_podcasts(
            json!({"code": 200, "count": 10, "hasMore": true, "djRadios": []}),
            20,
            10,
        )
        .expect("map empty subscribed podcast page");
        assert!(!empty.pagination.has_more);
        assert_eq!(empty.pagination.next_offset, None);

        for invalid in [
            json!({"code": 200}),
            json!({"code": 200, "djRadios": [{"name": "缺少 ID"}]}),
        ] {
            assert_eq!(
                map_netease_subscribed_podcasts(invalid, 20, 0)
                    .expect_err("invalid subscribed podcast response")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn maps_podcast_charts_without_faking_ignored_offset_or_losing_sparse_paid_entries() {
        let new_request = PodcastChartRequest::new(PodcastChartKind::New, 1, 40);
        let mut new_entry = fixture_podcast_radio(792_868_378, "一条小团团OvO的翻唱合集");
        new_entry["rank"] = json!(1);
        new_entry["lastRank"] = json!(3);
        new_entry["score"] = json!(116_563);
        let new_chart = map_netease_podcast_chart(
            json!({
                "code": 200,
                "updateTime": 1_704_067_200_000_u64,
                "toplist": [new_entry],
                "futureField": "preserved"
            }),
            &new_request,
        )
        .expect("map new podcast chart");
        assert_eq!(new_chart.items.len(), 1);
        assert_eq!(new_chart.items[0].rank, 1);
        assert_eq!(new_chart.items[0].previous_rank, Some(3));
        assert_eq!(new_chart.items[0].score, Some(116_563));
        assert_eq!(new_chart.items[0].podcast.id, "792868378");
        assert_eq!(new_chart.pagination.offset, 0);
        assert_eq!(new_chart.pagination.total, None);
        assert_eq!(new_chart.pagination.next_offset, None);
        assert!(!new_chart.pagination.has_more);
        assert_eq!(new_chart.pagination.extensions["kind"], "new");
        assert_eq!(new_chart.pagination.extensions["requested_offset"], 40);
        assert_eq!(new_chart.pagination.extensions["offset_submitted"], true);
        assert_eq!(new_chart.pagination.extensions["offset_applied"], false);
        assert_eq!(
            new_chart.pagination.extensions["continuation_supported"],
            false
        );
        assert_eq!(
            new_chart.pagination.extensions["updated_at"],
            "2024-01-01T00:00:00Z"
        );
        assert_eq!(
            new_chart.pagination.extensions["response"]["futureField"],
            "preserved"
        );

        let paid_request = PodcastChartRequest::new(PodcastChartKind::Paid, 3, 0);
        let paid_chart = map_netease_podcast_chart(
            json!({
                "code": 200,
                "data": {
                    "total": 3,
                    "updateTime": 1_704_067_260_000_u64,
                    "list": [{
                        "id": 1_490_425_014_u64,
                        "rank": 1,
                        "lastRank": -1,
                        "score": 193_200,
                        "name": "猫平安逆袭传奇",
                        "picUrl": "https://example.test/paid-podcast.jpg",
                        "creatorName": "猫平安"
                    }]
                }
            }),
            &paid_request,
        )
        .expect("map paid podcast chart");
        assert_eq!(paid_chart.items[0].rank, 1);
        assert_eq!(paid_chart.items[0].previous_rank, Some(-1));
        assert_eq!(paid_chart.items[0].score, Some(193_200));
        assert_eq!(paid_chart.items[0].podcast.id, "1490425014");
        assert_eq!(paid_chart.items[0].podcast.paid, Some(true));
        assert_eq!(
            paid_chart.items[0]
                .podcast
                .creator
                .as_ref()
                .map(|creator| creator.name.as_str()),
            Some("猫平安")
        );
        assert_eq!(
            paid_chart.items[0].extensions["chart_entry"]["creatorName"],
            "猫平安"
        );
        assert_eq!(paid_chart.pagination.total, Some(3));
        assert_eq!(paid_chart.pagination.extensions["kind"], "paid");
        assert_eq!(paid_chart.pagination.extensions["offset_submitted"], false);

        for (invalid, request) in [
            (json!({"code": 200}), &new_request),
            (
                json!({"code": 200, "toplist": [{"id": 1, "name": "缺少排名"}]}),
                &new_request,
            ),
            (
                json!({
                    "code": 200,
                    "toplist": [{"id": 1, "name": "非法排名", "rank": 0}]
                }),
                &new_request,
            ),
            (json!({"code": 200, "data": {}}), &paid_request),
        ] {
            assert_eq!(
                map_netease_podcast_chart(invalid, request)
                    .expect_err("invalid podcast chart response")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn maps_podcast_creator_charts_without_faking_ignored_offset_or_losing_user_fields() {
        let newcomer_request =
            PodcastCreatorChartRequest::new(PodcastCreatorChartKind::Newcomer, 1, 40);
        let newcomer = map_netease_podcast_creator_chart(
            json!({
                "code": 200,
                "data": {
                    "total": 3,
                    "updateTime": 1_704_067_200_000_u64,
                    "list": [{
                        "id": 401_590_285,
                        "rank": 1,
                        "lastRank": 1,
                        "score": 862_097,
                        "userFollowedCount": 32,
                        "nickName": "煎包比比",
                        "avatarUrl": "https://example.test/newcomer.jpg",
                        "userType": 0,
                        "liveStatus": -1,
                        "futureField": "preserved"
                    }]
                }
            }),
            &newcomer_request,
        )
        .expect("map newcomer podcast creator chart");
        assert_eq!(newcomer.items.len(), 1);
        assert_eq!(newcomer.items[0].rank, 1);
        assert_eq!(newcomer.items[0].previous_rank, Some(1));
        assert_eq!(newcomer.items[0].score, Some(862_097));
        assert_eq!(newcomer.items[0].follower_count, Some(32));
        assert_eq!(newcomer.items[0].creator.id, "401590285");
        assert_eq!(newcomer.items[0].creator.name, "煎包比比");
        assert_eq!(
            newcomer.items[0].creator.avatar_url.as_deref(),
            Some("https://example.test/newcomer.jpg")
        );
        assert_eq!(
            newcomer.items[0].extensions["chart_entry"]["futureField"],
            "preserved"
        );
        assert_eq!(newcomer.pagination.offset, 0);
        assert_eq!(newcomer.pagination.total, Some(3));
        assert_eq!(newcomer.pagination.next_offset, None);
        assert!(!newcomer.pagination.has_more);
        assert_eq!(newcomer.pagination.extensions["kind"], "newcomer");
        assert_eq!(newcomer.pagination.extensions["requested_offset"], 40);
        assert_eq!(newcomer.pagination.extensions["offset_submitted"], true);
        assert_eq!(newcomer.pagination.extensions["offset_applied"], false);
        assert_eq!(
            newcomer.pagination.extensions["continuation_supported"],
            false
        );
        assert_eq!(
            newcomer.pagination.extensions["updated_at"],
            "2024-01-01T00:00:00Z"
        );

        let hours_request =
            PodcastCreatorChartRequest::new(PodcastCreatorChartKind::Trending24Hours, 3, 0);
        let hours = map_netease_podcast_creator_chart(
            json!({
                "code": 200,
                "data": {
                    "total": 3,
                    "updateTime": 1_704_067_260_000_u64,
                    "list": [{
                        "id": 287_921_940,
                        "rank": 1,
                        "lastRank": 7,
                        "score": 1_339_233,
                        "userFollowedCount": 76_488,
                        "nickName": "开心锤锤",
                        "avatarUrl": "https://example.test/hours.jpg",
                        "avatarDetail": {"userType": 4, "identityLevel": 1}
                    }]
                }
            }),
            &hours_request,
        )
        .expect("map 24-hour podcast creator chart");
        assert_eq!(hours.items[0].previous_rank, Some(7));
        assert_eq!(hours.items[0].creator.id, "287921940");
        assert_eq!(hours.items[0].follower_count, Some(76_488));
        assert_eq!(
            hours.items[0].creator.extensions["creator"]["avatarDetail"]["identityLevel"],
            1
        );
        assert_eq!(hours.pagination.extensions["kind"], "trending24_hours");
        assert_eq!(hours.pagination.extensions["offset_submitted"], false);

        for invalid in [
            json!({"code": 200}),
            json!({"code": 200, "data": {}}),
            json!({
                "code": 200,
                "data": {"list": [{"rank": 1, "nickName": "缺少 ID"}]}
            }),
            json!({
                "code": 200,
                "data": {"list": [{"id": 1, "rank": 1, "nickName": "   "}]}
            }),
            json!({
                "code": 200,
                "data": {"list": [{"id": 1, "rank": 0, "nickName": "非法排名"}]}
            }),
        ] {
            assert_eq!(
                map_netease_podcast_creator_chart(invalid, &hours_request)
                    .expect_err("invalid podcast creator chart response")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn maps_episode_charts_without_faking_ignored_offset_or_losing_rank_movement() {
        let popular_request =
            PodcastEpisodeChartRequest::new(PodcastEpisodeChartKind::Popular, 1, 40);
        let popular = map_netease_podcast_episode_chart(
            json!({
                "code": 200,
                "updateTime": 1_704_067_200_000_u64,
                "toplist": [{
                    "program": fixture_podcast_program(
                        1_367_665_101,
                        336_355_127,
                        2_603_965_162
                    ),
                    "rank": 2,
                    "lastRank": -1,
                    "score": 41_300,
                    "programFeeType": 2,
                    "futureField": "preserved"
                }]
            }),
            &popular_request,
        )
        .expect("map popular episode chart");
        assert_eq!(popular.items.len(), 1);
        assert_eq!(popular.items[0].rank, 2);
        assert_eq!(popular.items[0].previous_rank, Some(-1));
        assert_eq!(popular.items[0].score, Some(41_300));
        assert_eq!(popular.items[0].episode.id, "1367665101");
        assert_eq!(popular.items[0].episode.paid, Some(true));
        assert_eq!(
            popular.items[0].extensions["chart_entry"]["futureField"],
            "preserved"
        );
        assert_eq!(popular.pagination.limit, 1);
        assert_eq!(popular.pagination.offset, 0);
        assert_eq!(popular.pagination.total, None);
        assert_eq!(popular.pagination.next_offset, None);
        assert!(!popular.pagination.has_more);
        assert_eq!(popular.pagination.extensions["kind"], "popular");
        assert_eq!(popular.pagination.extensions["requested_offset"], 40);
        assert_eq!(popular.pagination.extensions["offset_applied"], false);
        assert_eq!(popular.pagination.extensions["offset_submitted"], true);
        assert_eq!(
            popular.pagination.extensions["continuation_supported"],
            false
        );
        assert_eq!(
            popular.pagination.extensions["updated_at"],
            "2024-01-01T00:00:00Z"
        );

        let hours_request =
            PodcastEpisodeChartRequest::new(PodcastEpisodeChartKind::Trending24Hours, 1, 0);
        let hours = map_netease_podcast_episode_chart(
            json!({
                "code": 200,
                "msg": null,
                "data": {
                    "total": 1,
                    "updateTime": 1_704_067_260_000_u64,
                    "list": [{
                        "program": fixture_podcast_program(
                            2_058_695_201,
                            336_355_127,
                            1_336_048_748
                        ),
                        "rank": 1,
                        "lastRank": 3,
                        "score": 302_820,
                        "programFeeType": 0
                    }]
                }
            }),
            &hours_request,
        )
        .expect("map 24-hour episode chart");
        assert_eq!(hours.items[0].rank, 1);
        assert_eq!(hours.items[0].previous_rank, Some(3));
        assert_eq!(hours.items[0].score, Some(302_820));
        assert_eq!(hours.pagination.total, Some(1));
        assert_eq!(hours.pagination.extensions["kind"], "trending24_hours");
        assert_eq!(hours.pagination.extensions["offset_submitted"], false);
        assert_eq!(hours.pagination.extensions["offset_applied"], false);
        assert_eq!(hours.pagination.extensions["response"]["code"], 200);

        for (invalid, request) in [
            (json!({"code": 200}), &popular_request),
            (
                json!({"code": 200, "toplist": [{"rank": 1}]}),
                &popular_request,
            ),
            (
                json!({
                    "code": 200,
                    "toplist": [{
                        "program": fixture_podcast_program(1, 2, 3),
                        "rank": 0
                    }]
                }),
                &popular_request,
            ),
            (json!({"code": 200, "data": {}}), &hours_request),
        ] {
            assert_eq!(
                map_netease_podcast_episode_chart(invalid, request)
                    .expect_err("invalid episode chart response")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn maps_voice_transcript_and_empty_lyrics_without_losing_audio_identity() {
        let program = fixture_podcast_program(2_058_695_201, 336_355_127, 1_336_048_748);
        let episode = map_netease_podcast_episode_response(json!({
            "code": 200,
            "program": program.clone()
        }))
        .expect("map voice episode");
        let document = json!({
            "duration": 61_250,
            "sents": [
                {
                    "beg": 0,
                    "end": 1_250,
                    "name": "第一句",
                    "speaker": "speaker_0",
                    "sylls": [{"beg": 0, "end": 500, "name": "第一"}]
                },
                {
                    "beg": 60_010,
                    "end": 61_250,
                    "name": "第二句",
                    "speaker": "speaker_1",
                    "sylls": []
                }
            ],
            "paragraphs": null,
            "futureField": {"preserved": true}
        });
        let response = json!({
            "code": 200,
            "data": {
                "lyricUrl": "http://d1.music.126.net/voice/lyric.json",
                "source": {"kind": "automatic"}
            }
        });
        let lyrics = map_netease_podcast_episode_lyrics(episode, response, Some(document.clone()))
            .expect("map voice transcript");
        assert_eq!(lyrics.episode_ref.to_string(), "netease:2058695201");
        assert_eq!(
            lyrics.audio_ref.expect("audio reference").to_string(),
            "netease:1336048748"
        );
        assert_eq!(lyrics.lyrics.track_ref.to_string(), "netease:1336048748");
        assert_eq!(
            lyrics.lyrics.plain.as_deref(),
            Some("[00:00.000]第一句\n[01:00.010]第二句")
        );
        assert_eq!(lyrics.lyrics.format, "netease_voice_json");
        assert_eq!(lyrics.lyrics.extensions["available"], true);
        assert_eq!(lyrics.lyrics.extensions["duration_ms"], 61_250);
        assert_eq!(lyrics.lyrics.extensions["sentence_count"], 2);
        assert_eq!(lyrics.lyrics.extensions["source"]["kind"], "automatic");
        let word_synced: Value = serde_json::from_str(
            lyrics
                .lyrics
                .word_synced
                .as_deref()
                .expect("word-synced transcript"),
        )
        .expect("word-synced JSON");
        assert_eq!(word_synced, document);
        assert_eq!(lyrics.extensions["episode"]["id"], "2058695201");
        assert_eq!(lyrics.extensions["response"]["code"], 200);

        let empty_episode = map_netease_podcast_episode_response(json!({
            "code": 200,
            "program": program
        }))
        .expect("map empty lyric episode");
        let empty = map_netease_podcast_episode_lyrics(
            empty_episode,
            json!({"code": 200, "data": null}),
            None,
        )
        .expect("map empty voice lyrics");
        assert_eq!(empty.lyrics.track_ref.to_string(), "netease:1336048748");
        assert_eq!(empty.lyrics.plain, None);
        assert_eq!(empty.lyrics.word_synced, None);
        assert_eq!(empty.lyrics.format, "plain");
        assert_eq!(empty.lyrics.extensions["available"], false);
        assert_eq!(empty.extensions["response"]["data"], Value::Null);
    }

    #[test]
    fn voice_transcript_mapping_rejects_invalid_sentence_boundaries() {
        let episode =
            map_netease_podcast_episode(fixture_podcast_program(1, 2, 3)).expect("voice episode");
        let error = map_netease_podcast_episode_lyrics(
            episode,
            json!({"code": 200, "data": {"lyricUrl": "http://d1.music.126.net/a"}}),
            Some(json!({
                "sents": [{"beg": 2_000, "end": 1_000, "name": "无效句段"}]
            })),
        )
        .expect_err("invalid sentence boundary");
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(error.message.contains("sentence end time"));
        assert_eq!(voice_lrc_timestamp(3_661_007), "[61:01.007]");
    }

    #[test]
    fn podcast_mapping_rejects_missing_arrays_and_conflicting_audio_ids() {
        for result in [
            map_netease_podcast_response(json!({"code": 200})).map(|_| ()),
            map_netease_podcast_episodes(json!({"code": 200}), 20, 0).map(|_| ()),
            map_netease_podcast_episode_response(json!({"code": 200})).map(|_| ()),
        ] {
            assert_eq!(
                result.expect_err("missing podcast response shape").code,
                ErrorCode::UpstreamError
            );
        }

        let mut program = fixture_podcast_program(1_367_665_101, 336_355_127, 2_603_965_162);
        program["mainSong"]["id"] = json!(2_603_965_163_u64);
        let error = map_netease_podcast_episode(program)
            .expect_err("conflicting program and audio track ids");
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert_eq!(error.details["main_track_id"], "2603965162");
        assert_eq!(error.details["song_id"], "2603965163");
    }

    #[test]
    fn maps_filtered_broadcast_station_catalog_and_cursor() {
        let request = RadioStationListRequest {
            limit: 20,
            offset: 100,
            category_id: Some("1".to_owned()),
            region_id: Some("407".to_owned()),
            cursor: Some(RadioStationCursor {
                id: "172".to_owned(),
                score: 1542,
            }),
            account: Some("radio-user".to_owned()),
        };
        assert_eq!(
            netease_radio_station_list_payload(&request).expect("build station catalog request"),
            json!({
                "categoryId": "1",
                "regionId": "407",
                "limit": "20",
                "lastId": "172",
                "score": "1542"
            })
        );

        let page = map_radio_station_list_response(
            json!({
                "code": 200,
                "data": {
                    "hasMore": true,
                    "total": 843,
                    "list": [
                        {
                            "id": 175,
                            "name": "河北音乐广播",
                            "coverUrl": "https://example.test/175.jpg",
                            "regionName": "河北",
                            "score": 1492,
                            "source": "QT",
                            "subed": false
                        },
                        {
                            "id": 14,
                            "name": "河北交通广播",
                            "coverUrl": "https://example.test/14.jpg",
                            "regionName": "河北",
                            "score": "1472",
                            "source": "QT"
                        }
                    ]
                }
            }),
            &request,
        )
        .expect("map broadcast station catalog");

        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].resource_ref.to_string(), "netease:175");
        assert_eq!(page.items[0].subscribed, Some(false));
        assert_eq!(page.items[1].region.as_deref(), Some("河北"));
        assert_eq!(page.pagination.limit, 20);
        assert_eq!(page.pagination.offset, 0);
        assert_eq!(page.pagination.total, Some(843));
        assert_eq!(page.pagination.next_offset, None);
        assert!(page.pagination.has_more);
        assert_eq!(page.pagination.extensions["next_cursor"]["id"], "14");
        assert_eq!(page.pagination.extensions["next_cursor"]["score"], 1472);
        assert_eq!(page.pagination.extensions["requested_offset"], 100);
        assert_eq!(page.pagination.extensions["offset_applied"], false);
        assert_eq!(page.pagination.extensions["response"]["code"], 200);

        let error = map_radio_station_list_response(
            json!({
                "code": 200,
                "data": {
                    "hasMore": true,
                    "list": [{ "id": 14, "name": "缺少游标分值" }]
                }
            }),
            &request,
        )
        .expect_err("missing cursor score");
        assert_eq!(error.code, ErrorCode::UpstreamError);
    }

    #[test]
    fn maps_collected_broadcast_stations_and_preserves_pagination() {
        assert_eq!(
            netease_radio_collection_payload(25, 50),
            json!({
                "contentType": "BROADCAST",
                "limit": "25",
                "offset": "50",
                "timeReverseOrder": "true",
                "startDate": "4762584922000"
            })
        );

        let page = map_radio_collection_response(
            json!({
                "code": 200,
                "data": {
                    "list": [
                        {
                            "contentId": 362,
                            "contentName": "金山区广播电视台综合广播",
                            "collectTime": 1_700_000_000_000_i64,
                            "content": {
                                "id": 362,
                                "name": "金山区广播电视台综合广播",
                                "coverUrl": "https://example.test/362.jpg",
                                "regionName": "上海",
                                "subed": true,
                                "source": "QT"
                            }
                        },
                        {
                            "content": {},
                            "resourceJson": r#"{"id":1069201,"channelName":"24小时资讯热点","channelCoverUrl":"https://example.test/1069201.jpg","regionName":"网络台"}"#
                        }
                    ],
                    "total": null,
                    "count": 53,
                    "hasMore": null,
                    "more": true
                }
            }),
            25,
            50,
        )
        .expect("map broadcast station collection");

        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].resource_ref.to_string(), "netease:362");
        assert_eq!(page.items[0].name, "金山区广播电视台综合广播");
        assert_eq!(page.items[0].region.as_deref(), Some("上海"));
        assert_eq!(page.items[0].subscribed, Some(true));
        assert_eq!(
            page.items[0].extensions["broadcast_station"]["source"],
            "QT"
        );
        assert_eq!(page.items[1].resource_ref.to_string(), "netease:1069201");
        assert_eq!(page.items[1].name, "24小时资讯热点");
        assert_eq!(page.items[1].region.as_deref(), Some("网络台"));
        assert_eq!(page.pagination.total, Some(53));
        assert!(page.pagination.has_more);
        assert_eq!(page.pagination.next_offset, Some(52));
    }

    #[test]
    fn broadcast_station_collection_skips_empty_legacy_lists() {
        let page = map_radio_collection_response(
            json!({
                "code": 200,
                "data": {
                    "list": [],
                    "data": {
                        "records": [{
                            "contentId": 362,
                            "contentName": "金山区广播电视台综合广播"
                        }],
                        "total": 1,
                        "hasMore": false
                    }
                }
            }),
            25,
            0,
        )
        .expect("skip empty legacy broadcast collection list");

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].resource_ref.to_string(), "netease:362");
        assert_eq!(page.pagination.total, Some(1));
        assert!(!page.pagination.has_more);
    }

    #[test]
    fn broadcast_station_collection_rejects_missing_lists() {
        let error = map_radio_collection_response(json!({ "code": 200, "data": {} }), 25, 0)
            .expect_err("missing collection list");
        assert_eq!(error.code, ErrorCode::UpstreamError);
    }

    #[test]
    fn maps_broadcast_station_current_info_without_guessing_subscription() {
        let station = map_radio_station_response(json!({
            "code": 200,
            "data": {
                "id": 362,
                "channelName": "金山区广播电视台综合广播",
                "channelCoverUrl": "https://example.test/362.jpg",
                "regionName": "上海",
                "playUrl": "https://lhttp.qtfm.cn/live/362/64k.mp3",
                "programName": "晚安金山",
                "programId": 9001,
                "thirdChannelId": "4022",
                "duration": 3600
            }
        }))
        .expect("map broadcast station current info");

        assert_eq!(station.resource_ref.to_string(), "netease:362");
        assert_eq!(station.name, "金山区广播电视台综合广播");
        assert_eq!(station.region.as_deref(), Some("上海"));
        assert_eq!(
            station.stream_url.as_deref(),
            Some("https://lhttp.qtfm.cn/live/362/64k.mp3")
        );
        assert_eq!(station.current_program.as_deref(), Some("晚安金山"));
        assert_eq!(station.subscribed, None);
        assert_eq!(station.extensions["current_info"]["thirdChannelId"], "4022");
        assert_eq!(station.extensions["response"]["code"], 200);

        let error = map_radio_station_response(json!({ "code": 200, "data": {} }))
            .expect_err("missing broadcast station id");
        assert_eq!(error.code, ErrorCode::UpstreamError);
    }

    #[test]
    fn builds_and_maps_broadcast_station_subscription_actions() {
        assert_eq!(
            netease_radio_station_subscription_payload(362, true),
            json!({
                "contentType": "BROADCAST",
                "contentId": "362",
                "cancelCollect": "false"
            })
        );
        assert_eq!(
            netease_radio_station_subscription_payload(362, false),
            json!({
                "contentType": "BROADCAST",
                "contentId": "362",
                "cancelCollect": "true"
            })
        );

        let subscribed = map_radio_station_subscription_result(
            362,
            true,
            json!({ "code": 200, "message": "success" }),
        )
        .expect("map broadcast subscription");
        assert_eq!(subscribed.resource_ref.to_string(), "netease:362");
        assert!(subscribed.subscribed);
        assert_eq!(subscribed.extensions["response"]["code"], 200);

        let unsubscribed = map_radio_station_subscription_result(
            362,
            false,
            json!({ "code": 200, "message": "success" }),
        )
        .expect("map broadcast unsubscription");
        assert!(!unsubscribed.subscribed);
    }

    #[test]
    fn banner_mapping_rejects_items_without_any_image() {
        let error = map_banner(
            json!({"targetId": 185809, "targetType": 1}),
            BannerClient::Android,
        )
        .expect_err("missing banner image");
        assert_eq!(error.code, ErrorCode::UpstreamError);
    }

    #[test]
    fn maps_all_banner_clients_to_reference_values() {
        assert_eq!(netease_banner_client(BannerClient::Pc), "pc");
        assert_eq!(netease_banner_client(BannerClient::Android), "android");
        assert_eq!(netease_banner_client(BannerClient::Iphone), "iphone");
        assert_eq!(netease_banner_client(BannerClient::Ipad), "ipad");
    }

    #[test]
    fn selects_the_exact_banner_protocol_for_each_catalog() {
        let music = BannerListRequest::new(BannerClient::Iphone);
        let (path, payload, protocol) =
            netease_banner_request(&music).expect("build music banner request");
        assert_eq!(path, "/api/v2/banner/get");
        assert_eq!(payload, json!({ "clientType": "iphone" }));
        assert_eq!(protocol, NeteaseBannerProtocol::Eapi);

        let mut podcast = BannerListRequest::new(BannerClient::Pc);
        podcast.catalog = BannerCatalog::Podcast;
        let (path, payload, protocol) =
            netease_banner_request(&podcast).expect("build podcast banner request");
        assert_eq!(path, "/api/djradio/banner/get");
        assert_eq!(payload, json!({}));
        assert_eq!(protocol, NeteaseBannerProtocol::Weapi);

        podcast.client = BannerClient::Android;
        let error = netease_banner_request(&podcast).expect_err("reject unsupported client");
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert_eq!(error.platform, Some(Platform::Netease));
        assert_eq!(error.details["catalog"], "podcast");
        assert_eq!(error.details["client"], "android");
    }

    #[test]
    fn maps_image_upload_without_exposing_the_nos_token() {
        let allocation: ImageUploadAllocationEnvelope = serde_json::from_value(json!({
            "result": {
                "objectKey": "109951168/avatar.jpg",
                "token": "secret-nos-token",
                "docId": "109951168000000000"
            }
        }))
        .expect("image allocation fixture");
        let request = ImageUploadRequest {
            filename: "avatar.png".to_owned(),
            content_type: "image/png".to_owned(),
            data: vec![1, 2, 3],
            image_size: Some(300),
            crop_x: Some(0),
            crop_y: Some(0),
            account: Some("personal".to_owned()),
        };

        let result = map_image_upload_result(
            &request,
            allocation,
            json!({"code": 200, "size": "3"}),
            json!({
                "code": 200,
                "data": {"url": " "},
                "url": "https://p1.music.126.net/final-avatar.jpg"
            }),
        )
        .expect("map image upload result");

        assert_eq!(
            result.url.as_deref(),
            Some("https://p1.music.126.net/final-avatar.jpg")
        );
        assert_eq!(result.image_id.as_deref(), Some("109951168000000000"));
        assert_eq!(result.extensions["upload_response"]["size"], "3");
        assert_eq!(
            result.extensions["reference_crop_parameters"]["applied"],
            false
        );
        assert!(
            !serde_json::to_string(&result)
                .expect("serialize image upload result")
                .contains("secret-nos-token")
        );
    }

    #[test]
    fn playlist_cover_update_request_matches_reference_protocol() {
        let (path, payload) =
            netease_playlist_cover_update_request(2_039_116_066, &json!("109951168000000000"));

        assert_eq!(path, "/api/playlist/cover/update");
        assert_eq!(
            payload,
            json!({
                "id": 2_039_116_066_u64,
                "coverImgId": "109951168000000000"
            })
        );
    }

    #[test]
    fn maps_netease_playlist_to_unified_model() {
        let playlist: PlaylistDetail = serde_json::from_value(json!({
            "id": 3778678,
            "name": "云音乐热歌榜",
            "description": "热门歌曲",
            "coverImgUrl": "https://example.test/playlist.jpg",
            "creator": {"userId": 1, "nickname": "网易云音乐"},
            "trackCount": 2,
            "tags": ["流行"],
            "subscribed": false,
            "createTime": 1378721408222_u64,
            "updateTime": 1783987200000_u64,
            "privacy": 0,
            "specialType": 10,
            "playCount": 12345,
            "trackIds": [{"id": 185809}, {"id": 186001}]
        }))
        .expect("valid playlist fixture");

        let playlist = map_playlist(playlist).expect("map playlist");
        assert_eq!(playlist.resource_ref.to_string(), "netease:3778678");
        assert_eq!(playlist.creator.expect("creator").name, "网易云音乐");
        assert_eq!(playlist.track_count, Some(2));
        assert_eq!(playlist.extensions["special_type"], 10);
    }

    #[test]
    fn maps_netease_album_to_the_unified_model() {
        let album: AlbumDetail = serde_json::from_value(json!({
            "id": 18915,
            "name": "Jay",
            "alias": ["周杰伦首专"],
            "artists": [{"id": 6452, "name": "周杰伦"}],
            "description": "周杰伦首张专辑",
            "picUrl": "https://example.test/jay.jpg",
            "publishTime": 968428800000_u64,
            "size": 10,
            "company": "杰威尔",
            "type": "专辑",
            "subType": "录音室版",
            "paid": false,
            "onSale": true,
            "mark": 0
        }))
        .expect("album fixture");
        let album = map_album(album).expect("map album");
        assert_eq!(album.resource_ref.to_string(), "netease:18915");
        assert_eq!(album.artists[0].name, "周杰伦");
        assert_eq!(album.track_count, Some(10));
        assert_eq!(album.extensions["sub_type"], "录音室版");
        assert!(album.published_at.is_some());
    }

    #[test]
    fn maps_netease_album_catalog_items_without_losing_upstream_fields() {
        let album = map_album_list_item(json!({
            "id": 387169747,
            "name": "小海子村儿",
            "alias": [],
            "artists": [
                {"id": 2515, "name": "窦唯"},
                {"id": 33154502, "name": "朝简"}
            ],
            "description": "",
            "picUrl": "https://example.test/album.jpg",
            "publishTime": 1784163600000_u64,
            "size": 1,
            "company": "北京窦唯音乐工作室",
            "type": "专辑",
            "subType": "录音室版",
            "paid": false,
            "onSale": false,
            "mark": 0,
            "copyrightId": 2717412,
            "commentThreadId": "R_AL_3_387169747"
        }))
        .expect("map album catalog item");

        assert_eq!(album.resource_ref.to_string(), "netease:387169747");
        assert_eq!(album.artists.len(), 2);
        assert_eq!(album.track_count, Some(1));
        assert_eq!(album.company.as_deref(), Some("北京窦唯音乐工作室"));
        assert_eq!(album.extensions["catalog_item"]["copyrightId"], 2717412);
    }

    #[test]
    fn maps_netease_artist_catalog_filters_and_items() {
        assert_eq!(netease_artist_category(ArtistCategory::Male), 1);
        assert_eq!(netease_artist_category(ArtistCategory::Group), 3);
        assert_eq!(netease_artist_area(ArtistArea::Western), 96);
        assert_eq!(netease_artist_area(ArtistArea::Korean), 16);
        assert_eq!(
            netease_artist_initial(Some("b")).expect("letter initial"),
            Some(66)
        );
        assert_eq!(
            netease_artist_initial(Some("hot")).expect("hot initial"),
            Some(-1)
        );
        assert_eq!(
            netease_artist_initial(Some("#")).expect("other initial"),
            Some(0)
        );
        assert_eq!(netease_artist_initial(None).expect("missing initial"), None);
        let error = netease_artist_initial(Some("中文")).expect_err("invalid initial");
        assert_eq!(error.code, ErrorCode::InvalidRequest);

        let response: ArtistListEnvelope = serde_json::from_value(json!({
            "artists": [
                {
                    "id": 178059,
                    "name": "Bruno Mars",
                    "alias": [],
                    "transNames": ["布鲁诺·马尔斯"],
                    "trans": "布鲁诺·马尔斯",
                    "briefDesc": "歌手简介",
                    "img1v1Url": "https://example.test/avatar.jpg",
                    "picUrl": "https://example.test/cover.jpg",
                    "albumSize": 50,
                    "musicSize": 959,
                    "followed": false,
                    "accountId": 1671465495
                }
            ],
            "more": true
        }))
        .expect("artist list fixture");

        let page = map_artist_list_response(response, 1, 0).expect("map artist list");

        assert_eq!(page.items[0].resource_ref.to_string(), "netease:178059");
        assert_eq!(page.items[0].aliases, ["布鲁诺·马尔斯"]);
        assert_eq!(page.items[0].album_count, Some(50));
        assert_eq!(page.items[0].track_count, Some(959));
        assert_eq!(
            page.items[0].extensions["catalog_item"]["accountId"],
            1671465495
        );
        assert_eq!(page.pagination.next_offset, Some(1));
        assert!(page.pagination.has_more);
    }

    #[test]
    fn maps_netease_artist_mvs_to_the_unified_video_model() {
        let response: ArtistMvsEnvelope = serde_json::from_value(json!({
            "hasMore": true,
            "mvs": [
                {
                    "id": 22695250,
                    "name": "任性 (5525 Live版)",
                    "artist": {
                        "id": 6452,
                        "name": "周杰伦",
                        "img1v1Url": "https://example.test/artist.jpg"
                    },
                    "artists": [
                        {
                            "id": 6452,
                            "name": "周杰伦",
                            "img1v1Url": "https://example.test/artist.jpg"
                        },
                        {
                            "id": 13193,
                            "name": "五月天",
                            "img1v1Url": "https://example.test/collaborator.jpg"
                        }
                    ],
                    "artistName": "周杰伦",
                    "duration": 266000,
                    "imgurl": "https://example.test/square.jpg",
                    "imgurl16v9": "https://example.test/wide.jpg",
                    "playCount": 100726,
                    "publishTime": "2025-02-23",
                    "status": 0,
                    "subed": false
                }
            ],
            "time": 1469635200007_u64
        }))
        .expect("artist MV fixture");

        let page = map_artist_mvs_response(response, 1, 0).expect("map artist MVs");

        assert_eq!(page.items[0].resource_ref.to_string(), "netease:22695250");
        assert_eq!(
            page.items[0].creators[0]
                .resource_ref
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("netease:6452")
        );
        assert_eq!(page.items[0].creators.len(), 2);
        assert_eq!(page.items[0].creators[1].name, "五月天");
        assert_eq!(
            page.items[0].cover_url.as_deref(),
            Some("https://example.test/wide.jpg")
        );
        assert_eq!(page.items[0].duration_ms, Some(266_000));
        assert_eq!(page.items[0].published_at.as_deref(), Some("2025-02-23"));
        assert_eq!(page.items[0].play_count, Some(100_726));
        assert_eq!(page.items[0].extensions["mv"]["status"], 0);
        assert_eq!(page.pagination.extensions["time"], 1469635200007_u64);
        assert_eq!(page.pagination.next_offset, Some(1));
        assert!(page.pagination.has_more);
    }

    #[test]
    fn artist_mv_mapping_falls_back_after_blank_creator_and_cover_summaries() {
        let video = map_artist_mv(json!({
            "id": 22695250,
            "name": "任性",
            "artist": {
                "id": 6452,
                "name": "周杰伦",
                "img1v1Url": "https://example.test/artist.jpg"
            },
            "artists": [{"id": 0, "name": ""}],
            "imgurl16v9": "  ",
            "imgurl": "https://example.test/square.jpg"
        }))
        .expect("fall back from blank MV summaries");

        assert_eq!(video.creators.len(), 1);
        assert_eq!(video.creators[0].name, "周杰伦");
        assert_eq!(
            video.cover_url.as_deref(),
            Some("https://example.test/square.jpg")
        );
    }

    #[test]
    fn maps_followed_artist_catalog_and_subscription_metadata() {
        let response: ArtistSublistEnvelope = serde_json::from_value(json!({
            "data": [
                {
                    "id": 6452,
                    "name": "周杰伦",
                    "alias": ["Jay Chou"],
                    "transNames": [],
                    "briefDesc": "华语男歌手",
                    "img1v1Url": "https://example.test/avatar.jpg",
                    "picUrl": "https://example.test/cover.jpg",
                    "albumSize": 44,
                    "musicSize": 568,
                    "mvSize": 9,
                    "followed": true,
                    "subTime": 1_720_000_000_000_u64
                }
            ],
            "count": 8,
            "hasMore": true
        }))
        .expect("followed artists fixture");

        let page =
            map_artist_sublist_response(response, 1, 2).expect("map followed artist catalog");

        assert_eq!(page.items[0].resource_ref.to_string(), "netease:6452");
        assert_eq!(page.items[0].aliases, ["Jay Chou"]);
        assert_eq!(page.items[0].album_count, Some(44));
        assert_eq!(page.items[0].track_count, Some(568));
        assert_eq!(page.items[0].mv_count, Some(9));
        assert_eq!(
            page.items[0].extensions["following_item"]["subTime"],
            1_720_000_000_000_u64
        );
        assert!(!page.items[0].extensions.contains_key("catalog_item"));
        assert_eq!(page.pagination.total, Some(8));
        assert_eq!(page.pagination.next_offset, Some(3));
        assert!(page.pagination.has_more);
    }

    #[test]
    fn maps_netease_artist_tracks_and_pagination() {
        let response: ArtistTracksEnvelope = serde_json::from_value(json!({
            "songs": [
                {
                    "id": 298317,
                    "name": "屋顶",
                    "alia": [],
                    "ar": [
                        {"id": 6452, "name": "周杰伦"},
                        {"id": 7219, "name": "温岚"}
                    ],
                    "al": {
                        "id": 32311,
                        "name": "吴宗宪的台语歌",
                        "picUrl": "https://example.test/cover.jpg"
                    },
                    "dt": 319000,
                    "mv": 0,
                    "fee": 8,
                    "st": 0,
                    "mark": 524288,
                    "privilege": {
                        "id": 298317,
                        "st": 0,
                        "fee": 8,
                        "pl": 320000,
                        "maxbr": 999000
                    },
                    "copyright": 1
                }
            ],
            "more": true,
            "total": 566
        }))
        .expect("artist tracks fixture");

        let page = map_artist_tracks_response(response, 1, 20).expect("map artist tracks");

        assert_eq!(page.items[0].resource_ref.to_string(), "netease:298317");
        assert_eq!(page.items[0].name, "屋顶");
        assert_eq!(page.items[0].artists.len(), 2);
        assert_eq!(page.items[0].artists[0].name, "周杰伦");
        assert_eq!(
            page.items[0]
                .album
                .as_ref()
                .map(|album| album.name.as_str()),
            Some("吴宗宪的台语歌")
        );
        assert_eq!(page.items[0].duration_ms, Some(319_000));
        assert_eq!(page.items[0].playable, Some(true));
        assert_eq!(page.items[0].extensions["artist_track"]["copyright"], 1);
        assert_eq!(page.pagination.total, Some(566));
        assert_eq!(page.pagination.next_offset, Some(21));
        assert!(page.pagination.has_more);
    }

    #[test]
    fn maps_netease_artist_top_tracks_as_a_fixed_snapshot() {
        let raw = json!({
            "songs": [
                {
                    "id": 185809,
                    "name": "晴天",
                    "alia": [],
                    "ar": [{"id": 6452, "name": "周杰伦"}],
                    "al": {
                        "id": 18905,
                        "name": "叶惠美",
                        "picUrl": "https://example.test/cover.jpg"
                    },
                    "dt": 269000,
                    "mv": 186001,
                    "fee": 1,
                    "st": 0,
                    "copyright": 2
                }
            ],
            "privileges": [
                {
                    "id": 185809,
                    "st": 0,
                    "fee": 1,
                    "pl": 320000,
                    "maxbr": 999000
                }
            ],
            "code": 200
        });
        let response: ArtistTopTracksEnvelope =
            serde_json::from_value(raw.clone()).expect("artist top tracks fixture");

        let page = map_artist_top_tracks_response(response, raw).expect("map artist top tracks");

        assert_eq!(page.items[0].resource_ref.to_string(), "netease:185809");
        assert_eq!(page.items[0].name, "晴天");
        assert_eq!(page.items[0].artists[0].name, "周杰伦");
        assert_eq!(page.items[0].playable, Some(true));
        assert_eq!(
            page.items[0].extensions["privilege"]["play_bitrate"],
            320000
        );
        assert_eq!(page.items[0].extensions["artist_top_track"]["copyright"], 2);
        assert_eq!(page.pagination.limit, 50);
        assert_eq!(page.pagination.total, Some(1));
        assert_eq!(page.pagination.next_offset, None);
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.extensions["response"]["code"], 200);
    }

    #[test]
    fn maps_netease_artist_videos_and_cursor_to_the_unified_video_model() {
        let response: ArtistVideosEnvelope = serde_json::from_value(json!({
            "data": {
                "page": { "cursor": "2", "more": true, "size": 1 },
                "records": [
                    {
                        "id": "record-summary-id",
                        "type": 1,
                        "resource": {
                            "mlogBaseData": {
                                "id": "22695250",
                                "text": "任性 (5525 Live版)",
                                "desc": "现场版",
                                "coverUrl": "https://example.test/video.jpg",
                                "duration": 266000,
                                "pubTime": 1740377057300_u64
                            },
                            "mlogExtVO": {
                                "artistName": "周杰伦",
                                "artists": [
                                    {
                                        "id": 6452,
                                        "name": "周杰伦",
                                        "img1v1Url": "https://example.test/artist.jpg"
                                    }
                                ],
                                "playCount": 100726
                            },
                            "userProfile": null
                        }
                    }
                ]
            }
        }))
        .expect("artist videos fixture");

        let page = map_artist_videos_response(response, 1, 0).expect("map artist videos");

        assert_eq!(page.items[0].resource_ref.to_string(), "netease:22695250");
        assert_eq!(page.items[0].title, "任性 (5525 Live版)");
        assert_eq!(page.items[0].description, "现场版");
        assert_eq!(
            page.items[0].creators[0]
                .resource_ref
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("netease:6452")
        );
        assert_eq!(page.items[0].duration_ms, Some(266_000));
        assert!(
            page.items[0]
                .published_at
                .as_deref()
                .is_some_and(|published_at| published_at.starts_with("2025-02-24T"))
        );
        assert_eq!(page.items[0].play_count, Some(100_726));
        assert_eq!(page.items[0].extensions["artist_video"]["type"], 1);
        assert_eq!(page.pagination.extensions["next_cursor"], "2");
        assert_eq!(page.pagination.extensions["page_size"], 1);
        assert_eq!(page.pagination.next_offset, Some(1));
        assert!(page.pagination.has_more);
    }

    #[test]
    fn maps_mv_detail_stats_resolutions_and_stream_without_losing_responses() {
        let detail_raw = json!({
            "code": 200,
            "subed": false,
            "data": {
                "id": 22695250,
                "name": "任性 (5525 Live版)",
                "artistId": 6452,
                "artistName": "周杰伦",
                "artists": [
                    {"id": 6452, "name": "周杰伦", "followed": false},
                    {"id": 13193, "name": "五月天", "followed": false}
                ],
                "briefDesc": "",
                "desc": "回到那一天",
                "cover": "https://example.test/mv.jpg",
                "duration": 266000,
                "publishTime": "2025-02-23",
                "playCount": 100751,
                "brs": [
                    {"br": 0, "resolution": 240, "format": " ", "container": "mp4", "size": 17851236},
                    {"br": 480, "size": 28558850},
                    {"br": 720, "size": 44613276},
                    {"br": 1080, "size": 177950112, "future": true}
                ],
                "videoGroup": [{"id": 58100, "name": "现场"}]
            }
        });
        let detail: MvDetailEnvelope = parse_body(detail_raw.clone()).expect("MV detail fixture");
        let detail = map_mv_detail(detail, detail_raw).expect("map MV detail");
        assert_eq!(detail.kind, VideoResourceKind::Mv);
        assert_eq!(detail.video.resource_ref.to_string(), "netease:22695250");
        assert_eq!(detail.video.title, "任性 (5525 Live版)");
        assert_eq!(detail.video.creators.len(), 2);
        assert_eq!(
            detail.video.creators[1]
                .resource_ref
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("netease:13193")
        );
        assert_eq!(detail.video.duration_ms, Some(266_000));
        assert_eq!(detail.video.subscribed, Some(false));
        assert_eq!(detail.resolutions.len(), 4);
        assert_eq!(detail.resolutions[0].resolution, 240);
        assert_eq!(detail.resolutions[0].format.as_deref(), Some("mp4"));
        assert_eq!(detail.resolutions[3].resolution, 1080);
        assert_eq!(detail.resolutions[3].size, Some(177_950_112));
        assert_eq!(
            detail.resolutions[3].extensions["resolution"]["future"],
            true
        );
        assert_eq!(
            detail.extensions["response"]["data"]["videoGroup"][0]["id"],
            58100
        );

        let stats_raw = json!({
            "code": 200,
            "liked": false,
            "likedCount": 4662,
            "commentCount": 675,
            "shareCount": 1399
        });
        let stats: VideoStatsEnvelope = parse_body(stats_raw.clone()).expect("MV stats fixture");
        let stats = map_video_stats("22695250", VideoResourceKind::Mv, stats, stats_raw)
            .expect("map MV stats");
        assert_eq!(stats.video_ref.to_string(), "netease:22695250");
        assert_eq!(stats.like_count, Some(4_662));
        assert_eq!(stats.comment_count, Some(675));
        assert_eq!(stats.share_count, Some(1_399));

        let stream_raw = json!({
            "code": 200,
            "data": {
                "code": 200,
                "expi": 0,
                "validity": 3600,
                "fee": 0,
                "id": 22695250,
                "md5": "abcdef",
                "msg": "",
                "mvFee": 0,
                "r": 0,
                "resolution": 1080,
                "size": 177950120,
                "url": "https://example.test/live.mv.mp4?token=secret"
            }
        });
        let stream: MvUrlEnvelope = parse_body(stream_raw.clone()).expect("MV stream fixture");
        let request = VideoStreamRequest::new(VideoResourceKind::Mv, 1080);
        let stream =
            map_video_stream("22695250", &request, stream.data, stream_raw).expect("map MV stream");
        assert!(stream.available);
        assert_eq!(stream.format.as_deref(), Some("mp4"));
        assert_eq!(stream.actual_resolution, Some(1080));
        assert_eq!(stream.size, Some(177_950_120));
        assert_eq!(stream.platform_code, Some(200));
        assert_eq!(stream.extensions["md5"], "abcdef");
        assert!(stream.expires_at.is_some());
    }

    #[test]
    fn maps_cloud_video_detail_and_stream_and_validates_resource_boundaries() {
        let id = "D1C2B3A40987654321ABCDEF12345678";
        let detail_raw = json!({
            "code": 200,
            "data": {
                "vid": id,
                "title": "云村视频",
                "description": "视频简介",
                "coverUrl": "https://example.test/video.jpg",
                "publishTime": 1_720_000_000_000_u64,
                "durationms": 123456,
                "playTime": 9876,
                "subed": true,
                "creator": {
                    "userId": 32953014,
                    "nickname": "Lotus",
                    "avatarUrl": "https://example.test/avatar.jpg"
                },
                "resolutions": [
                    {"resolution": 480, "size": 1234567},
                    {"resolution": 1080, "width": 1920, "height": 1080, "size": 7654321}
                ],
                "videoGroup": [{"id": 1101, "name": "舞蹈"}]
            }
        });
        let detail: CloudVideoDetailEnvelope =
            parse_body(detail_raw.clone()).expect("cloud video detail fixture");
        let detail = map_cloud_video_detail(detail, detail_raw).expect("map cloud video detail");
        assert_eq!(detail.kind, VideoResourceKind::Video);
        assert_eq!(
            detail.video.resource_ref.to_string(),
            format!("netease:{id}")
        );
        assert_eq!(detail.video.creators[0].name, "Lotus");
        assert_eq!(
            detail.video.creators[0]
                .resource_ref
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("netease:32953014")
        );
        assert_eq!(detail.video.duration_ms, Some(123_456));
        assert_eq!(detail.resolutions[1].width, Some(1920));
        assert_eq!(detail.resolutions[1].height, Some(1080));
        assert_eq!(
            detail.extensions["response"]["data"]["videoGroup"][0]["name"],
            "舞蹈"
        );

        let stream_raw = json!({
            "code": 200,
            "urls": [{
                "id": id,
                "url": "https://example.test/video.m3u8",
                "size": 7654321,
                "validity": 1200,
                "resolution": 1080,
                "code": 200,
                "needPay": false
            }]
        });
        let stream: CloudVideoUrlEnvelope =
            parse_body(stream_raw.clone()).expect("cloud video stream fixture");
        let request = VideoStreamRequest::new(VideoResourceKind::Video, 1080);
        let stream = map_video_stream(
            id,
            &request,
            stream.urls.into_iter().next().expect("one stream"),
            stream_raw,
        )
        .expect("map cloud video stream");
        assert!(stream.available);
        assert_eq!(stream.format.as_deref(), Some("m3u8"));
        assert_eq!(stream.actual_resolution, Some(1080));

        assert_eq!(
            validated_video_id("22695250", VideoResourceKind::Mv).expect("valid MV"),
            "22695250"
        );
        assert_eq!(
            validated_video_id(id, VideoResourceKind::Video).expect("valid video"),
            id
        );
        assert_eq!(
            validated_video_id(id, VideoResourceKind::Mv)
                .expect_err("opaque MV id")
                .code,
            ErrorCode::InvalidRequest
        );
        for resolution in [0, 4_321] {
            assert_eq!(
                validated_video_resolution(resolution)
                    .expect_err("invalid resolution")
                    .code,
                ErrorCode::InvalidRequest
            );
        }
    }

    #[test]
    fn maps_followed_artist_new_videos_and_timestamp_cursor() {
        let raw = json!({
            "code": 200,
            "data": {
                "hasMore": true,
                "newWorks": [
                    {
                        "id": "work-summary-id",
                        "mvId": 1099001,
                        "name": "作品摘要",
                        "mvName": "新 MV",
                        "cover": "https://example.test/work-summary.jpg",
                        "mvCoverUrl": "https://example.test/new-mv.jpg",
                        "playCount": 3456,
                        "desc": " ",
                        "briefDesc": "关注歌手更新",
                        "artistName": "周杰伦",
                        "artistImgUrl": "https://example.test/artist.jpg",
                        "artistId": 6452,
                        "duration": 210000,
                        "publishTime": 1_720_000_000_000_u64,
                        "publishDate": "2024-07-03"
                    }
                ]
            }
        });
        let response: ArtistNewVideosEnvelope =
            serde_json::from_value(raw.clone()).expect("new artist videos fixture");

        let page = map_artist_new_videos_response(response, raw, 1, 1_730_000_000_000)
            .expect("map followed artist videos");

        assert_eq!(page.items[0].resource_ref.to_string(), "netease:1099001");
        assert_eq!(page.items[0].title, "新 MV");
        assert_eq!(page.items[0].description, "关注歌手更新");
        assert_eq!(
            page.items[0].cover_url.as_deref(),
            Some("https://example.test/new-mv.jpg")
        );
        assert_eq!(
            page.items[0].creators[0]
                .resource_ref
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("netease:6452")
        );
        assert_eq!(page.items[0].published_at.as_deref(), Some("2024-07-03"));
        assert_eq!(page.items[0].play_count, Some(3_456));
        assert_eq!(
            page.items[0].extensions["artist_new_video"]["artistId"],
            6452
        );
        assert_eq!(
            page.pagination.extensions["before_ms"],
            1_730_000_000_000_u64
        );
        assert_eq!(
            page.pagination.extensions["next_before_ms"],
            1_720_000_000_000_u64
        );
        assert_eq!(page.pagination.extensions["response"]["code"], 200);
        assert!(page.pagination.has_more);
    }

    #[test]
    fn maps_followed_artist_new_tracks_and_timestamp_cursor() {
        let raw = json!({
            "code": 200,
            "data": {
                "hasMore": true,
                "newSongCount": 3,
                "newWorks": [
                    {
                        "albumId": 3099001,
                        "blockType": "SONG",
                        "publishTime": 1_720_000_000_000_u64,
                        "blockTitle": { "artistName": "周杰伦" },
                        "songLists": [
                            {
                                "id": 2099001,
                                "name": "新歌",
                                "alia": ["New Song"],
                                "ar": [{ "id": 6452, "name": "周杰伦" }],
                                "al": {
                                    "id": 3099001,
                                    "name": "新专辑",
                                    "picUrl": "https://example.test/new-album.jpg"
                                },
                                "dt": 208000,
                                "mv": 1099001
                            },
                            {
                                "id": 2099002,
                                "name": "第二首新歌",
                                "ar": [{ "id": 6452, "name": "周杰伦" }],
                                "al": { "id": 3099001, "name": "新专辑" },
                                "dt": 198000,
                                "mv": 0
                            }
                        ]
                    }
                ]
            }
        });
        let response: ArtistNewTracksEnvelope =
            serde_json::from_value(raw.clone()).expect("new artist tracks fixture");

        let page = map_artist_new_tracks_response(response, raw, 1, 1_730_000_000_000)
            .expect("map followed artist tracks");

        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].resource_ref.to_string(), "netease:2099001");
        assert_eq!(page.items[1].resource_ref.to_string(), "netease:2099002");
        assert_eq!(page.items[0].name, "新歌");
        assert_eq!(page.items[0].artists[0].name, "周杰伦");
        assert_eq!(
            page.items[0]
                .album
                .as_ref()
                .and_then(|album| album.resource_ref.as_ref())
                .map(ToString::to_string)
                .as_deref(),
            Some("netease:3099001")
        );
        assert_eq!(
            page.items[0]
                .mv_ref
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("netease:1099001")
        );
        assert_eq!(page.items[0].extensions["artist_new_track"]["id"], 2099001);
        assert_eq!(
            page.items[0].extensions["artist_new_track_block"]["publishTime"],
            1_720_000_000_000_u64
        );
        assert_eq!(page.pagination.total, Some(3));
        assert_eq!(
            page.pagination.extensions["next_before_ms"],
            1_720_000_000_000_u64
        );
        assert_eq!(page.pagination.extensions["limit_unit"], "work_blocks");
        assert_eq!(page.pagination.extensions["work_block_count"], 1);
        assert!(page.pagination.has_more);
    }

    #[test]
    fn maps_followed_artist_mixed_works_and_preserves_unknown_sources() {
        let raw = json!({
            "code": 200,
            "data": {
                "hasMore": true,
                "latestVisitTime": 1_730_000_000_000_u64,
                "newWorks": [
                    {
                        "sourceType": 1,
                        "publishTime": 1_720_000_000_000_u64,
                        "info": {
                            "blockTitle": {
                                "artistName": "周杰伦",
                                "artistId": 6452,
                                "imgUrl": "https://example.test/artist.jpg",
                                "publishDate": "2024-07-03",
                                "resourcePicUrl": "https://example.test/new-album.jpg",
                                "resourceName": "新专辑"
                            },
                            "blockType": "SONG",
                            "songLists": [
                                {
                                    "id": 2099001,
                                    "name": "新歌",
                                    "artists": [{ "id": 6452, "name": "周杰伦" }],
                                    "album": { "id": 3099001, "name": "新专辑" },
                                    "duration": 208000,
                                    "mvid": 0
                                }
                            ]
                        }
                    },
                    {
                        "sourceType": 9,
                        "publishTime": 1_710_000_000_000_u64,
                        "info": {
                            "blockType": "FUTURE_RESOURCE",
                            "blockTitle": { "resourceName": "未知作品" },
                            "futurePayload": { "kept": true }
                        }
                    }
                ]
            }
        });
        let response: ArtistNewWorksEnvelope =
            serde_json::from_value(raw.clone()).expect("mixed artist works fixture");
        let mut request = ArtistWorksRequest::new(2);
        request.before_ms = Some(1_740_000_000_000);
        request.first_request = false;

        let page = map_artist_new_works_response(response, raw, &request, 2, 1_740_000_000_000)
            .expect("map mixed artist works");

        assert_eq!(page.items[0].kind, ArtistWorkKind::Track);
        assert_eq!(
            page.items[0].tracks[0].resource_ref.to_string(),
            "netease:2099001"
        );
        assert_eq!(
            page.items[0]
                .artist
                .as_ref()
                .and_then(|artist| artist.resource_ref.as_ref())
                .map(ToString::to_string)
                .as_deref(),
            Some("netease:6452")
        );
        assert_eq!(page.items[0].title.as_deref(), Some("新专辑"));
        assert_eq!(page.items[1].kind, ArtistWorkKind::Unknown);
        assert_eq!(page.items[1].source_type, 9);
        assert_eq!(
            page.items[1].extensions["artist_work"]["info"]["futurePayload"]["kept"],
            true
        );
        assert_eq!(
            page.pagination.extensions["next_before_ms"],
            1_710_000_000_000_u64
        );
        assert_eq!(
            page.pagination.extensions["latest_visit_time"],
            1_730_000_000_000_u64
        );
        assert_eq!(page.pagination.extensions["first_request"], false);
        assert!(page.pagination.has_more);
    }

    #[test]
    fn artist_work_mapping_prefers_real_nonempty_resources_over_empty_aliases_and_hints() {
        let resources = json!({
            "songLists": [],
            "songList": null,
            "songs": [{"id": 2099001}]
        });
        let selected = artist_work_resources(&resources, &["songLists", "songList", "songs"])
            .expect("select present artist work resource list");
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0]["id"], 2099001);

        assert_eq!(
            artist_work_kind(true, false, "video"),
            ArtistWorkKind::Track
        );
        assert_eq!(artist_work_kind(false, true, "song"), ArtistWorkKind::Video);
        assert_eq!(artist_work_kind(true, true, "song"), ArtistWorkKind::Mixed);
        assert_eq!(
            artist_work_kind(false, false, "song_video"),
            ArtistWorkKind::Mixed
        );
        assert_eq!(
            artist_work_kind(false, false, "future_resource"),
            ArtistWorkKind::Unknown
        );

        let update = map_artist_work_update(
            json!({
                "info": {
                    "blockTitle": {
                        "resourcePicUrl": " ",
                        "imgUrl": "https://example.test/artist-work.jpg"
                    }
                }
            }),
            0,
        )
        .expect("skip blank preferred artist work cover");
        assert_eq!(
            update.cover_url.as_deref(),
            Some("https://example.test/artist-work.jpg")
        );
    }

    #[test]
    fn maps_followed_artist_new_tracks_play_all_snapshot() {
        let raw = json!({
            "code": 200,
            "data": {
                "count": 1,
                "songList": [
                    {
                        "id": 2099001,
                        "name": "新歌",
                        "artists": [{ "id": 6452, "name": "周杰伦" }],
                        "album": { "id": 3099001, "name": "新专辑" },
                        "duration": 208000,
                        "mvid": 0
                    }
                ]
            }
        });
        let response: ArtistNewTracksPlayAllEnvelope =
            serde_json::from_value(raw.clone()).expect("play-all fixture");

        let page = map_artist_new_tracks_play_all_response(response, raw)
            .expect("map new tracks play-all");

        assert_eq!(page.items[0].resource_ref.to_string(), "netease:2099001");
        assert_eq!(page.items[0].name, "新歌");
        assert_eq!(
            page.items[0].extensions["artist_new_track_play_all"]["album"]["id"],
            3099001
        );
        assert_eq!(page.pagination.limit, 50);
        assert_eq!(page.pagination.total, Some(1));
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.extensions["response"]["code"], 200);
    }

    #[test]
    fn maps_netease_artist_albums_and_cursor_metadata() {
        let response: ArtistAlbumsEnvelope = serde_json::from_value(json!({
            "artist": { "id": 6452, "name": "周杰伦", "albumSize": 42 },
            "hotAlbums": [
                {
                    "id": 18915,
                    "name": "Jay",
                    "artists": [{ "id": 6452, "name": "周杰伦" }],
                    "picUrl": "https://example.test/jay.jpg",
                    "publishTime": 968428800000_u64,
                    "size": 10,
                    "copyrightId": 1007
                },
                {
                    "id": 18914,
                    "name": "范特西",
                    "artists": [{ "id": 6452, "name": "周杰伦" }],
                    "size": 10
                }
            ],
            "more": true
        }))
        .expect("artist albums fixture");
        let page = map_artist_albums_response(response, 2, 5).expect("map artist albums");

        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].resource_ref.to_string(), "netease:18915");
        assert_eq!(page.items[0].artists[0].name, "周杰伦");
        assert_eq!(
            page.items[0].extensions["artist_album_item"]["copyrightId"],
            1007
        );
        assert_eq!(page.pagination.offset, 5);
        assert_eq!(page.pagination.total, None);
        assert_eq!(page.pagination.next_offset, Some(7));
        assert!(page.pagination.has_more);
        assert_eq!(page.pagination.extensions["artist"]["albumSize"], 42);
    }

    #[test]
    fn maps_netease_artist_fans_to_users_and_preserves_profile_metadata() {
        let response: ArtistFansEnvelope = serde_json::from_value(json!({
            "code": 200,
            "data": [
                {
                    "userProfile": {
                        "userId": 6298206519_u64,
                        "nickname": "轻手揍人丸",
                        "avatarUrl": "https://example.test/avatar.jpg",
                        "signature": "111",
                        "followed": false,
                        "mutual": true,
                        "province": 350000,
                        "city": 350500,
                        "gender": 2
                    },
                    "vipRights": {
                        "redVipLevel": 0,
                        "redVipAnnualCount": -1
                    }
                }
            ],
            "hasMore": true,
            "count": 13704933
        }))
        .expect("artist fans fixture");

        let page = map_artist_fans_response(response, 1, 10).expect("map artist fans");

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].resource_ref.to_string(), "netease:6298206519");
        assert_eq!(page.items[0].name, "轻手揍人丸");
        assert_eq!(page.items[0].signature.as_deref(), Some("111"));
        assert_eq!(page.items[0].mutual, Some(true));
        assert_eq!(
            page.items[0].extensions["fan"]["userProfile"]["province"],
            350000
        );
        assert_eq!(
            page.items[0].extensions["fan"]["vipRights"]["redVipLevel"],
            0
        );
        assert_eq!(page.pagination.total, Some(13_704_933));
        assert_eq!(page.pagination.next_offset, Some(11));
        assert!(page.pagination.has_more);
    }

    #[test]
    fn maps_netease_artist_detail_and_description_without_losing_extensions() {
        let detail_raw = json!({
            "code": 200,
            "data": {
                "artist": {
                    "id": 6452,
                    "name": "周杰伦",
                    "alias": ["Jay Chou", "周董", "Jay Chou"],
                    "transNames": ["Chou Chieh-lun"],
                    "briefDesc": "详情简介",
                    "avatar": "https://example.test/avatar.jpg",
                    "cover": "https://example.test/cover.jpg",
                    "albumSize": 44,
                    "musicSize": 568,
                    "mvSize": 9,
                    "identities": ["作曲"]
                },
                "videoCount": 8,
                "identify": {"imageDesc": "歌手、作词、作曲、编曲、制作人、乐手"},
                "blacklist": true
            }
        });
        let description_raw = json!({
            "code": 200,
            "briefDesc": "传记简介",
            "introduction": [
                {"ti": "人物简介", "txt": "人物经历"},
                {"ti": "代表作品", "txt": "范特西"}
            ],
            "topicData": [{"mainTitle": "专题原始字段"}]
        });
        let detail: ArtistDetailEnvelope =
            serde_json::from_value(detail_raw.clone()).expect("artist detail fixture");
        let description: ArtistDescriptionEnvelope =
            serde_json::from_value(description_raw.clone()).expect("artist description fixture");

        let artist = map_artist(detail, description, detail_raw, description_raw)
            .expect("map artist detail");

        assert_eq!(artist.resource_ref.to_string(), "netease:6452");
        assert_eq!(artist.name, "周杰伦");
        assert_eq!(artist.aliases, ["Jay Chou", "周董", "Chou Chieh-lun"]);
        assert_eq!(artist.description, "传记简介");
        assert_eq!(artist.biography_sections.len(), 2);
        assert_eq!(artist.album_count, Some(44));
        assert_eq!(artist.track_count, Some(568));
        assert_eq!(artist.video_count, Some(8));
        assert_eq!(
            artist.extensions["detail_response"]["data"]["identify"]["imageDesc"],
            "歌手、作词、作曲、编曲、制作人、乐手"
        );
        assert_eq!(
            artist.extensions["description_response"]["topicData"][0]["mainTitle"],
            "专题原始字段"
        );
    }

    #[test]
    fn maps_legacy_artist_overview_without_collapsing_featured_tracks() {
        let raw = json!({
            "artist": {
                "id": 6452,
                "name": "周杰伦",
                "alias": ["Jay Chou"],
                "briefDesc": "歌手简介",
                "img1v1Url": "https://example.test/avatar.jpg",
                "picUrl": "https://example.test/cover.jpg",
                "albumSize": 44,
                "musicSize": 568,
                "mvSize": 9,
                "followed": false,
                "publishTime": 1_784_000_000_000_u64
            },
            "hotSongs": [
                {
                    "id": 210049,
                    "name": "布拉格广场",
                    "alia": [],
                    "ar": [
                        {"id": 7217, "name": "蔡依林"},
                        {"id": 6452, "name": "周杰伦"}
                    ],
                    "al": {"id": 18877, "name": "看我72变"},
                    "dt": 294600,
                    "mv": 186004,
                    "fee": 1,
                    "st": 0,
                    "copyright": 2
                }
            ],
            "more": true,
            "code": 200
        });
        let response: ArtistOverviewEnvelope =
            serde_json::from_value(raw.clone()).expect("artist overview fixture");

        let overview = map_artist_overview(response, raw).expect("map artist overview");

        assert_eq!(overview.artist.resource_ref.to_string(), "netease:6452");
        assert_eq!(overview.artist.name, "周杰伦");
        assert_eq!(overview.artist.track_count, Some(568));
        assert_eq!(
            overview.artist.extensions["overview_artist"]["publishTime"],
            1_784_000_000_000_u64
        );
        assert_eq!(
            overview.featured_tracks[0].resource_ref.to_string(),
            "netease:210049"
        );
        assert_eq!(overview.featured_tracks[0].artists.len(), 2);
        assert_eq!(
            overview.featured_tracks[0].extensions["overview_track"]["copyright"],
            2
        );
        assert!(overview.has_more_tracks);
        assert_eq!(overview.extensions["response"]["code"], 200);
    }

    #[test]
    fn maps_netease_artist_dynamic_stats_and_keeps_the_raw_response() {
        let raw = json!({
            "code": 200,
            "followed": false,
            "concert": {
                "onlineCount": 2,
                "simpleConcert": {"id": 42, "name": "线上演出"},
                "view": true
            },
            "videoNum": [
                {"cat": 0, "num": 9},
                {"cat": 1, "num": 8}
            ],
            "rcmdResource": {"resourceId": 123}
        });
        let follow_count_raw = json!({
            "code": 200,
            "data": {
                "fansCnt": 13704928,
                "follow": true,
                "followCnt": 0,
                "followDay": "",
                "followDayCnt": 0,
                "isFollow": true
            },
            "message": "success"
        });
        let response: ArtistDynamicEnvelope =
            serde_json::from_value(raw.clone()).expect("artist dynamic fixture");
        let follow_count: ArtistFollowCountEnvelope =
            serde_json::from_value(follow_count_raw.clone()).expect("artist follow count fixture");

        let stats = map_artist_stats(6452, response, raw, follow_count, follow_count_raw)
            .expect("map artist stats");

        assert_eq!(stats.artist_ref.to_string(), "netease:6452");
        assert_eq!(stats.followed, Some(true));
        assert_eq!(stats.follower_count, Some(13_704_928));
        assert_eq!(stats.video_counts.len(), 2);
        assert_eq!(stats.video_counts[0].category.as_deref(), Some("0"));
        assert_eq!(stats.video_counts[0].count, 9);
        assert_eq!(stats.online_concert_count, Some(2));
        assert_eq!(
            stats.extensions["response"]["concert"]["simpleConcert"]["id"],
            42
        );
        assert_eq!(
            stats.extensions["response"]["rcmdResource"]["resourceId"],
            123
        );
        assert_eq!(
            stats.extensions["follow_count_response"]["data"]["followDayCnt"],
            0
        );
    }

    #[test]
    fn maps_netease_album_dynamic_stats_to_the_unified_model() {
        let stats: AlbumStatsEnvelope = serde_json::from_value(json!({
            "commentCount": 1989,
            "isSub": true,
            "likedCount": 7,
            "onSale": false,
            "shareCount": 9306,
            "subCount": 71671,
            "subTime": 1704067200000_u64,
            "albumGameInfo": {"gameId": 42}
        }))
        .expect("album stats fixture");
        let stats = map_album_stats(32311, stats).expect("map album stats");

        assert_eq!(stats.album_ref.to_string(), "netease:32311");
        assert_eq!(stats.subscribed, Some(true));
        assert_eq!(stats.subscriber_count, Some(71671));
        assert_eq!(stats.comment_count, Some(1989));
        assert_eq!(stats.share_count, Some(9306));
        assert_eq!(stats.like_count, Some(7));
        assert_eq!(stats.on_sale, Some(false));
        assert_eq!(stats.subscribed_at.as_deref(), Some("2024-01-01T00:00:00Z"));
        assert_eq!(stats.extensions["album_game_info"]["gameId"], 42);
    }

    #[test]
    fn maps_netease_album_track_entitlements_and_quality_tiers() {
        let entitlement = map_track_entitlement(json!({
            "id": 2058263030,
            "st": 0,
            "fee": 8,
            "pl": 320000,
            "dl": 0,
            "maxbr": 999000,
            "playMaxbr": 0,
            "downloadMaxbr": 999000,
            "plLevel": "exhigh",
            "dlLevel": "none",
            "payed": 0,
            "chargeInfoList": [
                {"chargeType": 0, "rate": 128000},
                {"chargeType": 0, "rate": 192000},
                {"chargeType": 0, "rate": 320000},
                {"chargeType": 1, "rate": 999000},
                {"chargeType": 1, "rate": 1999000}
            ],
            "freeTrialPrivilege": {"resConsumable": false, "userConsumable": false}
        }))
        .expect("map track entitlement");

        assert_eq!(entitlement.track_ref.to_string(), "netease:2058263030");
        assert_eq!(entitlement.playable, Some(true));
        assert_eq!(entitlement.downloadable, Some(false));
        assert_eq!(entitlement.play_quality, Some(Quality::High));
        assert_eq!(entitlement.download_quality, None);
        assert_eq!(entitlement.max_play_bitrate, Some(999_000));
        assert_eq!(
            entitlement.available_qualities,
            vec![
                Quality::Standard,
                Quality::Higher,
                Quality::High,
                Quality::Lossless,
                Quality::Hires
            ]
        );
        assert_eq!(entitlement.fee, Some(8));
        assert_eq!(entitlement.paid, Some(false));
        assert_eq!(
            entitlement.extensions["privilege"]["chargeInfoList"][4]["rate"],
            1999000
        );

        let canonical = map_track_entitlement(json!({
            "id": 2058263030,
            "chargeInfoList": [
                {"rate": 1999000},
                {"rate": 320000},
                {"rate": 128000},
                {"rate": 999000},
                {"rate": 192000},
                {"rate": 320000}
            ]
        }))
        .expect("map reversed and duplicated quality tiers");
        assert_eq!(
            canonical.available_qualities,
            vec![
                Quality::Standard,
                Quality::Higher,
                Quality::High,
                Quality::Lossless,
                Quality::Hires
            ]
        );
    }

    #[test]
    fn builds_and_maps_netease_album_subscription_actions() {
        let (path, payload) = netease_album_subscription_request(32311, true);
        assert_eq!(path, "/api/album/sub");
        assert_eq!(payload["id"], 32311);
        let (path, payload) = netease_album_subscription_request(32311, false);
        assert_eq!(path, "/api/album/unsub");
        assert_eq!(payload["id"], 32311);

        let result = map_album_subscription_result(32311, true, json!({ "code": 200 }))
            .expect("map subscription result");
        assert_eq!(result.resource_ref.to_string(), "netease:32311");
        assert!(result.subscribed);
        assert_eq!(result.extensions["response"]["code"], 200);
    }

    #[test]
    fn builds_and_maps_netease_artist_subscription_actions() {
        let (path, payload) = netease_artist_subscription_request(6452, true);
        assert_eq!(path, "/api/artist/sub");
        assert_eq!(payload["artistId"], 6452);
        assert_eq!(payload["artistIds"], "[6452]");
        let (path, payload) = netease_artist_subscription_request(6452, false);
        assert_eq!(path, "/api/artist/unsub");
        assert_eq!(payload["artistId"], 6452);
        assert_eq!(payload["artistIds"], "[6452]");

        let result = map_artist_subscription_result(6452, true, json!({ "code": 200 }))
            .expect("map artist subscription result");
        assert_eq!(result.resource_ref.to_string(), "netease:6452");
        assert!(result.subscribed);
        assert_eq!(result.extensions["response"]["code"], 200);
    }

    #[test]
    fn playlist_create_update_and_delete_requests_cover_every_reference_branch() {
        let mut create = PlaylistCreateRequest::new("  测试歌单  ");
        create.visibility = PlaylistVisibility::Private;
        create.kind = PlaylistKind::Video;
        let payload = netease_playlist_create_payload(&create).expect("create playlist payload");
        assert_eq!(payload["name"], "测试歌单");
        assert_eq!(payload["privacy"], "10");
        assert_eq!(payload["type"], "VIDEO");

        let batch = PlaylistUpdateRequest {
            name: Some("新歌单".to_owned()),
            description: Some("描述".to_owned()),
            tags: Some(vec!["欧美".to_owned(), "现场".to_owned()]),
            variant: PlaylistMetadataUpdateVariant::Default,
            account: None,
        };
        let calls = netease_playlist_update_calls(24_381_616, &batch)
            .expect("batch playlist update payload");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, "/api/batch");
        let name: Value = serde_json::from_str(
            calls[0].1["/api/playlist/update/name"]
                .as_str()
                .expect("batch name payload"),
        )
        .expect("parse batch name payload");
        let tags: Value = serde_json::from_str(
            calls[0].1["/api/playlist/tags/update"]
                .as_str()
                .expect("batch tags payload"),
        )
        .expect("parse batch tags payload");
        assert_eq!(name, json!({ "id": 24_381_616, "name": "新歌单" }));
        assert_eq!(tags["tags"], "欧美;现场");

        let individual = PlaylistUpdateRequest {
            name: None,
            description: Some(String::new()),
            tags: Some(Vec::new()),
            variant: PlaylistMetadataUpdateVariant::Individual,
            account: None,
        };
        let calls = netease_playlist_update_calls(24_381_616, &individual)
            .expect("individual playlist update payloads");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].0, "/api/playlist/desc/update");
        assert_eq!(calls[0].1["desc"], "");
        assert_eq!(calls[1].0, "/api/playlist/tags/update");
        assert_eq!(calls[1].1["tags"], "");

        let missing_batch_fields = PlaylistUpdateRequest {
            name: Some("新歌单".to_owned()),
            description: None,
            tags: None,
            variant: PlaylistMetadataUpdateVariant::Batch,
            account: None,
        };
        let error = netease_playlist_update_calls(24_381_616, &missing_batch_fields)
            .expect_err("incomplete batch update");
        assert_eq!(error.code, ErrorCode::InvalidRequest);

        let references = vec![
            ResourceRef::new(Platform::Netease, "2947311456").expect("playlist reference"),
            ResourceRef::new(Platform::Netease, "2947311456")
                .expect("duplicate playlist reference"),
        ];
        let ids =
            netease_playlist_ids("playlist deletion", &references).expect("playlist deletion ids");
        assert_eq!(ids, vec![2_947_311_456, 2_947_311_456]);
        assert_eq!(join_numeric_ids(&ids), "2947311456,2947311456");
    }

    #[test]
    fn playlist_item_and_order_requests_keep_tracks_videos_and_ordering_distinct() {
        let track_refs = vec![
            ResourceRef::new(Platform::Netease, "5268328").expect("track reference"),
            ResourceRef::new(Platform::Netease, "1219871").expect("track reference"),
        ];
        let track_request =
            PlaylistItemMutationRequest::new(track_refs.clone(), PlaylistItemKind::Track);
        let track_ids = netease_playlist_item_ids(&track_request).expect("playlist track item ids");
        let add = netease_playlist_track_mutation_payload(
            2_039_116_066,
            PlaylistItemMutationAction::Add,
            &track_ids,
        );
        assert_eq!(add["op"], "add");
        assert_eq!(add["pid"], 2_039_116_066_u64);
        assert_eq!(add["trackIds"], r#"["5268328","1219871"]"#);
        assert_eq!(add["imme"], "true");

        let video_request = PlaylistItemMutationRequest::new(
            vec![
                ResourceRef::new(Platform::Netease, "89ADDE33C0AAE8EC14B99F6750DB954D")
                    .expect("video reference"),
            ],
            PlaylistItemKind::Video,
        );
        let video_ids = netease_playlist_item_ids(&video_request).expect("playlist video item ids");
        let (path, payload) = netease_playlist_video_mutation_request(
            5_271_999_357,
            PlaylistItemMutationAction::Remove,
            &video_ids,
        );
        assert_eq!(path, "/api/playlist/track/delete");
        let tracks: Value =
            serde_json::from_str(payload["tracks"].as_str().expect("video tracks payload"))
                .expect("parse video tracks payload");
        assert_eq!(tracks[0]["type"], 3);
        assert_eq!(tracks[0]["id"], "89ADDE33C0AAE8EC14B99F6750DB954D");

        let numeric_track_ids = netease_playlist_track_ids("playlist track order", &track_refs)
            .expect("playlist track order ids");
        let (path, payload) =
            netease_playlist_track_order_request(2_039_116_066, &numeric_track_ids);
        assert_eq!(path, "/api/playlist/manipulate/tracks");
        assert_eq!(payload["op"], "update");
        assert_eq!(payload["trackIds"], "[5268328,1219871]");

        let (path, payload) = netease_account_playlist_order_request(&[111, 222, 111]);
        assert_eq!(path, "/api/playlist/order/update");
        assert_eq!(payload["ids"], "[111,222,111]");
    }

    #[test]
    fn playlist_item_mapping_records_code_512_retry_without_rewriting_requested_items() {
        let request = PlaylistItemMutationRequest::new(
            vec![ResourceRef::new(Platform::Netease, "5268328").expect("track reference")],
            PlaylistItemKind::Track,
        );
        let result = map_netease_playlist_item_mutation_result(
            2_039_116_066,
            PlaylistItemMutationAction::Add,
            &request,
            json!({
                "code": 200,
                "snapshotId": " ",
                "snapshot_id": "snapshot-2",
                "cloudCount": 4
            }),
            Some(json!({ "code": 512, "message": "retry" })),
        )
        .expect("map retried playlist track update");
        assert_eq!(result.playlist_ref.to_string(), "netease:2039116066");
        assert_eq!(result.item_refs, request.item_refs);
        assert_eq!(result.snapshot_id.as_deref(), Some("snapshot-2"));
        assert_eq!(result.cloud_track_count, Some(4));
        assert_eq!(result.extensions["retried_code_512"], true);
        assert_eq!(result.extensions["initial_response"]["code"], 512);

        let ordered = map_netease_playlist_track_order_result(
            2_039_116_066,
            vec![ResourceRef::new(Platform::Netease, "5268328").expect("track reference")],
            json!({
                "code": 200,
                "snapshotId": "",
                "data": {"snapshotId": "snapshot-3"}
            }),
        )
        .expect("map playlist track order");
        assert_eq!(ordered.snapshot_id.as_deref(), Some("snapshot-3"));
        assert_eq!(ordered.extensions["response"]["code"], 200);
    }

    #[test]
    fn playlist_mutation_mapping_preserves_created_playlist_and_raw_responses() {
        let response = json!({
            "code": 200,
            "playlist": {
                "id": 2947311456_u64,
                "name": "测试歌单",
                "description": null,
                "coverImgUrl": "https://example.test/playlist.jpg",
                "trackCount": 0,
                "tags": [],
                "trackIds": []
            }
        });
        let result =
            map_netease_playlist_create_result(response).expect("map playlist creation response");
        assert_eq!(result.playlist_ref.to_string(), "netease:2947311456");
        assert_eq!(result.action, PlaylistMutationAction::Create);
        assert_eq!(result.playlist.expect("created playlist").name, "测试歌单");
        assert_eq!(result.extensions["response"]["code"], 200);

        let fallback_id = map_netease_playlist_create_result(json!({
            "code": 200,
            "playlist": {"id": 0},
            "data": {"id": 2947311456_u64}
        }))
        .expect("skip zero playlist id alias");
        assert_eq!(fallback_id.playlist_ref.to_string(), "netease:2947311456");

        let missing_id = map_netease_playlist_create_result(json!({ "code": 200 }))
            .expect_err("creation response without id");
        assert_eq!(missing_id.code, ErrorCode::UpstreamError);
    }

    #[tokio::test]
    async fn playlist_writes_require_authentication_before_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let create =
            MusicProvider::create_playlist(&provider, &PlaylistCreateRequest::new("测试歌单"))
                .await
                .expect_err("anonymous playlist creation");
        assert_eq!(create.code, ErrorCode::AuthenticationRequired);

        let update = MusicProvider::update_playlist(
            &provider,
            "2947311456",
            &PlaylistUpdateRequest {
                name: Some("新歌单".to_owned()),
                description: None,
                tags: None,
                variant: PlaylistMetadataUpdateVariant::Individual,
                account: None,
            },
        )
        .await
        .expect_err("anonymous playlist update");
        assert_eq!(update.code, ErrorCode::AuthenticationRequired);

        let delete = MusicProvider::delete_playlists(
            &provider,
            &PlaylistDeleteRequest {
                playlist_refs: vec![
                    ResourceRef::new(Platform::Netease, "2947311456").expect("playlist reference"),
                ],
                account: None,
            },
        )
        .await
        .expect_err("anonymous playlist deletion");
        assert_eq!(delete.code, ErrorCode::AuthenticationRequired);

        let item = MusicProvider::mutate_playlist_items(
            &provider,
            "2039116066",
            PlaylistItemMutationAction::Add,
            &PlaylistItemMutationRequest::new(
                vec![ResourceRef::new(Platform::Netease, "5268328").expect("track reference")],
                PlaylistItemKind::Track,
            ),
        )
        .await
        .expect_err("anonymous playlist item update");
        assert_eq!(item.code, ErrorCode::AuthenticationRequired);

        let track_order = MusicProvider::reorder_playlist_tracks(
            &provider,
            "2039116066",
            &PlaylistTrackOrderRequest {
                track_refs: vec![
                    ResourceRef::new(Platform::Netease, "5268328").expect("track reference"),
                ],
                account: None,
            },
        )
        .await
        .expect_err("anonymous playlist track order");
        assert_eq!(track_order.code, ErrorCode::AuthenticationRequired);

        let playlist_order = MusicProvider::reorder_account_playlists(
            &provider,
            &PlaylistOrderRequest {
                playlist_refs: vec![
                    ResourceRef::new(Platform::Netease, "2039116066").expect("playlist reference"),
                ],
                account: None,
            },
        )
        .await
        .expect_err("anonymous playlist order");
        assert_eq!(playlist_order.code, ErrorCode::AuthenticationRequired);

        let cover = MusicProvider::update_playlist_cover(
            &provider,
            "2039116066",
            &ImageUploadRequest {
                filename: "cover.jpg".to_owned(),
                content_type: "image/jpeg".to_owned(),
                data: vec![0xff, 0xd8, 0xff, 0xd9],
                image_size: Some(300),
                crop_x: Some(0),
                crop_y: Some(0),
                account: None,
            },
        )
        .await
        .expect_err("anonymous playlist cover update");
        assert_eq!(cover.code, ErrorCode::AuthenticationRequired);
    }

    #[test]
    fn maps_netease_digital_album_product_to_the_unified_model() {
        let raw = json!({
            "code": 200,
            "album": {
                "albumId": 120605500,
                "albumName": "冀西南林路行",
                "artistId": 13223,
                "artistName": "摘要艺人",
                "artistNames": "万能青年旅店/合作艺人",
                "coverUrl": "https://example.test/album.jpg"
            },
            "product": {
                "price": 22.0,
                "isFree": false,
                "pubTime": 1608566401510_u64,
                "saleNum": 42,
                "tags": ["独家", "无损品质收听＆下载"],
                "descr": [
                    {"resource": "发端似乎在2013年", "type": 1},
                    {"resource": "</br>", "type": 1},
                    {"resource": "西郊有密林 助君出重围", "type": 1}
                ],
                "albumType": 0,
                "albumfee": 4
            },
            "canBuy": true,
            "hasAlbum": false,
            "boughtCnt": 0,
            "board": {"hasFansBoard": true},
            "style": {"color": "#605848"},
            "singleSongProductId": 5933052,
            "visitorId": 0
        });
        let response: DigitalAlbumEnvelope =
            serde_json::from_value(raw.clone()).expect("digital album fixture");
        let album = map_digital_album(response, &raw, 120605500).expect("map digital album");

        assert_eq!(album.resource_ref.to_string(), "netease:120605500");
        assert_eq!(album.artists[0].name, "万能青年旅店/合作艺人");
        assert_eq!(album.price.expect("price").amount, 22.0);
        assert_eq!(album.purchasable, Some(true));
        assert_eq!(album.purchased, Some(false));
        assert_eq!(album.sale_count, Some(42));
        assert_eq!(album.tags.len(), 2);
        assert!(album.description.contains("西郊有密林"));
        assert!(!album.description.contains("</br>"));
        assert_eq!(album.extensions["product"]["albumfee"], 4);
        assert_eq!(album.extensions["board"]["hasFansBoard"], true);
    }

    #[test]
    fn maps_netease_digital_album_list_items_without_losing_product_fields() {
        let album = map_digital_album_list_item(json!({
            "albumId": 387169747,
            "albumName": "小海子村儿",
            "albumType": 1,
            "area": 7,
            "artistName": "窦唯/朝简",
            "artistType": 0,
            "coverUrl": "https://example.test/album.jpg",
            "newAlbum": true,
            "price": 100.0,
            "productId": 0,
            "pubTime": 1784163600496_u64,
            "saleNum": 24,
            "saleType": 0,
            "status": 0
        }))
        .expect("map digital album list item");

        assert_eq!(album.resource_ref.to_string(), "netease:387169747");
        assert_eq!(album.artists[0].name, "窦唯/朝简");
        assert_eq!(album.price.expect("price").amount, 100.0);
        assert_eq!(album.sale_count, Some(24));
        assert_eq!(album.extensions["product"]["newAlbum"], true);
        assert_eq!(album.extensions["product"]["area"], 7);
    }

    #[test]
    fn maps_netease_digital_album_chart_rank_and_product_fields() {
        let entry = map_digital_album_chart_entry(
            json!({
                "albumId": 156507145,
                "albumName": "希忘Hope",
                "albumType": 0,
                "artistName": "华晨宇",
                "coverUrl": "https://example.test/chart.jpg",
                "price": 27.0,
                "rank": 0,
                "rankIncr": 5,
                "saleNum": 324,
                "salesCertificationSystemLevelCode": "collectionDiamond"
            }),
            9,
        )
        .expect("map digital album chart entry");

        assert_eq!(entry.rank, 1);
        assert_eq!(entry.rank_change, Some(5));
        assert_eq!(entry.product.resource_ref.to_string(), "netease:156507145");
        assert_eq!(entry.product.sale_count, Some(324));
        assert_eq!(entry.product.price.expect("price").amount, 27.0);
        assert_eq!(entry.extensions["upstream_rank"], 0);
        assert_eq!(entry.extensions["album_type"], 0);
        assert_eq!(
            entry.product.extensions["product"]["salesCertificationSystemLevelCode"],
            "collectionDiamond"
        );
    }

    #[test]
    fn builds_netease_digital_album_chart_period_and_kind_requests() {
        let daily = DigitalAlbumChartRequest::new(20, 0);
        let (path, payload) =
            netease_digital_album_chart_request(&daily).expect("daily album chart request");
        assert_eq!(path, "/api/feealbum/songsaleboard/daily/type");
        assert_eq!(payload["albumType"], 0);
        assert!(payload.get("year").is_none());

        let mut yearly_single = DigitalAlbumChartRequest::new(10, 0);
        yearly_single.period = DigitalAlbumChartPeriod::Year;
        yearly_single.kind = DigitalAlbumChartKind::Single;
        yearly_single.year = Some(2025);
        let (path, payload) = netease_digital_album_chart_request(&yearly_single)
            .expect("yearly single chart request");
        assert_eq!(path, "/api/feealbum/songsaleboard/year/type");
        assert_eq!(payload["albumType"], 1);
        assert_eq!(payload["year"], 2025);

        let mut invalid = DigitalAlbumChartRequest::new(20, 0);
        invalid.year = Some(2025);
        let error =
            netease_digital_album_chart_request(&invalid).expect_err("year outside yearly chart");
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn maps_classic_chart_catalog_and_text_previews_without_losing_raw_fields() {
        let raw = json!({
            "code": 200,
            "artistToplist": {"id": 10520166, "name": "歌手榜"},
            "rewardToplist": {"coverUrl": "https://example.test/reward.jpg"},
            "list": [{
                "id": 19723756,
                "name": "飙升榜",
                "coverImgUrl": "https://example.test/soaring.jpg",
                "description": "每天热度上升最快的歌曲",
                "updateFrequency": "每天更新",
                "updateTime": 1_784_170_805_374_u64,
                "trackCount": 100,
                "playCount": 42_000,
                "subscribed": false,
                "ToplistType": "S",
                "tracks": [
                    {"first": "周旋", "second": "王以太/艾热 AIR"},
                    {"first": "盛夏的果实", "second": "莫文蔚"}
                ]
            }]
        });
        let response: ChartCatalogEnvelope =
            parse_body(raw.clone()).expect("classic chart catalog fixture");
        let catalog = map_chart_catalog(response, ChartCatalogView::Summary, raw)
            .expect("map classic chart catalog");

        assert_eq!(catalog.view, ChartCatalogView::Summary);
        assert_eq!(catalog.groups.len(), 1);
        assert_eq!(catalog.groups[0].name, "all");
        let chart = &catalog.groups[0].charts[0];
        assert_eq!(
            chart
                .resource_ref
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("netease:19723756")
        );
        assert_eq!(
            chart.cover_url.as_deref(),
            Some("https://example.test/soaring.jpg")
        );
        assert_eq!(chart.track_count, Some(100));
        assert_eq!(chart.previews[0].rank, Some(1));
        assert_eq!(chart.previews[0].track_ref, None);
        assert_eq!(chart.previews[0].name, "周旋");
        assert_eq!(chart.previews[0].byline.as_deref(), Some("王以太/艾热 AIR"));
        assert_eq!(chart.extensions["chart"]["ToplistType"], "S");
        assert_eq!(
            catalog.extensions["response"]["artistToplist"]["id"],
            10520166
        );
        assert_eq!(
            catalog.extensions["response"]["rewardToplist"]["coverUrl"],
            "https://example.test/reward.jpg"
        );
    }

    #[test]
    fn maps_modern_grouped_charts_ranked_tracks_and_non_playlist_targets() {
        let raw = json!({
            "code": 200,
            "data": [{
                "categoryCode": "OFFICIAL",
                "displayType": "  ",
                "frontDisplayType": "CAROUSEL",
                "name": "官方榜",
                "targetUrl": "orpheus://toplist",
                "list": [
                    {
                        "id": 19723756,
                        "name": "飙升榜",
                        "coverUrl": "https://example.test/soaring.jpg",
                        "canPlay": true,
                        "targetType": "PLAYLIST",
                        "updateFrequency": "每天更新",
                        "trackRankList": [
                            {
                                "trackId": 3404238777_u64,
                                "songName": " ",
                                "itemName": "周旋",
                                "artistName": "王以太/艾热 AIR",
                                "coverImgUrl": "https://example.test/song.jpg",
                                "rank": 1,
                                "lastRank": 5,
                                "futureField": "kept"
                            },
                            {
                                "trackId": 277382,
                                "songName": "盛夏的果实",
                                "artistName": "莫文蔚",
                                "rank": 2,
                                "lastRank": 0
                            }
                        ]
                    },
                    {
                        "id": 0,
                        "name": "实体专辑榜",
                        "firstCoverUrl": "https://example.test/old-album-chart.jpg",
                        "newFirstCoverUrl": "https://example.test/new-album-chart.jpg",
                        "canPlay": false,
                        "targetType": "H5",
                        "targetUrl": " ",
                        "frontTargetUrl": "https://example.test/store",
                        "toplistCode": "ALBUM_SELL_CHART##",
                        "trackRankList": null,
                        "tracks": null
                    }
                ]
            }]
        });
        let response: ChartCatalogEnvelope =
            parse_body(raw.clone()).expect("modern chart catalog fixture");
        let catalog = map_chart_catalog(response, ChartCatalogView::Modern, raw)
            .expect("map modern chart catalog");

        let group = &catalog.groups[0];
        assert_eq!(group.code.as_deref(), Some("OFFICIAL"));
        assert_eq!(group.name, "官方榜");
        assert_eq!(group.display_type.as_deref(), Some("CAROUSEL"));
        assert_eq!(group.extensions["group"]["frontDisplayType"], "CAROUSEL");
        let chart = &group.charts[0];
        assert_eq!(
            chart
                .resource_ref
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("netease:19723756")
        );
        assert_eq!(chart.target_kind.as_deref(), Some("playlist"));
        assert_eq!(chart.previews[0].rank, Some(1));
        assert_eq!(chart.previews[0].previous_rank, Some(5));
        assert_eq!(chart.previews[0].rank_change, Some(4));
        assert_eq!(
            chart.previews[0]
                .track_ref
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("netease:3404238777")
        );
        assert_eq!(
            chart.previews[0].extensions["preview"]["futureField"],
            "kept"
        );
        assert_eq!(chart.previews[1].previous_rank, None);
        assert_eq!(chart.previews[1].rank_change, None);
        let external = &group.charts[1];
        assert_eq!(external.id.as_deref(), Some("0"));
        assert_eq!(external.resource_ref, None);
        assert_eq!(external.playable, Some(false));
        assert_eq!(external.target_kind.as_deref(), Some("h5"));
        assert_eq!(
            external.cover_url.as_deref(),
            Some("https://example.test/new-album-chart.jpg")
        );
        assert_eq!(
            external.target_url.as_deref(),
            Some("https://example.test/store")
        );
    }

    #[test]
    fn maps_all_artist_chart_areas_rank_changes_and_artist_metadata() {
        assert_eq!(netease_artist_chart_area(ArtistChartArea::Chinese), 1);
        assert_eq!(netease_artist_chart_area(ArtistChartArea::Western), 2);
        assert_eq!(netease_artist_chart_area(ArtistChartArea::Korean), 3);
        assert_eq!(netease_artist_chart_area(ArtistChartArea::Japanese), 4);

        let raw = json!({
            "code": 200,
            "list": {
                "type": 2,
                "updateTime": 1_784_170_805_374_u64,
                "artists": [{
                    "id": 3684,
                    "name": "林俊杰",
                    "alias": ["JJ Lin", "Wayne Lim"],
                    "trans": "",
                    "briefDesc": "歌手简介",
                    "img1v1Url": "https://example.test/avatar.jpg",
                    "picUrl": "https://example.test/cover.jpg",
                    "albumSize": 73,
                    "musicSize": 598,
                    "lastRank": 5,
                    "score": 63_562_038,
                    "topicPerson": 40_831
                }]
            }
        });
        let response: ArtistChartEnvelope = parse_body(raw.clone()).expect("artist chart fixture");
        let chart =
            map_artist_chart(response, ArtistChartArea::Western, raw).expect("map artist chart");

        assert_eq!(chart.area, ArtistChartArea::Western);
        assert_eq!(chart.updated_at_ms, Some(1_784_170_805_374));
        assert_eq!(chart.entries[0].rank, 1);
        assert_eq!(chart.entries[0].previous_rank, Some(5));
        assert_eq!(chart.entries[0].rank_change, Some(4));
        assert_eq!(chart.entries[0].score, Some(63_562_038));
        assert_eq!(
            chart.entries[0].artist.resource_ref.to_string(),
            "netease:3684"
        );
        assert_eq!(chart.entries[0].artist.aliases, ["JJ Lin", "Wayne Lim"]);
        assert_eq!(chart.entries[0].artist.album_count, Some(73));
        assert_eq!(
            chart.entries[0].artist.extensions["artist_chart_item"]["topicPerson"],
            40_831
        );
        assert_eq!(chart.extensions["response"]["list"]["type"], 2);
    }

    #[test]
    fn builds_netease_dimension_chart_payload_without_inventing_pagination() {
        let request = DimensionChartRequest::new("CITY_SONG_CHART", "110000", "CITY");
        let payload = netease_dimension_chart_payload(&request).expect("dimension chart payload");
        assert_eq!(payload["chartCode"], "CITY_SONG_CHART");
        assert_eq!(payload["targetId"], "110000");
        assert_eq!(payload["targetType"], "CITY");
        assert!(payload.get("limit").is_none());
        assert!(payload.get("offset").is_none());

        for request in [
            DimensionChartRequest::new("", "110000", "CITY"),
            DimensionChartRequest::new("CITY_SONG_CHART", " ", "CITY"),
            DimensionChartRequest::new("CITY_SONG_CHART", "110000", ""),
        ] {
            assert_eq!(
                netease_dimension_chart_payload(&request)
                    .expect_err("empty dimension parameter")
                    .code,
                ErrorCode::InvalidRequest
            );
        }
    }

    #[test]
    fn maps_netease_dimension_chart_detail_and_complete_track_snapshot() {
        let detail_raw = json!({
            "code": 200,
            "data": {
                "chartCode": "CITY_SONG_CHART",
                "chartId": "CITY_SONG_CHART#110000@CITY#",
                "commentCount": 9,
                "coverUrl": "https://example.test/city.png",
                "description": "当前城市用户一周内收听的歌曲。",
                "name": "北京榜",
                "playCount": 120,
                "shareCount": 3,
                "supportComment": true,
                "updateTime": 1784181600000_u64,
                "commonChartExtInfoVO": {"color": "red"}
            }
        });
        let detail: DimensionChartDetailEnvelope =
            parse_body(detail_raw.clone()).expect("dimension chart detail fixture");
        let request = DimensionChartRequest::new("CITY_SONG_CHART", "110000", "CITY");
        let detail =
            map_dimension_chart(detail, &request, detail_raw).expect("map dimension chart detail");
        assert_eq!(
            detail.resource_ref.to_string(),
            "netease:CITY_SONG_CHART#110000@CITY#"
        );
        assert_eq!(detail.name, "北京榜");
        assert_eq!(detail.updated_at_ms, Some(1_784_181_600_000));
        assert_eq!(detail.supports_comments, Some(true));
        assert_eq!(
            detail.extensions["response"]["data"]["commonChartExtInfoVO"]["color"],
            "red"
        );

        let tracks_raw = json!({
            "code": 200,
            "data": {
                "chartCode": "CITY_STYLE_SONG_CHART",
                "chartId": "CITY_STYLE_SONG_CHART#110000_1020@CITY_STYLE#",
                "charts": [{
                    "collect": false,
                    "lastRank": 4,
                    "ratio": "0.98",
                    "reason": "城市流行热度上升",
                    "reasonId": 17,
                    "score": 98.5,
                    "songData": {
                        "id": 123,
                        "name": "反方向的钟",
                        "alia": ["Clockwise"],
                        "ar": [{"id": 6452, "name": "周杰伦"}],
                        "al": {"id": 456, "name": "Jay", "picUrl": "https://example.test/cover.jpg"},
                        "dt": 258000,
                        "mv": 0,
                        "fee": 1,
                        "st": 0,
                        "l": {"br": 128000},
                        "h": {"br": 320000}
                    },
                    "privilege": {"id": 123, "st": 0, "fee": 1, "pl": 320000, "maxbr": 999000},
                    "targetUrl": "https://example.test/reason"
                }],
                "groupNameMap": {"CITY": "城市", "1020": "流行"},
                "periodUpdateTimeText": "每周更新",
                "uuid": "snapshot-1"
            }
        });
        let tracks: DimensionChartTracksEnvelope =
            parse_body(tracks_raw.clone()).expect("dimension chart tracks fixture");
        let request =
            DimensionChartRequest::new("CITY_STYLE_SONG_CHART", "110000_1020", "CITY_STYLE");
        let snapshot = map_dimension_chart_tracks(tracks, &request, tracks_raw)
            .expect("map dimension chart tracks");
        assert_eq!(
            snapshot.chart_ref.to_string(),
            "netease:CITY_STYLE_SONG_CHART#110000_1020@CITY_STYLE#"
        );
        assert_eq!(snapshot.entries.len(), 1);
        assert_eq!(snapshot.entries[0].rank, 1);
        assert_eq!(snapshot.entries[0].previous_rank, Some(4));
        assert_eq!(snapshot.entries[0].rank_change, Some(3));
        assert_eq!(snapshot.entries[0].track.name, "反方向的钟");
        assert_eq!(snapshot.entries[0].track.playable, Some(true));
        assert_eq!(snapshot.entries[0].reason_id.as_deref(), Some("17"));
        assert_eq!(snapshot.entries[0].score, Some(98.5));
        assert_eq!(snapshot.entries[0].ratio, Some(0.98));
        assert_eq!(snapshot.groups["1020"], "流行");
        assert_eq!(snapshot.period_label.as_deref(), Some("每周更新"));
        assert_eq!(
            snapshot.entries[0].extensions["entry"]["targetUrl"],
            "https://example.test/reason"
        );
        assert_eq!(
            snapshot.extensions["response"]["data"]["uuid"],
            "snapshot-1"
        );
    }

    #[test]
    fn validates_netease_digital_album_areas() {
        assert_eq!(
            normalize_digital_album_area(DigitalAlbumCatalog::Latest, Some("zh"))
                .expect("valid latest area"),
            "ZH"
        );
        assert_eq!(
            normalize_digital_album_area(DigitalAlbumCatalog::Latest, None)
                .expect("default latest area"),
            "ALL"
        );
        assert_eq!(
            normalize_digital_album_area(DigitalAlbumCatalog::Style, Some("zh"))
                .expect("valid style area"),
            "Z_H"
        );
        assert_eq!(
            DigitalAlbumCatalog::parse(Some("style")).expect("style catalog"),
            DigitalAlbumCatalog::Style
        );
        let error = normalize_digital_album_area(DigitalAlbumCatalog::Style, Some("unknown"))
            .expect_err("invalid area");
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn validates_netease_album_catalogs_and_areas() {
        assert_eq!(
            AlbumCatalog::parse(Some("newest")).expect("newest catalog"),
            AlbumCatalog::Newest
        );
        assert_eq!(
            normalize_album_area(Some("e_a")).expect("valid album area"),
            "EA"
        );
        let error = normalize_album_area(Some("unknown")).expect_err("invalid album area");
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn broadcast_station_catalog_validates_filters_and_cursor_before_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for request in [
            RadioStationListRequest {
                category_id: Some("invalid".to_owned()),
                ..RadioStationListRequest::new(20)
            },
            RadioStationListRequest {
                region_id: Some("invalid".to_owned()),
                ..RadioStationListRequest::new(20)
            },
            RadioStationListRequest {
                cursor: Some(RadioStationCursor {
                    id: "invalid".to_owned(),
                    score: 1,
                }),
                ..RadioStationListRequest::new(20)
            },
        ] {
            let error = MusicProvider::radio_stations(&provider, &request)
                .await
                .expect_err("invalid broadcast station catalog parameter");
            assert_eq!(error.code, ErrorCode::InvalidRequest);
        }
    }

    #[tokio::test]
    async fn album_ids_are_validated_before_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let radio_error = MusicProvider::radio_station(&provider, "invalid", None)
            .await
            .expect_err("invalid broadcast station id");
        assert_eq!(radio_error.code, ErrorCode::InvalidRequest);
        let radio_subscription_error =
            MusicProvider::set_radio_station_subscription(&provider, "invalid", true, None)
                .await
                .expect_err("invalid broadcast station subscription id");
        assert_eq!(radio_subscription_error.code, ErrorCode::InvalidRequest);
        let detail_error = MusicProvider::album(&provider, "invalid", None)
            .await
            .expect_err("invalid album id");
        assert_eq!(detail_error.code, ErrorCode::InvalidRequest);
        let availability_error = MusicProvider::track_availability(
            &provider,
            "invalid",
            &TrackAvailabilityRequest::default(),
        )
        .await
        .expect_err("invalid track availability id");
        assert_eq!(availability_error.code, ErrorCode::InvalidRequest);
        let tracks_error =
            MusicProvider::album_tracks(&provider, "invalid", &PageRequest::new(30, 0))
                .await
                .expect_err("invalid album tracks id");
        assert_eq!(tracks_error.code, ErrorCode::InvalidRequest);
        let digital_error = MusicProvider::digital_album(&provider, "invalid", None)
            .await
            .expect_err("invalid digital album id");
        assert_eq!(digital_error.code, ErrorCode::InvalidRequest);
        let stats_error = MusicProvider::album_stats(&provider, "invalid", None)
            .await
            .expect_err("invalid album stats id");
        assert_eq!(stats_error.code, ErrorCode::InvalidRequest);
        let entitlement_error =
            MusicProvider::album_track_entitlements(&provider, "invalid", &PageRequest::new(30, 0))
                .await
                .expect_err("invalid album entitlement id");
        assert_eq!(entitlement_error.code, ErrorCode::InvalidRequest);
        let artist_error =
            MusicProvider::artist_albums(&provider, "invalid", &PageRequest::new(30, 0))
                .await
                .expect_err("invalid artist id");
        assert_eq!(artist_error.code, ErrorCode::InvalidRequest);
        let artist_detail_error = MusicProvider::artist(&provider, "invalid", None)
            .await
            .expect_err("invalid artist detail id");
        assert_eq!(artist_detail_error.code, ErrorCode::InvalidRequest);
        let artist_overview_error = MusicProvider::artist_overview(&provider, "invalid", None)
            .await
            .expect_err("invalid artist overview id");
        assert_eq!(artist_overview_error.code, ErrorCode::InvalidRequest);
        let artist_stats_error = MusicProvider::artist_stats(&provider, "invalid", None)
            .await
            .expect_err("invalid artist stats id");
        assert_eq!(artist_stats_error.code, ErrorCode::InvalidRequest);
        let artist_fans_error =
            MusicProvider::artist_fans(&provider, "invalid", &PageRequest::new(20, 0))
                .await
                .expect_err("invalid artist fans id");
        assert_eq!(artist_fans_error.code, ErrorCode::InvalidRequest);
        let mut video_request = ArtistVideoListRequest::new(20, 0);
        video_request.kind = VideoKind::Mv;
        let artist_videos_error =
            MusicProvider::artist_videos(&provider, "invalid", &video_request)
                .await
                .expect_err("invalid artist videos id");
        assert_eq!(artist_videos_error.code, ErrorCode::InvalidRequest);
        let artist_tracks_error =
            MusicProvider::artist_tracks(&provider, "invalid", &ArtistTrackListRequest::new(20, 0))
                .await
                .expect_err("invalid artist tracks id");
        assert_eq!(artist_tracks_error.code, ErrorCode::InvalidRequest);
        let artist_subscription_error =
            MusicProvider::set_artist_subscription(&provider, "invalid", true, None)
                .await
                .expect_err("invalid artist subscription id");
        assert_eq!(artist_subscription_error.code, ErrorCode::InvalidRequest);
        let artist_top_tracks_error = MusicProvider::artist_top_tracks(&provider, "invalid", None)
            .await
            .expect_err("invalid artist top tracks id");
        assert_eq!(artist_top_tracks_error.code, ErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn artist_catalog_rejects_invalid_initial_before_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let mut request = ArtistListRequest::new(30, 0);
        request.initial = Some("中文".to_owned());
        let error = MusicProvider::artists(&provider, &request)
            .await
            .expect_err("invalid artist initial");
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn audio_recognition_validates_fingerprint_boundaries_before_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let empty = MusicProvider::recognize_audio(
            &provider,
            &AudioRecognitionRequest {
                fingerprint: "   ".to_owned(),
                duration_seconds: 6,
                account: None,
            },
        )
        .await
        .expect_err("empty fingerprint");
        assert_eq!(empty.code, ErrorCode::InvalidRequest);

        let duration = MusicProvider::recognize_audio(
            &provider,
            &AudioRecognitionRequest {
                fingerprint: "fingerprint".to_owned(),
                duration_seconds: 0,
                account: None,
            },
        )
        .await
        .expect_err("invalid fingerprint duration");
        assert_eq!(duration.code, ErrorCode::InvalidRequest);

        let oversized = MusicProvider::recognize_audio(
            &provider,
            &AudioRecognitionRequest {
                fingerprint: "x".repeat(131_073),
                duration_seconds: 6,
                account: None,
            },
        )
        .await
        .expect_err("oversized fingerprint");
        assert_eq!(oversized.code, ErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn avatar_upload_validates_input_and_authentication_before_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let invalid_requests = [
            ImageUploadRequest {
                filename: "   ".to_owned(),
                content_type: "image/jpeg".to_owned(),
                data: vec![1],
                image_size: None,
                crop_x: None,
                crop_y: None,
                account: None,
            },
            ImageUploadRequest {
                filename: "avatar.txt".to_owned(),
                content_type: "text/plain".to_owned(),
                data: vec![1],
                image_size: None,
                crop_x: None,
                crop_y: None,
                account: None,
            },
            ImageUploadRequest {
                filename: "avatar.jpg".to_owned(),
                content_type: "image/jpeg".to_owned(),
                data: Vec::new(),
                image_size: None,
                crop_x: None,
                crop_y: None,
                account: None,
            },
            ImageUploadRequest {
                filename: "avatar.jpg".to_owned(),
                content_type: "image/jpeg".to_owned(),
                data: vec![1],
                image_size: Some(0),
                crop_x: None,
                crop_y: None,
                account: None,
            },
        ];
        for request in invalid_requests {
            let error = MusicProvider::upload_account_avatar(&provider, &request)
                .await
                .expect_err("invalid image upload request");
            assert_eq!(error.code, ErrorCode::InvalidRequest);
        }

        let unauthenticated = MusicProvider::upload_account_avatar(
            &provider,
            &ImageUploadRequest {
                filename: "avatar.jpg".to_owned(),
                content_type: "image/jpeg".to_owned(),
                data: vec![0xff, 0xd8, 0xff, 0xd9],
                image_size: Some(300),
                crop_x: Some(0),
                crop_y: Some(0),
                account: None,
            },
        )
        .await
        .expect_err("anonymous avatar upload");
        assert_eq!(unauthenticated.code, ErrorCode::AuthenticationRequired);
    }

    #[test]
    fn cloud_upload_ticket_normalizes_reference_file_fields() {
        let mut request = CloudUploadTicketRequest::new(
            "0123456789ABCDEF0123456789ABCDEF",
            42,
            " Track.Name .FLAC ",
        );
        request.content_type = Some("audio/x-flac".to_owned());
        let descriptor =
            validate_cloud_upload_ticket_request(&request).expect("valid cloud upload ticket");
        assert_eq!(descriptor.md5, "0123456789abcdef0123456789abcdef");
        assert_eq!(descriptor.filename, "Track.Name .FLAC");
        assert_eq!(descriptor.allocation_filename, "Track_Name");
        assert_eq!(descriptor.extension, "flac");
        assert_eq!(descriptor.content_type, "audio/x-flac");
    }

    #[test]
    fn cloud_upload_ticket_rejects_invalid_file_fields() {
        let mut zero_size =
            CloudUploadTicketRequest::new("0123456789abcdef0123456789abcdef", 0, "song.mp3");
        let bad_md5 = CloudUploadTicketRequest::new("not-md5", 1, "song.mp3");
        let bad_filename =
            CloudUploadTicketRequest::new("0123456789abcdef0123456789abcdef", 1, "../song.mp3");
        zero_size.content_type = Some("audio/mpeg".to_owned());
        let mut bad_content_type =
            CloudUploadTicketRequest::new("0123456789abcdef0123456789abcdef", 1, "song.mp3");
        bad_content_type.content_type = Some("audio/mpeg\r\nx-secret: value".to_owned());
        let mut bad_bitrate =
            CloudUploadTicketRequest::new("0123456789abcdef0123456789abcdef", 1, "song.mp3");
        bad_bitrate.bitrate = 0;

        for request in [
            zero_size,
            bad_md5,
            bad_filename,
            bad_content_type,
            bad_bitrate,
        ] {
            let error = validate_cloud_upload_ticket_request(&request)
                .expect_err("invalid cloud upload ticket");
            assert_eq!(error.code, ErrorCode::InvalidRequest);
        }
    }

    #[test]
    fn cloud_upload_completion_uses_reference_metadata_defaults() {
        let mut request = CloudUploadCompleteRequest {
            provisional_track_id: " 123 ".to_owned(),
            resource_id: " resource ".to_owned(),
            md5: "0123456789ABCDEF0123456789ABCDEF".to_owned(),
            filename: "反方向的钟.flac".to_owned(),
            song_name: Some("   ".to_owned()),
            artist: None,
            album: None,
            bitrate: 999_000,
            account: None,
        };
        let descriptor =
            validate_cloud_upload_complete_request(&request).expect("valid completion request");
        assert_eq!(descriptor.provisional_track_id, "123");
        assert_eq!(descriptor.resource_id, "resource");
        assert_eq!(descriptor.song_name, "反方向的钟");
        assert_eq!(descriptor.artist, "未知艺术家");
        assert_eq!(descriptor.album, "未知专辑");

        request.bitrate = 0;
        let error =
            validate_cloud_upload_complete_request(&request).expect_err("zero cloud audio bitrate");
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn proxy_cloud_upload_reads_tagged_metadata_and_computes_md5() {
        fn riff_info_item(id: &[u8; 4], value: &str) -> Vec<u8> {
            let mut payload = value.as_bytes().to_vec();
            payload.push(0);
            let mut item = Vec::new();
            item.extend_from_slice(id);
            item.extend_from_slice(&(payload.len() as u32).to_le_bytes());
            item.extend_from_slice(&payload);
            if payload.len() % 2 != 0 {
                item.push(0);
            }
            item
        }

        let mut info = b"INFO".to_vec();
        info.extend(riff_info_item(b"INAM", "Tagged Song"));
        info.extend(riff_info_item(b"IART", "Tagged Artist"));
        info.extend(riff_info_item(b"IPRD", "Tagged Album"));

        let mut audio = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        audio.extend_from_slice(&16_u32.to_le_bytes());
        audio.extend_from_slice(&1_u16.to_le_bytes());
        audio.extend_from_slice(&1_u16.to_le_bytes());
        audio.extend_from_slice(&8_000_u32.to_le_bytes());
        audio.extend_from_slice(&8_000_u32.to_le_bytes());
        audio.extend_from_slice(&1_u16.to_le_bytes());
        audio.extend_from_slice(&8_u16.to_le_bytes());
        audio.extend_from_slice(b"LIST");
        audio.extend_from_slice(&(info.len() as u32).to_le_bytes());
        audio.extend_from_slice(&info);
        if info.len() % 2 != 0 {
            audio.push(0);
        }
        audio.extend_from_slice(b"data");
        audio.extend_from_slice(&8_u32.to_le_bytes());
        audio.extend_from_slice(&[128; 8]);
        let riff_size = u32::try_from(audio.len() - 8).expect("small WAV fixture");
        audio[4..8].copy_from_slice(&riff_size.to_le_bytes());

        let tagged_file = Probe::new(Cursor::new(&audio))
            .guess_file_type()
            .expect("guess tagged WAV")
            .read()
            .expect("read tagged WAV");
        assert!(tagged_file.first_tag().is_some());
        let metadata = read_cloud_audio_metadata(&audio);
        assert_eq!(metadata.song_name.as_deref(), Some("Tagged Song"));
        assert_eq!(metadata.artist.as_deref(), Some("Tagged Artist"));
        assert_eq!(metadata.album.as_deref(), Some("Tagged Album"));
        assert_eq!(cloud_audio_md5(b"abc"), "900150983cd24fb0d6963f7d28e17f72");

        let mut primary = lofty::tag::Tag::new(lofty::tag::TagType::Id3v2);
        primary.set_title(" ".to_owned());
        primary.set_album("Primary Album".to_owned());
        let mut fallback = lofty::tag::Tag::new(lofty::tag::TagType::VorbisComments);
        fallback.set_title("Fallback Song".to_owned());
        fallback.set_artist("Fallback Artist".to_owned());
        fallback.set_album("Fallback Album".to_owned());
        let merged = cloud_audio_metadata_from_tags(Some(&primary), Some(&fallback));
        assert_eq!(merged.song_name.as_deref(), Some("Fallback Song"));
        assert_eq!(merged.artist.as_deref(), Some("Fallback Artist"));
        assert_eq!(merged.album.as_deref(), Some("Primary Album"));
    }

    #[test]
    fn proxy_cloud_upload_resolves_explicit_tag_and_reference_fallback_metadata() {
        let descriptor = cloud_upload_descriptor(
            "0123456789abcdef0123456789abcdef",
            " Track.Name .flac ",
            Some("audio/flac"),
        )
        .expect("cloud upload descriptor");
        let mut request = CloudUploadRequest {
            filename: "Track.Name .flac".to_owned(),
            content_type: "audio/flac".to_owned(),
            data: vec![1],
            bitrate: 999_000,
            song_name: Some("Explicit Song".to_owned()),
            artist: Some("   ".to_owned()),
            album: None,
            account: None,
        };
        let tagged = CloudAudioMetadata {
            song_name: Some("Tagged Song".to_owned()),
            artist: Some("Tagged Artist".to_owned()),
            album: Some("Tagged Album".to_owned()),
        };
        let resolved = resolve_cloud_audio_metadata(&request, &descriptor, &tagged)
            .expect("resolved cloud metadata");
        assert_eq!(resolved.0, "Explicit Song");
        assert_eq!(resolved.1, "Tagged Artist");
        assert_eq!(resolved.2, "Tagged Album");

        request.song_name = None;
        request.artist = None;
        request.album = None;
        let fallback =
            resolve_cloud_audio_metadata(&request, &descriptor, &CloudAudioMetadata::default())
                .expect("fallback cloud metadata");
        assert_eq!(fallback.0, "Track_Name");
        assert_eq!(fallback.1, "未知艺术家");
        assert_eq!(fallback.2, "未知专辑");

        request.artist = Some("artist\r\nheader".to_owned());
        let error = resolve_cloud_audio_metadata(&request, &descriptor, &tagged)
            .expect_err("invalid explicit metadata");
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn cloud_upload_allocation_builds_a_scoped_nos_destination() {
        let allocation = CloudUploadAllocationEnvelope {
            result: crate::dto::CloudUploadAllocation {
                object_key: "folder/song 1".to_owned(),
                token: "upload-secret".to_owned(),
                resource_id: json!(456),
            },
        };
        validate_cloud_upload_allocation(&allocation).expect("valid allocation");
        let upload_url = build_cloud_upload_url(
            "http://nosup-jd1.127.net/",
            CLOUD_UPLOAD_BUCKET,
            &allocation.result.object_key,
        )
        .expect("valid NOS destination");
        assert_eq!(
            upload_url,
            "http://nosup-jd1.127.net/jd-musicrep-privatecloud-audio-public/folder%2Fsong%201?offset=0&complete=true&version=1.0"
        );

        let error = build_cloud_upload_url(
            "https://nosup-jd1.127.net.evil.test",
            CLOUD_UPLOAD_BUCKET,
            "song",
        )
        .expect_err("foreign NOS destination");
        assert_eq!(error.code, ErrorCode::UpstreamError);
    }

    #[test]
    fn maps_cloud_upload_completion_to_a_unified_track_reference() {
        let result = map_cloud_upload_result(
            "123".to_owned(),
            Some(true),
            Some(true),
            json!({ "code": 200, "songId": 123 }),
            json!({ "code": 200 }),
        )
        .expect("cloud upload result");
        assert_eq!(
            result.track_ref.expect("track reference").to_string(),
            "netease:123"
        );
        assert_eq!(result.upload_required, Some(true));
        assert_eq!(result.uploaded, Some(true));
        assert!(result.published);
        assert_eq!(result.extensions["publish_response"]["code"], 200);
    }

    #[tokio::test]
    async fn cloud_upload_transactions_require_authentication_before_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let ticket =
            CloudUploadTicketRequest::new("0123456789abcdef0123456789abcdef", 42, "song.mp3");
        let ticket_error = MusicProvider::cloud_upload_ticket(&provider, &ticket)
            .await
            .expect_err("anonymous cloud upload ticket");
        assert_eq!(ticket_error.code, ErrorCode::AuthenticationRequired);

        let completion = CloudUploadCompleteRequest {
            provisional_track_id: "123".to_owned(),
            resource_id: "resource".to_owned(),
            md5: "0123456789abcdef0123456789abcdef".to_owned(),
            filename: "song.mp3".to_owned(),
            song_name: None,
            artist: None,
            album: None,
            bitrate: 999_000,
            account: None,
        };
        let completion_error = MusicProvider::complete_cloud_upload(&provider, &completion)
            .await
            .expect_err("anonymous cloud upload completion");
        assert_eq!(completion_error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn proxy_cloud_upload_validates_input_and_authentication_before_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let request = CloudUploadRequest {
            filename: "song.mp3".to_owned(),
            content_type: "audio/mpeg".to_owned(),
            data: b"not-a-real-audio-file".to_vec(),
            bitrate: 999_000,
            song_name: None,
            artist: None,
            album: None,
            account: None,
        };
        let error = MusicProvider::upload_cloud_track(&provider, &request)
            .await
            .expect_err("anonymous cloud proxy upload");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);

        let mut invalid = Vec::new();
        let mut empty = request.clone();
        empty.data.clear();
        invalid.push(empty);
        let mut bitrate = request.clone();
        bitrate.bitrate = 0;
        invalid.push(bitrate);
        let mut filename = request.clone();
        filename.filename = "../song.mp3".to_owned();
        invalid.push(filename);
        let mut metadata = request;
        metadata.song_name = Some("song\r\nheader".to_owned());
        invalid.push(metadata);
        for request in invalid {
            let error = MusicProvider::upload_cloud_track(&provider, &request)
                .await
                .expect_err("invalid cloud proxy upload");
            assert_eq!(error.code, ErrorCode::InvalidRequest);
        }
    }

    #[test]
    fn cloud_import_normalizes_reference_units_defaults_and_payloads() {
        let request = CloudImportRequest {
            md5: "D02B8AB79D91C01167BA31E349FE5275".to_owned(),
            source_track_id: None,
            bitrate: 1_652_999,
            file_size: 50_412_168,
            file_type: ".FLAC".to_owned(),
            song_name: "最伟大的作品".to_owned(),
            artist: "   ".to_owned(),
            album: String::new(),
            account: None,
        };
        let descriptor = validate_cloud_import_request(&request).expect("valid cloud import");
        assert_eq!(descriptor.md5, "d02b8ab79d91c01167ba31e349fe5275");
        assert_eq!(descriptor.source_track_id, "-2");
        assert_eq!(descriptor.bitrate_kbps, 1_652);
        assert_eq!(descriptor.file_type, "flac");
        assert_eq!(descriptor.artist, "未知");
        assert_eq!(descriptor.album, "未知");

        let check = cloud_import_check_payload(&descriptor, request.file_size);
        assert_eq!(check["uploadType"], 0);
        let check_songs: Value =
            serde_json::from_str(check["songs"].as_str().expect("serialized check songs"))
                .expect("valid check songs JSON");
        assert_eq!(check_songs[0]["md5"], descriptor.md5);
        assert_eq!(check_songs[0]["songId"], "-2");
        assert_eq!(check_songs[0]["bitrate"], 1_652);
        assert_eq!(check_songs[0]["fileSize"], 50_412_168);

        let import = cloud_import_payload(&descriptor, "123");
        assert_eq!(import["uploadType"], 0);
        let import_songs: Value =
            serde_json::from_str(import["songs"].as_str().expect("serialized import songs"))
                .expect("valid import songs JSON");
        assert_eq!(import_songs[0]["songId"], "123");
        assert_eq!(import_songs[0]["bitrate"], 1_652);
        assert_eq!(import_songs[0]["song"], "最伟大的作品");
        assert_eq!(import_songs[0]["artist"], "未知");
        assert_eq!(import_songs[0]["album"], "未知");
        assert_eq!(import_songs[0]["fileName"], "最伟大的作品.flac");
    }

    #[test]
    fn cloud_import_rejects_invalid_reference_boundaries() {
        let request = CloudImportRequest {
            md5: "d02b8ab79d91c01167ba31e349fe5275".to_owned(),
            source_track_id: None,
            bitrate: 1_652_000,
            file_size: 50_412_168,
            file_type: "flac".to_owned(),
            song_name: "最伟大的作品".to_owned(),
            artist: "周杰伦".to_owned(),
            album: "最伟大的作品".to_owned(),
            account: None,
        };
        let mut invalid = Vec::new();
        let mut file_size = request.clone();
        file_size.file_size = 0;
        invalid.push(file_size);
        let mut bitrate = request.clone();
        bitrate.bitrate = 999;
        invalid.push(bitrate);
        let mut source = request.clone();
        source.source_track_id = Some("-1".to_owned());
        invalid.push(source);
        let mut file_type = request.clone();
        file_type.file_type = "../flac".to_owned();
        invalid.push(file_type);
        let mut song_name = request.clone();
        song_name.song_name = "folder/song".to_owned();
        invalid.push(song_name);
        let mut artist = request;
        artist.artist = "artist\r\nheader".to_owned();
        invalid.push(artist);

        for request in invalid {
            let error =
                validate_cloud_import_request(&request).expect_err("invalid cloud import request");
            assert_eq!(error.code, ErrorCode::InvalidRequest);
        }
    }

    #[test]
    fn maps_cloud_import_status_and_final_track_reference() {
        let result = map_cloud_import_result(
            "123",
            Some(1),
            json!({ "code": 200, "data": [{ "songId": 123, "upload": 1 }] }),
            json!({ "code": 200, "songId": 0, "data": { "songId": 456 } }),
        )
        .expect("cloud import result");
        assert_eq!(
            result.track_ref.expect("track reference").to_string(),
            "netease:456"
        );
        assert!(result.imported);
        assert_eq!(result.already_present, Some(true));
        assert_eq!(result.extensions["upload_status"], 1);
        assert_eq!(result.extensions["check_response"]["code"], 200);
        assert_eq!(result.extensions["import_response"]["code"], 200);
    }

    #[test]
    fn cloud_lyrics_and_match_keep_opaque_ids_and_reference_payloads() {
        assert_eq!(
            cloud_lyrics_payload("32953014", "cloud-song"),
            json!({
                "userId": "32953014",
                "songId": "cloud-song",
                "lv": -1,
                "kv": -1
            })
        );
        assert_eq!(
            cloud_match_payload("32953014", "cloud-song", "185809"),
            json!({
                "userId": "32953014",
                "songId": "cloud-song",
                "adjustSongId": "185809"
            })
        );
        let envelope: LyricsEnvelope = serde_json::from_value(json!({
            "lrc": { "lyric": "[00:01.00]云盘歌词", "version": 1 }
        }))
        .expect("cloud lyrics fixture");
        let lyrics = map_lyrics("cloud-song", envelope).expect("cloud lyrics");
        assert_eq!(lyrics.track_ref.to_string(), "netease:cloud-song");
        assert_eq!(lyrics.plain.as_deref(), Some("[00:01.00]云盘歌词"));

        let matched =
            map_cloud_match_result("cloud-song", "185809", "32953014", json!({ "code": 200 }))
                .expect("matched cloud track");
        assert!(matched.matched);
        assert_eq!(matched.cloud_track_ref.to_string(), "netease:cloud-song");
        assert_eq!(
            matched
                .target_track_ref
                .expect("target track reference")
                .to_string(),
            "netease:185809"
        );

        let canceled =
            map_cloud_match_result("cloud-song", "0", "32953014", json!({ "code": 200 }))
                .expect("canceled cloud match");
        assert!(!canceled.matched);
        assert!(canceled.target_track_ref.is_none());
    }

    #[test]
    fn cloud_library_requests_match_all_reference_protocol_payloads() {
        let (path, payload) = netease_cloud_tracks_request(30, 60);
        assert_eq!(path, "/api/v1/cloud/get");
        assert_eq!(payload, json!({ "limit": 30, "offset": 60 }));

        let ids = vec!["5374627".to_owned(), "9001".to_owned()];
        let (path, payload) = netease_cloud_track_details_request(&ids);
        assert_eq!(path, "/api/v1/cloud/get/byids");
        assert_eq!(payload, json!({ "songIds": ["5374627", "9001"] }));

        let (path, payload) = netease_cloud_track_delete_request(&ids);
        assert_eq!(path, "/api/cloud/del");
        assert_eq!(payload, json!({ "songIds": ["5374627,9001"] }));

        let (path, payload) = netease_cloud_track_download_request("5374627");
        assert_eq!(path, "/api/cloud/dowonload");
        assert_eq!(payload, json!({ "songId": "5374627" }));
    }

    #[test]
    fn maps_cloud_library_tracks_storage_and_matched_metadata() {
        let page = map_netease_cloud_tracks(
            10,
            10,
            json!({
                "code": 200,
                "data": [fixture_cloud_item("9001", 185809)],
                "count": "12",
                "hasMore": true,
                "size": "50412168",
                "maxSize": 1073741824_u64,
                "upgradeSign": 7
            }),
        )
        .expect("map cloud library");
        assert_eq!(page.items.len(), 1);
        let cloud = &page.items[0];
        assert_eq!(cloud.cloud_track_ref.to_string(), "netease:9001");
        assert_eq!(cloud.track.resource_ref, cloud.cloud_track_ref);
        assert_eq!(cloud.track.id, "9001");
        assert_eq!(cloud.track.name, "反方向的钟");
        assert_eq!(cloud.track.artists[0].name, "周杰伦");
        assert_eq!(
            cloud.track.album.as_ref().map(|album| album.name.as_str()),
            Some("Jay")
        );
        assert_eq!(
            cloud
                .track
                .album
                .as_ref()
                .and_then(|album| album.cover_url.as_deref()),
            Some("https://example.test/cloud-cover.jpg")
        );
        assert_eq!(cloud.filename.as_deref(), Some("反方向的钟.flac"));
        assert_eq!(cloud.file_size, Some(50_412_168));
        assert_eq!(cloud.file_type.as_deref(), Some("flac"));
        assert_eq!(cloud.bitrate, Some(999_000));
        assert_eq!(
            cloud.md5.as_deref(),
            Some("d02b8ab79d91c01167ba31e349fe5275")
        );
        assert_eq!(cloud.added_at.as_deref(), Some("2024-01-01T00:00:00Z"));
        assert_eq!(
            cloud
                .matched_track_ref
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("netease:185809")
        );
        assert_eq!(cloud.extensions["cloud_item"]["futureField"]["kept"], true);
        assert_eq!(page.pagination.total, Some(12));
        assert!(page.pagination.has_more);
        assert_eq!(page.pagination.next_offset, Some(11));
        assert_eq!(page.pagination.extensions["storage_size"], "50412168");
        assert_eq!(
            page.pagination.extensions["storage_max_size"],
            1073741824_u64
        );
        assert_eq!(page.pagination.extensions["upgrade_sign"], 7);
        assert_eq!(page.pagination.extensions["response"]["code"], 200);
    }

    #[test]
    fn cloud_library_mapping_tolerates_nullable_song_artist_and_album_names() {
        let mut item = fixture_cloud_item("9001", 9001);
        item["simpleSong"]["name"] = Value::Null;
        item["simpleSong"]["alia"] = Value::Null;
        item["simpleSong"]["ar"] = Value::Null;
        item["simpleSong"]["al"] = Value::Null;

        let cloud = map_netease_cloud_track(item).expect("map nullable cloud metadata");
        assert_eq!(cloud.track.name, "反方向的钟（云盘）");
        assert_eq!(cloud.track.artists.len(), 1);
        assert_eq!(cloud.track.artists[0].name, "周杰伦");
        assert_eq!(
            cloud.track.album.as_ref().map(|album| album.name.as_str()),
            Some("Jay")
        );
        assert_eq!(
            cloud
                .track
                .album
                .as_ref()
                .and_then(|album| album.cover_url.as_deref()),
            Some("https://example.test/cloud-cover.jpg")
        );
    }

    #[test]
    fn cloud_library_mapping_skips_null_song_objects_and_zero_match_ids() {
        let mut item = fixture_cloud_item(" ", 9001);
        item["song"] = item["simpleSong"].clone();
        item["simpleSong"] = Value::Null;
        item["matchedId"] = json!(0);
        item["matchId"] = json!(185809);

        let cloud = map_netease_cloud_track(item).expect("map compatible cloud aliases");
        assert_eq!(cloud.cloud_track_ref.to_string(), "netease:9001");
        assert_eq!(
            cloud
                .matched_track_ref
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("netease:185809")
        );
    }

    #[test]
    fn cloud_detail_mapping_preserves_upstream_order_and_rejects_missing_song_data() {
        let tracks = map_netease_cloud_track_details(json!({
            "code": 200,
            "data": [
                fixture_cloud_item("9002", 185810),
                fixture_cloud_item("9001", 185809)
            ]
        }))
        .expect("map cloud detail batch");
        assert_eq!(
            tracks
                .iter()
                .map(|track| track.cloud_track_ref.id())
                .collect::<Vec<_>>(),
            vec!["9002", "9001"]
        );

        for response in [json!({"code": 200}), json!({"code": 200, "data": [{}]})] {
            assert_eq!(
                map_netease_cloud_track_details(response)
                    .expect_err("malformed cloud detail response")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn maps_cloud_download_available_unavailable_and_array_responses() {
        let available = map_netease_cloud_track_download(
            "9001",
            "/api/cloud/dowonload",
            json!({
                "code": 200,
                "data": {
                    "songId": "9001",
                    "url": "https://example.test/generic-preview.mp3",
                    "downloadUrl": "https://example.test/cloud.flac",
                    "fileType": ".FLAC",
                    "encodeType": "flac",
                    "br": "999000",
                    "fileSize": "50412168",
                    "time": 258000,
                    "level": "lossless",
                    "expi": 1200,
                    "fee": 1,
                    "code": 200,
                    "futureField": true
                }
            }),
        )
        .expect("map available cloud download");
        assert!(available.available);
        assert_eq!(
            available.url.as_deref(),
            Some("https://example.test/cloud.flac")
        );
        assert_eq!(available.track_ref.to_string(), "netease:9001");
        assert_eq!(available.format.as_deref(), Some("flac"));
        assert_eq!(available.codec.as_deref(), Some("flac"));
        assert_eq!(available.bitrate, Some(999_000));
        assert_eq!(available.size, Some(50_412_168));
        assert_eq!(available.duration_ms, Some(258_000));
        assert_eq!(available.actual_quality, Quality::Lossless);
        assert!(available.expires_at.is_some());
        assert_eq!(available.extensions["response_item"]["futureField"], true);
        assert_eq!(available.extensions["request_path"], "/api/cloud/dowonload");

        let unavailable = map_netease_cloud_track_download(
            "9001",
            "/api/cloud/dowonload",
            json!({
                "code": 200,
                "data": [{"songId": "9002", "url": "https://example.test/wrong.mp3"}, {
                    "songId": "9001",
                    "url": null,
                    "message": "unavailable"
                }]
            }),
        )
        .expect("select matching unavailable cloud download");
        assert!(!unavailable.available);
        assert!(unavailable.url.is_none());
        assert_eq!(unavailable.message.as_deref(), Some("unavailable"));

        let top_level = map_netease_cloud_track_download(
            "9001",
            "/api/cloud/dowonload",
            json!({
                "code": 200,
                "name": "cloud-song.MP3",
                "size": "12345",
                "url": "https://example.test/current-shape.mp3"
            }),
        )
        .expect("map current top-level cloud download");
        assert!(top_level.available);
        assert_eq!(top_level.format.as_deref(), Some("mp3"));
        assert_eq!(top_level.size, Some(12_345));
        assert_eq!(top_level.platform_code, Some(200));
    }

    #[test]
    fn cloud_download_mapping_rejects_missing_data_and_wrong_track_ids() {
        for response in [
            json!({"code": 200}),
            json!({"code": 200, "data": null}),
            json!({"code": 200, "data": {"songId": "9002", "url": "https://example.test/a.mp3"}}),
            json!({"code": 200, "data": [{"songId": "9002"}, {"songId": "9003"}]}),
        ] {
            assert_eq!(
                map_netease_cloud_track_download("9001", "/api/cloud/dowonload", response)
                    .expect_err("malformed cloud download response")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[test]
    fn cloud_track_references_preserve_order_duplicates_and_reject_foreign_platforms() {
        let refs = vec![
            ResourceRef::new(Platform::Netease, "9002").expect("cloud ref"),
            ResourceRef::new(Platform::Netease, "9001").expect("cloud ref"),
            ResourceRef::new(Platform::Netease, "9002").expect("duplicate cloud ref"),
        ];
        assert_eq!(
            validate_cloud_track_refs(&refs).expect("valid cloud refs"),
            vec!["9002", "9001", "9002"]
        );
        assert_eq!(
            validate_cloud_track_refs(&[])
                .expect_err("empty cloud refs")
                .code,
            ErrorCode::InvalidRequest
        );
        let foreign = [ResourceRef::new(Platform::Qq, "9001").expect("foreign ref")];
        assert_eq!(
            validate_cloud_track_refs(&foreign)
                .expect_err("foreign cloud ref")
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[tokio::test]
    async fn cloud_account_operations_require_authentication_before_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let import = CloudImportRequest {
            md5: "d02b8ab79d91c01167ba31e349fe5275".to_owned(),
            source_track_id: None,
            bitrate: 1_652_000,
            file_size: 50_412_168,
            file_type: "flac".to_owned(),
            song_name: "最伟大的作品".to_owned(),
            artist: "周杰伦".to_owned(),
            album: "最伟大的作品".to_owned(),
            account: None,
        };
        let import_error = MusicProvider::import_cloud_track(&provider, &import)
            .await
            .expect_err("anonymous cloud import");
        assert_eq!(import_error.code, ErrorCode::AuthenticationRequired);

        let lyrics_error = MusicProvider::cloud_lyrics(
            &provider,
            &CloudLyricsRequest {
                user_id: "32953014".to_owned(),
                track_id: "cloud-song".to_owned(),
                account: None,
            },
        )
        .await
        .expect_err("anonymous cloud lyrics");
        assert_eq!(lyrics_error.code, ErrorCode::AuthenticationRequired);

        let match_error = MusicProvider::match_cloud_track(
            &provider,
            &CloudMatchRequest {
                user_id: "32953014".to_owned(),
                cloud_track_id: "cloud-song".to_owned(),
                target_track_id: None,
                account: None,
            },
        )
        .await
        .expect_err("anonymous cloud match cancellation");
        assert_eq!(match_error.code, ErrorCode::AuthenticationRequired);

        let list_error = MusicProvider::cloud_tracks(&provider, &PageRequest::new(30, 0))
            .await
            .expect_err("anonymous cloud library");
        assert_eq!(list_error.code, ErrorCode::AuthenticationRequired);

        let cloud_ref = ResourceRef::new(Platform::Netease, "9001").expect("cloud ref");
        let detail_error = MusicProvider::cloud_track_details(
            &provider,
            &CloudTrackDetailRequest::new(vec![cloud_ref.clone()]),
        )
        .await
        .expect_err("anonymous cloud details");
        assert_eq!(detail_error.code, ErrorCode::AuthenticationRequired);

        let delete_error = MusicProvider::delete_cloud_tracks(
            &provider,
            &CloudTrackDeleteRequest::new(vec![cloud_ref]),
        )
        .await
        .expect_err("anonymous cloud deletion");
        assert_eq!(delete_error.code, ErrorCode::AuthenticationRequired);

        let download_error = MusicProvider::download_cloud_track(&provider, "9001", None)
            .await
            .expect_err("anonymous cloud download");
        assert_eq!(download_error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn artist_mv_catalog_rejects_cursor_before_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let mut request = ArtistVideoListRequest::new(20, 0);
        request.kind = VideoKind::Mv;
        request.cursor = Some("next".to_owned());
        let error = MusicProvider::artist_videos(&provider, "6452", &request)
            .await
            .expect_err("unsupported MV cursor");
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }

    #[tokio::test]
    async fn followed_artist_new_videos_require_the_selected_account_alias() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let mut request = ArtistUpdatesRequest::new(20);
        request.account = Some("collector".to_owned());
        let error = MusicProvider::account_artist_new_videos(&provider, &request)
            .await
            .expect_err("missing account alias");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn artist_subscription_requires_the_selected_account_alias() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let error =
            MusicProvider::set_artist_subscription(&provider, "6452", true, Some("collector"))
                .await
                .expect_err("missing account alias");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn followed_artist_catalog_requires_the_selected_account_alias() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let mut request = PageRequest::new(25, 0);
        request.account = Some("collector".to_owned());
        let error = MusicProvider::account_following_artists(&provider, &request)
            .await
            .expect_err("missing account alias");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn followed_artist_new_tracks_require_the_selected_account_alias() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let mut request = ArtistUpdatesRequest::new(20);
        request.account = Some("collector".to_owned());
        let error = MusicProvider::account_artist_new_tracks(&provider, &request)
            .await
            .expect_err("missing account alias");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn followed_artist_new_works_require_the_selected_account_alias() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let mut request = ArtistWorksRequest::new(10);
        request.account = Some("collector".to_owned());
        let error = MusicProvider::account_artist_new_works(&provider, &request)
            .await
            .expect_err("missing account alias");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn followed_artist_new_tracks_play_all_requires_the_selected_account_alias() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let error = MusicProvider::account_artist_new_tracks_play_all(&provider, Some("collector"))
            .await
            .expect_err("missing account alias");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[test]
    fn maps_netease_lyrics_and_contributors() {
        let lyrics: LyricsEnvelope = serde_json::from_value(json!({
            "lrc": {"version": 12, "lyric": "[00:01.00]素胚勾勒出青花"},
            "tlyric": {"version": 3, "lyric": "[00:01.00]Blue and white porcelain"},
            "romalrc": {"version": 1, "lyric": "[00:01.00]su pei gou le"},
            "yrc": {"version": 7, "lyric": "[1000,2000](1000,500,0)素胚"},
            "ytlrc": {"version": 2, "lyric": "[1000,2000]Blue porcelain"},
            "yromalrc": null,
            "lyricUser": {"id": 10, "nickname": "歌词贡献者"},
            "transUser": {"id": 0, "userid": 0, "userId": 11, "nickname": "翻译贡献者"},
            "pureMusic": false
        }))
        .expect("valid lyrics fixture");

        let lyrics = map_lyrics("185809", lyrics).expect("map lyrics");
        assert_eq!(lyrics.track_ref.to_string(), "netease:185809");
        assert_eq!(lyrics.format, "yrc");
        assert!(lyrics.plain.is_some_and(|lyrics| lyrics.contains("青花")));
        assert!(lyrics.word_synced.is_some());
        assert_eq!(lyrics.contributors.len(), 2);
        assert_eq!(lyrics.contributors[1].role, "translation");
        assert_eq!(
            lyrics.contributors[1]
                .resource_ref
                .as_ref()
                .map(ToString::to_string)
                .as_deref(),
            Some("netease:11")
        );
        assert_eq!(lyrics.extensions["word_synced_version"], 7);
    }

    #[test]
    fn lyric_primary_format_always_prefers_word_sync_over_line_sync() {
        assert_eq!(primary_lyric_format(true, true), "yrc");
        assert_eq!(primary_lyric_format(true, false), "yrc");
        assert_eq!(primary_lyric_format(false, true), "lrc");
        assert_eq!(primary_lyric_format(false, false), "plain");

        for (fixture, expected) in [
            (
                json!({
                    "lrc": {"lyric": "[00:01.00]逐行歌词"},
                    "yrc": {"lyric": "[1000,1000](1000,500,0)逐字歌词"}
                }),
                "yrc",
            ),
            (json!({"lrc": {"lyric": "[00:01.00]逐行歌词"}}), "lrc"),
            (
                json!({"yrc": {"lyric": "[1000,1000](1000,500,0)逐字歌词"}}),
                "yrc",
            ),
            (json!({}), "plain"),
        ] {
            let envelope: LyricsEnvelope =
                serde_json::from_value(fixture).expect("valid lyric format fixture");
            let lyrics = map_lyrics("185809", envelope).expect("map lyric format fixture");
            assert_eq!(lyrics.format, expected);
        }
    }

    #[test]
    fn maps_netease_stream_quality_expiry_and_trial() {
        let mut track = Track::new(
            ResourceRef::new(Platform::Netease, "123").expect("track reference"),
            "测试歌曲",
        );
        track.duration_ms = Some(258_000);
        let request = StreamRequest {
            quality: Quality::High,
            variant: StreamVariant::Modern,
            bitrate: None,
            account: None,
        };
        let stream: StreamData = serde_json::from_value(json!({
            "id": 123,
            "url": "https://example.test/audio.mp3",
            "br": 320000,
            "size": 1024,
            "code": 200,
            "expi": 1200,
            "type": "mp3",
            "level": "exhigh",
            "encodeType": "  ",
            "time": 0,
            "fee": 1,
            "freeTrialInfo": {"start": 0, "end": 30}
        }))
        .expect("valid stream fixture");

        let stream = map_stream(&track, &request, stream, false).expect("map stream");
        assert_eq!(stream.requested_quality, Quality::High);
        assert_eq!(stream.actual_quality, Quality::High);
        assert_eq!(stream.bitrate, Some(320000));
        assert_eq!(stream.format.as_deref(), Some("mp3"));
        assert_eq!(stream.codec.as_deref(), Some("mp3"));
        assert_eq!(stream.duration_ms, Some(258_000));
        assert_eq!(stream.trial.expect("trial").end_ms, 30_000);
        assert!(
            stream
                .expires_at
                .is_some_and(|expires| expires.ends_with('Z'))
        );
    }

    #[test]
    fn modern_stream_requests_cover_every_reference_level_and_sky_payload() {
        for (quality, level) in [
            (Quality::Standard, "standard"),
            (Quality::Higher, "higher"),
            (Quality::High, "exhigh"),
            (Quality::Lossless, "lossless"),
            (Quality::Hires, "hires"),
            (Quality::Surround, "jyeffect"),
            (Quality::Spatial, "sky"),
            (Quality::Dolby, "dolby"),
            (Quality::Master, "jymaster"),
        ] {
            let request = StreamRequest {
                quality,
                variant: StreamVariant::Modern,
                bitrate: None,
                account: None,
            };
            let (variant, path, payload, mapped_level) =
                netease_stream_request(&[1_969_519_579, 33_894_312], &request);
            assert_eq!(variant, StreamVariant::Modern, "{quality:?}");
            assert_eq!(path, "/api/song/enhance/player/url/v1", "{quality:?}");
            assert_eq!(payload["ids"], "[1969519579,33894312]", "{quality:?}");
            assert_eq!(payload["level"], level, "{quality:?}");
            assert_eq!(payload["encodeType"], "flac", "{quality:?}");
            assert_eq!(mapped_level, Some(level), "{quality:?}");
            if quality == Quality::Spatial {
                assert_eq!(payload["immerseType"], "c51");
            } else {
                assert!(payload.get("immerseType").is_none(), "{quality:?}");
            }
        }

        let request = StreamRequest {
            quality: Quality::Auto,
            variant: StreamVariant::Default,
            bitrate: Some(192_123),
            account: None,
        };
        let (variant, _, payload, level) = netease_stream_request(&[123], &request);
        assert_eq!(variant, StreamVariant::Modern);
        assert_eq!(payload["level"], "exhigh");
        assert!(payload.get("br").is_none());
        assert_eq!(level, Some("exhigh"));
    }

    #[test]
    fn legacy_stream_request_preserves_reference_batch_bitrate_protocol() {
        let request = StreamRequest {
            quality: Quality::High,
            variant: StreamVariant::Legacy,
            bitrate: Some(192_123),
            account: Some("legacy-user".to_owned()),
        };
        let (variant, path, payload, level) =
            netease_stream_request(&[1_969_519_579, 33_894_312], &request);
        assert_eq!(variant, StreamVariant::Legacy);
        assert_eq!(path, "/api/song/enhance/player/url");
        assert_eq!(payload["ids"], r#"["1969519579","33894312"]"#);
        assert_eq!(payload["br"], 192_123);
        assert_eq!(level, None);

        let (_, _, payload, _) = netease_stream_request(
            &[1_969_519_579],
            &StreamRequest {
                quality: Quality::High,
                variant: StreamVariant::Legacy,
                bitrate: None,
                account: None,
            },
        );
        assert_eq!(payload["br"], 320_000);
    }

    #[test]
    fn download_requests_cover_legacy_bitrate_and_every_modern_level() {
        let legacy = StreamRequest {
            quality: Quality::High,
            variant: StreamVariant::Legacy,
            bitrate: Some(192_123),
            account: None,
        };
        let (variant, path, payload, level) = netease_download_request(2_709_812_973, &legacy);
        assert_eq!(variant, StreamVariant::Legacy);
        assert_eq!(path, "/api/song/enhance/download/url");
        assert_eq!(payload["id"], "2709812973");
        assert_eq!(payload["br"], 192_123);
        assert_eq!(level, None);

        for (quality, level) in [
            (Quality::Standard, "standard"),
            (Quality::Higher, "higher"),
            (Quality::High, "exhigh"),
            (Quality::Lossless, "lossless"),
            (Quality::Hires, "hires"),
            (Quality::Surround, "jyeffect"),
            (Quality::Spatial, "sky"),
            (Quality::Dolby, "dolby"),
            (Quality::Master, "jymaster"),
        ] {
            let request = StreamRequest {
                quality,
                variant: StreamVariant::Modern,
                bitrate: Some(1),
                account: None,
            };
            let (variant, path, payload, mapped_level) =
                netease_download_request(2_709_812_973, &request);
            assert_eq!(variant, StreamVariant::Modern, "{quality:?}");
            assert_eq!(path, "/api/song/enhance/download/url/v1", "{quality:?}");
            assert_eq!(payload["id"], "2709812973", "{quality:?}");
            assert_eq!(payload["immerseType"], "c51", "{quality:?}");
            assert_eq!(payload["level"], level, "{quality:?}");
            assert!(payload.get("br").is_none(), "{quality:?}");
            assert_eq!(mapped_level, Some(level), "{quality:?}");
        }
    }

    #[test]
    fn download_mapping_preserves_success_unavailable_and_full_responses() {
        let mut track = Track::new(
            ResourceRef::new(Platform::Netease, "2709812973").expect("track reference"),
            "测试歌曲",
        );
        track.duration_ms = Some(238_378);
        let request = StreamRequest {
            quality: Quality::Higher,
            variant: StreamVariant::Legacy,
            bitrate: Some(192_123),
            account: None,
        };
        let download = map_netease_download(
            &track,
            &request,
            StreamVariant::Legacy,
            "/api/song/enhance/download/url",
            None,
            json!({
                "code": 200,
                "data": {
                    "id": 2709812973_u64,
                    "url": " https://example.test/audio.mp3 ",
                    "br": 192000,
                    "size": 5722605,
                    "code": 200,
                    "expi": 1200,
                    "type": "mp3",
                    "level": "higher",
                    "encodeType": "  ",
                    "time": 0,
                    "fee": 0,
                    "message": null,
                    "freeTrialInfo": null
                }
            }),
        )
        .expect("map available download");
        assert!(download.available);
        assert_eq!(
            download.url.as_deref(),
            Some("https://example.test/audio.mp3")
        );
        assert_eq!(download.requested_quality, Quality::Higher);
        assert_eq!(download.actual_quality, Quality::Higher);
        assert_eq!(download.bitrate, Some(192_000));
        assert_eq!(download.format.as_deref(), Some("mp3"));
        assert_eq!(download.codec.as_deref(), Some("mp3"));
        assert_eq!(download.duration_ms, Some(238_378));
        assert_eq!(download.platform_code, Some(200));
        assert_eq!(download.extensions["variant"], "legacy");
        assert_eq!(download.extensions["response"]["code"], 200);

        let unavailable = map_netease_download(
            &track,
            &StreamRequest {
                quality: Quality::Spatial,
                variant: StreamVariant::Modern,
                bitrate: None,
                account: None,
            },
            StreamVariant::Modern,
            "/api/song/enhance/download/url/v1",
            Some("sky"),
            json!({
                "code": 200,
                "data": [{
                    "id": 2709812973_u64,
                    "url": null,
                    "br": 0,
                    "size": 0,
                    "code": -110,
                    "expi": 1200,
                    "type": null,
                    "level": null,
                    "encodeType": null,
                    "time": 0,
                    "fee": 0,
                    "message": "quality unavailable",
                    "freeTrialInfo": null
                }]
            }),
        )
        .expect("map unavailable download");
        assert!(!unavailable.available);
        assert_eq!(unavailable.url, None);
        assert_eq!(unavailable.actual_quality, Quality::Auto);
        assert_eq!(unavailable.platform_code, Some(-110));
        assert_eq!(unavailable.message.as_deref(), Some("quality unavailable"));
        assert_eq!(unavailable.extensions["requested_level"], "sky");
    }

    #[test]
    fn download_mapping_rejects_missing_data_and_wrong_track_ids() {
        let track = Track::new(
            ResourceRef::new(Platform::Netease, "2709812973").expect("track reference"),
            "测试歌曲",
        );
        let request = StreamRequest::default();
        for response in [
            json!({ "code": 200 }),
            json!({
                "code": 200,
                "data": {
                    "id": 1,
                    "url": null,
                    "br": 0,
                    "size": 0,
                    "code": -110,
                    "type": null,
                    "level": null,
                    "encodeType": null,
                    "fee": 0,
                    "freeTrialInfo": null
                }
            }),
        ] {
            let error = map_netease_download(
                &track,
                &request,
                StreamVariant::Modern,
                "/api/song/enhance/download/url/v1",
                Some("exhigh"),
                response,
            )
            .expect_err("invalid download response");
            assert_eq!(error.code, ErrorCode::UpstreamError);
        }
    }

    #[test]
    fn stream_batch_preserves_input_order_duplicates_failures_and_full_response() {
        let first = Track::new(
            ResourceRef::new(Platform::Netease, "123").expect("first track reference"),
            "first",
        );
        let second = Track::new(
            ResourceRef::new(Platform::Netease, "456").expect("second track reference"),
            "second",
        );
        let request = StreamRequest {
            quality: Quality::High,
            variant: StreamVariant::Modern,
            bitrate: None,
            account: None,
        };
        let response = json!({
            "code": 200,
            "data": [
                {
                    "id": 456,
                    "url": null,
                    "br": 320000,
                    "size": 0,
                    "code": 200,
                    "type": null,
                    "level": "exhigh",
                    "encodeType": null,
                    "fee": 1,
                    "freeTrialInfo": null
                },
                {
                    "id": 123,
                    "url": "https://example.test/123.flac",
                    "br": 320000,
                    "size": 1024,
                    "code": 200,
                    "expi": 1200,
                    "type": "flac",
                    "level": "exhigh",
                    "encodeType": "flac",
                    "time": 258000,
                    "fee": 0,
                    "freeTrialInfo": null
                }
            ]
        });
        let batch = map_netease_stream_batch(
            &[first.clone(), second, first],
            &request,
            false,
            StreamVariant::Modern,
            "/api/song/enhance/player/url/v1",
            Some("exhigh"),
            response,
        )
        .expect("map stream batch");
        assert_eq!(batch.outcomes.len(), 3);
        assert_eq!(batch.outcomes[0].track_ref.to_string(), "netease:123");
        assert_eq!(batch.outcomes[0].status, ResolutionStatus::Success);
        assert_eq!(
            batch.outcomes[0]
                .stream
                .as_ref()
                .map(|stream| stream.url.as_str()),
            Some("https://example.test/123.flac")
        );
        assert_eq!(batch.outcomes[1].track_ref.to_string(), "netease:456");
        assert_eq!(
            batch.outcomes[1].status,
            ResolutionStatus::AuthenticationRequired
        );
        assert_eq!(
            batch.outcomes[1].error_code,
            Some(ErrorCode::AuthenticationRequired)
        );
        assert_eq!(batch.outcomes[2].track_ref.to_string(), "netease:123");
        assert_eq!(batch.outcomes[2].status, ResolutionStatus::Success);
        assert_eq!(batch.extensions["variant"], "modern");
        assert_eq!(
            batch.extensions["request_path"],
            "/api/song/enhance/player/url/v1"
        );
        assert_eq!(batch.extensions["level"], "exhigh");
        assert_eq!(batch.extensions["response"]["code"], 200);
        assert_eq!(batch.outcomes[0].extensions["response_item"]["id"], 123);
    }

    #[test]
    fn stream_batch_reports_omitted_items_and_rejects_bad_tracks_or_shapes() {
        let track = Track::new(
            ResourceRef::new(Platform::Netease, "123").expect("track reference"),
            "test",
        );
        let request = StreamRequest {
            quality: Quality::High,
            variant: StreamVariant::Modern,
            bitrate: None,
            account: None,
        };
        let batch = map_netease_stream_batch(
            std::slice::from_ref(&track),
            &request,
            false,
            StreamVariant::Modern,
            "/api/song/enhance/player/url/v1",
            Some("exhigh"),
            json!({"code": 200, "data": []}),
        )
        .expect("map omitted stream item");
        assert_eq!(batch.outcomes[0].status, ResolutionStatus::UpstreamError);
        assert_eq!(batch.outcomes[0].error_code, Some(ErrorCode::UpstreamError));

        assert_eq!(
            map_netease_stream_batch(
                std::slice::from_ref(&track),
                &request,
                false,
                StreamVariant::Modern,
                "/api/song/enhance/player/url/v1",
                Some("exhigh"),
                json!({"code": 200})
            )
            .expect_err("missing stream data array")
            .code,
            ErrorCode::UpstreamError
        );

        let qq_track = Track::new(
            ResourceRef::new(Platform::Qq, "123").expect("QQ track reference"),
            "test",
        );
        assert_eq!(
            validate_netease_stream_track(&qq_track)
                .expect_err("cross-platform track")
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn maps_track_availability_without_leaking_the_temporary_player_url() {
        let available = map_track_availability(
            1_969_519_579,
            999_000,
            json!({
                "code": 200,
                "data": [{
                    "id": 1969519579_u64,
                    "url": "https://example.test/temporary.mp3",
                    "br": 320000,
                    "size": 8798445,
                    "code": 200,
                    "type": "mp3",
                    "level": "exhigh",
                    "encodeType": "mp3",
                    "fee": 8,
                    "payed": 0,
                    "freeTrialInfo": null,
                    "freeTimeTrialPrivilege": {"remainTime": 0, "type": 0}
                }]
            }),
        )
        .expect("map playable availability");
        assert_eq!(available.track_ref.to_string(), "netease:1969519579");
        assert!(available.playable);
        assert_eq!(available.requested_bitrate, 999_000);
        assert_eq!(available.actual_bitrate, Some(320_000));
        assert_eq!(available.platform_code, Some(200));
        assert_eq!(available.message, "ok");
        assert_eq!(
            available.extensions["response"]["data"][0]["url"],
            Value::Null
        );
        assert_eq!(
            available.extensions["response"]["data"][0]["freeTimeTrialPrivilege"]["type"],
            0
        );

        let unavailable = map_track_availability(
            1,
            128_000,
            json!({
                "code": 200,
                "data": [{
                    "id": 1,
                    "url": null,
                    "br": 0,
                    "size": 0,
                    "code": 404,
                    "type": null,
                    "level": null,
                    "encodeType": null,
                    "fee": 0,
                    "freeTrialInfo": null
                }]
            }),
        )
        .expect("map unavailable result as data");
        assert!(!unavailable.playable);
        assert_eq!(unavailable.actual_bitrate, None);
        assert_eq!(unavailable.platform_code, Some(404));
        assert_eq!(unavailable.message, "亲爱的,暂无版权");
    }

    #[test]
    fn reports_missing_paid_stream_as_authentication_required() {
        let track = Track::new(
            ResourceRef::new(Platform::Netease, "123").expect("track reference"),
            "测试歌曲",
        );
        let request = StreamRequest {
            quality: Quality::Lossless,
            variant: StreamVariant::Modern,
            bitrate: None,
            account: None,
        };
        let stream: StreamData = serde_json::from_value(json!({
            "id": 123,
            "url": null,
            "br": 999000,
            "size": 0,
            "code": 200,
            "expi": 0,
            "type": null,
            "level": "lossless",
            "encodeType": null,
            "time": 258000,
            "fee": 1,
            "freeTrialInfo": null
        }))
        .expect("valid unavailable stream fixture");

        let error = map_stream(&track, &request, stream, false).expect_err("stream must fail");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[test]
    fn formats_unix_time_without_a_datetime_dependency() {
        assert_eq!(unix_rfc3339(0).as_deref(), Some("1970-01-01T00:00:00Z"));
        assert_eq!(
            unix_rfc3339(1_704_067_200).as_deref(),
            Some("2024-01-01T00:00:00Z")
        );
    }

    #[test]
    fn account_aliases_select_isolated_authenticated_clients() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        assert!(
            !provider
                .client_for(Some("default"))
                .expect("default client")
                .is_authenticated()
        );
        let missing = provider
            .client_for(Some("green-diamond"))
            .err()
            .expect("unknown alias");
        assert_eq!(missing.code, ErrorCode::AuthenticationRequired);

        provider
            .install_session("green-diamond", "MUSIC_U=account-session".to_owned())
            .expect("install account session");
        assert!(
            provider
                .client_for(Some("green-diamond"))
                .expect("account client")
                .is_authenticated()
        );
        assert!(
            provider
                .remove_session("green-diamond")
                .expect("remove account session")
        );
        assert!(provider.client_for(Some("green-diamond")).is_err());
    }

    #[test]
    fn account_sessions_persist_across_provider_restarts_and_remain_isolated() {
        let directory = TestCredentialDirectory::new();
        let store = directory.store();
        let provider = NeteaseProvider::new(NeteaseConfig {
            credential_store: Some(store.clone()),
            ..NeteaseConfig::default()
        })
        .expect("build persistent provider");
        provider
            .install_session("personal", "MUSIC_U=personal-session".to_owned())
            .expect("persist personal session");
        provider
            .install_session("premium/账号", "MUSIC_U=premium-session".to_owned())
            .expect("persist premium session");
        drop(provider);

        let restored = NeteaseProvider::new(NeteaseConfig {
            credential_store: Some(store.clone()),
            ..NeteaseConfig::default()
        })
        .expect("restore persistent provider");
        for account in ["personal", "premium/账号"] {
            assert!(
                restored
                    .client_for(Some(account))
                    .expect("restored account client")
                    .is_authenticated(),
                "{account}"
            );
        }
        assert!(
            restored
                .remove_session("personal")
                .expect("remove persisted personal session")
        );
        drop(restored);

        let after_removal = NeteaseProvider::new(NeteaseConfig {
            credential_store: Some(store),
            ..NeteaseConfig::default()
        })
        .expect("restore provider after removal");
        assert!(after_removal.client_for(Some("personal")).is_err());
        assert!(
            after_removal
                .client_for(Some("premium/账号"))
                .expect("remaining persisted account")
                .is_authenticated()
        );
    }

    #[tokio::test]
    async fn logout_removes_the_local_persisted_session_when_upstream_is_unavailable() {
        let directory = TestCredentialDirectory::new();
        let store = directory.store();
        let config = NeteaseConfig {
            base_url: "http://127.0.0.1:1".to_owned(),
            timeout: std::time::Duration::from_millis(100),
            credential_store: Some(store.clone()),
            ..NeteaseConfig::default()
        };
        let provider = NeteaseProvider::new(config.clone()).expect("build persistent provider");
        provider
            .install_session("personal", "MUSIC_U=personal-session".to_owned())
            .expect("persist account session");

        let error = provider
            .logout_account("personal")
            .await
            .expect_err("upstream logout is unavailable");
        assert_eq!(error.details["local_session_removed"], true);
        drop(provider);

        let restored = NeteaseProvider::new(config).expect("restore provider after local logout");
        assert!(restored.client_for(Some("personal")).is_err());
    }

    #[test]
    fn account_aliases_are_validated_before_store_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let alias = "x".repeat(65);
        let error = provider
            .client_for(Some(&alias))
            .err()
            .expect("oversized account alias");
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn advertises_every_implemented_authentication_flow() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let capabilities = provider.capabilities();
        assert!(capabilities.contains(&Capability::QrLogin));
        assert!(capabilities.contains(&Capability::PasswordLogin));
        assert!(capabilities.contains(&Capability::PhoneLogin));
        assert!(capabilities.contains(&Capability::CountryCallingCodes));
        assert!(capabilities.contains(&Capability::ChallengeValidation));
        assert!(capabilities.contains(&Capability::PrincipalStatus));
        assert!(capabilities.contains(&Capability::SessionManagement));
        assert!(capabilities.contains(&Capability::AccountProfile));
        assert!(capabilities.contains(&Capability::AccountPlaylists));
        assert!(capabilities.contains(&Capability::AccountAlbums));
        assert!(capabilities.contains(&Capability::AccountRadioStations));
        assert!(capabilities.contains(&Capability::AccountArtistNewVideos));
        assert!(capabilities.contains(&Capability::AccountArtistNewTracks));
        assert!(capabilities.contains(&Capability::AccountArtistNewWorks));
        assert!(capabilities.contains(&Capability::AccountArtistNewTracksPlayAll));
        assert!(capabilities.contains(&Capability::AccountCloudUpload));
        assert!(capabilities.contains(&Capability::AccountCloudDirectUpload));
        assert!(capabilities.contains(&Capability::AccountCloudImport));
        assert!(capabilities.contains(&Capability::AccountCloudLyrics));
        assert!(capabilities.contains(&Capability::AccountCloudMatch));
        assert!(capabilities.contains(&Capability::AccountCloudRead));
        assert!(capabilities.contains(&Capability::AccountCloudDelete));
        assert!(capabilities.contains(&Capability::AccountCloudDownload));
        assert!(capabilities.contains(&Capability::PlaylistWrite));
        assert!(capabilities.contains(&Capability::Favorites));
        assert!(capabilities.contains(&Capability::ListeningHistory));
        assert!(capabilities.contains(&Capability::Recommendations));
        assert!(capabilities.contains(&Capability::AlbumDetail));
        assert!(capabilities.contains(&Capability::AlbumList));
        assert!(capabilities.contains(&Capability::AlbumStats));
        assert!(capabilities.contains(&Capability::AlbumTrackEntitlements));
        assert!(capabilities.contains(&Capability::TrackAvailability));
        assert!(capabilities.contains(&Capability::AlbumSubscriptionWrite));
        assert!(capabilities.contains(&Capability::DigitalAlbumDetail));
        assert!(capabilities.contains(&Capability::DigitalAlbumList));
        assert!(capabilities.contains(&Capability::DigitalAlbumCharts));
        assert!(capabilities.contains(&Capability::DimensionCharts));
        assert!(capabilities.contains(&Capability::CommentWrite));
        assert!(capabilities.contains(&Capability::CommentsRead));
        assert!(capabilities.contains(&Capability::CommentReactionsRead));
        assert!(capabilities.contains(&Capability::CommentReactionsWrite));
        assert!(capabilities.contains(&Capability::CommentReportsWrite));
        assert!(capabilities.contains(&Capability::CommentThreadStats));
        assert!(capabilities.contains(&Capability::PlatformApi));
        assert!(capabilities.contains(&Capability::PlatformBatch));
        assert!(capabilities.contains(&Capability::RadioTaxonomy));
        assert!(capabilities.contains(&Capability::RadioStationDetail));
        assert!(capabilities.contains(&Capability::RadioStationList));
        assert!(capabilities.contains(&Capability::RadioStationSubscriptionWrite));
        assert!(capabilities.contains(&Capability::PodcastCategories));
        assert!(capabilities.contains(&Capability::PodcastList));
        assert!(capabilities.contains(&Capability::PodcastDetail));
        assert!(capabilities.contains(&Capability::PodcastWorkbenchDetail));
        assert!(capabilities.contains(&Capability::PodcastEpisodeList));
        assert!(capabilities.contains(&Capability::PodcastEpisodeDetail));
        assert!(capabilities.contains(&Capability::PodcastEpisodeWorkbenchDetail));
        assert!(capabilities.contains(&Capability::PodcastEpisodeStream));
        assert!(capabilities.contains(&Capability::PodcastEpisodeLyrics));
    }

    #[test]
    fn validates_netease_extension_api_protocols_and_request_boundaries() {
        assert_eq!(
            NeteaseApiProtocol::parse(None).expect("default protocol"),
            NeteaseApiProtocol::Eapi
        );
        assert_eq!(
            NeteaseApiProtocol::parse(Some("linuxapi")).expect("LinuxAPI"),
            NeteaseApiProtocol::Linuxapi
        );
        assert_eq!(
            NeteaseApiProtocol::parse(Some("xeapi")).expect("XEAPI"),
            NeteaseApiProtocol::Xeapi
        );
        assert_eq!(
            NeteaseApiProtocol::parse(Some("unknown"))
                .expect_err("unknown protocol")
                .code,
            ErrorCode::InvalidRequest
        );

        let valid = PlatformApiRequest::new(
            "/api/search/get?source=tuneweave",
            json!({ "s": "TuneWeave" }),
        );
        assert_eq!(
            validate_platform_api_request(&valid).expect("valid request"),
            "/api/search/get?source=tuneweave"
        );
        for uri in [
            "https://example.com/api/search/get",
            "/api/../login",
            "/api/search/get#fragment",
            " /api/search/get",
        ] {
            let request = PlatformApiRequest::new(uri, json!({}));
            assert_eq!(
                validate_platform_api_request(&request)
                    .expect_err("unsafe uri")
                    .code,
                ErrorCode::InvalidRequest
            );
        }
        let cookie =
            PlatformApiRequest::new("/api/search/get", json!({ "cookie": "MUSIC_U=raw-secret" }));
        assert_eq!(
            validate_platform_api_request(&cookie)
                .expect_err("raw Cookie injection")
                .code,
            ErrorCode::InvalidRequest
        );
    }

    #[test]
    fn platform_api_preserves_reference_special_business_codes() {
        for code in [200, 201, 302, 400, 502, 800, 801, 802, 803] {
            ensure_platform_api_success(&json!({ "code": code }))
                .expect("reference special code remains a raw response");
        }
        let error = ensure_platform_api_success(&json!({
            "code": 401,
            "message": " ",
            "msg": "login"
        }))
        .expect_err("authentication failure");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
        assert_eq!(error.message, "login");
    }

    #[test]
    fn validates_netease_batch_dynamic_request_paths() {
        let mut requests = BTreeMap::new();
        requests.insert(
            "/api/v2/banner/get".to_owned(),
            json!({ "clientType": "pc" }),
        );
        requests.insert(
            "/api/search/get".to_owned(),
            Value::String(r#"{"s":"TuneWeave","type":1}"#.to_owned()),
        );
        let request = PlatformBatchRequest::new(requests);
        validate_platform_batch_request(&request).expect("valid batch");
        let serialized = serialize_netease_batch_requests(&request);
        let banner: Value = serde_json::from_str(
            serialized["/api/v2/banner/get"]
                .as_str()
                .expect("serialized banner parameters"),
        )
        .expect("valid banner parameter JSON");
        assert_eq!(banner["clientType"], "pc");
        assert_eq!(
            serialized["/api/search/get"],
            r#"{"s":"TuneWeave","type":1}"#
        );
        assert_eq!(serialized["e_r"], false);

        assert_eq!(
            validate_platform_batch_request(&PlatformBatchRequest::new(BTreeMap::new()))
                .expect_err("empty batch")
                .code,
            ErrorCode::InvalidRequest
        );
        for uri in [
            "https://example.com/api/search/get",
            "/api/../login",
            "/api/search/get#fragment",
            "/api/search/get ",
        ] {
            let request = PlatformBatchRequest::new(BTreeMap::from([(uri.to_owned(), json!({}))]));
            assert_eq!(
                validate_platform_batch_request(&request)
                    .expect_err("unsafe batch uri")
                    .code,
                ErrorCode::InvalidRequest
            );
        }
    }

    #[tokio::test]
    async fn rejects_unsupported_authentication_variants_before_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let qr_error = MusicProvider::start_qr_login(&provider, Some("mobile"))
            .await
            .expect_err("unsupported QR type");
        assert_eq!(qr_error.code, ErrorCode::InvalidRequest);

        let password_error = MusicProvider::password_login(
            &provider,
            &PasswordLoginRequest {
                account: "default".to_owned(),
                principal_type: PrincipalType::Username,
                principal: "username".to_owned(),
                password: "password".to_owned(),
                password_format: PasswordFormat::Plain,
                country_code: None,
            },
        )
        .await
        .expect_err("unsupported principal type");
        assert_eq!(password_error.code, ErrorCode::InvalidRequest);
    }

    #[test]
    fn maps_netease_login_profile_to_the_unified_account_shape() {
        let profile = map_account_profile(
            "green-diamond",
            NeteaseAccountSummary {
                id: Some("123".to_owned()),
                user_id: Some("456".to_owned()),
                nickname: Some("TuneWeave".to_owned()),
                avatar_url: None,
            },
        );
        assert_eq!(profile.platform, Platform::Netease);
        assert_eq!(profile.account, "green-diamond");
        assert_eq!(profile.user_id.as_deref(), Some("456"));
        assert_eq!(profile.nickname.as_deref(), Some("TuneWeave"));
        assert!(profile.authenticated);

        let fallback_profile = map_account_profile(
            "fallback",
            NeteaseAccountSummary {
                id: Some("123".to_owned()),
                user_id: Some("  ".to_owned()),
                nickname: None,
                avatar_url: None,
            },
        );
        assert_eq!(fallback_profile.user_id.as_deref(), Some("123"));
    }

    #[test]
    fn maps_anonymous_session_without_claiming_authentication() {
        let profile = map_session_profile(
            "missing",
            NeteaseSessionStatus {
                authenticated: false,
                account: NeteaseAccountSummary {
                    id: None,
                    user_id: None,
                    nickname: None,
                    avatar_url: None,
                },
            },
        );
        assert_eq!(profile.account, "missing");
        assert!(!profile.authenticated);
    }

    #[tokio::test]
    async fn unknown_account_session_status_is_anonymous_without_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let profile = MusicProvider::session_profile(&provider, "missing")
            .await
            .expect("anonymous profile");
        assert_eq!(profile.account, "missing");
        assert!(!profile.authenticated);
    }

    #[tokio::test]
    async fn account_playlists_require_the_selected_logged_in_alias() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let error = MusicProvider::account_playlists(
            &provider,
            &PageRequest {
                limit: 30,
                offset: 0,
                account: Some("missing".to_owned()),
            },
        )
        .await
        .expect_err("missing account alias");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn account_albums_require_the_selected_logged_in_alias() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let error = MusicProvider::account_albums(
            &provider,
            &PageRequest {
                limit: 25,
                offset: 0,
                account: Some("missing".to_owned()),
            },
        )
        .await
        .expect_err("missing account alias");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn account_radio_stations_require_the_selected_logged_in_alias() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let error = MusicProvider::account_radio_stations(
            &provider,
            &PageRequest {
                limit: 25,
                offset: 0,
                account: Some("missing".to_owned()),
            },
        )
        .await
        .expect_err("missing account alias");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn account_podcasts_require_the_selected_logged_in_alias() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let error = MusicProvider::account_podcasts(
            &provider,
            &PageRequest {
                limit: 30,
                offset: 0,
                account: Some("missing".to_owned()),
            },
        )
        .await
        .expect_err("missing account alias");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn podcast_voice_list_workbench_detail_requires_a_logged_in_account() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for account in [None, Some("missing")] {
            let error = MusicProvider::podcast_workbench(&provider, "336355127", account)
                .await
                .expect_err("podcast workbench detail requires authentication");
            assert_eq!(error.code, ErrorCode::AuthenticationRequired);
        }
    }

    #[tokio::test]
    async fn podcast_voice_workbench_detail_requires_a_logged_in_account() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for account in [None, Some("missing")] {
            let error = MusicProvider::podcast_episode_workbench(&provider, "2058695201", account)
                .await
                .expect_err("workbench voice detail requires authentication");
            assert_eq!(error.code, ErrorCode::AuthenticationRequired);
        }
    }

    #[tokio::test]
    async fn podcast_subscription_requires_the_selected_logged_in_alias() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for subscribed in [true, false] {
            let error = MusicProvider::set_podcast_subscription(
                &provider,
                "336355127",
                subscribed,
                Some("missing"),
            )
            .await
            .expect_err("missing account alias");
            assert_eq!(error.code, ErrorCode::AuthenticationRequired);
        }
    }

    #[tokio::test]
    async fn radio_station_subscription_requires_the_selected_logged_in_alias() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for subscribed in [true, false] {
            let error = MusicProvider::set_radio_station_subscription(
                &provider,
                "362",
                subscribed,
                Some("missing"),
            )
            .await
            .expect_err("missing account alias");
            assert_eq!(error.code, ErrorCode::AuthenticationRequired);
        }
    }

    #[tokio::test]
    async fn favorite_tracks_require_the_selected_logged_in_alias() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let error = MusicProvider::favorite_tracks(
            &provider,
            &PageRequest {
                limit: 30,
                offset: 0,
                account: Some("missing".to_owned()),
            },
        )
        .await
        .expect_err("missing account alias");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn user_favorite_tracks_validate_user_and_account_before_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let invalid_user = MusicProvider::user_favorite_tracks(
            &provider,
            "not-a-number",
            &PageRequest::new(30, 0),
        )
        .await
        .expect_err("invalid user id");
        assert_eq!(invalid_user.code, ErrorCode::InvalidRequest);

        let missing_account = MusicProvider::user_favorite_tracks(
            &provider,
            "32953014",
            &PageRequest {
                limit: 30,
                offset: 0,
                account: Some("missing".to_owned()),
            },
        )
        .await
        .expect_err("missing account alias");
        assert_eq!(missing_account.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    async fn play_history_requires_a_valid_user_and_selected_account() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let request = PlaybackHistoryRequest::new(PlaybackHistoryPeriod::Week, 30, 0);
        let account_error = MusicProvider::account_history(
            &provider,
            &PlaybackHistoryRequest {
                account: Some("missing".to_owned()),
                ..request.clone()
            },
        )
        .await
        .expect_err("missing account alias");
        assert_eq!(account_error.code, ErrorCode::AuthenticationRequired);

        let invalid_user = MusicProvider::user_history(&provider, "not-a-number", &request)
            .await
            .expect_err("invalid user id");
        assert_eq!(invalid_user.code, ErrorCode::InvalidRequest);

        let missing_account = MusicProvider::user_history(
            &provider,
            "32953014",
            &PlaybackHistoryRequest {
                account: Some("missing".to_owned()),
                ..request
            },
        )
        .await
        .expect_err("missing account alias");
        assert_eq!(missing_account.code, ErrorCode::AuthenticationRequired);
    }

    #[test]
    fn maps_play_history_metadata_without_hiding_the_track() {
        let entry = map_play_history_record(PlayHistoryRecord {
            song: fixture_song(),
            play_count: Some(42),
            score: Some(99),
        })
        .expect("map play history");
        assert_eq!(entry.track.resource_ref.to_string(), "netease:123");
        assert_eq!(entry.play_count, Some(42));
        assert_eq!(entry.score, Some(99));
        assert_eq!(entry.last_played_at, None);
    }

    #[test]
    fn maps_anonymous_play_history_permission_to_authentication_required() {
        let client = NeteaseClient::new(NeteaseConfig::default()).expect("build client");
        let error = ensure_account_access(
            &client,
            &json!({ "code": -2, "message": "无权限访问" }),
            "play history",
        )
        .expect_err("anonymous access must fail");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
        assert_eq!(error.details["upstream_code"], -2);
    }

    #[tokio::test]
    async fn recommendations_require_the_selected_account_alias() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let request = RecommendationRequest {
            limit: 30,
            offset: 0,
            account: Some("missing".to_owned()),
            refresh: true,
        };
        let track_error = MusicProvider::recommended_tracks(&provider, &request)
            .await
            .expect_err("missing track recommendation account");
        assert_eq!(track_error.code, ErrorCode::AuthenticationRequired);
        let playlist_error = MusicProvider::recommended_playlists(&provider, &request)
            .await
            .expect_err("missing playlist recommendation account");
        assert_eq!(playlist_error.code, ErrorCode::AuthenticationRequired);
    }

    #[test]
    fn maps_daily_track_reasons_and_recommended_playlist_extensions() {
        let tracks = map_recommended_tracks(
            RecommendedTracksEnvelope {
                data: crate::dto::RecommendedTracksData {
                    daily_songs: vec![fixture_song()],
                    recommend_reasons: vec![RecommendationReason {
                        song_id: 123,
                        reason: Some("因为你喜欢周杰伦".to_owned()),
                        reason_id: Some(json!("artist")),
                        target_url: Some("orpheus://artist/6452".to_owned()),
                    }],
                },
            },
            30,
            0,
        )
        .expect("map recommended tracks");
        assert_eq!(tracks.items[0].resource_ref.to_string(), "netease:123");
        assert_eq!(
            tracks.items[0].extensions["recommendation"]["reason"],
            "因为你喜欢周杰伦"
        );
        assert_eq!(tracks.pagination.total, Some(1));

        let response: RecommendedPlaylistsEnvelope = serde_json::from_value(json!({
            "recommend": [{
                "id": 99,
                "name": "每日歌单",
                "picUrl": "https://example.test/recommend.jpg",
                "trackCount": 20,
                "copywriter": "根据你的口味生成",
                "alg": "daily"
            }]
        }))
        .expect("recommended playlists fixture");
        let playlists =
            map_recommended_playlists(response, 30, 0).expect("map recommended playlists");
        assert_eq!(playlists.items[0].resource_ref.to_string(), "netease:99");
        assert_eq!(
            playlists.items[0].cover_url.as_deref(),
            Some("https://example.test/recommend.jpg")
        );
        assert_eq!(playlists.items[0].extensions["algorithm"], "daily");
        assert_eq!(
            playlists.items[0].extensions["copywriter"],
            "根据你的口味生成"
        );
    }

    #[test]
    fn paginates_favorite_track_ids_without_reordering_them() {
        let (ids, pagination) = select_page(vec![1, 2, 3, 4], 2, 1);
        assert_eq!(ids, vec![2, 3]);
        assert_eq!(pagination.limit, 2);
        assert_eq!(pagination.offset, 1);
        assert_eq!(pagination.total, Some(4));
        assert_eq!(pagination.next_offset, Some(3));
        assert!(pagination.has_more);

        let (ids, pagination) = select_page(vec![1, 2, 3, 4], 2, 3);
        assert_eq!(ids, vec![4]);
        assert_eq!(pagination.next_offset, None);
        assert!(!pagination.has_more);
    }

    #[test]
    fn maps_account_playlists_to_unified_pagination() {
        let response: UserPlaylistsEnvelope = serde_json::from_value(json!({
            "playlist": [
                {
                    "id": 1,
                    "name": "我喜欢的音乐",
                    "trackCount": 10,
                    "subscribed": false
                },
                {
                    "id": 2,
                    "name": "收藏歌单",
                    "trackCount": 20,
                    "subscribed": true
                }
            ],
            "more": true
        }))
        .expect("user playlists fixture");
        let page = map_user_playlists(response, 2, 4).expect("map user playlists");
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.items[0].resource_ref.to_string(), "netease:1");
        assert_eq!(page.items[1].subscribed, Some(true));
        assert_eq!(page.pagination.offset, 4);
        assert_eq!(page.pagination.next_offset, Some(6));
        assert!(page.pagination.has_more);
    }

    #[test]
    fn maps_account_albums_and_preserves_list_metadata() {
        let raw = json!({
            "code": 200,
            "data": [{
                "id": 32311,
                "name": "The Mass",
                "alias": [],
                "artists": [{"id": 5197, "name": "Era"}],
                "picUrl": "https://example.test/album.jpg",
                "publishTime": 1072886400000_u64,
                "size": 10,
                "company": "Universal Music",
                "type": "专辑",
                "subTime": 1704067200000_u64
            }],
            "count": 1,
            "hasMore": false,
            "paidCount": 0
        });
        let page = map_subscribed_albums_response(
            raw,
            &PageRequest {
                limit: 25,
                offset: 0,
                account: Some("collector".to_owned()),
            },
            25,
        )
        .expect("map subscribed albums");

        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].resource_ref.to_string(), "netease:32311");
        assert_eq!(page.items[0].artists[0].name, "Era");
        assert_eq!(
            page.items[0].extensions["subscription_item"]["subTime"],
            1704067200000_u64
        );
        assert_eq!(page.pagination.total, Some(1));
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.extensions["response"]["paidCount"], 0);
        assert!(page.pagination.extensions["response"].get("data").is_none());
    }

    #[test]
    fn maps_country_calling_code_groups_entries_and_catalog_metadata() {
        let groups = map_netease_country_calling_codes(json!({
            "code": 200,
            "message": null,
            "data": [
                {
                    "label": "常用",
                    "countryList": [
                        {"code": "86", "en": "China", "locale": "CN", "zh": "中国"},
                        {"code": "852", "en": "Hongkong", "locale": "HK", "zh": "中国香港"}
                    ]
                },
                {
                    "label": "A",
                    "countryList": [
                        {"code": "355", "en": "Albania", "locale": "AL", "zh": "阿尔巴尼亚"}
                    ]
                }
            ]
        }))
        .expect("map country calling code catalog");

        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].label, "常用");
        assert_eq!(groups[0].entries.len(), 2);
        assert_eq!(groups[0].entries[0].calling_code, "86");
        assert_eq!(groups[0].entries[0].region_code, "CN");
        assert_eq!(groups[0].entries[0].name, "中国");
        assert_eq!(groups[0].entries[0].english_name, "China");
        assert_eq!(groups[0].entries[0].extensions["response"]["locale"], "CN");
        assert_eq!(groups[0].extensions["catalog_response"]["code"], 200);
        assert!(
            groups[0].extensions["catalog_response"]
                .get("data")
                .is_none()
        );
    }

    #[test]
    fn rejects_malformed_country_calling_code_catalogs() {
        for response in [
            json!({"code": 200}),
            json!({"code": 200, "data": [{"label": "常用"}]}),
            json!({
                "code": 200,
                "data": [{"label": "常用", "countryList": [{"code": "86"}]}]
            }),
        ] {
            assert_eq!(
                map_netease_country_calling_codes(response)
                    .expect_err("malformed country calling code catalog")
                    .code,
                ErrorCode::UpstreamError
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_country_calling_codes_cover_complete_public_catalog() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let groups = MusicProvider::country_calling_codes(
            &provider,
            &CountryCallingCodeListRequest { account: None },
        )
        .await
        .expect("live country calling code catalog");
        let entries = groups
            .iter()
            .flat_map(|group| group.entries.iter())
            .collect::<Vec<_>>();
        let unique_regions = entries
            .iter()
            .map(|entry| entry.region_code.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(groups.len(), 22);
        assert_eq!(entries.len(), 189);
        assert_eq!(unique_regions.len(), 189);
        assert!(entries.iter().any(|entry| {
            entry.calling_code == "86" && entry.region_code == "CN" && entry.name == "中国"
        }));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_audio_recognition_no_match_path() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let recognition = MusicProvider::recognize_audio(
            &provider,
            &AudioRecognitionRequest {
                fingerprint: "invalid-fingerprint".to_owned(),
                duration_seconds: 6,
                account: None,
            },
        )
        .await
        .expect("live audio recognition no-match response");
        assert!(recognition.matches.is_empty());
        assert!(recognition.no_match_reason.is_some());
        assert!(recognition.extensions.contains_key("response"));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_banners_cover_every_reference_client() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for client in [
            BannerClient::Pc,
            BannerClient::Android,
            BannerClient::Iphone,
            BannerClient::Ipad,
        ] {
            let banners = MusicProvider::banners(&provider, &BannerListRequest::new(client))
                .await
                .expect("live banners");
            assert!(!banners.is_empty());
            assert!(banners.iter().all(|banner| !banner.image_url.is_empty()));
            assert!(
                banners
                    .iter()
                    .all(|banner| banner.extensions["client"] == json!(client))
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_broadcast_category_and_region_taxonomy() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let taxonomy = provider
            .radio_taxonomy(&RadioTaxonomyRequest { account: None })
            .await
            .expect("live broadcast taxonomy");
        assert_eq!(taxonomy.categories.len(), 12);
        assert_eq!(taxonomy.regions.len(), 32);
        assert!(
            taxonomy
                .categories
                .iter()
                .any(|category| category.id == "1" && category.name == "音乐台")
        );
        assert!(
            taxonomy
                .regions
                .iter()
                .any(|region| region.id == "407" && region.name == "网络台")
        );
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_podcast_categories() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let taxonomy = MusicProvider::podcast_categories(&provider, None)
            .await
            .expect("live public podcast categories");
        assert!(!taxonomy.categories.is_empty());
        assert!(taxonomy.categories.iter().all(|category| {
            !category.id.is_empty() && !category.name.is_empty() && category.icon_url.is_some()
        }));
        assert_eq!(taxonomy.extensions["response"]["code"], 200);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_hot_podcast_catalog() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let request = PodcastListRequest::new(PodcastCatalog::Hot, 2, 0);
        let page = MusicProvider::podcasts(&provider, &request)
            .await
            .expect("live public hot podcasts");
        assert_eq!(page.items.len(), 2);
        assert!(page.items.iter().all(|podcast| {
            !podcast.id.is_empty() && !podcast.name.is_empty() && podcast.cover_url.is_some()
        }));
        assert!(page.pagination.has_more);
        assert_eq!(page.pagination.next_offset, Some(2));
        assert_eq!(page.pagination.extensions["catalog"], "hot");
        assert_eq!(page.pagination.extensions["response"]["code"], 200);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_featured_podcast_catalog() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let request = PodcastListRequest::new(PodcastCatalog::Featured, 30, 0);
        let page = MusicProvider::podcasts(&provider, &request)
            .await
            .expect("live public featured podcasts");
        assert!(!page.items.is_empty());
        assert!(page.items.iter().all(|podcast| {
            !podcast.id.is_empty() && !podcast.name.is_empty() && podcast.cover_url.is_some()
        }));
        assert_eq!(page.pagination.total, Some(page.items.len() as u64));
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.next_offset, None);
        assert_eq!(page.pagination.extensions["catalog"], "featured");
        assert_eq!(page.pagination.extensions["limit_applied"], false);
        assert_eq!(page.pagination.extensions["response"]["code"], 200);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_personalized_podcast_catalog() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let request = PodcastListRequest::new(PodcastCatalog::Personalized, 3, 0);
        let page = MusicProvider::podcasts(&provider, &request)
            .await
            .expect("live public personalized podcasts");
        assert_eq!(page.items.len(), 3);
        assert!(page.items.iter().all(|podcast| {
            !podcast.id.is_empty() && !podcast.name.is_empty() && podcast.cover_url.is_some()
        }));
        assert_eq!(page.pagination.total, None);
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.next_offset, None);
        assert_eq!(page.pagination.extensions["catalog"], "personalized");
        assert_eq!(page.pagination.extensions["returned_count"], 3);
        assert_eq!(page.pagination.extensions["limit_applied"], true);
        assert_eq!(page.pagination.extensions["response"]["code"], 200);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_category_hot_podcast_catalog() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let mut request = PodcastListRequest::new(PodcastCatalog::CategoryHot, 3, 0);
        request.category_id = Some("2".to_owned());
        let page = MusicProvider::podcasts(&provider, &request)
            .await
            .expect("live public category hot podcasts");
        assert!(page.items.len() >= 3);
        assert!(page.items.iter().all(|podcast| {
            !podcast.id.is_empty() && !podcast.name.is_empty() && podcast.cover_url.is_some()
        }));
        assert!(page.pagination.total.is_some_and(|total| total > 0));
        assert!(page.pagination.has_more);
        assert_eq!(page.pagination.next_offset, Some(3));
        assert_eq!(page.pagination.extensions["catalog"], "category_hot");
        assert_eq!(page.pagination.extensions["category_id"], "2");
        assert_eq!(page.pagination.extensions["limit_applied"], false);
        assert_eq!(page.pagination.extensions["response"]["code"], 200);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_category_featured_podcast_catalog() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let mut request = PodcastListRequest::new(PodcastCatalog::CategoryFeatured, 3, 0);
        request.category_id = Some("2".to_owned());
        let page = MusicProvider::podcasts(&provider, &request)
            .await
            .expect("live public category featured podcasts");
        assert!(page.items.len() >= 3);
        assert!(page.items.iter().all(|podcast| {
            !podcast.id.is_empty() && !podcast.name.is_empty() && podcast.cover_url.is_some()
        }));
        assert_eq!(page.pagination.total, None);
        assert!(page.pagination.has_more);
        assert_eq!(page.pagination.next_offset, None);
        assert_eq!(page.pagination.extensions["catalog"], "category_featured");
        assert_eq!(page.pagination.extensions["category_id"], "2");
        assert_eq!(page.pagination.extensions["limit_applied"], false);
        assert_eq!(page.pagination.extensions["continuation_supported"], false);
        assert_eq!(page.pagination.extensions["response"]["code"], 200);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_today_preferred_podcast_catalog_preserves_public_business_state() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let request = PodcastListRequest::new(PodcastCatalog::TodayPreferred, 30, 0);
        let page = MusicProvider::podcasts(&provider, &request)
            .await
            .expect("live today preferred podcasts");
        assert!(page.items.iter().all(|podcast| {
            !podcast.id.is_empty() && !podcast.name.is_empty() && podcast.cover_url.is_some()
        }));
        assert_eq!(page.pagination.total, None);
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.next_offset, None);
        assert_eq!(page.pagination.extensions["catalog"], "today_preferred");
        assert_eq!(page.pagination.extensions["page"], 0);
        assert_eq!(page.pagination.extensions["limit_applied"], false);
        assert_eq!(page.pagination.extensions["response"]["code"], 200);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_paid_podcast_catalog_preserves_price_and_pagination() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let request = PodcastListRequest::new(PodcastCatalog::Paid, 3, 0);
        let page = MusicProvider::podcasts(&provider, &request)
            .await
            .expect("live public paid podcasts");
        assert_eq!(page.items.len(), 3);
        assert!(page.items.iter().all(|podcast| {
            !podcast.id.is_empty()
                && !podcast.name.is_empty()
                && podcast.cover_url.is_some()
                && podcast.paid == Some(true)
                && podcast
                    .price
                    .as_ref()
                    .is_some_and(|price| price.amount > 0.0 && price.currency == "CNY")
        }));
        assert_eq!(page.pagination.total, None);
        assert!(page.pagination.has_more);
        assert_eq!(page.pagination.next_offset, Some(3));
        assert_eq!(page.pagination.extensions["catalog"], "paid");
        assert_eq!(page.pagination.extensions["limit_applied"], true);
        assert_eq!(page.pagination.extensions["response"]["code"], 200);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_episode_charts_preserve_rank_and_playable_audio_identity() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for kind in [
            PodcastEpisodeChartKind::Popular,
            PodcastEpisodeChartKind::Trending24Hours,
        ] {
            let page = MusicProvider::podcast_episode_chart(
                &provider,
                &PodcastEpisodeChartRequest::new(kind, 3, 0),
            )
            .await
            .expect("live public episode chart");
            assert_eq!(page.items.len(), 3);
            assert!(page.items.iter().all(|entry| {
                entry.rank > 0
                    && !entry.episode.id.is_empty()
                    && !entry.episode.name.is_empty()
                    && entry.episode.audio.is_some()
            }));
            assert_eq!(page.pagination.offset, 0);
            assert_eq!(page.pagination.next_offset, None);
            assert!(!page.pagination.has_more);
            assert_eq!(page.pagination.extensions["kind"], json!(kind));
            assert_eq!(page.pagination.extensions["response"]["code"], 200);
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_podcast_detail_programs_and_episode() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let podcast = MusicProvider::podcast(&provider, "336355127", None)
            .await
            .expect("live public podcast detail");
        assert_eq!(podcast.resource_ref.to_string(), "netease:336355127");
        assert!(!podcast.name.is_empty());
        assert!(podcast.episode_count.is_some_and(|count| count > 0));

        let episodes = MusicProvider::podcast_episodes(
            &provider,
            "336355127",
            &PodcastEpisodeListRequest::new(2, 0),
        )
        .await
        .expect("live public podcast episodes");
        assert_eq!(episodes.items.len(), 2);
        assert!(episodes.pagination.total.is_some_and(|total| total >= 2));
        assert!(episodes.items.iter().all(|episode| {
            episode
                .podcast_ref
                .as_ref()
                .is_some_and(|reference| reference.to_string() == "netease:336355127")
        }));
        assert!(episodes.items.iter().all(|episode| episode.audio.is_some()));

        let episode_id = episodes.items[0].id.clone();
        let episode = MusicProvider::podcast_episode(&provider, &episode_id, None)
            .await
            .expect("live public podcast episode detail");
        assert_eq!(episode.id, episode_id);
        assert!(episode.audio.is_some());
        assert_eq!(episode.extensions["response"]["code"], 200);

        let stream = MusicProvider::podcast_episode_stream(
            &provider,
            &episode.id,
            &StreamRequest::default(),
        )
        .await
        .expect("live public podcast episode stream");
        assert_eq!(stream.episode_ref, episode.resource_ref);
        assert_eq!(
            stream.audio_ref,
            episode.audio.expect("episode audio").resource_ref
        );
        assert!(stream.stream.url.starts_with("http"));
        assert_eq!(stream.stream.resolved_platform, Platform::Netease);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_voice_lyrics_cover_transcript_and_empty_data() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let lyrics = MusicProvider::podcast_episode_lyrics(&provider, "2058695201", None)
            .await
            .expect("live voice transcript");
        assert_eq!(lyrics.episode_ref.to_string(), "netease:2058695201");
        assert_eq!(
            lyrics.audio_ref.expect("voice audio").to_string(),
            "netease:1336048748"
        );
        assert_eq!(lyrics.lyrics.track_ref.to_string(), "netease:1336048748");
        assert!(lyrics.lyrics.plain.is_some_and(|plain| !plain.is_empty()));
        assert_eq!(lyrics.lyrics.format, "netease_voice_json");
        let document: Value = serde_json::from_str(
            lyrics
                .lyrics
                .word_synced
                .as_deref()
                .expect("voice word-synced document"),
        )
        .expect("valid live voice document JSON");
        assert!(
            document
                .get("sents")
                .and_then(Value::as_array)
                .is_some_and(|sentences| !sentences.is_empty())
        );
        assert_eq!(lyrics.extensions["response"]["code"], 200);

        let empty = MusicProvider::podcast_episode_lyrics(&provider, "1367665101", None)
            .await
            .expect("live empty voice lyric response");
        assert_eq!(empty.lyrics.extensions["available"], false);
        assert_eq!(empty.lyrics.plain, None);
        assert_eq!(empty.lyrics.word_synced, None);
        assert_eq!(empty.extensions["response"]["data"], Value::Null);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_broadcast_station_current_info() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let station = MusicProvider::radio_station(&provider, "362", None)
            .await
            .expect("live broadcast station current info");
        assert_eq!(station.resource_ref.to_string(), "netease:362");
        assert!(!station.name.is_empty());
        assert!(
            station
                .stream_url
                .as_deref()
                .is_some_and(|url| url.starts_with("http"))
        );
        assert_eq!(station.subscribed, None);
        assert_eq!(station.extensions["response"]["code"], 200);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_broadcast_station_catalog_filters_and_cursor() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");

        let mut category_request = RadioStationListRequest::new(20);
        category_request.category_id = Some("1".to_owned());
        let first = MusicProvider::radio_stations(&provider, &category_request)
            .await
            .expect("live music broadcast station catalog");
        assert!(!first.items.is_empty());
        assert!(first.pagination.has_more);
        assert_eq!(first.pagination.extensions["response"]["code"], 200);
        let cursor: RadioStationCursor =
            serde_json::from_value(first.pagination.extensions["next_cursor"].clone())
                .expect("live station cursor");

        let first_ids = first
            .items
            .iter()
            .map(|station| station.id.as_str())
            .collect::<BTreeSet<_>>();
        category_request.cursor = Some(cursor);
        let second = MusicProvider::radio_stations(&provider, &category_request)
            .await
            .expect("live second broadcast station catalog page");
        assert!(!second.items.is_empty());
        assert!(
            second
                .items
                .iter()
                .all(|station| !first_ids.contains(station.id.as_str()))
        );

        let mut region_request = RadioStationListRequest::new(20);
        region_request.region_id = Some("407".to_owned());
        let network = MusicProvider::radio_stations(&provider, &region_request)
            .await
            .expect("live network station region");
        assert_eq!(network.pagination.total, Some(4));
        assert!(!network.pagination.has_more);
        assert!(
            network
                .items
                .iter()
                .all(|station| station.region.as_deref() == Some("网络台"))
        );
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_provider_search_and_track_detail() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let page = provider
            .search(&SearchQuery::tracks("反方向的钟", 2, 0))
            .await
            .expect("live provider search");
        let first = page.items.first().expect("at least one song");
        assert!(!first.name.is_empty());
        assert!(!first.artists.is_empty());
        let detail = provider
            .track(&first.id, None)
            .await
            .expect("live track detail");
        assert_eq!(detail.id, first.id);
        assert!(!detail.name.is_empty());
        assert!(!detail.artists.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_dedicated_podcast_search_returns_typed_catalogs() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let page = MusicProvider::search_catalog(
            &provider,
            &SearchQuery {
                query: "故事".to_owned(),
                kind: SearchKind::Podcast,
                variant: SearchVariant::Default,
                limit: 3,
                offset: 0,
                account: None,
            },
        )
        .await
        .expect("live dedicated podcast search");
        assert_eq!(page.pagination.extensions["variant"], "default");
        assert_eq!(
            page.pagination.extensions["request_path"],
            "/api/search/voicelist/get"
        );
        assert_eq!(page.pagination.extensions["response"]["code"], 200);
        assert!(!page.items.is_empty());
        assert!(page.items.iter().all(|item| {
            matches!(item, SearchItem::Podcast(podcast) if !podcast.name.trim().is_empty())
        }));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_default_search_keyword_is_public_and_actionable() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let prompt = MusicProvider::default_search_keyword(
            &provider,
            &SearchDefaultKeywordRequest { account: None },
        )
        .await
        .expect("live default search keyword");
        assert!(!prompt.keyword.is_empty());
        assert!(!prompt.display_text.is_empty());
        assert_eq!(prompt.extensions["response"]["code"], 200);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_trending_searches_cover_brief_and_full_catalogs() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for (detail, minimum_entries) in [
            (SearchTrendingDetail::Brief, 10),
            (SearchTrendingDetail::Full, 20),
        ] {
            let list = MusicProvider::trending_searches(
                &provider,
                &SearchTrendingRequest {
                    detail,
                    account: None,
                },
            )
            .await
            .unwrap_or_else(|error| panic!("live {detail:?} trending searches failed: {error}"));
            assert_eq!(list.detail, detail);
            assert!(list.entries.len() >= minimum_entries);
            assert!(list.entries.iter().all(|entry| !entry.keyword.is_empty()));
            assert_eq!(list.extensions["response"]["code"], 200);
            if detail == SearchTrendingDetail::Full {
                assert!(list.entries.iter().any(|entry| entry.score.is_some()));
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_search_suggestions_cover_web_mobile_and_pc_protocols() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for client in [
            SearchSuggestionClient::Web,
            SearchSuggestionClient::Mobile,
            SearchSuggestionClient::Pc,
        ] {
            let list = MusicProvider::search_suggestions(
                &provider,
                &SearchSuggestionRequest {
                    query: "海阔天空".to_owned(),
                    client,
                    account: None,
                },
            )
            .await
            .unwrap_or_else(|error| panic!("live {client:?} suggestions failed: {error}"));
            assert_eq!(list.client, client);
            assert_eq!(list.query, "海阔天空");
            assert!(
                !list.suggestions.is_empty(),
                "live {client:?} suggestions empty"
            );
            assert!(
                list.suggestions
                    .iter()
                    .all(|suggestion| !suggestion.keyword.is_empty())
            );
            assert_eq!(list.extensions["response"]["code"], 200);
            if client == SearchSuggestionClient::Web {
                assert!(
                    list.suggestions
                        .iter()
                        .any(|suggestion| suggestion.resource.is_some())
                );
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_multi_match_search_preserves_ordered_cross_type_resources() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for (kind, platform_type) in [
            (SearchKind::Track, 1),
            (SearchKind::Album, 10),
            (SearchKind::Artist, 100),
            (SearchKind::Playlist, 1_000),
            (SearchKind::User, 1_002),
            (SearchKind::Mv, 1_004),
            (SearchKind::Lyric, 1_006),
            (SearchKind::Podcast, 1_009),
            (SearchKind::Video, 1_014),
            (SearchKind::Mixed, 1_018),
            (SearchKind::Voice, 2_000),
        ] {
            let result = MusicProvider::search_multi_match(
                &provider,
                &SearchMultiMatchRequest {
                    query: "海阔天空".to_owned(),
                    kind,
                    account: None,
                },
            )
            .await
            .unwrap_or_else(|error| panic!("live {kind:?} multi-match search failed: {error}"));
            assert_eq!(result.query, "海阔天空");
            assert_eq!(result.requested_kind, kind);
            assert_eq!(result.extensions["platform_type"], platform_type);
            assert_eq!(result.extensions["response"]["code"], 200);
            if kind == SearchKind::Track {
                assert!(!result.sections.is_empty());
                assert!(result.sections.iter().any(|section| {
                    section.kind == Some(SearchKind::Artist) && !section.items.is_empty()
                }));
                assert!(result.sections.iter().any(|section| {
                    section.kind == Some(SearchKind::Playlist) && !section.items.is_empty()
                }));
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_local_track_match_covers_match_and_no_match_paths() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let matched = MusicProvider::match_local_track(
            &provider,
            &LocalTrackMatchRequest {
                title: "富士山下".to_owned(),
                album: String::new(),
                artist: "陈奕迅".to_owned(),
                duration_ms: 259_210,
                md5: "bd708d006912a09d827f02e754cf8e56".to_owned(),
                account: None,
            },
        )
        .await
        .expect("live matching local track metadata");
        assert_eq!(matched.matches.len(), 1);
        assert_eq!(matched.matches[0].resource_ref.to_string(), "netease:65766");
        assert_eq!(matched.matches[0].name, "富士山下");
        assert_eq!(matched.extensions["response"]["code"], 200);

        let no_match = MusicProvider::match_local_track(
            &provider,
            &LocalTrackMatchRequest {
                title: "TuneWeave不存在曲目xyz987".to_owned(),
                album: String::new(),
                artist: String::new(),
                duration_ms: 0,
                md5: "00000000000000000000000000000000".to_owned(),
                account: None,
            },
        )
        .await
        .expect("live no-match local metadata response");
        assert!(no_match.matches.is_empty());
        assert_eq!(no_match.extensions["matched_ids"], json!([]));
        assert_eq!(no_match.extensions["response"]["code"], 200);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_user_membership_covers_public_user_and_current_account_auth() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let public = MusicProvider::user_membership(&provider, Some("32953014"), None)
            .await
            .expect("live public user membership");
        assert_eq!(
            public.user_ref.as_ref().map(ToString::to_string).as_deref(),
            Some("netease:32953014")
        );
        assert_eq!(public.level, Some(7));
        assert_eq!(public.extensions["response"]["code"], 200);

        let error = MusicProvider::user_membership(&provider, None, None)
            .await
            .expect_err("anonymous current membership should require authentication");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
        assert_eq!(error.details["upstream_code"], 301);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_cloudsearch_covers_every_reference_type() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for kind in [
            SearchKind::Track,
            SearchKind::Album,
            SearchKind::Artist,
            SearchKind::Playlist,
            SearchKind::User,
            SearchKind::Mv,
            SearchKind::Lyric,
            SearchKind::Podcast,
            SearchKind::Video,
            SearchKind::Mixed,
            SearchKind::Voice,
        ] {
            let page = MusicProvider::search_catalog(
                &provider,
                &SearchQuery {
                    query: "周杰伦".to_owned(),
                    kind,
                    variant: SearchVariant::Cloud,
                    limit: 2,
                    offset: 0,
                    account: None,
                },
            )
            .await
            .unwrap_or_else(|error| panic!("live {kind:?} cloud search failed: {error}"));
            assert_eq!(page.pagination.extensions["response"]["code"], 200);
            assert_eq!(
                page.pagination.extensions["platform_type"],
                netease_cloud_search_type(kind)
            );
            if !matches!(
                kind,
                SearchKind::Video | SearchKind::Mixed | SearchKind::Voice
            ) {
                assert!(!page.items.is_empty(), "live {kind:?} search was empty");
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_legacy_search_covers_every_reference_type_and_voice_path() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for kind in [
            SearchKind::Track,
            SearchKind::Album,
            SearchKind::Artist,
            SearchKind::Playlist,
            SearchKind::User,
            SearchKind::Mv,
            SearchKind::Lyric,
            SearchKind::Podcast,
            SearchKind::Video,
            SearchKind::Mixed,
            SearchKind::Voice,
        ] {
            let page = MusicProvider::search_catalog(
                &provider,
                &SearchQuery {
                    query: "周杰伦".to_owned(),
                    kind,
                    variant: SearchVariant::Legacy,
                    limit: 2,
                    offset: 0,
                    account: None,
                },
            )
            .await
            .unwrap_or_else(|error| panic!("live {kind:?} legacy search failed: {error}"));
            assert_eq!(page.pagination.extensions["response"]["code"], 200);
            assert_eq!(page.pagination.extensions["variant"], "legacy");
            assert_eq!(
                page.pagination.extensions["request_path"],
                if kind == SearchKind::Voice {
                    "/api/search/voice/get"
                } else {
                    "/api/search/get"
                }
            );
            if !matches!(kind, SearchKind::Video | SearchKind::Voice) {
                assert!(!page.items.is_empty(), "live {kind:?} search was empty");
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_platform_api_supports_every_reference_protocol() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for protocol in ["eapi", "weapi", "api", "linuxapi", "xeapi"] {
            let mut request = PlatformApiRequest::new(
                "/api/search/get",
                json!({ "s": "TuneWeave", "type": 1, "limit": 1, "offset": 0 }),
            );
            request.protocol = Some(protocol.to_owned());
            let body = MusicProvider::platform_api(&provider, &request)
                .await
                .unwrap_or_else(|error| panic!("{protocol} request failed: {error}"));
            assert_eq!(body["code"], 200, "{protocol} response");
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_captcha_validation_preserves_an_invalid_code_as_data() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let result = MusicProvider::validate_auth_challenge(
            &provider,
            &AuthChallengeRequest {
                account: "default".to_owned(),
                method: ChallengeMethod::Sms,
                principal: "13800138000".to_owned(),
                country_code: Some("86".to_owned()),
            },
            "0000",
        )
        .await
        .expect("live invalid captcha response");
        assert!(!result.valid);
        assert_ne!(result.platform_code.as_deref(), Some("200"));
        assert!(result.extensions["response"]["code"].is_number());
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_phone_principal_status_covers_registered_and_unregistered_values() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let mut request = AuthPrincipalStatusRequest {
            account: "default".to_owned(),
            principal_type: PrincipalType::Phone,
            principal: "13800138000".to_owned(),
            country_code: Some("86".to_owned()),
        };
        let registered = MusicProvider::auth_principal_status(&provider, &request)
            .await
            .expect("registered phone status");
        assert!(registered.exists);
        assert_eq!(registered.has_password, Some(true));
        assert_eq!(registered.platform_code.as_deref(), Some("200"));
        assert_eq!(
            registered.extensions["response"]["cellphone"],
            "138****8000"
        );

        request.principal = "1".to_owned();
        let unregistered = MusicProvider::auth_principal_status(&provider, &request)
            .await
            .expect("unregistered phone status");
        assert!(!unregistered.exists);
        assert_eq!(unregistered.has_password, Some(false));
        assert_eq!(unregistered.extensions["response"]["exist"], -1);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_platform_batch_supports_every_reference_protocol() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for protocol in ["eapi", "weapi", "api", "linuxapi", "xeapi"] {
            let mut request = PlatformBatchRequest::new(BTreeMap::from([(
                "/api/v2/banner/get".to_owned(),
                json!({ "clientType": "pc" }),
            )]));
            request.protocol = Some(protocol.to_owned());
            let body = MusicProvider::platform_batch(&provider, &request)
                .await
                .unwrap_or_else(|error| panic!("{protocol} batch failed: {error}"));
            assert_eq!(body["code"], 200, "{protocol} response: {body}");
            assert!(
                body["/api/v2/banner/get"].is_object(),
                "{protocol} batch item"
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_platform_batch_decrypts_encrypted_responses() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for protocol in ["eapi", "weapi"] {
            let mut request = PlatformBatchRequest::new(BTreeMap::from([(
                "/api/v2/banner/get".to_owned(),
                json!({ "clientType": "pc" }),
            )]));
            request.protocol = Some(protocol.to_owned());
            request.encrypted_response = true;
            let body = MusicProvider::platform_batch(&provider, &request)
                .await
                .unwrap_or_else(|error| panic!("{protocol} encrypted batch failed: {error}"));
            assert_eq!(body["code"], 200, "{protocol} encrypted response: {body}");
            assert!(
                body["/api/v2/banner/get"].is_object(),
                "{protocol} encrypted batch item"
            );
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_playlist_and_tracks() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let playlist = provider
            .playlist("3778678", None)
            .await
            .expect("live playlist detail");
        assert_eq!(playlist.resource_ref.to_string(), "netease:3778678");
        assert!(!playlist.name.is_empty());

        let page = provider
            .playlist_tracks("3778678", &PageRequest::new(2, 0))
            .await
            .expect("live playlist tracks");
        assert_eq!(page.items.len(), 2);
        assert!(page.pagination.total.is_some_and(|total| total >= 2));
        assert!(page.items.iter().all(|track| !track.artists.is_empty()));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_album_and_tracks() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let album = MusicProvider::album(&provider, "18915", None)
            .await
            .expect("live album detail");
        assert_eq!(album.resource_ref.to_string(), "netease:18915");
        assert!(!album.name.is_empty());
        let tracks = MusicProvider::album_tracks(&provider, "18915", &PageRequest::new(2, 0))
            .await
            .expect("live album tracks");
        assert_eq!(tracks.items.len(), 2);
        assert!(tracks.items.iter().all(|track| !track.name.is_empty()));
        assert!(tracks.pagination.total.is_some_and(|total| total >= 2));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_artist_albums() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let page = MusicProvider::artist_albums(&provider, "6452", &PageRequest::new(5, 0))
            .await
            .expect("live artist albums");
        assert_eq!(page.items.len(), 5);
        assert!(page.items.iter().all(|album| !album.name.is_empty()));
        assert!(
            page.items
                .iter()
                .all(|album| album.artists.iter().any(|artist| artist.name == "周杰伦"))
        );
        assert!(page.pagination.has_more);
        assert_eq!(page.pagination.next_offset, Some(5));
        assert_eq!(page.pagination.extensions["artist"]["id"], 6452);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_artist_catalog() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let mut request = ArtistListRequest::new(2, 0);
        request.category = ArtistCategory::Male;
        request.area = ArtistArea::Western;
        request.initial = Some("b".to_owned());
        let page = MusicProvider::artists(&provider, &request)
            .await
            .expect("live artist catalog");
        assert_eq!(page.items.len(), 2);
        assert!(page.items.iter().all(|artist| !artist.name.is_empty()));
        assert!(page.items.iter().all(|artist| artist.avatar_url.is_some()));
        assert_eq!(page.pagination.next_offset, Some(2));
        assert!(page.pagination.has_more);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_artist_mvs() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let mut request = ArtistVideoListRequest::new(2, 0);
        request.kind = VideoKind::Mv;
        let page = MusicProvider::artist_videos(&provider, "6452", &request)
            .await
            .expect("live artist MVs");
        assert_eq!(page.items.len(), 2);
        assert!(page.items.iter().all(|video| !video.title.is_empty()));
        assert!(page.items.iter().all(|video| !video.creators.is_empty()));
        assert_eq!(page.pagination.next_offset, Some(2));
        assert!(page.pagination.has_more);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_mv_detail_stats_and_every_advertised_resolution() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let detail_request = VideoDetailRequest::new(VideoResourceKind::Mv);
        let detail = MusicProvider::video(&provider, "22695250", &detail_request)
            .await
            .expect("live MV detail");
        assert_eq!(detail.video.resource_ref.to_string(), "netease:22695250");
        assert!(!detail.video.title.is_empty());
        assert!(!detail.video.creators.is_empty());
        assert!(
            detail
                .video
                .duration_ms
                .is_some_and(|duration| duration > 0)
        );
        assert!(!detail.resolutions.is_empty());
        assert_eq!(detail.extensions["response"]["code"], 200);

        let stats = MusicProvider::video_stats(&provider, "22695250", &detail_request)
            .await
            .expect("live MV stats");
        assert_eq!(stats.video_ref.to_string(), "netease:22695250");
        assert!(stats.like_count.is_some());
        assert!(stats.comment_count.is_some());
        assert!(stats.share_count.is_some());
        assert_eq!(stats.extensions["response"]["code"], 200);

        for resolution in detail
            .resolutions
            .iter()
            .map(|resolution| resolution.resolution)
        {
            let stream = MusicProvider::video_stream(
                &provider,
                "22695250",
                &VideoStreamRequest::new(VideoResourceKind::Mv, resolution),
            )
            .await
            .unwrap_or_else(|error| panic!("live {resolution}p MV stream failed: {error}"));
            assert!(stream.available, "{resolution}p");
            assert!(
                stream
                    .url
                    .as_deref()
                    .is_some_and(|url| url.starts_with("http")),
                "{resolution}p"
            );
            assert_eq!(stream.requested_resolution, resolution, "{resolution}p");
            assert_eq!(stream.actual_resolution, Some(resolution), "{resolution}p");
            assert_eq!(stream.platform_code, Some(200), "{resolution}p");
            assert_eq!(stream.extensions["response"]["code"], 200, "{resolution}p");
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_cloud_video_stale_sample_keeps_stats_unavailable_stream_and_missing_detail() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let id = "89ADDE33C0AAE8EC14B99F6750DB954D";
        let detail_request = VideoDetailRequest::new(VideoResourceKind::Video);

        let detail_error = MusicProvider::video(&provider, id, &detail_request)
            .await
            .expect_err("stale reference video detail must be missing");
        assert_eq!(detail_error.code, ErrorCode::ResourceNotFound);

        let stats = MusicProvider::video_stats(&provider, id, &detail_request)
            .await
            .expect("live cloud video stats");
        assert_eq!(stats.video_ref.to_string(), format!("netease:{id}"));
        assert!(stats.like_count.is_some());
        assert!(stats.comment_count.is_some());
        assert!(stats.share_count.is_some());
        assert_eq!(stats.extensions["response"]["code"], 200);

        let stream = MusicProvider::video_stream(
            &provider,
            id,
            &VideoStreamRequest::new(VideoResourceKind::Video, 1080),
        )
        .await
        .expect("stale cloud video stream response");
        assert!(!stream.available);
        assert_eq!(stream.url, None);
        assert_eq!(stream.platform_code, Some(200));
        assert_eq!(stream.extensions["response"]["code"], 200);
        assert_eq!(stream.extensions["response"]["urls"], json!([]));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_artist_videos() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let request = ArtistVideoListRequest::new(2, 0);
        let page = MusicProvider::artist_videos(&provider, "2116", &request)
            .await
            .expect("live artist videos");
        assert_eq!(page.items.len(), 2);
        assert!(page.items.iter().all(|video| !video.title.is_empty()));
        assert!(page.items.iter().all(|video| !video.creators.is_empty()));
        assert!(
            page.pagination.extensions["next_cursor"]
                .as_str()
                .is_some_and(|cursor| !cursor.is_empty())
        );
        assert!(page.pagination.has_more);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_artist_tracks() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let page =
            MusicProvider::artist_tracks(&provider, "6452", &ArtistTrackListRequest::new(2, 0))
                .await
                .expect("live artist tracks");
        assert_eq!(page.items.len(), 2);
        assert!(page.items.iter().all(|track| !track.name.is_empty()));
        assert!(
            page.items
                .iter()
                .all(|track| track.artists.iter().any(|artist| artist.name == "周杰伦"))
        );
        assert!(page.pagination.total.is_some_and(|total| total > 2));
        assert_eq!(page.pagination.next_offset, Some(2));
        assert!(page.pagination.has_more);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_artist_top_tracks() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let page = MusicProvider::artist_top_tracks(&provider, "6452", None)
            .await
            .expect("live artist top tracks");
        assert_eq!(page.items.len(), 50);
        assert!(page.items.iter().all(|track| !track.name.is_empty()));
        assert!(
            page.items
                .iter()
                .all(|track| track.artists.iter().any(|artist| artist.name == "周杰伦"))
        );
        assert_eq!(page.pagination.limit, 50);
        assert_eq!(page.pagination.total, Some(50));
        assert_eq!(page.pagination.next_offset, None);
        assert!(!page.pagination.has_more);
        assert!(page.pagination.extensions.contains_key("response"));
    }

    #[tokio::test]
    #[ignore = "requires NETEASE_COOKIE for a live logged-in account"]
    async fn live_account_followed_artist_new_videos() {
        let cookie = std::env::var("NETEASE_COOKIE").expect("NETEASE_COOKIE must be set");
        let provider = NeteaseProvider::new(NeteaseConfig {
            cookie: Some(cookie),
            ..NeteaseConfig::default()
        })
        .expect("build provider");
        let page =
            MusicProvider::account_artist_new_videos(&provider, &ArtistUpdatesRequest::new(2))
                .await
                .expect("live followed artist new videos");
        assert!(page.items.len() <= 2);
        assert!(page.items.iter().all(|video| !video.title.is_empty()));
        assert!(page.pagination.extensions.contains_key("response"));
    }

    #[tokio::test]
    #[ignore = "requires NETEASE_COOKIE for a live logged-in account"]
    async fn live_account_followed_artist_new_tracks() {
        let cookie = std::env::var("NETEASE_COOKIE").expect("NETEASE_COOKIE must be set");
        let provider = NeteaseProvider::new(NeteaseConfig {
            cookie: Some(cookie),
            ..NeteaseConfig::default()
        })
        .expect("build provider");
        let page =
            MusicProvider::account_artist_new_tracks(&provider, &ArtistUpdatesRequest::new(2))
                .await
                .expect("live followed artist new tracks");
        assert!(page.items.len() <= 2);
        assert!(page.items.iter().all(|track| !track.name.is_empty()));
        assert!(page.pagination.extensions.contains_key("response"));
    }

    #[tokio::test]
    #[ignore = "requires NETEASE_COOKIE for a live logged-in account"]
    async fn live_account_followed_artist_new_works() {
        let cookie = std::env::var("NETEASE_COOKIE").expect("NETEASE_COOKIE must be set");
        let provider = NeteaseProvider::new(NeteaseConfig {
            cookie: Some(cookie),
            ..NeteaseConfig::default()
        })
        .expect("build provider");
        let page = MusicProvider::account_artist_new_works(&provider, &ArtistWorksRequest::new(2))
            .await
            .expect("live followed artist new works");
        assert!(page.items.len() <= 2);
        assert!(page.pagination.extensions.contains_key("response"));
    }

    #[tokio::test]
    #[ignore = "requires NETEASE_COOKIE for a live logged-in account"]
    async fn live_account_followed_artist_new_tracks_play_all() {
        let cookie = std::env::var("NETEASE_COOKIE").expect("NETEASE_COOKIE must be set");
        let provider = NeteaseProvider::new(NeteaseConfig {
            cookie: Some(cookie),
            ..NeteaseConfig::default()
        })
        .expect("build provider");
        let page = MusicProvider::account_artist_new_tracks_play_all(&provider, None)
            .await
            .expect("live followed artist new tracks play-all");
        assert!(page.items.len() <= 50);
        assert!(page.items.iter().all(|track| !track.name.is_empty()));
        assert!(!page.pagination.has_more);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_artist_detail_and_description() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let artist = MusicProvider::artist(&provider, "6452", None)
            .await
            .expect("live artist detail");
        assert_eq!(artist.resource_ref.to_string(), "netease:6452");
        assert_eq!(artist.name, "周杰伦");
        assert!(artist.aliases.iter().any(|alias| alias == "Jay Chou"));
        assert!(!artist.description.is_empty());
        assert!(!artist.biography_sections.is_empty());
        assert!(artist.track_count.is_some_and(|count| count > 0));
        assert!(artist.extensions.contains_key("detail_response"));
        assert!(artist.extensions.contains_key("description_response"));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_artist_overview() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let overview = MusicProvider::artist_overview(&provider, "6452", None)
            .await
            .expect("live artist overview");
        assert_eq!(overview.artist.resource_ref.to_string(), "netease:6452");
        assert_eq!(overview.artist.name, "周杰伦");
        assert_eq!(overview.featured_tracks.len(), 50);
        assert!(
            overview
                .featured_tracks
                .iter()
                .all(|track| track.artists.iter().any(|artist| artist.name == "周杰伦"))
        );
        assert!(overview.has_more_tracks);
        assert!(overview.extensions.contains_key("response"));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_artist_dynamic_stats() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let stats = MusicProvider::artist_stats(&provider, "6452", None)
            .await
            .expect("live artist stats");
        assert_eq!(stats.artist_ref.to_string(), "netease:6452");
        assert!(stats.followed.is_some());
        assert!(stats.follower_count.is_some_and(|count| count > 0));
        assert!(!stats.video_counts.is_empty());
        assert!(stats.extensions.contains_key("response"));
        assert!(stats.extensions.contains_key("follow_count_response"));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_artist_fans() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let page = MusicProvider::artist_fans(&provider, "2116", &PageRequest::new(2, 0))
            .await
            .expect("live artist fans");
        assert_eq!(page.items.len(), 2);
        assert!(page.items.iter().all(|user| !user.name.is_empty()));
        assert!(page.items.iter().all(|user| user.avatar_url.is_some()));
        assert_eq!(page.pagination.next_offset, Some(2));
        assert!(page.pagination.has_more);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_new_album_catalog() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let mut request = AlbumListRequest::new(2, 0);
        request.catalog = Some("new".to_owned());
        request.area = Some("ALL".to_owned());
        let page = MusicProvider::albums(&provider, &request)
            .await
            .expect("live new album catalog");
        assert_eq!(page.items.len(), 2);
        assert!(page.pagination.total.is_some_and(|total| total >= 2));
        assert!(page.items.iter().all(|album| !album.name.is_empty()));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_newest_album_catalog() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let mut request = AlbumListRequest::new(2, 0);
        request.catalog = Some("newest".to_owned());
        let page = MusicProvider::albums(&provider, &request)
            .await
            .expect("live newest album catalog");
        assert_eq!(page.items.len(), 2);
        assert!(page.pagination.total.is_some_and(|total| total >= 2));
        assert!(page.items.iter().all(|album| !album.name.is_empty()));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_album_stats() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let stats = MusicProvider::album_stats(&provider, "32311", None)
            .await
            .expect("live album stats");
        assert_eq!(stats.album_ref.to_string(), "netease:32311");
        assert!(stats.comment_count.is_some());
        assert!(stats.share_count.is_some());
        assert!(stats.subscriber_count.is_some());
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_album_track_entitlements() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let page = MusicProvider::album_track_entitlements(
            &provider,
            "168223858",
            &PageRequest::new(2, 0),
        )
        .await
        .expect("live album track entitlements");
        assert_eq!(page.items.len(), 2);
        assert!(page.pagination.total.is_some_and(|total| total >= 2));
        assert!(
            page.items
                .iter()
                .all(|entitlement| !entitlement.available_qualities.is_empty())
        );
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_album_subscription_requires_authentication_without_a_session() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let error = MusicProvider::set_album_subscription(&provider, "32311", true, None)
            .await
            .expect_err("anonymous album subscription must fail");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_artist_subscription_requires_authentication_without_a_session() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let error = MusicProvider::set_artist_subscription(&provider, "6452", true, None)
            .await
            .expect_err("anonymous artist subscription must fail");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_account_albums_require_authentication_without_a_session() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let error = MusicProvider::account_albums(&provider, &PageRequest::new(2, 0))
            .await
            .expect_err("anonymous subscribed albums must fail");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_broadcast_station_collection_requires_authentication_without_a_session() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let error = MusicProvider::account_radio_stations(&provider, &PageRequest::new(2, 0))
            .await
            .expect_err("anonymous broadcast station collection must fail");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_broadcast_station_subscription_actions_require_authentication() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for subscribed in [true, false] {
            let error =
                MusicProvider::set_radio_station_subscription(&provider, "362", subscribed, None)
                    .await
                    .expect_err("anonymous broadcast station subscription must fail");
            assert_eq!(error.code, ErrorCode::AuthenticationRequired);
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_followed_artist_catalog_requires_authentication_without_a_session() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let error = MusicProvider::account_following_artists(&provider, &PageRequest::new(2, 0))
            .await
            .expect_err("anonymous followed artist catalog must fail");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_digital_album_detail() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let album = MusicProvider::digital_album(&provider, "120605500", None)
            .await
            .expect("live digital album detail");
        assert_eq!(album.resource_ref.to_string(), "netease:120605500");
        assert!(!album.name.is_empty());
        assert!(!album.artists.is_empty());
        assert!(album.price.is_some());
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_digital_album_list() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let page = MusicProvider::digital_albums(&provider, &DigitalAlbumListRequest::new(2, 0))
            .await
            .expect("live digital album list");
        assert_eq!(page.items.len(), 2);
        assert_eq!(page.pagination.total, None);
        assert!(page.items.iter().all(|album| !album.name.is_empty()));
        assert!(page.items.iter().all(|album| album.price.is_some()));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_digital_album_style_catalog() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let mut request = DigitalAlbumListRequest::new(2, 0);
        request.catalog = Some("style".to_owned());
        request.area = Some("ZH".to_owned());
        let page = MusicProvider::digital_albums(&provider, &request)
            .await
            .expect("live digital album style catalog");
        assert_eq!(page.items.len(), 2);
        assert!(page.pagination.has_more);
        assert!(page.items.iter().all(|album| !album.name.is_empty()));
        assert!(page.items.iter().all(|album| album.price.is_some()));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_digital_album_chart_periods_and_kinds() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for (period, kind, year) in [
            (
                DigitalAlbumChartPeriod::Daily,
                DigitalAlbumChartKind::Album,
                None,
            ),
            (
                DigitalAlbumChartPeriod::Daily,
                DigitalAlbumChartKind::Single,
                None,
            ),
            (
                DigitalAlbumChartPeriod::Week,
                DigitalAlbumChartKind::Album,
                None,
            ),
            (
                DigitalAlbumChartPeriod::Year,
                DigitalAlbumChartKind::Album,
                Some(2025_u16),
            ),
            (
                DigitalAlbumChartPeriod::Total,
                DigitalAlbumChartKind::Album,
                None,
            ),
        ] {
            let mut request = DigitalAlbumChartRequest::new(2, 0);
            request.period = period;
            request.kind = kind;
            request.year = year;
            let page = MusicProvider::digital_album_chart(&provider, &request)
                .await
                .expect("live digital album chart");
            assert_eq!(page.items.len(), 2);
            assert!(page.pagination.total.is_some_and(|total| total >= 2));
            assert_eq!(page.items[0].rank, 1);
            assert!(!page.items[0].product.name.is_empty());
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_general_and_artist_charts_cover_every_reference_branch() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        for view in [
            ChartCatalogView::Overview,
            ChartCatalogView::Summary,
            ChartCatalogView::Modern,
        ] {
            let catalog = MusicProvider::chart_catalog(&provider, &ChartCatalogRequest::new(view))
                .await
                .unwrap_or_else(|error| panic!("live {view:?} chart catalog failed: {error}"));
            assert_eq!(catalog.view, view, "{view:?}");
            assert!(!catalog.groups.is_empty(), "{view:?}");
            assert!(
                catalog.groups.iter().any(|group| !group.charts.is_empty()),
                "{view:?}"
            );
            assert!(
                catalog
                    .groups
                    .iter()
                    .flat_map(|group| &group.charts)
                    .all(|chart| !chart.name.is_empty()),
                "{view:?}"
            );
            assert_eq!(catalog.extensions["response"]["code"], 200, "{view:?}");
        }

        for area in [
            ArtistChartArea::Chinese,
            ArtistChartArea::Western,
            ArtistChartArea::Korean,
            ArtistChartArea::Japanese,
        ] {
            let chart = MusicProvider::artist_chart(&provider, &ArtistChartRequest::new(area))
                .await
                .unwrap_or_else(|error| panic!("live {area:?} artist chart failed: {error}"));
            assert_eq!(chart.area, area, "{area:?}");
            assert!(!chart.entries.is_empty(), "{area:?}");
            assert!(chart.entries.len() <= 100, "{area:?}");
            assert_eq!(chart.entries[0].rank, 1, "{area:?}");
            assert!(!chart.entries[0].artist.name.is_empty(), "{area:?}");
            assert_eq!(chart.extensions["response"]["code"], 200, "{area:?}");
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_dimension_chart_detail_and_track_snapshot() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let detail_request = DimensionChartRequest::new("CITY_SONG_CHART", "110000", "CITY");
        let detail = MusicProvider::dimension_chart(&provider, &detail_request)
            .await
            .expect("live dimension chart detail");
        assert_eq!(detail.chart_code, "CITY_SONG_CHART");
        assert_eq!(detail.target_id, "110000");
        assert!(!detail.name.is_empty());
        assert!(detail.updated_at_ms.is_some());

        let tracks_request =
            DimensionChartRequest::new("CITY_STYLE_SONG_CHART", "110000_1020", "CITY_STYLE");
        let snapshot = MusicProvider::dimension_chart_tracks(&provider, &tracks_request)
            .await
            .expect("live dimension chart track snapshot");
        assert_eq!(snapshot.chart_code, "CITY_STYLE_SONG_CHART");
        assert_eq!(snapshot.target_id, "110000_1020");
        assert!(!snapshot.entries.is_empty());
        assert_eq!(snapshot.entries[0].rank, 1);
        assert!(!snapshot.entries[0].track.name.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_liked_track_ids_require_authentication_without_a_session() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let response = provider
            .client
            .request_eapi("/api/song/like/get", json!({ "uid": "32953014" }))
            .await
            .expect("live liked track ids");
        let error = ensure_success(&response.body).expect_err("anonymous request must fail");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    #[ignore = "requires NETEASE_COOKIE for a live logged-in account"]
    async fn live_account_favorite_tracks() {
        let cookie = std::env::var("NETEASE_COOKIE").expect("NETEASE_COOKIE must be set");
        let provider = NeteaseProvider::new(NeteaseConfig {
            cookie: Some(cookie),
            ..NeteaseConfig::default()
        })
        .expect("build provider");
        let page = MusicProvider::favorite_tracks(&provider, &PageRequest::new(2, 0))
            .await
            .expect("live account favorite tracks");
        assert!(page.pagination.total.is_some());
        assert!(page.items.iter().all(|track| !track.name.is_empty()));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_play_history_requires_authentication_without_a_session() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let error = MusicProvider::user_history(
            &provider,
            "32953014",
            &PlaybackHistoryRequest::new(PlaybackHistoryPeriod::Week, 20, 0),
        )
        .await
        .expect_err("anonymous request must fail");
        assert_eq!(
            error.code,
            ErrorCode::AuthenticationRequired,
            "unexpected live error: {error:?}"
        );
    }

    #[tokio::test]
    #[ignore = "requires NETEASE_COOKIE for a live logged-in account"]
    async fn live_account_play_history() {
        let cookie = std::env::var("NETEASE_COOKIE").expect("NETEASE_COOKIE must be set");
        let provider = NeteaseProvider::new(NeteaseConfig {
            cookie: Some(cookie),
            ..NeteaseConfig::default()
        })
        .expect("build provider");
        let page = MusicProvider::account_history(
            &provider,
            &PlaybackHistoryRequest::new(PlaybackHistoryPeriod::Week, 20, 0),
        )
        .await
        .expect("live account play history");
        assert!(page.pagination.total.is_some());
        assert!(page.items.iter().all(|entry| !entry.track.name.is_empty()));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_anonymous_daily_track_recommendations_are_usable() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let request = RecommendationRequest::new(30, 0);
        let tracks = MusicProvider::recommended_tracks(&provider, &request)
            .await
            .expect("anonymous daily tracks");
        assert!(!tracks.items.is_empty());
        assert!(tracks.items.iter().all(|track| !track.name.is_empty()));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_daily_playlist_recommendations_require_authentication() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let error =
            MusicProvider::recommended_playlists(&provider, &RecommendationRequest::new(30, 0))
                .await
                .expect_err("anonymous daily playlists must fail");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }

    #[tokio::test]
    #[ignore = "requires NETEASE_COOKIE for a live logged-in account"]
    async fn live_daily_recommendations() {
        let cookie = std::env::var("NETEASE_COOKIE").expect("NETEASE_COOKIE must be set");
        let provider = NeteaseProvider::new(NeteaseConfig {
            cookie: Some(cookie),
            ..NeteaseConfig::default()
        })
        .expect("build provider");
        let request = RecommendationRequest::new(30, 0);
        let tracks = MusicProvider::recommended_tracks(&provider, &request)
            .await
            .expect("live recommended tracks");
        let playlists = MusicProvider::recommended_playlists(&provider, &request)
            .await
            .expect("live recommended playlists");
        assert!(!tracks.items.is_empty());
        assert!(!playlists.items.is_empty());
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_track_lyrics() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let lyrics = provider
            .lyrics("185809", None)
            .await
            .expect("live track lyrics");
        assert_eq!(lyrics.track_ref.to_string(), "netease:185809");
        assert!(lyrics.plain.is_some());
        assert!(lyrics.word_synced.is_some());
        assert_eq!(lyrics.format, "yrc");
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_public_track_stream() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let track = Track::new(
            ResourceRef::new(Platform::Netease, "2709812973").expect("track reference"),
            "live stream fixture",
        );
        let stream = provider
            .stream(
                &track,
                &StreamRequest {
                    quality: Quality::High,
                    variant: StreamVariant::Modern,
                    bitrate: None,
                    account: None,
                },
            )
            .await
            .expect("live track stream");
        assert!(stream.url.starts_with("http"));
        assert_eq!(stream.resolved_track.to_string(), "netease:2709812973");
        assert!(stream.bitrate.is_some_and(|bitrate| bitrate > 0));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_modern_stream_covers_every_reference_level() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let track = Track::new(
            ResourceRef::new(Platform::Netease, "2709812973").expect("track reference"),
            "live stream level fixture",
        );
        for (quality, level) in [
            (Quality::Standard, "standard"),
            (Quality::Higher, "higher"),
            (Quality::High, "exhigh"),
            (Quality::Lossless, "lossless"),
            (Quality::Hires, "hires"),
            (Quality::Surround, "jyeffect"),
            (Quality::Spatial, "sky"),
            (Quality::Dolby, "dolby"),
            (Quality::Master, "jymaster"),
        ] {
            let batch = MusicProvider::streams(
                &provider,
                std::slice::from_ref(&track),
                &StreamRequest {
                    quality,
                    variant: StreamVariant::Modern,
                    bitrate: None,
                    account: None,
                },
            )
            .await
            .unwrap_or_else(|error| panic!("live {level} stream request failed: {error}"));
            assert_eq!(batch.outcomes.len(), 1, "{level}");
            assert_eq!(batch.outcomes[0].track_ref, track.resource_ref, "{level}");
            assert_eq!(batch.extensions["variant"], "modern", "{level}");
            assert_eq!(batch.extensions["level"], level, "{level}");
            assert_eq!(batch.extensions["response"]["code"], 200, "{level}");
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_modern_stream_batch_preserves_input_order_and_duplicates() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let first = Track::new(
            ResourceRef::new(Platform::Netease, "2709812973").expect("first track reference"),
            "first live batch track",
        );
        let second = Track::new(
            ResourceRef::new(Platform::Netease, "1969519579").expect("second track reference"),
            "second live batch track",
        );
        let batch = MusicProvider::streams(
            &provider,
            &[first.clone(), second.clone(), first.clone()],
            &StreamRequest {
                quality: Quality::High,
                variant: StreamVariant::Modern,
                bitrate: None,
                account: None,
            },
        )
        .await
        .expect("live modern stream batch");
        assert_eq!(batch.outcomes.len(), 3);
        assert_eq!(batch.outcomes[0].track_ref, first.resource_ref);
        assert_eq!(batch.outcomes[1].track_ref, second.resource_ref);
        assert_eq!(batch.outcomes[2].track_ref, first.resource_ref);
        assert_eq!(batch.extensions["response"]["code"], 200);
        assert_eq!(
            batch.extensions["request_path"],
            "/api/song/enhance/player/url/v1"
        );
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_legacy_stream_batch_uses_raw_api_and_bitrate() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let tracks = [
            Track::new(
                ResourceRef::new(Platform::Netease, "2709812973").expect("first track reference"),
                "first legacy stream track",
            ),
            Track::new(
                ResourceRef::new(Platform::Netease, "1969519579").expect("second track reference"),
                "second legacy stream track",
            ),
        ];
        let batch = MusicProvider::streams(
            &provider,
            &tracks,
            &StreamRequest {
                quality: Quality::High,
                variant: StreamVariant::Legacy,
                bitrate: None,
                account: None,
            },
        )
        .await
        .expect("live legacy stream batch");
        assert_eq!(batch.outcomes.len(), 2);
        assert_eq!(batch.outcomes[0].track_ref, tracks[0].resource_ref);
        assert_eq!(batch.outcomes[1].track_ref, tracks[1].resource_ref);
        assert_eq!(batch.extensions["variant"], "legacy");
        assert_eq!(
            batch.extensions["request_path"],
            "/api/song/enhance/player/url"
        );
        assert_eq!(batch.extensions["response"]["code"], 200);
        assert!(!batch.extensions.contains_key("level"));
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_download_urls_cover_legacy_and_every_modern_level() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let track = Track::new(
            ResourceRef::new(Platform::Netease, "2709812973").expect("track reference"),
            "live download track",
        );
        let legacy = MusicProvider::download(
            &provider,
            &track,
            &StreamRequest {
                quality: Quality::Higher,
                variant: StreamVariant::Legacy,
                bitrate: Some(192_123),
                account: None,
            },
        )
        .await
        .expect("live legacy download");
        assert!(legacy.available);
        assert!(
            legacy
                .url
                .as_deref()
                .is_some_and(|url| url.starts_with("http"))
        );
        assert_eq!(legacy.extensions["variant"], "legacy");
        assert_eq!(
            legacy.extensions["request_path"],
            "/api/song/enhance/download/url"
        );
        assert_eq!(legacy.extensions["response"]["code"], 200);

        for (quality, level) in [
            (Quality::Standard, "standard"),
            (Quality::Higher, "higher"),
            (Quality::High, "exhigh"),
            (Quality::Lossless, "lossless"),
            (Quality::Hires, "hires"),
            (Quality::Surround, "jyeffect"),
            (Quality::Spatial, "sky"),
            (Quality::Dolby, "dolby"),
            (Quality::Master, "jymaster"),
        ] {
            let download = MusicProvider::download(
                &provider,
                &track,
                &StreamRequest {
                    quality,
                    variant: StreamVariant::Modern,
                    bitrate: None,
                    account: None,
                },
            )
            .await
            .unwrap_or_else(|error| panic!("live {level} download failed: {error}"));
            assert_eq!(download.requested_quality, quality, "{level}");
            assert_eq!(download.extensions["variant"], "modern", "{level}");
            assert_eq!(download.extensions["requested_level"], level, "{level}");
            assert_eq!(download.extensions["response"]["code"], 200, "{level}");
            assert_eq!(download.available, download.url.is_some(), "{level}");
            if let Some(url) = download.url {
                assert!(url.starts_with("http"), "{level}");
            } else {
                assert_ne!(download.platform_code, Some(200), "{level}");
            }
        }
    }

    #[tokio::test]
    #[ignore = "requires live NetEase access"]
    async fn live_track_availability_covers_playable_and_unavailable_results() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let request = TrackAvailabilityRequest::default();
        let available = MusicProvider::track_availability(&provider, "1969519579", &request)
            .await
            .expect("live playable availability");
        assert!(available.playable);
        assert_eq!(available.platform_code, Some(200));
        assert!(available.actual_bitrate.is_some_and(|bitrate| bitrate > 0));
        assert_eq!(
            available.extensions["response"]["data"][0]["url"],
            Value::Null
        );

        let unavailable = MusicProvider::track_availability(&provider, "1", &request)
            .await
            .expect("live unavailable result");
        assert!(!unavailable.playable);
        assert_eq!(unavailable.platform_code, Some(404));
        assert_eq!(unavailable.actual_bitrate, None);
    }
}
