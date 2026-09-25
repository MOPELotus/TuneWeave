//! Native QR completion. Only authenticated exchange output can establish identity.

use std::{
    collections::BTreeMap,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use md5::{Digest, Md5};
use reqwest::{
    StatusCode,
    header::{CONTENT_LENGTH, CONTENT_TYPE},
};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{DeserializeOwned, IgnoredAny},
};
use serde_json::{Value, json};
use tuneweave_core::{
    AccountProfile, ErrorCode, Platform, ProviderAuthResult, ProviderCredential, Result,
    TuneWeaveError,
};

use crate::{
    KugouClient, KugouLoginClient, KugouQrAuthorization,
    client::{normalize_image_url, rsa_pkcs1_v15_encrypt_for_client},
    credential::{KugouCredential, NativeSession, valid_secret, valid_uid},
    device::valid_dfid,
    login::crypto::{self, ExchangeCipher},
    signing::{ANDROID_SALT, android_signature, concept_signature},
};

const HOST: &str = "gateway.kugou.com";
const RESPONSE_LIMIT: usize = 131_072;

pub(crate) mod cloud;
pub(crate) mod following;
pub(crate) mod library;
pub(crate) mod media;
pub(crate) mod membership;
pub(crate) mod purchases;

#[derive(Clone, Copy, Eq, PartialEq)]
enum Endpoint {
    Exchange,
    Profile,
    Library,
    LibraryTracks,
    LibraryAdd,
    LibraryAddConcept,
    LibraryRemove,
    ListCreate,
    ListCollect,
    ListModify,
    ListDelete,
    ListDeleteStandard,
    PurchasedTracks,
    PurchasedAlbums,
}
impl Endpoint {
    fn path(self) -> &'static str {
        match self {
            Self::Exchange => "/login.user/v5/login_by_token",
            Self::Profile => "/usercenter/v3/get_my_info",
            Self::Library => "/cloudlist.service/v8/get_all_list",
            Self::LibraryTracks => "/v4/get_list_all_file_v3",
            Self::LibraryAdd => "/cloudlist.service/v6/add_song",
            Self::LibraryAddConcept => "/cloudlist.service/v4/add_song",
            Self::LibraryRemove => "/v4/delete_songs",
            Self::ListCreate | Self::ListCollect => "/cloudlist.service/v5/add_list",
            Self::ListModify => "/cloudlist.service/v4/modify_list",
            Self::ListDelete => "/cloudlist.service/v3/delete_list",
            Self::ListDeleteStandard => "/cloudlist.service/v2/delete_list",
            Self::PurchasedTracks => "/openapi/copyright/v1/audio/get_goods",
            Self::PurchasedAlbums => "/openapi/v1/copyright/get_album_goods",
        }
    }
    fn operation(self) -> &'static str {
        match self {
            Self::Exchange => "native_token_exchange",
            Self::Profile => "native_self_profile",
            Self::Library => "native_account_playlists",
            Self::LibraryTracks => "native_account_playlist_tracks",
            Self::LibraryAdd | Self::LibraryAddConcept => "native_playlist_tracks_add",
            Self::LibraryRemove => "native_playlist_tracks_remove",
            Self::ListCreate => "native_playlist_create",
            Self::ListCollect => "native_playlist_collect",
            Self::ListModify => "native_playlist_modify",
            Self::ListDelete | Self::ListDeleteStandard => "native_playlist_delete",
            Self::PurchasedTracks => "native_purchased_tracks",
            Self::PurchasedAlbums => "native_purchased_albums",
        }
    }
}

#[cfg(test)]
pub(crate) fn response_bytes_for_request(request: &[u8], frame: &str) -> Vec<u8> {
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};

    let request_head = String::from_utf8_lossy(request);
    if !request_head
        .lines()
        .next()
        .is_some_and(|line| line.starts_with("POST /cloudlist.service/v2/delete_list?"))
    {
        return frame.as_bytes().to_vec();
    }
    let Some((head, body)) = frame.split_once("\r\n\r\n") else {
        return frame.as_bytes().to_vec();
    };
    if !head.starts_with("HTTP/1.1 200 ")
        || !head
            .lines()
            .any(|line| line.eq_ignore_ascii_case("Content-Type: application/json"))
    {
        return frame.as_bytes().to_vec();
    }
    let Ok(encrypted) = crate::client::encrypt_device_profile(
        body.as_bytes(),
        crate::account::library::management::TEST_RANDOM_SEED,
    ) else {
        return frame.as_bytes().to_vec();
    };
    let Ok(encrypted) = BASE64.decode(encrypted.as_bytes()) else {
        return frame.as_bytes().to_vec();
    };
    let response_head = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        encrypted.len()
    );
    let mut response = response_head.into_bytes();
    response.extend_from_slice(&encrypted);
    response
}

