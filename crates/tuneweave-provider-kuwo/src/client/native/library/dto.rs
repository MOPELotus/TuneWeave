use super::*;

pub(in crate::client::native) mod ordering;

#[derive(Deserialize)]
struct OwnedResponse {
    #[serde(default, deserialize_with = "unsigned")]
    errcode: Option<u64>,
    result: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    uid: Option<String>,
    plist: Vec<Owned>,
}
#[derive(Deserialize)]
struct SavedResponse {
    result: String,
    #[serde(default, deserialize_with = "unsigned")]
    errcode: Option<u64>,
    #[serde(default, deserialize_with = "deserialize_code")]
    uid: Option<String>,
    data: Vec<Saved>,
    // The observed empty response's top-level total is not yet proven to be
    // a nonempty collection count. Completion follows the official short page.
}
#[derive(Deserialize, serde::Serialize)]
struct Owned {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default, deserialize_with = "deserialize_code")]
    id: Option<String>,
    title: Option<String>,
    pic: Option<String>,
    info: Option<String>,
    #[serde(default, deserialize_with = "boolean")]
    ispub: Option<bool>,
    #[serde(default, deserialize_with = "unsigned")]
    playnum: Option<u64>,
    #[serde(default, deserialize_with = "unsigned")]
    musicnum: Option<u64>,
    #[serde(default, deserialize_with = "sequence")]
    turn: Option<i32>,
}
#[derive(Deserialize)]
struct Saved {
    #[serde(default, deserialize_with = "deserialize_code")]
    id: Option<String>,
    name: String,
    desc: Option<String>,
    pic: Option<String>,
    #[serde(default, deserialize_with = "unsigned")]
    total: Option<u64>,
}

/// A write readback must detect a target that changed to an excluded system kind,
/// and must not mistake an existing system ID for a newly created playlist.
pub(in crate::client::native) fn all_owned_ids(bytes: &[u8]) -> Result<BTreeSet<String>> {
    let response: OwnedResponse =
        serde_json::from_slice(bytes).map_err(|_| failed("owned_ids_schema", None))?;
    let mut ids = BTreeSet::new();
    for item in response.plist {
        // A `PLAYLIST` row has no verified public playlist contract, but a
        // canonical nonzero ID still stays reserved for write collision checks.
        if let Some(id) = item.id {
            if id == "0" && !matches!(item.kind.as_str(), "GENERAL" | "MYFAVORITE") {
                continue;
            }
            crate::client::native::playlist::validate_id(&id)
                .map_err(|_| failed("owned_ids_id", Some(ids.len())))?;
            if !ids.insert(id) {
                return Err(failed("owned_ids_dedupe", Some(ids.len())));
            }
        }
    }
    Ok(ids)
}

pub(in crate::client::native) fn parse(
    bytes: &[u8],
    input: &KuwoNativeSessionInput,
    section: Section,
) -> Result<Vec<Playlist>> {
    let result = parse_inner(bytes, input, section);
    if result.is_err() {
        #[cfg(debug_assertions)]
        eprintln!(
            "DIAGNOSTIC kuwo_library_shape={}",
            crate::client::native::diagnostic_response_shape(bytes)
        );
    }
    result
}

