use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    sync::{Arc, RwLock},
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tuneweave_core::{
    AccountProfile, Album, AlbumListRequest, AlbumStats, AlbumSummary, Artist, ArtistArea,
    ArtistBiographySection, ArtistCategory, ArtistContentCount, ArtistListRequest, ArtistStats,
    ArtistSummary, ArtistTrackListRequest, ArtistTrackOrder, ArtistUpdatesRequest,
    ArtistVideoListRequest, ArtistWorkKind, ArtistWorkUpdate, ArtistWorksRequest,
    AuthChallengeRequest, AuthState, Capability, ChallengeMethod, CreatorSummary, DigitalAlbum,
    DigitalAlbumChartEntry, DigitalAlbumChartKind, DigitalAlbumChartPeriod,
    DigitalAlbumChartRequest, DigitalAlbumListRequest, ErrorCode, Extensions, LyricContributor,
    Lyrics, MediaStream, Money, MusicProvider, Page, PageMeta, PageRequest, ParseResourceRefError,
    PasswordFormat, PasswordLoginRequest, Platform, PlatformApiRequest, PlaybackHistoryEntry,
    PlaybackHistoryPeriod, PlaybackHistoryRequest, Playlist, PrincipalType, ProviderQrPoll,
    ProviderQrStart, Quality, RecommendationRequest, ResourceRef, Result, SearchKind, SearchQuery,
    StreamRequest, SubscriptionResult, Track, TrackEntitlement, TrialWindow, TuneWeaveError, User,
    Video, VideoKind,
};

use crate::{
    NeteaseAccountSummary, NeteaseCaptchaVerification, NeteaseClient, NeteaseConfig,
    NeteaseLoginResult, NeteaseQrCheck, NeteaseQrLogin, NeteaseQrState, NeteaseSessionStatus,
    dto::{
        AlbumDetail, AlbumEntitlementsEnvelope, AlbumEnvelope, AlbumListEnvelope,
        AlbumStatsEnvelope, ArtistAlbumsEnvelope, ArtistDescriptionEnvelope, ArtistDetailEnvelope,
        ArtistDynamicEnvelope, ArtistFanProfile, ArtistFansEnvelope, ArtistFollowCountEnvelope,
        ArtistListEnvelope, ArtistListItem, ArtistMvItem, ArtistMvsEnvelope,
        ArtistNewTracksEnvelope, ArtistNewTracksPlayAllEnvelope, ArtistNewVideoItem,
        ArtistNewVideosEnvelope, ArtistNewWorksEnvelope, ArtistTracksEnvelope, ArtistVideoCreator,
        ArtistVideoRecord, ArtistVideosEnvelope, AudioQuality, DigitalAlbumChartEnvelope,
        DigitalAlbumChartItem, DigitalAlbumEnvelope, DigitalAlbumListEnvelope,
        DigitalAlbumListItem, LikedTracksEnvelope, LyricText, LyricUser, LyricsEnvelope,
        PlayHistoryEnvelope, PlayHistoryRecord, PlaylistDetail, PlaylistEnvelope, Privilege,
        RecommendationReason, RecommendedPlaylistsEnvelope, RecommendedTracksEnvelope,
        SearchEnvelope, Song, StreamData, StreamEnvelope, SubscribedAlbumsEnvelope,
        TrackEntitlementData, TrackEnvelope, UserPlaylistsEnvelope,
    },
};

#[derive(Clone)]
pub struct NeteaseProvider {
    client: NeteaseClient,
    accounts: Arc<RwLock<BTreeMap<String, NeteaseClient>>>,
}

impl NeteaseProvider {
    pub fn new(config: NeteaseConfig) -> Result<Self> {
        Ok(Self {
            client: NeteaseClient::new(config)?,
            accounts: Arc::new(RwLock::new(BTreeMap::new())),
        })
    }

