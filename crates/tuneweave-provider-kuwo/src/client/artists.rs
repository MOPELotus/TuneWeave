use super::*;
use catalog::{AlbumDto, ArtistDto, Unsigned};
use tuneweave_core::{Album, Artist, CreatorSummary, SearchItem, Video};

mod bcsy;
mod list;

pub(crate) const PAGE_SIZE: u32 = 20;
enum UnsignedArtistEndpoint {
    Albums,
    Mvs,
}
impl UnsignedArtistEndpoint {
    const fn target(&self) -> (&'static str, &'static str, &'static str) {
        match self {
            Self::Albums => (
                "https://wapi.kuwo.cn/api/www/artist/artistAlbum",
                "/api/www/artist/artistAlbum",
                "artist_albums",
            ),
            Self::Mvs => (
                "https://wapi.kuwo.cn/api/www/artist/artistMv",
                "/api/www/artist/artistMv",
                "artist_mvs",
            ),
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum SignedArtistEndpoint {
    Info,
    Tracks,
}
impl SignedArtistEndpoint {
    pub(super) const fn url(self) -> &'static str {
        match self {
            Self::Info => "https://www.kuwo.cn/api/www/artist/artist",
            Self::Tracks => "https://www.kuwo.cn/api/www/artist/artistMusic",
        }
    }
    pub(super) const fn path(self) -> &'static str {
        match self {
            Self::Info => "/api/www/artist/artist",
            Self::Tracks => "/api/www/artist/artistMusic",
        }
    }
    pub(super) const fn operation(self) -> &'static str {
        match self {
            Self::Info => "artist_detail",
            Self::Tracks => "artist_tracks",
        }
    }
}

pub(crate) struct ArtistPage<T> {
    pub items: Vec<T>,
    pub total: u64,
}
#[derive(Serialize)]
struct Query<'a> {
    artistid: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pn: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rn: Option<u32>,
    #[serde(rename = "httpsStatus")]
    https_status: u8,
    #[serde(rename = "reqId")]
    request_id: String,
    plat: &'static str,
    from: &'static str,
}
impl<'a> Query<'a> {
    fn new(id: &'a str, page: Option<u32>) -> Self {
        Self {
            artistid: id,
            pn: page,
            rn: page.map(|_| PAGE_SIZE),
            https_status: 1,
            request_id: new_request_id(),
            plat: "web_www",
            from: "",
        }
    }
}

impl KuwoClient {
    pub(crate) async fn bcsy_artist_catalog(&self) -> Result<Vec<Artist>> {
        bcsy::read(self).await
    }

