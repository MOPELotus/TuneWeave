use super::*;
use catalog::Unsigned;
use std::collections::BTreeSet;
use tuneweave_core::Album;

mod hifi;

const ENDPOINT: &str = "https://searchlist.kuwo.cn/r.s";
const RESPONSE_LIMIT: u64 = 2 * 1024 * 1024;
const MAX_TRACKS: u64 = 1000;

pub(crate) struct AlbumDetail {
    pub album: Album,
    pub tracks: Vec<Track>,
}

#[derive(Serialize)]
struct Query<'a> {
    stype: &'static str,
    albumid: &'a str,
    show_copyright_off: u8,
    alflac: u8,
    vipver: u8,
    sortby: u8,
    newver: u8,
    mobi: u8,
}

impl KuwoClient {
    pub(crate) async fn album_detail(&self, id: &str) -> Result<AlbumDetail> {
        let started = Instant::now();
        let mut status = None;
        let result = async {
            // The current official album page uses a direct unsigned Axios request.
            let response = self
                .http
                .get(self.web_target(ENDPOINT))
                .header(ACCEPT, "application/json, text/javascript")
                .header(REFERER, format!("https://www.kuwo.cn/album_detail/{id}"))
                .query(&Query {
                    stype: "albuminfo",
                    albumid: id,
                    show_copyright_off: 1,
                    alflac: 1,
                    vipver: 1,
                    sortby: 1,
                    newver: 1,
                    mobi: 1,
                })
                .send()
                .await
                .map_err(kuwo_network_error)?;
            status = Some(response.status());
            if !response.status().is_success() {
                return Err(kuwo_http_error(response.status()));
            }
            let mime = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next())
                .map(str::trim);
            // This endpoint labels its plain JSON as JavaScript. Never evaluate it.
            if !mime.is_some_and(|value| {
                value.eq_ignore_ascii_case("application/json")
                    || value.eq_ignore_ascii_case("text/javascript")
            }) {
                return Err(invalid());
            }
            let bytes =
                read_bounded_response_with_limit(response, "Kuwo album", RESPONSE_LIMIT).await?;
            parse(&bytes, id)
        }
        .await;
        self.log_upstream_request(
            "album_detail",
            "searchlist.kuwo.cn",
            "/r.s",
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
struct Detail {
    id: Unsigned,
    albumid: Unsigned,
    name: String,
    artist: String,
    artistid: Unsigned,
    songnum: Unsigned,
    musiclist: Vec<Song>,
    #[serde(default)]
    info: String,
    pic: Option<String>,
    #[serde(rename = "pub")]
    published_at: Option<String>,
    company: Option<String>,
    lang: Option<String>,
    content_type: Option<Unsigned>,
    ad_type: Option<String>,
    code: Option<i64>,
}

#[derive(Deserialize)]
struct Song {
    id: Unsigned,
    musicrid: Unsigned,
    #[serde(rename = "albumId")]
    album_id: Unsigned,
    duration: Unsigned,
    allartistid: Option<String>,
    web_albumpic_short: Option<String>,
    #[serde(rename = "MINFO", default)]
    formats: String,
    #[serde(rename = "MVFLAG")]
    mv_flag: Option<Unsigned>,
    #[serde(flatten)]
    detail: KuwoTrackDetail,
}

fn parse(bytes: &[u8], requested: &str) -> Result<AlbumDetail> {
    let raw: Detail = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let id = raw.id.id()?;
    let total = raw.songnum.value()?;
    if id != requested
        || raw.albumid.id()? != requested
        || total > MAX_TRACKS
        || total != raw.musiclist.len() as u64
        || raw.code.is_some_and(|value| value != 200)
        || raw
            .content_type
            .as_ref()
            .map(Unsigned::value)
            .transpose()?
            .is_some_and(|v| v != 0)
        || raw
            .ad_type
            .as_deref()
            .is_some_and(|value| !matches!(value, "" | "0"))
    {
        return Err(invalid());
    }
    let name = catalog::text(&raw.name, 512, false)?;
    if name.is_empty() {
        return Err(invalid());
    }
    let mut extensions = Extensions::from([
        ("backend".into(), json!("current_web_albuminfo")),
        ("complete_read".into(), json!(true)),
    ]);
    catalog::optional_text(&mut extensions, "language", raw.lang.as_deref())?;
    let album = Album {
        resource_ref: kuwo_track_ref(&id)?,
        platform: Platform::Kuwo,
        id: id.clone(),
        name,
        aliases: vec![],
        artists: artists::credits(&raw.artist, &raw.artistid.id()?)?,
        description: catalog::text(&raw.info, 64 * 1024, true)?,
        cover_url: raw.pic.as_deref().and_then(cover),
        published_at: raw
            .published_at
            .as_deref()
            .map(catalog::date)
            .transpose()?
            .flatten(),
        track_count: Some(total),
        company: raw
            .company
            .as_deref()
            .map(|v| catalog::text(v, 512, false))
            .transpose()?
            .filter(|v| !v.is_empty()),
        kind: None,
        extensions,
    };
    let mut seen = BTreeSet::new();
    let mut tracks = Vec::with_capacity(raw.musiclist.len());
    for song in raw.musiclist {
        let rid = song.musicrid.id()?;
        if song.id.id()? != rid || song.album_id.id()? != id || !seen.insert(rid.clone()) {
            return Err(invalid());
        }
        let mut detail = song.detail;
        if detail
            .content_type
            .as_text()
            .is_some_and(|s| !s.is_empty() && s != "0")
            || !matches!(detail.ad_type.as_str(), "" | "0")
        {
            return Err(invalid());
        }
        let primary = detail.artistid.as_text().ok_or_else(invalid)?;
        let full = song.allartistid.as_deref().unwrap_or(&primary);
        if full.split('&').next() != Some(primary.as_str()) {
            return Err(invalid());
        }
        let credits = artists::credits(&detail.artist, full)?;
        let seconds = song.duration.value()?;
        seconds.checked_mul(1000).ok_or_else(invalid)?;
        detail.name = catalog::text(&detail.name, 512, false)?;
        if let Some(flag) = song.mv_flag {
            let flag = flag.value()?;
            if flag > 1 {
                return Err(invalid());
            }
            detail.hasmv = FlexibleText::Number(flag.into());
        }
        detail.musicrid = format!("MUSIC_{rid}");
        detail.rid = FlexibleText::String(rid.clone());
        detail.duration = FlexibleText::Number(seconds.into());
        detail.album = album.name.clone();
        detail.albumid = FlexibleText::String(id.clone());
        detail.albumpic = song
            .web_albumpic_short
            .as_deref()
            .and_then(cover)
            .or_else(|| album.cover_url.clone())
            .unwrap_or_default();
        if song.formats.len() > 16 * 1024 {
            return Err(invalid());
        }
        if song
            .formats
            .split(';')
            .any(|format| format.split(',').any(|part| part == "level:ff"))
        {
            detail.has_lossless = FlexibleBoolean::Boolean(true);
        }
        let mut track = map_track_detail(detail, &rid, "current_web_albuminfo")?;
        track.artists = credits;
        track.extensions.insert("source_album_id".into(), json!(id));
        tracks.push(track);
    }
    Ok(AlbumDetail { album, tracks })
}

fn cover(path: &str) -> Option<String> {
    if path.is_empty()
        || path.len() > 1024
        || path.starts_with('/')
        || !path
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"/_.-".contains(&c))
        || path
            .split('/')
            .any(|part| part.is_empty() || matches!(part, "." | ".."))
    {
        return None;
    }
    normalize_official_image_url(&format!("https://img4.kuwo.cn/star/albumcover/{path}"))
}

fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo album returned an invalid or incomplete response")
}

#[cfg(test)]
mod tests;