fn parse_inner(
    bytes: &[u8],
    input: &KuwoNativeSessionInput,
    section: Section,
) -> Result<Vec<Playlist>> {
    let items = match section {
        Section::Created | Section::Favorite | Section::Owned => {
            let response: OwnedResponse =
                serde_json::from_slice(bytes).map_err(|_| failed("owned_schema_decode", None))?;
            if response.errcode != Some(0)
                || response.result.as_deref().is_some_and(|v| v != "ok")
                || response
                    .uid
                    .as_deref()
                    .is_some_and(|v| v != input.user_id())
            {
                return Err(failed("owned_response_guard", Some(response.plist.len())));
            }
            if response.plist.len() > MAX_OWNED {
                return Err(failed("owned_row_limit", Some(response.plist.len())));
            }
            let row_count = response.plist.len();
            let mut owned = Vec::new();
            let mut favorites = 0;
            let mut owned_ids = BTreeSet::new();
            for (row_index, item) in response.plist.into_iter().enumerate() {
                if section != Section::Created
                    && matches!(item.kind.as_str(), "GENERAL" | "MYFAVORITE")
                {
                    let id = item
                        .id
                        .as_deref()
                        .ok_or_else(|| failed("owned_row_id_missing", Some(row_count)))?;
                    crate::client::native::playlist::validate_id(id)
                        .map_err(|_| failed("owned_row_id_invalid", Some(row_count)))?;
                    if !owned_ids.insert(id.to_owned()) {
                        return Err(failed("owned_owner_dedupe", Some(owned_ids.len())));
                    }
                }
                match item.kind.as_str() {
                    "GENERAL" if section != Section::Favorite => owned.push((row_index, item)),
                    "MYFAVORITE" if section != Section::Created => {
                        favorites += 1;
                        if favorites > 1 {
                            return Err(failed("favorite_row_duplicate", Some(row_count)));
                        }
                        owned.push((row_index, item));
                    }
                    "PLAYLIST" | "MOBI_DEFAULT" | "PC_DEFAULT" | "RADIO" | "ORDER" => (),
                    "GENERAL" | "MYFAVORITE" => (),
                    _ => return Err(failed("owned_row_kind", Some(row_count))),
                }
            }
            let ordered = owned.iter().all(|(_, item)| item.turn.is_some());
            if ordered {
                owned.sort_by_key(|(_, item)| item.turn);
            }
            let mapped = owned
                .into_iter()
                .map(|(row_index, item)| {
                    let favorite = item.kind == "MYFAVORITE";
                    let kind = diagnostic_kind(&item.kind);
                    let title_state = diagnostic_title_state(item.title.as_deref());
                    let result = playlist(
                        item.id,
                        if favorite {
                            Some("我喜欢听".into())
                        } else {
                            item.title
                        },
                        item.info,
                        item.pic,
                        item.musicnum,
                        input,
                        if favorite {
                            Section::Favorite
                        } else {
                            Section::Created
                        },
                    );
                    if result.is_err() {
                        super::diagnostic_owned_row(row_index, kind, title_state);
                    }
                    let mut playlist = result?;
                    if favorite {
                        playlist.extensions.extend([
                            ("is_favorite".into(), json!(true)),
                            ("upstream_type".into(), json!("MYFAVORITE")),
                        ]);
                    }
                    playlist.extensions.extend([
                        ("owner_id".into(), json!(input.user_id())),
                        ("is_public".into(), json!(item.ispub)),
                        ("play_count".into(), json!(item.playnum)),
                        ("turn".into(), json!(item.turn)),
                        (
                            "ordering".into(),
                            json!(if ordered {
                                "turn_ascending"
                            } else {
                                "upstream"
                            }),
                        ),
                    ]);
                    Ok(playlist)
                })
                .collect::<Result<Vec<_>>>();
            mapped.map_err(|_| failed("owned_row_map", Some(row_count)))?
        }
        Section::Saved => {
            let response: SavedResponse =
                serde_json::from_slice(bytes).map_err(|_| failed("saved_schema_decode", None))?;
            if response.result != "ok"
                || response.errcode.is_some_and(|v| v != 0)
                || response
                    .uid
                    .as_deref()
                    .is_some_and(|v| v != input.user_id())
            {
                return Err(failed("saved_response_guard", Some(response.data.len())));
            }
            if response.data.len() > PAGE_SIZE {
                return Err(failed("saved_row_limit", Some(response.data.len())));
            }
            let row_count = response.data.len();
            let mapped = response
                .data
                .into_iter()
                .map(|item| {
                    playlist(
                        item.id,
                        Some(item.name),
                        item.desc,
                        item.pic,
                        item.total,
                        input,
                        section,
                    )
                })
                .collect::<Result<Vec<_>>>();
            mapped.map_err(|_| failed("saved_row_map", Some(row_count)))?
        }
    };
    let mut seen = BTreeSet::new();
    if items.iter().any(|item| !seen.insert(&item.id)) {
        return Err(failed("mapped_dedupe", Some(items.len())));
    }
    Ok(items)
}

fn failed(stage: &'static str, count: Option<usize>) -> TuneWeaveError {
    super::diagnostic_stage(stage, false, count);
    invalid()
}

fn diagnostic_kind(value: &str) -> &'static str {
    match value {
        "GENERAL" => "general",
        "PLAYLIST" => "playlist",
        "MYFAVORITE" => "myfavorite",
        "MOBI_DEFAULT" => "mobi_default",
        "PC_DEFAULT" => "pc_default",
        "RADIO" => "radio",
        "ORDER" => "order",
        _ => "other",
    }
}

fn diagnostic_title_state(value: Option<&str>) -> &'static str {
    match value {
        None => "missing",
        Some("") => "empty",
        Some(value) if value.trim().is_empty() => "blank",
        Some(_) => "nonempty",
    }
}

