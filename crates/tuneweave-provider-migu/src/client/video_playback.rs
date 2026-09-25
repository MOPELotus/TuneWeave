use super::account::{AccountData, read_session_body_limited, rotated_token_for_host};
use super::*;
use crate::credential::{authentication_required, validate_token};
use reqwest::header::{CONTENT_TYPE, HeaderValue};
use serde_json::Value;
use tuneweave_core::{
    MiguNativeMvFormat, VideoResourceKind, VideoSourceRange, VideoStream, VideoStreamRequest,
};

const HOST: &str = "app.c.nf.migu.cn";
const PATH: &str = "/MIGUM2.0/v1.0/content/mvplayinfo.do";
const MAX_RESPONSE: u64 = 1024 * 1024;
const MAX_MANIFEST: u64 = 256 * 1024;
const NATIVE_MV_HOST: &str = "freevod.nf.migu.cn";

pub(crate) struct Source {
    pub id: String,
    pub copyright: String,
    pub duration_ms: u64,
    pub level: String,
    format: String,
    catalogue_url: String,
    size: String,
}

pub(crate) struct NativeGrant {
    pub(crate) url: String,
    format: String,
    offset_ms: u64,
}
pub(crate) fn validate(id: &str, request: &VideoStreamRequest) -> Result<()> {
    if !super::videos::valid_id(id)
        || request.kind != VideoResourceKind::Mv
        || !matches!(request.resolution, 0 | 1080)
    {
        return Err(migu_invalid_request(
            "Migu MV playback supports canonical MV IDs and resolution=auto (SDK 0) or 1080",
        ));
    }
    Ok(())
}
fn invalid() -> TuneWeaveError {
    migu_upstream_error("Migu MV playback response is invalid")
}
fn denied() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::PermissionDenied,
        "Migu MV playback was not authorized",
    )
    .with_platform(Platform::Migu)
}
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key).and_then(Value::as_str).ok_or_else(invalid)
}
fn parse_source(root: Value, id: &str, request: &VideoStreamRequest) -> Result<Source> {
    let detail = super::videos::parse_detail(root.clone(), id)?.detail;
    let raw = &root["resource"][0];
    if string(raw, "copyright")? != "1" {
        return Err(denied());
    }
    let copyright = raw
        .get("copyrightId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .or_else(|| raw.get("mvCopyrightId").and_then(Value::as_str))
        .filter(|s| !s.is_empty() && s.len() <= 64 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
        .ok_or_else(invalid)?;
    let duration_ms = detail
        .video
        .duration_ms
        .filter(|n| *n > 0)
        .ok_or_else(invalid)?;
    let formats = raw
        .get("rateFormats")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let mut by_level = BTreeMap::new();
    for format in formats {
        let level = string(format, "formatType")?;
        if by_level.insert(level, format).is_some() {
            return Err(invalid());
        }
    }
    let levels: &[&str] = if request.resolution == 0 {
        &["PQ", "HQ", "SQ"]
    } else {
        &["SQ"]
    };
    let (level, format) = levels
        .iter()
        .find_map(|level| by_level.get(level).map(|f| (*level, *f)))
        .ok_or_else(denied)?;
    let selector = string(format, "format")?;
    let url = string(format, "url")?;
    let size = string(format, "size")?;
    if selector.is_empty()
        || selector.len() > 32
        || !selector.bytes().all(|b| b.is_ascii_digit())
        || url.is_empty()
        || url.len() > 16384
        || url.chars().any(char::is_control)
        || size.is_empty()
        || size.len() > 20
        || !size.bytes().all(|b| b.is_ascii_digit())
        || !size.parse::<u64>().is_ok_and(|n| n > 0)
    {
        return Err(invalid());
    }
    Ok(Source {
        id: id.into(),
        copyright: copyright.into(),
        duration_ms,
        level: level.into(),
        format: selector.into(),
        catalogue_url: url.into(),
        size: size.into(),
    })
}
fn media_url(value: &str, id: &str) -> Result<Url> {
    if value.len() > 16384 || value.chars().any(char::is_control) {
        return Err(invalid());
    }
    let u = Url::parse(value).map_err(|_| invalid())?;
    if u.scheme() != "https"
        || u.host_str() != Some("freevod.nf.migu.cn")
        || u.port().is_some()
        || !u.username().is_empty()
        || u.password().is_some()
        || u.fragment().is_some()
        || !u.path().ends_with("/index.m3u8")
        || u.path().contains("%2f")
        || u.path().contains("%2F")
    {
        return Err(invalid());
    }
    let mut params = BTreeMap::new();
    for (k, v) in u.query_pairs() {
        if k.len() > 128
            || v.len() > 4096
            || params.insert(k.into_owned(), v.into_owned()).is_some()
        {
            return Err(invalid());
        }
    }
    if params.get("resourceId").map(String::as_str) != Some(id)
        || params.get("resourceType").map(String::as_str) != Some("D")
        || !params.get("playSessionId").is_some_and(|s| !s.is_empty())
    {
        return Err(invalid());
    }
    Ok(u)
}
fn parse_grant(root: Value, source: &Source) -> Result<String> {
    match root.get("code").and_then(Value::as_str) {
        Some("000000") => {}
        Some("000001" | "200004") => return Err(denied()),
        _ => return Err(invalid()),
    }
    for key in ["cannotType", "cannotCode"] {
        if root
            .get(key)
            .is_some_and(|v| !v.is_null() && v.as_str() != Some(""))
        {
            return Err(denied());
        }
    }
    if root.get("offset").is_some_and(|v| v.as_u64() != Some(0)) {
        return Err(invalid());
    }
    media_url(string(&root, "playUrl")?, &source.id).map(String::from)
}

fn native_media_url(value: &str, id: &str) -> Result<Url> {
    if value.len() > 16384 || value.chars().any(char::is_control) {
        return Err(invalid());
    }
    let url = Url::parse(value).map_err(|_| invalid())?;
    if url.scheme() != "http"
        || url.host_str() != Some(NATIVE_MV_HOST)
        || url.port() != Some(8080)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || url.path_segments().is_none_or(|mut segments| {
            segments.next() != Some("hls")
                || segments.next() != Some("v2")
                || segments.next().is_none_or(str::is_empty)
                || segments.next() != Some("index.m3u8")
                || segments.next().is_some()
        })
    {
        return Err(invalid());
    }
    let mut params = BTreeMap::new();
    for (key, value) in url.query_pairs() {
        if key.len() > 128
            || value.len() > 4096
            || params
                .insert(key.into_owned(), value.into_owned())
                .is_some()
        {
            return Err(invalid());
        }
    }
    if params.get("resourceId").map(String::as_str) != Some(id)
        || params.get("resourceType").map(String::as_str) != Some("D")
        || !params
            .get("playSessionId")
            .is_some_and(|value| !value.is_empty())
    {
        return Err(invalid());
    }
    Ok(url)
}

fn parse_native_grant(
    root: Value,
    id: &str,
    catalogue_duration_ms: u64,
    requested_format: MiguNativeMvFormat,
) -> Result<NativeGrant> {
    match root.get("code").and_then(Value::as_str) {
        Some("000000") => {}
        Some("000001" | "200004") => return Err(denied()),
        _ => return Err(invalid()),
    }
    let data = root.get("data").ok_or_else(invalid)?;
    for key in ["cannotType", "cannotCode"] {
        if data
            .get(key)
            .is_some_and(|value| !value.is_null() && value.as_str() != Some(""))
        {
            return Err(denied());
        }
    }
    let format = string(data, "formatType")?;
    if !matches!(format, "PQ" | "HQ" | "SQ") {
        return Err(invalid());
    }
    let expected = match requested_format {
        MiguNativeMvFormat::Auto => None,
        MiguNativeMvFormat::Pq => Some("PQ"),
        MiguNativeMvFormat::Hq => Some("HQ"),
        MiguNativeMvFormat::Sq => Some("SQ"),
    };
    if expected.is_some_and(|expected| expected != format) {
        return Err(invalid());
    }
    let offset_ms = data
        .get("offset")
        .and_then(Value::as_u64)
        .ok_or_else(invalid)?;
    if offset_ms >= catalogue_duration_ms {
        return Err(denied());
    }
    match data.get("backgroundDuration").and_then(Value::as_u64) {
        Some(0) => {}
        Some(_) => {
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "Migu native MV returned a background playback limit",
            )
            .with_platform(Platform::Migu));
        }
        None => return Err(invalid()),
    }
    let url = string(data, "playUrl")?;
    native_media_url(url, id)?;
    Ok(NativeGrant {
        url: url.to_owned(),
        format: format.to_owned(),
        offset_ms,
    })
}
fn segment_duration(value: &str) -> Result<u64> {
    let mut parts = value.split('.');
    let whole = parts.next().ok_or_else(invalid)?;
    let fraction = parts.next().unwrap_or("");
    if whole.is_empty()
        || whole.len() > 8
        || !whole.bytes().all(|b| b.is_ascii_digit())
        || fraction.len() > 6
        || !fraction.bytes().all(|b| b.is_ascii_digit())
        || parts.next().is_some()
    {
        return Err(invalid());
    }
    let seconds = whole.parse::<u64>().map_err(|_| invalid())?;
    let fraction = if fraction.is_empty() {
        0
    } else {
        fraction.parse::<u64>().map_err(|_| invalid())? * 10u64.pow(6 - fraction.len() as u32)
    };
    seconds
        .checked_mul(1_000_000)
        .and_then(|n| n.checked_add(fraction))
        .ok_or_else(invalid)
}
pub(crate) fn manifest_duration(bytes: &[u8], source_duration_ms: u64) -> Result<u64> {
    let text = std::str::from_utf8(bytes).map_err(|_| invalid())?;
    let mut lines = text.lines();
    if lines.next() != Some("#EXTM3U") {
        return Err(invalid());
    }
    let mut pending = None;
    let mut total = 0u64;
    let mut segments = 0;
    let mut ended = false;
    let mut zero = false;
    let mut target = false;
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if ended || zero && line != "#EXT-X-ENDLIST" {
            return Err(invalid());
        }
        if let Some(v) = line.strip_prefix("#EXTINF:") {
            if pending.is_some() {
                return Err(invalid());
            }
            let (duration, _) = v.split_once(',').ok_or_else(invalid)?;
            pending = Some(segment_duration(duration)?);
        } else if let Some(v) = line.strip_prefix("#EXT-X-TARGETDURATION:") {
            if target || !v.parse::<u32>().is_ok_and(|n| (1..=120).contains(&n)) {
                return Err(invalid());
            }
            target = true;
        } else if line == "#EXT-X-ENDLIST" {
            if pending.is_some() {
                return Err(invalid());
            }
            ended = true;
        } else if line == "#EXT-X-PLAYLIST-TYPE:VOD"
            || line == "#EXT-X-INDEPENDENT-SEGMENTS"
            || line == "#EXT-X-MEDIA-SEQUENCE:0"
        {
        } else if let Some(v) = line.strip_prefix("#EXT-X-VERSION:") {
            if !v.parse::<u32>().is_ok_and(|n| (1..=10).contains(&n)) {
                return Err(invalid());
            }
        } else if line.starts_with('#') {
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "Migu MV manifest contains an unsupported playback feature",
            )
            .with_platform(Platform::Migu));
        } else {
            let duration = pending.take().ok_or_else(invalid)?;
            if line.len() > 256
                || !line.ends_with(".ts")
                || !line
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
                || line.starts_with('.')
            {
                return Err(invalid());
            }
            segments += 1;
            if segments > 4096 {
                return Err(invalid());
            }
            total = total.checked_add(duration).ok_or_else(invalid)?;
            zero = duration == 0;
        }
    }
    if !ended
        || !target
        || pending.is_some()
        || segments == 0
        || total == 0
        || total > 24 * 3600 * 1_000_000
    {
        return Err(invalid());
    }
    let duration = total / 1000;
    if duration.saturating_add(1500) < source_duration_ms {
        return Err(TuneWeaveError::new(
            ErrorCode::PermissionDenied,
            "Migu MV returned a shortened playlist; full playback is unconfirmed",
        )
        .with_platform(Platform::Migu));
    }
    Ok(duration)
}
impl MiguClient {
    pub(crate) async fn native_mv_stream(
        &self,
        id: &str,
        requested_format: MiguNativeMvFormat,
    ) -> Result<VideoStream> {
        let catalogue_duration_ms = self.native_mv_duration(id).await?;
        let grant = self
            .native_mv_authorization(id, catalogue_duration_ms, requested_format, None)
            .await?;
        self.native_mv_manifest(grant, id, catalogue_duration_ms, requested_format)
            .await
    }

