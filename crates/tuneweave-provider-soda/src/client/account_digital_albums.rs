use crate::account::{add_luna_pc_headers, luna_pc_endpoint};
use crate::login::SodaCredential;

use super::*;

pub(crate) const BACKEND: &str = "official_pc_account_digital_albums";
const ENDPOINT_PATH: &str = "/luna/pc/me/assets/albums";
const MAX_ALBUMS: usize = 10_000;

#[derive(Debug)]
pub(crate) struct SodaAccountDigitalAlbums {
    pub albums: Vec<DigitalAlbum>,
    pub credential: SodaCredential,
}

#[derive(Deserialize)]
struct Business {
    status_code: Option<i64>,
    status_info: Option<BusinessInfo>,
}

#[derive(Deserialize)]
struct BusinessInfo {
    status_code: Option<i64>,
}

#[derive(Deserialize)]
struct Envelope {
    // The official client consumes `albums ?? []`, so an absent or null list is empty.
    #[serde(default)]
    albums: Option<Vec<DigitalAlbumWire>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyEnvelope {
    status_info: EmptyStatusInfo,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmptyStatusInfo {
    log_id: String,
    now: u64,
    now_ts_ms: u64,
}

#[derive(Default, Deserialize)]
#[serde(default)]
struct DigitalAlbumWire {
    id: String,
    name: String,
    artists: Option<Vec<SodaArtist>>,
    count_tracks: Option<u64>,
    url_cover: Option<SodaImage>,
    release_date: Option<u64>,
}

impl SodaClient {
    pub(crate) async fn account_digital_albums(
        &self,
        credential: &SodaCredential,
    ) -> Result<SodaAccountDigitalAlbums> {
        if credential.user_id().is_none() {
            return Err(authentication_required());
        }
        let started = Instant::now();
        let mut status = None;
        let result = async {
            let device = self.login_device()?;
            let mut url = luna_pc_endpoint(ENDPOINT_PATH, &device)?;
            {
                let mut query = url.query_pairs_mut();
                query.append_pair("cursor", "");
                query.append_pair("count", "100");
            }
            let response = self
                .send_login_request(
                    add_luna_pc_headers(self.login_request(reqwest::Method::GET, url))
                        .header("x-luna-background-type", "foreground")
                        .header("x-luna-is-background-req", "0")
                        .header("x-luna-is-local-user", "1")
                        .header(reqwest::header::COOKIE, credential.cookie_header()?),
                )
                .await?;
            status = Some(response.status());
            if response.status() == StatusCode::UNAUTHORIZED {
                return Err(authentication_required());
            }
            if response.status() != StatusCode::OK {
                return Err(soda_http_error(response.status()));
            }
            if response.headers().contains_key("bdturing-verify") {
                return Err(TuneWeaveError::new(
                    ErrorCode::CapabilityNotSupported,
                    "Soda purchased-albums request requires an additional verification challenge",
                )
                .with_platform(Platform::Soda));
            }
            if !response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| {
                    value
                        .split(';')
                        .next()
                        .unwrap_or_default()
                        .trim()
                        .eq_ignore_ascii_case("application/json")
                })
            {
                return Err(invalid(
                    "Soda purchased-albums request returned an unexpected content type",
                ));
            }
            let headers = response.headers().clone();
            let body = read_bounded_response(response, "Soda purchased albums").await?;
            let albums = parse(&body)?;
            let credential = credential.with_response_cookies(&headers)?;
            Ok(SodaAccountDigitalAlbums { albums, credential })
        }
        .await;
        self.log_upstream_request(
            "account_digital_albums",
            "api.qishui.com",
            ENDPOINT_PATH,
            status,
            started,
            &result,
        );
        result
    }
}

fn parse(body: &[u8]) -> Result<Vec<DigitalAlbum>> {
    let business: Business = serde_json::from_slice(body)
        .map_err(|_| invalid("Soda purchased-albums response was malformed"))?;
    let codes = [
        business.status_code,
        business.status_info.and_then(|info| info.status_code),
    ];
    if codes.contains(&Some(1_000_016)) {
        return Err(authentication_required());
    }
    if codes.into_iter().flatten().any(|code| code != 0) {
        return Err(invalid("Soda rejected the purchased-albums request"));
    }
    if !codes.contains(&Some(0)) {
        return if parse_empty(body) {
            Ok(Vec::new())
        } else {
            Err(invalid(
                "Soda purchased-albums response omitted a verifiable success status",
            ))
        };
    }

    let envelope: Envelope = serde_json::from_slice(body)
        .map_err(|_| invalid("Soda purchased-albums response was malformed"))?;
    let rows = envelope.albums.unwrap_or_default();
    if rows.len() > MAX_ALBUMS {
        return Err(invalid(
            "Soda purchased-albums response exceeded the safety limit",
        ));
    }
    let mut albums = Vec::with_capacity(rows.len());
    let mut seen = BTreeSet::new();
    for row in rows {
        let album = map_album(row)?;
        // The official Me page deduplicates the response by album.id, keeping the first item.
        if seen.insert(album.id.clone()) {
            albums.push(album);
        }
    }
    Ok(albums)
}

fn parse_empty(body: &[u8]) -> bool {
    let Ok(response) = serde_json::from_slice::<EmptyEnvelope>(body) else {
        return false;
    };
    let info = response.status_info;
    !info.log_id.trim().is_empty()
        && info.log_id.len() <= 256
        && !info.log_id.chars().any(char::is_control)
        && info.now > 0
        && info.now_ts_ms > 0
}

fn map_album(source: DigitalAlbumWire) -> Result<DigitalAlbum> {
    let id = canonical_positive_decimal(&source.id)
        .ok_or_else(|| invalid("Soda purchased-albums response contained an invalid album ID"))?;
    let name = bounded_text(&source.name, 1_000)
        .ok_or_else(|| invalid("Soda purchased-albums response omitted an album name"))?;
    let artists = source.artists.unwrap_or_default();
    if artists.len() > 128
        || artists.iter().any(|artist| {
            artist.name.len() > 1_000
                || artist.simple_display_name.len() > 1_000
                || (!artist.id.trim().is_empty()
                    && canonical_positive_decimal(&artist.id).is_none())
        })
        || source.count_tracks.is_some_and(|count| count > 10_000)
        || source
            .release_date
            .is_some_and(|timestamp| timestamp > 4_102_444_800)
    {
        return Err(invalid(
            "Soda purchased-albums response contained invalid album metadata",
        ));
    }
    let resource_ref = ResourceRef::new(Platform::Soda, id)
        .map_err(|_| invalid("Soda purchased-albums identity could not be normalized"))?;
    Ok(DigitalAlbum {
        resource_ref,
        platform: Platform::Soda,
        id: id.to_owned(),
        name,
        artists: artists.iter().filter_map(map_artist_summary).collect(),
        description: String::new(),
        cover_url: source.url_cover.as_ref().and_then(normalize_image),
        published_at: source
            .release_date
            .filter(|timestamp| *timestamp > 0)
            .and_then(unix_rfc3339),
        price: None,
        is_free: None,
        purchasable: None,
        purchased: Some(true),
        sale_count: None,
        track_count: source.count_tracks,
        tags: Vec::new(),
        extensions: Extensions::from([("backend".to_owned(), json!(BACKEND))]),
    })
}

fn invalid(message: impl Into<String>) -> TuneWeaveError {
    soda_upstream_error(message)
}

fn authentication_required() -> TuneWeaveError {
    TuneWeaveError::new(
        ErrorCode::AuthenticationRequired,
        "Soda purchased-albums request requires the selected account session",
    )
    .with_platform(Platform::Soda)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> serde_json::Value {
        json!({
            "status_code": 0,
            "albums": [{
                "id": "7528799183039825936",
                "name": "Digital Album",
                "artists": [{"id":"654321","name":"Artist","simple_display_name":""}],
                "count_tracks": 12,
                "url_cover": {
                    "uri":"album/cover",
                    "urls":["https://p1-luna.douyinpic.com/img/"],
                    "template_prefix":""
                },
                "release_date": 1700000000,
                "digital_album_info": {"sale_period": 2, "count_purchased": 1},
                "private_token": "must-not-escape"
            }]
        })
    }

    #[test]
    fn maps_only_officially_consumed_album_fields_and_marks_membership() {
        let albums = parse(&fixture().to_string().into_bytes()).unwrap();
        assert_eq!(albums.len(), 1);
        let album = &albums[0];
        assert_eq!(album.id, "7528799183039825936");
        assert_eq!(album.resource_ref.to_string(), "soda:7528799183039825936");
        assert_eq!(album.name, "Digital Album");
        assert_eq!(album.artists[0].name, "Artist");
        assert_eq!(
            album.artists[0].resource_ref.as_ref().unwrap().id(),
            "654321"
        );
        assert_eq!(
            album.cover_url.as_deref(),
            Some("https://p1-luna.douyinpic.com/img/album/cover")
        );
        assert_eq!(album.track_count, Some(12));
        assert_eq!(album.published_at.as_deref(), Some("2023-11-14T22:13:20Z"));
        assert_eq!(album.purchased, Some(true));
        assert_eq!(album.price, None);
        assert_eq!(album.is_free, None);
        assert_eq!(album.purchasable, None);
        assert_eq!(album.sale_count, None);
        assert_eq!(album.description, "");
        assert!(album.tags.is_empty());
        assert!(
            !serde_json::to_string(album)
                .unwrap()
                .contains("must-not-escape")
        );
        assert_eq!(album.extensions["backend"], BACKEND);
    }

    #[test]
    fn mirrors_official_first_occurrence_dedup_and_empty_album_semantics() {
        let mut body = fixture();
        let duplicate = body["albums"][0].clone();
        body["albums"].as_array_mut().unwrap().push(duplicate);
        let albums = parse(&body.to_string().into_bytes()).unwrap();
        assert_eq!(albums.len(), 1);
        assert_eq!(albums[0].name, "Digital Album");
        assert!(parse(br#"{"status_code":0}"#).unwrap().is_empty());
        assert!(
            parse(br#"{"status_code":0,"albums":null}"#)
                .unwrap()
                .is_empty()
        );
        assert!(
            parse(br#"{"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456}}"#)
                .unwrap()
                .is_empty()
        );
        assert!(parse(br#"{"status_info":{"log_id":"request-log","now":123,"now_ts_ms":123456},"albums":[]}"#).is_err());
        assert!(parse(br#"{"status_info":{"log_id":"","now":123,"now_ts_ms":123456}}"#).is_err());
    }

    #[test]
    fn rejects_unverified_status_identity_and_unbounded_fields() {
        for body in [
            json!({"albums": []}),
            json!({"status_code": 1000016, "albums": []}),
            json!({"status_code": 0, "albums": [{"id":"01","name":"bad"}]}),
            json!({"status_code": 0, "albums": [{"id":"1","name":"ok","count_tracks":10001}]}),
            json!({"status_code": 0, "albums": [{"id":"1","name":"ok","release_date":4_102_444_801u64}]}),
        ] {
            assert!(parse(&body.to_string().into_bytes()).is_err(), "{body}");
        }
        assert_eq!(
            parse(br#"{"status_info":{"status_code":1000016}}"#)
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
    }

    #[tokio::test]
    async fn account_digital_albums_uses_pc_route_device_identity_and_selected_cookie() {
        let source = SodaCredential::test_credential("selected-cookie")
            .bind_user("123456")
            .unwrap();
        let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
            &fixture().to_string(),
            Some("sessionid_ss=rotated-session; Path=/"),
        )])
        .await;
        let client = SodaClient::test_client().with_auth_test_origin(origin);
        let device = client.login_device().unwrap();
        let result = client.account_digital_albums(&source).await.unwrap();
        assert_eq!(result.albums.len(), 1);
        assert!(
            result
                .credential
                .cookie_header()
                .unwrap()
                .contains("rotated-session")
        );
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert!(request.starts_with("GET /luna/pc/me/assets/albums?"));
        assert!(request.contains("aid=386088"));
        assert!(request.contains("app_name=luna_pc"));
        assert!(request.contains("device_platform=windows"));
        assert!(request.contains("channel=official"));
        assert!(request.contains("version_name=3.7.0"));
        assert!(request.contains("version_code=30070000"));
        assert!(request.contains(&format!("device_id={}", device.device_id)));
        assert!(request.contains(&format!("fp={}", device.device_id)));
        assert!(request.contains("iid="));
        assert!(request.contains("cursor=&count=100"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("user-agent: lunapc/3.7.0")
        );
        assert!(request.contains("x-luna-background-type: foreground\r\n"));
        assert!(request.contains("x-luna-is-background-req: 0\r\n"));
        assert!(request.contains("x-luna-is-local-user: 1\r\n"));
        assert!(
            request
                .to_ascii_lowercase()
                .contains("cookie: sessionid_ss=selected-cookie")
        );
    }

    #[tokio::test]
    async fn account_digital_albums_rejects_anonymous_before_network() {
        let error = SodaClient::test_client()
            .account_digital_albums(&SodaCredential::test_credential("anonymous"))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    }
}
