//! Video rights and HTTPS tracker resolution, bound to verified metadata and identity.
use super::dto::Number;
use super::*;
use crate::{KugouLoginClient, credential::NativeSession};
use tuneweave_core::{VideoDetail, VideoStream};

const PRIVILEGE_PATH: &str = "/v1/get_video_privilege";
const URL_PATH: &str = "/v2/interface/index";

struct VideoIdentity<'a> {
    device: KugouDeviceIdentity,
    session: Option<&'a NativeSession>,
    vip_type: u32,
}
impl VideoIdentity<'_> {
    fn appid(&self) -> u16 {
        self.session.map_or(ANDROID_APP_ID, |s| s.client.appid())
    }
    fn clientver(&self) -> u32 {
        self.session
            .map_or(ANDROID_CLIENT_VERSION, |s| s.client.clientver())
    }
    fn userid(&self) -> &str {
        self.session.map_or("0", |s| s.user_id.as_str())
    }
    fn token(&self) -> &str {
        self.session.map_or("", |s| s.token.as_str())
    }
    fn signature(&self, query: &BTreeMap<&str, String>, body: &[u8]) -> String {
        if self
            .session
            .is_some_and(|s| s.client == KugouLoginClient::Concept)
        {
            crate::signing::concept_signature(query, body)
        } else {
            crate::signing::android_signature(query, body)
        }
    }
    fn key(&self, hash: &str) -> String {
        let salt = if self
            .session
            .is_some_and(|s| s.client == KugouLoginClient::Concept)
        {
            "185672dd44712f60bb1736df5a377e82"
        } else {
            TRACKER_KEY_SALT
        };
        md5_hex(format!(
            "{hash}{salt}{}{}{}",
            self.appid(),
            self.device.mid,
            self.userid()
        ))
    }
    fn check_url(&self, url: &str) -> Result<()> {
        let Some(session) = self.session else {
            return Ok(());
        };
        let decoded = url::form_urlencoded::parse(url.as_bytes())
            .flat_map(|(k, v)| [k.into_owned(), v.into_owned()])
            .collect::<Vec<_>>();
        for secret in std::iter::once(session.token.as_str())
            .chain(session.vip_token.as_deref())
            .chain(session.t1.as_deref())
        {
            if url.contains(secret) || decoded.iter().any(|part| part.contains(secret)) {
                return Err(malformed());
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize)]
struct Asset {
    source_key: String,
    hash: String,
    bitrate: Option<u64>,
    size: Option<u64>,
    width: Option<u32>,
    height: Option<u32>,
}
#[derive(Deserialize)]
struct Privileges {
    data: Vec<Privilege>,
}
#[derive(Debug, Deserialize, Serialize)]
struct Privilege {
    hash: String,
    id: Number,
    status: Number,
    privilege: Number,
    pay_type: Number,
    fail_process: Number,
    info: Option<FileInfo>,
}
#[derive(Debug, Deserialize, Serialize)]
struct FileInfo {
    filesize: Number,
    bitrate: Number,
}
#[derive(Deserialize)]
struct Tracker {
    status: i64,
    errcode: Option<i64>,
    error_code: Option<i64>,
    privileges: Option<HashMap<Number>>,
    data: Option<HashMap<TrackerFile>>,
}
#[derive(Deserialize)]
struct TrackerFile {
    filesize: Option<Number>,
    downurl: Option<String>,
    #[serde(default)]
    backupdownurl: Vec<String>,
}

// JSON object keys carry resource identities. Reject duplicates before collecting a map,
// including different casing of the same hash.
struct HashMap<T>(BTreeMap<String, T>);
impl<'de, T: Deserialize<'de>> Deserialize<'de> for HashMap<T> {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        struct Visitor<T>(std::marker::PhantomData<T>);
        impl<'de, T: Deserialize<'de>> serde::de::Visitor<'de> for Visitor<T> {
            type Value = HashMap<T>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("one unambiguous video resource hash")
            }
            fn visit_map<M: serde::de::MapAccess<'de>>(
                self,
                mut m: M,
            ) -> std::result::Result<Self::Value, M::Error> {
                let mut values = BTreeMap::new();
                while let Some((key, value)) = m.next_entry::<String, T>()? {
                    if !valid_hash(&key)
                        || values.insert(key.to_ascii_uppercase(), value).is_some()
                        || values.len() > 1
                    {
                        return Err(serde::de::Error::custom("ambiguous video resource hash"));
                    }
                }
                Ok(HashMap(values))
            }
        }
        d.deserialize_map(Visitor(std::marker::PhantomData))
    }
}

impl KugouClient {
    pub(crate) async fn public_video_stream(
        &self,
        detail: &VideoDetail,
        resolution: u32,
    ) -> Result<VideoStream> {
        self.resolve_video_stream(detail, resolution, None, || Ok(()))
            .await
    }

