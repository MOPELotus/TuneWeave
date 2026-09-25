//! Official browser song information. Login cookies never accompany catalogue or CDN requests.
use super::{dto::Number, *};
use crate::web::WebSession;

mod legacy_library;

const HOST: &str = "wwwapi.kugou.com";
const PATH: &str = "/play/songinfo";

impl KugouClient {
    pub(crate) async fn web_media_stream(
        &self,
        session: &WebSession,
        track: Track,
        request: &StreamRequest,
        check: impl FnMut() -> Result<()>,
    ) -> Result<MediaStream> {
        account_media::validate_request(request)?;
        let (data, spec, token) = self.verified_web_song_info(session, &track, check).await?;
        map(data, track, spec, request.quality, &token)
    }

    pub(crate) async fn web_lyrics(
        &self,
        session: &WebSession,
        track: Track,
        check: impl FnMut() -> Result<()>,
    ) -> Result<Lyrics> {
        let (data, _, token) = self.verified_web_song_info(session, &track, check).await?;
        map_lyrics(data, track, &token)
    }

    async fn verified_web_song_info(
        &self,
        session: &WebSession,
        track: &Track,
        mut check: impl FnMut() -> Result<()>,
    ) -> Result<(Value, SelectedMediaSpec, String)> {
        let id = account_media::validate_track(track)?;
        let token = session.media_token()?;
        // The browser API has no quality selector. Resolve the canonical resource;
        // report its returned bitrate rather than asserting the requested tier.
        let spec = select_media_spec(track, &StreamRequest::default())?;
        let album_id = canonical_album_id(track)?;
        let mut query = BTreeMap::from([
            ("hash", spec.hash.clone()),
            ("album_audio_id", id.to_string()),
        ]);
        if album_id != 0 {
            query.insert("album_id", album_id.to_string());
        }
        let response = self.web_song_info(session, &token, query).await;
        check()?;
        let mut data = response?;
        let first = identity(&data, track, &spec, album_id, &session.user_id)?;
        // The official player prefers an encoded identity. Only accept one bound
        // by this response to the requested mixsong ID, never a caller extension.
        let encoded = first
            .encode_album_audio_id
            .filter(|_| first.album_audio_id.is_some());
        if let Some(encoded) = &encoded {
            let response = self
                .web_song_info(
                    session,
                    &token,
                    BTreeMap::from([("encode_album_audio_id", encoded.clone())]),
                )
                .await;
            check()?;
            data = response?;
            let next = identity(&data, track, &spec, album_id, &session.user_id)?;
            if next
                .encode_album_audio_id
                .as_ref()
                .is_some_and(|value| value != encoded)
            {
                return Err(invalid());
            }
        }
        Ok((data, spec, token))
    }

    async fn web_song_info(
        &self,
        session: &WebSession,
        token: &str,
        mut query: BTreeMap<&str, String>,
    ) -> Result<Value> {
        query.extend([
            ("srcappid", "2919".into()),
            ("clientver", "20000".into()),
            ("clienttime", crate::account::now_ms()?.to_string()),
            ("appid", "1014".into()),
            ("platid", "4".into()),
            ("mid", session.device.mid.clone()),
            ("uuid", session.device.mid.clone()),
            ("dfid", session.device.dfid().to_owned()),
            ("userid", session.user_id.clone()),
            ("token", token.to_owned()),
        ]);
        query.insert("signature", crate::signing::web_signature(&query, b""));
        let url = format!("https://{HOST}{PATH}");
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(PATH).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let response = self
                .http
                .get(url)
                .query(&query)
                .header("User-Agent", WEB_USER_AGENT)
                .header(REFERER, "https://www.kugou.com/song/")
                .header("accept", "application/json")
                .send()
                .await
                .map_err(crate::account::network_error)?;
            status = Some(response.status());
            // Cross-origin browser requests pass the explicit token, not a Cookie
            // header. Only the login exchange may replace our persisted cookie.
            let bytes = crate::account::read_response_with_limit(response, 1024 * 1024).await?;
            let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            if envelope.error_code.is_some_and(|v| v != envelope.err_code)
                || envelope.errcode.is_some_and(|v| v != envelope.err_code)
            {
                return Err(invalid());
            }
            if envelope.status != 1 || envelope.err_code != 0 {
                let mut error = if envelope.err_code == 30022 {
                    denied("KuGou requires its native client for this resource")
                } else if envelope.err_code == 30020 {
                    TuneWeaveError::new(
                        ErrorCode::PermissionDenied,
                        "KuGou Web song information requires additional verification",
                    )
                    .with_platform(Platform::Kugou)
                    .with_details(json!({"additional_verification_required":true}))
                } else {
                    invalid()
                };
                error.details["platform_status"] = json!(envelope.status);
                error.details["platform_code"] = json!(envelope.err_code);
                return Err(error);
            }
            envelope.data.ok_or_else(invalid)
        }
        .await;
        self.log_upstream_request(
            "web_account_song_info",
            HOST,
            PATH,
            status,
            started,
            0,
            false,
            &result,
        );
        result
    }
}

