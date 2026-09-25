use super::*;
use tuneweave_core::{Album, DigitalAlbum};

pub(crate) const MAX_PAGE_ITEMS: usize = 1_000;
pub(crate) const MAX_ALBUM_ITEMS: usize = 10_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AlbumKind {
    Ordinary,
    Digital,
}
impl AlbumKind {
    pub(crate) fn resource_type(self) -> &'static str {
        match self {
            Self::Ordinary => "2003",
            Self::Digital => "5",
        }
    }
    pub(super) fn account_detail_path(self) -> &'static str {
        match self {
            Self::Ordinary => "/resource/album/v2.0",
            Self::Digital => "/pc/resource/dalbum/v2.0",
        }
    }
    fn detail_path(self) -> &'static str {
        match self {
            Self::Ordinary => "/MIGUM3.0/resource/album/v2.0",
            Self::Digital => "/MIGUM3.0/resource/dalbum/v2.0",
        }
    }
    fn tracks_path(self) -> &'static str {
        match self {
            Self::Ordinary => "/MIGUM3.0/resource/album/song/v2.0",
            Self::Digital => "/MIGUM3.0/resource/dalbum/song/v2.0",
        }
    }
    pub(super) fn parameter(self) -> &'static str {
        match self {
            Self::Ordinary => "albumId",
            Self::Digital => "dAlbumId",
        }
    }
    pub(crate) fn source_type(self) -> &'static str {
        match self {
            Self::Ordinary => "album",
            Self::Digital => "digital_album",
        }
    }
}