impl KugouClient {
    /// Completes QR authorization and returns a caller-owned credential.
    ///
    /// Performs a token exchange with an explicit same-UID response, followed by an
    /// authenticated self-profile read. Writes no account or device state. The account
    /// alias is `default`; server/both ownership belongs to the provider layer.
    /// Web authorization uses the separate official cookie exchange instead of native APIs.
    /// Real account acceptance remains separate from the offline protocol tests.
    pub async fn complete_qr_login(
        &self,
        authorization: KugouQrAuthorization,
    ) -> Result<ProviderAuthResult> {
        if authorization.client_kind() == KugouLoginClient::Web {
            return self.complete_web_qr_login(authorization).await;
        }
        let session = self.exchange_qr_authorization(authorization).await?;
        let profile = self.native_profile(&session).await?;
        let credential = KugouCredential::verified(session)?.caller()?;
        Ok(ProviderAuthResult {
            profile,
            credential: Some(credential),
        })
    }

    pub(crate) async fn exchange_qr_authorization(
        &self,
        authorization: KugouQrAuthorization,
    ) -> Result<NativeSession> {
        let (client, device, user_id, token) = authorization.into_parts();
        if client == KugouLoginClient::Web {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "KuGou Web QR completion requires its cookie exchange",
            ));
        }
        let session = NativeSession {
            client,
            device,
            user_id,
            token,
            vip_token: None,
            t1: None,
        };
        validate_session(&session)?;
        self.exchange_native_token(&session).await
    }

    /// Explicitly exchanges a caller-owned native token and verifies the self profile.
    ///
    /// Keep the returned credential. If the exchange succeeds but a later non-authentication
    /// error occurs, retain `TuneWeaveError::take_caller_credential_update()` instead.
    /// Invalidated or mismatched identities never export a credential. No server account
    /// is read or changed, and this does not establish a new login generation.
    pub async fn refresh_native_login(
        &self,
        source: &ProviderCredential,
    ) -> Result<ProviderAuthResult> {
        let previous = KugouCredential::parse_caller(source)?;
        let session = self.exchange_native_token(&previous.session).await?;
        let updated = previous.rotate(session)?;
        let caller = updated.caller()?;
        match self.native_profile(&updated.session).await {
            Ok(profile) => Ok(ProviderAuthResult {
                profile,
                credential: Some(caller),
            }),
            Err(failure) => Err(failure.with_caller_credential_update(caller)),
        }
    }

    pub(crate) async fn exchange_native_token(
        &self,
        session: &NativeSession,
    ) -> Result<NativeSession> {
        validate_session(session)?;
        let milliseconds = now_ms()?;
        let cipher = ExchangeCipher::random()?;
        let body = exchange_body(session, milliseconds, &cipher)?;
        self.native_post(
            Endpoint::Exchange,
            session,
            milliseconds / 1000,
            body,
            |bytes| parse_exchange(bytes, session, &cipher),
        )
        .await
    }

    pub(crate) async fn native_profile(&self, session: &NativeSession) -> Result<AccountProfile> {
        let seconds = now_ms()? / 1000;
        #[derive(Serialize)]
        struct Body {
            visit_time: u64,
            usertype: u8,
            p: String,
            userid: u64,
        }
        let body = crypto::encode(&Body {
            visit_time: seconds,
            usertype: 1,
            p: crypto::profile_p(session.client, &session.token, seconds)?,
            userid: session.user_id.parse().map_err(|_| malformed())?,
        })?;
        self.native_post(Endpoint::Profile, session, seconds, body, |bytes| {
            parse_profile(bytes, session)
        })
        .await
    }

    async fn native_post<T>(
        &self,
        endpoint: Endpoint,
        session: &NativeSession,
        seconds: u64,
        body: Vec<u8>,
        decode: impl FnOnce(&[u8]) -> Result<T>,
    ) -> Result<T> {
        self.native_post_with_standard_seed(endpoint, session, seconds, body, None, decode)
            .await
    }

    async fn native_post_standard_delete<T>(
        &self,
        session: &NativeSession,
        seconds: u64,
        seed: &str,
        body: Vec<u8>,
        decode: impl FnOnce(&[u8]) -> Result<T>,
    ) -> Result<T> {
        self.native_post_with_standard_seed(
            Endpoint::ListDeleteStandard,
            session,
            seconds,
            body,
            Some(seed),
            decode,
        )
        .await
    }

    async fn native_post_with_standard_seed<T>(
        &self,
        endpoint: Endpoint,
        session: &NativeSession,
        seconds: u64,
        body: Vec<u8>,
        standard_seed: Option<&str>,
        decode: impl FnOnce(&[u8]) -> Result<T>,
    ) -> Result<T> {
        let mut params = if let Some(seed) = standard_seed {
            if endpoint != Endpoint::ListDeleteStandard
                || session.client != KugouLoginClient::Standard
                || seed.len() != 6
                || !seed.bytes().all(|byte| byte.is_ascii_alphanumeric())
            {
                return Err(malformed());
            }
            let appid = session.client.appid().to_string();
            let clientver = session.client.clientver().to_string();
            let clienttime = seconds.to_string();
            // Standard's single-delete request signs this legacy query profile; it
            // carries UID/token in RSA `p` and the AES seed shared with the body.
            let query_key = format!("{appid}{ANDROID_SALT}{clientver}{clienttime}");
            let key = hex::encode(Md5::digest(query_key.as_bytes()));
            let profile = standard_playlist_profile(session, seed)?;
            BTreeMap::from([
                ("appid", appid),
                ("clientver", clientver),
                ("clienttime", clienttime),
                ("mid", session.device.mid.clone()),
                ("dfid", session.device.dfid().to_owned()),
                ("key", key),
                ("p", profile),
            ])
        } else {
            if endpoint == Endpoint::ListDeleteStandard {
                return Err(malformed());
            }
            BTreeMap::from([
                ("appid", session.client.appid().to_string()),
                ("clientver", session.client.clientver().to_string()),
                ("clienttime", seconds.to_string()),
                ("uuid", "-".to_owned()),
                ("mid", session.device.mid.clone()),
                ("dfid", session.device.dfid().to_owned()),
                ("userid", session.user_id.clone()),
                ("token", session.token.clone()),
            ])
        };
        if matches!(
            endpoint,
            Endpoint::Profile
                | Endpoint::Library
                | Endpoint::LibraryTracks
                | Endpoint::LibraryAdd
                | Endpoint::LibraryAddConcept
                | Endpoint::LibraryRemove
                | Endpoint::ListCreate
                | Endpoint::ListCollect
                | Endpoint::ListModify
                | Endpoint::ListDelete
        ) {
            params.insert("plat", "1".to_owned());
        }
        if matches!(
            endpoint,
            Endpoint::LibraryAdd | Endpoint::LibraryAddConcept | Endpoint::ListCreate
        ) {
            params.insert("last_time", seconds.to_string());
            params.insert("last_area", "gztx".to_owned());
        }
        let signature = match session.client {
            KugouLoginClient::Standard => android_signature(&params, &body),
            KugouLoginClient::Concept => concept_signature(&params, &body),
            KugouLoginClient::Web => return Err(malformed()),
        };
        params.insert("signature", signature);
        let url = format!("https://{HOST}{}", endpoint.path());
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(endpoint.path()).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let request = self
                .http
                .post(url)
                .header(CONTENT_TYPE, "application/json")
                .header("accept", "application/json")
                .header(
                    "user-agent",
                    "Android15-1070-11083-46-0-DiscoveryDRADProtocol-wifi",
                )
                .header("dfid", session.device.dfid())
                .header("mid", &session.device.mid)
                .header("clienttime", seconds)
                .query(&params)
                .body(body);
            let request = if matches!(endpoint, Endpoint::LibraryTracks | Endpoint::LibraryRemove) {
                request.header("x-router", "cloudlist.service.kugou.com")
            } else {
                request
            };
            let response = request.send().await.map_err(network_error)?;
            status = Some(response.status());
            let limit = match endpoint {
                Endpoint::LibraryTracks => 4_194_304,
                Endpoint::Library
                | Endpoint::LibraryAdd
                | Endpoint::LibraryAddConcept
                | Endpoint::LibraryRemove
                | Endpoint::ListCreate
                | Endpoint::ListCollect
                | Endpoint::ListModify
                | Endpoint::ListDelete
                | Endpoint::ListDeleteStandard
                | Endpoint::PurchasedTracks
                | Endpoint::PurchasedAlbums => 1_048_576,
                _ => RESPONSE_LIMIT,
            };
            let bytes = read_response_with_limit(response, limit).await?;
            decode(&bytes)
        }
        .await;
        // Fixed path and result category only: the URL query, body and secrets are excluded.
        self.log_upstream_request(
            endpoint.operation(),
            HOST,
            endpoint.path(),
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

fn standard_playlist_profile(session: &NativeSession, seed: &str) -> Result<String> {
    #[derive(Serialize)]
    struct Profile<'a> {
        aes: &'a str,
        uid: u64,
        token: &'a str,
    }
    let plaintext = crypto::encode(&Profile {
        aes: seed,
        uid: session.user_id.parse().map_err(|_| malformed())?,
        token: &session.token,
    })?;
    rsa_pkcs1_v15_encrypt_for_client(session.client, &plaintext)
        .map(|value| value.to_ascii_uppercase())
}