    #[must_use]
    pub fn from_client(client: NeteaseClient) -> Self {
        Self {
            client,
            accounts: Arc::new(RwLock::new(BTreeMap::new())),
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
        client.logout().await?;
        self.remove_session(&account)
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

    fn install_session(&self, account: &str, cookie: String) -> Result<()> {
        let account = normalize_account_label(Some(account))?.to_owned();
        if !crate::client::has_authenticated_cookie(Some(cookie.as_str())) {
            return Err(TuneWeaveError::new(
                ErrorCode::AuthenticationRequired,
                "NetEase session cookie does not contain MUSIC_U",
            )
            .with_platform(Platform::Netease));
        }
        self.accounts
            .write()
            .map_err(|_| account_store_error())?
            .insert(account, self.client.with_cookie(cookie));
        Ok(())
    }

    fn remove_session(&self, account: &str) -> Result<bool> {
        let account = normalize_account_label(Some(account))?;
        let mut accounts = self.accounts.write().map_err(|_| account_store_error())?;
        let removed = accounts.remove(account).is_some();
        if account == "default" {
            let had_default = removed || self.client.is_authenticated();
            accounts.insert(account.to_owned(), self.client.without_cookie());
            return Ok(had_default);
        }
        Ok(removed)
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
            Capability::TrackDetail,
            Capability::AlbumDetail,
            Capability::AlbumList,
            Capability::AlbumStats,
            Capability::AlbumTrackEntitlements,
            Capability::AlbumSubscriptionWrite,
            Capability::DigitalAlbumDetail,
            Capability::DigitalAlbumList,
            Capability::DigitalAlbumCharts,
            Capability::ArtistDetail,
            Capability::ArtistStats,
            Capability::ArtistList,
            Capability::ArtistAlbums,
            Capability::ArtistFans,
            Capability::ArtistVideos,
            Capability::ArtistTracks,
            Capability::PlaylistRead,
            Capability::Lyrics,
            Capability::AudioStream,
            Capability::QrLogin,
            Capability::PasswordLogin,
            Capability::PhoneLogin,
            Capability::SessionManagement,
            Capability::AccountProfile,
            Capability::AccountPlaylists,
            Capability::AccountAlbums,
            Capability::AccountArtistNewVideos,
            Capability::AccountArtistNewTracks,
            Capability::AccountArtistNewWorks,
            Capability::AccountArtistNewTracksPlayAll,
            Capability::Favorites,
            Capability::ListeningHistory,
            Capability::Recommendations,
            Capability::PlatformApi,
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
        map_lyrics(id, response)
    }

    async fn stream(&self, track: &Track, request: &StreamRequest) -> Result<MediaStream> {
        if track.platform != Platform::Netease
            || track.resource_ref.platform() != Platform::Netease
            || track.resource_ref.id() != track.id
        {
            return Err(TuneWeaveError::invalid_request(
                "NetEase provider can only resolve NetEase tracks",
            )
            .with_platform(Platform::Netease)
            .with_details(json!({ "track_ref": track.resource_ref })));
        }
        let id = parse_numeric_id("track", &track.id)?;
        let client = self.client_for(request.account.as_deref())?;
        let response = client
            .request_eapi(
                "/api/song/enhance/player/url",
                json!({
                    "ids": Value::Array(vec![json!(id.to_string())]).to_string(),
                    "br": requested_bitrate(request.quality)
                }),
            )
            .await?;
        ensure_success(&response.body)?;
        let response: StreamEnvelope = parse_body(response.body)?;
        let stream = response
            .data
            .into_iter()
            .find(|stream| stream.id == id)
            .ok_or_else(|| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "NetEase omitted the requested stream result",
                )
                .with_platform(Platform::Netease)
                .with_details(json!({ "id": id }))
            })?;
        map_stream(track, request, stream, client.is_authenticated())
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
            url: login.url,
            image_data_url: None,
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
    let uri = request.uri.trim();
    if uri != request.uri || !uri.starts_with("/api/") || uri.len() == "/api/".len() {
        return Err(TuneWeaveError::invalid_request(
            "NetEase extension API uri must start with /api/ and name an endpoint",
        )
        .with_platform(Platform::Netease));
    }
    if uri.contains(['\r', '\n', '#', '\\']) || uri.contains("://") {
        return Err(TuneWeaveError::invalid_request(
            "NetEase extension API uri contains a forbidden character",
        )
        .with_platform(Platform::Netease));
    }
    let path = uri.split_once('?').map_or(uri, |(path, _)| path);
    if path
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err(TuneWeaveError::invalid_request(
            "NetEase extension API uri cannot contain dot path segments",
        )
        .with_platform(Platform::Netease));
    }
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
    status.account.user_id.or(status.account.id).ok_or_else(|| {
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
    profile.user_id = summary.user_id.or(summary.id);
    profile.nickname = summary.nickname;
    profile.avatar_url = summary.avatar_url;
    profile
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

    Ok(MediaStream {
        url,
        backup_urls: Vec::new(),
        headers: BTreeMap::new(),
        expires_at: stream
            .expi
            .filter(|expires_in_seconds| *expires_in_seconds > 0)
            .and_then(expiration_rfc3339),
        format: stream.kind.clone(),
        codec: stream.encode_type.or(stream.kind),
        bitrate: stream.br,
        size: stream.size,
        duration_ms: stream.time.or(track.duration_ms),
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
        Quality::High => 320_000,
        Quality::Auto | Quality::Lossless | Quality::Hires | Quality::Spatial | Quality::Master => {
            999_000
        }
    }
}

fn stream_quality(level: Option<&str>, bitrate: Option<u64>) -> Quality {
    match level.unwrap_or_default().to_ascii_lowercase().as_str() {
        "standard" => Quality::Standard,
        "higher" | "exhigh" => Quality::High,
        "lossless" => Quality::Lossless,
        "hires" => Quality::Hires,
        "jyeffect" | "sky" | "dolby" => Quality::Spatial,
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
        96_001..=192_000 => Quality::Standard,
        192_001..=500_000 => Quality::High,
        500_001..=1_500_000 => Quality::Lossless,
        1_500_001.. => Quality::Hires,
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

fn map_lyrics(id: u64, lyrics: LyricsEnvelope) -> Result<Lyrics> {
    let track_ref = ResourceRef::new(Platform::Netease, id.to_string()).map_err(|error| {
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
    let format = if plain.is_some() {
        "lrc"
    } else if word_synced.is_some() {
        "yrc"
    } else {
        "plain"
    }
    .to_owned();
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
        .id
        .or(user.userid)
        .or(user.user_id)
        .filter(|id| *id > 0)
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
        mv_count: None,
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
    let creators = if let Some(creator) = item.artist {
        let creator_ref =
            ResourceRef::new(Platform::Netease, creator.id.to_string()).map_err(|error| {
                TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    format!("NetEase returned an invalid MV artist id: {error}"),
                )
                .with_platform(Platform::Netease)
            })?;
        vec![CreatorSummary {
            resource_ref: Some(creator_ref),
            name: creator.name,
            avatar_url: creator.avatar_url,
        }]
    } else {
        item.artist_name
            .filter(|name| !name.trim().is_empty())
            .map(|name| {
                vec![CreatorSummary {
                    resource_ref: None,
                    name,
                    avatar_url: None,
                }]
            })
            .unwrap_or_default()
    };
    let mut extensions = Extensions::new();
    extensions.insert("mv".to_owned(), raw);
    Ok(Video {
        resource_ref,
        platform: Platform::Netease,
        id: item.id.to_string(),
        title: item.name,
        creators,
        description: String::new(),
        cover_url: item.image_16x9_url.or(item.imgurl),
        duration_ms: item.duration,
        published_at: item.published_at,
        play_count: item.play_count,
        subscribed: item.subed,
        extensions,
    })
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
        .id
        .as_ref()
        .and_then(json_scalar_string)
        .or_else(|| item.resource.base.id.as_ref().and_then(json_scalar_string))
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
        title: base
            .text
            .filter(|title| !title.trim().is_empty())
            .or(base.original_title)
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
    let next_before_ms = response
        .data
        .new_works
        .last()
        .and_then(artist_update_timestamp);
    let items = response
        .data
        .new_works
        .into_iter()
        .map(|raw| {
            let song: Song = parse_body(raw.clone())?;
            let mut track = map_song(song, None)?;
            track.extensions.insert("artist_new_track".to_owned(), raw);
            Ok(track)
        })
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
            total: response.data.new_song_count,
            next_offset: None,
            has_more,
            extensions,
        },
    })
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
    let kind = if !tracks.is_empty() || block_type.contains("song") || block_type.contains("track")
    {
        ArtistWorkKind::Track
    } else if !videos.is_empty() || block_type.contains("mv") || block_type.contains("video") {
        ArtistWorkKind::Video
    } else {
        ArtistWorkKind::Unknown
    };
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
    let cover_url = ["resourcePicUrl", "imgUrl"]
        .into_iter()
        .find_map(|key| block_title[key].as_str())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
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
    keys.iter().find_map(|key| info.get(*key)?.as_array())
}

fn map_artist_new_video(raw: Value) -> Result<Video> {
    let item: ArtistNewVideoItem = parse_body(raw.clone())?;
    let id = item
        .id
        .as_ref()
        .and_then(json_scalar_string)
        .or_else(|| item.mv_id.as_ref().and_then(json_scalar_string))
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
    let published_at = item
        .published_date
        .filter(|published_at| !published_at.trim().is_empty())
        .or_else(|| item.published_at.as_ref().and_then(netease_published_at));
    let mut extensions = Extensions::new();
    extensions.insert("artist_new_video".to_owned(), raw);
    Ok(Video {
        resource_ref,
        platform: Platform::Netease,
        id,
        title: item
            .name
            .filter(|title| !title.trim().is_empty())
            .or(item.mv_name)
            .unwrap_or_default(),
        creators,
        description: item.desc.or(item.brief_description).unwrap_or_default(),
        cover_url: item.cover.or(item.mv_cover_url),
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
        max_play_bitrate: entitlement.play_max_bitrate.or(entitlement.maxbr),
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
    let artist_name = album.artist_name.or(album.artist_names);
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

fn capability_for_search(kind: SearchKind) -> Capability {
    match kind {
        SearchKind::Track => Capability::SearchTracks,
        SearchKind::Album => Capability::SearchAlbums,
        SearchKind::Artist => Capability::SearchArtists,
        SearchKind::Playlist => Capability::SearchPlaylists,
        SearchKind::Video => Capability::SearchVideos,
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
    let message = body["message"]
        .as_str()
        .or_else(|| body["msg"].as_str())
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

    #[test]
    fn maps_netease_song_to_unified_track() {
        let track = map_song(fixture_song(), None).expect("map song");
        assert_eq!(track.resource_ref.to_string(), "netease:123");
        assert_eq!(track.name, "反方向的钟");
        assert_eq!(track.artists[0].name, "周杰伦");
        assert_eq!(track.duration_ms, Some(258000));
        assert_eq!(
            track.available_qualities,
            vec![Quality::Standard, Quality::High, Quality::Lossless]
        );
        assert_eq!(track.playable, Some(true));
        assert_eq!(track.extensions["fee"], 1);
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
            vec![Quality::Standard, Quality::High, Quality::Lossless]
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
    fn maps_netease_artist_videos_and_cursor_to_the_unified_video_model() {
        let response: ArtistVideosEnvelope = serde_json::from_value(json!({
            "data": {
                "page": { "cursor": "2", "more": true, "size": 1 },
                "records": [
                    {
                        "id": "22695250",
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
    fn maps_followed_artist_new_videos_and_timestamp_cursor() {
        let raw = json!({
            "code": 200,
            "data": {
                "hasMore": true,
                "newWorks": [
                    {
                        "id": 1099001,
                        "name": "新 MV",
                        "cover": "https://example.test/new-mv.jpg",
                        "playCount": 3456,
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
                        "id": 2099001,
                        "name": "新歌",
                        "alias": ["New Song"],
                        "artists": [{ "id": 6452, "name": "周杰伦" }],
                        "album": {
                            "id": 3099001,
                            "name": "新专辑",
                            "picUrl": "https://example.test/new-album.jpg"
                        },
                        "duration": 208000,
                        "mvid": 1099001,
                        "publishTime": 1_720_000_000_000_u64
                    }
                ]
            }
        });
        let response: ArtistNewTracksEnvelope =
            serde_json::from_value(raw.clone()).expect("new artist tracks fixture");

        let page = map_artist_new_tracks_response(response, raw, 1, 1_730_000_000_000)
            .expect("map followed artist tracks");

        assert_eq!(page.items[0].resource_ref.to_string(), "netease:2099001");
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
        assert_eq!(
            page.items[0].extensions["artist_new_track"]["publishTime"],
            1_720_000_000_000_u64
        );
        assert_eq!(page.pagination.total, Some(3));
        assert_eq!(
            page.pagination.extensions["next_before_ms"],
            1_720_000_000_000_u64
        );
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
            "playMaxbr": 999000,
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
        assert_eq!(
            entitlement.available_qualities,
            vec![
                Quality::Standard,
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
    fn maps_netease_digital_album_product_to_the_unified_model() {
        let raw = json!({
            "code": 200,
            "album": {
                "albumId": 120605500,
                "albumName": "冀西南林路行",
                "artistId": 13223,
                "artistName": "万能青年旅店",
                "artistNames": "万能青年旅店",
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
        assert_eq!(album.artists[0].name, "万能青年旅店");
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
    async fn album_ids_are_validated_before_network_access() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let detail_error = MusicProvider::album(&provider, "invalid", None)
            .await
            .expect_err("invalid album id");
        assert_eq!(detail_error.code, ErrorCode::InvalidRequest);
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
            "transUser": {"userId": 11, "nickname": "翻译贡献者"},
            "pureMusic": false
        }))
        .expect("valid lyrics fixture");

        let lyrics = map_lyrics(185809, lyrics).expect("map lyrics");
        assert_eq!(lyrics.track_ref.to_string(), "netease:185809");
        assert_eq!(lyrics.format, "lrc");
        assert!(lyrics.plain.is_some_and(|lyrics| lyrics.contains("青花")));
        assert!(lyrics.word_synced.is_some());
        assert_eq!(lyrics.contributors.len(), 2);
        assert_eq!(lyrics.contributors[1].role, "translation");
        assert_eq!(lyrics.extensions["word_synced_version"], 7);
    }

    #[test]
    fn maps_netease_stream_quality_expiry_and_trial() {
        let track = Track::new(
            ResourceRef::new(Platform::Netease, "123").expect("track reference"),
            "测试歌曲",
        );
        let request = StreamRequest {
            quality: Quality::High,
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
            "encodeType": "mp3",
            "time": 258000,
            "fee": 1,
            "freeTrialInfo": {"start": 0, "end": 30}
        }))
        .expect("valid stream fixture");

        let stream = map_stream(&track, &request, stream, false).expect("map stream");
        assert_eq!(stream.requested_quality, Quality::High);
        assert_eq!(stream.actual_quality, Quality::High);
        assert_eq!(stream.bitrate, Some(320000));
        assert_eq!(stream.trial.expect("trial").end_ms, 30_000);
        assert!(
            stream
                .expires_at
                .is_some_and(|expires| expires.ends_with('Z'))
        );
    }

    #[test]
    fn reports_missing_paid_stream_as_authentication_required() {
        let track = Track::new(
            ResourceRef::new(Platform::Netease, "123").expect("track reference"),
            "测试歌曲",
        );
        let request = StreamRequest {
            quality: Quality::Lossless,
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
        assert!(capabilities.contains(&Capability::SessionManagement));
        assert!(capabilities.contains(&Capability::AccountProfile));
        assert!(capabilities.contains(&Capability::AccountPlaylists));
        assert!(capabilities.contains(&Capability::AccountAlbums));
        assert!(capabilities.contains(&Capability::AccountArtistNewVideos));
        assert!(capabilities.contains(&Capability::AccountArtistNewTracks));
        assert!(capabilities.contains(&Capability::AccountArtistNewWorks));
        assert!(capabilities.contains(&Capability::AccountArtistNewTracksPlayAll));
        assert!(capabilities.contains(&Capability::Favorites));
        assert!(capabilities.contains(&Capability::ListeningHistory));
        assert!(capabilities.contains(&Capability::Recommendations));
        assert!(capabilities.contains(&Capability::AlbumDetail));
        assert!(capabilities.contains(&Capability::AlbumList));
        assert!(capabilities.contains(&Capability::AlbumStats));
        assert!(capabilities.contains(&Capability::AlbumTrackEntitlements));
        assert!(capabilities.contains(&Capability::AlbumSubscriptionWrite));
        assert!(capabilities.contains(&Capability::DigitalAlbumDetail));
        assert!(capabilities.contains(&Capability::DigitalAlbumList));
        assert!(capabilities.contains(&Capability::DigitalAlbumCharts));
        assert!(capabilities.contains(&Capability::PlatformApi));
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
        let error = ensure_platform_api_success(&json!({ "code": 401, "msg": "login" }))
            .expect_err("authentication failure");
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
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
    async fn live_account_albums_require_authentication_without_a_session() {
        let provider = NeteaseProvider::new(NeteaseConfig::default()).expect("build provider");
        let error = MusicProvider::account_albums(&provider, &PageRequest::new(2, 0))
            .await
            .expect_err("anonymous subscribed albums must fail");
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
        assert!(lyrics.plain.is_some() || lyrics.word_synced.is_some());
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
                    account: None,
                },
            )
            .await
            .expect("live track stream");
        assert!(stream.url.starts_with("http"));
        assert_eq!(stream.resolved_track.to_string(), "netease:2709812973");
        assert!(stream.bitrate.is_some_and(|bitrate| bitrate > 0));
    }
}
