use super::*;
use crate::KuwoProvider;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
    task::JoinHandle,
};
use tuneweave_core::{MusicProvider, SearchKind, SearchQuery, SearchVariant};

const COOKIE_VALUE: &str = "anonymousCatalogueCookie123456";

pub(crate) fn item(kind: CatalogKind, id: u64) -> serde_json::Value {
    match kind {
        CatalogKind::Mv => {
            json!({"id":id,"name":format!("Video {id} &amp; Live"),"artist":"Artist &amp; Co","artistid":336,"duration":269,"mvPlayCnt":1234,"online":1,"pic":"https://img1.kuwo.cn/wmvpic/324/a.jpg"})
        }
        CatalogKind::Album => {
            json!({"albumid":id,"album":format!("Album {id}"),"artist":"Artist &amp; Co","artistid":336,"albuminfo":"Line one\nLine two","releaseDate":"2024-02-29","pic":"https://img1.kuwo.cn/star/albumcover/300/a.jpg","lang":"国语","content_type":"0"})
        }
        CatalogKind::Artist => {
            json!({"id":id,"name":format!("Artist {id}"),"musicNum":"29","artistFans":0,"country":"中国","pic":"https://star.kuwo.cn/star/starheads/240/a.jpg","content_type":0})
        }
        CatalogKind::Playlist => {
            json!({"id":id.to_string(),"name":format!("Playlist {id}"),"total":"135","uname":"Listener","listencnt":"430","img":"https://img1.kuwo.cn/star/userpl2015/a.jpg"})
        }
    }
}
pub(crate) fn body(kind: CatalogKind, page: u32, total: u64) -> serde_json::Value {
    let size = u64::from(kind.page_size());
    let start = u64::from(page - 1) * size;
    let items: Vec<_> = (start..total.min(start + size))
        .map(|id| item(kind, id + 1))
        .collect();
    let data = match kind {
        CatalogKind::Mv => json!({"total":total.to_string(),"mvlist":items}),
        CatalogKind::Album => json!({"total":total.to_string(),"albumList":items}),
        CatalogKind::Artist => {
            json!({"total":total.to_string(),"artistList":items,"pn":page-1,"rn":size})
        }
        CatalogKind::Playlist => json!({"total":total.to_string(),"list":items}),
    };
    json!({"code":200,"data":data})
}
fn field(kind: CatalogKind) -> &'static str {
    match kind {
        CatalogKind::Mv => "mvlist",
        CatalogKind::Album => "albumList",
        CatalogKind::Artist => "artistList",
        CatalogKind::Playlist => "list",
    }
}
fn query(kind: CatalogKind, limit: u32, offset: u32) -> SearchQuery {
    let mut query = SearchQuery::tracks(" 周杰伦 & + / ? ", limit, offset);
    query.kind = match kind {
        CatalogKind::Mv => SearchKind::Mv,
        CatalogKind::Album => SearchKind::Album,
        CatalogKind::Artist => SearchKind::Artist,
        CatalogKind::Playlist => SearchKind::Playlist,
    };
    query
}
fn ids(items: &[SearchItem]) -> Vec<&str> {
    items
        .iter()
        .map(|item| match item {
            SearchItem::Album(item) => item.id.as_str(),
            SearchItem::Artist(item) => item.id.as_str(),
            SearchItem::Playlist(item) => item.id.as_str(),
            _ => panic!("unexpected search type"),
        })
        .collect()
}
fn parse(kind: CatalogKind, page: u32, value: &serde_json::Value) -> Result<CatalogPage> {
    parse_page(kind, page, &serde_json::to_vec(value).unwrap())
}
pub(crate) fn response(status: u16, mime: &str, headers: &str, body: &[u8]) -> Vec<u8> {
    let mut wire = format!("HTTP/1.1 {status} Fixture\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",body.len()).into_bytes();
    wire.extend(body);
    wire
}
pub(crate) fn json_response(body: &serde_json::Value) -> Vec<u8> {
    response(
        200,
        "application/json;charset=UTF-8",
        "",
        &serde_json::to_vec(body).unwrap(),
    )
}
pub(crate) fn home() -> Vec<u8> {
    home_with(COOKIE_VALUE)
}
pub(crate) fn home_with(cookie: &str) -> Vec<u8> {
    response(
        200,
        "text/html",
        &format!("Set-Cookie: {WEB_SESSION_COOKIE}={cookie}; Path=/; Secure\r\n"),
        b"<html></html>",
    )
}
pub(crate) struct Fixture {
    pub(crate) provider: KuwoProvider,
    pub(crate) client: KuwoClient,
    pub(crate) seen: mpsc::UnboundedReceiver<String>,
    pub(crate) server: JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
pub(crate) async fn setup(responses: Vec<Vec<u8>>) -> Fixture {
    setup_with_gates(responses.into_iter().map(|body| (body, None)).collect()).await
}
pub(crate) async fn setup_with_gates(
    responses: Vec<(Vec<u8>, Option<Arc<tokio::sync::Notify>>)>,
) -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    let (tx, seen) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        for (response, gate) in responses {
            let (mut stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
                .await
                .unwrap()
                .unwrap();
            let mut bytes = Vec::new();
            while !bytes.ends_with(b"\r\n\r\n") {
                let mut buffer = [0; 1024];
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0);
                bytes.extend_from_slice(&buffer[..read]);
                assert!(bytes.len() < 16 * 1024);
            }
            tx.send(String::from_utf8(bytes).unwrap()).unwrap();
            if let Some(gate) = gate {
                gate.notified().await;
            }
            let _ = stream.write_all(&response).await;
        }
    });
    let mut client = KuwoClient::test_client();
    client.http = Client::builder()
        .no_proxy()
        .redirect(Policy::none())
        .timeout(Duration::from_secs(2))
        .build()
        .unwrap();
    client.web_test_origin = Some(Url::parse(&origin).unwrap());
    Fixture {
        provider: KuwoProvider::from_client(client.clone()),
        client,
        seen,
        server,
    }
}
pub(crate) async fn requests(fixture: &mut Fixture, expected: usize) -> Vec<String> {
    (&mut fixture.server).await.unwrap();
    let mut result = Vec::new();
    while let Ok(request) = fixture.seen.try_recv() {
        result.push(request);
    }
    assert_eq!(result.len(), expected);
    result
}

