use super::albums::{AlbumDto, map_album};
use super::assets::{Audio, map_audio};
use super::dto::{Number, optional_text, required_text, resource};
use super::openapi::{Endpoint, check_status};
use super::*;
use tuneweave_core::{Album, Artist, ArtistBiographySection};

const TRACK_PAGE_SIZE: u32 = 100;
const ALBUM_PAGE_SIZE: u32 = 30;
const MAX_PAGES: u32 = 128;

#[derive(Deserialize)]
struct Envelope<T> {
    data: T,
    total: Option<Number>,
    extra: Option<Extra>,
}
#[derive(Deserialize)]
struct Extra {
    page_total: Option<Number>,
}
#[derive(Deserialize)]
struct ArtistDto {
    author_id: Number,
    author_name: String,
    intro: Option<String>,
    long_intro: Option<Vec<Biography>>,
    sizable_avatar: Option<String>,
    song_count: Option<Number>,
    album_count: Option<Number>,
    mv_count: Option<Number>,
    fansnums: Option<Number>,
    birthday: Option<String>,
    area_id: Option<Number>,
    is_publish: Option<Number>,
}
#[derive(Deserialize)]
struct Biography {
    title: String,
    content: String,
}

pub(crate) struct Catalogue<T> {
    pub artist: Artist,
    pub items: Vec<T>,
    pub total: u64,
    pub pages: u32,
}

impl KugouClient {
    pub(crate) async fn artist_metadata(&self, id: u64) -> Result<Artist> {
        self.artist_with_device(id, &self.device_identity()?).await
    }
    async fn artist_with_device(&self, id: u64, device: &KugouDeviceIdentity) -> Result<Artist> {
        let bytes = self
            .public_openapi(Endpoint::Artist, &json!({"author_id":id}), device)
            .await?;
        parse_artist(&bytes, id)
    }
    pub(crate) async fn complete_artist_tracks(
        &self,
        id: u64,
        sort: u8,
    ) -> Result<Catalogue<Track>> {
        let device = self.device_identity()?;
        let artist = self.artist_with_device(id, &device).await?;
        let mut items = vec![];
        let mut seen = BTreeSet::new();
        let mut expected = artist.track_count;
        for page in 1..=MAX_PAGES {
            let query = BTreeMap::from([
                ("author_id", id.to_string()),
                ("area_code", "all".into()),
                ("sort", sort.to_string()),
                ("page", page.to_string()),
                ("pagesize", TRACK_PAGE_SIZE.to_string()),
                ("replace_api_version", "1".into()),
                ("mvdata_need", "1".into()),
                ("show_audio_honor", "1".into()),
                ("show_audio_tag", "1".into()),
                ("replace_need", "1".into()),
            ]);
            let bytes = self
                .public_catalogue_get(Endpoint::ArtistTracks, query, &device)
                .await?;
            let (tracks, total) = parse_tracks(&bytes, id, page, sort)?;
            if expected.is_some_and(|v| v != total) {
                return Err(invalid());
            }
            expected = Some(total);
            for track in tracks {
                // A group can contain multiple album versions. Only the actual track ID is unique.
                if !seen.insert(track.id.clone()) {
                    return Err(invalid());
                }
                items.push(track);
            }
            if items.len() as u64 == total {
                return Ok(Catalogue {
                    artist,
                    items,
                    total,
                    pages: page,
                });
            }
        }
        Err(invalid())
    }
    pub(crate) async fn complete_artist_albums(&self, id: u64) -> Result<Catalogue<Album>> {
        let device = self.device_identity()?;
        let artist = self.artist_with_device(id, &device).await?;
        let mut items = vec![];
        let mut seen = BTreeSet::new();
        let mut expected = artist.album_count;
        for page in 1..=MAX_PAGES {
            let bytes = self
                .public_openapi(
                    Endpoint::ArtistAlbums,
                    &json!({
                        "author_id":id,"page":page,"pagesize":ALBUM_PAGE_SIZE,
                        "sort":1,"category":1,"area_code":"all"
                    }),
                    &device,
                )
                .await?;
            let (albums, total) = parse_albums(&bytes, id, page)?;
            if expected.is_some_and(|v| v != total) {
                return Err(invalid());
            }
            expected = Some(total);
            for album in albums {
                if !seen.insert(album.id.clone()) {
                    return Err(invalid());
                }
                items.push(album);
            }
            if items.len() as u64 == total {
                return Ok(Catalogue {
                    artist,
                    items,
                    total,
                    pages: page,
                });
            }
        }
        Err(invalid())
    }
}

