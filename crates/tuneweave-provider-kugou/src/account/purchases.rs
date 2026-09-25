//! Purchase records and catalogue identities are distinct. No playback rights are inferred.
use super::library::Number;
use super::*;
use tuneweave_core::{
    Album, AlbumSummary, ArtistSummary, Extensions, PurchasedAlbum, PurchasedTrack, ResourceRef,
    Track,
};

pub(crate) const MAX_PAGES: u32 = 128;
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Kind {
    Tracks,
    Albums,
}
impl Kind {
    pub(crate) fn page_size(self) -> usize {
        match self {
            Self::Tracks => 50,
            Self::Albums => 15,
        }
    }
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Tracks => "tracks",
            Self::Albums => "albums",
        }
    }
    fn endpoint(self) -> Endpoint {
        match self {
            Self::Tracks => Endpoint::PurchasedTracks,
            Self::Albums => Endpoint::PurchasedAlbums,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Serialize)]
pub(crate) enum Item {
    Track(PurchasedTrack),
    Album(PurchasedAlbum),
}
impl Item {
    pub(crate) fn extensions(&self) -> &Extensions {
        match self {
            Self::Track(t) => &t.extensions,
            Self::Album(a) => &a.extensions,
        }
    }
    pub(crate) fn catalogue_id(&self) -> Option<&str> {
        match self {
            Self::Track(t) => t.track.as_ref().map(|t| t.id.as_str()),
            Self::Album(a) => a.album.as_ref().map(|a| a.id.as_str()),
        }
    }
    pub(crate) fn identity(&self) -> Result<String> {
        // These are upstream goods identifiers, never payment/order IDs. Do not dedup by song.
        let e = self.extensions();
        let tuple = (
            e.get("goods_id"),
            e.get("good_scid"),
            e.get("album_audio_id"),
            e.get("album_id"),
        );
        if tuple.0.is_none() && tuple.1.is_none() && tuple.2.is_none() && tuple.3.is_none() {
            return Err(malformed());
        }
        serde_json::to_string(&tuple).map_err(|_| malformed())
    }
}
pub(crate) struct PurchasePage {
    pub(crate) items: Vec<Item>,
    pub(crate) total: u64,
    pub(crate) response_bytes: usize,
}
#[derive(Deserialize)]
struct WirePage<T> {
    userid: Option<Number>,
    total: Number,
    goods: Vec<T>,
    page: Option<Number>,
    pagesize: Option<Number>,
}
#[derive(Deserialize)]
struct WireTrack {
    id: Option<Number>,
    good_scid: Option<Number>,
    album_audio_id: Option<Number>,
    album_id: Option<Number>,
    songname: Option<String>,
    author_name: Option<String>,
    album_cover: Option<String>,
    audio_info: Option<AudioInfo>,
    hash: Option<String>,
    deleted: Option<Number>,
    status: Option<Number>,
}
#[derive(Default, Deserialize)]
struct AudioInfo {
    album_id: Option<Number>,
    album_name: Option<String>,
    duration: Option<Number>,
    hash_high: Option<String>,
    hash_flac: Option<String>,
    hash_320: Option<String>,
    hash_128: Option<String>,
}
#[derive(Deserialize)]
struct WireAlbum {
    id: Option<Number>,
    album_id: Option<Number>,
    album_name: Option<String>,
    singer_name: Option<String>,
    cover: Option<String>,
    deleted: Option<Number>,
    status: Option<Number>,
    buy_total: Option<Number>,
    is_publish: Option<Number>,
    mp_count: Option<Number>,
}

