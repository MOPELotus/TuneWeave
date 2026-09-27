//! Account-bound artist collection calls from the official Android API.
use super::*;
use crate::login::SodaCredential;
use reqwest::header::{ACCEPT, COOKIE};
use tuneweave_core::Artist;

const ARTIST_COLLECTION_PATH: &str = "/luna/me/collection/artist";
const ARTIST_COLLECTION_DELETE_PATH: &str = "/luna/me/collection/artist/delete";
const LUNA_API_VERSION: &str = "2023-01-04";
const ARTIST_PAGE_SIZE: u32 = 100;
const MAX_PAGE_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct SodaArtistCollectionPage {
    pub(crate) artists: Vec<Artist>,
    pub(crate) artist_ids: Vec<String>,
    pub(crate) next_cursor: Option<String>,
    pub(crate) total_num: u64,
    pub(crate) has_more: bool,
    pub(crate) credential: SodaCredential,
}

#[derive(Serialize)]
struct ArtistIdsRequest<'a> {
    artist_ids: [&'a str; 1],
}

#[derive(Deserialize)]
struct WriteAck {
    status_code: Option<i64>,
    collect_artist_status: Option<std::collections::BTreeMap<String, i64>>,
    deleted_artist_status: Option<std::collections::BTreeMap<String, i64>>,
}

#[derive(Deserialize)]
struct CollectionPage {
    status_code: Option<i64>,
    artists: Option<Vec<NetArtistSummary>>,
    next_cursor: Option<String>,
    total_num: Option<u64>,
    has_more: Option<bool>,
}

#[derive(Deserialize)]
struct NetArtistSummary {
    id: String,
    name: String,
    count_albums: Option<u64>,
    count_tracks: Option<u64>,
}

impl SodaClient {
    pub(crate) async fn account_artist_collection_page(
        &self,
        cursor: Option<&str>,
        credential: &SodaCredential,
    ) -> Result<SodaArtistCollectionPage> {
        if credential.user_id().is_none() {
            return Err(artist_collection_authentication_required());
        }
        if cursor.is_some_and(|cursor| !valid_cursor(cursor)) {
            return Err(soda_invalid_request(
                "Soda artist collection cursor is invalid",
            ));
        }

        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            let mut url = Url::parse("https://api.qishui.com")
                .map_err(|_| soda_upstream_error("Soda artist collection endpoint is invalid"))?;
            url.set_path(ARTIST_COLLECTION_PATH);
            {
                let mut query = url.query_pairs_mut();
                if let Some(cursor) = cursor {
                    query.append_pair("cursor", cursor);
                }
                query.append_pair("count", &ARTIST_PAGE_SIZE.to_string());
            }
            let response = self
                .send_login_request(
                    self.login_request(reqwest::Method::GET, url)
                        .header(ACCEPT, "application/json")
                        .header("x-luna-api-version", LUNA_API_VERSION)
                        .header("x-luna-is-login", "1")
                        .header(COOKIE, credential.cookie_header()?),
                )
                .await?;
            http_status = Some(response.status());
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(artist_collection_authentication_required());
            }
            if !response.status().is_success() {
                return Err(soda_http_error(response.status()));
            }
            if response.headers().contains_key("bdturing-verify") {
                return Err(artist_collection_challenge());
            }
            require_json_content_type(response.headers(), "Soda artist collection")?;
            let headers = response.headers().clone();
            let bytes = read_bounded_response(response, "Soda artist collection").await?;
            let page = parse_collection_page(&bytes)?;
            let credential = credential.with_response_cookies(&headers)?;
            Ok(SodaArtistCollectionPage {
                artists: page.artists,
                artist_ids: page.artist_ids,
                next_cursor: page.next_cursor,
                total_num: page.total_num,
                has_more: page.has_more,
                credential,
            })
        }
        .await;
        self.log_upstream_request(
            "account_artist_collection_read",
            "api.qishui.com",
            ARTIST_COLLECTION_PATH,
            http_status,
            started,
            &result,
        );
        result
    }

    pub(crate) async fn write_account_artist_collection(
        &self,
        artist_id: &str,
        subscribed: bool,
        credential: &SodaCredential,
    ) -> Result<SodaCredential> {
        if canonical_positive_decimal(artist_id) != Some(artist_id) {
            return Err(soda_invalid_request(
                "Soda artist ID must be a canonical positive decimal",
            ));
        }
        if credential.user_id().is_none() {
            return Err(artist_collection_authentication_required());
        }

        let path = if subscribed {
            ARTIST_COLLECTION_PATH
        } else {
            ARTIST_COLLECTION_DELETE_PATH
        };
        let started = Instant::now();
        let mut http_status = None;
        let result = async {
            let url = Url::parse(&format!("https://api.qishui.com{path}"))
                .map_err(|_| soda_upstream_error("Soda artist collection endpoint is invalid"))?;
            let body = ArtistIdsRequest {
                artist_ids: [artist_id],
            };
            let response = self
                .send_login_request(
                    self.login_request(reqwest::Method::POST, url)
                        .header(ACCEPT, "application/json")
                        .header("x-luna-api-version", LUNA_API_VERSION)
                        .header("x-luna-is-login", "1")
                        .header(COOKIE, credential.cookie_header()?)
                        .json(&body),
                )
                .await?;
            http_status = Some(response.status());
            if response.status() == reqwest::StatusCode::UNAUTHORIZED {
                return Err(artist_collection_authentication_required());
            }
            if !response.status().is_success() {
                return Err(soda_http_error(response.status()));
            }
            if response.headers().contains_key("bdturing-verify") {
                return Err(artist_collection_challenge());
            }
            require_json_content_type(response.headers(), "Soda artist collection write")?;
            let headers = response.headers().clone();
            let bytes = read_bounded_response(response, "Soda artist collection write").await?;
            parse_write_ack(&bytes, artist_id, subscribed)?;
            credential.with_response_cookies(&headers)
        }
        .await;
        self.log_upstream_request(
            if subscribed {
                "account_artist_collection_add"
            } else {
                "account_artist_collection_delete"
            },
            "api.qishui.com",
            path,
            http_status,
            started,
            &result,
        );
        result
    }
}