    pub(crate) async fn artist_info(&self, id: &str) -> Result<Artist> {
        let bytes = self
            .artist_bytes(SignedArtistEndpoint::Info, id, None)
            .await?;
        parse_artist(&bytes, id)
    }
    pub(crate) async fn artist_tracks_page(
        &self,
        artist: &Artist,
        page: u32,
    ) -> Result<ArtistPage<Track>> {
        let bytes = self
            .artist_bytes(SignedArtistEndpoint::Tracks, &artist.id, Some(page))
            .await?;
        parse_tracks(&bytes, artist, page)
    }
    async fn artist_bytes(
        &self,
        kind: SignedArtistEndpoint,
        id: &str,
        page: Option<u32>,
    ) -> Result<Vec<u8>> {
        let referer = format!("https://www.kuwo.cn/singer_detail/{id}");
        for refresh in [false, true] {
            let response = self
                .signed_get(
                    KuwoSignedEndpoint::Artist(kind),
                    &Query::new(id, page),
                    &referer,
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
                    return Ok(bytes);
                }
            }
        }
        Err(invalid())
    }
    pub(crate) async fn artist_albums_page(
        &self,
        artist: &Artist,
        page: u32,
    ) -> Result<ArtistPage<Album>> {
        self.unsigned_artist_page(UnsignedArtistEndpoint::Albums, artist, page, parse_albums)
            .await
    }
    pub(crate) async fn artist_mvs_page(
        &self,
        artist: &Artist,
        page: u32,
    ) -> Result<ArtistPage<Video>> {
        self.unsigned_artist_page(UnsignedArtistEndpoint::Mvs, artist, page, parse_mvs)
            .await
    }
    async fn unsigned_artist_page<T>(
        &self,
        kind: UnsignedArtistEndpoint,
        artist: &Artist,
        page: u32,
        parse: fn(&[u8], &Artist, u32) -> Result<ArtistPage<T>>,
    ) -> Result<ArtistPage<T>> {
        let id = &artist.id;
        let (url, path, operation) = kind.target();
        let started = Instant::now();
        let mut status = None;
        let result = async {
            // The official browser passes an absolute wapi URL to request export a.
            // It sends neither the www tracking Cookie nor Secret to this host.
            let response = self
                .http
                .get(self.web_target(url))
                .header(ACCEPT, "application/json")
                .header(REFERER, format!("https://www.kuwo.cn/singer_detail/{id}"))
                .query(&Query::new(id, Some(page)))
                .send()
                .await
                .map_err(kuwo_network_error)?;
            status = Some(response.status());
            let bytes = catalog::read_response(response).await?;
            parse(&bytes, artist, page)
        }
        .await;
        self.log_upstream_request(
            operation,
            "wapi.kuwo.cn",
            path,
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
struct Envelope {
    code: i64,
    data: Option<serde_json::Value>,
}
fn data(bytes: &[u8], detail: bool) -> Result<serde_json::Value> {
    let response: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if detail && response.code == -1 && response.data.is_none() {
        return Err(
            TuneWeaveError::new(ErrorCode::ResourceNotFound, "Kuwo artist was not found")
                .with_platform(Platform::Kuwo),
        );
    }
    if response.code != 200 {
        return Err(kuwo_upstream_error(
            "Kuwo artist request returned an unsuccessful business code",
        )
        .with_details(json!({"upstream_code":response.code})));
    }
    response.data.ok_or_else(invalid)
}

#[derive(Deserialize)]
struct Detail {
    #[serde(flatten)]
    artist: ArtistDto,
    #[serde(default)]
    info: String,
    aartist: Option<String>,
    #[serde(rename = "albumNum")]
    album_count: Option<Unsigned>,
    #[serde(rename = "mvNum")]
    mv_count: Option<Unsigned>,
    birthday: Option<String>,
    birthplace: Option<String>,
    gener: Option<String>,
    tall: Option<String>,
    weight: Option<String>,
    constellation: Option<String>,
    language: Option<String>,
}
fn parse_artist(bytes: &[u8], id: &str) -> Result<Artist> {
    let detail: Detail = serde_json::from_value(data(bytes, true)?).map_err(|_| invalid())?;
    let SearchItem::Artist(mut artist) = catalog::map_artist(detail.artist)? else {
        unreachable!("artist mapper")
    };
    if artist.id != id {
        return Err(kuwo_upstream_error(
            "Kuwo artist response has a different identity",
        ));
    }
    artist.description = catalog::text(&detail.info, 64 * 1024, true)?;
    if let Some(alias) = detail.aartist {
        let alias = catalog::text(&alias, 512, false)?;
        if !alias.is_empty() && alias != artist.name {
            artist.aliases.push(alias);
        }
    }
    artist.album_count = detail
        .album_count
        .as_ref()
        .map(Unsigned::value)
        .transpose()?;
    artist.mv_count = detail.mv_count.as_ref().map(Unsigned::value).transpose()?;
    for (key, value) in [
        ("birthday", detail.birthday),
        ("birthplace", detail.birthplace),
        ("gender", detail.gener),
        ("height", detail.tall),
        ("weight", detail.weight),
        ("constellation", detail.constellation),
        ("language", detail.language),
    ] {
        catalog::optional_text(&mut artist.extensions, key, value.as_deref())?;
    }
    artist
        .extensions
        .insert("backend".into(), json!("current_web_artist_detail"));
    Ok(artist)
}

#[derive(Deserialize)]
struct Songs {
    total: Unsigned,
    list: Vec<KuwoTrackDetail>,
}
fn parse_tracks(bytes: &[u8], artist: &Artist, page: u32) -> Result<ArtistPage<Track>> {
    let body: Songs = serde_json::from_value(data(bytes, false)?).map_err(|_| invalid())?;
    let total = body.total.value()?;
    check_count(total, page, body.list.len())?;
    let mut items = Vec::with_capacity(body.list.len());
    for mut detail in body.list {
        if detail
            .content_type
            .as_text()
            .is_some_and(|s| !s.is_empty() && s != "0")
            || !matches!(detail.ad_type.as_str(), "" | "0")
        {
            return Err(invalid());
        }
        let rid = detail.rid.as_text().ok_or_else(invalid)?;
        let credits = artist_credits(
            &detail.artist,
            &detail.artistid.as_text().ok_or_else(invalid)?,
            artist,
        )?;
        detail.name = catalog::text(&detail.name, 512, false)?;
        let mut track = map_track_detail(detail, &rid, "current_web_artist_music")?;
        track.artists = credits;
        track
            .extensions
            .insert("source_artist_id".into(), json!(artist.id));
        items.push(track);
    }
    Ok(ArtistPage { items, total })
}

fn artist_credits(raw_names: &str, raw_ids: &str, artist: &Artist) -> Result<Vec<ArtistSummary>> {
    let credits = credits(raw_names, raw_ids)?;
    let directly_bound = credits.iter().any(|credit| {
        credit
            .resource_ref
            .as_ref()
            .is_some_and(|id| id.id() == artist.id)
    });
    // The fixed artist collection may contain a collaboration with only a primary ID.
    let partial_collaboration = credits.len() > 1
        && credits[1..]
            .iter()
            .all(|credit| credit.resource_ref.is_none())
        && credits[1..]
            .iter()
            .any(|credit| credit.name == artist.name || artist.aliases.contains(&credit.name));
    if !directly_bound && !partial_collaboration {
        return Err(kuwo_upstream_error(
            "Kuwo artist catalogue returned a work with a different artist",
        ));
    }
    Ok(credits)
}

pub(super) fn credits(raw_names: &str, raw_ids: &str) -> Result<Vec<ArtistSummary>> {
    let mut names = Vec::new();
    let mut start = 0;
    // '&' separates names, while an HTML entity belongs to the name itself.
    for (offset, character) in raw_names.char_indices() {
        if character != '&'
            || ["&amp;", "&nbsp;", "&quot;", "&lt;", "&gt;", "&#39;"]
                .iter()
                .any(|entity| raw_names[offset..].starts_with(entity))
        {
            continue;
        }
        if names.len() == 15 {
            return Err(invalid());
        }
        names.push(catalog::text(&raw_names[start..offset], 512, false)?);
        start = offset + 1;
    }
    names.push(catalog::text(&raw_names[start..], 512, false)?);
    if names.len() > 16 || names.iter().any(String::is_empty) {
        return Err(invalid());
    }
    let ids = raw_ids
        .split('&')
        .take(17)
        .map(|id| canonical_positive_decimal(id).ok_or_else(invalid))
        .collect::<Result<Vec<_>>>()?;
    if ids.len() > 16 || (ids.len() != names.len() && ids.len() != 1) {
        return Err(invalid());
    }
    names
        .into_iter()
        .enumerate()
        .map(|(index, name)| {
            Ok(ArtistSummary {
                resource_ref: ids
                    .get(index)
                    .map(|id| {
                        ResourceRef::new(Platform::Kuwo, (*id).to_owned()).map_err(|_| invalid())
                    })
                    .transpose()?,
                name,
            })
        })
        .collect()
}
#[derive(Deserialize)]
struct Albums {
    total: Unsigned,
    #[serde(rename = "albumList")]
    items: Vec<AlbumDto>,
}
fn parse_albums(bytes: &[u8], artist: &Artist, page: u32) -> Result<ArtistPage<Album>> {
    let body: Albums = serde_json::from_value(data(bytes, false)?).map_err(|_| invalid())?;
    let total = body.total.value()?;
    check_count(total, page, body.items.len())?;
    let mut items = Vec::with_capacity(body.items.len());
    for item in body.items {
        let credits = artist_credits(
            &item.artist,
            &item.artistid.as_ref().ok_or_else(invalid)?.id()?,
            artist,
        )?;
        let SearchItem::Album(mut album) = catalog::map_album(item)? else {
            unreachable!("typed album mapper")
        };
        album.artists = credits;
        album
            .extensions
            .insert("source_artist_id".into(), json!(artist.id));
        items.push(album);
    }
    Ok(ArtistPage { items, total })
}
#[derive(Deserialize)]
struct Mvs {
    total: Unsigned,
    mvlist: Vec<Mv>,
}
#[derive(Deserialize)]
pub(super) struct Mv {
    id: Unsigned,
    name: String,
    artist: String,
    artistid: Unsigned,
    duration: Option<Unsigned>,
    #[serde(rename = "mvPlayCnt")]
    play_count: Option<Unsigned>,
    online: Option<Unsigned>,
    pic: Option<String>,
}
fn parse_mvs(bytes: &[u8], artist: &Artist, page: u32) -> Result<ArtistPage<Video>> {
    let body: Mvs = serde_json::from_value(data(bytes, false)?).map_err(|_| invalid())?;
    let total = body.total.value()?;
    check_count(total, page, body.mvlist.len())?;
    let items = body
        .mvlist
        .into_iter()
        .map(|item| map_mv(item, Some(artist)))
        .collect::<Result<Vec<_>>>()?;
    Ok(ArtistPage { items, total })
}

// Global MV search and artist MV pages share the official catalogue row shape.
// Only the artist-scoped endpoint may assert that every row belongs to its artist.
pub(super) fn map_mv(item: Mv, artist: Option<&Artist>) -> Result<Video> {
    let id = item.id.id()?;
    let title = catalog::text(&item.name, 512, false)?;
    if title.is_empty() {
        return Err(invalid());
    }
    let artist_id = item.artistid.id()?;
    let credits = match artist {
        Some(artist) => artist_credits(&item.artist, &artist_id, artist)?,
        None => credits(&item.artist, &artist_id)?,
    };
    let creators = credits
        .into_iter()
        .map(|credit| CreatorSummary {
            resource_ref: credit.resource_ref,
            name: credit.name,
            avatar_url: None,
        })
        .collect();
    let mut extensions = Extensions::from([
        (
            "backend".into(),
            json!(if artist.is_some() {
                "current_web_artist_mv"
            } else {
                "current_web_mv_search"
            }),
        ),
        ("kind".into(), json!("mv")),
        ("source_track_id".into(), json!(id)),
    ]);
    if let Some(artist) = artist {
        extensions.insert("source_artist_id".into(), json!(artist.id));
    }
    if let Some(online) = item.online {
        let value = online.value()?;
        if value > 1 {
            return Err(invalid());
        }
        extensions.insert("online".into(), json!(value));
    }
    Ok(Video {
        resource_ref: kuwo_track_ref(&id)?,
        platform: Platform::Kuwo,
        id,
        title,
        creators,
        description: String::new(),
        cover_url: item.pic.as_deref().and_then(normalize_official_image_url),
        duration_ms: item
            .duration
            .as_ref()
            .map(|duration| duration.value()?.checked_mul(1000).ok_or_else(invalid))
            .transpose()?,
        published_at: None,
        play_count: item.play_count.as_ref().map(Unsigned::value).transpose()?,
        subscribed: None,
        extensions,
    })
}

fn check_count(total: u64, page: u32, count: usize) -> Result<()> {
    let start = u64::from(page.checked_sub(1).ok_or_else(invalid)?) * u64::from(PAGE_SIZE);
    if count as u64 != total.saturating_sub(start).min(u64::from(PAGE_SIZE)) {
        return Err(invalid());
    }
    Ok(())
}
fn invalid() -> TuneWeaveError {
    kuwo_upstream_error("Kuwo artist returned an invalid typed response")
}

#[cfg(test)]
mod mv_tests;
#[cfg(test)]
mod tests;