    pub(crate) async fn native_video_stream(
        &self,
        detail: &VideoDetail,
        resolution: u32,
        session: &NativeSession,
        vip_type: u32,
        check: impl FnMut() -> Result<()>,
    ) -> Result<VideoStream> {
        if !session.valid() {
            return Err(malformed());
        }
        self.resolve_video_stream(detail, resolution, Some((session, vip_type)), check)
            .await
    }

    async fn resolve_video_stream(
        &self,
        detail: &VideoDetail,
        resolution: u32,
        native: Option<(&NativeSession, u32)>,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<VideoStream> {
        let mut assets: Vec<Asset> = serde_json::from_value(
            detail
                .video
                .extensions
                .get("catalogue_assets")
                .cloned()
                .ok_or_else(malformed)?,
        )
        .map_err(|_| malformed())?;
        if resolution == 0 || !super::videos::canonical_id(&detail.video.id) || assets.len() > 11 {
            return Err(malformed());
        }
        let mut seen = BTreeMap::new();
        for a in &assets {
            if !valid_hash(&a.hash)
                || a.height == Some(0)
                || a.width == Some(0)
                || a.size == Some(0)
            {
                return Err(malformed());
            }
            if let Some(previous) = seen.insert(a.hash.clone(), a) {
                if (
                    previous.size,
                    previous.width,
                    previous.height,
                    previous.bitrate,
                ) != (a.size, a.width, a.height, a.bitrate)
                {
                    return Err(malformed());
                }
            }
        }
        assets.sort_by_key(|a| selection_key(a, resolution));
        let mut seen = BTreeSet::new();
        assets.retain(|a| seen.insert(a.hash.clone()));
        let mut stream = empty_stream(detail, resolution);
        if assets.is_empty() {
            stream.message = Some("KuGou returned no video resources".into());
            return Ok(stream);
        }
        let identity = match native {
            Some((session, vip_type)) => VideoIdentity {
                device: session.device.clone(),
                session: Some(session),
                vip_type,
            },
            None => VideoIdentity {
                device: self.device_identity()?,
                session: None,
                vip_type: 0,
            },
        };
        let device = &identity.device;
        let body = serde_json::to_vec(&json!({
            "appid":identity.appid(),"clientver":identity.clientver(),
            "area_code":1,"behavior":"play","dfid":device.dfid(),"mid":device.mid,
            "resource":assets.iter().map(|a|json!({"hash":a.hash,"id":0,"name":""})).collect::<Vec<_>>(),
            "token":identity.token(),"userid":identity.userid().parse::<u64>().map_err(|_|malformed())?,"vip":identity.vip_type
        })).map_err(|_| malformed())?;
        let response = self
            .video_media_request(false, BTreeMap::new(), body, &identity)
            .await;
        check()?;
        let bytes = response?;
        let privileges = parse_privileges(&bytes, &detail.video.id, &assets)?;
        stream.extensions.insert(
            "resource_privileges".into(),
            serde_json::to_value(&privileges).map_err(|_| malformed())?,
        );
        let selected = assets
            .iter()
            .zip(&privileges)
            .find(|(_, p)| p.status.0 == 1 && p.fail_process.0 == 0);
        let Some((asset, privilege)) = selected else {
            stream.message =
                Some("KuGou did not authorize a video resource for this identity".into());
            return Ok(stream);
        };
        let size = privilege.info.as_ref().ok_or_else(malformed)?.filesize.0;
        let query = BTreeMap::from([
            ("backupdomain", "1".into()),
            ("cmd", "123".into()),
            ("ext", "mp4".into()),
            ("ismp3", "0".into()),
            ("hash", asset.hash.clone()),
            ("pid", "1".into()),
            ("type", "1".into()),
            ("ssl", "1".into()),
            ("key", identity.key(&asset.hash)),
        ]);
        let response = self
            .video_media_request(true, query, Vec::new(), &identity)
            .await;
        check()?;
        let bytes = response?;
        let (code, urls) = parse_tracker(&bytes, &asset.hash, size)?;
        for url in &urls {
            identity.check_url(url)?;
        }
        stream.platform_code = Some(code);
        stream.extensions.insert(
            "selected_resource".into(),
            json!({
                "source_key":asset.source_key,"hash":asset.hash,"bitrate":asset.bitrate,
                "privilege":privilege.privilege.0,"pay_type":privilege.pay_type.0
            }),
        );
        stream.available = !urls.is_empty();
        stream.url = urls.first().cloned();
        stream.backup_urls = urls.into_iter().skip(1).collect();
        if stream.available {
            stream.width = asset.width;
            stream.height = asset.height;
            stream.actual_resolution = asset.height;
            stream.size = Some(size);
            stream.extensions.insert(
                "resolution_below_request".into(),
                json!(asset.height.map(|h| h < resolution)),
            );
        } else {
            stream.message =
                Some("KuGou video tracker did not return an authorized media URL".into());
        }
        Ok(stream)
    }

    async fn video_media_request(
        &self,
        tracker: bool,
        extra: BTreeMap<&str, String>,
        body: Vec<u8>,
        identity: &VideoIdentity<'_>,
    ) -> Result<Vec<u8>> {
        let device = &identity.device;
        let time = unix_seconds_now().to_string();
        let mut query = BTreeMap::from([
            ("appid", identity.appid().to_string()),
            ("clientver", identity.clientver().to_string()),
            ("clienttime", time.clone()),
            ("dfid", device.dfid().to_owned()),
            ("mid", device.mid.clone()),
            (
                "uuid",
                if identity.session.is_some() {
                    "-".into()
                } else {
                    device.guid.clone()
                },
            ),
            ("userid", identity.userid().into()),
            ("token", identity.token().into()),
        ]);
        for (key, value) in extra {
            if query.insert(key, value).is_some() {
                return Err(malformed());
            }
        }
        query.insert("signature", identity.signature(&query, &body));
        let (path, router, operation) = if tracker {
            (URL_PATH, "trackermv.kugou.com", "video_tracker")
        } else {
            (PRIVILEGE_PATH, "media.store.kugou.com", "video_privilege")
        };
        let url = format!("{ANDROID_GATEWAY}{path}");
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|o| o.join(path).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let request = if tracker {
                self.http.get(url)
            } else {
                self.http.post(url).body(body)
            };
            let request = if identity.session.is_some() {
                request.header("kg-rc", "1").header("kg-rec", "1")
            } else {
                request
            };
            let mut response = request
                .query(&query)
                .header("x-router", router)
                .header("user-agent", ANDROID_USER_AGENT)
                .header(CONTENT_TYPE, "application/json")
                .header("mid", &device.mid)
                .header("dfid", device.dfid())
                .header("clienttime", time)
                .send()
                .await
                .map_err(kugou_network_error)?;
            status = Some(response.status());
            if !response.status().is_success() {
                return Err(kugou_http_error(response.status()));
            }
            if response.headers().contains_key("ssa-code") {
                return Err(TuneWeaveError::new(
                    ErrorCode::PermissionDenied,
                    "KuGou video playback requires additional verification",
                )
                .with_platform(Platform::Kugou));
            }
            const LIMIT: usize = 1_048_576;
            let mime = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .map(str::trim);
            // The fixed video tracker serves its JSON envelope as octet-stream.
            // Keep this exception local; the privilege endpoint still requires JSON/plain.
            if !(matches!(mime, Some("application/json" | "text/plain"))
                || (tracker && mime == Some("application/octet-stream")))
                || response.content_length().is_some_and(|v| v > LIMIT as u64)
            {
                return Err(malformed());
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(kugou_network_error)? {
                if bytes.len().saturating_add(chunk.len()) > LIMIT {
                    return Err(malformed());
                }
                bytes.extend_from_slice(&chunk);
            }
            if identity.session.is_some() {
                let envelope = crate::account::media::Status::parse(&bytes)?;
                if matches!(envelope.code(), 20017 | 20018) {
                    return Err(TuneWeaveError::new(
                        ErrorCode::AuthenticationRequired,
                        "KuGou video account session is no longer authorized",
                    )
                    .with_platform(Platform::Kugou)
                    .with_details(json!({"platform_code":envelope.code()})));
                }
            }
            if tracker {
                check_tracker_status(&bytes)?;
            } else {
                super::openapi::check_status(&bytes)?;
            }
            Ok(bytes)
        }
        .await;
        self.log_upstream_request(
            operation,
            "gateway.kugou.com",
            path,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

fn parse_privileges(bytes: &[u8], id: &str, assets: &[Asset]) -> Result<Vec<Privilege>> {
    super::openapi::check_status(bytes)?;
    let p: Privileges = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if p.data.len() != assets.len() {
        return Err(malformed());
    }
    let mut by_hash = BTreeMap::new();
    for p in p.data {
        if !valid_hash(&p.hash)
            || p.status.0 > 1
            || by_hash.insert(p.hash.to_ascii_uppercase(), p).is_some()
        {
            return Err(malformed());
        }
    }
    assets
        .iter()
        .map(|a| {
            let p = by_hash.remove(&a.hash).ok_or_else(malformed)?;
            // The real upstream returns status=1/id=0/size=0 for an unknown hash.
            // A successful status alone must never grant availability.
            if p.id.0.to_string() != id {
                return Err(malformed());
            }
            if let Some(info) = &p.info {
                if a.size.is_some_and(|s| s != info.filesize.0)
                    || a.bitrate.is_some_and(|b| b != info.bitrate.0)
                {
                    return Err(malformed());
                }
            }
            if p.status.0 == 1
                && p.fail_process.0 == 0
                && p.info.as_ref().is_none_or(|i| i.filesize.0 == 0)
            {
                return Err(malformed());
            }
            Ok(p)
        })
        .collect()
}

fn check_tracker_status(bytes: &[u8]) -> Result<Tracker> {
    let t: Tracker = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if t.errcode == Some(40002) || t.error_code == Some(40002) {
        return Err(TuneWeaveError::new(
            ErrorCode::ResourceNotFound,
            "KuGou video resource was not found",
        )
        .with_platform(Platform::Kugou));
    }
    if !matches!(t.status, 0 | 1)
        || t.errcode.is_some_and(|v| v != 0)
        || t.error_code.is_some_and(|v| v != 0)
    {
        return Err(malformed());
    }
    Ok(t)
}
fn parse_tracker(bytes: &[u8], hash: &str, size: u64) -> Result<(i64, Vec<String>)> {
    let t = check_tracker_status(bytes)?;
    let mut privileges = t.privileges.ok_or_else(malformed)?.0;
    if privileges.len() != 1 {
        return Err(malformed());
    }
    let privilege = privileges.remove(hash).ok_or_else(malformed)?.0;
    let code = i64::try_from(privilege).map_err(|_| malformed())?;
    if t.status == 0 || privilege != 0 {
        return Ok((code, Vec::new()));
    }
    let mut data = t.data.ok_or_else(malformed)?.0;
    if data.len() != 1 {
        return Err(malformed());
    }
    let file = data.remove(hash).ok_or_else(malformed)?;
    if file.filesize.as_ref().is_some_and(|n| n.0 != size) {
        return Err(malformed());
    }
    let mut urls = Vec::new();
    for url in file.downurl.into_iter().chain(file.backupdownurl) {
        if url.is_empty() {
            continue;
        }
        if file.filesize.as_ref().is_none_or(|n| n.0 == 0) {
            return Err(malformed());
        }
        validate_url(&url)?;
        if !urls.contains(&url) {
            urls.push(url);
        }
        if urls.len() > 8 {
            return Err(malformed());
        }
    }
    Ok((code, urls))
}
fn validate_url(value: &str) -> Result<()> {
    if value.len() > 8192
        || value
            .bytes()
            .any(|c| c.is_ascii_whitespace() || c.is_ascii_control() || c == b'\\')
    {
        return Err(malformed());
    }
    let u = Url::parse(value).map_err(|_| malformed())?;
    if u.scheme() != "https"
        || !u.username().is_empty()
        || u.password().is_some()
        || u.port().is_some()
        || u.fragment().is_some()
        || !u
            .host_str()
            .is_some_and(|h| h.ends_with(".kugou.com") || h == "kgv.stream.tencentmusic.com")
    {
        return Err(malformed());
    }
    Ok(())
}
fn valid_hash(hash: &str) -> bool {
    hash.len() == 32 && hash.bytes().all(|c| c.is_ascii_hexdigit())
}
fn selection_key(a: &Asset, resolution: u32) -> (u8, u32, u8, std::cmp::Reverse<u64>) {
    let (group, distance) = match a.height {
        Some(h) if h <= resolution => (0, resolution - h),
        Some(h) => (1, h - resolution),
        None => (2, 0),
    };
    let source = match a.source_key.as_str() {
        "ld" | "sd" | "qhd" | "hd" | "fhd" => 0,
        "mkv_sd" | "mkv_qhd" => 1,
        _ => 2,
    };
    (
        group,
        distance,
        source,
        std::cmp::Reverse(a.bitrate.unwrap_or(0)),
    )
}
fn empty_stream(detail: &VideoDetail, resolution: u32) -> VideoStream {
    VideoStream {
        video_ref: detail.video.resource_ref.clone(),
        platform: Platform::Kugou,
        available: false,
        url: None,
        backup_urls: Vec::new(),
        headers: BTreeMap::new(),
        expires_at: None,
        format: None,
        codec: None,
        width: None,
        height: None,
        size: None,
        duration_ms: detail.video.duration_ms,
        source_range: None,
        requested_resolution: resolution,
        actual_resolution: None,
        platform_code: None,
        fee: None,
        message: None,
        extensions: Extensions::from([
            ("backend".into(), json!("anonymous_video_tracker")),
            ("kind".into(), json!(detail.kind)),
        ]),
    }
}
fn malformed() -> TuneWeaveError {
    kugou_upstream_error("KuGou video playback returned inconsistent or invalid resource data")
}

#[cfg(test)]
mod tests;