#[test]
fn typed_catalogue_metadata_preserves_identity_and_unknowns() {
    for kind in [
        CatalogKind::Album,
        CatalogKind::Artist,
        CatalogKind::Playlist,
    ] {
        let mut value = body(kind, 1, 1);
        value["data"][field(kind)][0]["opaque"] =
            json!({"sid":"do-not-export","url":"https://example.test"});
        let page = parse(kind, 1, &value).unwrap();
        assert_eq!(ids(&page.items), vec!["1"]);
        let encoded = serde_json::to_string(&page.items).unwrap();
        assert!(!encoded.contains("do-not-export"));
        assert!(!encoded.contains("example.test"));
        match &page.items[0] {
            SearchItem::Album(album) => {
                assert_eq!(album.resource_ref.to_string(), "kuwo:1");
                assert_eq!(
                    album.artists[0].resource_ref.as_ref().unwrap().to_string(),
                    "kuwo:336"
                );
                assert_eq!(album.artists[0].name, "Artist & Co");
                assert_eq!(album.description, "Line one\nLine two");
                assert_eq!(album.published_at.as_deref(), Some("2024-02-29"));
                assert!(album.track_count.is_none());
                assert!(album.kind.is_none());
            }
            SearchItem::Artist(artist) => {
                assert_eq!(artist.track_count, Some(29));
                assert_eq!(artist.extensions["artist_fans"], 0);
                assert!(
                    artist
                        .avatar_url
                        .as_ref()
                        .unwrap()
                        .starts_with("https://star.kuwo.cn/")
                );
                assert!(artist.album_count.is_none());
                assert!(artist.mv_count.is_none());
            }
            SearchItem::Playlist(playlist) => {
                assert_eq!(playlist.track_count, Some(135));
                assert_eq!(playlist.extensions["creator_name"], "Listener");
                assert!(playlist.creator.is_none());
                assert!(playlist.subscribed.is_none());
            }
            _ => unreachable!(),
        }
    }
    let mut artist = body(CatalogKind::Artist, 1, 1);
    artist["data"]["artistList"][0]
        .as_object_mut()
        .unwrap()
        .remove("musicNum");
    let SearchItem::Artist(item) = parse(CatalogKind::Artist, 1, &artist)
        .unwrap()
        .items
        .remove(0)
    else {
        panic!()
    };
    assert!(item.track_count.is_none());
}