fn exchange_body(
    session: &NativeSession,
    milliseconds: u64,
    cipher: &ExchangeCipher,
) -> Result<Vec<u8>> {
    #[derive(Serialize)]
    struct Body<'a> {
        dfid: &'a str,
        p3: String,
        plat: u8,
        t1: Value,
        t2: Value,
        t3: &'static str,
        pk: String,
        params: String,
        userid: &'a str,
        clienttime_ms: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        dev: Option<&'static str>,
    }
    let (t1, t2, dev) = if session.client == KugouLoginClient::Concept {
        let (t1, t2) = crypto::concept_fingerprints(session, milliseconds)?;
        (json!(t1), json!(t2), Some(crypto::DESKTOP_MODEL))
    } else {
        (json!(0), json!(0), None)
    };
    crypto::encode(&Body {
        dfid: session.device.dfid(),
        p3: crypto::p3(session, milliseconds / 1000)?,
        plat: 1,
        t1,
        t2,
        t3: "MCwwLDAsMCwwLDAsMCwwLDA=",
        pk: cipher.pk(session.client, milliseconds)?,
        params: cipher.encrypt(b"{}")?,
        userid: &session.user_id,
        clienttime_ms: milliseconds,
        dev,
    })
}

#[derive(Deserialize)]
struct Envelope<T> {
    status: i64,
    error_code: i64,
    data: Option<T>,
}

