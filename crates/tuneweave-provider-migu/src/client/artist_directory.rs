//! Public native singer directory: dynamic taxonomy followed by one complete view.
use super::*;
use serde_json::Value;
use std::collections::BTreeSet;
use tuneweave_core::{
    Artist, ArtistArea, ArtistCatalog, ArtistCatalogFilterOption, ArtistCatalogFilters,
    ArtistCategory, ArtistGenre, Capability,
};

pub(crate) const TABS_PATH: &str = "/MIGUM3.0/bmw/singer-index/tabs/v2.0";
pub(crate) const LIST_PATH: &str = "/MIGUM3.0/bmw/singer-index/list/v1.0";
const MAX_RESPONSE: u64 = 2 * 1024 * 1024;
const MAX_ARTISTS: usize = 10_000;

pub(crate) struct Selection {
    area: ArtistArea,
    category: ArtistCategory,
    group: &'static str,
    subtype: &'static str,
}

impl Selection {
    pub(crate) fn new(
        area: ArtistArea,
        category: ArtistCategory,
        genre: ArtistGenre,
    ) -> Result<Self> {
        let unsupported = || TuneWeaveError::unsupported(Platform::Migu, Capability::ArtistCatalog);
        let group = match area {
            ArtistArea::Chinese => "huayu",
            ArtistArea::Western => "oumei",
            ArtistArea::JapaneseKorean => "rihan",
            _ => return Err(unsupported()),
        };
        let subtype = match category {
            ArtistCategory::Male => "nan",
            ArtistCategory::Female => "nv",
            ArtistCategory::Group => "group",
            ArtistCategory::All => return Err(unsupported()),
        };
        if genre != ArtistGenre::All {
            return Err(unsupported());
        }
        Ok(Self {
            area,
            category,
            group,
            subtype,
        })
    }
}

#[derive(Deserialize)]
struct Envelope {
    code: String,
    data: Option<Data>,
}

#[derive(Deserialize)]
struct Data {
    header: Header,
    contents: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Header {
    title: String,
    data_version: String,
    next_page_no: u32,
    next_page_no2: u32,
    update: bool,
    next_page_url: Option<String>,
    has_next_page: Option<bool>,
    has_next: Option<bool>,
}

fn invalid() -> TuneWeaveError {
    migu_upstream_error("Migu artist directory response has invalid identity or structure")
}

fn text<'a>(value: &'a Value, key: &str, limit: usize) -> Result<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| {
            !s.trim().is_empty()
                && s.len() <= limit
                && *s == s.trim()
                && !s.chars().any(char::is_control)
        })
        .ok_or_else(invalid)
}

fn array<'a>(value: &'a Value, key: &str) -> Result<&'a [Value]> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .ok_or_else(invalid)
}

fn decode(bytes: &[u8]) -> Result<Data> {
    let envelope: Envelope = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if envelope.code != "000000" {
        return Err(migu_upstream_error(
            "Migu artist directory request was rejected",
        ));
    }
    let data = envelope.data.ok_or_else(invalid)?;
    let header = &data.header;
    // These observed headers belong to an unpaged, full-view producer. Never
    // reinterpret a changed continuation as permission to return a partial list.
    if header.next_page_no != 1
        || header.next_page_no2 != 1
        || header.update
        || header
            .next_page_url
            .as_deref()
            .is_some_and(|s| !s.is_empty())
        || header.has_next_page == Some(true)
        || header.has_next == Some(true)
        || header.title.trim().is_empty()
        || header.title.len() > 256
        || header.title.chars().any(char::is_control)
        || header.data_version.is_empty()
        || header.data_version.len() > 64
        || !header.data_version.bytes().all(|c| c.is_ascii_digit())
    {
        return Err(invalid());
    }
    Ok(data)
}

fn option(id: &str, name: &str, source: &str) -> ArtistCatalogFilterOption {
    ArtistCatalogFilterOption {
        id: id.into(),
        name: name.into(),
        extensions: Extensions::from([("source_key".into(), json!(source))]),
    }
}