#[test]
fn malformed_pages_cannot_become_empty_or_partial_success() {
    for kind in [
        CatalogKind::Album,
        CatalogKind::Artist,
        CatalogKind::Playlist,
    ] {
        for value in [
            json!({}),
            json!({"code":200}),
            json!({"code":200,"data":null}),
            json!({"code":200,"data":{}}),
            json!({"code":"200","data":{}}),
            json!({"code":2001,"data":{}}),
        ] {
            assert!(parse(kind, 1, &value).is_err(), "{kind:?}: {value}");
        }
        for total in [
            json!(-1),
            json!(true),
            json!(1.5),
            json!("01"),
            json!("+1"),
            json!("18446744073709551616"),
        ] {
            let mut value = body(kind, 1, 1);
            value["data"]["total"] = total;
            assert!(parse(kind, 1, &value).is_err());
        }
        let mut short = body(kind, 1, 100);
        short["data"][field(kind)].as_array_mut().unwrap().pop();
        assert!(parse(kind, 1, &short).is_err());
        let mut extra = body(kind, 1, 1);
        extra["data"][field(kind)]
            .as_array_mut()
            .unwrap()
            .push(item(kind, 2));
        assert!(parse(kind, 1, &extra).is_err());
        let mut wrong = body(kind, 1, 1);
        wrong["data"][field(kind)] = json!([{"id":1}]);
        assert!(parse(kind, 1, &wrong).is_err());
        assert_eq!(parse(kind, 1, &body(kind, 1, 0)).unwrap().total, 0);
        assert!(parse(kind, 2, &body(kind, 2, 1)).unwrap().items.is_empty());
        let error = parse(
            kind,
            1,
            &json!({"code":-111,"msg":"private-cookie-secret","data":null}),
        )
        .err()
        .unwrap();
        assert!(!format!("{error:?}").contains("private-cookie-secret"));
    }
    for (field, value) in [("pn", json!(1)), ("rn", json!(20))] {
        let mut wrong = body(CatalogKind::Artist, 1, 1);
        wrong["data"][field] = value;
        assert!(parse(CatalogKind::Artist, 1, &wrong).is_err());
    }
}

#[test]
fn metadata_boundaries_do_not_invent_ids_dates_or_image_hosts() {
    for (field, value) in [
        ("albumid", json!("01")),
        ("albumid", json!(0)),
        ("album", json!("\u{0000}")),
        ("album", json!(" ")),
        ("album", json!("a".repeat(513))),
        ("artistid", json!(true)),
        ("content_type", json!(1)),
        ("releaseDate", json!("2023-02-29")),
        ("releaseDate", json!("2024-04-31")),
        ("releaseDate", json!("2024-12-01T00:00:00")),
    ] {
        let mut value_body = body(CatalogKind::Album, 1, 1);
        value_body["data"]["albumList"][0][field] = value;
        assert!(
            parse(CatalogKind::Album, 1, &value_body).is_err(),
            "{field}"
        );
    }
    for bad in [
        "http://star.kuwo.cn/star/starheads/a.jpg",
        "https://star.kuwo.cn.evil.test/star/starheads/a.jpg",
        "https://star.kuwo.cn/star/starheads/a.jpg?token=secret",
        "https://name@star.kuwo.cn/star/starheads/a.jpg",
        "https://star.kuwo.cn/other/a.jpg",
    ] {
        let mut value = body(CatalogKind::Artist, 1, 1);
        value["data"]["artistList"][0]["pic"] = json!(bad);
        let SearchItem::Artist(artist) = parse(CatalogKind::Artist, 1, &value)
            .unwrap()
            .items
            .remove(0)
        else {
            panic!()
        };
        assert!(artist.avatar_url.is_none());
    }
    assert_eq!(name("A&nbsp;B &amp; C").unwrap(), "A B & C");
}

