use super::assets::{Audio, map_audio};
use super::dto::{Number, optional_text, required_text, resource};
use super::openapi::{Endpoint, check_status};
use super::*;
use tuneweave_core::Album;

pub(crate) const PAGE_SIZE: u32 = 20;
pub(crate) const MAX_ITEMS: u64 = 1280;
const FIELDS: &str = "album_id,album_name,publish_date,sizable_cover,intro,language,is_publish,heat,type,quality,authors,exclusive,author_name,publish_company,category";

#[derive(Serialize)]
struct AlbumRequest {
    data: [AlbumId; 1],
    is_buy: u8,
    fields: &'static str,
}
#[derive(Serialize)]
struct AlbumId {
    album_id: u64,
}
#[derive(Serialize)]
struct TracksRequest {
    album_id: u64,
    is_buy: &'static str,
    page: u32,
    pagesize: u32,
}

#[derive(Deserialize)]
struct Envelope<T> {
    total: Option<u64>,
    extra: Option<Extra>,
    data: Option<T>,
}
#[derive(Deserialize)]
struct Extra {
    disc_cnt: Option<Number>,
}

impl KugouClient {
    pub(crate) async fn album_metadata(&self, id: u64) -> Result<Album> {
        let device = self.device_identity()?;
        self.album_metadata_with_device(id, &device, &mut |_| Ok(()))
            .await
    }
    async fn album_metadata_with_device(
        &self,
        id: u64,
        device: &KugouDeviceIdentity,
        observe: &mut (impl FnMut(usize) -> Result<()> + Send),
    ) -> Result<Album> {
        observe(0)?;
        let bytes = self
            .public_openapi(
                Endpoint::Album,
                &AlbumRequest {
                    data: [AlbumId { album_id: id }],
                    is_buy: 0,
                    fields: FIELDS,
                },
                device,
            )
            .await;
        // Check the account again even when the anonymous request failed. The
        // observer also charges successful raw responses before DTO allocation.
        observe(bytes.as_ref().map_or(0, Vec::len))?;
        parse_album(&bytes?, id)
    }
    pub(crate) async fn complete_album_tracks(&self, id: u64) -> Result<AlbumTracks> {
        self.complete_album_tracks_observed(id, |_| Ok(())).await
    }
    pub(crate) async fn complete_album_tracks_observed(
        &self,
        id: u64,
        mut observe: impl FnMut(usize) -> Result<()> + Send,
    ) -> Result<AlbumTracks> {
        let device = self.device_identity()?;
        let album = self
            .album_metadata_with_device(id, &device, &mut observe)
            .await?;
        let mut tracks = Vec::new();
        let mut total = None;
        let mut discs = None;
        let mut signatures = BTreeSet::new();
        let mut positions = BTreeSet::new();
        for page in 1..=64 {
            observe(0)?;
            let bytes = self
                .public_openapi(
                    Endpoint::AlbumTracks,
                    &TracksRequest {
                        album_id: id,
                        is_buy: "",
                        page,
                        pagesize: PAGE_SIZE,
                    },
                    &device,
                )
                .await;
            observe(bytes.as_ref().map_or(0, Vec::len))?;
            let data = parse_tracks(&bytes?, &album, page)?;
            if total.is_some_and(|v| v != data.total)
                || discs.is_some_and(|v| Some(v) != data.discs)
            {
                return Err(invalid());
            }
            total = Some(data.total);
            discs = data.discs;
            let signature: Vec<_> = data
                .items
                .iter()
                .map(|track| {
                    (
                        track.id.clone(),
                        track.extensions.get("disc_number").and_then(Value::as_u64),
                        track.extensions.get("track_number").and_then(Value::as_u64),
                    )
                })
                .collect();
            if !signature.is_empty() && !signatures.insert(signature) {
                return Err(invalid());
            }
            for track in data.items {
                let disc = track.extensions.get("disc_number").and_then(Value::as_u64);
                let sort = track.extensions.get("track_number").and_then(Value::as_u64);
                if disc
                    .zip(sort)
                    .is_some_and(|position| !positions.insert(position))
                {
                    return Err(invalid());
                }
                tracks.push(track);
            }
            if tracks.len() as u64 == data.total {
                return Ok(AlbumTracks {
                    tracks,
                    total: data.total,
                    discs,
                    pages: page,
                });
            }
        }
        Err(invalid())
    }
}

