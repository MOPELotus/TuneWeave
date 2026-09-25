//! Public signed catalogue search. Account credentials never enter this transport.
use super::dto::{Number, optional_text, required_text, resource};
use super::*;
use serde::de::{DeserializeOwned, IgnoredAny};
use tuneweave_core::{Album, Artist, SearchItem};

pub(crate) const PAGE_SIZE: u32 = 20;
const RESPONSE_LIMIT: usize = 1_048_576;

#[derive(Clone, Copy, Debug)]
pub(crate) enum CatalogKind {
    Album,
    Artist,
    Playlist,
    Mv,
}
impl CatalogKind {
    pub(crate) fn path(self) -> &'static str {
        match self {
            Self::Album => "/v1/search/album",
            Self::Artist => "/v1/search/author",
            Self::Playlist => "/v1/search/special",
            Self::Mv => "/v1/search/mv",
        }
    }
    pub(crate) fn backend(self) -> &'static str {
        match self {
            Self::Album => "complexsearch_album_v1",
            Self::Artist => "complexsearch_author_v1",
            Self::Playlist => "complexsearch_special_v1",
            Self::Mv => "complexsearch_mv_v1",
        }
    }
}

pub(crate) struct CatalogPage {
    pub items: Vec<SearchItem>,
    pub total: u64,
    pub extensions: Extensions,
}

