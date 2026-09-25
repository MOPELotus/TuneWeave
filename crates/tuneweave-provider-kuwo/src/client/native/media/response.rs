use super::*;

#[derive(Deserialize)]
struct Envelope {
    #[serde(deserialize_with = "integer")]
    code: u64,
    #[serde(rename = "loginSid")]
    login_sid: Option<String>,
    #[serde(default, deserialize_with = "optional_integer")]
    duration: Option<u64>,
    data: Option<Media>,
}
#[derive(Deserialize)]
struct Media {
    #[serde(deserialize_with = "integer")]
    rid: u64,
    format: String,
    #[serde(deserialize_with = "integer")]
    bitrate: u64,
    quality: Option<String>,
    url: Option<String>,
    surl: Option<String>,
    ekey: Option<String>,
    #[serde(rename = "type", deserialize_with = "integer")]
    kind: u64,
    #[serde(rename = "startPos", deserialize_with = "integer")]
    start: u64,
    #[serde(rename = "endPos", deserialize_with = "integer")]
    end: u64,
}
fn integer<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<u64, D::Error> {
    optional_integer(d)?.ok_or_else(|| serde::de::Error::custom("missing native media number"))
}
fn optional_integer<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<u64>, D::Error> {
    deserialize_code(d)?
        .map(|value| {
            value
                .parse::<u64>()
                .ok()
                .filter(|n| n.to_string() == value)
                .ok_or_else(|| serde::de::Error::custom("invalid native media number"))
        })
        .transpose()
}
pub(super) fn parse(
    bytes: &[u8],
    input: &KuwoNativeSessionInput,
    id: &str,
    spec: Spec,
) -> Result<Outcome> {
    parse_inner(bytes, input, id, spec, false)
}
pub(super) fn parse_sing_along(
    bytes: &[u8],
    input: &KuwoNativeSessionInput,
    id: &str,
    spec: Spec,
) -> Result<Outcome> {
    parse_inner(bytes, input, id, spec, true)
}
fn parse_inner(
    bytes: &[u8],
    input: &KuwoNativeSessionInput,
    id: &str,
    spec: Spec,
    sing_along: bool,
) -> Result<Outcome> {
    let body: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if body
        .login_sid
        .as_deref()
        .is_some_and(|sid| !sid.is_empty() && sid != input.session_id())
    {
        return Err(invalid());
    }
    match body.code {
        4015 | 4017 => return Err(authentication_required()),
        401 | 402 | 403 | 404 | 407 | 4012 | 4018 | 5001 | 5002 => {
            return Ok(Outcome::Denied {
                code: Some(body.code as i64),
                message: "Kuwo refused full account media authorization",
            });
        }
        200 => {}
        _ => return Err(invalid()),
    }
    let duration = body
        .duration
        .filter(|d| *d > 0 && *d <= 86_400)
        .ok_or_else(invalid)?;
    let data = body.data.ok_or_else(invalid)?;
    if sing_along {
        match parse_data(data, input, id, spec, duration)? {
            denied @ Outcome::Denied { .. } => return Ok(denied),
            Outcome::Allowed { .. } => {}
        }
        #[derive(Deserialize)]
        struct SidecarEnvelope {
            data: SidecarParent,
        }
        #[derive(Deserialize)]
        struct SidecarParent {
            bc_data: Media,
        }
        let extra: SidecarEnvelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
        let companion = extra.data.bc_data;
        if companion.quality.as_deref() != Some(BCMS.tag) {
            return Err(invalid());
        }
        return parse_data(companion, input, id, BCMS, duration);
    }
    parse_data(data, input, id, spec, duration)
}
fn parse_data(
    data: Media,
    input: &KuwoNativeSessionInput,
    id: &str,
    spec: Spec,
    duration: u64,
) -> Result<Outcome> {
    if data.rid.to_string() != id
        || !data.format.eq_ignore_ascii_case(spec.format)
        || data.bitrate != u64::from(spec.selector)
        || (spec == DTSX && data.quality.as_deref() != Some(DTSX.tag))
        || data
            .quality
            .as_deref()
            .is_some_and(|v| !v.is_empty() && v != spec.tag)
    {
        return Err(invalid());
    }
    if data.kind != 0 || data.start != 0 || data.end != 0 {
        return Ok(Outcome::Denied {
            code: Some(200),
            message: "Kuwo returned restricted or partial media instead of a full track",
        });
    }
    let key = data
        .ekey
        .as_deref()
        .filter(|v| !v.is_empty())
        .map(|value| content::Key::parse(value, input))
        .transpose()?;
    if matches!(spec.format, "mflac" | "mgg" | "mmp4") && key.is_none() {
        return Err(invalid());
    }
    let mut urls = Vec::new();
    // The native response may carry HTTP `url` plus independent HTTPS `surl`.
    // Validate both supplied locations; return only actual HTTPS locations.
    for value in [data.surl.as_deref(), data.url.as_deref()]
        .into_iter()
        .flatten()
        .filter(|v| !v.is_empty())
    {
        let url = validate_url(value, input, spec)?;
        if url.scheme() == "https" && !urls.iter().any(|v| v == url.as_str()) {
            urls.push(url.to_string());
        }
    }
    if urls.is_empty() {
        return Err(invalid());
    }
    let url = urls.remove(0);
    Ok(Outcome::Allowed {
        url,
        backups: urls,
        format: spec.format,
        bitrate: spec.bitrate,
        quality: spec.quality,
        duration_ms: duration * 1000,
        key,
        trial: None,
    })
}
pub(super) fn validate_url(value: &str, input: &KuwoNativeSessionInput, spec: Spec) -> Result<Url> {
    if value.len() > 4096
        || !value.is_ascii()
        || value
            .bytes()
            .any(|b| b.is_ascii_control() || b.is_ascii_whitespace() || b == b'\\')
        || echoes_secret(value, input.session_id())
    {
        return Err(invalid());
    }
    let url = Url::parse(value).map_err(|_| invalid())?;
    let prefix = url
        .host_str()
        .and_then(|h| h.strip_suffix(".kuwo.cn"))
        .filter(|p| {
            !p.is_empty()
                && p.len() <= 63
                && p.ends_with("-sycdn")
                && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        .ok_or_else(invalid)?;
    let path = url.path().to_ascii_lowercase();
    let suffix = match spec.format {
        "aac" => path.ends_with(".aac") || path.ends_with(".m4a"),
        "mp3" => path.ends_with(".mp3"),
        "flac" => path.ends_with(".flac"),
        "mflac" => path.ends_with(".mflac"),
        "mgg" => path.ends_with(".mgg"),
        "mmp4" => path.ends_with(".mmp4"),
        _ => false,
    };
    if prefix.contains('.')
        || !matches!(url.scheme(), "https" | "http")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || !suffix
        || path.contains('%')
    {
        return Err(invalid());
    }
    Ok(url)
}