fn parse_artist(bytes: &[u8], id: u64) -> Result<Artist> {
    check_status(bytes)?;
    let e: Envelope<ArtistDto> = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let d = e.data;
    if d.author_id.0 != id || id == 0 {
        return Err(invalid());
    }
    let mut sections = vec![];
    let mut length = 0usize;
    for part in d.long_intro.unwrap_or_default() {
        length = length.saturating_add(part.content.len());
        if sections.len() >= 128 || length > 524288 {
            return Err(invalid());
        }
        sections.push(ArtistBiographySection {
            title: required_text(part.title)?,
            text: optional_text(Some(part.content), 262144)?.unwrap_or_default(),
        });
    }
    let mut extensions = Extensions::new();
    for (key, value) in [
        ("fans_count", d.fansnums),
        ("area_id", d.area_id),
        ("is_publish", d.is_publish),
    ] {
        if let Some(value) = value {
            extensions.insert(key.into(), json!(value.0));
        }
    }
    if let Some(birthday) = optional_text(d.birthday, 128)? {
        extensions.insert("birthday".into(), json!(birthday));
    }
    Ok(Artist {
        resource_ref: resource(id.to_string())?,
        platform: Platform::Kugou,
        id: id.to_string(),
        name: required_text(d.author_name)?,
        aliases: vec![],
        description: optional_text(d.intro, 131072)?.unwrap_or_default(),
        biography_sections: sections,
        avatar_url: d.sizable_avatar.as_deref().and_then(normalize_image_url),
        cover_url: None,
        album_count: d.album_count.map(|n| n.0),
        track_count: d.song_count.map(|n| n.0),
        mv_count: d.mv_count.map(|n| n.0),
        video_count: None,
        identities: vec![],
        extensions,
    })
}

#[derive(Deserialize)]
struct TrackData {
    total: Number,
    songs: Vec<Song>,
    input_param: Input,
}
#[derive(Deserialize)]
struct Input {
    sort: u8,
}
#[derive(Deserialize)]
struct Song {
    album_audio_id: Number,
    audio_id: Option<Number>,
    audio_group_id: Option<Number>,
    author_id: Option<Number>,
    audio_name: String,
    album_id: Number,
    album_info: Option<SongAlbum>,
    authors: Vec<SongAuthor>,
    audio_info: Option<Audio>,
    publish_date: Option<String>,
}
#[derive(Deserialize)]
struct SongAlbum {
    album_name: String,
    cover: Option<String>,
}
#[derive(Deserialize)]
struct SongAuthor {
    base: AuthorBase,
}
#[derive(Deserialize)]
struct AuthorBase {
    author_id: Option<Number>,
    author_name: String,
}

