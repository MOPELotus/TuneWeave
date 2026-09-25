//! The official native song-list reader, bound to a validated collection directory.
use super::*;
use library::dto::unsigned;

const HOST: &str = "mobilist.kuwo.cn";
const PAGE_SIZE: usize = 100;

impl KuwoClient {
    pub(super) async fn native_collected_playlist_traversal(
        &self,
        input: &KuwoNativeSessionInput,
        id: &str,
        expected_count: Option<u64>,
        check: &mut (impl FnMut() -> Result<()> + Send),
    ) -> Result<(Vec<Track>, u64)> {
        let mut all = Vec::new();
        let mut total = None;
        let mut bytes = 0;
        for pn in 0..(MAX_TRACKS / PAGE_SIZE) as u64 {
            check()?;
            let query = query(input, id, pn)?;
            let target = format!("{}?{query}", self.native_target(HOST, COLLECTED_PATH));
            let response = self
                .native_get_with_metadata(
                    HOST,
                    COLLECTED_PATH,
                    "native_collected_playlist",
                    target,
                    Some(session_metadata(input)?),
                    |bytes| parse(bytes, input, id, pn),
                )
                .await;
            check()?;
            let page = response?;
            if total.is_some_and(|n| n != page.total)
                || expected_count.is_some_and(|n| n != page.total)
            {
                return Err(changed());
            }
            // The upstream also returns an empty success for nonexistent IDs.
            // A directory with an unknown count cannot establish a valid empty list.
            if page.total == 0 && expected_count != Some(0) {
                return Err(invalid());
            }
            total = Some(page.total);
            bytes += serde_json::to_vec(&page.tracks)
                .map_err(|_| invalid())?
                .len();
            if bytes > MAX_BYTES || all.len() + page.tracks.len() > MAX_TRACKS {
                return Err(invalid());
            }
            all.extend(page.tracks);
            if all.len() as u64 == page.total {
                return Ok((all, pn + 1));
            }
        }
        Err(invalid())
    }
}

fn query(input: &KuwoNativeSessionInput, id: &str, pn: u64) -> Result<String> {
    // The shared o() context is the same as for the collected directory. Replace
    // directory-specific fields; uid here is the installation ID, not loginUid.
    let context = library::query(input, Section::Saved, 0, "")?;
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in url::form_urlencoded::parse(context.as_bytes()) {
        if !matches!(key.as_ref(), "f" | "type" | "uid" | "count" | "start") {
            q.append_pair(&key, &value);
        }
    }
    q.extend_pairs([
        ("type", "songlist"),
        ("uid", input.device_id()),
        ("vipsec", "1"),
        ("kubb", "2"),
        ("fs", "1"),
        ("showtype", "1"),
        ("id", id),
        ("sorttype", "0"),
        ("apiv", "0"),
        ("pn", &pn.to_string()),
        ("rn", "100"),
        ("digest", "8"),
        ("hasmv", "1"),
        ("hasinner", "1"),
        ("hasad", "1"),
        ("hsy", "1"),
        ("isnew", "2"),
        ("newcate", "1"),
        ("supportfan", "1"),
        ("isg", "1"),
        ("aioper", "0"),
    ]);
    // isvip is a presentation hint derived from a separate membership model.
    // Omit the hint instead of making up the selected account's membership.
    Ok(q.finish())
}

#[derive(Deserialize)]
struct Response {
    #[serde(default, deserialize_with = "unsigned")]
    code: Option<u64>,
    msg: Option<String>,
    data: Data,
}
#[derive(Deserialize)]
struct Data {
    #[serde(default, deserialize_with = "deserialize_code")]
    id: Option<String>,
    #[serde(default, deserialize_with = "unsigned")]
    count: Option<u64>,
    #[serde(default, deserialize_with = "unsigned")]
    total: Option<u64>,
    #[serde(default, deserialize_with = "unsigned")]
    pn: Option<u64>,
    #[serde(default, deserialize_with = "unsigned")]
    rn: Option<u64>,
    #[serde(default, rename = "type", deserialize_with = "unsigned")]
    kind: Option<u64>,
    #[serde(default, deserialize_with = "unsigned")]
    sorttype: Option<u64>,
    musiclist: Vec<Song>,
}
#[derive(Deserialize)]
struct Song {
    #[serde(deserialize_with = "unsigned")]
    rid: Option<u64>,
    name: String,
    artist: Option<String>,
    album: Option<String>,
    img: Option<String>,
    #[serde(default, deserialize_with = "unsigned")]
    artistid: Option<u64>,
    #[serde(default, deserialize_with = "unsigned")]
    albumid: Option<u64>,
    #[serde(default, deserialize_with = "unsigned")]
    duration: Option<u64>,
    #[serde(default, deserialize_with = "unsigned")]
    isstar: Option<u64>,
    #[serde(default, deserialize_with = "unsigned")]
    content_type: Option<u64>,
    #[serde(default, deserialize_with = "unsigned")]
    digest: Option<u64>,
}
pub(super) struct Contents {
    pub total: u64,
    pub tracks: Vec<Track>,
}
pub(super) fn parse(
    bytes: &[u8],
    input: &KuwoNativeSessionInput,
    id: &str,
    pn: u64,
) -> Result<Contents> {
    let response: Response = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if response.code != Some(0) || response.msg.as_deref().is_some_and(|v| v != "ok") {
        return Err(invalid());
    }
    let data = response.data;
    let total = data
        .total
        .filter(|n| *n <= MAX_TRACKS as u64)
        .ok_or_else(invalid)?;
    let offset = pn.checked_mul(PAGE_SIZE as u64).ok_or_else(invalid)?;
    if data.id.as_deref().is_some_and(|v| v != id)
        || data.pn != Some(pn)
        || data.rn != Some(PAGE_SIZE as u64)
        || data.kind != Some(0)
        || data.sorttype != Some(0)
        || data.count != Some(data.musiclist.len() as u64)
        || data.musiclist.len() as u64 != total.saturating_sub(offset).min(PAGE_SIZE as u64)
        || (pn > 0 && offset >= total)
    {
        return Err(invalid());
    }
    let tracks = data
        .musiclist
        .into_iter()
        .map(|song| {
            if song.digest.is_some_and(|v| v != 15) {
                return Err(TuneWeaveError::new(
                    ErrorCode::CapabilityNotSupported,
                    "Kuwo collected playlist contains a non-music item",
                )
                .with_platform(Platform::Kuwo));
            }
            dto::track(
                dto::Song {
                    id: song.rid,
                    name: song.name,
                    artist: song.artist,
                    album: song.album,
                    albumpic: song.img,
                    artistid: song.artistid,
                    albumid: song.albumid,
                    duration: song.duration,
                    isstar: song.isstar,
                    content_type: song.content_type,
                },
                input,
                "native_collected_playlist",
            )
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Contents { total, tracks })
}
