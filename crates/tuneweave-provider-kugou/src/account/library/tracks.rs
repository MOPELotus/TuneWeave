//! Current native v3 file list. File IDs identify occurrences, never catalogue tracks.
use super::*;
use md5::{Digest, Md5};
use tuneweave_core::{AlbumSummary, Track};

pub(crate) const TRACK_PAGE_SIZE: usize = 300;
pub(crate) const MAX_TRACK_PAGES: u32 = 128;

#[derive(Deserialize)]
struct TrackEnvelope<T> {
    #[serde(default, deserialize_with = "present_status")]
    status: Option<i64>,
    error_code: i64,
    data: Option<T>,
}
fn present_status<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<i64>, D::Error> {
    i64::deserialize(d).map(Some)
}

#[derive(Serialize, Deserialize)]
struct WireTracks {
    userid: Option<Number>,
    listid: Option<Number>,
    #[serde(rename = "type")]
    kind: Option<Number>,
    list_ver: Number,
    count: Number,
    page: Option<Number>,
    pagesize: Option<Number>,
    info: Option<Vec<WireTrack>>,
}

#[derive(Serialize, Deserialize)]
struct WireTrack {
    fileid: Number,
    sort: Option<Number>,
    mixsongid: Option<Number>,
    add_mixsongid: Option<Number>,
    audio_id: Option<Number>,
    name: Option<String>,
    timelen: Option<Number>,
    collecttime: Option<Number>,
    hash: Option<String>,
    mvhash: Option<String>,
    size: Option<Number>,
    bitrate: Option<Number>,
    privilege: Option<Number>,
    feetype: Option<Number>,
    album_id: Option<Number>,
    albuminfo: Option<WireAlbum>,
    remark: Option<String>,
    cover: Option<String>,
    #[serde(default)]
    singerinfo: Vec<WireSinger>,
}
#[derive(Serialize, Deserialize)]
struct WireSinger {
    id: Option<Number>,
    name: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct WireAlbum {
    id: Option<Number>,
    name: Option<String>,
}

pub(crate) struct TrackRow {
    pub(crate) file_id: u64,
    pub(crate) metadata_fingerprint: String,
    pub(crate) sort: Option<u64>,
    // Unresolvable occurrences remain in the raw snapshot for future list editing.
    pub(crate) track: Option<Track>,
}
pub(crate) struct TrackPage {
    pub(crate) version: u64,
    pub(crate) total: u64,
    pub(crate) rows: Vec<TrackRow>,
}

impl KugouClient {
    pub(crate) async fn native_library_tracks_page(
        &self,
        session: &NativeSession,
        list_id: u64,
        kind: u8,
        page: u32,
    ) -> Result<TrackPage> {
        validate_session(session)?;
        if list_id == 0 || kind > 1 || !(1..=MAX_TRACK_PAGES).contains(&page) {
            return Err(error(
                ErrorCode::InvalidRequest,
                "KuGou native track list request is invalid",
            ));
        }
        #[derive(Serialize)]
        struct Body<'a> {
            userid: u64,
            token: &'a str,
            listid: u64,
            #[serde(rename = "type")]
            kind: u8,
            page: u32,
            pagesize: usize,
            area_code: u8,
            allplatform: u8,
            show_cover: u8,
        }
        let body = crypto::encode(&Body {
            userid: session.user_id.parse().map_err(|_| malformed())?,
            token: &session.token,
            listid: list_id,
            kind,
            page,
            pagesize: TRACK_PAGE_SIZE,
            area_code: 1,
            allplatform: 1,
            show_cover: 1,
        })?;
        self.native_post(
            Endpoint::LibraryTracks,
            session,
            now_ms()? / 1000,
            body,
            |bytes| parse(bytes, &session.user_id, list_id, kind, page),
        )
        .await
    }
}

fn parse(bytes: &[u8], uid: &str, list_id: u64, kind: u8, page: u32) -> Result<TrackPage> {
    // Some current v3 clients receive error_code=0 without status. An explicit failure
    // always wins; a missing info array is empty only when the upstream count is zero.
    let status: TrackEnvelope<IgnoredAny> =
        serde_json::from_slice(bytes).map_err(|_| malformed_at("envelope_json"))?;
    if status.status.is_some_and(|v| v != 1) || status.error_code != 0 {
        let code = if status.error_code == 20017 {
            ErrorCode::AuthenticationRequired
        } else {
            ErrorCode::UpstreamError
        };
        return Err(
            error(code, "KuGou native track list request was rejected").with_details(json!({
                "response_stage": "business_status",
                "platform_code": status.error_code
            })),
        );
    }
    let envelope: TrackEnvelope<WireTracks> =
        serde_json::from_slice(bytes).map_err(|_| malformed_at("track_data_schema"))?;
    let wire = envelope
        .data
        .ok_or_else(|| malformed_at("track_data_missing"))?;
    if wire.userid.is_some_and(|v| v.0.to_string() != uid)
        || wire.listid.is_some_and(|v| v.0 != list_id)
        || wire.kind.is_some_and(|v| v.0 != u64::from(kind))
    {
        return Err(identity_conflict());
    }
    let info = match wire.info {
        Some(info) => info,
        None if wire.count.0 == 0 => Vec::new(),
        None => {
            return Err(malformed_at("track_data_schema").with_details(json!({
                "response_stage": "track_data_schema",
                "response_fields": ["info"],
            })));
        }
    };
    if info.len() > TRACK_PAGE_SIZE
        || wire.page.is_some_and(|v| v.0 != u64::from(page))
        || wire.pagesize.is_some_and(|v| v.0 != TRACK_PAGE_SIZE as u64)
    {
        return Err(malformed());
    }
    let mut seen = BTreeSet::new();
    let rows = info
        .into_iter()
        .map(|entry| {
            let file_id = positive(entry.fileid).map_err(|_| malformed_at("track_row"))?;
            if !seen.insert(file_id) {
                return Err(malformed_at("track_row"));
            }
            let sort = entry.sort.map(|v| v.0);
            let mut content =
                serde_json::to_value(&entry).map_err(|_| malformed_at("track_row"))?;
            content
                .as_object_mut()
                .ok_or_else(|| malformed_at("track_row"))?
                .remove("sort");
            let metadata_fingerprint = format!(
                "{:x}",
                Md5::digest(serde_json::to_vec(&content).map_err(|_| malformed_at("track_row"))?)
            );
            let track =
                map_track(entry).map_err(|error| with_response_stage(error, "track_row"))?;
            Ok(TrackRow {
                file_id,
                metadata_fingerprint,
                sort,
                track,
            })
        })
        .collect::<Result<_>>()?;
    Ok(TrackPage {
        version: wire.list_ver.0,
        total: wire.count.0,
        rows,
    })
}

