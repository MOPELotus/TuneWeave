//! Native cloud-list full synchronization. Public offset lists use a different protocol.
use super::*;
use std::collections::BTreeSet;
use tuneweave_core::{ArtistSummary, Extensions, Playlist, ResourceRef};

pub(crate) mod cover;
pub(crate) mod management;
pub(crate) mod tracks;
pub(crate) mod write;

pub(crate) const PAGE_SIZE: usize = 30;
pub(crate) const MAX_PAGES: u32 = 128;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "Value", into = "u64")]
pub(in crate::account) struct Number(pub(in crate::account) u64);
impl TryFrom<Value> for Number {
    type Error = &'static str;
    fn try_from(value: Value) -> std::result::Result<Self, Self::Error> {
        let number = match value {
            Value::Number(n) => n.as_u64(),
            Value::String(s) => s.parse::<u64>().ok().filter(|n| n.to_string() == s),
            _ => None,
        };
        number
            .map(Self)
            .ok_or("expected a canonical unsigned integer")
    }
}
impl From<Number> for u64 {
    fn from(value: Number) -> Self {
        value.0
    }
}

#[derive(Debug, Deserialize)]
struct WirePage {
    userid: Option<Number>,
    total_ver: Number,
    list_count: Option<Number>,
    collect_count: Option<Number>,
    album_count: Option<Number>,
    page: Option<Number>,
    pagesize: Option<Number>,
    info: Vec<Entry>,
}

#[derive(Debug, Deserialize)]
struct Entry {
    listid: Number,
    #[serde(rename = "type")]
    kind: Number,
    is_del: Option<Number>,
    status: Option<Number>,
    name: Option<String>,
    intro: Option<String>,
    pic: Option<String>,
    tags: Option<String>,
    global_collection_id: Option<String>,
    list_ver: Option<Number>,
    count: Option<Number>,
    m_count: Option<Number>,
    list_create_userid: Option<Number>,
    list_create_listid: Option<Number>,
    list_create_gid: Option<String>,
    list_create_username: Option<String>,
    is_def: Option<Number>,
    is_pri: Option<Number>,
    is_mutual: Option<Number>,
    is_publish: Option<Number>,
    is_drop: Option<Number>,
    sort: Option<Number>,
    create_time: Option<Number>,
    update_time: Option<Number>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LibraryVersion {
    pub(crate) total_ver: u64,
    counts: [Option<Number>; 3],
}
impl LibraryVersion {
    pub(crate) fn extensions(&self) -> Extensions {
        let mut result = Extensions::from([("total_ver".into(), json!(self.total_ver))]);
        for (key, value) in ["list_count", "collect_count", "album_count"]
            .into_iter()
            .zip(self.counts)
        {
            if let Some(value) = value {
                result.insert(key.into(), json!(value));
            }
        }
        result
    }
}

pub(crate) struct LibraryRow {
    pub(crate) list_id: u64,
    pub(crate) kind: u8,
    pub(crate) playlist: Option<Playlist>,
}
pub(crate) struct LibraryPage {
    pub(crate) version: LibraryVersion,
    pub(crate) rows: Vec<LibraryRow>,
}

impl KugouClient {
    pub(crate) async fn native_library_page(
        &self,
        session: &NativeSession,
        page: u32,
    ) -> Result<LibraryPage> {
        validate_session(session)?;
        if !(1..=MAX_PAGES).contains(&page) {
            return Err(error(
                ErrorCode::InvalidRequest,
                "KuGou library page is invalid",
            ));
        }
        #[derive(Serialize)]
        struct Body<'a> {
            userid: u64,
            token: &'a str,
            total_ver: u8,
            #[serde(rename = "type")]
            kind: u8,
            page: u32,
            pagesize: usize,
        }
        let body = crypto::encode(&Body {
            userid: session.user_id.parse().map_err(|_| malformed())?,
            token: &session.token,
            total_ver: 0,
            kind: 2,
            page,
            pagesize: PAGE_SIZE,
        })?;
        self.native_post(
            Endpoint::Library,
            session,
            now_ms()? / 1000,
            body,
            |bytes| parse(bytes, &session.user_id, page),
        )
        .await
    }
}

fn parse(bytes: &[u8], uid: &str, page: u32) -> Result<LibraryPage> {
    let wire: WirePage = data(bytes)?;
    if wire.userid.is_some_and(|v| v.0.to_string() != uid) {
        return Err(identity_conflict());
    }
    if wire.info.len() > PAGE_SIZE
        || wire.page.is_some_and(|v| v.0 != u64::from(page))
        || wire.pagesize.is_some_and(|v| v.0 != PAGE_SIZE as u64)
    {
        return Err(malformed());
    }
    let version = LibraryVersion {
        total_ver: wire.total_ver.0,
        counts: [wire.list_count, wire.collect_count, wire.album_count],
    };
    let mut seen = BTreeSet::new();
    let mut rows = Vec::with_capacity(wire.info.len());
    for entry in wire.info {
        let list_id = positive(entry.listid)?;
        let kind = u8::try_from(entry.kind.0).map_err(|_| malformed())?;
        if kind > 1 || !seen.insert((list_id, kind)) {
            return Err(malformed());
        }
        let deleted = flag(entry.is_del)?.unwrap_or(false);
        let playlist = if deleted {
            None
        } else {
            Some(map(entry, uid, kind, &version)?)
        };
        rows.push(LibraryRow {
            list_id,
            kind,
            playlist,
        });
    }
    Ok(LibraryPage { version, rows })
}

