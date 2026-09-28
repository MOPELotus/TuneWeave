use super::*;
use crate::client::unix_rfc3339;

#[derive(Deserialize)]
struct AlbumEnvelope {
    mixed_collections: Option<Vec<AlbumCollection>>,
    total_num: Option<u64>,
    has_more: Option<bool>,
    next_cursor: Option<String>,
}
#[derive(Deserialize)]
struct AlbumCollection {
    item_type: String,
    album: Option<LibraryAlbum>,
}
#[derive(Deserialize)]
struct LibraryAlbum {
    id: String,
    name: String,
    #[serde(default)]
    artists: Vec<LibraryArtist>,
    #[serde(default)]
    url_cover: SodaImage,
    count_tracks: Option<u64>,
    release_date: Option<u64>,
    intro: Option<String>,
    company: Option<String>,
    state: Option<AlbumState>,
}
#[derive(Deserialize)]
struct LibraryArtist {
    id: Option<String>,
    name: String,
}
#[derive(Deserialize)]
struct AlbumState {
    is_collected: Option<bool>,
}

impl SodaClient {
    pub(crate) async fn album_library_page(
        &self,
        cursor: &str,
        credential: &SodaCredential,
    ) -> Result<LibraryPage<Album>> {
        self.read_library_page(
            LibrarySection::Saved,
            cursor,
            credential,
            "account_albums",
            |_, owner, credential, body| parse_page(owner, credential, body),
        )
        .await
    }

    pub(crate) async fn write_album_collection(
        &self,
        id: &str,
        subscribed: bool,
        credential: &SodaCredential,
    ) -> Result<SodaCredential> {
        let path = if subscribed {
            "/luna/pc/me/collection/album"
        } else {
            "/luna/pc/me/collection/album/delete"
        };
        self.write_collection(
            path,
            "album_collection_write",
            json!({"album_ids":[id]}),
            credential,
        )
        .await
    }
}

fn parse_page(owner: &str, credential: &SodaCredential, body: &[u8]) -> Result<LibraryPage<Album>> {
    validate_library_status(body)?;
    // The official mixed-collection endpoint returns only `status_info` for an
    // empty account library. Accept the same strictly validated empty envelope
    // used by the saved-playlist reader instead of treating it as malformed.
    if let Some(page) = super::empty_saved_library_page(credential, body) {
        return Ok(page);
    }
    let envelope: AlbumEnvelope = serde_json::from_slice(body)
        .map_err(|_| soda_upstream_error("Soda saved albums returned malformed data"))?;
    let entries = envelope
        .mixed_collections
        .ok_or_else(|| soda_upstream_error("Soda saved albums omitted mixed collections"))?;
    let raw_count = entries.len();
    if raw_count > LIBRARY_PAGE_SIZE
        || envelope
            .total_num
            .is_some_and(|count| count > MAX_LIBRARY_ITEMS as u64)
    {
        return Err(soda_upstream_error(
            "Soda saved albums exceeded the bounded collection size",
        ));
    }
    let mut items = Vec::new();
    let mut unclassified_items = 0;
    for entry in entries {
        if entry.item_type.is_empty()
            || entry.item_type.len() > 64
            || entry.item_type.chars().any(char::is_control)
        {
            return Err(soda_upstream_error("Soda saved collection type is invalid"));
        }
        match entry.item_type.as_str() {
            "album" => items.push(map_album(
                entry.album.ok_or_else(|| {
                    soda_upstream_error("Soda album collection omitted its album")
                })?,
                owner,
            )?),
            "playlist" => (),
            _ => unclassified_items += 1,
        }
    }
    Ok(LibraryPage {
        items,
        credential: credential.clone(),
        raw_count,
        total: envelope.total_num,
        next_cursor: envelope.next_cursor,
        has_more: envelope.has_more,
        unclassified_items,
    })
}

