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
    #[serde(default, deserialize_with = "optional_integer")]
    duration: Option<u64>,
    #[serde(rename = "fileSize", default, deserialize_with = "optional_integer")]
    file_size: Option<u64>,
    #[serde(rename = "filePath")]
    file_path: Option<String>,
    #[serde(rename = "media_basic_info")]
    media_basic_info: Option<serde_json::Value>,
    quality: Option<String>,
    url: Option<String>,
    surl: Option<String>,
    ekey: Option<String>,
    #[serde(rename = "type", deserialize_with = "integer")]
    kind: u64,
    #[serde(
        rename = "startPos",
        default,
        deserialize_with = "present_optional_integer"
    )]
    start: Option<u64>,
    #[serde(
        rename = "endPos",
        default,
        deserialize_with = "present_optional_integer"
    )]
    end: Option<u64>,
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
fn present_optional_integer<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<u64>, D::Error> {
    let value =
        deserialize_code(d)?.ok_or_else(|| serde::de::Error::custom("null native media number"))?;
    let number = value
        .parse::<u64>()
        .ok()
        .filter(|number| number.to_string() == value)
        .ok_or_else(|| serde::de::Error::custom("invalid native media number"))?;
    Ok(Some(number))
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
    let result = parse_inner_impl(bytes, input, id, spec, sing_along);
    if result.is_err() {
        #[cfg(debug_assertions)]
        eprintln!(
            "DIAGNOSTIC kuwo_native_media_shape={}",
            safe_response_context(bytes, input, id, spec)
        );
    }
    result
}

fn parse_inner_impl(
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
    let data = body.data.ok_or_else(invalid)?;
    let duration = match (body.duration, data.duration) {
        (Some(root), Some(media)) if root == media => root,
        (Some(_), Some(_)) => return Err(invalid()),
        (Some(root), None) => root,
        (None, Some(media)) if data.has_current_response_metadata() => media,
        _ => return Err(invalid()),
    };
    if !(1..=86_400).contains(&duration) {
        return Err(invalid());
    }
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

#[cfg(debug_assertions)]
pub(super) fn safe_response_shape(bytes: &[u8]) -> serde_json::Value {
    use serde_json::{Map, Value, json};

    fn field_types(value: &Value) -> Value {
        let Some(object) = value.as_object() else {
            return json!({"type":match value { Value::Null=>"null", Value::Bool(_)=>"bool", Value::Number(_)=>"number", Value::String(_)=>"string", Value::Array(_)=>"array", Value::Object(_)=>"object" }});
        };
        let mut fields = Map::new();
        for (index, (key, value)) in object.iter().take(64).enumerate() {
            let kind = match value {
                Value::Null => "null",
                Value::Bool(_) => "bool",
                Value::Number(_) => "number",
                Value::String(_) => "string",
                Value::Array(_) => "array",
                Value::Object(_) => "object",
            };
            let safe_key = if key.len() <= 64
                && key
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
            {
                key.clone()
            } else {
                format!("field_{index}")
            };
            fields.insert(safe_key, json!(kind));
        }
        json!({"fields":Value::Object(fields),"truncated":object.len()>64})
    }

    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return json!({"kind":"non_json","bytes":bytes.len()});
    };
    let mut shape = Map::new();
    shape.insert("root_fields".into(), field_types(&value));
    for key in ["code", "duration"] {
        if let Some(field) = value.get(key) {
            let safe_code = field.as_u64().or_else(|| {
                field
                    .as_str()
                    .filter(|text| text.len() <= 10 && text.bytes().all(|b| b.is_ascii_digit()))
                    .and_then(|text| text.parse::<u64>().ok())
            });
            if let Some(number) = safe_code {
                shape.insert(key.into(), json!(number));
            }
        }
    }
    if let Some(data) = value.get("data") {
        shape.insert("data_fields".into(), field_types(data));
        if let Some(data) = data.as_object() {
            for key in ["url", "surl", "ekey"] {
                if let Some(value) = data.get(key) {
                    shape.insert(
                        format!("{key}_nonempty"),
                        json!(value.as_str().is_some_and(|s| !s.is_empty())),
                    );
                }
            }
            for key in ["url", "surl"] {
                if let Some(value) = data.get(key).and_then(Value::as_str) {
                    shape.insert(format!("{key}_url_class"), safe_url_class(value));
                }
            }
            for key in ["format", "quality"] {
                if let Some(value) = data.get(key).and_then(Value::as_str) {
                    let class = match value.to_ascii_lowercase().as_str() {
                        "aac" | "mp3" | "flac" | "mflac" | "mgg" | "mmp4" => {
                            value.to_ascii_lowercase()
                        }
                        _ => "other".into(),
                    };
                    shape.insert(format!("{key}_class"), json!(class));
                }
            }
            for key in [
                "duration", "fileSize", "bitrate", "type", "startPos", "endPos",
            ] {
                if let Some(number) = data.get(key).and_then(safe_integer) {
                    shape.insert(format!("data_{key}"), json!(number));
                }
            }
            if let Some(info) = data.get("media_basic_info") {
                shape.insert("media_basic_info_fields".into(), field_types(info));
                if let Some(info) = info.as_object() {
                    for key in [
                        "duration",
                        "duration_ms",
                        "bitrate",
                        "filesize",
                        "fileSize",
                        "type",
                        "startPos",
                        "endPos",
                    ] {
                        if let Some(number) = info.get(key).and_then(safe_integer) {
                            shape.insert(format!("media_{key}"), json!(number));
                        }
                    }
                    for key in ["format", "quality"] {
                        if let Some(value) = info.get(key).and_then(Value::as_str) {
                            let class = match value.to_ascii_lowercase().as_str() {
                                "aac" | "mp3" | "flac" | "mflac" | "mgg" | "mmp4" => {
                                    value.to_ascii_lowercase()
                                }
                                _ => "other".into(),
                            };
                            shape.insert(format!("media_{key}_class"), json!(class));
                        }
                    }
                }
            }
        }
    }
    Value::Object(shape)
}

