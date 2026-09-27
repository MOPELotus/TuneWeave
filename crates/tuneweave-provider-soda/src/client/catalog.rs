use tuneweave_core::SearchItem;

use super::artist::{SodaArtistMetadata, map_artist_metadata};

use super::*;

#[derive(Clone, Copy)]
pub(crate) enum SodaCatalogKind {
    Album,
    Playlist,
    Artist,
}

impl SodaCatalogKind {
    fn path(self) -> &'static str {
        match self {
            Self::Album => "/luna/search/album",
            Self::Playlist => "/luna/search/playlist",
            Self::Artist => "/luna/search/artist",
        }
    }
    fn group(self) -> &'static str {
        match self {
            Self::Album => "albums",
            Self::Playlist => "playlists",
            Self::Artist => "artists",
        }
    }
    pub(crate) fn backend(self) -> &'static str {
        match self {
            Self::Album => "official_android_album_search",
            Self::Playlist => "official_android_playlist_search",
            Self::Artist => "official_android_artist_search",
        }
    }
}

pub(crate) struct SodaCatalogPage {
    pub items: Vec<SearchItem>,
    pub next_cursor: Option<u32>,
    pub has_more: bool,
}

#[derive(Deserialize)]
struct CatalogEnvelope {
    status_code: Option<i64>,
    status_info: SodaStatusInfo,
    result_groups: Vec<CatalogGroup>,
    #[serde(default)]
    extra: SodaSearchExtra,
}

#[derive(Deserialize)]
struct CatalogGroup {
    id: String,
    has_more: bool,
    #[serde(default)]
    next_cursor: FlexibleText,
    data: Vec<CatalogEntry>,
}

#[derive(Deserialize)]
struct CatalogEntry {
    entity: CatalogEntity,
}

#[derive(Deserialize)]
struct CatalogEntity {
    album: Option<CatalogAlbum>,
    playlist: Option<CatalogPlaylist>,
    artist: Option<SodaArtistMetadata>,
}

// Counts are optional in catalogue summaries. Keep absent counts unknown while
// reusing the detail mappers for identity and bounded metadata normalization.
#[derive(Deserialize)]
struct CatalogAlbum {
    count_tracks: Option<u64>,
    #[serde(flatten)]
    metadata: SodaAlbumMetadata,
}

#[derive(Deserialize)]
struct CatalogPlaylist {
    count_tracks: Option<u64>,
    #[serde(flatten)]
    metadata: SodaPlaylistMetadata,
}

impl SodaClient {
    pub(crate) async fn search_catalog_page(
        &self,
        kind: SodaCatalogKind,
        query: &str,
        cursor: u32,
    ) -> Result<SodaCatalogPage> {
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let mut url = Url::parse("https://api.qishui.com")
                .map_err(|_| soda_upstream_error("Soda catalogue search endpoint is invalid"))?;
            url.set_path(kind.path());
            // This builder only selects the fixed transport. No login state or
            // Cookie is attached to public catalogue search requests.
            let response = self
                .send_login_request(self.login_request(reqwest::Method::GET, url).query(
                    &SodaSearchQuery {
                        q: query,
                        aid: SODA_APP_ID,
                        cursor,
                        count: UPSTREAM_SEARCH_PAGE_SIZE,
                        app_name: "luna",
                        device_platform: "android",
                        version_name: "19.8.0",
                        version_code: "100198030",
                    },
                ))
                .await?;
            status = Some(response.status());
            let bytes = read_bounded_response(response, "Soda catalogue search").await?;
            parse_catalog(&bytes, kind, cursor)
        }
        .await;
        self.log_upstream_request(
            "search",
            "api.qishui.com",
            kind.path(),
            status,
            started,
            &result,
        );
        result
    }
}