    pub(crate) async fn native_mv_duration(&self, id: &str) -> Result<u64> {
        let mut budget = 2 * 1024 * 1024;
        let detail = self.mv_detail(id, &mut budget).await?;
        detail
            .detail
            .video
            .duration_ms
            .filter(|duration| *duration > 0)
            .ok_or_else(invalid)
    }

    pub(crate) async fn native_mv_authorization(
        &self,
        id: &str,
        catalogue_duration_ms: u64,
        requested_format: MiguNativeMvFormat,
        auth: Option<&super::account_download::NativeAuthorization>,
    ) -> Result<NativeGrant> {
        let root = self.native_mv_grant(id, requested_format, auth).await?;
        parse_native_grant(root, id, catalogue_duration_ms, requested_format)
    }

    pub(crate) async fn mv_play_source(
        &self,
        id: &str,
        request: &VideoStreamRequest,
    ) -> Result<Source> {
        let mut budget = 2 * 1024 * 1024;
        let root = self
            .mv_response(
                "c.musicapp.migu.cn",
                "/MIGUM2.0/v1.0/content/resourceinfo.do",
                &[
                    ("resourceId", id.into()),
                    ("resourceType", "D".into()),
                    ("needSimple", "01".into()),
                ],
                &mut budget,
            )
            .await?;
        parse_source(root, id, request)
    }
    pub(crate) async fn mv_play_grant(
        &self,
        token: Option<&str>,
        source: &Source,
    ) -> Result<AccountData<String>> {
        let mut builder = self
            .http
            .get(self.catalog_endpoint(&format!("https://{HOST}{PATH}"))?)
            .header(ACCEPT, "application/json")
            .header("channel", "014X031")
            .header("subchannel", "014X031")
            .header("platform", "H5")
            .header("version", "6.8.8");
        if let Some(token) = token {
            validate_token(token)?;
            let mut value = HeaderValue::from_str(token)
                .map_err(|_| migu_invalid_request("Invalid Migu MV credential"))?;
            value.set_sensitive(true);
            builder = builder
                .header("pacmtoken", value)
                .header("deviceId", self.music_device.identity()?)
                .header("ua", "Android_migu");
        }
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = builder
                .query(&[
                    ("mvContentId", source.id.as_str()),
                    ("mvCopyrightId", source.copyright.as_str()),
                    ("format", source.format.as_str()),
                    ("url", source.catalogue_url.as_str()),
                    ("size", source.size.as_str()),
                    ("needHttps", "true"),
                ])
                .send()
                .await
                .map_err(migu_network_error)?;
            status = Some(response.status());
            match response.status() {
                StatusCode::UNAUTHORIZED => return Err(authentication_required()),
                StatusCode::FORBIDDEN => return Err(denied()),
                s if !s.is_success() => return Err(migu_http_error(s)),
                _ => {}
            }
            if !response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
            {
                return Err(invalid());
            }
            let headers = response.headers().clone();
            let body = read_session_body_limited(response, MAX_RESPONSE).await?;
            let root = serde_json::from_slice(&body).map_err(|_| invalid())?;
            let next = if let Some(token) = token {
                rotated_token_for_host(&headers, HOST, PATH)?.unwrap_or_else(|| token.into())
            } else {
                String::new()
            };
            Ok(AccountData {
                token: next,
                data: parse_grant(root, source),
            })
        }
        .await;
        let log: Result<()> = match &result {
            Ok(v) => v
                .data
                .as_ref()
                .map(|_| ())
                .map_err(|e| TuneWeaveError::new(e.code, "Migu MV grant failed")),
            Err(e) => Err(TuneWeaveError::new(e.code, "Migu MV request failed")),
        };
        self.log_upstream_request("mv_play_grant", HOST, PATH, status, started, &log);
        result
    }
    pub(crate) async fn mv_manifest(
        &self,
        url: &str,
        source: &Source,
        request: &VideoStreamRequest,
    ) -> Result<VideoStream> {
        let original = media_url(url, &source.id)?;
        #[cfg(test)]
        let target = if let Some(origin) = &self.catalog_test_origin {
            let mut u = origin.join(original.path()).map_err(|_| invalid())?;
            u.set_query(original.query());
            u
        } else {
            original.clone()
        };
        #[cfg(not(test))]
        let target = original.clone();
        let response = self
            .http
            .get(target)
            .header(
                ACCEPT,
                "application/vnd.apple.mpegurl, application/x-mpegURL, audio/mpegurl",
            )
            .send()
            .await
            .map_err(migu_network_error)?;
        if !response.status().is_success() {
            return Err(migu_http_error(response.status()));
        }
        let mime = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(';').next())
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if ![
            "application/vnd.apple.mpegurl",
            "application/x-mpegurl",
            "audio/mpegurl",
            "audio/x-mpegurl",
        ]
        .contains(&mime.as_str())
        {
            return Err(invalid());
        }
        let bytes =
            read_bounded_response_with_limit(response, "Migu MV manifest", MAX_MANIFEST).await?;
        let duration = manifest_duration(&bytes, source.duration_ms)?;
        Ok(VideoStream {
            video_ref: ResourceRef::new(Platform::Migu, &source.id).map_err(|_| invalid())?,
            platform: Platform::Migu,
            available: true,
            url: Some(original.into()),
            backup_urls: vec![],
            headers: BTreeMap::new(),
            expires_at: None,
            format: Some("hls".into()),
            codec: None,
            width: None,
            height: None,
            size: None,
            duration_ms: Some(duration),
            source_range: None,
            requested_resolution: request.resolution,
            actual_resolution: None,
            platform_code: Some(0),
            fee: None,
            message: None,
            extensions: Extensions::from([
                ("backend".into(), json!("official_pc_mv_play_v1")),
                ("format_type".into(), json!(source.level)),
                ("catalogue_duration_ms".into(), json!(source.duration_ms)),
                ("playlist_duration_ms".into(), json!(duration)),
                ("manifest_validated".into(), json!(true)),
            ]),
        })
    }

    pub(crate) async fn native_mv_manifest(
        &self,
        grant: NativeGrant,
        id: &str,
        catalogue_duration_ms: u64,
        requested_format: MiguNativeMvFormat,
    ) -> Result<VideoStream> {
        let original = native_media_url(&grant.url, id)?;
        #[cfg(test)]
        let target = if let Some(origin) = &self.catalog_test_origin {
            let mut url = origin.join(original.path()).map_err(|_| invalid())?;
            url.set_query(original.query());
            url
        } else {
            original.clone()
        };
        #[cfg(not(test))]
        let target = original.clone();
        let response = self
            .http
            .get(target)
            .header(
                ACCEPT,
                "application/vnd.apple.mpegurl, application/x-mpegURL, audio/mpegurl",
            )
            .send()
            .await
            .map_err(migu_network_error)?;
        if !response.status().is_success() {
            return Err(migu_http_error(response.status()));
        }
        let mime = response
            .headers()
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if ![
            "application/vnd.apple.mpegurl",
            "application/x-mpegurl",
            "audio/mpegurl",
            "audio/x-mpegurl",
        ]
        .contains(&mime.as_str())
        {
            return Err(invalid());
        }
        let bytes =
            read_bounded_response_with_limit(response, "Migu native MV manifest", MAX_MANIFEST)
                .await?;
        let manifest_ms = manifest_duration(&bytes, catalogue_duration_ms)?;
        let source_range = VideoSourceRange {
            start_ms: grant.offset_ms,
            end_ms: catalogue_duration_ms,
        };
        let duration_ms = source_range.end_ms - source_range.start_ms;
        Ok(VideoStream {
            video_ref: ResourceRef::new(Platform::Migu, id).map_err(|_| invalid())?,
            platform: Platform::Migu,
            available: true,
            url: Some(original.into()),
            backup_urls: Vec::new(),
            headers: BTreeMap::new(),
            expires_at: None,
            format: Some("hls".into()),
            codec: None,
            width: None,
            height: None,
            size: None,
            duration_ms: Some(duration_ms),
            source_range: Some(source_range),
            requested_resolution: 0,
            actual_resolution: None,
            platform_code: Some(0),
            fee: None,
            message: None,
            extensions: Extensions::from([
                ("backend".into(), json!("official_native_mv_v1_1")),
                ("requested_format".into(), json!(requested_format)),
                ("actual_format".into(), json!(grant.format)),
                (
                    "format_fallback_allowed".into(),
                    json!(requested_format == MiguNativeMvFormat::Auto),
                ),
                ("catalogue_duration_ms".into(), json!(catalogue_duration_ms)),
                ("manifest_duration_ms".into(), json!(manifest_ms)),
                ("background_duration_ms".into(), json!(0)),
                ("authorization_scope".into(), json!("anonymous_device")),
            ]),
        })
    }
}

#[cfg(test)]
pub(crate) mod tests;