#[cfg(debug_assertions)]
fn safe_response_context(
    bytes: &[u8],
    input: &KuwoNativeSessionInput,
    id: &str,
    spec: Spec,
) -> serde_json::Value {
    use serde_json::{Value, json};

    let mut shape = safe_response_shape(bytes);
    let Some(object) = shape.as_object_mut() else {
        return shape;
    };
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return shape;
    };
    if let Some(data) = value.get("data").and_then(Value::as_object) {
        if let Some(rid) = data.get("rid").and_then(safe_integer) {
            object.insert(
                "data_rid_matches_request".into(),
                json!(rid.to_string() == id),
            );
        }
        if let Some(bitrate) = data.get("bitrate").and_then(safe_integer) {
            object.insert(
                "data_bitrate_matches_request".into(),
                json!(bitrate == u64::from(spec.selector)),
            );
        }
        if let Some(format) = data.get("format").and_then(Value::as_str) {
            object.insert(
                "data_format_matches_request".into(),
                json!(format.eq_ignore_ascii_case(spec.format)),
            );
        }
        if let Some(quality) = data.get("quality").and_then(Value::as_str) {
            object.insert(
                "data_quality_matches_request".into(),
                json!(quality.is_empty() || quality == spec.tag),
            );
        }
        for key in ["url", "surl"] {
            if let Some(media_url) = data.get(key).and_then(Value::as_str) {
                object.insert(
                    format!("{key}_validates"),
                    json!(validate_url(media_url, input, spec).is_ok()),
                );
                object.insert(
                    format!("{key}_echoes_session_id"),
                    json!(echoes_secret(media_url, input.session_id())),
                );
                if let Ok(url) = Url::parse(media_url) {
                    object.insert(
                        format!("{key}_signed_query_valid"),
                        json!(
                            url.host_str() == Some("kw-lv.kuwo.cn")
                                && valid_signed_query(&url, input)
                        ),
                    );
                    if url.host_str() == Some("kw-lv.kuwo.cn") {
                        object.insert(
                            format!("{key}_signed_query_class"),
                            json!(signed_query_rejection(&url, input).unwrap_or("accepted")),
                        );
                    }
                }
            }
        }
    }
    if let Some(login_sid) = value.get("loginSid").and_then(Value::as_str) {
        object.insert(
            "login_sid_matches_request".into(),
            json!(login_sid == input.session_id()),
        );
    }
    shape
}

#[cfg(debug_assertions)]
fn safe_url_class(value: &str) -> serde_json::Value {
    use serde_json::json;

    let Ok(url) = Url::parse(value) else {
        return json!({"parseable":false});
    };
    let host = url.host_str().unwrap_or_default();
    let host_class = if host
        .strip_suffix(".kuwo.cn")
        .is_some_and(|prefix| prefix.ends_with("-sycdn"))
    {
        "kuwo_sycdn"
    } else if host == "kw-er.kuwo.cn" {
        "kuwo_kw_er"
    } else if host.ends_with(".kuwo.cn") {
        "kuwo_other"
    } else if host.is_empty() {
        "empty"
    } else {
        "external"
    };
    let path = url.path().to_ascii_lowercase();
    let suffix_class = [".mmp4", ".mflac", ".mgg", ".flac", ".mp3", ".aac", ".m4a"]
        .into_iter()
        .find(|suffix| path.ends_with(suffix))
        .unwrap_or("other");
    let official_host = host
        .strip_suffix(".kuwo.cn")
        .map(|_| host)
        .unwrap_or("other");
    let query_fields = url
        .query_pairs()
        .take(16)
        .map(|(key, _)| {
            let length = key.len();
            let class = if key.len() > 48 {
                "long"
            } else if !key.is_ascii() {
                "unicode"
            } else if key.bytes().any(|byte| byte.is_ascii_control()) {
                "control"
            } else if key
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
            {
                "safe_name"
            } else {
                "ascii_punctuation"
            };
            let name = if class == "safe_name" {
                key.into_owned()
            } else {
                "other".to_owned()
            };
            json!({"name":name,"class":class,"length":length})
        })
        .collect::<Vec<_>>();
    json!({
        "scheme":match url.scheme() { "https"=>"https", "http"=>"http", _=>"other"},
        "host_class":host_class,
        "official_host":official_host,
        "suffix_class":suffix_class,
        "query_present":url.query().is_some(),
        "query_fields":query_fields,
        "fragment_present":url.fragment().is_some(),
        "port_present":url.port().is_some(),
        "userinfo_present":!url.username().is_empty() || url.password().is_some(),
        "path_percent_encoded":path.contains('%'),
        "url_length_bucket":match value.len() { 0..=255=>"short", 256..=1024=>"medium", _=>"long"},
    })
}

