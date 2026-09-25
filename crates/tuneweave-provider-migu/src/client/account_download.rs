use super::account_media::reject_url_secret;
use super::*;
use crate::credential::{error, validate_uid};

const VALIDATE_HOST: &str = "app.u.nf.migu.cn";
const VALIDATE_PATH: &str = "/user/token-validate/v2.0";
const DOWNLOAD_HOST: &str = "app.c.nf.migu.cn";
const DOWNLOAD_PATH: &str = "/MIGUM2.0/strategy/download-url/by-songid/v1.0";

pub(crate) struct DownloadContentGrant {
    pub(crate) media: MediaDownload,
    // Official DownloadTaskRunnable preserves the ordinary suffix for an empty
    // fileKey; only a nonempty fileKey selects the MG3D byte transform.
    pub(crate) key: Option<[u8; 32]>,
}

pub(crate) struct NativeAuthorization {
    pub(super) token: String,
    pub(super) uid: String,
}
impl fmt::Debug for NativeAuthorization {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NativeAuthorization")
            .finish_non_exhaustive()
    }
}

impl MiguClient {
    pub(crate) async fn validate_native_token(
        &self,
        token: String,
        uid: &str,
    ) -> Result<NativeAuthorization> {
        validate_uid(uid)?;
        let value = self
            .native_get(
                VALIDATE_HOST,
                VALIDATE_PATH,
                &token,
                None,
                vec![
                    ("tokenId", &token),
                    ("token", &token),
                    ("sourceId", "220024"),
                    ("loginType", "1"),
                ],
                true,
            )
            .await?;
        let data = success_data(value)?;
        let actual = data
            .get("userInfoItem")
            .and_then(|value| value.get("userId"))
            .and_then(serde_json::Value::as_str)
            .filter(|value| validate_uid(value).is_ok())
            .ok_or_else(|| {
                migu_upstream_error("Migu native validation omitted a valid account identity")
            })?;
        if actual != uid {
            return Err(error(
                ErrorCode::PermissionDenied,
                "Migu native token belongs to a different account",
            ));
        }
        Ok(NativeAuthorization {
            token,
            uid: uid.to_owned(),
        })
    }

    pub(crate) async fn account_download(
        &self,
        auth: &NativeAuthorization,
        track: &Track,
        request: &StreamRequest,
        tone: &str,
    ) -> Result<MediaDownload> {
        let value = self.request_native_download(auth, track, tone).await?;
        let result = parse_download(value, track, request, tone)?;
        if let Some(url) = result.url.as_deref() {
            reject_url_secret(url, &auth.token)?;
        }
        Ok(result)
    }

    pub(crate) async fn account_download_content(
        &self,
        auth: &NativeAuthorization,
        track: &Track,
        request: &StreamRequest,
        tone: &str,
    ) -> Result<DownloadContentGrant> {
        let value = self.request_native_download(auth, track, tone).await?;
        let grant = parse_download_content(value, track, request, tone)?;
        if let Some(url) = grant.media.url.as_deref() {
            reject_url_secret(url, &auth.token)?;
            if let Some(key) = &grant.key {
                reject_url_secret(url, std::str::from_utf8(key).expect("validated ASCII key"))?;
            }
        }
        Ok(grant)
    }

    async fn request_native_download(
        &self,
        auth: &NativeAuthorization,
        track: &Track,
        tone: &str,
    ) -> Result<serde_json::Value> {
        let song_id = track
            .extensions
            .get("song_id")
            .and_then(serde_json::Value::as_str)
            .filter(|value| canonical_platform_id(value).is_some())
            .ok_or_else(|| {
                migu_upstream_error("Migu download metadata omitted its song identity")
            })?;
        let value = self
            .native_get(
                DOWNLOAD_HOST,
                DOWNLOAD_PATH,
                &auth.token,
                Some(&auth.uid),
                vec![
                    ("songId", song_id),
                    ("formatType", tone),
                    ("contentId", &track.id),
                ],
                false,
            )
            .await?;
        success_data(value)
    }
}

fn success_data(value: serde_json::Value) -> Result<serde_json::Value> {
    #[derive(Deserialize)]
    struct Envelope {
        code: String,
        data: Option<serde_json::Value>,
    }
    let body: Envelope = serde_json::from_value(value)
        .map_err(|_| migu_upstream_error("Migu native authorization envelope is invalid"))?;
    if body.code != "000000" {
        let code = if matches!(
            body.code.as_str(),
            "200000" | "200010" | "200013" | "200004" | "220000" | "290001"
        ) {
            ErrorCode::PermissionDenied
        } else {
            ErrorCode::UpstreamError
        };
        let mut failure = error(code, "Migu native authorization was not granted");
        if body.code.len() == 6 && body.code.bytes().all(|byte| byte.is_ascii_digit()) {
            failure = failure.with_details(json!({"platform_code":body.code}));
        }
        return Err(failure);
    }
    body.data
        .ok_or_else(|| migu_upstream_error("Migu native authorization omitted data"))
}

fn parse_download(
    value: serde_json::Value,
    track: &Track,
    request: &StreamRequest,
    tone: &str,
) -> Result<MediaDownload> {
    parse_grant(value, track, request, tone, false).map(|(media, _)| media)
}

fn parse_download_content(
    value: serde_json::Value,
    track: &Track,
    request: &StreamRequest,
    tone: &str,
) -> Result<DownloadContentGrant> {
    if !matches!(tone, "PQ" | "HQ" | "SQ" | "ZQ24") {
        return Err(error(
            ErrorCode::CapabilityNotSupported,
            "Migu download content supports PQ/HQ MP3 and SQ/ZQ24 FLAC renditions only",
        ));
    }
    let (media, key) = parse_grant(value, track, request, tone, true)?;
    Ok(DownloadContentGrant { media, key })
}