impl KugouClient {
    pub(crate) async fn search_catalog_page(
        &self,
        kind: CatalogKind,
        keyword: &str,
        page: u32,
    ) -> Result<CatalogPage> {
        if page == 0
            || keyword.is_empty()
            || keyword.len() > 512
            || keyword.chars().any(char::is_control)
        {
            return Err(kugou_invalid_media_request(
                "Invalid KuGou catalogue search request",
            ));
        }
        let identity = self.device_identity()?;
        let time = unix_seconds_now().to_string();
        let mut query = BTreeMap::from([
            ("appid", ANDROID_APP_ID.to_string()),
            ("clientver", ANDROID_CLIENT_VERSION.to_string()),
            ("clienttime", time.clone()),
            ("dfid", identity.dfid().to_owned()),
            ("mid", identity.mid.clone()),
            ("uuid", identity.guid.clone()),
            ("userid", "0".to_owned()),
            ("token", String::new()),
            ("keyword", keyword.to_owned()),
            ("page", page.to_string()),
            ("pagesize", PAGE_SIZE.to_string()),
            ("platform", "AndroidFilter".to_owned()),
            ("iscorrection", "1".to_owned()),
        ]);
        query.insert("signature", crate::signing::android_signature(&query, b""));
        let url = format!("{ANDROID_GATEWAY}{}", kind.path());
        #[cfg(test)]
        let url = self
            .login_test_origin
            .as_ref()
            .map(|origin| origin.join(kind.path()).unwrap().to_string())
            .unwrap_or(url);
        let started = Instant::now();
        let mut status = None;
        let outcome = async {
            let mut response = self
                .http
                .get(url)
                .header("x-router", "complexsearch.kugou.com")
                .header("user-agent", ANDROID_USER_AGENT)
                .header("dfid", identity.dfid())
                .header("mid", &identity.mid)
                .header("clienttime", &time)
                .query(&query)
                .send()
                .await
                .map_err(kugou_network_error)?;
            status = Some(response.status());
            if !response.status().is_success() {
                return Err(kugou_http_error(response.status()));
            }
            if response.headers().contains_key("ssa-code") {
                return Err(TuneWeaveError::new(
                    ErrorCode::PermissionDenied,
                    "KuGou catalogue search requires additional verification",
                )
                .with_platform(Platform::Kugou));
            }
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .map(str::trim);
            if !matches!(content_type, Some("application/json" | "text/plain"))
                || response
                    .content_length()
                    .is_some_and(|v| v > RESPONSE_LIMIT as u64)
            {
                return Err(malformed());
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.map_err(kugou_network_error)? {
                if bytes.len().saturating_add(chunk.len()) > RESPONSE_LIMIT {
                    return Err(malformed());
                }
                bytes.extend_from_slice(&chunk);
            }
            let mut page = parse(kind, &bytes, page)?;
            if matches!(kind, CatalogKind::Mv) {
                self.enrich_mv_search(&mut page.items).await?;
            }
            Ok(page)
        }
        .await;
        self.log_upstream_request(
            kind.backend(),
            "gateway.kugou.com",
            kind.path(),
            status,
            started,
            0,
            false,
            &outcome,
        );
        outcome
    }
}

#[derive(Deserialize)]
struct Envelope<T> {
    status: i64,
    error_code: i64,
    data: Option<T>,
}
#[derive(Deserialize)]
struct Data<T> {
    page: u32,
    pagesize: u32,
    from: u64,
    size: u32,
    total: u64,
    lists: Vec<T>,
    correctiontype: Option<i64>,
    correctionforce: Option<i64>,
    correctiontip: Option<String>,
}

pub(super) fn parse(kind: CatalogKind, bytes: &[u8], page: u32) -> Result<CatalogPage> {
    let envelope: Envelope<IgnoredAny> = serde_json::from_slice(bytes).map_err(|_| malformed())?;
    if envelope.status != 1 || envelope.error_code != 0 {
        if matches!(kind, CatalogKind::Mv) && envelope.status == 1 && envelope.error_code == 149 {
            return Err(kugou_invalid_media_request(
                "KuGou MV search page is outside the upstream range",
            )
            .with_details(json!({"platform_code":149})));
        }
        return Err(kugou_upstream_error("KuGou catalogue search was rejected")
            .with_details(json!({"platform_code":envelope.error_code})));
    }
    match kind {
        CatalogKind::Album => parse_items::<AlbumHit>(bytes, page, map_album),
        CatalogKind::Artist => parse_items::<ArtistHit>(bytes, page, map_artist),
        CatalogKind::Playlist => parse_items::<PlaylistHit>(bytes, page, map_playlist),
        CatalogKind::Mv => parse_items::<super::videos::MvHit>(bytes, page, super::videos::map_mv),
    }
}
fn parse_items<T: DeserializeOwned>(
    bytes: &[u8],
    page: u32,
    map: fn(T) -> Result<SearchItem>,
) -> Result<CatalogPage> {
    let data = serde_json::from_slice::<Envelope<Data<T>>>(bytes)
        .map_err(|_| malformed())?
        .data
        .ok_or_else(malformed)?;
    let from = u64::from(page.checked_sub(1).ok_or_else(malformed)?) * u64::from(PAGE_SIZE);
    if data.page != page
        || data.pagesize != PAGE_SIZE
        || data.size != PAGE_SIZE
        || data.from != from
        || data.lists.len() as u64 != data.total.saturating_sub(from).min(u64::from(PAGE_SIZE))
    {
        return Err(malformed());
    }
    let items = data
        .lists
        .into_iter()
        .map(map)
        .collect::<Result<Vec<_>>>()?;
    let mut ids = BTreeSet::new();
    if items.iter().any(|item| !ids.insert(item_id(item))) {
        return Err(malformed());
    }
    let mut extensions = Extensions::new();
    if let Some(v) = data.correctiontype {
        extensions.insert("correction_type".into(), json!(v));
    }
    if let Some(v) = data.correctionforce {
        extensions.insert("correction_force".into(), json!(v));
    }
    if let Some(v) = optional_text(data.correctiontip, 512)? {
        extensions.insert("correction_tip".into(), json!(v));
    }
    Ok(CatalogPage {
        items,
        total: data.total,
        extensions,
    })
}

pub(crate) fn item_id(item: &SearchItem) -> &str {
    match item {
        SearchItem::Album(v) => &v.id,
        SearchItem::Artist(v) => &v.id,
        SearchItem::Playlist(v) => &v.id,
        SearchItem::Video(v) => &v.id,
        _ => unreachable!("catalogue response has a supported concrete type"),
    }
}

#[derive(Deserialize)]
struct Singer {
    id: Number,
    name: String,
}
#[derive(Deserialize)]
struct AlbumHit {
    albumid: Number,
    albumname: String,
    singers: Option<Vec<Singer>>,
    singer: Option<String>,
    singerid: Option<Number>,
    intro: Option<String>,
    img: Option<String>,
    publish_time: Option<String>,
    songcount: Option<Number>,
    company: Option<String>,
    language: Option<String>,
}
fn map_album(hit: AlbumHit) -> Result<SearchItem> {
    let id = hit.albumid.id()?;
    let mut artists = vec![];
    if let Some(singers) = hit.singers.filter(|v| !v.is_empty()) {
        if singers.len() > 100 {
            return Err(malformed());
        }
        let mut seen = BTreeSet::new();
        for singer in singers {
            // Search may return several named singers with unknown (zero) IDs.
            // Keep each name in order without inventing or merging resource references.
            let resource_ref = if singer.id.0 == 0 {
                None
            } else {
                let artist_id = singer.id.id()?;
                if !seen.insert(artist_id.clone()) {
                    return Err(malformed());
                }
                Some(resource(artist_id)?)
            };
            artists.push(ArtistSummary {
                resource_ref,
                name: required_text(singer.name)?,
            });
        }
    } else if let Some(name) = optional_text(hit.singer, 1024)? {
        artists.push(ArtistSummary {
            resource_ref: hit
                .singerid
                .filter(|v| v.0 > 0)
                .map(|v| resource(v.0.to_string()))
                .transpose()?,
            name,
        });
    }
    let mut extensions = Extensions::new();
    if let Some(v) = optional_text(hit.language, 256)? {
        extensions.insert("language".into(), json!(v));
    }
    Ok(SearchItem::Album(Album {
        resource_ref: resource(id.clone())?,
        platform: Platform::Kugou,
        id,
        name: required_text(hit.albumname)?,
        aliases: vec![],
        artists,
        description: optional_text(hit.intro, 32768)?.unwrap_or_default(),
        cover_url: hit.img.as_deref().and_then(normalize_image_url),
        published_at: optional_text(hit.publish_time, 128)?,
        track_count: hit.songcount.map(|v| v.0),
        company: optional_text(hit.company, 1024)?,
        kind: None,
        extensions,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ArtistHit {
    author_id: Number,
    author_name: String,
    avatar: Option<String>,
    album_count: Option<Number>,
    audio_count: Option<Number>,
    video_count: Option<Number>,
    identity: Option<Number>,
    fans_num: Option<Number>,
}
fn map_artist(hit: ArtistHit) -> Result<SearchItem> {
    let id = hit.author_id.id()?;
    let mut extensions = Extensions::new();
    if let Some(v) = hit.identity {
        extensions.insert("identity_code".into(), json!(v.0));
    }
    if let Some(v) = hit.fans_num {
        extensions.insert("fans_count".into(), json!(v.0));
    }
    Ok(SearchItem::Artist(Artist {
        resource_ref: resource(id.clone())?,
        platform: Platform::Kugou,
        id,
        name: required_text(hit.author_name)?,
        aliases: vec![],
        description: String::new(),
        biography_sections: vec![],
        avatar_url: hit.avatar.as_deref().and_then(normalize_image_url),
        cover_url: None,
        album_count: hit.album_count.map(|v| v.0),
        track_count: hit.audio_count.map(|v| v.0),
        mv_count: None,
        video_count: hit.video_count.map(|v| v.0),
        identities: vec![],
        extensions,
    }))
}

#[derive(Deserialize)]
struct PlaylistHit {
    gid: String,
    specialid: Option<Number>,
    specialname: String,
    intro: Option<String>,
    img: Option<String>,
    song_count: Option<Number>,
    nickname: Option<String>,
    suid: Option<Number>,
    tag_str: Option<String>,
}
fn map_playlist(hit: PlaylistHit) -> Result<SearchItem> {
    validate_collection_id(&hit.gid).map_err(|_| malformed())?;
    let creator = optional_text(hit.nickname, 1024)?.map(|name| ArtistSummary {
        resource_ref: None,
        name,
    });
    let mut extensions = Extensions::new();
    if let Some(owner) = hit.suid.filter(|v| v.0 > 0) {
        extensions.insert("owner_id".into(), json!(owner.0.to_string()));
    }
    if let Some(v) = hit.specialid {
        extensions.insert("special_id".into(), json!(v.0));
    }
    // The upstream tag string has no proven delimiter contract; keep it intact.
    if let Some(v) = optional_text(hit.tag_str, 4096)? {
        extensions.insert("tag_text".into(), json!(v));
    }
    Ok(SearchItem::Playlist(Playlist {
        resource_ref: resource(hit.gid.clone())?,
        platform: Platform::Kugou,
        id: hit.gid,
        name: required_text(hit.specialname)?,
        description: optional_text(hit.intro, 32768)?.unwrap_or_default(),
        cover_url: hit.img.as_deref().and_then(normalize_image_url),
        creator,
        track_count: hit.song_count.map(|v| v.0),
        tags: vec![],
        subscribed: None,
        created_at: None,
        updated_at: None,
        extensions,
    }))
}

fn malformed() -> TuneWeaveError {
    kugou_upstream_error("KuGou catalogue response has invalid identity, pagination or fields")
}

#[cfg(test)]
mod tests;