fn taxonomy(data: &Data, selected: &Selection) -> Result<ArtistCatalogFilters> {
    if data.contents.is_empty() || data.contents.len() > 32 || data.contents.len() % 2 != 0 {
        return Err(invalid());
    }
    let mut areas = Vec::new();
    let mut categories = Vec::new();
    let mut seen = BTreeSet::new();
    let mut selected_found = false;
    for pair in data.contents.chunks_exact(2) {
        if text(&pair[0], "view", 64)? != "ZJ-Title"
            || text(&pair[1], "view", 64)? != "ZJ-SubTab-Scroll"
        {
            return Err(invalid());
        }
        let [title] = array(&pair[0], "contents")? else {
            return Err(invalid());
        };
        if text(title, "view", 64)? != "ZJ-Title" {
            return Err(invalid());
        }
        let group = text(title, "txt2", 32)?;
        let label = text(title, "txt", 32)?;
        if !seen.insert(group) {
            return Err(invalid());
        }
        let area = match (group, label) {
            ("huayu", "华语") => "chinese",
            ("oumei", "欧美") => "western",
            ("rihan", "日韩") => "japanese_korean",
            _ => return Err(invalid()),
        };
        areas.push(option(area, label, group));
        let values = array(&pair[1], "contents")?;
        if values.is_empty() || values.len() > 3 {
            return Err(invalid());
        }
        let mut subtypes = BTreeSet::new();
        let mut selected_subtype = false;
        for value in values {
            if text(value, "view", 64)? != "ZJ-Tab-Item" {
                return Err(invalid());
            }
            let key = text(value, "action", 32)?;
            let label = text(value, "txt", 32)?;
            let category = match (key, label) {
                ("nan", "男") => "male",
                ("nv", "女") => "female",
                ("group", "组合") => "group",
                _ => return Err(invalid()),
            };
            if !subtypes.insert(key) {
                return Err(invalid());
            }
            if group == selected.group {
                categories.push(option(category, label, key));
                selected_subtype |= key == selected.subtype;
            }
        }
        if group == selected.group {
            selected_found = selected_subtype;
        }
    }
    if !selected_found {
        return Err(invalid());
    }
    Ok(ArtistCatalogFilters {
        areas,
        categories,
        genres: vec![ArtistCatalogFilterOption {
            id: "all".into(),
            name: "全部".into(),
            extensions: Extensions::from([("upstream_filter_sent".into(), json!(false))]),
        }],
        initials: Vec::new(),
        extensions: Extensions::from([
            ("area_all_supported".into(), json!(false)),
            ("category_all_supported".into(), json!(false)),
            ("genre_filter_supported".into(), json!(false)),
            (
                "source_data_version".into(),
                json!(data.header.data_version),
            ),
        ]),
    })
}

fn row(value: &Value) -> Result<Artist> {
    if text(value, "view", 64)? != "ZJ-Singer-Item" || text(value, "resType", 16)? != "2002" {
        return Err(invalid());
    }
    let id = text(value, "txt2", 64)?;
    if id.starts_with('0')
        || !id.bytes().all(|c| c.is_ascii_digit())
        || text(value, "resId", 64)? != id
    {
        return Err(invalid());
    }
    let action = Url::parse(text(value, "action", 512)?).map_err(|_| invalid())?;
    let pairs = action.query_pairs().collect::<Vec<_>>();
    if action.scheme() != "mgmusic"
        || action.host_str() != Some("singer-info")
        || !action.path().is_empty()
        || !action.username().is_empty()
        || action.password().is_some()
        || action.port().is_some()
        || action.fragment().is_some()
        || pairs.len() != 1
        || pairs[0].0 != "id"
        || pairs[0].1 != id
    {
        return Err(invalid());
    }
    let image = match value.get("img") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if s.is_empty() => None,
        Some(Value::String(s)) => {
            let url = Url::parse(s).map_err(|_| invalid())?;
            if s.len() > 8192
                || s.chars().any(char::is_control)
                || url.scheme() != "https"
                || url.host_str() != Some(MEDIA_HOST)
                || !url.username().is_empty()
                || url.password().is_some()
                || url.port().is_some()
                || !image_path(url.path())
                || url.path().contains('%')
                || url.query().is_some()
                || url.fragment().is_some()
            {
                return Err(invalid());
            }
            Some(s.clone())
        }
        _ => return Err(invalid()),
    };
    let mut extensions = Extensions::new();
    if let Some(fans) = value.get("txt4").filter(|v| !v.is_null()) {
        let fans = fans.as_str().ok_or_else(invalid)?;
        if !fans.is_empty() {
            if fans.len() > 20 || !fans.bytes().all(|v| v.is_ascii_digit()) {
                return Err(invalid());
            }
            let fans: u64 = fans.parse().map_err(|_| invalid())?;
            extensions.insert("follower_count".into(), json!(fans));
        }
    }
    extensions.insert("source_initial".into(), json!(text(value, "txt3", 8)?));
    Ok(Artist {
        resource_ref: ResourceRef::new(Platform::Migu, id).map_err(|_| invalid())?,
        platform: Platform::Migu,
        id: id.into(),
        name: text(value, "txt", 2048)?.into(),
        aliases: Vec::new(),
        description: String::new(),
        biography_sections: Vec::new(),
        avatar_url: image,
        cover_url: None,
        album_count: None,
        track_count: None,
        mv_count: None,
        video_count: None,
        identities: Vec::new(),
        extensions,
    })
}

