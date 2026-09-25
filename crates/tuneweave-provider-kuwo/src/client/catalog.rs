//! Typed current-web catalogue searches. The anonymous session is independent of login.
use super::*;
use tuneweave_core::{Album, Artist, SearchItem};

const RESPONSE_LIMIT: u64 = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug)]
pub(crate) enum CatalogKind {
    Album,
    Artist,
    Playlist,
    Mv,
}
impl CatalogKind {
    pub(crate) const fn page_size(self) -> u32 {
        match self {
            Self::Album | Self::Mv => 20,
            Self::Artist | Self::Playlist => 30,
        }
    }
    pub(super) const fn url(self) -> &'static str {
        match self {
            Self::Album => "https://www.kuwo.cn/api/www/search/searchAlbumBykeyWord",
            Self::Artist => "https://www.kuwo.cn/api/www/search/searchArtistBykeyWord",
            Self::Playlist => "https://www.kuwo.cn/api/www/search/searchPlayListBykeyWord",
            Self::Mv => "https://www.kuwo.cn/api/www/search/searchMvBykeyWord",
        }
    }
    pub(super) const fn path(self) -> &'static str {
        match self {
            Self::Album => "/api/www/search/searchAlbumBykeyWord",
            Self::Artist => "/api/www/search/searchArtistBykeyWord",
            Self::Playlist => "/api/www/search/searchPlayListBykeyWord",
            Self::Mv => "/api/www/search/searchMvBykeyWord",
        }
    }
    pub(crate) const fn operation(self) -> &'static str {
        match self {
            Self::Album => "catalog_search_albums",
            Self::Artist => "catalog_search_artists",
            Self::Playlist => "catalog_search_playlists",
            Self::Mv => "catalog_search_mvs",
        }
    }
}

pub(crate) struct CatalogPage {
    pub items: Vec<SearchItem>,
    pub total: u64,
}

#[derive(Serialize)]
struct Query<'a> {
    key: &'a str,
    pn: u32,
    rn: u32,
    #[serde(rename = "httpsStatus")]
    https_status: u8,
    #[serde(rename = "reqId")]
    request_id: String,
    plat: &'static str,
    from: &'static str,
}

impl KuwoClient {
    pub(crate) async fn search_catalog_page(
        &self,
        kind: CatalogKind,
        keyword: &str,
        page: u32,
    ) -> Result<CatalogPage> {
        for refresh in [false, true] {
            let response = self
                .signed_get(
                    KuwoSignedEndpoint::Catalog(kind),
                    &Query {
                        key: keyword,
                        pn: page,
                        rn: kind.page_size(),
                        https_status: 1,
                        request_id: new_request_id(),
                        plat: "web_www",
                        from: "",
                    },
                    SEARCH_REFERER,
                    refresh,
                    u8::from(refresh),
                )
                .await?;
            match response {
                KuwoSignedResponse::SessionRejected if !refresh => continue,
                KuwoSignedResponse::SessionRejected => return Err(invalid()),
                KuwoSignedResponse::Body(bytes) => {
                    if !refresh && is_signed_session_rejection(&bytes) {
                        continue;
                    }
                    return parse_page(kind, page, &bytes);
                }
            }
        }
        Err(invalid())
    }
}

pub(super) async fn read_response(response: reqwest::Response) -> Result<Vec<u8>> {
    if !response.status().is_success() {
        return Err(kuwo_http_error(response.status()));
    }
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next());
    if !mime.is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json")) {
        return Err(invalid());
    }
    read_bounded_response_with_limit(response, "Kuwo catalogue", RESPONSE_LIMIT).await
}

#[derive(Deserialize)]
struct Envelope {
    code: i64,
    data: Option<serde_json::Value>,
}
#[derive(Deserialize)]
struct Mvs {
    total: Unsigned,
    mvlist: Vec<artists::Mv>,
}
#[derive(Deserialize)]
struct Albums {
    total: Unsigned,
    #[serde(rename = "albumList")]
    items: Vec<AlbumDto>,
}
#[derive(Deserialize)]
struct Artists {
    total: Unsigned,
    pn: Unsigned,
    rn: Unsigned,
    #[serde(rename = "artistList")]
    items: Vec<ArtistDto>,
}
#[derive(Deserialize)]
struct Playlists {
    total: Unsigned,
    list: Vec<PlaylistDto>,
}
#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Unsigned {
    Number(u64),
    Text(String),
}
impl Unsigned {
    pub(super) fn value(&self) -> Result<u64> {
        match self {
            Self::Number(value) => Ok(*value),
            Self::Text(value) => value
                .parse::<u64>()
                .ok()
                .filter(|number| number.to_string() == *value)
                .ok_or_else(invalid),
        }
    }
    pub(super) fn id(&self) -> Result<String> {
        let value = self.value()?;
        if value == 0 {
            return Err(invalid());
        }
        Ok(value.to_string())
    }
}