impl KugouClient {
    pub(crate) async fn native_purchases_page(
        &self,
        session: &NativeSession,
        kind: Kind,
        page: u32,
    ) -> Result<PurchasePage> {
        validate_session(session)?;
        if !(1..=MAX_PAGES).contains(&page) {
            return Err(error(
                ErrorCode::InvalidRequest,
                "KuGou purchase page is outside its read budget",
            ));
        }
        #[derive(Serialize)]
        struct Body<'a> {
            appid: u16,
            userid: u64,
            token: &'a str,
            page: u32,
            pagesize: usize,
            clientver: String,
            deleted: u8,
            #[serde(skip_serializing_if = "Option::is_none")]
            need_audio_info: Option<u8>,
            #[serde(skip_serializing_if = "Option::is_none")]
            area_code: Option<&'static str>,
            #[serde(skip_serializing_if = "Option::is_none")]
            use_custom_sort: Option<u8>,
        }
        let body = crypto::encode(&Body {
            appid: session.client.appid(),
            userid: session.user_id.parse().map_err(|_| malformed())?,
            token: &session.token,
            page,
            pagesize: kind.page_size(),
            clientver: session.client.clientver().to_string(),
            deleted: 0,
            need_audio_info: (kind == Kind::Tracks).then_some(1),
            area_code: (kind == Kind::Tracks).then_some("1"),
            // Current Standard AlbumStoreManagerProtocol requests its saved order.
            // Concept has no equivalent evidence; preserve its existing request.
            use_custom_sort: (kind == Kind::Albums && session.client == KugouLoginClient::Standard)
                .then_some(1),
        })?;
        self.native_post(kind.endpoint(), session, now_ms()? / 1000, body, |bytes| {
            parse(bytes, &session.user_id, kind, page)
        })
        .await
    }
}
fn parse(bytes: &[u8], uid: &str, kind: Kind, page: u32) -> Result<PurchasePage> {
    match kind {
        Kind::Tracks => {
            let wire: WirePage<WireTrack> = data(bytes)?;
            validate_page(&wire, uid, kind, page)?;
            Ok(PurchasePage {
                total: wire.total.0,
                response_bytes: bytes.len(),
                items: wire
                    .goods
                    .into_iter()
                    .map(map_track)
                    .map(|r| r.map(Item::Track))
                    .collect::<Result<_>>()?,
            })
        }
        Kind::Albums => {
            let wire: WirePage<WireAlbum> = data(bytes)?;
            validate_page(&wire, uid, kind, page)?;
            Ok(PurchasePage {
                total: wire.total.0,
                response_bytes: bytes.len(),
                items: wire
                    .goods
                    .into_iter()
                    .map(map_album)
                    .map(|r| r.map(Item::Album))
                    .collect::<Result<_>>()?,
            })
        }
    }
}
fn validate_page<T>(wire: &WirePage<T>, uid: &str, kind: Kind, page: u32) -> Result<()> {
    if wire.userid.is_some_and(|n| n.0.to_string() != uid) {
        return Err(identity_conflict());
    }
    if wire.page.is_some_and(|n| n.0 != u64::from(page))
        || wire
            .pagesize
            .is_some_and(|n| n.0 != kind.page_size() as u64)
        || wire.total.0 > u64::from(MAX_PAGES) * kind.page_size() as u64
        || wire.goods.len() > kind.page_size()
    {
        return Err(malformed());
    }
    let consumed = u64::from(page - 1) * kind.page_size() as u64;
    let expected = wire
        .total
        .0
        .saturating_sub(consumed)
        .min(kind.page_size() as u64);
    if wire.goods.len() as u64 != expected {
        return Err(error(
            ErrorCode::UpstreamError,
            "KuGou purchase list returned incomplete or excessive rows",
        ));
    }
    Ok(())
}
fn text(value: Option<String>) -> Result<Option<String>> {
    value
        .map(|s| {
            if s.len() > 8192 || s.chars().any(char::is_control) {
                return Err(malformed());
            }
            let t = s.trim();
            Ok((!t.is_empty()).then(|| t.to_owned()))
        })
        .transpose()
        .map(Option::flatten)
}
fn id(value: Option<Number>) -> Option<String> {
    value.filter(|n| n.0 > 0).map(|n| n.0.to_string())
}
fn reference(id: &str) -> Result<ResourceRef> {
    ResourceRef::new(Platform::Kugou, id).map_err(|_| malformed())
}
fn artists(name: Option<String>) -> Result<Vec<ArtistSummary>> {
    Ok(text(name)?
        .map(|name| ArtistSummary {
            resource_ref: None,
            name,
        })
        .into_iter()
        .collect())
}
fn cover(value: Option<String>) -> Result<Option<String>> {
    text(value)?
        .map(|v| normalize_image_url(&v).ok_or_else(malformed))
        .transpose()
}
fn state(deleted: Option<Number>, status: Option<Number>, e: &mut Extensions) -> Result<()> {
    // deleted=0 is requested. Never silently filter a contradicting row or alter total.
    if deleted.is_some_and(|n| n.0 != 0) {
        return Err(error(
            ErrorCode::UpstreamError,
            "KuGou active purchase selection returned a deleted record",
        ));
    }
    if let Some(n) = deleted {
        e.insert("deleted".into(), json!(n.0));
    }
    if let Some(n) = status {
        e.insert("upstream_status".into(), json!(n.0));
    }
    Ok(())
}
fn identity_fields(values: &[(&str, Option<String>)]) -> Extensions {
    values
        .iter()
        .filter_map(|(k, v)| v.as_ref().map(|v| ((*k).into(), json!(v))))
        .collect()
}
fn map_track(row: WireTrack) -> Result<PurchasedTrack> {
    let goods_id = id(row.id);
    let good_scid = id(row.good_scid);
    let catalogue = id(row.album_audio_id);
    if goods_id.is_none() && good_scid.is_none() && catalogue.is_none() {
        return Err(malformed());
    }
    let audio = row.audio_info.unwrap_or_default();
    let outer_album = id(row.album_id);
    let inner_album = id(audio.album_id);
    if outer_album.is_some() && inner_album.is_some() && outer_album != inner_album {
        return Err(identity_conflict());
    }
    let album_id = inner_album.or(outer_album);
    let album_name = text(audio.album_name)?;
    let name = text(row.songname)?;
    let artists = artists(row.author_name)?;
    let cover_url = cover(row.album_cover)?;
    let mut extensions = identity_fields(&[
        ("goods_id", goods_id),
        ("good_scid", good_scid),
        ("album_audio_id", catalogue.clone()),
        ("album_id", album_id.clone()),
    ]);
    state(row.deleted, row.status, &mut extensions)?;
    let duration = audio.duration.map(|n| n.0);
    if let Some(n) = duration {
        extensions.insert("duration_ms".into(), json!(n));
    }
    if let Some(n) = &album_name {
        extensions.insert("album_name".into(), json!(n));
    }
    let mut hashes = serde_json::Map::new();
    for (field, value) in [
        ("hash_high", audio.hash_high),
        ("hash_flac", audio.hash_flac),
        ("hash_320", audio.hash_320),
        ("hash_128", audio.hash_128),
        ("hash", row.hash),
    ] {
        if let Some(value) = text(value)? {
            if value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(malformed());
            }
            if value.bytes().any(|b| b != b'0') {
                hashes.insert(field.into(), json!(value.to_ascii_lowercase()));
            }
        }
    }
    if !hashes.is_empty() {
        extensions.insert("catalogue_hashes".into(), Value::Object(hashes));
    }
    let track = match (catalogue, name.as_ref()) {
        (Some(catalogue), Some(name)) => {
            let mut track = Track::new(reference(&catalogue)?, name);
            track.artists = artists.clone();
            track.duration_ms = duration;
            if album_id.is_some() || album_name.is_some() || cover_url.is_some() {
                track.album = Some(AlbumSummary {
                    resource_ref: album_id.as_deref().map(reference).transpose()?,
                    name: album_name.unwrap_or_default(),
                    cover_url: cover_url.clone(),
                });
            }
            track.extensions = extensions.clone();
            Some(track)
        }
        _ => None,
    };
    extensions.insert("catalogue_resolved".into(), json!(track.is_some()));
    Ok(PurchasedTrack {
        track,
        name,
        artists,
        cover_url,
        extensions,
    })
}
fn map_album(row: WireAlbum) -> Result<PurchasedAlbum> {
    let goods_id = id(row.id);
    let catalogue = id(row.album_id);
    if goods_id.is_none() && catalogue.is_none() {
        return Err(malformed());
    }
    let name = text(row.album_name)?;
    let artists = artists(row.singer_name)?;
    let cover_url = cover(row.cover)?;
    let mut extensions =
        identity_fields(&[("goods_id", goods_id), ("album_id", catalogue.clone())]);
    state(row.deleted, row.status, &mut extensions)?;
    for (key, value) in [
        ("buy_total", row.buy_total),
        ("is_publish", row.is_publish),
        ("mp_count", row.mp_count),
    ] {
        if let Some(value) = value {
            extensions.insert(key.into(), json!(value.0));
        }
    }
    let album = match (catalogue, name.as_ref()) {
        (Some(catalogue), Some(name)) => Some(Album {
            resource_ref: reference(&catalogue)?,
            platform: Platform::Kugou,
            id: catalogue,
            name: name.clone(),
            artists: artists.clone(),
            cover_url: cover_url.clone(),
            aliases: vec![],
            description: String::new(),
            published_at: None,
            track_count: None,
            company: None,
            kind: None,
            extensions: extensions.clone(),
        }),
        _ => None,
    };
    extensions.insert("catalogue_resolved".into(), json!(album.is_some()));
    Ok(PurchasedAlbum {
        album,
        digital_album: None,
        name,
        artists,
        cover_url,
        extensions,
    })
}
#[cfg(test)]
mod tests;