fn image_path(path: &str) -> bool {
    if let Some(tail) = path.strip_prefix("/data/oss/resource/") {
        return !tail.is_empty()
            && tail.split('/').all(|part| {
                !part.is_empty()
                    && !matches!(part, "." | "..")
                    && part
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
            });
    }
    // Both legacy forms are explicit HTTPS avatar locations in the same
    // successful directory response. They are metadata only, never fetched here.
    if let Some(tail) = path.strip_prefix("/data/resource-service/file-down/") {
        let parts: Vec<_> = tail.split('/').collect();
        return parts.len() == 4
            && parts
                .iter()
                .all(|part| part.len() == 2 && part.bytes().all(|c| c.is_ascii_alphanumeric()));
    }
    if let Some(tail) = path.strip_prefix("/prod/file-service/file-down/") {
        let parts: Vec<_> = tail.split('/').collect();
        return parts.len() == 3
            && parts
                .iter()
                .all(|part| part.len() == 32 && part.bytes().all(|c| c.is_ascii_hexdigit()));
    }
    false
}

fn catalog(
    data: Data,
    selection: Selection,
    mut filters: ArtistCatalogFilters,
) -> Result<ArtistCatalog> {
    if data.contents.len() > MAX_ARTISTS {
        return Err(invalid());
    }
    let mut featured_artists = Vec::new();
    let mut artists = Vec::new();
    let mut seen_featured = BTreeSet::new();
    let mut seen_artists = BTreeSet::new();
    let mut identities = BTreeMap::new();
    let mut initials = BTreeSet::new();
    for value in &data.contents {
        let artist = row(value)?;
        let initial = text(value, "txt3", 8)?;
        let identity = (artist.name.clone(), artist.avatar_url.clone());
        if identities
            .insert(artist.id.clone(), identity.clone())
            .is_some_and(|previous| previous != identity)
        {
            return Err(invalid());
        }
        if initial == "热" {
            if !artists.is_empty() || !seen_featured.insert(artist.id.clone()) {
                return Err(invalid());
            }
            featured_artists.push(artist);
        } else {
            if !(initial == "#"
                || (initial.len() == 1 && initial.as_bytes()[0].is_ascii_uppercase()))
                || !seen_artists.insert(artist.id.clone())
            {
                return Err(invalid());
            }
            if initials.insert(initial.to_owned()) {
                filters.initials.push(option(initial, initial, initial));
            }
            artists.push(artist);
        }
    }
    Ok(ArtistCatalog {
        platform: Platform::Migu,
        area: selection.area,
        category: selection.category,
        genre: ArtistGenre::All,
        featured_artists,
        artists,
        filters,
        extensions: Extensions::from([
            ("backend".into(), json!("official_native_singer_directory")),
            (
                "source_tab".into(),
                json!(format!("{}-{}", selection.group, selection.subtype)),
            ),
            (
                "source_data_version".into(),
                json!(data.header.data_version),
            ),
            ("complete_read".into(), json!(true)),
            ("upstream_pages_fetched".into(), json!(1)),
            ("upstream_raw_count".into(), json!(data.contents.len())),
            (
                "upstream_unique_artist_count".into(),
                json!(identities.len()),
            ),
        ]),
    })
}

impl MiguClient {
    async fn artist_directory_response(
        &self,
        path: &'static str,
        tab: Option<&str>,
    ) -> Result<Data> {
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let url = self.catalog_endpoint(&format!("https://app.c.nf.migu.cn{path}"))?;
            // This public endpoint was independently observed without native
            // device/token/CE headers. Do not borrow any account identity.
            let mut request = self.http.get(url).header(ACCEPT, "application/json");
            if let Some(tab) = tab {
                request = request.query(&[("tab", tab), ("templateVersion", "3")]);
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
            decode(
                &read_bounded_response_with_limit(response, "Migu artist directory", MAX_RESPONSE)
                    .await?,
            )
        }
        .await;
        self.log_upstream_request(
            "artist_directory",
            "app.c.nf.migu.cn",
            path,
            status,
            started,
            &result,
        );
        result
    }

    pub(crate) async fn artist_directory(&self, selection: Selection) -> Result<ArtistCatalog> {
        let tabs = self.artist_directory_response(TABS_PATH, None).await?;
        let filters = taxonomy(&tabs, &selection)?;
        let tab = format!("{}-{}", selection.group, selection.subtype);
        let data = self
            .artist_directory_response(LIST_PATH, Some(&tab))
            .await?;
        catalog(data, selection, filters)
    }
}

#[cfg(test)]
pub(crate) mod tests;