#[derive(Deserialize)]
struct Envelope<T> {
    code: String,
    data: Option<T>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AlbumMetadata {
    resource_type: String,
    album_id: String,
    title: String,
    singer: Option<String>,
    singer_id: Option<String>,
    summary: Option<String>,
    #[serde(default)]
    img_items: Vec<AlbumImage>,
    total_count: Option<FlexibleU64>,
    publish_date: Option<String>,
    publish_time: Option<String>,
    publish_company: Option<String>,
    album_class: Option<String>,
    album_alias_name: Option<String>,
    translate_name: Option<String>,
    language: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct DigitalAlbumMetadata {
    resource_type: String,
    content_id: String,
    title: String,
    item_id: Option<String>,
    copyright_id: Option<String>,
    singer: Option<String>,
    singer_id: Option<String>,
    summary: Option<String>,
    #[serde(default)]
    img_items: Vec<AlbumImage>,
    #[serde(default)]
    img_item: Vec<AlbumImage>,
    total_count: Option<FlexibleU64>,
    publish_date: Option<String>,
}

#[derive(Deserialize)]
struct AlbumImage {
    #[serde(default)]
    img: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TrackData {
    song_list: Vec<MiguSong>,
    total_count: Option<FlexibleU64>,
    has_next: Option<bool>,
}

pub(crate) struct AlbumTrackPage {
    pub tracks: Vec<Track>,
    pub total: Option<u64>,
    pub has_more: bool,
}

impl MiguClient {
    async fn album_response<T: serde::de::DeserializeOwned>(
        &self,
        kind: AlbumKind,
        id: &str,
        page: Option<u32>,
    ) -> Result<T> {
        let started = Instant::now();
        let mut http_status = None;
        let path = if page.is_some() {
            kind.tracks_path()
        } else {
            kind.detail_path()
        };
        let outcome = async {
            let url = Url::parse(&format!("https://app.c.nf.migu.cn{path}"))
                .map_err(|_| migu_upstream_error("Migu album endpoint is invalid"))?;
            #[cfg(test)]
            let url = if let Some(origin) = &self.catalog_test_origin {
                origin
                    .join(path)
                    .map_err(|_| migu_upstream_error("Invalid test album endpoint"))?
            } else {
                url
            };
            let mut request = self
                .http
                .get(url)
                .header(ACCEPT, "application/json")
                .query(&[(kind.parameter(), id)]);
            if let (AlbumKind::Ordinary, Some(page)) = (kind, page) {
                request = request.query(&[("pageNo", page)]);
            }
            let response = request.send().await.map_err(migu_network_error)?;
            http_status = Some(response.status());
            if response.status().is_success()
                && response
                    .headers()
                    .get(reqwest::header::CONTENT_TYPE)
                    .is_some_and(|v| {
                        v.to_str().map_or(true, |v| {
                            !v.split(';')
                                .next()
                                .unwrap_or_default()
                                .trim()
                                .eq_ignore_ascii_case("application/json")
                        })
                    })
            {
                return Err(migu_upstream_error(
                    "Migu album returned an unexpected content type",
                ));
            }
            let body = read_bounded_response(response, "Migu album").await?;
            let envelope: Envelope<T> = serde_json::from_slice(&body)
                .map_err(|_| migu_upstream_error("Migu album returned malformed data"))?;
            if envelope.code != "000000" {
                return Err(migu_upstream_error("Migu album request was rejected")
                    .with_details(json!({"platform_code":bounded_text(&envelope.code,64)})));
            }
            envelope.data.ok_or_else(|| {
                TuneWeaveError::new(ErrorCode::ResourceNotFound, "Migu album data was not found")
                    .with_platform(Platform::Migu)
            })
        }
        .await;
        self.log_upstream_request(
            if page.is_some() {
                "album_tracks"
            } else {
                "album_detail"
            },
            "app.c.nf.migu.cn",
            path,
            http_status,
            started,
            &outcome,
        );
        outcome
    }

    pub(crate) async fn album_metadata(&self, id: &str) -> Result<Album> {
        let value: AlbumMetadata = self.album_response(AlbumKind::Ordinary, id, None).await?;
        if value.album_id != id {
            return Err(migu_upstream_error(
                "Migu album detail returned a mismatched ID",
            ));
        }
        map_album_metadata(value, "official_album_detail")
    }

    pub(crate) async fn digital_album_metadata(&self, id: &str) -> Result<DigitalAlbum> {
        let value: DigitalAlbumMetadata = self.album_response(AlbumKind::Digital, id, None).await?;
        if value.content_id != id {
            return Err(migu_upstream_error(
                "Migu digital album detail returned a mismatched content ID",
            ));
        }
        map_digital_album_metadata(value, "official_digital_album_detail")
    }

    pub(crate) async fn album_track_page(
        &self,
        kind: AlbumKind,
        id: &str,
        page: u32,
    ) -> Result<AlbumTrackPage> {
        let data: TrackData = self.album_response(kind, id, Some(page)).await?;
        if data.song_list.len() > MAX_PAGE_ITEMS {
            return Err(migu_upstream_error(
                "Migu album page exceeded its size limit",
            ));
        }
        let total = optional_count(data.total_count.as_ref())?;
        let has_more = match kind {
            AlbumKind::Ordinary => data.has_next.ok_or_else(|| {
                migu_upstream_error("Migu album page omitted continuation status")
            })?,
            AlbumKind::Digital => {
                if data.has_next == Some(true) || total != Some(data.song_list.len() as u64) {
                    return Err(migu_upstream_error(
                        "Migu digital album did not return its complete counted track list",
                    ));
                }
                false
            }
        };
        if has_more && data.song_list.is_empty() {
            return Err(migu_upstream_error(
                "Migu album cannot continue with an empty page",
            ));
        }
        let tracks = data
            .song_list
            .into_iter()
            .map(|song| {
                if matches!(kind, AlbumKind::Ordinary) && song.album_id != id {
                    return Err(migu_upstream_error(
                        "Migu track belongs to a different ordinary album",
                    ));
                }
                // A digital product groups tracks whose own albumId may be a different,
                // ordinary album. Keep that association instead of replacing it.
                map_song(song)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(AlbumTrackPage {
            tracks,
            total,
            has_more,
        })
    }
}

pub(super) fn map_album_metadata(value: AlbumMetadata, backend: &str) -> Result<Album> {
    if value.resource_type != "2003" {
        return Err(migu_upstream_error(
            "Migu ordinary album returned the wrong resource type",
        ));
    }
    let id = metadata_id(&value.album_id)?;
    let name = metadata_name(&value.title)?;
    let mut aliases = Vec::new();
    for alias in [
        value.album_alias_name.as_deref(),
        value.translate_name.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        push_alias(&mut aliases, name, alias);
    }
    let mut extensions = Extensions::from([
        ("backend".to_owned(), json!(backend)),
        ("resource_type".to_owned(), json!("2003")),
    ]);
    if let Some(language) = optional_text(value.language, 256)? {
        extensions.insert("language".to_owned(), json!(language));
    }
    let published_at = publication_date(
        value
            .publish_date
            .as_deref()
            .or(value.publish_time.as_deref()),
    )?;
    Ok(Album {
        resource_ref: ResourceRef::new(Platform::Migu, id)
            .map_err(|_| migu_upstream_error("Migu album identity is invalid"))?,
        platform: Platform::Migu,
        id: id.to_owned(),
        name: name.to_owned(),
        aliases,
        artists: album_artists(value.singer.as_deref(), value.singer_id.as_deref())?,
        description: description(value.summary)?,
        cover_url: album_cover(&value.img_items)?,
        published_at,
        track_count: optional_count(value.total_count.as_ref())?,
        company: optional_text(value.publish_company, 2048)?,
        kind: optional_text(value.album_class, 256)?,
        extensions,
    })
}

pub(super) fn map_digital_album_metadata(
    value: DigitalAlbumMetadata,
    backend: &str,
) -> Result<DigitalAlbum> {
    if value.resource_type != "5" {
        return Err(migu_upstream_error(
            "Migu digital album returned the wrong resource type",
        ));
    }
    let id = metadata_id(&value.content_id)?;
    let name = metadata_name(&value.title)?;
    let mut extensions = Extensions::from([
        ("backend".to_owned(), json!(backend)),
        ("resource_type".to_owned(), json!("5")),
    ]);
    for (key, value) in [
        ("item_id", value.item_id.as_deref()),
        ("copyright_id", value.copyright_id.as_deref()),
    ] {
        if let Some(value) = value.filter(|v| !v.is_empty()) {
            let value = canonical_platform_id(value).ok_or_else(|| {
                migu_upstream_error("Migu digital album product identity is invalid")
            })?;
            extensions.insert(key.to_owned(), json!(value));
        }
    }
    Ok(DigitalAlbum {
        resource_ref: ResourceRef::new(Platform::Migu, id)
            .map_err(|_| migu_upstream_error("Migu digital album identity is invalid"))?,
        platform: Platform::Migu,
        id: id.to_owned(),
        name: name.to_owned(),
        artists: album_artists(value.singer.as_deref(), value.singer_id.as_deref())?,
        description: description(value.summary)?,
        cover_url: album_cover(&value.img_items)?.or(album_cover(&value.img_item)?),
        published_at: publication_date(value.publish_date.as_deref())?,
        price: None,
        is_free: None,
        purchasable: None,
        purchased: None,
        sale_count: None,
        track_count: optional_count(value.total_count.as_ref())?,
        tags: Vec::new(),
        extensions,
    })
}

fn metadata_id(value: &str) -> Result<&str> {
    if value.is_empty()
        || value.len() > 64
        || value.starts_with('0')
        || !value.bytes().all(|v| v.is_ascii_digit())
    {
        return Err(migu_upstream_error("Migu album returned an invalid ID"));
    }
    Ok(value)
}
fn metadata_name(value: &str) -> Result<&str> {
    validated_name(value).ok_or_else(|| migu_upstream_error("Migu album omitted a valid title"))
}
fn optional_count(value: Option<&FlexibleU64>) -> Result<Option<u64>> {
    value
        .map(|value| match value {
            FlexibleU64::Number(n) => Ok(*n),
            FlexibleU64::String(s) if !s.is_empty() && s.bytes().all(|v| v.is_ascii_digit()) => s
                .parse()
                .map_err(|_| migu_upstream_error("Migu album count overflowed")),
            _ => Err(migu_upstream_error("Migu album count is invalid")),
        })
        .transpose()
}
fn album_artists(name: Option<&str>, id: Option<&str>) -> Result<Vec<ArtistSummary>> {
    let Some(name) = name.filter(|name| !name.trim().is_empty()) else {
        return Ok(Vec::new());
    };
    let name = metadata_name(name)?;
    Ok(vec![ArtistSummary {
        resource_ref: id
            .and_then(canonical_platform_id)
            .and_then(|id| ResourceRef::new(Platform::Migu, id).ok()),
        name: name.to_owned(),
    }])
}
fn description(value: Option<String>) -> Result<String> {
    let value = value.unwrap_or_default();
    if value.len() > 256 * 1024
        || value
            .chars()
            .any(|v| v.is_control() && !matches!(v, '\n' | '\r' | '\t'))
    {
        return Err(migu_upstream_error(
            "Migu album description exceeded its bounds",
        ));
    }
    Ok(value.trim().to_owned())
}
fn optional_text(value: Option<String>, limit: usize) -> Result<Option<String>> {
    value
        .filter(|v| !v.trim().is_empty())
        .map(|v| {
            if v.len() > limit || v.chars().any(char::is_control) {
                Err(migu_upstream_error("Migu album text is invalid"))
            } else {
                Ok(v.trim().to_owned())
            }
        })
        .transpose()
}
fn album_cover(images: &[AlbumImage]) -> Result<Option<String>> {
    if images.len() > 64 {
        return Err(migu_upstream_error("Migu album returned too many images"));
    }
    Ok(images.iter().find_map(|image| album_image_url(&image.img)))
}
pub(super) fn album_image_url(value: &str) -> Option<String> {
    if let Some(url) = normalize_media_url(value) {
        return Some(url);
    }
    if value.len() > 8192 {
        return None;
    }
    let mut url = Url::parse(value).ok()?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str() != Some(MEDIA_HOST)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.fragment().is_some()
    {
        return None;
    }
    let tail = url
        .path()
        .strip_prefix("/prod/file-service/file-down01/")
        .or_else(|| url.path().strip_prefix("/prod/file-service/file-down/"))?;
    let parts: Vec<_> = tail.split('/').collect();
    if parts.len() != 3
        || !parts
            .iter()
            .all(|part| part.len() == 32 && part.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return None;
    }
    // The official file-service path was verified to serve the same image via HTTPS.
    url.set_scheme("https").ok()?;
    Some(url.into())
}
fn publication_date(value: Option<&str>) -> Result<Option<String>> {
    let Some(value) = value.filter(|v| !v.is_empty()) else {
        return Ok(None);
    };
    let invalid = || migu_upstream_error("Migu album publication date is invalid");
    if value.len() != 10
        || value.as_bytes()[4] != b'-'
        || value.as_bytes()[7] != b'-'
        || !value
            .bytes()
            .enumerate()
            .all(|(i, b)| matches!(i, 4 | 7) || b.is_ascii_digit())
    {
        return Err(invalid());
    }
    let year = value[..4].parse::<u32>().map_err(|_| invalid())?;
    let month = value[5..7].parse::<u32>().map_err(|_| invalid())?;
    let day = value[8..].parse::<u32>().map_err(|_| invalid())?;
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 400 == 0 || (year % 4 == 0 && year % 100 != 0) => 29,
        2 => 28,
        _ => return Err(invalid()),
    };
    if year == 0 || day == 0 || day > days {
        return Err(invalid());
    }
    Ok(Some(value.to_owned()))
}

#[cfg(test)]
mod tests;