pub(super) fn parse_catalog(
    bytes: &[u8],
    kind: SodaCatalogKind,
    cursor: u32,
) -> Result<SodaCatalogPage> {
    let envelope: CatalogEnvelope = serde_json::from_slice(bytes)
        .map_err(|_| soda_upstream_error("Soda catalogue search returned invalid data"))?;
    if envelope.status_code.is_some_and(|code| code != 0) {
        return Err(soda_upstream_error("Soda catalogue search was rejected"));
    }
    validate_status_metadata(&envelope.status_info, "Soda catalogue search")?;
    if envelope.result_groups.len() > 32 {
        return Err(soda_upstream_error(
            "Soda catalogue search returned too many groups",
        ));
    }
    let mut groups = envelope
        .result_groups
        .into_iter()
        .filter(|group| group.id == kind.group());
    let Some(group) = groups.next() else {
        if envelope.extra.empty_search == Some(1) {
            return Ok(SodaCatalogPage {
                items: Vec::new(),
                next_cursor: None,
                has_more: false,
            });
        }
        return Err(soda_upstream_error(
            "Soda catalogue search omitted its requested result group",
        ));
    };
    if groups.next().is_some() {
        return Err(soda_upstream_error(
            "Soda catalogue search repeated its result group",
        ));
    }
    if group.data.len() > UPSTREAM_SEARCH_PAGE_SIZE as usize
        || (group.has_more && group.data.is_empty())
    {
        return Err(soda_upstream_error(
            "Soda catalogue search returned an invalid physical page",
        ));
    }
    let next_cursor = if group.has_more {
        Some(
            group
                .next_cursor
                .to_u32()
                .filter(|next| *next > cursor)
                .ok_or_else(|| {
                    soda_upstream_error("Soda catalogue search omitted an advancing cursor")
                })?,
        )
    } else {
        None
    };
    let items = group
        .data
        .into_iter()
        .map(|entry| {
            match (
                kind,
                entry.entity.album,
                entry.entity.playlist,
                entry.entity.artist,
            ) {
                (SodaCatalogKind::Album, Some(mut source), None, None) => {
                    source.metadata.count_tracks = source.count_tracks.unwrap_or_default();
                    if source.metadata.has_error {
                        return Err(soda_upstream_error(
                            "Soda catalogue search returned an unavailable album",
                        ));
                    }
                    let mut album = map_album_metadata(
                        &source.metadata,
                        &source.metadata.id,
                        source.metadata.count_tracks,
                    )?;
                    album.track_count = source.count_tracks;
                    album
                        .extensions
                        .insert("backend".to_owned(), json!(kind.backend()));
                    Ok(SearchItem::Album(album))
                }
                (SodaCatalogKind::Playlist, None, Some(mut source), None) => {
                    source.metadata.count_tracks = source.count_tracks.unwrap_or_default();
                    let id = source.metadata.id.clone();
                    let (mut playlist, _, _, _) = map_playlist(source.metadata, &id)?;
                    playlist.track_count = source.count_tracks;
                    playlist
                        .extensions
                        .insert("backend".to_owned(), json!(kind.backend()));
                    Ok(SearchItem::Playlist(playlist))
                }
                (SodaCatalogKind::Artist, None, None, Some(source)) => {
                    map_artist_metadata(source, kind.backend()).map(SearchItem::Artist)
                }
                _ => Err(soda_upstream_error(
                    "Soda catalogue search item has a missing or conflicting resource type",
                )),
            }
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(SodaCatalogPage {
        items,
        next_cursor,
        has_more: group.has_more,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn envelope(kind: SodaCatalogKind, payload: serde_json::Value) -> serde_json::Value {
        let entity = match kind {
            SodaCatalogKind::Album => json!({"album":payload}),
            SodaCatalogKind::Playlist => json!({"playlist":payload}),
            SodaCatalogKind::Artist => json!({"artist":payload}),
        };
        json!({"status_info":{"log_id":"fixture","now":1,"now_ts_ms":1000},"result_groups":[{"id":kind.group(),"has_more":false,"data":[{"entity":entity}]}]})
    }

    fn parse(value: &serde_json::Value, kind: SodaCatalogKind) -> Result<SodaCatalogPage> {
        parse_catalog(&serde_json::to_vec(value).unwrap(), kind, 20)
    }

    #[test]
    fn album_catalogue_maps_typed_metadata_and_preserves_missing_counts() {
        let mut value = envelope(
            SodaCatalogKind::Album,
            json!({"id":"123","name":"album","count_tracks":12,"artists":[{"id":"456","name":"artist"}],"release_date":1609459200,"company":"label","url_cover":{"uri":"cover","urls":["https://p3-luna.douyinpic.com/img/"]}}),
        );
        let page = parse(&value, SodaCatalogKind::Album).unwrap();
        let SearchItem::Album(album) = &page.items[0] else {
            panic!("expected album");
        };
        assert_eq!(album.resource_ref.to_string(), "soda:123");
        assert_eq!(
            album.artists[0].resource_ref.as_ref().unwrap().to_string(),
            "soda:456"
        );
        assert_eq!(album.track_count, Some(12));
        assert_eq!(album.published_at.as_deref(), Some("2021-01-01T00:00:00Z"));
        assert_eq!(album.company.as_deref(), Some("label"));
        assert_eq!(
            album.cover_url.as_deref(),
            Some("https://p3-luna.douyinpic.com/img/cover")
        );
        assert_eq!(album.extensions["backend"], "official_android_album_search");
        assert!(!page.has_more);
        value["result_groups"][0]["data"][0]["entity"]["album"]
            .as_object_mut()
            .unwrap()
            .remove("count_tracks");
        let page = parse(&value, SodaCatalogKind::Album).unwrap();
        let SearchItem::Album(album) = &page.items[0] else {
            panic!("expected album");
        };
        assert!(album.track_count.is_none());
    }

    #[test]
    fn playlist_catalogue_keeps_its_actual_owner_without_claiming_subscription() {
        let value = envelope(
            SodaCatalogKind::Playlist,
            json!({"id":"123","title":"playlist","owner":{"id":"456","public_name":"creator","secret":"not-forwarded"},"count_tracks":25,"type":7,"url_cover":{"uri":"cover","urls":["https://p3-luna.douyinpic.com/img/"]}}),
        );
        let page = parse(&value, SodaCatalogKind::Playlist).unwrap();
        let SearchItem::Playlist(playlist) = &page.items[0] else {
            panic!("expected playlist");
        };
        assert_eq!(playlist.resource_ref.to_string(), "soda:123");
        assert_eq!(playlist.creator.as_ref().unwrap().name, "creator");
        assert_eq!(playlist.extensions["owner_id"], "456");
        assert_eq!(playlist.track_count, Some(25));
        assert_eq!(
            playlist.extensions["backend"],
            "official_android_playlist_search"
        );
        assert!(playlist.subscribed.is_none());
        assert!(
            !serde_json::to_string(playlist)
                .unwrap()
                .contains("not-forwarded")
        );
        let page = parse(
            &envelope(
                SodaCatalogKind::Playlist,
                json!({"id":"123","title":"unknown"}),
            ),
            SodaCatalogKind::Playlist,
        )
        .unwrap();
        let SearchItem::Playlist(playlist) = &page.items[0] else {
            panic!("expected playlist");
        };
        assert!(playlist.track_count.is_none());
        assert!(playlist.creator.is_none());
    }

    #[test]
    fn catalogue_selects_the_requested_group_and_rejects_ambiguous_or_malformed_pages() {
        for kind in [
            SodaCatalogKind::Album,
            SodaCatalogKind::Playlist,
            SodaCatalogKind::Artist,
        ] {
            let mut valid = envelope(kind, json!({"id":"123","name":"album","title":"playlist"}));
            valid["result_groups"].as_array_mut().unwrap().insert(
                0,
                json!({"id":"other","has_more":false,"data":[{"entity":{}}]}),
            );
            assert_eq!(parse(&valid, kind).unwrap().items.len(), 1);
            let mut duplicate = valid.clone();
            let group = duplicate["result_groups"][1].clone();
            duplicate["result_groups"]
                .as_array_mut()
                .unwrap()
                .push(group);
            assert!(parse(&duplicate, kind).is_err());
            for replacement in [
                json!({}),
                json!({"album":{"id":"123","name":"album"},"playlist":{"id":"123","title":"playlist"}}),
            ] {
                let mut bad = valid.clone();
                bad["result_groups"][1]["data"][0]["entity"] = replacement;
                assert!(parse(&bad, kind).is_err());
            }
            let mut rejected = valid.clone();
            rejected["status_code"] = json!(9);
            assert!(parse(&rejected, kind).is_err());
            for next in [
                json!(null),
                json!("20"),
                json!("-1"),
                json!("4294967296"),
                json!("021"),
                json!({}),
            ] {
                let mut bad = valid.clone();
                bad["result_groups"][1]["has_more"] = json!(true);
                bad["result_groups"][1]["next_cursor"] = next;
                assert!(parse(&bad, kind).is_err());
            }
            let mut advancing = valid.clone();
            advancing["result_groups"][1]["has_more"] = json!(true);
            advancing["result_groups"][1]["next_cursor"] = json!(40);
            assert_eq!(parse(&advancing, kind).unwrap().next_cursor, Some(40));
            let entry = advancing["result_groups"][1]["data"][0].clone();
            advancing["result_groups"][1]["data"] = json!([]);
            assert!(parse(&advancing, kind).is_err());
            advancing["result_groups"][1]["data"] = json!(vec![entry; 21]);
            assert!(parse(&advancing, kind).is_err());
            let empty = json!({"status_info":{},"result_groups":[],"extra":{"empty_search":1}});
            assert!(parse(&empty, kind).unwrap().items.is_empty());
            assert!(parse(&json!({"status_info":{},"result_groups":[]}), kind).is_err());
            assert!(parse(&json!({}), kind).is_err());
            for payload in [
                json!({"id":"0123","name":"bad","title":"bad"}),
                json!({"id":"123","name":"","title":""}),
                json!({"id":"123","name":"album","title":"playlist","count_tracks":-1}),
            ] {
                assert!(parse(&envelope(kind, payload), kind).is_err());
            }
        }
    }
}