#[cfg(debug_assertions)]
fn safe_integer(value: &serde_json::Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_str()
            .filter(|text| text.len() <= 12 && text.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|text| text.parse::<u64>().ok())
    })
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
    let partial_range = match (data.start, data.end) {
        (Some(0), Some(0)) => false,
        (None, None) if data.has_current_response_metadata() => false,
        (None, None) => return Err(invalid()),
        (Some(start), Some(end)) if end > start => true,
        (Some(_), Some(_)) => return Err(invalid()),
        _ => return Err(invalid()),
    };
    if data.kind != 0 || partial_range {
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
    // Validate each independently, discard invalid/insecure candidates, and
    // return only actual HTTPS locations.
    for value in [data.surl.as_deref(), data.url.as_deref()]
        .into_iter()
        .flatten()
        .filter(|v| !v.is_empty())
    {
        if let Ok(url) = validate_url(value, input, spec) {
            if url.scheme() == "https" && !urls.iter().any(|v| v == url.as_str()) {
                urls.push(url.to_string());
            }
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

impl Media {
    fn has_current_response_metadata(&self) -> bool {
        self.file_path.as_deref().is_some_and(|path| {
            !path.is_empty()
                && path.len() <= 4096
                && !path.bytes().any(|byte| byte.is_ascii_control())
        }) && self
            .file_size
            .is_some_and(|size| (1..=512 * 1024 * 1024).contains(&size))
            && self
                .media_basic_info
                .as_ref()
                .is_some_and(serde_json::Value::is_object)
    }
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
    let host = url.host_str().ok_or_else(invalid)?;
    let is_sycdn = host.strip_suffix(".kuwo.cn").is_some_and(|prefix| {
        !prefix.is_empty()
            && prefix.len() <= 63
            && prefix.ends_with("-sycdn")
            && prefix
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    });
    let is_kw_lv = host == "kw-lv.kuwo.cn";
    let is_kw_er = host == "kw-er.kuwo.cn";
    if !is_sycdn && !is_kw_lv && !is_kw_er {
        return Err(invalid());
    }
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
    if !matches!(url.scheme(), "https" | "http")
        || (is_kw_er && url.scheme() != "https")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !suffix
        || path.contains('%')
        || match url.query() {
            Some(_) => !is_kw_er && (!is_kw_lv || !valid_signed_query(&url, input)),
            None => false,
        }
    {
        return Err(invalid());
    }
    Ok(url)
}

fn valid_signed_query(url: &Url, input: &KuwoNativeSessionInput) -> bool {
    signed_query_rejection(url, input).is_none()
}

fn signed_query_rejection(url: &Url, input: &KuwoNativeSessionInput) -> Option<&'static str> {
    let Some(query) = url.query() else {
        return Some("missing");
    };
    if query.is_empty() {
        return Some("empty");
    }
    if query.len() > 2048 {
        return Some("too_long");
    }
    if !query.is_ascii() {
        return Some("non_ascii");
    }
    if query
        .bytes()
        .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace() || byte == b'\\')
    {
        return Some("unsafe_raw_character");
    }
    if echoes_secret(url.as_str(), input.session_id()) {
        return Some("session_id_echo");
    }
    let mut names = Vec::new();
    let mut count = 0usize;
    for (name, value) in url.query_pairs() {
        count += 1;
        if count > 12 {
            return Some("too_many_fields");
        }
        if !bounded_query_name(&name) || !bounded_query_value(&value) {
            return Some("invalid_component");
        }
        if names
            .iter()
            .any(|seen: &String| seen.eq_ignore_ascii_case(&name))
        {
            return Some("duplicate_field");
        }
        names.push(name.into_owned());
    }
    if count == 6 {
        None
    } else {
        Some("invalid_field_count")
    }
}

fn bounded_query_name(value: &str) -> bool {
    (8..=128).contains(&value.len())
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b'\\')
}

fn bounded_query_value(value: &str) -> bool {
    value.len() <= 1024
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b'\\')
}