struct ParsedCollectionPage {
    artists: Vec<Artist>,
    artist_ids: Vec<String>,
    next_cursor: Option<String>,
    total_num: u64,
    has_more: bool,
}

fn parse_collection_page(body: &[u8]) -> Result<ParsedCollectionPage> {
    if body.len() > MAX_PAGE_BYTES {
        return Err(artist_collection_invalid(
            "Soda artist collection page is too large",
        ));
    }
    let page: CollectionPage = serde_json::from_slice(body)
        .map_err(|_| artist_collection_invalid("Soda artist collection returned malformed data"))?;
    match page.status_code {
        Some(100_001 | 1_000_016) => return Err(artist_collection_authentication_required()),
        Some(0) => {}
        _ => {
            return Err(artist_collection_invalid(
                "Soda artist collection read was rejected",
            ));
        }
    }
    let items = page.artists.ok_or_else(|| {
        artist_collection_invalid("Soda artist collection omitted its page items")
    })?;
    let total_num = page.total_num.ok_or_else(|| {
        artist_collection_invalid("Soda artist collection omitted its complete total")
    })?;
    let has_more = page.has_more.ok_or_else(|| {
        artist_collection_invalid("Soda artist collection omitted its continuation state")
    })?;
    let next_cursor = page.next_cursor.filter(|cursor| !cursor.is_empty());
    if items.len() > ARTIST_PAGE_SIZE as usize
        || total_num < items.len() as u64
        || (!has_more && items.is_empty() && total_num > 0)
        || (has_more
            && next_cursor
                .as_deref()
                .is_none_or(|cursor| !valid_cursor(cursor)))
    {
        return Err(artist_collection_invalid(
            "Soda artist collection returned inconsistent page metadata",
        ));
    }
    let mut ids = std::collections::BTreeSet::new();
    let mut artists = Vec::with_capacity(items.len());
    for item in items {
        let artist = map_following_artist(item)?;
        if !ids.insert(artist.id.clone()) {
            return Err(artist_collection_invalid(
                "Soda artist collection contains a duplicate artist ID",
            ));
        }
        artists.push(artist);
    }
    let artist_ids = artists.iter().map(|artist| artist.id.clone()).collect();
    Ok(ParsedCollectionPage {
        artists,
        artist_ids,
        next_cursor,
        total_num,
        has_more,
    })
}