pub(crate) struct AlbumTracks {
    pub tracks: Vec<Track>,
    pub total: u64,
    pub discs: Option<u64>,
    pub pages: u32,
}
struct PhysicalPage {
    items: Vec<Track>,
    total: u64,
    discs: Option<u64>,
}

#[derive(Deserialize)]
struct Author {
    author_id: Option<Number>,
    author_name: String,
}
fn authors(values: Option<Vec<Author>>, fallback: Option<String>) -> Result<Vec<ArtistSummary>> {
    let mut result = vec![];
    let mut seen = BTreeSet::new();
    if let Some(values) = values.filter(|v| !v.is_empty()) {
        if values.len() > 100 {
            return Err(invalid());
        }
        for a in values {
            let id = a.author_id.filter(|n| n.0 > 0).map(|n| n.0.to_string());
            if id.as_ref().is_some_and(|id| !seen.insert(id.clone())) {
                return Err(invalid());
            }
            result.push(ArtistSummary {
                resource_ref: id.map(resource).transpose()?,
                name: required_text(a.author_name)?,
            });
        }
    } else if let Some(name) = optional_text(fallback, 1024)? {
        result.push(ArtistSummary {
            resource_ref: None,
            name,
        });
    }
    Ok(result)
}
#[derive(Deserialize)]
pub(super) struct AlbumDto {
    album_id: Number,
    album_name: String,
    authors: Option<Vec<Author>>,
    author_name: Option<String>,
    intro: Option<String>,
    sizable_cover: Option<String>,
    publish_date: Option<String>,
    publish_company: Option<String>,
    language: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    category: Option<Number>,
    is_publish: Option<Number>,
    heat: Option<Number>,
}
fn parse_album(bytes: &[u8], id: u64) -> Result<Album> {
    check_status(bytes)?;
    let e: Envelope<Vec<AlbumDto>> = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let [v]: [AlbumDto; 1] = e
        .data
        .ok_or_else(invalid)?
        .try_into()
        .map_err(|_| invalid())?;
    map_album(v, Some(id))
}

pub(super) fn map_album(v: AlbumDto, expected_id: Option<u64>) -> Result<Album> {
    let id = v.album_id.0;
    if id == 0 || expected_id.is_some_and(|expected| id != expected) {
        return Err(invalid());
    }
    let mut extensions = Extensions::new();
    for (key, value) in [
        ("category", v.category),
        ("is_publish", v.is_publish),
        ("heat", v.heat),
    ] {
        if let Some(n) = value {
            extensions.insert(key.into(), json!(n.0));
        }
    }
    if let Some(s) = optional_text(v.language, 256)? {
        extensions.insert("language".into(), json!(s));
    }
    Ok(Album {
        resource_ref: resource(id.to_string())?,
        platform: Platform::Kugou,
        id: id.to_string(),
        name: required_text(v.album_name)?,
        aliases: vec![],
        artists: authors(v.authors, v.author_name)?,
        description: optional_text(v.intro, 131072)?.unwrap_or_default(),
        cover_url: v.sizable_cover.as_deref().and_then(normalize_image_url),
        published_at: optional_text(v.publish_date, 128)?,
        track_count: None,
        company: optional_text(v.publish_company, 1024)?,
        kind: optional_text(v.kind, 256)?,
        extensions,
    })
}