fn data<T: DeserializeOwned>(bytes: &[u8]) -> Result<T> {
    // Read the business status independently of the success payload's schema.
    let status: Envelope<IgnoredAny> = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if status.status != 1 || status.error_code != 0 {
        let code = if status.status == 0 && status.error_code == 20017 {
            ErrorCode::AuthenticationRequired
        } else {
            ErrorCode::UpstreamError
        };
        return Err(error(code, "KuGou native account request was rejected")
            .with_details(json!({"platform_code": status.error_code})));
    }
    let envelope: Envelope<T> = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    envelope.data.ok_or_else(malformed)
}

#[derive(Default, Deserialize)]
struct TokenFields {
    #[serde(
        default,
        alias = "user_id",
        alias = "uid",
        deserialize_with = "present"
    )]
    userid: Option<Value>,
    #[serde(default, deserialize_with = "present")]
    token: Option<String>,
    #[serde(default, deserialize_with = "optional_update")]
    vip_token: Option<Option<String>>,
    #[serde(default, deserialize_with = "optional_update")]
    t1: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    dfid: Option<String>,
    #[serde(default, deserialize_with = "present")]
    secu_params: Option<String>,
}

fn present<'de, D, T>(deserializer: D) -> std::result::Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
}
fn optional_update<'de, D>(deserializer: D) -> std::result::Result<Option<Option<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