fn parse_grant(
    value: serde_json::Value,
    track: &Track,
    request: &StreamRequest,
    tone: &str,
    allow_mg3d: bool,
) -> Result<(MediaDownload, Option<[u8; 32]>)> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Download {
        content_id: String,
        copyright_id: String,
        song_id: Option<String>,
        encryption_type: String,
        file_key: Option<String>,
        format_id: String,
        size: FlexibleU64,
        suffix: String,
        url: String,
        auditions_length: Option<FlexibleU64>,
        auditions_start_time: Option<FlexibleU64>,
    }
    let data: Download = serde_json::from_value(value)
        .map_err(|_| migu_upstream_error("Migu download authorization data is invalid"))?;
    if data.content_id != track.id
        || Some(data.copyright_id.as_str())
            != track
                .extensions
                .get("copyright_id")
                .and_then(serde_json::Value::as_str)
        || data.song_id.as_deref().is_some_and(|value| {
            Some(value)
                != track
                    .extensions
                    .get("song_id")
                    .and_then(serde_json::Value::as_str)
        })
    {
        return Err(migu_upstream_error(
            "Migu download returned conflicting resource identity",
        ));
    }
    let has_file_key = data
        .file_key
        .as_deref()
        .is_some_and(|value| !value.is_empty());
    // The official downloader chooses MG3D from fileKey before applying the
    // encryption policy. MusicEncryptionUtils explicitly excludes MG3D from
    // additional local MGM wrapping even when encryptionType is "1". Only
    // the content API can consume that keyed branch; keyless MGM stays denied.
    let supported_encryption =
        data.encryption_type == "0" || (allow_mg3d && has_file_key && data.encryption_type == "1");
    if !supported_encryption
        || (!allow_mg3d && has_file_key)
        || data.auditions_length.is_some()
        || data.auditions_start_time.is_some()
    {
        return Err(error(
            ErrorCode::PermissionDenied,
            "Migu did not authorize a supported full download",
        ));
    }
    let key = if let Some(key) = data.file_key.as_deref().filter(|key| !key.is_empty()) {
        if key.len() != 32 || !key.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(error(
                ErrorCode::CapabilityNotSupported,
                "Migu MG3D content requires a supported file key",
            ));
        }
        Some(key.as_bytes().try_into().expect("validated key length"))
    } else {
        None
    };
    let mut actual = None;
    for field in ["rate_formats", "new_rate_formats"] {
        if let Some(value) = track.extensions.get(field) {
            let formats: Vec<MiguRateFormat> = serde_json::from_value(value.clone())
                .map_err(|_| migu_upstream_error("Migu download metadata formats are invalid"))?;
            for format in formats {
                if !data.format_id.is_empty()
                    && [&format.format, &format.android_format, &format.ios_format]
                        .contains(&&data.format_id)
                {
                    let found = canonical_playback_tone(&format.format_type)?;
                    if actual.is_some_and(|previous| previous != found) {
                        return Err(migu_upstream_error(
                            "Migu download format identity is ambiguous",
                        ));
                    }
                    actual = Some(found);
                }
            }
        }
    }
    let actual = actual.ok_or_else(|| {
        migu_upstream_error("Migu download format does not match resource metadata")
    })?;
    if actual != tone {
        return Err(error(
            ErrorCode::PermissionDenied,
            "Migu did not authorize the requested download format",
        ));
    }
    let (suffix, bitrate) = match actual {
        "PQ" => ("mp3", Some(128_000)),
        "HQ" => ("mp3", Some(320_000)),
        _ => ("flac", None),
    };
    if data.suffix != suffix {
        return Err(migu_upstream_error(
            "Migu download suffix contradicts its format",
        ));
    }
    let size = data
        .size
        .get()
        .filter(|size| *size > 0)
        .ok_or_else(|| migu_upstream_error("Migu download omitted a valid file size"))?;
    let url = validate_download_url(&data.url)?;
    Ok((
        MediaDownload {
            track_ref: track.resource_ref.clone(),
            platform: Platform::Migu,
            available: true,
            url: Some(url),
            headers: BTreeMap::new(),
            expires_at: None,
            format: Some(suffix.into()),
            codec: Some(suffix.into()),
            bitrate,
            size: Some(size),
            duration_ms: track.duration_ms,
            requested_quality: request.quality,
            actual_quality: quality_for_migu_tone(actual)?,
            platform_code: Some(0),
            fee: None,
            message: None,
            extensions: Extensions::from([
                (
                    "backend".into(),
                    json!("native_account_download_by_songid_v1"),
                ),
                ("format_id".into(), json!(data.format_id)),
                ("preview_url_withheld".into(), json!(false)),
            ]),
        },
        key,
    ))
}

pub(super) fn validate_download_url(value: &str) -> Result<String> {
    // Conservative supported CDN subset. Unknown account CDN responses require
    // separate evidence; never weaken this boundary by using a playback URL.
    let validated =
        validate_other_media_url(value, "dlsdownfree.nf.migu.cn", "/wlansst", &["pars"])?;
    let url =
        Url::parse(&validated).map_err(|_| migu_upstream_error("Migu download URL is invalid"))?;
    if url.scheme() != "https"
        || !(url.path() == "/wlansst" || url.path().starts_with("/wlansst/"))
        || url.query_pairs().filter(|(name, _)| name == "pars").count() != 1
    {
        return Err(migu_upstream_error(
            "Migu download URL is outside the supported authorization boundary",
        ));
    }
    Ok(validated)
}

#[cfg(test)]
mod tests;
