use super::albums::{
    AlbumMetadata, DigitalAlbumMetadata, map_album_metadata, map_digital_album_metadata,
};
use super::*;
use tuneweave_core::{Artist, SearchItem};

pub(crate) const PAGE_SIZE: usize = 20;

#[derive(Clone, Copy, Debug)]
pub(crate) enum CatalogKind {
    Playlist,
    Artist,
    Album,
}

impl CatalogKind {
    fn path(self) -> &'static str {
        match self {
            Self::Playlist => "/bmw/search/music-list/v1.0",
            Self::Artist => "/bmw/search/singer/v2.0",
            Self::Album => "/bmw/search/album/v1.0",
        }
    }
    pub(crate) fn backend(self) -> &'static str {
        match self {
            Self::Playlist => "bmw_music_list_search_v1",
            Self::Artist => "bmw_singer_search_v2",
            Self::Album => "bmw_album_search_v1",
        }
    }
}

pub(crate) struct CatalogPage {
    pub items: Vec<SearchItem>,
    pub has_next: bool,
    pub sequence: Option<String>,
    pub conditions: Vec<MiguSearchCondition>,
}

#[derive(Deserialize)]
struct CatalogEnvelope {
    code: String,
    data: Option<CatalogData>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogData {
    has_next: bool,
    #[serde(default)]
    items: Vec<CatalogEntry>,
    #[serde(default)]
    conditions: Vec<MiguSearchCondition>,
    #[serde(default)]
    seq: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogEntry {
    music_list: Option<CatalogPlaylist>,
    singer: Option<CatalogArtist>,
    album: Option<AlbumMetadata>,
    dalbum: Option<DigitalAlbumMetadata>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogPlaylist {
    resource_type: String,
    music_list_id: String,
    title: String,
    music_num: Option<u64>,
    owner_id: Option<String>,
    owner_name: Option<String>,
    img_item: Option<CatalogImage>,
    original_img_url: Option<String>,
    publish_time: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogArtist {
    resource_type: String,
    singer_id: String,
    singer: String,
    summary: Option<String>,
    #[serde(default)]
    imgs: Vec<CatalogImage>,
    song_num: Option<u64>,
    album_num: Option<u64>,
    mv_num: Option<u64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CatalogImage {
    #[serde(default)]
    img: String,
}

impl MiguClient {
    #[cfg(test)]
    pub(crate) fn with_catalog_test_origin(mut self, origin: Url) -> Self {
        assert_eq!(origin.scheme(), "http");
        assert_eq!(origin.host_str(), Some("127.0.0.1"));
        self.catalog_test_origin = Some(origin);
        self
    }

    pub(crate) async fn search_catalog_page(
        &self,
        kind: CatalogKind,
        keyword: &str,
        page: u32,
    ) -> Result<CatalogPage> {
        let started = Instant::now();
        let mut http_status = None;
        let outcome =
            async {
                let url = self.catalog_url(kind)?;
                let mut request = self.http.get(url).header(ACCEPT, "application/json").query(
                    &MiguSearchRequest {
                        page_no: page,
                        text: keyword,
                    },
                );
                if matches!(kind, CatalogKind::Playlist | CatalogKind::Album) {
                    request = request.query(&[("typeOrder", "0")]);
                }
                let response = request.send().await.map_err(migu_network_error)?;
                http_status = Some(response.status());
                if response.status().is_success()
                    && response
                        .headers()
                        .get(reqwest::header::CONTENT_TYPE)
                        .is_some_and(|value| {
                            value.to_str().map_or(true, |value| {
                                !value
                                    .split(';')
                                    .next()
                                    .unwrap_or_default()
                                    .trim()
                                    .eq_ignore_ascii_case("application/json")
                            })
                        })
                {
                    return Err(migu_upstream_error(
                        "Migu catalogue returned an unexpected content type",
                    ));
                }
                let bytes = read_bounded_response(response, "Migu catalogue search").await?;
                parse_catalog_response(kind, &bytes)
            }
            .await;
        self.log_upstream_request(
            "search_catalog",
            "app.c.nf.migu.cn",
            kind.path(),
            http_status,
            started,
            &outcome,
        );
        outcome
    }

    fn catalog_url(&self, kind: CatalogKind) -> Result<Url> {
        #[cfg(test)]
        if let Some(origin) = &self.catalog_test_origin {
            return origin
                .join(kind.path())
                .map_err(|_| migu_upstream_error("Invalid test catalogue endpoint"));
        }
        Url::parse(&format!("https://app.c.nf.migu.cn{}", kind.path()))
            .map_err(|_| migu_upstream_error("Migu catalogue endpoint is invalid"))
    }
}

fn parse_catalog_response(kind: CatalogKind, bytes: &[u8]) -> Result<CatalogPage> {
    let envelope: CatalogEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| migu_upstream_error("Migu catalogue returned malformed data"))?;
    if envelope.code != "000000" {
        return Err(migu_upstream_error("Migu catalogue rejected the request")
            .with_details(json!({"platform_code":bounded_text(&envelope.code,64)})));
    }
    let data = envelope
        .data
        .ok_or_else(|| migu_upstream_error("Migu catalogue omitted data"))?;
    if data.items.len() > PAGE_SIZE || (data.has_next && data.items.len() != PAGE_SIZE) {
        return Err(migu_upstream_error(
            "Migu catalogue page size contradicts its continuation",
        ));
    }
    let items = data
        .items
        .into_iter()
        .map(
            |item| match (kind, item.music_list, item.singer, item.album, item.dalbum) {
                (CatalogKind::Playlist, Some(playlist), None, None, None) => {
                    map_playlist(playlist).map(SearchItem::Playlist)
                }
                (CatalogKind::Artist, None, Some(artist), None, None) => {
                    map_artist(artist).map(SearchItem::Artist)
                }
                (CatalogKind::Album, None, None, Some(album), None) => {
                    map_album_metadata(album, kind.backend()).map(SearchItem::Album)
                }
                (CatalogKind::Album, None, None, None, Some(album)) => {
                    map_digital_album_metadata(album, kind.backend()).map(SearchItem::DigitalAlbum)
                }
                _ => Err(migu_upstream_error(
                    "Migu catalogue returned an incompatible result kind",
                )),
            },
        )
        .collect::<Result<Vec<_>>>()?;
    Ok(CatalogPage {
        items,
        has_next: data.has_next,
        sequence: nonempty(&data.seq).map(|value| bounded_text(value, 512)),
        conditions: bounded_conditions(data.conditions),
    })
}

fn catalog_id(value: &str) -> Result<&str> {
    if value.is_empty()
        || value.len() > 64
        || value.starts_with('0')
        || !value.bytes().all(|c| c.is_ascii_digit())
    {
        return Err(migu_upstream_error(
            "Migu catalogue returned an invalid resource ID",
        ));
    }
    Ok(value)
}

fn map_playlist(value: CatalogPlaylist) -> Result<Playlist> {
    if value.resource_type != "2021" {
        return Err(migu_upstream_error(
            "Migu playlist search returned the wrong resource type",
        ));
    }
    let id = catalog_id(&value.music_list_id)?;
    let name = validated_name(&value.title)
        .ok_or_else(|| migu_upstream_error("Migu playlist search omitted a valid title"))?;
    let mut extensions = Extensions::from([
        ("backend".to_owned(), json!(CatalogKind::Playlist.backend())),
        ("resource_type".to_owned(), json!(value.resource_type)),
    ]);
    if let Some(owner) = value.owner_id.as_deref().filter(|value| !value.is_empty()) {
        let owner = canonical_playlist_owner_id(owner).ok_or_else(|| {
            migu_upstream_error("Migu playlist search returned an invalid owner ID")
        })?;
        extensions.insert("owner_id".to_owned(), json!(owner));
    }
    if let Some(time) = value
        .publish_time
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        if time.len() != 14 || !time.bytes().all(|c| c.is_ascii_digit()) {
            return Err(migu_upstream_error(
                "Migu playlist publication time is invalid",
            ));
        }
        // The platform omits the timezone; retain its raw value without inventing UTC.
        extensions.insert("publish_time".to_owned(), json!(time));
    }
    let creator = value
        .owner_name
        .as_deref()
        .filter(|name| !name.is_empty())
        .map(|name| {
            let name = validated_name(name)
                .ok_or_else(|| migu_upstream_error("Migu playlist owner name is invalid"))?;
            Ok(ArtistSummary {
                resource_ref: None,
                name: name.to_owned(),
            })
        })
        .transpose()?;
    Ok(Playlist {
        resource_ref: ResourceRef::new(Platform::Migu, id)
            .map_err(|_| migu_upstream_error("Migu playlist identity is invalid"))?,
        platform: Platform::Migu,
        id: id.to_owned(),
        name: name.to_owned(),
        description: String::new(),
        cover_url: value
            .original_img_url
            .as_deref()
            .and_then(normalize_media_url)
            .or_else(|| {
                value
                    .img_item
                    .as_ref()
                    .and_then(|image| normalize_media_url(&image.img))
            }),
        creator,
        track_count: value.music_num,
        tags: Vec::new(),
        subscribed: None,
        created_at: None,
        updated_at: None,
        extensions,
    })
}

fn map_artist(value: CatalogArtist) -> Result<Artist> {
    if value.resource_type != "2002" {
        return Err(migu_upstream_error(
            "Migu artist search returned the wrong resource type",
        ));
    }
    let id = catalog_id(&value.singer_id)?;
    let name = validated_name(&value.singer)
        .ok_or_else(|| migu_upstream_error("Migu artist search omitted a valid name"))?;
    let summary = value.summary.unwrap_or_default();
    if summary.len() > 256 * 1024
        || summary
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t'))
        || value.imgs.len() > 64
    {
        return Err(migu_upstream_error(
            "Migu artist search metadata exceeded its bounds",
        ));
    }
    Ok(Artist {
        resource_ref: ResourceRef::new(Platform::Migu, id)
            .map_err(|_| migu_upstream_error("Migu artist identity is invalid"))?,
        platform: Platform::Migu,
        id: id.to_owned(),
        name: name.to_owned(),
        aliases: Vec::new(),
        description: summary.trim().to_owned(),
        biography_sections: Vec::new(),
        avatar_url: value
            .imgs
            .iter()
            .find_map(|image| normalize_media_url(&image.img)),
        cover_url: None,
        album_count: value.album_num,
        track_count: value.song_num,
        mv_count: value.mv_num,
        video_count: None,
        identities: Vec::new(),
        extensions: Extensions::from([
            ("backend".to_owned(), json!(CatalogKind::Artist.backend())),
            ("resource_type".to_owned(), json!(value.resource_type)),
        ]),
    })
}

#[cfg(test)]
mod tests;