#[derive(Deserialize)]
struct Envelope {
    status: i64,
    err_code: i64,
    error_code: Option<i64>,
    errcode: Option<i64>,
    data: Option<Value>,
}
#[derive(Deserialize)]
struct Identity {
    hash: String,
    album_audio_id: Option<Number>,
    album_id: Option<Number>,
    userid: Option<Number>,
    encode_album_audio_id: Option<String>,
    has_privilege: Option<bool>,
    is_publish: Option<Number>,
}
fn identity(
    data: &Value,
    track: &Track,
    spec: &SelectedMediaSpec,
    album: u64,
    uid: &str,
) -> Result<Identity> {
    let value: Identity = serde_json::from_value(data.clone()).map_err(|_| invalid())?;
    if !value.hash.eq_ignore_ascii_case(&spec.hash)
        || value
            .album_audio_id
            .as_ref()
            .is_some_and(|v| v.0.to_string() != track.id)
        || value
            .album_id
            .as_ref()
            .is_some_and(|v| album != 0 && v.0 != album)
        || value
            .userid
            .as_ref()
            .is_some_and(|v| v.0.to_string() != uid)
        || value.encode_album_audio_id.as_ref().is_some_and(|v| {
            v.is_empty()
                || v.len() > 256
                || !v
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_=".contains(&b))
        })
    {
        return Err(invalid());
    }
    if value.has_privilege == Some(false) || value.is_publish.as_ref().is_some_and(|v| v.0 == 0) {
        return Err(denied("KuGou did not authorize this Web resource"));
    }
    Ok(value)
}

#[derive(Deserialize)]
struct Song {
    play_url: String,
    play_backup_url: Option<String>,
    timelength: Number,
    filesize: Number,
    bitrate: Number,
    extname: Option<String>,
    is_free_part: Number,
    trans_param: Option<Trans>,
    store_type: Option<String>,
}
#[derive(Deserialize)]
struct Trans {
    hash_offset: Option<Offset>,
}
#[derive(Deserialize)]
struct Offset {
    start_ms: Number,
    end_ms: Number,
}