fn uid(value: Value) -> Result<String> {
    let value = match value {
        Value::String(v) => v,
        Value::Number(n) => n.as_u64().map(|n| n.to_string()).ok_or_else(malformed)?,
        _ => return Err(malformed()),
    };
    if !valid_uid(&value) {
        return Err(malformed());
    }
    Ok(value)
}

fn merge<T: PartialEq>(a: Option<T>, b: Option<T>) -> Result<Option<T>> {
    match (a, b) {
        (Some(a), Some(b)) if a != b => Err(identity_conflict()),
        (Some(a), _) => Ok(Some(a)),
        (_, b) => Ok(b),
    }
}

fn parse_exchange(
    bytes: &[u8],
    previous: &NativeSession,
    cipher: &ExchangeCipher,
) -> Result<NativeSession> {
    let outer: TokenFields = data(bytes)?;
    let inner = match &outer.secu_params {
        Some(encrypted) => {
            let plain = cipher.decrypt(encrypted)?;
            let text = std::str::from_utf8(&plain).map_err(|_| malformed())?;
            if text.trim_start().starts_with('{') {
                let fields: TokenFields = serde_json::from_str(text).map_err(|_| malformed())?;
                if fields.secu_params.is_some() {
                    return Err(malformed());
                }
                fields
            } else {
                let token = if text.trim_start().starts_with('"') {
                    serde_json::from_str::<String>(text).map_err(|_| malformed())?
                } else {
                    // The reference also returns raw token text, without JSON quoting.
                    if !valid_secret(text) || text.bytes().any(|b| b"{}[]\"".contains(&b)) {
                        return Err(malformed());
                    }
                    text.to_owned()
                };
                TokenFields {
                    token: Some(token),
                    ..TokenFields::default()
                }
            }
        }
        None => TokenFields::default(),
    };
    let user_id = merge(
        outer.userid.map(uid).transpose()?,
        inner.userid.map(uid).transpose()?,
    )?
    .ok_or_else(malformed)?;
    if user_id != previous.user_id {
        return Err(identity_conflict());
    }
    let token = merge(outer.token, inner.token)?
        .filter(|v| valid_secret(v))
        .ok_or_else(malformed)?;
    let vip_token = token_update(
        merge(outer.vip_token, inner.vip_token)?,
        &previous.vip_token,
    )?;
    let t1 = token_update(merge(outer.t1, inner.t1)?, &previous.t1)?;
    let mut device = previous.device.clone();
    if let Some(dfid) = merge(outer.dfid, inner.dfid)? {
        if !valid_dfid(&dfid) {
            return Err(malformed());
        }
        device.dfid = Some(dfid);
    }
    let session = NativeSession {
        client: previous.client,
        device,
        user_id,
        token,
        vip_token,
        t1,
    };
    if !session.valid() {
        return Err(malformed());
    }
    Ok(session)
}

fn token_update(
    update: Option<Option<String>>,
    previous: &Option<String>,
) -> Result<Option<String>> {
    match update {
        None => Ok(previous.clone()),
        Some(None) => Ok(None),
        Some(Some(v)) if v.is_empty() => Ok(None),
        Some(Some(v)) if valid_secret(&v) => Ok(Some(v)),
        _ => Err(malformed()),
    }
}

#[derive(Deserialize)]
struct Profile {
    #[serde(
        default,
        alias = "user_id",
        alias = "uid",
        deserialize_with = "present"
    )]
    userid: Option<Value>,
    nickname: Option<String>,
    username: Option<String>,
    pic: Option<String>,
    servertime: Option<u64>,
    vip_type: Option<library::Number>,
}

