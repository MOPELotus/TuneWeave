use super::*;
use library::dto::{picture, text, unsigned};

#[derive(Deserialize)]
struct Response {
    #[serde(default, deserialize_with = "unsigned")]
    errcode: Option<u64>,
    result: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    uid: Option<String>,
    #[serde(default, deserialize_with = "deserialize_code")]
    pid: Option<String>,
    #[serde(default, deserialize_with = "unsigned")]
    pagenum: Option<u64>,
    #[serde(default, deserialize_with = "flag")]
    songchange: Option<bool>,
    info: Option<Info>,
}
#[derive(Deserialize)]
struct Info {
    musiclist: Vec<Song>,
}
#[derive(Deserialize)]
pub(super) struct Song {
    #[serde(deserialize_with = "unsigned")]
    pub(super) id: Option<u64>,
    pub(super) name: String,
    pub(super) artist: Option<String>,
    pub(super) album: Option<String>,
    pub(super) albumpic: Option<String>,
    #[serde(default, deserialize_with = "unsigned")]
    pub(super) artistid: Option<u64>,
    #[serde(default, deserialize_with = "unsigned")]
    pub(super) albumid: Option<u64>,
    #[serde(default, deserialize_with = "unsigned")]
    pub(super) duration: Option<u64>,
    #[serde(default, deserialize_with = "unsigned")]
    pub(super) isstar: Option<u64>,
    #[serde(default, deserialize_with = "unsigned")]
    pub(super) content_type: Option<u64>,
}
pub(super) struct Contents {
    pub pages: u64,
    pub tracks: Vec<Track>,
}

pub(super) fn parse(
    bytes: &[u8],
    input: &KuwoNativeSessionInput,
    id: &str,
    known_empty: bool,
) -> Result<Contents> {
    let response: Response = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if response.errcode != Some(0)
        || response.result.as_deref().is_some_and(|v| v != "ok")
        || response
            .uid
            .as_deref()
            .is_some_and(|v| v != input.user_id())
        || response.pid.as_deref().is_some_and(|v| v != id)
    {
        return Err(invalid());
    }
    let changed = response.songchange.ok_or_else(invalid)?;
    let songs = match response.info {
        Some(info) => info.musiclist,
        None if !changed && known_empty => Vec::new(),
        None => return Err(invalid()),
    };
    // sig=0 describes an empty local cache. "Unchanged" supplies no usable
    // nonempty cache. Only independently observed zero-count metadata supports it.
    if !changed && (!known_empty || !songs.is_empty()) {
        return Err(invalid());
    }
    let pages = match response.pagenum {
        None | Some(0) if known_empty && songs.is_empty() => 1,
        Some(n) if (1..=MAX_PAGES).contains(&n) => n,
        _ => return Err(invalid()),
    };
    if songs.len() > MAX_TRACKS {
        return Err(invalid());
    }
    let tracks = songs
        .into_iter()
        .map(|song| track(song, input, "native_created_playlist"))
        .collect::<Result<Vec<_>>>()?;
    Ok(Contents { pages, tracks })
}

pub(super) fn track(song: Song, input: &KuwoNativeSessionInput, backend: &str) -> Result<Track> {
    if song.isstar.is_some_and(|v| v != 0) || song.content_type.is_some_and(|v| v != 0) {
        return Err(TuneWeaveError::new(
            ErrorCode::CapabilityNotSupported,
            "Kuwo native playlist contains a non-music item",
        )
        .with_platform(Platform::Kuwo));
    }
    let id = song
        .id
        .filter(|n| *n > 0 && *n <= i64::MAX as u64)
        .ok_or_else(invalid)?
        .to_string();
    let name = text(Some(song.name), 1024, false, input)?
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(invalid)?;
    let mut track = Track::new(
        ResourceRef::new(Platform::Kuwo, id).map_err(|_| invalid())?,
        name,
    );
    if let Some(name) = text(song.artist, 4096, false, input)? {
        // Native fields contain one credit string and one primary artist ID, not
        // an aligned list of artist IDs. Preserve compound credits literally.
        let resource_ref = if name.contains('&') {
            None
        } else {
            reference(song.artistid)?
        };
        track.artists.push(ArtistSummary { resource_ref, name });
    }
    let cover_url = picture(song.albumpic, input)?;
    if let Some(name) = text(song.album, 1024, false, input)? {
        track.album = Some(AlbumSummary {
            resource_ref: reference(song.albumid)?,
            name,
            cover_url,
        });
    }
    track.duration_ms = song
        .duration
        .map(|n| n.checked_mul(1000).ok_or_else(invalid))
        .transpose()?;
    track.extensions.insert("backend".into(), json!(backend));
    // No token, payInfo, N_MINFO, membership, or download flag is copied. A
    // directory record does not establish any playback or download entitlement.
    Ok(track)
}
fn reference(id: Option<u64>) -> Result<Option<ResourceRef>> {
    id.filter(|n| *n != 0)
        .map(|n| {
            if n > i64::MAX as u64 {
                return Err(invalid());
            }
            ResourceRef::new(Platform::Kuwo, n.to_string()).map_err(|_| invalid())
        })
        .transpose()
}
fn flag<'de, D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Option<bool>, D::Error> {
    match Option::<serde_json::Value>::deserialize(d)? {
        None => Ok(None),
        Some(serde_json::Value::Bool(value)) => Ok(Some(value)),
        Some(serde_json::Value::String(value)) if value.eq_ignore_ascii_case("true") => {
            Ok(Some(true))
        }
        Some(serde_json::Value::String(value)) if value.eq_ignore_ascii_case("false") => {
            Ok(Some(false))
        }
        // Current negative containers encode false as numeric zero.
        Some(serde_json::Value::Number(value)) if value.as_u64() == Some(0) => Ok(Some(false)),
        _ => Err(serde::de::Error::custom("invalid native playlist flag")),
    }
}