#[derive(Deserialize)]
pub(super) struct AlbumDto {
    albumid: Unsigned,
    album: String,
    #[serde(default)]
    pub(super) artist: String,
    pub(super) artistid: Option<Unsigned>,
    #[serde(default)]
    albuminfo: String,
    pic: Option<String>,
    #[serde(rename = "releaseDate")]
    release_date: Option<String>,
    lang: Option<String>,
    content_type: Option<Unsigned>,
}
#[derive(Deserialize)]
pub(super) struct ArtistDto {
    id: Unsigned,
    name: String,
    pic: Option<String>,
    #[serde(rename = "musicNum")]
    music_num: Option<Unsigned>,
    #[serde(rename = "artistFans")]
    fans: Option<Unsigned>,
    country: Option<String>,
    content_type: Option<Unsigned>,
}
#[derive(Deserialize)]
struct PlaylistDto {
    id: Unsigned,
    name: String,
    img: Option<String>,
    total: Option<Unsigned>,
    uname: Option<String>,
    listencnt: Option<Unsigned>,
}

pub(super) fn parse_page(kind: CatalogKind, page: u32, bytes: &[u8]) -> Result<CatalogPage> {
    let root: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if root.code != 200 {
        return Err(
            kuwo_upstream_error("Kuwo catalogue returned an unsuccessful business code")
                .with_details(json!({"upstream_code":root.code})),
        );
    }
    let data = root.data.ok_or_else(invalid)?;
    let (total, items) = match kind {
        CatalogKind::Mv => {
            let data: Mvs = serde_json::from_value(data).map_err(|_| invalid())?;
            (
                data.total.value()?,
                data.mvlist
                    .into_iter()
                    .map(|item| artists::map_mv(item, None).map(SearchItem::Video))
                    .collect::<Result<Vec<_>>>()?,
            )
        }
        CatalogKind::Album => {
            let data: Albums = serde_json::from_value(data).map_err(|_| invalid())?;
            (
                data.total.value()?,
                data.items
                    .into_iter()
                    .map(map_album)
                    .collect::<Result<Vec<_>>>()?,
            )
        }
        CatalogKind::Artist => {
            let data: Artists = serde_json::from_value(data).map_err(|_| invalid())?;
            if data.pn.value()?.checked_add(1) != Some(u64::from(page))
                || data.rn.value()? != u64::from(kind.page_size())
            {
                return Err(invalid());
            }
            (
                data.total.value()?,
                data.items
                    .into_iter()
                    .map(map_artist)
                    .collect::<Result<Vec<_>>>()?,
            )
        }
        CatalogKind::Playlist => {
            let data: Playlists = serde_json::from_value(data).map_err(|_| invalid())?;
            (
                data.total.value()?,
                data.list
                    .into_iter()
                    .map(map_playlist)
                    .collect::<Result<Vec<_>>>()?,
            )
        }
    };
    let start = u64::from(page.checked_sub(1).ok_or_else(invalid)?) * u64::from(kind.page_size());
    let expected = total.saturating_sub(start).min(u64::from(kind.page_size()));
    if items.len() as u64 != expected {
        return Err(kuwo_upstream_error(
            "Kuwo catalogue page length disagrees with its total",
        ));
    }
    Ok(CatalogPage { items, total })
}

pub(super) fn map_album(item: AlbumDto) -> Result<SearchItem> {
    music_type(item.content_type.as_ref())?;
    let id = item.albumid.id()?;
    let artist_name = text(&item.artist, 512, false)?;
    let artist_ref = item
        .artistid
        .as_ref()
        .map(Unsigned::id)
        .transpose()?
        .map(|id| ResourceRef::new(Platform::Kuwo, id).map_err(|_| invalid()))
        .transpose()?;
    if artist_ref.is_some() && artist_name.is_empty() {
        return Err(invalid());
    }
    let mut extensions = Extensions::new();
    optional_text(&mut extensions, "language", item.lang.as_deref())?;
    Ok(SearchItem::Album(Album {
        resource_ref: reference(&id)?,
        platform: Platform::Kuwo,
        id,
        name: name(&item.album)?,
        aliases: vec![],
        artists: if artist_name.is_empty() {
            vec![]
        } else {
            vec![ArtistSummary {
                resource_ref: artist_ref,
                name: artist_name,
            }]
        },
        description: text(&item.albuminfo, 64 * 1024, true)?,
        cover_url: item.pic.as_deref().and_then(normalize_official_image_url),
        published_at: item
            .release_date
            .as_deref()
            .map(date)
            .transpose()?
            .flatten(),
        track_count: None,
        company: None,
        kind: None,
        extensions,
    }))
}