fn playlist(
    id: Option<String>,
    name: Option<String>,
    description: Option<String>,
    cover: Option<String>,
    count: Option<u64>,
    input: &KuwoNativeSessionInput,
    section: Section,
) -> Result<Playlist> {
    let id = id
        .filter(|value| {
            value
                .parse::<u64>()
                .ok()
                .is_some_and(|n| n > 0 && n <= i64::MAX as u64 && n.to_string() == *value)
        })
        .ok_or_else(|| failed("playlist_id_map", None))?;
    let name = playlist_name(name, input)?;
    let description = text(description, 16 * 1024, true, input)
        .map_err(|_| failed("playlist_description_map", None))?
        .unwrap_or_default();
    let cover_url = picture(cover, input).map_err(|_| failed("playlist_picture_map", None))?;
    let resource_ref = ResourceRef::new(Platform::Kuwo, &id)
        .map_err(|_| failed("playlist_reference_map", None))?;
    Ok(Playlist {
        resource_ref,
        platform: Platform::Kuwo,
        id,
        name,
        description,
        cover_url,
        creator: None,
        track_count: count,
        tags: Vec::new(),
        subscribed: (section == Section::Saved).then_some(true),
        created_at: None,
        updated_at: None,
        extensions: Extensions::from([
            ("backend".into(), json!("native_account_library")),
            ("library_owner_id".into(), json!(input.user_id())),
            ("library_section".into(), json!(section.name())),
        ]),
    })
}

fn playlist_name(value: Option<String>, input: &KuwoNativeSessionInput) -> Result<String> {
    let Some(value) = value else {
        return Err(failed("playlist_name_missing", None));
    };
    if value.is_empty() {
        return Err(failed("playlist_name_empty", None));
    }
    if value.len() > 1024 {
        return Err(failed("playlist_name_too_long", None));
    }
    if value.chars().any(char::is_control) {
        return Err(failed("playlist_name_control", None));
    }
    if echoes_secret(&value, input.session_id()) {
        return Err(failed("playlist_name_secret_echo", None));
    }
    if value.trim().is_empty() {
        return Err(failed("playlist_name_blank", None));
    }
    Ok(value)
}

pub(in crate::client::native) fn text(
    value: Option<String>,
    max: usize,
    multiline: bool,
    input: &KuwoNativeSessionInput,
) -> Result<Option<String>> {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    if value.len() > max
        || value
            .chars()
            .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\r' | '\t')))
        || echoes_secret(&value, input.session_id())
    {
        return Err(invalid());
    }
    Ok(Some(value))
}
pub(in crate::client::native) fn picture(
    value: Option<String>,
    input: &KuwoNativeSessionInput,
) -> Result<Option<String>> {
    let Some(value) = text(value, 2048, false, input)? else {
        return Ok(None);
    };
    let url = Url::parse(&value).map_err(|_| invalid())?;
    if value.trim() != value
        || value.contains('\\')
        || !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() == "/"
        || !matches!(
            url.host_str(),
            Some(
                "img1.kuwo.cn"
                    | "img2.kuwo.cn"
                    | "img3.kuwo.cn"
                    | "img4.kuwo.cn"
                    | "img1.kwcdn.kuwo.cn"
            )
        )
    {
        return Err(invalid());
    }
    Ok(Some(value))
}
pub(in crate::client::native) fn unsigned<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<Option<u64>, D::Error> {
    deserialize_code(d)?
        .map(|v| {
            v.parse::<u64>()
                .ok()
                .filter(|n| n.to_string() == v)
                .ok_or_else(|| serde::de::Error::custom("invalid library number"))
        })
        .transpose()
}
fn sequence<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<i32>, D::Error> {
    deserialize_code(d)?
        .map(|v| {
            v.parse::<i32>()
                .ok()
                .filter(|n| n.to_string() == v)
                .ok_or_else(|| serde::de::Error::custom("invalid library sequence"))
        })
        .transpose()
}
fn boolean<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<bool>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    match value {
        None => Ok(None),
        Some(serde_json::Value::Bool(v)) => Ok(Some(v)),
        Some(serde_json::Value::String(v)) if v.eq_ignore_ascii_case("true") => Ok(Some(true)),
        Some(serde_json::Value::String(v)) if v.eq_ignore_ascii_case("false") => Ok(Some(false)),
        _ => Err(serde::de::Error::custom("invalid library visibility")),
    }
}