fn parse_profile(bytes: &[u8], verified: &NativeSession) -> Result<AccountProfile> {
    let profile: Profile = data(bytes)?;
    if profile.userid.is_none()
        && profile.nickname.is_none()
        && profile.username.is_none()
        && profile.pic.is_none()
        && profile.servertime.is_none()
    {
        return Err(malformed());
    }
    if let Some(value) = profile.userid {
        if uid(value)? != verified.user_id {
            return Err(identity_conflict());
        }
    }
    let nickname = profile_text(profile.nickname, 512)?;
    // Validate but do not expose a login name that might contain account contact details.
    profile_text(profile.username, 512)?;
    let pic = profile_text(profile.pic, 4096)?;
    let avatar_url = match pic {
        Some(v) => Some(normalize_image_url(&v).ok_or_else(malformed)?),
        None => None,
    };
    let mut extensions = BTreeMap::new();
    if let Some(vip_type) = profile.vip_type {
        let vip_type = u32::try_from(vip_type.0).map_err(|_| malformed())?;
        // Preserve the upstream code without inferring memberships or media rights.
        extensions.insert("vip_type".into(), json!(vip_type));
    }
    Ok(AccountProfile {
        platform: Platform::Kugou,
        account: "default".to_owned(),
        user_id: Some(verified.user_id.clone()),
        nickname,
        avatar_url,
        authenticated: true,
        extensions,
    })
}

fn profile_text(value: Option<String>, limit: usize) -> Result<Option<String>> {
    match value {
        Some(value) if value.len() > limit || value.chars().any(char::is_control) => {
            Err(malformed())
        }
        Some(value) if value.trim().is_empty() => Ok(None),
        value => Ok(value),
    }
}

pub(crate) async fn read_response(response: reqwest::Response) -> Result<Vec<u8>> {
    read_response_with_limit(response, RESPONSE_LIMIT).await
}

pub(crate) async fn read_response_with_limit(
    response: reqwest::Response,
    limit: usize,
) -> Result<Vec<u8>> {
    read_response_with_types(response, limit, &["application/json"]).await
}

pub(crate) async fn read_response_with_types(
    mut response: reqwest::Response,
    limit: usize,
    types: &[&str],
) -> Result<Vec<u8>> {
    let status = response.status();
    if response
        .headers()
        .get("ssa-code")
        .is_some_and(|v| v.as_bytes() != b"0" && !v.is_empty())
    {
        return Err(error(
            ErrorCode::PermissionDenied,
            "KuGou login requires additional verification",
        )
        .with_details(json!({"additional_verification_required":true})));
    }
    if !status.is_success() {
        let code = match status {
            StatusCode::TOO_MANY_REQUESTS => ErrorCode::RateLimited,
            StatusCode::UNAUTHORIZED => ErrorCode::AuthenticationRequired,
            _ => ErrorCode::UpstreamError,
        };
        let mut details = json!({"http_status":status.as_u16()});
        if code == ErrorCode::RateLimited {
            let delay = response
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(2)
                .clamp(2, 300);
            details["retry_after_secs"] = json!(delay);
        }
        return Err(error(code, "KuGou account HTTP request failed").with_details(details));
    }
    let json_type = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|v| types.iter().any(|mime| v.trim().eq_ignore_ascii_case(mime)));
    if !json_type
        || response
            .headers()
            .get(CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<u64>().ok())
            .is_some_and(|v| v > limit as u64)
    {
        return Err(malformed());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(network_error)? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(malformed());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn validate_session(session: &NativeSession) -> Result<()> {
    if !session.valid() {
        return Err(error(
            ErrorCode::InvalidRequest,
            "KuGou native login input is invalid",
        ));
    }
    Ok(())
}
pub(crate) fn now_ms() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|v| u64::try_from(v.as_millis()).ok())
        .ok_or_else(|| {
            error(
                ErrorCode::InternalError,
                "KuGou native login requires a valid clock",
            )
        })
}
fn error(code: ErrorCode, message: &'static str) -> TuneWeaveError {
    TuneWeaveError::new(code, message).with_platform(Platform::Kugou)
}
fn malformed() -> TuneWeaveError {
    error(
        ErrorCode::UpstreamError,
        "KuGou native account response is invalid",
    )
}
fn identity_conflict() -> TuneWeaveError {
    error(
        ErrorCode::Conflict,
        "KuGou native account response has inconsistent identity or credentials",
    )
}
pub(crate) fn network_error(e: reqwest::Error) -> TuneWeaveError {
    error(
        if e.is_timeout() {
            ErrorCode::UpstreamTimeout
        } else {
            ErrorCode::UpstreamError
        },
        "KuGou account transport failed",
    )
}

#[cfg(test)]
mod tests;
