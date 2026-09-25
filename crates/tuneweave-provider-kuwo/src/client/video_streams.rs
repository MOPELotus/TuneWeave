use super::*;
use tuneweave_core::{VideoDetail, VideoStream};

#[derive(Serialize)]
struct Query<'a> {
    mid: &'a str,
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(rename = "httpsStatus")]
    https_status: u8,
    #[serde(rename = "reqId")]
    request_id: String,
    plat: &'static str,
    from: &'static str,
}

impl KuwoClient {
    pub(crate) async fn mv_stream(&self, id: &str, resolution: u32) -> Result<VideoStream> {
        // Read permission from the current platform response, never caller metadata.
        let detail = self.mv_detail(id).await?;
        let mut stream = empty(&detail, resolution);
        if let Some(reason) = metadata_denial(&detail)? {
            unavailable(&mut stream, reason);
            return Ok(stream);
        }
        for refresh in [false, true] {
            let response = self
                .signed_get(
                    KuwoSignedEndpoint::MvPlayback,
                    &Query {
                        mid: id,
                        kind: "mv",
                        https_status: 1,
                        request_id: new_request_id(),
                        plat: "web_www",
                        from: "",
                    },
                    &format!("https://www.kuwo.cn/mvplay/{id}"),
                    refresh,
                    u8::from(refresh),
                )
                .await?;
            match response {
                KuwoSignedResponse::SessionRejected if !refresh => continue,
                KuwoSignedResponse::SessionRejected => return Err(invalid()),
                KuwoSignedResponse::Body(bytes) => {
                    if !refresh && is_signed_session_rejection(&bytes) {
                        continue;
                    }
                    match parse(&bytes)? {
                        Outcome::Allowed(url) => {
                            stream.available = true;
                            stream.url = Some(url);
                            stream.format = Some("mp4".into());
                            stream.platform_code = Some(200);
                            stream.extensions.insert(
                                "authorization_source".into(),
                                json!("current_metadata_and_play_url"),
                            );
                        }
                        Outcome::Denied(code) => {
                            stream.platform_code = Some(code);
                            unavailable(&mut stream, "upstream_denied");
                        }
                    }
                    return Ok(stream);
                }
            }
        }
        Err(invalid())
    }
}

fn metadata_denial(detail: &VideoDetail) -> Result<Option<&'static str>> {
    let fields = &detail.video.extensions;
    if fields.get("online").and_then(serde_json::Value::as_u64) == Some(0) {
        return Ok(Some("offline"));
    }
    if fields.get("disable").and_then(serde_json::Value::as_u64) == Some(1) {
        return Ok(Some("disabled"));
    }
    match fields
        .get("mv_pay_info")
        .and_then(|v| v.get("play"))
        .and_then(serde_json::Value::as_u64)
    {
        Some(0) => Ok(None),
        Some(1) => Ok(Some("permission_denied")),
        _ => Err(kuwo_upstream_error(
            "Kuwo MV metadata omitted a recognized playback permission",
        )),
    }
}

fn empty(detail: &VideoDetail, resolution: u32) -> VideoStream {
    VideoStream {
        video_ref: detail.video.resource_ref.clone(),
        platform: Platform::Kuwo,
        available: false,
        url: None,
        backup_urls: vec![],
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
            ("backend".into(), json!("current_web_mv_play_url")),
            ("kind".into(), json!("mv")),
            ("source_track_id".into(), json!(detail.video.id)),
            ("resolution_selection".into(), json!("platform_default")),
            ("duration_source".into(), json!("music_info")),
        ]),
    }
}
fn unavailable(stream: &mut VideoStream, reason: &'static str) {
    stream
        .extensions
        .insert("unavailable_reason".into(), json!(reason));
    stream.message = Some(
        match reason {
            "offline" => "Kuwo reported this MV as offline",
            "disabled" => "Kuwo disabled this MV",
            _ => "Kuwo did not authorize anonymous MV playback",
        }
        .into(),
    );
}

#[derive(Deserialize)]
struct Envelope {
    code: i64,
    data: Option<Media>,
}
#[derive(Deserialize)]
struct Media {
    url: String,
}
enum Outcome {
    Allowed(String),
    Denied(i64),
}
fn parse(bytes: &[u8]) -> Result<Outcome> {
    let body: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    match body.code {
        200 => Ok(Outcome::Allowed(validate_url(
            &body.data.ok_or_else(invalid)?.url,
        )?)),
        // These are explicit failures of the shared official playUrl endpoint.
        -1 | -1001 if body.data.is_none() => Ok(Outcome::Denied(body.code)),
        _ => Err(invalid().with_details(json!({"upstream_code":body.code}))),
    }
}

fn validate_url(value: &str) -> Result<String> {
    if value.len() > 4096
        || !value.is_ascii()
        || value
            .bytes()
            .any(|c| c.is_ascii_whitespace() || c.is_ascii_control() || b"%\\?#@".contains(&c))
    {
        return Err(invalid());
    }
    let url = Url::parse(value).map_err(|_| invalid())?;
    if url.scheme() != "https"
        || url.host_str() != Some("kw-bj.kuwo.cn")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.as_str() != value
    {
        return Err(invalid());
    }
    let parts: Vec<_> = url
        .path()
        .strip_prefix('/')
        .ok_or_else(invalid)?
        .split('/')
        .collect();
    if parts.len() != 8
        || !hex(parts[0], 32)
        || !hex(parts[1], 8)
        || !matches!(parts[2], "ll" | "rc")
        || parts[3] != "resource"
        || !parts[4].strip_prefix('m').is_some_and(|v| decimal(v, 2))
        || !decimal(parts[5], 2)
        || !decimal(parts[6], 2)
        || !parts[7]
            .strip_suffix(".mp4")
            .is_some_and(|v| decimal(v, 32))
    {
        return Err(invalid());
    }
    Ok(value.to_owned())
}
fn hex(value: &str, length: usize) -> bool {
    value.len() == length && value.bytes().all(|c| c.is_ascii_hexdigit())
}
fn decimal(value: &str, limit: usize) -> bool {
    !value.is_empty() && value.len() <= limit && value.bytes().all(|c| c.is_ascii_digit())
}
fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo MV playback returned an invalid or untrusted response")
}

#[cfg(test)]
mod tests;