fn map_following_artist(source: NetArtistSummary) -> Result<Artist> {
    let id = source.id;
    let name = source.name;
    if id.len() > 64
        || canonical_positive_decimal(&id).is_none()
        || name.trim().is_empty()
        || name.len() > 1_000
        || name.chars().any(char::is_control)
        || source.count_albums.is_some_and(|count| count > 1_000_000)
        || source.count_tracks.is_some_and(|count| count > 1_000_000)
    {
        return Err(artist_collection_invalid(
            "Soda artist collection contains invalid artist metadata",
        ));
    }
    Ok(Artist {
        resource_ref: tuneweave_core::ResourceRef::new(Platform::Soda, &id)
            .map_err(|_| artist_collection_invalid("Soda artist identity is invalid"))?,
        platform: Platform::Soda,
        id,
        name,
        aliases: Vec::new(),
        description: String::new(),
        biography_sections: Vec::new(),
        avatar_url: None,
        cover_url: None,
        album_count: source.count_albums,
        track_count: source.count_tracks,
        mv_count: None,
        video_count: None,
        identities: Vec::new(),
        extensions: Extensions::from([(
            "backend".to_owned(),
            json!("official_android_account_following_artists"),
        )]),
    })
}

fn parse_write_ack(body: &[u8], artist_id: &str, subscribed: bool) -> Result<()> {
    if body.len() > MAX_PAGE_BYTES {
        return Err(artist_collection_invalid(
            "Soda artist collection ACK is too large",
        ));
    }
    let ack: WriteAck = serde_json::from_slice(body)
        .map_err(|_| artist_collection_invalid("Soda artist collection ACK is malformed"))?;
    match ack.status_code {
        Some(100_001 | 1_000_016) => return Err(artist_collection_authentication_required()),
        Some(0) => {}
        _ => {
            return Err(artist_collection_invalid(
                "Soda artist collection write was rejected",
            ));
        }
    }
    let status = if subscribed {
        ack.collect_artist_status
    } else {
        ack.deleted_artist_status
    }
    .ok_or_else(|| {
        artist_collection_invalid("Soda artist collection ACK omitted its status map")
    })?;
    if status.len() != 1 || !status.contains_key(artist_id) {
        return Err(artist_collection_invalid(
            "Soda artist collection ACK did not identify the requested artist",
        ));
    }
    Ok(())
}

fn valid_cursor(cursor: &str) -> bool {
    !cursor.is_empty() && cursor.len() <= 256 && !cursor.chars().any(char::is_control)
}

fn require_json_content_type(headers: &reqwest::header::HeaderMap, operation: &str) -> Result<()> {
    if headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
    {
        Ok(())
    } else {
        Err(artist_collection_invalid(&format!(
            "{operation} returned an unexpected content type"
        )))
    }
}

fn artist_collection_authentication_required() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "Soda artist collection requires the selected authenticated account",
    )
    .with_platform(Platform::Soda)
}

fn artist_collection_challenge() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::CapabilityNotSupported,
        "Soda artist collection requires an additional platform verification challenge",
    )
    .with_platform(Platform::Soda)
}