pub(super) fn map_artist(item: ArtistDto) -> Result<SearchItem> {
    music_type(item.content_type.as_ref())?;
    let id = item.id.id()?;
    let mut extensions = Extensions::new();
    optional_text(&mut extensions, "country", item.country.as_deref())?;
    optional_number(&mut extensions, "artist_fans", item.fans.as_ref())?;
    Ok(SearchItem::Artist(Artist {
        resource_ref: reference(&id)?,
        platform: Platform::Kuwo,
        id,
        name: name(&item.name)?,
        aliases: vec![],
        description: String::new(),
        biography_sections: vec![],
        avatar_url: item.pic.as_deref().and_then(artist_image),
        cover_url: None,
        track_count: item.music_num.as_ref().map(Unsigned::value).transpose()?,
        album_count: None,
        mv_count: None,
        video_count: None,
        identities: vec![],
        extensions,
    }))
}

fn map_playlist(item: PlaylistDto) -> Result<SearchItem> {
    let id = item.id.id()?;
    let mut extensions = Extensions::new();
    optional_text(&mut extensions, "creator_name", item.uname.as_deref())?;
    optional_number(&mut extensions, "listen_count", item.listencnt.as_ref())?;
    Ok(SearchItem::Playlist(Playlist {
        resource_ref: reference(&id)?,
        platform: Platform::Kuwo,
        id,
        name: name(&item.name)?,
        description: String::new(),
        cover_url: item.img.as_deref().and_then(normalize_playlist_image_url),
        creator: None,
        track_count: item.total.as_ref().map(Unsigned::value).transpose()?,
        tags: vec![],
        subscribed: None,
        created_at: None,
        updated_at: None,
        extensions,
    }))
}

fn reference(id: &str) -> Result<ResourceRef> {
    ResourceRef::new(Platform::Kuwo, id.to_owned()).map_err(|_| invalid())
}
fn music_type(value: Option<&Unsigned>) -> Result<()> {
    if value
        .map(Unsigned::value)
        .transpose()?
        .is_some_and(|value| value != 0)
    {
        return Err(kuwo_upstream_error(
            "Kuwo catalogue item is not a supported music resource",
        ));
    }
    Ok(())
}
fn name(value: &str) -> Result<String> {
    let value = text(value, 512, false)?;
    if value.is_empty() {
        return Err(invalid());
    }
    Ok(value)
}
pub(super) fn text(value: &str, limit: usize, multiline: bool) -> Result<String> {
    if value.len() > limit * 6 {
        return Err(invalid());
    }
    let value = value
        .replace("&nbsp;", " ")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&");
    let value = value.trim();
    if value.len() > limit
        || value
            .chars()
            .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\r' | '\t')))
    {
        return Err(invalid());
    }
    Ok(value.to_owned())
}
pub(super) fn optional_text(
    extensions: &mut Extensions,
    key: &str,
    value: Option<&str>,
) -> Result<()> {
    if let Some(value) = value {
        let value = text(value, 512, false)?;
        if !value.is_empty() {
            extensions.insert(key.to_owned(), json!(value));
        }
    }
    Ok(())
}
fn optional_number(extensions: &mut Extensions, key: &str, value: Option<&Unsigned>) -> Result<()> {
    if let Some(value) = value {
        extensions.insert(key.to_owned(), json!(value.value()?));
    }
    Ok(())
}
pub(super) fn date(value: &str) -> Result<Option<String>> {
    if value.is_empty() {
        return Ok(None);
    }
    let bytes = value.as_bytes();
    if bytes.len() != 10
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !bytes
            .iter()
            .enumerate()
            .all(|(i, b)| i == 4 || i == 7 || b.is_ascii_digit())
    {
        return Err(invalid());
    }
    let year = value[..4].parse::<u32>().map_err(|_| invalid())?;
    let month = value[5..7].parse::<u32>().map_err(|_| invalid())?;
    let day = value[8..].parse::<u32>().map_err(|_| invalid())?;
    let days = match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        _ => return Err(invalid()),
    };
    if year == 0 || !(1..=days).contains(&day) {
        return Err(invalid());
    }
    Ok(Some(value.to_owned()))
}
pub(super) fn artist_image(value: &str) -> Option<String> {
    if let Some(value) = normalize_official_image_url(value) {
        return Some(value);
    }
    let url = Url::parse(value).ok()?;
    (url.scheme() == "https"
        && url.host_str() == Some("star.kuwo.cn")
        && url.username().is_empty()
        && url.password().is_none()
        && url.port().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && url.path().starts_with("/star/starheads/"))
    .then(|| url.into())
}
fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo catalogue returned an invalid typed response")
}

#[cfg(test)]
pub(crate) mod tests;