fn map(
    data: Value,
    track: Track,
    spec: SelectedMediaSpec,
    requested_quality: Quality,
    token: &str,
) -> Result<MediaStream> {
    let value: Song = serde_json::from_value(data).map_err(|_| invalid())?;
    if value.play_url.is_empty() && value.play_backup_url.as_ref().is_none_or(String::is_empty) {
        return Err(denied("KuGou did not authorize Web playback"));
    }
    let expected = spec
        .duration_ms
        .or(track.duration_ms)
        .filter(|v| *v > 0)
        .ok_or_else(invalid)?;
    if value.timelength.0 == 0 || value.filesize.0 == 0 || !(1..=512).contains(&value.bitrate.0) {
        return Err(invalid());
    }
    let offset = value.trans_param.and_then(|v| v.hash_offset);
    let trial = match (value.is_free_part.0, offset) {
        (0, None) => None,
        (0, Some(v)) if v.start_ms.0 == 0 && v.end_ms.0.abs_diff(expected) <= 999 => None,
        (1, Some(v)) if v.end_ms.0 > v.start_ms.0 && v.end_ms.0 <= expected.saturating_add(999) => {
            if value.store_type.as_deref() == Some("album") {
                return Err(denied(
                    "KuGou requires its native client for this album trial",
                ));
            }
            Some(TrialWindow {
                start_ms: v.start_ms.0,
                end_ms: v.end_ms.0,
            })
        }
        _ => return Err(invalid()),
    };
    if value.timelength.0.abs_diff(expected) > 999
        && !trial
            .as_ref()
            .is_some_and(|v| value.timelength.0.abs_diff(v.end_ms - v.start_ms) <= 999)
    {
        return Err(invalid());
    }
    let format = value.extname.map(|v| v.to_ascii_lowercase());
    if format
        .as_ref()
        .is_some_and(|v| !matches!(v.as_str(), "mp3" | "ogg" | "aac" | "m4a"))
    {
        return Err(TuneWeaveError::new(
            ErrorCode::PermissionDenied,
            "KuGou returned an unsupported Web media format",
        )
        .with_platform(Platform::Kugou));
    }
    let mut urls = Vec::new();
    for raw in [Some(value.play_url), value.play_backup_url]
        .into_iter()
        .flatten()
        .filter(|v| !v.is_empty())
    {
        if raw.len() > 8192 || raw.chars().any(char::is_control) {
            return Err(invalid());
        }
        let url = normalize_media_url(&raw)?;
        let decoded: Vec<_> = url::form_urlencoded::parse(url.as_bytes()).collect();
        if url.contains(token)
            || decoded
                .iter()
                .any(|(k, v)| k.contains(token) || v.contains(token))
        {
            return Err(invalid());
        }
        if !urls.contains(&url) {
            urls.push(url);
        }
    }
    if urls.is_empty() {
        return Err(invalid());
    }
    let bitrate = value.bitrate.0 * 1000;
    Ok(MediaStream {
        url: urls.remove(0),
        backup_urls: urls,
        headers: BTreeMap::new(),
        expires_at: None,
        codec: format
            .as_ref()
            .filter(|v| matches!(v.as_str(), "mp3" | "aac"))
            .cloned(),
        format,
        bitrate: Some(bitrate),
        size: Some(value.filesize.0),
        duration_ms: Some(value.timelength.0),
        requested_quality,
        actual_quality: if bitrate <= 96_000 {
            Quality::Low
        } else if bitrate <= 128_000 {
            Quality::Standard
        } else {
            Quality::High
        },
        trial,
        origin_track: Some(track.resource_ref.clone()),
        resolved_track: track.resource_ref,
        resolved_platform: Platform::Kugou,
        match_score: Some(1.0),
        attempts: Vec::new(),
    })
}
fn invalid() -> TuneWeaveError {
    kugou_upstream_error("KuGou Web song information response is invalid")
}

fn map_lyrics(data: Value, track: Track, token: &str) -> Result<Lyrics> {
    #[derive(Deserialize)]
    struct WebLyrics {
        lyrics: String,
        is_free_part: Number,
        store_type: Option<String>,
    }
    let value: WebLyrics = serde_json::from_value(data).map_err(|_| invalid())?;
    match value.is_free_part.0 {
        0 => {}
        1 => match value.store_type.as_deref() {
            Some("album") => {
                return Err(denied(
                    "KuGou does not provide Web lyrics for this album trial",
                ));
            }
            Some(kind)
                if !kind.is_empty() && kind.len() <= 64 && !kind.chars().any(char::is_control) => {}
            _ => return Err(invalid()),
        },
        _ => return Err(invalid()),
    }
    if value.lyrics.trim().is_empty() {
        return Err(TuneWeaveError::new(
            ErrorCode::ResourceNotFound,
            "KuGou did not return Web lyrics",
        )
        .with_platform(Platform::Kugou));
    }
    validate_lyric_text(&value.lyrics)?;
    // A lyric body is public output, even when its request included an account token.
    // Do not expose accidental raw or form-encoded credential echoes as lyric text.
    if value.lyrics.contains(token)
        || url::form_urlencoded::parse(value.lyrics.as_bytes())
            .any(|(key, value)| key.contains(token) || value.contains(token))
    {
        return Err(invalid());
    }
    Ok(Lyrics {
        track_ref: track.resource_ref,
        plain: Some(value.lyrics),
        translated: None,
        romanized: None,
        word_synced: None,
        singing_annotations: None,
        singing_annotations_timestamp: None,
        format: "lrc".into(),
        contributors: Vec::new(),
        extensions: Extensions::new(),
    })
}
fn denied(message: &'static str) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::PermissionDenied, message)
        .with_platform(Platform::Kugou)
        .with_details(json!({"web_media_denied":true}))
}