fn artist_collection_invalid(message: &str) -> TuneWeaveError {
    soda_upstream_error(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artist_collection_page_requires_complete_pagination_and_valid_unique_ids() {
        let first = br#"{"status_code":0,"artists":[{"id":"11","name":"A"}],"next_cursor":"next","total_num":2,"has_more":true}"#;
        let parsed = parse_collection_page(first).unwrap();
        assert_eq!(parsed.artist_ids, ["11"]);
        assert_eq!(parsed.artists[0].resource_ref.to_string(), "soda:11");
        assert_eq!(parsed.artists[0].name, "A");
        assert_eq!(parsed.next_cursor.as_deref(), Some("next"));
        assert_eq!(parsed.total_num, 2);
        assert!(parsed.has_more);

        let final_page = br#"{"status_code":0,"artists":[{"id":"22","name":"B"}],"next_cursor":"","total_num":1,"has_more":false}"#;
        assert!(parse_collection_page(final_page).is_ok());
        let terminal_cursor = br#"{"status_code":0,"artists":[],"next_cursor":"terminal-token","total_num":0,"has_more":false}"#;
        assert!(parse_collection_page(terminal_cursor).is_ok());
        for body in [
            br#"{"status_code":0,"artists":[],"total_num":0}"#.as_slice(),
            br#"{"status_code":0,"artists":[],"total_num":1,"has_more":false}"#,
            br#"{"status_code":0,"artists":[],"next_cursor":"","total_num":0,"has_more":true}"#,
            br#"{"status_code":0,"artists":[{"id":"01","name":"A"}],"next_cursor":"","total_num":1,"has_more":false}"#,
            br#"{"status_code":0,"artists":[{"id":"11"}],"next_cursor":"","total_num":1,"has_more":false}"#,
            br#"{"status_code":0,"artists":[{"id":"11","name":" "}],"next_cursor":"","total_num":1,"has_more":false}"#,
            br#"{"status_code":0,"artists":[{"id":"11","name":"A","count_albums":1000001}],"next_cursor":"","total_num":1,"has_more":false}"#,
            br#"{"status_code":0,"artists":[{"id":"11","name":"A"},{"id":"11","name":"A"}],"next_cursor":"","total_num":2,"has_more":false}"#,
            br#"{"status_code":1000016,"artists":[],"next_cursor":"","total_num":0,"has_more":false}"#,
        ] {
            assert!(parse_collection_page(body).is_err());
        }
    }

    #[test]
    fn artist_collection_write_ack_requires_status_and_requested_id() {
        assert!(
            parse_write_ack(
                br#"{"status_code":0,"artist_ids":["11"],"collect_artist_status":{"11":1}}"#,
                "11",
                true,
            )
            .is_ok()
        );
        assert!(
            parse_write_ack(
                br#"{"status_code":0,"deleted_artists":["11"],"deleted_artist_status":{"11":0}}"#,
                "11",
                false,
            )
            .is_ok()
        );
        for (body, subscribed) in [
            (br#"{"collect_artist_status":{"11":1}}"#.as_slice(), true),
            (
                br#"{"status_code":7,"collect_artist_status":{"11":1}}"#,
                true,
            ),
            (
                br#"{"status_code":0,"collect_artist_status":{"12":1}}"#,
                true,
            ),
            (
                br#"{"status_code":0,"collect_artist_status":{"11":1,"12":1}}"#,
                true,
            ),
            (
                br#"{"status_code":0,"deleted_artist_status":{"11":0}}"#,
                true,
            ),
            (
                br#"{"status_code":0,"collect_artist_status":{"11":1}}"#,
                false,
            ),
        ] {
            assert!(parse_write_ack(body, "11", subscribed).is_err());
        }
        for code in [100_001, 1_000_016] {
            let body = json!({"status_code":code,"collect_artist_status":{"11":1}});
            assert_eq!(
                parse_write_ack(&serde_json::to_vec(&body).unwrap(), "11", true)
                    .unwrap_err()
                    .code,
                ErrorCode::AuthenticationRequired
            );
        }
    }

    #[tokio::test]
    async fn artist_collection_wire_uses_single_ids_selected_cookie_and_mobile_login_headers() {
        let credential = SodaCredential::test_credential("selected-artist-cookie")
            .bind_user("123456")
            .unwrap();
        let replies = vec![
            crate::test_http::json(
                r#"{"status_code":0,"collect_artist_status":{"11":1}}"#,
                Some("sessionid_ss=write-rotated"),
            ),
            crate::test_http::json(
                r#"{"status_code":0,"artists":[{"id":"11","name":"Artist 11"}],"next_cursor":"","total_num":1,"has_more":false}"#,
                None,
            ),
        ];
        let (origin, server) = crate::test_http::serve(replies).await;
        let client = SodaClient::new(&SodaConfig::default())
            .unwrap()
            .with_auth_test_origin(origin);
        let rotated = client
            .write_account_artist_collection("11", true, &credential)
            .await
            .unwrap();
        assert_eq!(
            rotated.cookie_header().unwrap(),
            "sessionid_ss=write-rotated"
        );
        let page = client
            .account_artist_collection_page(None, &rotated)
            .await
            .unwrap();
        assert_eq!(page.artist_ids, ["11"]);
        assert_eq!(page.artists[0].name, "Artist 11");
        assert_eq!(page.total_num, 1);
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].starts_with("POST /luna/me/collection/artist "));
        assert!(requests[0].contains("cookie: sessionid_ss=selected-artist-cookie\r\n"));
        assert!(requests[0].contains("x-luna-api-version: 2023-01-04\r\n"));
        assert!(requests[0].contains("x-luna-is-login: 1\r\n"));
        let body = requests[0].split("\r\n\r\n").nth(1).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            json!({"artist_ids":["11"]})
        );
        assert!(requests[1].starts_with("GET /luna/me/collection/artist?count=100 "));
        assert!(requests[1].contains("cookie: sessionid_ss=write-rotated\r\n"));
        assert!(requests[1].contains("x-luna-api-version: 2023-01-04\r\n"));
        assert!(requests[1].contains("x-luna-is-login: 1\r\n"));
    }
}