#[tokio::test]
async fn maximum_windows_keep_typed_order_and_exact_signed_requests() {
    for kind in [
        CatalogKind::Album,
        CatalogKind::Artist,
        CatalogKind::Playlist,
    ] {
        let skip = kind.page_size() - 1;
        let budget = (skip + 100).div_ceil(kind.page_size());
        let mut responses = vec![home()];
        responses.extend((1..=budget).map(|page| json_response(&body(kind, page, 162))));
        let mut fixture = setup(responses).await;
        let result = fixture
            .provider
            .search_catalog(&query(kind, 100, skip))
            .await
            .unwrap();
        assert_eq!(
            ids(&result.items),
            ((skip + 1)..=(skip + 100))
                .map(|id| id.to_string())
                .collect::<Vec<_>>()
        );
        assert_eq!(result.pagination.total, Some(162));
        assert_eq!(result.pagination.next_offset, Some(skip + 100));
        assert_eq!(
            result.pagination.extensions["upstream_pages_fetched"],
            budget
        );
        let calls = requests(&mut fixture, 1 + budget as usize).await;
        assert!(calls[0].starts_with("GET / HTTP/1.1"));
        assert!(!calls[0].to_ascii_lowercase().contains("cookie:"));
        let mut request_ids = std::collections::BTreeSet::new();
        for (index, call) in calls[1..].iter().enumerate() {
            let line = call.lines().next().unwrap();
            let target = line.split_whitespace().nth(1).unwrap();
            let url = Url::parse(&format!("https://www.kuwo.cn{target}")).unwrap();
            assert_eq!(url.path(), kind.path());
            let params = url.query_pairs().collect::<BTreeMap<_, _>>();
            assert_eq!(params.len(), 7);
            assert_eq!(params["key"], "周杰伦 & + / ?");
            assert_eq!(params["pn"], (index + 1).to_string());
            assert_eq!(params["rn"], kind.page_size().to_string());
            assert_eq!(params["plat"], "web_www");
            assert_eq!(params["httpsStatus"], "1");
            assert_eq!(params["from"], "");
            assert!(request_ids.insert(params["reqId"].to_string()));
            assert_eq!(&params["reqId"][14..15], "4");
            let lower = call.to_ascii_lowercase();
            assert!(!lower.contains("authorization:"));
            assert!(!lower.contains("sid="));
            assert!(call.contains(&format!("cookie: {WEB_SESSION_COOKIE}={COOKIE_VALUE}")));
            let signed = call
                .lines()
                .find_map(|line| line.strip_prefix("secret: "))
                .unwrap();
            let nonce = u64::from_str_radix(&signed[signed.len() - 8..], 16).unwrap();
            assert_eq!(signed, web_secret_for_nonce(COOKIE_VALUE, nonce).unwrap());
            assert!(call.contains("referer: https://www.kuwo.cn/search/list"));
        }
    }
}

#[tokio::test]
async fn windows_verify_end_empty_total_drift_and_repeated_ids() {
    let kind = CatalogKind::Playlist;
    for (offset, limit, total, expected) in [(29, 10, 33, 4), (0, 20, 0, 0), (60, 20, 33, 0)] {
        let first = offset / 30 + 1;
        let mut responses = vec![home(), json_response(&body(kind, first, total))];
        if expected > 1 && offset == 29 {
            responses.push(json_response(&body(kind, first + 1, total)));
        }
        let mut fixture = setup(responses).await;
        let page = fixture
            .provider
            .search_catalog(&query(kind, limit, offset))
            .await
            .unwrap();
        assert_eq!(page.items.len(), expected);
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.next_offset, None);
        requests(&mut fixture, if offset == 29 { 3 } else { 2 }).await;
    }
    for failure in [0, 1, 2] {
        let first = body(kind, 1, 70);
        let mut second = body(kind, 2, 70);
        match failure {
            0 => second["data"]["total"] = json!(71),
            1 => second["data"]["list"] = first["data"]["list"].clone(),
            _ => second["data"]["list"][0] = first["data"]["list"][29].clone(),
        }
        let mut fixture = setup(vec![home(), json_response(&first), json_response(&second)]).await;
        assert!(
            fixture
                .provider
                .search_catalog(&query(kind, 40, 0))
                .await
                .is_err()
        );
        requests(&mut fixture, 3).await;
    }
    let mut duplicate = body(kind, 1, 2);
    duplicate["data"]["list"][1] = duplicate["data"]["list"][0].clone();
    let mut fixture = setup(vec![home(), json_response(&duplicate)]).await;
    assert!(
        fixture
            .provider
            .search_catalog(&query(kind, 1, 0))
            .await
            .is_err()
    );
    requests(&mut fixture, 2).await;
}