#[derive(Deserialize)]
struct TrackData {
    total: u64,
    songs: Vec<Song>,
}
#[derive(Deserialize)]
struct Song {
    base: Base,
    album_info: Option<SongAlbum>,
    authors: Option<Vec<Author>>,
    audio_info: Option<Audio>,
    extend: Option<Position>,
}
#[derive(Deserialize)]
struct Base {
    album_id: Number,
    album_audio_id: Number,
    audio_id: Option<Number>,
    audio_name: String,
    author_name: Option<String>,
    is_publish: Option<Number>,
}
#[derive(Deserialize)]
struct SongAlbum {
    album_id: Option<Number>,
    album_name: Option<String>,
    cover: Option<String>,
}
#[derive(Deserialize)]
struct Position {
    disc: Option<Number>,
    sort: Option<Number>,
    cd_name: Option<String>,
}
fn parse_tracks(bytes: &[u8], album: &Album, page: u32) -> Result<PhysicalPage> {
    check_status(bytes)?;
    let e: Envelope<TrackData> = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    let data = e.data.ok_or_else(invalid)?;
    if e.total != Some(data.total) || data.total > MAX_ITEMS {
        return Err(invalid());
    }
    let offset = u64::from(page.checked_sub(1).ok_or_else(invalid)?) * u64::from(PAGE_SIZE);
    if data.songs.len() as u64 != data.total.saturating_sub(offset).min(u64::from(PAGE_SIZE)) {
        return Err(invalid());
    }
    let discs = e.extra.and_then(|v| v.disc_cnt).map(|n| n.0);
    if discs == Some(0) && data.total > 0 {
        return Err(invalid());
    }
    let items = data
        .songs
        .into_iter()
        .enumerate()
        .map(|(i, s)| map_song(s, album, offset + i as u64, discs))
        .collect::<Result<Vec<_>>>()?;
    Ok(PhysicalPage {
        items,
        total: data.total,
        discs,
    })
}
fn map_song(song: Song, album: &Album, position: u64, discs: Option<u64>) -> Result<Track> {
    if song.base.album_id.0.to_string() != album.id {
        return Err(invalid());
    }
    let id = song.base.album_audio_id.id()?;
    let mut track = Track::new(resource(id.clone())?, required_text(song.base.audio_name)?);
    track.artists = authors(song.authors, song.base.author_name)?;
    let mut cover = album.cover_url.clone();
    if let Some(a) = song.album_info {
        if a.album_id.is_some_and(|v| v.0.to_string() != album.id) {
            return Err(invalid());
        }
        if optional_text(a.album_name, 1024)?.is_some_and(|name| name != album.name) {
            return Err(invalid());
        }
        cover = a.cover.as_deref().and_then(normalize_image_url).or(cover);
    }
    track.album = Some(AlbumSummary {
        resource_ref: Some(album.resource_ref.clone()),
        name: album.name.clone(),
        cover_url: cover,
    });
    track.extensions.insert("album_audio_id".into(), json!(id));
    if let Some(n) = song.base.audio_id.filter(|v| v.0 > 0) {
        track
            .extensions
            .insert("audio_id".into(), json!(n.0.to_string()));
    }
    if let Some(n) = song.base.is_publish {
        track.extensions.insert("is_publish".into(), json!(n.0));
    }
    track
        .extensions
        .insert("album_position".into(), json!(position));
    track
        .extensions
        .insert("detail_backend".into(), json!("openapi_album_audio_lite"));
    if let Some(p) = song.extend {
        if p.disc
            .as_ref()
            .is_some_and(|n| discs.is_some_and(|count| n.0 > count))
        {
            return Err(invalid());
        }
        for (key, value) in [("disc_number", p.disc), ("track_number", p.sort)] {
            if let Some(n) = value.filter(|n| n.0 > 0) {
                track.extensions.insert(key.into(), json!(n.0));
            }
        }
        if let Some(name) = optional_text(p.cd_name, 1024)? {
            track.extensions.insert("disc_name".into(), json!(name));
        }
    }
    if let Some(audio) = song.audio_info {
        map_audio(audio, &mut track)?;
    }
    Ok(track)
}

fn invalid() -> TuneWeaveError {
    kugou_upstream_error("KuGou album returned inconsistent identity, pagination or metadata")
}

#[cfg(test)]
mod tests;
