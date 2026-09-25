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
    let response: OwnedResponse = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let mut ids = BTreeSet::new();
    for item in response.plist {
        if let Some(id) = item.id {
            if id == "0" && !matches!(item.kind.as_str(), "GENERAL" | "MYFAVORITE") {
                continue;
            }
            crate::client::native::playlist::validate_id(&id).map_err(|_| invalid())?;
            if !ids.insert(id) {
                return Err(invalid());
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
    let items = match section {
        Section::Created | Section::Favorite | Section::Owned => {
            let response: OwnedResponse = serde_json::from_slice(bytes).map_err(|_| invalid())?;
            if response.errcode != Some(0)
                || response.result.as_deref().is_some_and(|v| v != "ok")
                || response
                    .uid
                    .as_deref()
                    .is_some_and(|v| v != input.user_id())
                || response.plist.len() > MAX_OWNED
            {
                return Err(invalid());
            }
            let mut owned = Vec::new();
            let mut favorites = 0;
            let mut owned_ids = BTreeSet::new();
            for item in response.plist {
                if section != Section::Created
                    && matches!(item.kind.as_str(), "GENERAL" | "MYFAVORITE")
                {
                    let id = item.id.as_deref().ok_or_else(invalid)?;
                    crate::client::native::playlist::validate_id(id).map_err(|_| invalid())?;
                    if !owned_ids.insert(id.to_owned()) {
                        return Err(invalid());
                    }
                }
                match item.kind.as_str() {
                    "GENERAL" if section != Section::Favorite => owned.push(item),
                    "MYFAVORITE" if section != Section::Created => {
                        favorites += 1;
                        if favorites > 1 {
                            return Err(invalid());
                        }
                        owned.push(item);
                    }
                    "GENERAL" | "MYFAVORITE" | "MOBI_DEFAULT" | "PC_DEFAULT" | "RADIO"
                    | "ORDER" => (),
                    _ => return Err(invalid()),
                }
            }
            let ordered = owned.iter().all(|item| item.turn.is_some());
            if ordered {
                owned.sort_by_key(|item| item.turn);
            }
            owned
                .into_iter()
                .map(|item| {
                    let favorite = item.kind == "MYFAVORITE";
                    let mut playlist = playlist(
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
                    )?;
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
                .collect::<Result<Vec<_>>>()?
        }
        Section::Saved => {
            let response: SavedResponse = serde_json::from_slice(bytes).map_err(|_| invalid())?;
            if response.result != "ok"
                || response.errcode.is_some_and(|v| v != 0)
                || response
                    .uid
                    .as_deref()
                    .is_some_and(|v| v != input.user_id())
                || response.data.len() > PAGE_SIZE
            {
                return Err(invalid());
            }
            response
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
                .collect::<Result<Vec<_>>>()?
        }
    };
    let mut seen = BTreeSet::new();
    if items.iter().any(|item| !seen.insert(&item.id)) {
        return Err(invalid());
    }
    Ok(items)
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
        .ok_or_else(invalid)?;
    let name = text(name, 1024, false, input)?
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(invalid)?;
    Ok(Playlist {
        resource_ref: ResourceRef::new(Platform::Kuwo, &id).map_err(|_| invalid())?,
        platform: Platform::Kuwo,
        id,
        name,
        description: text(description, 16 * 1024, true, input)?.unwrap_or_default(),
        cover_url: picture(cover, input)?,
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