fn positive(value: Number) -> Result<u64> {
    if value.0 == 0 {
        Err(malformed())
    } else {
        Ok(value.0)
    }
}
fn flag(value: Option<Number>) -> Result<Option<bool>> {
    value
        .map(|v| match v.0 {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(malformed()),
        })
        .transpose()
}
fn text(value: Option<String>, limit: usize) -> Result<Option<String>> {
    match value {
        Some(value)
            if value.len() > limit
                || value
                    .chars()
                    .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')) =>
        {
            Err(malformed())
        }
        Some(value) if value.trim().is_empty() => Ok(None),
        value => Ok(value),
    }
}
fn gid(value: Option<String>) -> Result<Option<String>> {
    let value = text(value, 256)?;
    if value
        .as_ref()
        .is_some_and(|v| !v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
    {
        return Err(malformed());
    }
    Ok(value)
}

fn map(entry: Entry, uid: &str, kind: u8, version: &LibraryVersion) -> Result<Playlist> {
    let source_uid = entry.list_create_userid.map(positive).transpose()?;
    if kind == 0 && source_uid.is_some_and(|v| v.to_string() != uid) {
        return Err(identity_conflict());
    }
    let source_list_id = entry.list_create_listid.map(positive).transpose()?;
    if kind == 0 && source_list_id.is_some_and(|v| v != entry.listid.0) {
        return Err(identity_conflict());
    }
    let global_gid = gid(entry.global_collection_id)?;
    let source_gid = gid(entry.list_create_gid)?;
    if kind == 0 && global_gid.is_some() && source_gid.is_some() && global_gid != source_gid {
        return Err(identity_conflict());
    }
    let private = flag(entry.is_pri)?;
    let mutual = flag(entry.is_mutual)?;
    let published = flag(entry.is_publish)?;
    let dropped = flag(entry.is_drop)?;
    let mut extensions = version.extensions();
    // Preserve present empty strings and exact delimiters for metadata updates.
    // Missing fields remain missing; a rename must not silently clear them.
    let mut raw_metadata = serde_json::Map::new();
    if let Some(value) = &entry.intro {
        raw_metadata.insert("intro".into(), json!(value));
    }
    if let Some(value) = &entry.tags {
        raw_metadata.insert("tags".into(), json!(value));
    }
    extensions.insert("native_metadata".into(), Value::Object(raw_metadata));
    extensions.extend([
        ("backend".into(), json!("native_cloudlist_v8")),
        ("library_owner_id".into(), json!(uid)),
        ("list_id".into(), json!(entry.listid)),
        ("list_type".into(), json!(kind)),
        (
            "library_section".into(),
            json!(if kind == 0 { "created" } else { "collected" }),
        ),
    ]);
    for (key, value) in [
        ("list_ver", entry.list_ver),
        ("count", entry.count),
        ("m_count", entry.m_count),
        ("status", entry.status),
        ("is_def", entry.is_def),
        ("sort", entry.sort),
        ("create_time", entry.create_time),
        ("update_time", entry.update_time),
    ] {
        if let Some(value) = value {
            extensions.insert(key.into(), json!(value));
        }
    }
    for (key, value) in [
        ("is_private", private),
        ("is_mutual", mutual),
        ("is_published", published),
        ("is_dropped", dropped),
    ] {
        if let Some(value) = value {
            extensions.insert(key.into(), json!(value));
        }
    }
    for (key, value) in [
        ("global_collection_id", global_gid),
        ("source_global_collection_id", source_gid),
        ("source_user_id", source_uid.map(|v| v.to_string())),
        ("source_list_id", source_list_id.map(|v| v.to_string())),
    ] {
        if let Some(value) = value {
            extensions.insert(key.into(), json!(value));
        }
    }
    if kind == 0 {
        if let Some(system) = entry.is_def.and_then(|v| match v.0 {
            1 => Some("default_collection"),
            2 => Some("liked_tracks"),
            _ => None,
        }) {
            extensions.insert("system_playlist".into(), json!(system));
        }
    }
    let tags = text(entry.tags, 4096)?
        .map(|v| v.split(',').map(str::to_owned).collect::<Vec<_>>())
        .unwrap_or_default();
    if tags.len() > 128 {
        return Err(malformed());
    }
    let tags = tags
        .into_iter()
        .map(|v| text(Some(v), 256)?.ok_or_else(malformed))
        .collect::<Result<Vec<_>>>()?;
    let cover_url = text(entry.pic, 4096)?
        .map(|v| normalize_image_url(&v).ok_or_else(malformed))
        .transpose()?;
    let creator = text(entry.list_create_username, 512)?.map(|name| ArtistSummary {
        resource_ref: None,
        name,
    });
    let id = format!("cloudlist:{uid}:{kind}:{}", entry.listid.0);
    Ok(Playlist {
        resource_ref: ResourceRef::new(Platform::Kugou, &id).map_err(|_| malformed())?,
        platform: Platform::Kugou,
        id,
        name: text(entry.name, 1024)?.ok_or_else(malformed)?,
        description: text(entry.intro, 16384)?.unwrap_or_default(),
        cover_url,
        creator,
        track_count: entry.count.or(entry.m_count).map(|v| v.0),
        tags,
        subscribed: (kind == 1).then_some(true),
        created_at: None,
        updated_at: None,
        extensions,
    })
}

#[cfg(test)]
mod tests;