fn malformed_at(response_stage: &'static str) -> TuneWeaveError {
    with_response_stage(malformed(), response_stage)
}

fn with_response_stage(mut error: TuneWeaveError, response_stage: &'static str) -> TuneWeaveError {
    if !error.details.is_object() {
        error.details = json!({});
    }
    error.details["response_stage"] = json!(response_stage);
    error
}

fn optional_id(value: Option<Number>) -> Option<u64> {
    value.map(|v| v.0).filter(|v| *v > 0)
}
fn hash(value: Option<String>) -> Result<Option<String>> {
    let value = text(value, 32)?;
    if value
        .as_ref()
        .is_some_and(|v| v.len() != 32 || !v.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(malformed());
    }
    Ok(value)
}
fn map_track(entry: WireTrack) -> Result<Option<Track>> {
    let name = text(entry.name, 8192)?;
    let track_id = optional_id(entry.mixsongid).or(optional_id(entry.add_mixsongid));
    let mut extensions = Extensions::from([
        ("backend".into(), json!("native_cloudlist_tracks_v3")),
        ("file_id".into(), json!(entry.fileid)),
    ]);
    for (key, value) in [
        ("sort", entry.sort),
        ("collected_at", entry.collecttime),
        ("size", entry.size),
        ("bitrate", entry.bitrate),
        ("privilege", entry.privilege),
        ("fee_type", entry.feetype),
    ] {
        if let Some(value) = value {
            extensions.insert(key.into(), json!(value));
        }
    }
    for (key, value) in [
        ("hash", hash(entry.hash)?),
        ("mv_hash", hash(entry.mvhash)?),
    ] {
        if let Some(value) = value {
            extensions.insert(key.into(), json!(value));
        }
    }
    if entry.singerinfo.len() > 100 {
        return Err(malformed());
    }
    let mut artists = Vec::new();
    for singer in entry.singerinfo {
        if let Some(name) = text(singer.name, 1024)? {
            let resource_ref = optional_id(singer.id)
                .map(|id| {
                    ResourceRef::new(Platform::Kugou, id.to_string()).map_err(|_| malformed())
                })
                .transpose()?;
            artists.push(ArtistSummary { resource_ref, name });
        }
    }
    let nested_id = entry.albuminfo.as_ref().and_then(|v| optional_id(v.id));
    let outer_id = optional_id(entry.album_id);
    if nested_id.is_some() && outer_id.is_some() && nested_id != outer_id {
        return Err(identity_conflict());
    }
    let album_id = nested_id.or(outer_id);
    let album_name =
        text(entry.albuminfo.and_then(|a| a.name), 8192)?.or(text(entry.remark, 8192)?);
    let cover = text(entry.cover, 8192)?
        .map(|v| normalize_image_url(&v).ok_or_else(malformed))
        .transpose()?;
    let album = if album_id.is_some() || album_name.is_some() {
        Some(AlbumSummary {
            resource_ref: album_id
                .map(|id| {
                    ResourceRef::new(Platform::Kugou, id.to_string()).map_err(|_| malformed())
                })
                .transpose()?,
            name: album_name.unwrap_or_default(),
            cover_url: cover,
        })
    } else {
        None
    };
    let (Some(id), Some(mut name)) = (track_id, name) else {
        return Ok(None);
    };
    if let Some((prefix, title)) = name.split_once(" - ") {
        let joined = artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join("、");
        if !title.trim().is_empty()
            && (prefix == joined || artists.iter().any(|a| a.name == prefix))
        {
            name = title.trim().to_owned();
        }
    }
    extensions.insert("album_audio_id".into(), json!(id.to_string()));
    for (key, value) in [
        ("audio_id", optional_id(entry.audio_id)),
        (
            "added_album_audio_id",
            optional_id(entry.add_mixsongid).filter(|v| *v != id),
        ),
    ] {
        if let Some(value) = value {
            extensions.insert(key.into(), json!(value.to_string()));
        }
    }
    let mut track = Track::new(
        ResourceRef::new(Platform::Kugou, id.to_string()).map_err(|_| malformed())?,
        name,
    );
    track.artists = artists;
    track.album = album;
    track.duration_ms = optional_id(entry.timelen);
    // Catalogue metadata is not a fresh account playback authorization.
    track.extensions = extensions;
    Ok(Some(track))
}

#[cfg(test)]
mod tests;