fn validate_page(
    total: u64,
    extra: Option<Extra>,
    page: u32,
    count: usize,
    size: u32,
) -> Result<u64> {
    let offset = u64::from(page.checked_sub(1).ok_or_else(invalid)?) * u64::from(size);
    if page > MAX_PAGES
        || total > u64::from(size) * u64::from(MAX_PAGES)
        || extra
            .and_then(|e| e.page_total)
            .is_some_and(|n| n.0 != total)
        || count as u64 != total.saturating_sub(offset).min(u64::from(size))
    {
        return Err(invalid());
    }
    Ok(offset)
}
fn parse_tracks(bytes: &[u8], artist_id: u64, page: u32, sort: u8) -> Result<(Vec<Track>, u64)> {
    check_status(bytes)?;
    let e: Envelope<TrackData> = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let total = e.data.total.0;
    if e.total.is_some_and(|n| n.0 != total) || e.data.input_param.sort != sort {
        return Err(invalid());
    }
    let offset = validate_page(total, e.extra, page, e.data.songs.len(), TRACK_PAGE_SIZE)?;
    let tracks = e
        .data
        .songs
        .into_iter()
        .enumerate()
        .map(|(i, s)| map_song(s, artist_id, offset + i as u64))
        .collect::<Result<_>>()?;
    Ok((tracks, total))
}
fn map_song(s: Song, artist_id: u64, position: u64) -> Result<Track> {
    let id = s.album_audio_id.id()?;
    let mut t = Track::new(resource(id.clone())?, required_text(s.audio_name)?);
    if s.authors.is_empty() || s.authors.len() > 100 {
        return Err(invalid());
    }
    let mut seen = BTreeSet::new();
    let mut selected = false;
    for author in s.authors {
        let a = author.base;
        let id = a.author_id.filter(|n| n.0 > 0).map(|n| n.0);
        if let Some(id) = id {
            if !seen.insert(id) {
                return Err(invalid());
            }
            selected |= id == artist_id;
        }
        t.artists.push(ArtistSummary {
            resource_ref: id.map(|id| resource(id.to_string())).transpose()?,
            name: required_text(a.author_name)?,
        });
    }
    if !selected {
        return Err(invalid());
    }
    if s.album_id.0 > 0 || s.album_info.is_some() {
        let (name, cover) = match s.album_info {
            Some(a) => (
                required_text(a.album_name)?,
                a.cover.as_deref().and_then(normalize_image_url),
            ),
            None => (String::new(), None),
        };
        t.album = Some(AlbumSummary {
            resource_ref: if s.album_id.0 > 0 {
                Some(resource(s.album_id.0.to_string())?)
            } else {
                None
            },
            name,
            cover_url: cover,
        });
    }
    t.extensions.insert("album_audio_id".into(), json!(id));
    for (key, value) in [
        ("audio_id", s.audio_id),
        ("audio_group_id", s.audio_group_id),
        ("primary_author_id", s.author_id),
    ] {
        if let Some(n) = value.filter(|n| n.0 > 0) {
            t.extensions.insert(key.into(), json!(n.0.to_string()));
        }
    }
    if let Some(date) = optional_text(s.publish_date, 128)? {
        t.extensions.insert("publish_date".into(), json!(date));
    }
    t.extensions
        .insert("artist_position".into(), json!(position));
    t.extensions
        .insert("detail_backend".into(), json!("openapi_author_audio_v2"));
    if let Some(a) = s.audio_info {
        map_audio(a, &mut t)?;
    }
    Ok(t)
}
fn parse_albums(bytes: &[u8], artist_id: u64, page: u32) -> Result<(Vec<Album>, u64)> {
    check_status(bytes)?;
    let e: Envelope<Vec<AlbumDto>> = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let total = e.total.ok_or_else(invalid)?.0;
    validate_page(total, e.extra, page, e.data.len(), ALBUM_PAGE_SIZE)?;
    let albums = e
        .data
        .into_iter()
        .map(|d| {
            let a = map_album(d, None)?;
            if !a.artists.iter().any(|a| {
                a.resource_ref
                    .as_ref()
                    .is_some_and(|r| r.id() == artist_id.to_string())
            }) {
                return Err(invalid());
            }
            Ok(a)
        })
        .collect::<Result<_>>()?;
    Ok((albums, total))
}
fn invalid() -> TuneWeaveError {
    kugou_upstream_error("KuGou artist returned inconsistent identity, pagination or metadata")
}

#[cfg(test)]
pub(crate) mod tests;