fn map_album(album: LibraryAlbum, owner: &str) -> Result<Album> {
    if !valid_id(&album.id)
        || album.name.trim().is_empty()
        || !valid_text(&album.name, 1000)
        || album.artists.len() > 128
        || album.count_tracks.is_some_and(|count| count > 1_000_000)
        || album.release_date.is_some_and(|date| date > 4_102_444_800)
        || album
            .intro
            .as_deref()
            .is_some_and(|text| !valid_text(text, 64 * 1024))
        || album
            .company
            .as_deref()
            .is_some_and(|text| !valid_text(text, 4000))
        || album.artists.iter().any(|artist| {
            artist.name.trim().is_empty()
                || !valid_text(&artist.name, 1000)
                || artist.id.as_deref().is_some_and(|id| !valid_id(id))
        })
    {
        return Err(soda_upstream_error("Soda saved album metadata is invalid"));
    }
    if album.state.and_then(|state| state.is_collected) == Some(false) {
        return Err(soda_upstream_error(
            "Soda saved album contradicts its collection state",
        ));
    }
    let artists = album
        .artists
        .into_iter()
        .map(|artist| {
            Ok(ArtistSummary {
                resource_ref: artist
                    .id
                    .map(|id| ResourceRef::new(Platform::Soda, id))
                    .transpose()
                    .map_err(|_| soda_upstream_error("Soda saved album artist ID is invalid"))?,
                name: artist.name.trim().to_owned(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Album {
        resource_ref: ResourceRef::new(Platform::Soda, &album.id)
            .map_err(|_| soda_upstream_error("Soda saved album ID is invalid"))?,
        platform: Platform::Soda,
        id: album.id,
        name: album.name.trim().to_owned(),
        aliases: Vec::new(),
        artists,
        description: album.intro.unwrap_or_default().trim().to_owned(),
        cover_url: normalize_image(&album.url_cover),
        published_at: album
            .release_date
            .filter(|date| *date > 0)
            .and_then(unix_rfc3339),
        track_count: album.count_tracks,
        company: album
            .company
            .filter(|value| !value.trim().is_empty())
            .map(|value| value.trim().to_owned()),
        kind: None,
        extensions: Extensions::from([
            (
                "backend".to_owned(),
                json!("official_pc_mixed_album_collection"),
            ),
            ("library_section".to_owned(), json!("saved")),
            ("source_user_id".to_owned(), json!(owner)),
            ("subscribed".to_owned(), json!(true)),
        ]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn page(body: serde_json::Value) -> Result<LibraryPage<Album>> {
        parse_page(
            "123456",
            &SodaCredential::test_credential("test"),
            &serde_json::to_vec(&body).unwrap(),
        )
    }

    #[test]
    fn saved_album_mapping_preserves_unknown_counts_and_counts_other_collection_kinds() {
        let result = page(json!({"mixed_collections":[
            {"item_type":"playlist","playlist":{"id":"11"}},
            {"item_type":"album","album":{"id":"11","name":" Unknown ","artists":[{"id":"31","name":" Artist "}],"intro":"Intro","release_date":1700000000}},
            {"item_type":"album","album":{"id":"22","name":"Empty","count_tracks":0,"state":{"is_collected":true}}}
        ],"total_num":3,"has_more":false})).unwrap();
        assert_eq!(result.raw_count, 3);
        assert_eq!(result.items.len(), 2);
        assert_eq!(result.items[0].id, "11");
        assert_eq!(result.items[0].name, "Unknown");
        assert_eq!(result.items[0].track_count, None);
        assert_eq!(
            result.items[0].artists[0]
                .resource_ref
                .as_ref()
                .unwrap()
                .to_string(),
            "soda:31"
        );
        assert_eq!(result.items[0].description, "Intro");
        assert!(result.items[0].published_at.is_some());
        assert_eq!(result.items[1].track_count, Some(0));
        assert_eq!(result.items[1].extensions["subscribed"], true);
        assert!(result.items[0].cover_url.is_none());
        assert!(result.items[0].kind.is_none());
        let mut pagination = LibraryPagination::default();
        assert!(pagination.accept("0", &result).unwrap().is_none());
        assert!(pagination.absence_is_proven());
    }

    #[test]
    fn empty_saved_album_envelope_proves_a_complete_empty_collection() {
        let result = page(json!({"status_info": {
            "log_id": "request-log",
            "now": 123,
            "now_ts_ms": 123456
        }}))
        .unwrap();
        assert!(result.items.is_empty());
        let mut pagination = LibraryPagination::default();
        assert!(pagination.accept("0", &result).unwrap().is_none());
        assert!(pagination.absence_is_proven());
    }

    #[test]
    fn saved_album_pages_reject_malformed_entries_and_prioritize_authentication_failure() {
        for album in [
            json!(null),
            json!({"id":"01","name":"bad"}),
            json!({"id":"11","name":""}),
            json!({"id":"11","name":"bad","count_tracks":-1}),
            json!({"id":"11","name":"bad","state":{"is_collected":false}}),
            json!({"id":"11","name":"bad","artists":[{"id":"0","name":"artist"}]}),
        ] {
            assert!(
                page(
                    json!({"mixed_collections":[{"item_type":"album","album":album}],"total_num":1})
                )
                .is_err()
            );
        }
        assert_eq!(
            page(json!({"status_code":1000016,"mixed_collections":"bad"}))
                .err()
                .unwrap()
                .code,
            ErrorCode::AuthenticationRequired
        );
        let repeated=page(json!({"mixed_collections":[{"item_type":"album","album":{"id":"11","name":"first"}},{"item_type":"album","album":{"id":"11","name":"again"}}],"total_num":2})).unwrap();
        assert!(LibraryPagination::default().accept("0", &repeated).is_err());
    }

    #[test]
    fn saved_album_removal_needs_counted_completion_without_unknown_item_types() {
        for body in [
            json!({"mixed_collections":[],"has_more":false}),
            json!({"mixed_collections":[{"item_type":"future_album_kind"}],"has_more":false,"total_num":1}),
        ] {
            let mut pagination = LibraryPagination::default();
            assert!(
                pagination
                    .accept("0", &page(body).unwrap())
                    .unwrap()
                    .is_none()
            );
            assert!(!pagination.absence_is_proven());
        }
        let mut pagination = LibraryPagination::default();
        assert!(
            pagination
                .accept(
                    "0",
                    &page(json!({"mixed_collections":[],"total_num":0})).unwrap()
                )
                .unwrap()
                .is_none()
        );
        assert!(pagination.absence_is_proven());
    }
}
