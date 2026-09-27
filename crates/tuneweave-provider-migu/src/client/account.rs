use super::albums::AlbumKind;
use super::playlist_collection::Kind as CollectionKind;
use super::*;
use crate::credential::{MiguCredential, authentication_required, validate_token, validate_uid};
use reqwest::header::{CONTENT_TYPE, HeaderMap, REFERER, SET_COOKIE};
use tuneweave_core::AccountProfile;

const HOST: &str = "c.musicapp.migu.cn";
const MAX_RESPONSE: u64 = 65_536;

#[derive(Clone, Copy)]
enum Operation<'a> {
    Profile,
    H5Token(&'a str),
    Check,
    Logout,
    Membership,
    MemberIcons,
    MediaIdentities,
    Library(super::library::Section, u32),
    Home,
    Playlist(&'a str),
    PlaylistTracks(&'a str, u32),
    SetFavorite(&'a str, bool),
    FavoriteState(&'a str),
    SetCollection(CollectionKind, &'a str, &'a str, bool),
    CollectionState(CollectionKind, &'a str),
    AlbumCollections(u32),
    AlbumCollectionMetadata(AlbumKind, &'a str),
    PlaylistWrite(super::playlist_write::Write<'a>),
    PurchasedTracks,
    PurchasedAlbums(u32, u64),
}
impl Operation<'_> {
    fn path(self) -> &'static str {
        match self {
            Self::Profile => "/user/h5/user-info/v1.0",
            Self::H5Token(_) => "/user/h5/token/v1.0",
            Self::Check => "/mgateway/api/checkPacMtoken",
            Self::Logout => "/mgateway/api/clearPacMtoken",
            Self::Membership => "/user/member/center/v3.0",
            Self::MemberIcons => "/pc/open/api/member/icon/v1.0",
            Self::MediaIdentities => "/user/i/member/identity/v1.0",
            Self::Library(section, _) => section.path(),
            Self::Home => "/pc/user/home-page/v2.0",
            Self::Playlist(_) => "/resource/playlist/v2.0",
            Self::PlaylistTracks(..) => "/MIGUM3.0/resource/playlist/song/v2.0",
            Self::SetFavorite(_, true) => "/pc/user/api/add-music-list-song/v1.0",
            Self::SetFavorite(_, false) => "/pc/user/h5-import-musiclist/v1.0",
            Self::FavoriteState(_) => "/pc/v1.0/content/inMusicLists.do",
            Self::SetCollection(_, _, _, true) => "/pc/v1.0/user/add_collection.do",
            Self::SetCollection(_, _, _, false) => "/pc/v1.0/user/del_collection.do",
            Self::CollectionState(kind, _) => kind.state_path(),
            Self::AlbumCollections(_) => "/pc/v1.0/user/collections.do",
            Self::AlbumCollectionMetadata(kind, _) => kind.account_detail_path(),
            Self::PlaylistWrite(write) => write.path(),
            Self::PurchasedTracks => "/MIGUM3.0/strategy/song-ordered/v2.0",
            Self::PurchasedAlbums(..) => "/strategy/album-subscription/list/v1.0",
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Profile => "account_profile",
            Self::H5Token(_) => "account_h5_token",
            Self::Check => "session_check",
            Self::Logout => "session_logout",
            Self::Membership => "account_membership",
            Self::MemberIcons => "account_member_icons",
            Self::MediaIdentities => "account_media_identities",
            Self::Library(section, _) => section.backend(),
            Self::Home => "account_favorite_identity",
            Self::Playlist(_) => "account_playlist_detail",
            Self::PlaylistTracks(..) => "account_playlist_tracks",
            Self::SetFavorite(_, true) => "account_track_like",
            Self::SetFavorite(_, false) => "account_track_unlike",
            Self::FavoriteState(_) => "account_track_favorite_state",
            Self::SetCollection(CollectionKind::Playlist, _, _, true) => "account_playlist_collect",
            Self::SetCollection(CollectionKind::Album(_), _, _, true) => "account_album_collect",
            Self::SetCollection(CollectionKind::Playlist, _, _, false) => {
                "account_playlist_uncollect"
            }
            Self::SetCollection(CollectionKind::Album(_), _, _, false) => "account_album_uncollect",
            Self::CollectionState(CollectionKind::Playlist, _) => {
                "account_playlist_collection_state"
            }
            Self::CollectionState(CollectionKind::Album(_), _) => "account_album_collection_state",
            Self::AlbumCollections(_) => "account_album_collections",
            Self::AlbumCollectionMetadata(_, _) => "account_collection_album_title",
            Self::PlaylistWrite(write) => write.name(),
            Self::PurchasedTracks => "account_purchased_track_ids",
            Self::PurchasedAlbums(..) => "account_purchased_albums",
        }
    }
    fn host(self) -> &'static str {
        match self {
            Self::Membership
            | Self::MemberIcons
            | Self::MediaIdentities
            | Self::Library(..)
            | Self::Home
            | Self::Playlist(_)
            | Self::PlaylistTracks(..)
            | Self::SetFavorite(..)
            | Self::FavoriteState(_)
            | Self::SetCollection(..)
            | Self::CollectionState(..)
            | Self::AlbumCollections(_)
            | Self::AlbumCollectionMetadata(..)
            | Self::PlaylistWrite(_)
            | Self::PurchasedTracks
            | Self::PurchasedAlbums(..) => "app.c.nf.migu.cn",
            _ => HOST,
        }
    }
}

#[derive(Deserialize)]
struct Envelope {
    code: String,
    data: Option<serde_json::Value>,
    #[serde(flatten)]
    fields: serde_json::Map<String, serde_json::Value>,
}
fn parse_envelope(operation: Operation<'_>, body: &[u8]) -> Result<Envelope> {
    let envelope: Envelope = serde_json::from_slice(body)
        .map_err(|_| migu_upstream_error("Migu session response is invalid"))?;
    if envelope.code != "000000" {
        let auth = match operation {
            Operation::Profile
            | Operation::H5Token(_)
            | Operation::Membership
            | Operation::MemberIcons
            | Operation::MediaIdentities
            | Operation::Library(..)
            | Operation::Home
            | Operation::Playlist(_)
            | Operation::PlaylistTracks(..)
            | Operation::SetFavorite(..)
            | Operation::FavoriteState(_)
            | Operation::SetCollection(..)
            | Operation::CollectionState(..)
            | Operation::AlbumCollections(_)
            | Operation::AlbumCollectionMetadata(..)
            | Operation::PlaylistWrite(_)
            | Operation::PurchasedTracks
            | Operation::PurchasedAlbums(..) => envelope.code == "290001",
            Operation::Check | Operation::Logout => envelope.code == "850001",
        };
        if auth {
            return Err(authentication_required());
        }
        let mut e = migu_upstream_error("Migu session business request failed");
        if envelope.code.len() == 6 && envelope.code.bytes().all(|b| b.is_ascii_digit()) {
            e = e.with_details(json!({"platform_code":envelope.code}));
        }
        return Err(e);
    }
    Ok(envelope)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Profile {
    user_id: String,
    nick_name: Option<String>,
    small_icon: Option<String>,
}

pub(crate) struct AccountRead {
    pub user_id: String,
    pub token: String,
    pub profile: Result<AccountProfile>,
    pub native_session: Option<String>,
}
impl std::fmt::Debug for AccountRead {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountRead").finish_non_exhaustive()
    }
}

pub(crate) fn require_native_exchange_session(session: Option<String>) -> Result<String> {
    session
        .ok_or_else(|| migu_upstream_error("Migu profile omitted a valid native exchange session"))
}

trait SessionResponse {
    fn conversion_error(&self) -> Option<&TuneWeaveError> {
        None
    }
}
impl SessionResponse for String {}
impl SessionResponse for () {}
impl SessionResponse for AccountRead {
    fn conversion_error(&self) -> Option<&TuneWeaveError> {
        self.profile.as_ref().err()
    }
}

pub(crate) struct AccountData<T> {
    pub token: String,
    pub data: Result<T>,
}
impl<T> std::fmt::Debug for AccountData<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AccountData").finish_non_exhaustive()
    }
}
impl<T> SessionResponse for AccountData<T> {
    fn conversion_error(&self) -> Option<&TuneWeaveError> {
        self.data.as_ref().err()
    }
}

impl MiguClient {
    #[cfg(test)]
    pub(crate) fn with_session_test_timeout(mut self, timeout: Duration) -> Self {
        self.http = Client::builder()
            .retry(reqwest::retry::never())
            .no_proxy()
            .redirect(Policy::none())
            .timeout(timeout)
            .build()
            .unwrap();
        self
    }

    async fn session_request<T: SessionResponse>(
        &self,
        operation: Operation<'_>,
        token: &str,
        parse: impl FnOnce(Envelope, &HeaderMap) -> Result<T>,
    ) -> Result<T> {
        self.raw_session_request(operation, token, |value, headers| {
            parse(parse_envelope(operation, value)?, headers)
        })
        .await
    }

    async fn raw_session_request<T: SessionResponse>(
        &self,
        operation: Operation<'_>,
        token: &str,
        parse: impl FnOnce(&[u8], &HeaderMap) -> Result<T>,
    ) -> Result<T> {
        validate_token(token)?;
        let mut auth_header = reqwest::header::HeaderValue::from_str(token)
            .map_err(|_| migu_invalid_request("Migu session token cannot form a header"))?;
        auth_header.set_sensitive(true);
        let started = Instant::now();
        let mut http_status = None;
        let outcome = async {
            let url = Url::parse(&format!("https://{}{}", operation.host(), operation.path()))
                .map_err(|_| migu_upstream_error("Invalid Migu session endpoint"))?;
            #[cfg(test)]
            let url = self
                .catalog_test_origin
                .as_ref()
                .map_or(Ok(url), |origin| origin.join(operation.path()))
                .map_err(|_| migu_upstream_error("Invalid Migu session test endpoint"))?;
            let (channel, referer) = if matches!(operation, Operation::Library(..)) {
                ("014021I", "https://m.music.migu.cn/")
            } else if matches!(
                operation,
                Operation::Membership | Operation::MediaIdentities
            ) {
                ("014021I", "https://h5.nf.migu.cn/")
            } else {
                ("014X031", "https://music.migu.cn/")
            };
            let mut request = self
                .http
                .request(
                    if matches!(operation, Operation::SetFavorite(..))
                        || matches!(operation, Operation::PlaylistWrite(write) if write.is_post())
                    {
                        reqwest::Method::POST
                    } else {
                        reqwest::Method::GET
                    },
                    url,
                )
                .header(ACCEPT, "application/json")
                .header(REFERER, referer);
            if matches!(
                operation,
                Operation::PurchasedTracks | Operation::PlaylistWrite(_)
            ) {
                // These official account endpoints use the PACM cookie transport.
                // The native client's global token is a different credential.
                let mut cookie =
                    reqwest::header::HeaderValue::from_str(&format!("pacmtoken={token}"))
                        .map_err(|_| migu_invalid_request("Invalid Migu account cookie"))?;
                cookie.set_sensitive(true);
                request = request.header(reqwest::header::COOKIE, cookie);
                if matches!(operation, Operation::PurchasedTracks) {
                    request = request.header("channel", "014X031");
                }
            } else {
                request = request.header("pacmtoken", auth_header);
            }
            if matches!(
                operation,
                Operation::Membership
                    | Operation::MemberIcons
                    | Operation::MediaIdentities
                    | Operation::Library(..)
                    | Operation::Home
                    | Operation::Playlist(_)
                    | Operation::PlaylistTracks(..)
                    | Operation::SetFavorite(..)
                    | Operation::FavoriteState(_)
                    | Operation::SetCollection(..)
                    | Operation::CollectionState(..)
                    | Operation::AlbumCollections(_)
                    | Operation::AlbumCollectionMetadata(..)
                    | Operation::PlaylistWrite(_)
                    | Operation::PurchasedAlbums(..)
            ) {
                request = request
                    .header("platform", "H5")
                    .header("channel", channel)
                    .header("subchannel", channel)
                    .header("deviceId", self.music_device.identity()?);
            }
            if let Operation::Library(section, page) = operation {
                request = request.query(&section.query(page));
            }
            match operation {
                Operation::H5Token(session) => {
                    let timestamp = super::native_http::timestamp()?;
                    request = request.query(&[("uSessionId", session), ("_t", &timestamp)]);
                }
                Operation::Playlist(id) => {
                    request = request.query(&[("playlistId", id)]);
                }
                Operation::PlaylistTracks(id, page) => {
                    request = request.query(&MiguPlaylistTracksQuery {
                        playlist_id: id,
                        page_no: page,
                        page_size: super::account_playlist::PAGE_SIZE,
                    });
                }
                Operation::SetFavorite(id, subscribed) => {
                    request = request.json(&if subscribed {
                        json!({"contentIds":[id]})
                    } else {
                        json!({"channel":"23","contentId":id,"songflag":"2"})
                    });
                }
                Operation::FavoriteState(id) => {
                    request = request.query(&[("type", "1"), ("contentId", id)]);
                }
                Operation::SetCollection(kind, id, title, true) => {
                    request = request.query(&[
                        ("outOPType", "03"),
                        ("outResourceName", title),
                        ("outResourceId", id),
                        ("outResourceType", kind.resource_type()),
                    ]);
                }
                Operation::SetCollection(kind, id, _, false) => {
                    request = request.query(&[
                        ("oPType", "03"),
                        ("resourceId", id),
                        ("resourceType", kind.resource_type()),
                    ]);
                }
                Operation::CollectionState(_, id) => {
                    request = request.query(&[("opType", "03"), ("resourceId", id)]);
                }
                Operation::AlbumCollections(page) => {
                    request = request.query(&[
                        ("pageNo", page.to_string()),
                        ("pageSize", super::album_collections::PAGE_SIZE.to_string()),
                        ("type", "1".into()),
                        ("oPType", "03".into()),
                        ("resourceType", "2003|5".into()),
                    ]);
                }
                Operation::AlbumCollectionMetadata(kind, id) => {
                    request = request.query(&[(kind.parameter(), id)]);
                }
                Operation::PlaylistWrite(write) => request = write.request(request),
                Operation::PurchasedAlbums(page, _) => {
                    request = request
                        .query(&[
                            ("pageNumber", page),
                            ("pageSize", super::purchased_albums::PAGE_SIZE as u32),
                        ])
                        .header("version", "6.8.8")
                        .header("ua", "Android_migu");
                }
                _ => {}
            }
            let response = request.send().await.map_err(|e| {
                crate::credential::error(
                    if e.is_timeout() {
                        ErrorCode::UpstreamTimeout
                    } else {
                        ErrorCode::UpstreamError
                    },
                    "Migu session request failed",
                )
            })?;
            let status = response.status();
            http_status = Some(status);
            if matches!(status, StatusCode::UNAUTHORIZED) {
                return Err(authentication_required());
            }
            if status == StatusCode::FORBIDDEN {
                return Err(crate::credential::error(
                    ErrorCode::PermissionDenied,
                    "Migu session request was denied",
                ));
            }
            if status == StatusCode::TOO_MANY_REQUESTS {
                return Err(crate::credential::error(
                    ErrorCode::RateLimited,
                    "Migu session request was rate limited",
                ));
            }
            if !status.is_success() {
                return Err(migu_upstream_error("Migu session HTTP request failed"));
            }
            let mime = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .map(str::trim);
            if !mime.is_some_and(|v| v.eq_ignore_ascii_case("application/json")) {
                return Err(migu_upstream_error("Migu session response is not JSON"));
            }
            let headers = response.headers().clone();
            let maximum = if let Operation::PurchasedAlbums(_, maximum) = operation {
                maximum
            } else if matches!(
                operation,
                Operation::PlaylistTracks(..) | Operation::PurchasedTracks
            ) {
                1_048_576
            } else if matches!(
                operation,
                Operation::Membership
                    | Operation::Library(..)
                    | Operation::Home
                    | Operation::Playlist(_)
                    | Operation::AlbumCollections(_)
                    | Operation::AlbumCollectionMetadata(..)
            ) {
                262_144
            } else {
                MAX_RESPONSE
            };
            let body = read_session_body_limited(response, maximum).await?;
            parse(&body, &headers)
        }
        .await;
        let log_outcome: Result<()> = match outcome.as_ref().err().or_else(|| {
            outcome
                .as_ref()
                .ok()
                .and_then(SessionResponse::conversion_error)
        }) {
            Some(error) => Err(crate::credential::error(
                error.code,
                "Migu session operation failed",
            )
            .retryable(error.retryable)
            .with_details(json!({"platform_code":error.details.get("platform_code")}))),
            None => Ok(()),
        };
        self.log_upstream_request(
            operation.name(),
            operation.host(),
            operation.path(),
            http_status,
            started,
            &log_outcome,
        );
        outcome
    }

    pub(crate) async fn check_pacm(&self, token: &str) -> Result<String> {
        self.session_request(Operation::Check, token, |_, headers| {
            Ok(
                rotated_token(headers, Operation::Check.path())?
                    .unwrap_or_else(|| token.to_owned()),
            )
        })
        .await
    }

    async fn account_playlist_read<T>(
        &self,
        operation: Operation<'_>,
        token: &str,
        uid: &str,
        parse: impl FnOnce(serde_json::Value) -> Result<T>,
    ) -> Result<AccountData<T>> {
        self.account_data_read_sized(operation, token, uid, |data, _| parse(data))
            .await
    }

    async fn account_data_read_sized<T>(
        &self,
        operation: Operation<'_>,
        token: &str,
        uid: &str,
        parse: impl FnOnce(serde_json::Value, usize) -> Result<T>,
    ) -> Result<AccountData<T>> {
        self.raw_session_request(operation, token, |bytes, headers| {
            let body = parse_envelope(operation, bytes)?;
            let data = body
                .data
                .ok_or_else(|| migu_upstream_error("Migu account playlist omitted its data"))?;
            for id in [body.fields.get("userId"), data.get("userId")]
                .into_iter()
                .flatten()
            {
                if id.as_str() != Some(uid) {
                    return Err(authentication_required());
                }
            }
            Ok(AccountData {
                token: rotated_token_for_host(headers, operation.host(), operation.path())?
                    .unwrap_or_else(|| token.to_owned()),
                data: parse(data, bytes.len()),
            })
        })
        .await
    }

    pub(crate) async fn account_purchased_albums_page(
        &self,
        token: &str,
        uid: &str,
        page: u32,
        remaining: u64,
    ) -> Result<AccountData<super::purchased_albums::AlbumPage>> {
        use super::purchased_albums::{MAX_PAGES, parse_page};
        validate_uid(uid)?;
        if !(1..=MAX_PAGES).contains(&page) || remaining == 0 {
            return Err(migu_invalid_request(
                "Migu purchased album read budget is invalid",
            ));
        }
        self.account_data_read_sized(
            Operation::PurchasedAlbums(page, remaining.min(1_048_576)),
            token,
            uid,
            |value, bytes| parse_page(value, uid, bytes),
        )
        .await
    }

    pub(crate) async fn account_purchased_track_ids(
        &self,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<Vec<String>>> {
        validate_uid(uid)?;
        self.account_playlist_read(
            Operation::PurchasedTracks,
            token,
            uid,
            super::purchases::parse_ids,
        )
        .await
    }

    async fn account_favorite_operation<T>(
        &self,
        operation: Operation<'_>,
        token: &str,
        uid: &str,
        parse: impl FnOnce(serde_json::Map<String, serde_json::Value>) -> Result<T>,
    ) -> Result<AccountData<T>> {
        self.session_request(operation, token, |body, headers| {
            for id in [
                body.fields.get("userId"),
                body.data.as_ref().and_then(|data| data.get("userId")),
            ]
            .into_iter()
            .flatten()
            {
                if id.as_str() != Some(uid) {
                    return Err(authentication_required());
                }
            }
            Ok(AccountData {
                token: rotated_token_for_host(headers, operation.host(), operation.path())?
                    .unwrap_or_else(|| token.to_owned()),
                data: parse(body.fields),
            })
        })
        .await
    }

    pub(crate) async fn write_account_playlist(
        &self,
        write: super::playlist_write::Write<'_>,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<Option<String>>> {
        self.account_favorite_operation(
            Operation::PlaylistWrite(write),
            token,
            uid,
            super::playlist_write::acknowledged_id,
        )
        .await
    }

    pub(crate) async fn set_account_favorite(
        &self,
        id: &str,
        subscribed: bool,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<()>> {
        // Business success is only an acknowledgement. The provider confirms both
        // the complete favorite collection and the explicit single-track state.
        self.account_favorite_operation(Operation::SetFavorite(id, subscribed), token, uid, |_| {
            Ok(())
        })
        .await
    }

    pub(crate) async fn account_favorite_state(
        &self,
        id: &str,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<bool>> {
        self.account_favorite_operation(Operation::FavoriteState(id), token, uid, |fields| {
            super::favorites::state(fields, id)
        })
        .await
    }

    pub(crate) async fn set_account_playlist_collection(
        &self,
        id: &str,
        title: &str,
        subscribed: bool,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<()>> {
        self.account_favorite_operation(
            Operation::SetCollection(CollectionKind::Playlist, id, title, subscribed),
            token,
            uid,
            |_| Ok(()),
        )
        .await
    }

    pub(crate) async fn account_playlist_collection_state(
        &self,
        id: &str,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<bool>> {
        self.collection_state(CollectionKind::Playlist, id, token, uid)
            .await
    }

    async fn collection_state(
        &self,
        kind: CollectionKind,
        id: &str,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<bool>> {
        let operation = Operation::CollectionState(kind, id);
        self.raw_session_request(operation, token, |body, headers| {
            let value: serde_json::Value = serde_json::from_slice(body)
                .map_err(|_| migu_upstream_error("Migu collection state response is invalid"))?;
            let rows = match value {
                serde_json::Value::Array(rows) => rows,
                _ => {
                    // Error responses still use the common code/info object. Do
                    // not synthesize a success code for an unexpected envelope.
                    parse_envelope(operation, body)?;
                    return Err(migu_upstream_error(
                        "Migu collection state did not return an operation array",
                    ));
                }
            };
            for row in &rows {
                if row
                    .get("userId")
                    .is_some_and(|value| value.as_str() != Some(uid))
                {
                    return Err(authentication_required());
                }
            }
            Ok(AccountData {
                token: rotated_token_for_host(headers, operation.host(), operation.path())?
                    .unwrap_or_else(|| token.to_owned()),
                data: super::playlist_collection::state(rows, id, kind),
            })
        })
        .await
    }

    pub(crate) async fn account_album_collections(
        &self,
        page: u32,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<super::album_collections::CollectionPage>> {
        self.account_favorite_operation(Operation::AlbumCollections(page), token, uid, |fields| {
            super::album_collections::parse_page(fields, uid)
        })
        .await
    }
    pub(crate) async fn account_collection_album_title(
        &self,
        kind: AlbumKind,
        id: &str,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<String>> {
        self.account_playlist_read(
            Operation::AlbumCollectionMetadata(kind, id),
            token,
            uid,
            |data| super::album_collections::title(kind, data, id),
        )
        .await
    }
    pub(crate) async fn set_account_album_collection(
        &self,
        kind: AlbumKind,
        id: &str,
        title: &str,
        subscribed: bool,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<()>> {
        self.account_favorite_operation(
            Operation::SetCollection(CollectionKind::Album(kind), id, title, subscribed),
            token,
            uid,
            |_| Ok(()),
        )
        .await
    }
    pub(crate) async fn account_album_collection_state(
        &self,
        kind: AlbumKind,
        id: &str,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<bool>> {
        self.collection_state(CollectionKind::Album(kind), id, token, uid)
            .await
    }

    pub(crate) async fn account_favorite_id_for_write(
        &self,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<Option<String>>> {
        self.account_playlist_read(
            Operation::Home,
            token,
            uid,
            super::account_playlist::favorite_id_for_write,
        )
        .await
    }

    pub(crate) async fn account_favorite_id(
        &self,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<String>> {
        self.account_playlist_read(
            Operation::Home,
            token,
            uid,
            super::account_playlist::favorite_id,
        )
        .await
    }

    pub(crate) async fn account_playlist_detail(
        &self,
        id: &str,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<Playlist>> {
        self.account_playlist_read(Operation::Playlist(id), token, uid, |data| {
            super::account_playlist::detail(data, id)
        })
        .await
    }

    pub(crate) async fn account_playlist_tracks(
        &self,
        id: &str,
        page: u32,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<super::account_playlist::TrackPage>> {
        if !(1..=super::account_playlist::MAX_PAGES).contains(&page) {
            return Err(migu_invalid_request(
                "Migu account playlist page exceeds its bounds",
            ));
        }
        self.account_playlist_read(Operation::PlaylistTracks(id, page), token, uid, |data| {
            super::account_playlist::tracks(data, id, page)
        })
        .await
    }

    pub(crate) async fn account_library_page(
        &self,
        section: super::library::Section,
        page: u32,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<super::library::LibraryPage>> {
        if !(1..=super::library::MAX_PAGES).contains(&page) {
            return Err(migu_invalid_request("Migu library page exceeds its bounds"));
        }
        let operation = Operation::Library(section, page);
        self.session_request(operation, token, |body, headers| {
            // These legacy endpoints put list/collections at the response root.
            // SDK originData is the original body, not a nested server field.
            for id in [
                body.fields.get("userId"),
                body.data.as_ref().and_then(|v| v.get("userId")),
            ]
            .into_iter()
            .flatten()
            {
                if id.as_str() != Some(uid) {
                    return Err(authentication_required());
                }
            }
            Ok(AccountData {
                token: rotated_token_for_host(headers, operation.host(), operation.path())?
                    .unwrap_or_else(|| token.to_owned()),
                data: super::library::parse_page(section, body.fields, uid),
            })
        })
        .await
    }

    pub(crate) async fn account_profile(
        &self,
        account: &str,
        token: &str,
        expected_uid: Option<&str>,
    ) -> Result<AccountRead> {
        self.session_request(Operation::Profile, token, |body, headers| {
            let data = body
                .data
                .ok_or_else(|| migu_upstream_error("Migu profile omitted its data"))?;
            let user_id = data
                .get("userId")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| migu_upstream_error("Migu profile omitted a valid user identity"))?
                .to_owned();
            validate_uid(&user_id)
                .map_err(|_| migu_upstream_error("Migu profile omitted a valid user identity"))?;
            if expected_uid.is_some_and(|id| id != user_id) {
                return Err(authentication_required());
            }
            // Identity and token acceptance precede optional profile presentation fields.
            let token = rotated_token(headers, Operation::Profile.path())?
                .unwrap_or_else(|| token.to_owned());
            // The official H5 profile spells this field with a lowercase s.
            // Some valid profiles omit it, which only limits native-only reads.
            let native_session = data
                .get("usessionId")
                .and_then(serde_json::Value::as_str)
                .filter(|value| validate_token(value).is_ok())
                .map(str::to_owned);
            let profile = (|| {
                let data: Profile = serde_json::from_value(data)
                    .map_err(|_| migu_upstream_error("Migu profile fields are invalid"))?;
                let nickname = data.nick_name.filter(|v| !v.trim().is_empty());
                if nickname
                    .as_ref()
                    .is_some_and(|v| v.len() > 512 || v.chars().any(char::is_control))
                {
                    return Err(migu_upstream_error("Migu profile nickname is invalid"));
                }
                let mut profile = AccountProfile::authenticated(Platform::Migu, account);
                profile.user_id = Some(data.user_id);
                profile.nickname = nickname;
                profile.avatar_url = data.small_icon.as_deref().and_then(profile_image);
                profile
                    .extensions
                    .insert("backend".to_owned(), json!("official_h5_user_info"));
                Ok(profile)
            })();
            Ok(AccountRead {
                user_id,
                token,
                profile,
                native_session,
            })
        })
        .await
    }

    pub(crate) async fn account_h5_token(
        &self,
        token: &str,
        session: &str,
    ) -> Result<AccountData<String>> {
        validate_token(session)?;
        self.session_request(Operation::H5Token(session), token, |body, headers| {
            Ok(AccountData {
                token: rotated_token(headers, Operation::H5Token(session).path())?
                    .unwrap_or_else(|| token.to_owned()),
                data: body
                    .data
                    .and_then(|value| value.as_str().map(str::to_owned))
                    .filter(|value| validate_token(value).is_ok())
                    .ok_or_else(|| migu_upstream_error("Migu H5 exchange omitted a token string")),
            })
        })
        .await
    }

    pub(crate) async fn clear_pacm(&self, source: &MiguCredential) -> Result<()> {
        // A successful clear may delete cookies; no replacement session is accepted here.
        self.session_request(Operation::Logout, source.token(), |_, _| Ok(()))
            .await
    }

    pub(crate) async fn account_membership(
        &self,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<tuneweave_core::MembershipSummary>> {
        self.session_request(Operation::Membership, token, |body, headers| {
            let data = body
                .data
                .ok_or_else(|| migu_upstream_error("Migu membership omitted its data"))?;
            if let Some(id) = data.get("userId") {
                if id.as_str() != Some(uid) {
                    return Err(authentication_required());
                }
            }
            Ok(AccountData {
                token: rotated_token_for_host(
                    headers,
                    Operation::Membership.host(),
                    Operation::Membership.path(),
                )?
                .unwrap_or_else(|| token.to_owned()),
                data: super::membership::parse_membership(data, uid),
            })
        })
        .await
    }

    pub(crate) async fn account_member_icons(
        &self,
        token: &str,
    ) -> Result<AccountData<Vec<super::membership::MemberIcon>>> {
        self.session_request(Operation::MemberIcons, token, |body, headers| {
            let data = body
                .data
                .ok_or_else(|| migu_upstream_error("Migu member icons omitted their data"))?;
            Ok(AccountData {
                token: rotated_token_for_host(
                    headers,
                    Operation::MemberIcons.host(),
                    Operation::MemberIcons.path(),
                )?
                .unwrap_or_else(|| token.to_owned()),
                data: super::membership::parse_icons(data),
            })
        })
        .await
    }

    pub(crate) async fn account_media_identities(
        &self,
        token: &str,
        uid: &str,
    ) -> Result<AccountData<Vec<super::membership::MediaIdentity>>> {
        self.session_request(Operation::MediaIdentities, token, |body, headers| {
            let data = body
                .data
                .ok_or_else(|| migu_upstream_error("Migu media identities omitted their data"))?;
            if let Some(id) = data.get("userId") {
                if id.as_str() != Some(uid) {
                    return Err(authentication_required());
                }
            }
            Ok(AccountData {
                token: rotated_token_for_host(
                    headers,
                    Operation::MediaIdentities.host(),
                    Operation::MediaIdentities.path(),
                )?
                .unwrap_or_else(|| token.to_owned()),
                data: super::membership::parse_media_identities(data),
            })
        })
        .await
    }
}

pub(super) fn profile_image(value: &str) -> Option<String> {
    if value.len() > 4096 {
        return None;
    }
    let url = Url::parse(value).ok()?;
    let host = url.host_str()?;
    (url.scheme() == "https"
        && (host == "migu.cn" || host.ends_with(".migu.cn"))
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.fragment().is_none())
    .then(|| url.to_string())
}

pub(super) fn exchanged_token(headers: &HeaderMap) -> Result<String> {
    if !headers.contains_key("pacmtoken") {
        return Err(migu_upstream_error(
            "Migu token exchange omitted its PACM header",
        ));
    }
    rotated_token(headers, "/user/h5/token-validate/v3.0")?
        .ok_or_else(|| migu_upstream_error("Migu token exchange omitted its PACM update"))
}

fn rotated_token(headers: &HeaderMap, request_path: &str) -> Result<Option<String>> {
    rotated_token_for_host(headers, HOST, request_path)
}

pub(super) fn rotated_token_for_host(
    headers: &HeaderMap,
    host: &str,
    request_path: &str,
) -> Result<Option<String>> {
    let mut result: Option<String> = None;
    let mut accept = |token: &str| -> Result<()> {
        validate_token(token)
            .map_err(|_| migu_upstream_error("Invalid Migu session token update"))?;
        if result.as_deref().is_some_and(|v| v != token) {
            return Err(migu_upstream_error(
                "Conflicting Migu session token updates",
            ));
        }
        result = Some(token.to_owned());
        Ok(())
    };
    for header in headers.get_all("pacmtoken") {
        accept(
            header
                .to_str()
                .map_err(|_| migu_upstream_error("Invalid Migu session token header"))?,
        )?;
    }
    for header in headers.get_all(SET_COOKIE) {
        let Ok(raw) = header.to_str() else {
            return Err(migu_upstream_error("Invalid Migu session cookie"));
        };
        let mut parts = raw.split(';');
        let Some((name, token)) = parts.next().and_then(|p| p.trim().split_once('=')) else {
            continue;
        };
        if name != "pacmtoken" {
            continue;
        }
        let mut max_age = None;
        let mut expires: Option<&str> = None;
        let mut scoped = true;
        let mut attributes = std::collections::BTreeSet::new();
        for part in parts {
            let (name, value) = part.trim().split_once('=').unwrap_or((part.trim(), ""));
            let name = name.to_ascii_lowercase();
            if matches!(name.as_str(), "domain" | "path" | "max-age" | "expires")
                && !attributes.insert(name.clone())
            {
                return Err(migu_upstream_error(
                    "Ambiguous Migu session cookie attributes",
                ));
            }
            match name.as_str() {
                "domain" => {
                    let domain = value.trim_start_matches('.').to_ascii_lowercase();
                    scoped &= domain == "migu.cn" || domain == host;
                }
                "path" => {
                    scoped &= value == "/"
                        || request_path == value
                        || (request_path.starts_with(value)
                            && (value.ends_with('/')
                                || request_path.as_bytes().get(value.len()) == Some(&b'/')))
                }
                "max-age" => {
                    max_age = Some(
                        value
                            .parse::<i64>()
                            .map_err(|_| migu_upstream_error("Invalid Migu session cookie age"))?,
                    )
                }
                "expires" => {
                    expires = Some(value);
                }
                _ => {}
            }
        }
        if !scoped {
            continue;
        }
        // Cookie dates that a user agent cannot parse are ignored. Max-Age
        // takes precedence over Expires, so only inspect Expires when the
        // upstream did not provide a Max-Age value.
        let expired = max_age.map(|age| age <= 0).unwrap_or_else(|| {
            expires
                .and_then(|date| httpdate::parse_http_date(date).ok())
                .is_some_and(|date| date <= std::time::SystemTime::now())
        });
        if token.is_empty() || matches!(token, "null" | "undefined") || expired {
            return Err(authentication_required());
        }
        accept(token)?;
    }
    Ok(result)
}

pub(super) async fn read_session_body(response: reqwest::Response) -> Result<Vec<u8>> {
    read_session_body_limited(response, MAX_RESPONSE).await
}

pub(super) async fn read_session_body_limited(
    mut response: reqwest::Response,
    maximum: u64,
) -> Result<Vec<u8>> {
    if response.content_length().is_some_and(|n| n > maximum) {
        return Err(migu_upstream_error("Migu session response is oversized"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|e| {
        crate::credential::error(
            if e.is_timeout() {
                ErrorCode::UpstreamTimeout
            } else {
                ErrorCode::UpstreamError
            },
            "Migu session response could not be read",
        )
    })? {
        if bytes.len().saturating_add(chunk.len()) > maximum as usize {
            return Err(migu_upstream_error("Migu session response is oversized"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}
