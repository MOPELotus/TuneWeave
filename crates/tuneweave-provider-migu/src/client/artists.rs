use super::*;
use serde_json::Value;
use tuneweave_core::{Artist, ArtistBiographySection, DigitalAlbum};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArtistOperation {
    Info,
    Biography,
    Songs,
    Albums,
}
impl ArtistOperation {
    fn path(self) -> &'static str {
        match self {
            Self::Info => "/pc/bmw/singer/info/v1.1",
            Self::Biography => "/bmw/singer/index/v1.0",
            Self::Songs => "/pc/bmw/singer/song/v1.0",
            Self::Albums => "/pc/bmw/singer/album/v1.0",
        }
    }
}
#[derive(Deserialize)]
struct Envelope {
    code: String,
    data: Option<Data>,
}
#[derive(Deserialize)]
pub(crate) struct Data {
    header: Header,
    contents: Vec<Value>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Header {
    next_page_url: Option<String>,
}
pub(crate) enum ArtistAlbum {
    Ordinary(tuneweave_core::Album),
    Digital(DigitalAlbum),
}
impl ArtistAlbum {
    pub(crate) fn identity(&self) -> (&'static str, &str) {
        match self {
            Self::Ordinary(v) => ("2003", &v.id),
            Self::Digital(v) => ("5", &v.id),
        }
    }
}
pub(crate) struct ArtistPage<T> {
    pub items: Vec<T>,
    pub has_more: bool,
}
fn invalid() -> TuneWeaveError {
    migu_upstream_error("Migu artist response has invalid identity or structure")
}
fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str> {
    v.get(key).and_then(Value::as_str).ok_or_else(invalid)
}
fn array<'a>(v: &'a Value, key: &str) -> Result<&'a [Value]> {
    v.get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(invalid)
}
fn text(v: &str, max: usize, multiline: bool) -> Result<String> {
    if v.trim().is_empty()
        || v.len() > max
        || v.chars()
            .any(|c| c.is_control() && !(multiline && matches!(c, '\n' | '\r' | '\t')))
    {
        return Err(invalid());
    }
    Ok(v.trim().into())
}
fn id(v: &str) -> Result<&str> {
    if v.is_empty() || v.len() > 64 || v.starts_with('0') || !v.bytes().all(|b| b.is_ascii_digit())
    {
        return Err(invalid());
    }
    Ok(v)
}
fn count(v: &Value, key: &str) -> Result<Option<u64>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s))
            if !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) =>
        {
            s.parse().map(Some).map_err(|_| invalid())
        }
        _ => Err(invalid()),
    }
}
fn image(v: &Value, key: &str) -> Result<Option<String>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) if s.is_empty() => Ok(None),
        Some(Value::String(s)) => super::albums::album_image_url(s)
            .map(Some)
            .ok_or_else(invalid),
        _ => Err(invalid()),
    }
}
fn action(value: &str, host: &str, expected: &str) -> Result<()> {
    let url = Url::parse(value).map_err(|_| invalid())?;
    let pairs = url.query_pairs().collect::<Vec<_>>();
    if url.scheme() != "mgmusic"
        || url.host_str() != Some(host)
        || !url.path().is_empty()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
        || pairs.len() != 1
        || pairs[0].0 != "id"
        || pairs[0].1 != expected
    {
        return Err(invalid());
    }
    Ok(())
}
fn continuation(header: &Header, operation: ArtistOperation, uid: &str, page: u32) -> Result<bool> {
    let Some(value) = header.next_page_url.as_deref().filter(|v| !v.is_empty()) else {
        return Ok(false);
    };
    if value.len() > 2048 {
        return Err(invalid());
    }
    let url = Url::parse(value).map_err(|_| invalid())?;
    let legacy = match operation {
        ArtistOperation::Songs => "/MIGUM3.0/bmw/singer/song/v1.0",
        ArtistOperation::Albums => "/MIGUM3.0/bmw/singer/album/v1.0",
        _ => return Err(invalid()),
    };
    if !matches!(url.scheme(), "https" | "http")
        || url.host_str() != Some("app.c.nf.migu.cn")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
        || ![operation.path(), legacy].contains(&url.path())
    {
        return Err(invalid());
    }
    let mut query = BTreeMap::new();
    for (k, v) in url.query_pairs() {
        if query.insert(k.into_owned(), v.into_owned()).is_some() {
            return Err(invalid());
        }
    }
    let mut expected = BTreeMap::from([
        ("singerId".into(), uid.into()),
        (
            "pageNo".into(),
            page.checked_add(1).ok_or_else(invalid)?.to_string(),
        ),
    ]);
    if operation == ArtistOperation::Songs {
        expected.insert("type".into(), "1".into());
    }
    if query != expected {
        return Err(invalid());
    }
    Ok(true)
}
pub(crate) fn metadata(data: Data, uid: &str) -> Result<Artist> {
    if data
        .header
        .next_page_url
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        return Err(invalid());
    }
    if data.contents.is_empty() {
        return Err(
            TuneWeaveError::new(ErrorCode::ResourceNotFound, "Migu artist was not found")
                .with_platform(Platform::Migu),
        );
    }
    if data.contents.len() > 64 {
        return Err(invalid());
    }
    let mut identity = None;
    let mut counts = BTreeMap::new();
    let mut tab_seen = false;
    for node in &data.contents {
        match string(node, "view")? {
            "ZJ-SingerDetail-Scroll" => {
                if identity.is_some() {
                    return Err(invalid());
                }
                let [item] = array(node, "contents")? else {
                    return Err(invalid());
                };
                if string(item, "view")? != "ZJ-SingerDetail-Item" {
                    return Err(invalid());
                }
                action(
                    string(item, "action")?,
                    "singer-detail-self-revealing-wall",
                    uid,
                )?;
                identity = Some(item);
            }
            "ZJ-Tab-Scroll" => {
                if tab_seen {
                    return Err(invalid());
                }
                tab_seen = true;
                let entries = array(node, "contents")?;
                if entries.len() > 32 {
                    return Err(invalid());
                }
                for item in entries {
                    if string(item, "view")? != "ZJ-Tab-Item" {
                        return Err(invalid());
                    }
                    let key = text(string(item, "action")?, 64, false)?;
                    let value = count(item, "txt2")?;
                    if counts.insert(key, value).is_some() {
                        return Err(invalid());
                    }
                }
            }
            _ => {}
        }
    }
    let item = identity.ok_or_else(invalid)?;
    let track_count = counts.get("song").copied().flatten();
    let album_count = counts.get("album").copied().flatten();
    Ok(Artist {
        resource_ref: ResourceRef::new(Platform::Migu, uid).map_err(|_| invalid())?,
        platform: Platform::Migu,
        id: uid.into(),
        name: text(string(item, "txt")?, 2048, false)?,
        aliases: vec![],
        description: String::new(),
        biography_sections: vec![],
        avatar_url: image(item, "img2")?,
        cover_url: image(item, "img")?,
        track_count,
        album_count,
        mv_count: None,
        video_count: None,
        identities: vec![],
        extensions: Extensions::from([
            ("backend".into(), json!("official_pc_artist")),
            ("fan_count".into(), json!(count(item, "txt4")?)),
            ("tab_counts".into(), json!(counts)),
            ("album_count_includes_digital".into(), json!(true)),
        ]),
    })
}
fn biography(data: Data) -> Result<Vec<ArtistBiographySection>> {
    if data
        .header
        .next_page_url
        .as_deref()
        .is_some_and(|value| !value.is_empty())
    {
        return Err(invalid());
    }
    if data.contents.len() > 64 {
        return Err(invalid());
    }
    let mut sections = None;
    for node in &data.contents {
        if string(node, "view")? == "ZJ-Singer-Intro-Scroll" {
            if sections.is_some() {
                return Err(invalid());
            }
            let entries = array(node, "contents")?;
            if entries.len() > 32 {
                return Err(invalid());
            }
            sections = Some(
                entries
                    .iter()
                    .map(|item| {
                        if string(item, "view")? != "ZJ-Singer-Intro-Item" {
                            return Err(invalid());
                        }
                        Ok(ArtistBiographySection {
                            title: text(string(item, "txt")?, 2048, false)?,
                            text: text(string(item, "txt2")?, 131072, true)?,
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
            );
        }
    }
    Ok(sections.unwrap_or_default())
}
pub(crate) fn songs(data: Data, uid: &str, page: u32) -> Result<ArtistPage<Track>> {
    if data.contents.len() > 64 {
        return Err(invalid());
    }
    let more = continuation(&data.header, ArtistOperation::Songs, uid, page)?;
    let mut rows = None;
    for node in &data.contents {
        match string(node, "view")? {
            "ZJ-Singer-Song-Scroll" => {
                if rows.is_some() {
                    return Err(invalid());
                }
                rows = Some(array(node, "contents")?);
            }
            "ZJ-Img-Scroll" | "ZJ-Title" => {}
            _ => return Err(invalid()),
        }
    }
    let rows = rows.unwrap_or_default();
    if rows.len() > 50 || more && rows.is_empty() {
        return Err(invalid());
    }
    let mut items = Vec::new();
    for item in rows {
        if string(item, "view")? != "ZJ-Singer-Song-Item" || string(item, "resType")? != "4001" {
            return Err(invalid());
        }
        let resource_id = id(string(item, "resId")?)?;
        action(string(item, "action")?, "song-player", resource_id)?;
        let song: MiguSong =
            serde_json::from_value(item.get("songItem").cloned().ok_or_else(invalid)?)
                .map_err(|_| invalid())?;
        if song.content_id != resource_id
            || song.singer_list.is_empty()
            || song.singer_list.len() > 32
            || !song.singer_list.iter().any(|s| s.id == uid)
        {
            return Err(invalid());
        }
        for singer in &song.singer_list {
            id(&singer.id)?;
            text(&singer.name, 2048, false)?;
        }
        let mut track = map_song(song)?;
        track
            .extensions
            .insert("source_artist_id".into(), json!(uid));
        items.push(track);
    }
    Ok(ArtistPage {
        items,
        has_more: more,
    })
}
pub(crate) fn albums(data: Data, uid: &str, page: u32) -> Result<ArtistPage<ArtistAlbum>> {
    if data.contents.len() > 10 {
        return Err(invalid());
    }
    let more = continuation(&data.header, ArtistOperation::Albums, uid, page)?;
    if more && data.contents.is_empty() {
        return Err(invalid());
    }
    let mut items = Vec::new();
    for item in &data.contents {
        if string(item, "view")? != "ZJ-Album-Item" {
            return Err(invalid());
        }
        let id = id(string(item, "resId")?)?;
        let digital = match string(item, "resType")? {
            "0" => false,
            "1" => true,
            _ => return Err(invalid()),
        };
        action(
            string(item, "action")?,
            if digital {
                "digital-album-info"
            } else {
                "album-info"
            },
            id,
        )?;
        let resource_ref = ResourceRef::new(Platform::Migu, id).map_err(|_| invalid())?;
        let name = text(string(item, "txt")?, 2048, false)?;
        let artist_name = text(string(item, "txt2")?, 8192, false)?;
        let artists = vec![ArtistSummary {
            resource_ref: None,
            name: artist_name,
        }];
        let cover_url = image(item, "img")?;
        let published_at = match item.get("txt3") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) if s.is_empty() => None,
            Some(Value::String(s)) => Some(text(s, 64, false)?),
            _ => return Err(invalid()),
        };
        let extensions = Extensions::from([
            ("backend".into(), json!("official_pc_artist_albums")),
            (
                "resource_type".into(),
                json!(if digital { "5" } else { "2003" }),
            ),
            ("source_artist_id".into(), json!(uid)),
        ]);
        items.push(if digital {
            ArtistAlbum::Digital(DigitalAlbum {
                resource_ref,
                platform: Platform::Migu,
                id: id.into(),
                name,
                artists,
                description: String::new(),
                cover_url,
                published_at,
                price: None,
                is_free: None,
                purchasable: None,
                purchased: None,
                sale_count: None,
                track_count: None,
                tags: vec![],
                extensions,
            })
        } else {
            ArtistAlbum::Ordinary(tuneweave_core::Album {
                resource_ref,
                platform: Platform::Migu,
                id: id.into(),
                name,
                aliases: vec![],
                artists,
                description: String::new(),
                cover_url,
                published_at,
                track_count: None,
                company: None,
                kind: None,
                extensions,
            })
        });
    }
    Ok(ArtistPage {
        items,
        has_more: more,
    })
}
impl MiguClient {
    pub(crate) async fn artist_response(
        &self,
        operation: ArtistOperation,
        uid: &str,
        page: u32,
    ) -> Result<Data> {
        let started = Instant::now();
        let mut status = None;
        let path = operation.path();
        let result = async {
            let url =
                Url::parse(&format!("https://app.c.nf.migu.cn{path}")).map_err(|_| invalid())?;
            #[cfg(test)]
            let url = if let Some(origin) = &self.catalog_test_origin {
                origin.join(path).map_err(|_| invalid())?
            } else {
                url
            };
            let mut request = self
                .http
                .get(url)
                .header(ACCEPT, "application/json")
                .query(&[("singerId", uid)]);
            if matches!(operation, ArtistOperation::Songs | ArtistOperation::Albums) {
                request = request.query(&[("pageNo", page)]);
            }
            if operation == ArtistOperation::Songs {
                request = request.query(&[("type", "1")]);
            }
            let response = request.send().await.map_err(migu_network_error)?;
            status = Some(response.status());
            if response.status().is_success()
                && !response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.split(';').next())
                    .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
            {
                return Err(invalid());
            }
            let bytes = read_bounded_response(response, "Migu artist").await?;
            let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
            if envelope.code != "000000" {
                return Err(migu_upstream_error("Migu artist request was rejected"));
            }
            envelope.data.ok_or_else(invalid)
        }
        .await;
        self.log_upstream_request(
            "artist_catalog",
            "app.c.nf.migu.cn",
            path,
            status,
            started,
            &result,
        );
        result
    }
    pub(crate) async fn artist_info(&self, uid: &str) -> Result<Artist> {
        metadata(
            self.artist_response(ArtistOperation::Info, uid, 1).await?,
            uid,
        )
    }
    pub(crate) async fn artist_biography(&self, uid: &str) -> Result<Vec<ArtistBiographySection>> {
        biography(
            self.artist_response(ArtistOperation::Biography, uid, 1)
                .await?,
        )
    }
}

#[cfg(test)]
pub(crate) mod tests;