#[tokio::test]
async fn anonymous_signature_refresh_is_once_and_terminal_errors_never_retry() {
    let kind = CatalogKind::Artist;
    for rejection in [
        response(403, "text/plain", "", b"denied"),
        json_response(&json!({"success":false,"message":"The request is illegal!"})),
    ] {
        let mut fixture = setup(vec![
            home(),
            rejection.clone(),
            home_with("refreshedAnonymousCookie234567"),
            json_response(&body(kind, 1, 1)),
        ])
        .await;
        assert_eq!(
            fixture
                .provider
                .search_catalog(&query(kind, 1, 0))
                .await
                .unwrap()
                .items
                .len(),
            1
        );
        let calls = requests(&mut fixture, 4).await;
        assert!(calls[3].contains(&format!(
            "cookie: {WEB_SESSION_COOKIE}=refreshedAnonymousCookie234567"
        )));
        assert!(!calls[3].contains(COOKIE_VALUE));
        let mut fixture = setup(vec![home(), rejection.clone(), home(), rejection]).await;
        assert!(
            fixture
                .provider
                .search_catalog(&query(kind, 1, 0))
                .await
                .is_err()
        );
        requests(&mut fixture, 4).await;
    }
    for response in [
        response(429, "application/json", "", b"{}"),
        response(500, "application/json", "", b"{}"),
        response(302, "text/html", "Location: https://example.test/\r\n", b""),
        response(200, "text/html", "", b"{}"),
        json_response(&json!({"code":200,"data":{}})),
        json_response(&json!({"code":-1,"msg":"private-message"})),
    ] {
        let mut fixture = setup(vec![home(), response]).await;
        let error = fixture
            .provider
            .search_catalog(&query(kind, 1, 0))
            .await
            .unwrap_err();
        assert!(!format!("{error:?}").contains("private-message"));
        requests(&mut fixture, 2).await;
    }
}

#[tokio::test]
async fn catalogue_transport_rejects_declared_and_streamed_oversize() {
    let large = vec![b' '; RESPONSE_LIMIT as usize + 1];
    let mut chunked=b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
    chunked.extend(format!("{:x}\r\n", large.len()).bytes());
    chunked.extend(&large);
    chunked.extend(b"\r\n0\r\n\r\n");
    for response in [b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2097153\r\nConnection: close\r\n\r\n".to_vec(),chunked] {
        let mut fixture=setup(vec![home(),response]).await;
        assert!(fixture.provider.search_catalog(&query(CatalogKind::Album,1,0)).await.is_err());requests(&mut fixture,2).await;
    }
}

#[tokio::test]
async fn catalogue_options_fail_before_any_request() {
    let mut fixture = setup(vec![]).await;
    let credential = tuneweave_core::ProviderCredential::new(
        Platform::Kuwo,
        "unsupported",
        "private-value",
        None,
    )
    .unwrap();
    assert_eq!(
        fixture
            .provider
            .with_caller_credential(&credential)
            .err()
            .unwrap()
            .code,
        ErrorCode::InvalidRequest
    );
    let valid = query(CatalogKind::Album, 1, 0);
    let mut cases = vec![];
    let mut q = valid.clone();
    q.account = Some("private".into());
    cases.push(q);
    let mut q = valid.clone();
    q.highlight = true;
    cases.push(q);
    let mut q = valid.clone();
    q.search_id = Some("foreign".into());
    cases.push(q);
    let mut q = valid.clone();
    q.query = "\n\t".into();
    cases.push(q);
    let mut q = valid.clone();
    q.query = "a".repeat(513);
    cases.push(q);
    let mut q = valid.clone();
    q.limit = 101;
    cases.push(q);
    let mut q = valid.clone();
    q.limit = 0;
    cases.push(q);
    let mut q = valid.clone();
    q.offset = u32::MAX;
    cases.push(q);
    let mut q = valid.clone();
    q.kind = SearchKind::User;
    cases.push(q);
    let mut q = valid.clone();
    q.variant = SearchVariant::Cloud;
    cases.push(q);
    for q in cases {
        assert!(fixture.provider.search_catalog(&q).await.is_err());
    }
    requests(&mut fixture, 0).await;
}

#[tokio::test]
#[ignore = "Current official anonymous catalogue requests; no account or media"]
async fn live_catalogue_search_types_and_cross_page_windows() {
    let provider = KuwoProvider::new(KuwoConfig::default()).unwrap();
    for (kind, key, limit) in [
        (CatalogKind::Artist, "周杰伦", 35),
        (CatalogKind::Album, "周", 22),
        (CatalogKind::Playlist, "周", 32),
    ] {
        let mut q = query(kind, limit, 0);
        q.query = key.into();
        let result = provider.search_catalog(&q).await.unwrap();
        assert_eq!(result.items.len(), limit as usize);
        assert_eq!(result.pagination.extensions["upstream_pages_fetched"], 2);
        assert!(result.pagination.total.unwrap() >= u64::from(limit));
    }
    let mut q = query(CatalogKind::Playlist, 1, 0);
    q.query = "zzzzTuneWeaveNoCatalogue928415".into();
    let result = provider.search_catalog(&q).await.unwrap();
    assert!(result.items.is_empty());
    assert_eq!(result.pagination.total, Some(0));
}
