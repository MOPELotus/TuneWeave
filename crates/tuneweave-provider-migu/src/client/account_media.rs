use super::account::{AccountData, read_session_body_limited, rotated_token_for_host};
use super::*;
use crate::credential::{authentication_required, error, validate_token};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue, REFERER};

const HOST: &str = "app.c.nf.migu.cn";
const RIGHTS: &str = "/strategy/pc/can-listen/v1.0";
const PLAY: &str = "/strategy/pc/listen/v2.0";
const MAX_RESPONSE: u64 = 1024 * 1024;

#[derive(Deserialize)]
struct Envelope {
    code: String,
    data: Option<serde_json::Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Rights {
    content_id: String,
    pub can_listen: bool,
    pub limit_length: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RightsData {
    can_listen_resp_item_list: Vec<Rights>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Playback {
    #[serde(default)]
    cannot_code: String,
    url: Option<String>,
    audio_format_type: Option<String>,
    auditions_length: Option<FlexibleU64>,
    auditions_start_time: Option<FlexibleU64>,
    song: Option<Song>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Song {
    resource_type: String,
    content_id: String,
    copyright_id: String,
    song_id: Option<String>,
    duration: FlexibleU64,
}

impl MiguClient {
    // Only test builds can redirect these fixed public endpoints to a fixture server.
    pub(super) fn catalog_endpoint(&self, endpoint: &str) -> Result<Url> {
        let url = Url::parse(endpoint).map_err(|_| migu_upstream_error("Invalid Migu endpoint"))?;
        #[cfg(test)]
        let url = self
            .catalog_test_origin
            .as_ref()
            .map_or(Ok(url.clone()), |origin| origin.join(url.path()))
            .map_err(|_| migu_upstream_error("Invalid Migu test endpoint"))?;
        Ok(url)
    }

    async fn account_media_request<T>(
        &self,
        token: &str,
        path: &'static str,
        request: impl FnOnce(reqwest::RequestBuilder) -> reqwest::RequestBuilder,
        parse: impl FnOnce(serde_json::Value) -> Result<T>,
    ) -> Result<AccountData<T>> {
        validate_token(token)?;
        let mut auth = HeaderValue::from_str(token)
            .map_err(|_| migu_invalid_request("Invalid Migu account media header"))?;
        auth.set_sensitive(true);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let url = self.catalog_endpoint(&format!("https://{HOST}{path}"))?;
            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| error(ErrorCode::InternalError, "System clock is unavailable"))?
                .as_millis()
                .to_string();
            let builder = self
                .http
                .request(
                    if path == RIGHTS {
                        reqwest::Method::POST
                    } else {
                        reqwest::Method::GET
                    },
                    url,
                )
                .header(ACCEPT, "application/json")
                .header(CONTENT_TYPE, "application/json")
                .header(REFERER, "https://music.migu.cn/")
                .header("pacmtoken", auth)
                .header("channel", "014X031")
                .header("subchannel", "014X031")
                .header("deviceId", self.music_device.identity()?)
                .header("ua", "Android_migu")
                .header("version", "6.8.8")
                .header("platform", "H5")
                .header("timestamp", timestamp);
            let builder = if path == PLAY {
                builder.header("birth", "h5page").header("signature", "1")
            } else {
                builder
            };
            let response = request(builder).send().await.map_err(|e| {
                error(
                    if e.is_timeout() {
                        ErrorCode::UpstreamTimeout
                    } else {
                        ErrorCode::UpstreamError
                    },
                    "Migu account media request failed",
                )
            })?;
            status = Some(response.status());
            match response.status() {
                StatusCode::UNAUTHORIZED => return Err(authentication_required()),
                StatusCode::FORBIDDEN => {
                    return Err(error(
                        ErrorCode::PermissionDenied,
                        "Migu account media request was denied",
                    ));
                }
                StatusCode::TOO_MANY_REQUESTS => {
                    return Err(error(
                        ErrorCode::RateLimited,
                        "Migu account media request was rate limited",
                    ));
                }
                value if !value.is_success() => {
                    return Err(migu_upstream_error(
                        "Migu account media returned an unsuccessful HTTP status",
                    ));
                }
                _ => {}
            }
            let headers = response.headers().clone();
            let bytes = read_session_body_limited(response, MAX_RESPONSE).await?;
            let data = decode_envelope(&bytes, &headers, path == PLAY)?;
            let token =
                rotated_token_for_host(&headers, HOST, path)?.unwrap_or_else(|| token.to_owned());
            Ok(AccountData {
                token,
                data: parse(data),
            })
        }
        .await;
        let log_result: Result<()> = match result.as_ref() {
            Ok(reply) => reply
                .data
                .as_ref()
                .map(|_| ())
                .map_err(|e| error(e.code, "Migu account media conversion failed")),
            Err(e) => Err(error(e.code, "Migu account media request failed")),
        };
        self.log_upstream_request("account_media", HOST, path, status, started, &log_result);
        result
    }

    pub(crate) async fn account_media_rights(
        &self,
        token: &str,
        id: &str,
    ) -> Result<AccountData<Rights>> {
        self.account_media_request(
            token,
            RIGHTS,
            |request| request.json(&json!({"contentIds":id,"curPlayContentId":""})),
            |value| parse_rights(value, id),
        )
        .await
    }

    pub(crate) async fn account_media_play(
        &self,
        token: &str,
        track: &Track,
        rights: &Rights,
        request: &StreamRequest,
        tone: &'static str,
    ) -> Result<AccountData<MediaStream>> {
        let copyright = track
            .extensions
            .get("copyright_id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| canonical_platform_id(id).is_some())
            .ok_or_else(|| {
                migu_upstream_error("Migu account media omitted its copyright identity")
            })?;
        self.account_media_request(
            token,
            PLAY,
            |builder| {
                builder.query(&[
                    ("contentId", track.id.as_str()),
                    ("copyrightId", copyright),
                    ("resourceType", "2"),
                    ("netType", "01"),
                    ("toneFlag", tone),
                    ("scene", ""),
                ])
            },
            |value| parse_playback(value, track, rights, request, tone, token),
        )
        .await
    }
}

fn decode_envelope(
    bytes: &[u8],
    headers: &HeaderMap,
    encrypted_allowed: bool,
) -> Result<serde_json::Value> {
    let mime = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .map(str::trim);
    if !matches!(
        mime,
        Some("application/json") | Some("application/octet-stream")
    ) {
        return Err(migu_upstream_error(
            "Migu account media returned an unsupported content type",
        ));
    }
    let signatures: Vec<_> = headers.get_all("signature").iter().collect();
    let encrypted = match signatures.as_slice() {
        [] => false,
        [value] if value.as_bytes() == b"1" && encrypted_allowed => true,
        _ => {
            return Err(migu_upstream_error(
                "Migu account media returned an invalid signature header",
            ));
        }
    };
    if !encrypted && mime != Some("application/json") {
        return Err(migu_upstream_error(
            "Migu account media omitted its envelope signature",
        ));
    }
    let decoded;
    let bytes = if encrypted {
        decoded = decrypt_public_stream_response(bytes)?;
        decoded.as_slice()
    } else {
        bytes
    };
    let envelope: Envelope = serde_json::from_slice(bytes)
        .map_err(|_| migu_upstream_error("Migu account media returned an invalid JSON envelope"))?;
    if envelope.code != "000000" {
        // Media business failures do not prove that this account's PACM is invalid.
        return Err(migu_upstream_error(
            "Migu account media authorization failed",
        ));
    }
    envelope
        .data
        .ok_or_else(|| migu_upstream_error("Migu account media omitted data"))
}

fn parse_rights(value: serde_json::Value, id: &str) -> Result<Rights> {
    let mut data: RightsData = serde_json::from_value(value)
        .map_err(|_| migu_upstream_error("Migu account media returned invalid rights"))?;
    if data.can_listen_resp_item_list.len() != 1 {
        return Err(migu_upstream_error(
            "Migu account media returned ambiguous rights",
        ));
    }
    let rights = data.can_listen_resp_item_list.remove(0);
    if rights.content_id != id || (rights.can_listen && rights.limit_length) {
        return Err(migu_upstream_error(
            "Migu account media returned mismatched rights",
        ));
    }
    Ok(rights)
}

pub(crate) fn validate_request(track: &Track, request: &StreamRequest) -> Result<()> {
    canonical_media_track_id(track)?;
    let mut public = request.clone();
    public.account = None;
    select_media_tone(track, &public)?;
    Ok(())
}

pub(crate) fn selected_tones(track: &Track, request: &StreamRequest) -> Result<Vec<&'static str>> {
    let mut public = request.clone();
    public.account = None;
    let first = select_media_tone(track, &public)?.tone_flag;
    if request.quality != Quality::Auto || request.bitrate.is_some() {
        return Ok(vec![first]);
    }
    let tones = [
        ("ZQ24", Quality::Hires),
        ("SQ", Quality::Lossless),
        ("HQ", Quality::High),
        ("PQ", Quality::Standard),
    ];
    Ok(tones
        .into_iter()
        .filter(|(tone, quality)| *tone == "PQ" || track.available_qualities.contains(quality))
        .map(|(tone, _)| tone)
        .collect())
}

fn parse_playback(
    value: serde_json::Value,
    track: &Track,
    rights: &Rights,
    request: &StreamRequest,
    requested_tone: &str,
    token: &str,
) -> Result<MediaStream> {
    let playback: Playback = serde_json::from_value(value)
        .map_err(|_| migu_upstream_error("Migu account media returned invalid playback data"))?;
    if !playback.cannot_code.is_empty() || (!rights.can_listen && !rights.limit_length) {
        return Err(error(
            ErrorCode::PermissionDenied,
            "Migu did not authorize account playback",
        ));
    }
    let song = playback
        .song
        .ok_or_else(|| migu_upstream_error("Migu account media omitted its song identity"))?;
    if song.content_id != track.id
        || rights.content_id != track.id
        || song.resource_type != "2"
        || Some(song.copyright_id.as_str())
            != track
                .extensions
                .get("copyright_id")
                .and_then(serde_json::Value::as_str)
        || song.song_id.as_deref().is_some_and(|id| {
            Some(id)
                != track
                    .extensions
                    .get("song_id")
                    .and_then(serde_json::Value::as_str)
        })
    {
        return Err(migu_upstream_error(
            "Migu account media returned mismatched song identity",
        ));
    }
    let duration_ms = song
        .duration
        .get()
        .filter(|n| *n > 0)
        .and_then(|n| n.checked_mul(1000))
        .ok_or_else(|| migu_upstream_error("Migu account media returned invalid duration"))?;
    if track
        .duration_ms
        .is_some_and(|duration| duration != duration_ms)
    {
        return Err(migu_upstream_error(
            "Migu account media duration changed from resource metadata",
        ));
    }
    let tone = canonical_playback_tone(playback.audio_format_type.as_deref().unwrap_or(""))?;
    let actual_quality = quality_for_migu_tone(tone)?;
    if tone != requested_tone {
        return Err(error(
            ErrorCode::PermissionDenied,
            "Migu did not authorize the requested audio format",
        ));
    }
    let raw_url = playback
        .url
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| migu_upstream_error("Migu account media omitted its authorized URL"))?;
    let url = validate_public_audio_url(raw_url)?;
    let parsed_url =
        Url::parse(&url).map_err(|_| migu_upstream_error("Migu account media URL is invalid"))?;
    for key in ["Tim", "Key", "playSessionId"] {
        if parsed_url
            .query_pairs()
            .filter(|(name, _)| name == key)
            .count()
            != 1
        {
            return Err(migu_upstream_error(
                "Migu account media URL contains ambiguous authorization",
            ));
        }
    }
    if playback
        .auditions_start_time
        .as_ref()
        .is_some_and(|value| value.get().is_none())
        || playback
            .auditions_length
            .as_ref()
            .is_some_and(|value| value.get().is_none())
    {
        return Err(migu_upstream_error(
            "Migu account media returned invalid preview timing",
        ));
    }
    reject_url_secret(&url, token)?;
    let (format, codec, bitrate) = media_spec_from_url(&url, tone)?;
    let trial = playback_trial(
        &MiguListeningRights {
            content_id: rights.content_id.clone(),
            can_listen: rights.can_listen,
            limit_length: rights.limit_length,
        },
        &MiguPlaybackData {
            auditions_start_time: playback.auditions_start_time,
            auditions_length: playback.auditions_length,
            ..Default::default()
        },
    )?;
    if trial
        .as_ref()
        .is_some_and(|window| window.end_ms > duration_ms)
    {
        return Err(migu_upstream_error(
            "Migu account media preview exceeds the original song duration",
        ));
    }
    Ok(MediaStream {
        url,
        backup_urls: Vec::new(),
        headers: BTreeMap::new(),
        expires_at: None,
        format,
        codec,
        bitrate,
        size: None,
        duration_ms: Some(duration_ms),
        requested_quality: request.quality,
        actual_quality,
        trial,
        origin_track: Some(track.resource_ref.clone()),
        resolved_track: track.resource_ref.clone(),
        resolved_platform: Platform::Migu,
        match_score: Some(1.0),
        attempts: Vec::new(),
    })
}

pub(crate) fn reject_url_secret(value: &str, token: &str) -> Result<()> {
    let url =
        Url::parse(value).map_err(|_| migu_upstream_error("Migu account media URL is invalid"))?;
    let path = format!("path={}", url.path());
    if value.contains(token)
        || url
            .query_pairs()
            .any(|(key, value)| key.contains(token) || value.contains(token))
        || url::form_urlencoded::parse(path.as_bytes()).any(|(_, path)| path.contains(token))
    {
        return Err(migu_upstream_error(
            "Migu account media URL exposed session data",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
