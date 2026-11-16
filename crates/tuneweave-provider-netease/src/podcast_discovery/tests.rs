use super::*;
use crate::crypto::{xeapi_registration_fixture, xeapi_response_fixture};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
};

fn category_tree_fixture() -> Value {
    json!({"code":200,"data":{"parentCategoryVoList":[
        {"id":"0020141111","name":"精品播客","desc":"深度对谈","coverUrl":"https://cdn.example/cover.jpg",
            "subCategoryVO":[{"id":"000","name":"全部"},{"id":"2","name":"音乐播客"}]},
        {"id":"2","name":"音乐播客","coverUrl":"file:///local.jpg",
            "subCategoryVO":[{"id":"000","name":"全部"}]}
    ]}})
}

fn dislike_list_fixture() -> Value {
    json!({"code":200,"data":{
        "songIdList":[7, "9223372036854775808"],"artistIdList":[],"podcastIds":["8"],"checkPoint":"3456",
        "cookie":"private-field"
    }})
}

fn collected_feed_fixture() -> Value {
    json!({"code":200,"data":{
        "feeds":[{
            "resourceId":"aBc_123","resource":{"mlogBaseData":{
                "title":"专栏","text":"播客动态","pubTime":1700000000000_u64,
                "coverUrl":"https://cdn.example/feed.jpg","audioUrl":"https://cdn.example/private.mp4"
            }},
            "token":"private-field"
        }],"totalCount":"2","more":true,"time":1699999999999_u64
    }})
}

#[test]
fn category_tree_preserves_opaque_ids_branch_order_and_placeholders() {
    let tree = parse_category_tree(&category_tree_fixture()).unwrap();
    assert_eq!(tree.categories[0].id, "0020141111");
    assert_eq!(tree.categories[0].children[0].id, "000");
    assert_eq!(tree.categories[0].children[1].id, tree.categories[1].id);
    assert_eq!(
        tree.categories[0].children[0],
        tree.categories[1].children[0]
    );
    assert_eq!(
        tree.categories[0].cover_url.as_deref(),
        Some("https://cdn.example/cover.jpg")
    );
    assert_eq!(tree.categories[1].cover_url, None);
    assert!(
        parse_category_tree(&json!({"data":{"parentCategoryVoList":[{"id":"2","name":"n"}]}}))
            .is_err()
    );
}

#[test]
fn dislike_list_preserves_large_ids_and_distinguishes_malformed_lists_from_empty() {
    let response = parse_dislike_list(&dislike_list_fixture()).unwrap();
    assert_eq!(response.song_ids, ["7", "9223372036854775808"]);
    assert!(response.artist_ids.is_empty());
    assert_eq!(response.podcast_ids, ["8"]);
    assert_eq!(response.checkpoint.as_deref(), Some("3456"));
    assert!(
        !serde_json::to_string(&response)
            .unwrap()
            .contains("private-field")
    );
    assert!(parse_dislike_list(&json!({"data":{"songIdList":[],"artistIdList":[]}})).is_err());

    let failure = check_success(
        &json!({"code":301, "message":"token=private-field"}),
        "recommendation dislike list",
    )
    .unwrap_err();
    assert_eq!(failure.code, ErrorCode::AuthenticationRequired);
    assert!(!failure.message.contains("private-field"));
}

#[test]
fn collected_feed_pages_preserve_the_native_cursor_and_only_presentation_fields() {
    let body = collected_feed_fixture();
    let first = parse_collected_feed(&body, 2, None).unwrap();
    assert_eq!(first.items[0].feed_id.as_deref(), Some("aBc_123"));
    assert_eq!(first.items[0].extensions["text"], "播客动态");
    assert_eq!(first.items[0].extensions["publish_time"], 1700000000000_u64);
    assert_eq!(first.total, Some(2));
    assert!(first.has_more);
    let next = first.next_cursor.unwrap();
    assert_eq!(next, "1699999999999");
    assert!(
        !serde_json::to_string(&first.items)
            .unwrap()
            .contains("private")
    );
    assert!(parse_collected_feed(&body, 2, Some(parse_cursor(&next).unwrap())).is_err());

    let last = parse_collected_feed(
        &json!({"data":{"feeds":[],"more":false,"time":0,"totalCount":2}}),
        2,
        Some(parse_cursor(&next).unwrap()),
    )
    .unwrap();
    assert!(!last.has_more);
    assert_eq!(last.next_cursor, None);
    assert!(last.items.is_empty());
    for invalid in ["0", "-1", "01", " 1", "+1", "9223372036854775808"] {
        assert_eq!(
            parse_cursor(invalid).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
}

async fn read_request(socket: &mut TcpStream) -> (String, String) {
    let mut bytes = Vec::new();
    let (offset, content_length) = loop {
        let mut buffer = [0; 2048];
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0, "request closed before its headers");
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(offset) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..offset]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.split_once(':')
                        .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                })
                .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                .unwrap_or(0);
            break (offset, content_length);
        }
    };
    let body_start = offset + 4;
    while bytes.len() < body_start + content_length {
        let mut buffer = [0; 2048];
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0, "request closed before its body");
        bytes.extend_from_slice(&buffer[..count]);
    }
    (
        String::from_utf8(bytes[..offset].to_vec()).unwrap(),
        String::from_utf8(bytes[body_start..body_start + content_length].to_vec()).unwrap(),
    )
}

#[tokio::test]
async fn discovery_calls_official_xeapi_paths_with_isolated_account_cookies() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let mut expected = [
            (
                "/xeapi/dj/radio/category/list",
                None,
                category_tree_fixture(),
            ),
            (
                "/xeapi/music/dislike/get",
                Some("fixture-selected-cookie"),
                dislike_list_fixture(),
            ),
            (
                "/xeapi/mlog/mycollect/bytime/get",
                Some("fixture-selected-cookie"),
                collected_feed_fixture(),
            ),
        ]
        .into_iter();
        let mut registrations = 0;
        while expected.len() > 0 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let (headers, payload) = read_request(&mut socket).await;
            let path = headers
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let form = url::form_urlencoded::parse(payload.as_bytes()).collect::<BTreeMap<_, _>>();
            let response = if path == "/api/bsr/sk/get" {
                registrations += 1;
                assert!(registrations <= 2);
                assert!(!headers.contains("MUSIC_U="));
                xeapi_registration_fixture(&form["nonce"])
            } else {
                let (expected_path, cookie, body) = expected.next().unwrap();
                assert_eq!(path, expected_path);
                for field in ["C", "S", "R"] {
                    assert!(form.get(field).is_some_and(|value| !value.is_empty()));
                }
                assert!(!headers.contains("fixture-default-cookie"));
                if let Some(cookie) = cookie {
                    assert!(
                        headers
                            .to_ascii_lowercase()
                            .contains(&format!("x-music-u: {cookie}"))
                    );
                } else {
                    assert!(!headers.to_ascii_lowercase().contains("x-music-u:"));
                    assert!(!headers.contains("MUSIC_U="));
                }
                xeapi_response_fixture(&body)
            };
            socket
                .write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        response.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            socket.write_all(&response).await.unwrap();
        }
        assert_eq!(registrations, 2);
    });
    let provider = NeteaseProvider::new(NeteaseConfig {
        base_url: endpoint.clone(),
        xeapi_base_url: endpoint,
        cookie: Some("MUSIC_U=fixture-default-cookie".to_owned()),
        timeout: Duration::from_secs(3),
        ..NeteaseConfig::default()
    })
    .unwrap();
    provider
        .install_session_in_memory(
            "selected".to_owned(),
            "MUSIC_U=fixture-selected-cookie".to_owned(),
        )
        .unwrap();

    let categories = provider.podcast_category_tree(None).await.unwrap();
    assert_eq!(categories.categories[0].id, "0020141111");
    let dislikes = provider
        .recommendation_dislikes(Some("selected"))
        .await
        .unwrap();
    assert_eq!(dislikes.artist_ids, Vec::<String>::new());
    let feed = provider
        .collected_podcast_feed(&CollectedPodcastFeedRequest {
            account: Some("selected".to_owned()),
            cursor: None,
            limit: 2,
        })
        .await
        .unwrap();
    assert_eq!(feed.next_cursor.as_deref(), Some("1699999999999"));
    tokio::time::timeout(Duration::from_secs(3), server)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn private_discovery_requires_authentication_before_starting_network_io() {
    let provider = NeteaseProvider::new(NeteaseConfig {
        base_url: "http://127.0.0.1:1".to_owned(),
        xeapi_base_url: "http://127.0.0.1:1".to_owned(),
        ..NeteaseConfig::default()
    })
    .unwrap();
    assert_eq!(
        provider
            .recommendation_dislikes(None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert_eq!(
        provider
            .collected_podcast_feed(&CollectedPodcastFeedRequest {
                account: None,
                cursor: None,
                limit: 30
            })
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert_eq!(
        provider
            .podcast_category_tree(Some("default"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
}

#[tokio::test]
#[ignore = "requires live NetEase access"]
async fn live_public_podcast_tree_retains_branch_local_placeholders() {
    let provider = NeteaseProvider::new(NeteaseConfig::default()).unwrap();
    let tree = provider.podcast_category_tree(None).await.unwrap();
    assert!(!tree.categories.is_empty());
    assert!(
        tree.categories
            .iter()
            .any(|branch| branch.children.iter().any(|child| child.id == "000"))
    );
}

#[tokio::test]
#[ignore = "requires a scoped live NetEase test account in NETEASE_COOKIE"]
async fn live_private_discovery_reads_dislikes_and_collected_feed() {
    let cookie = std::env::var("NETEASE_COOKIE").expect("NETEASE_COOKIE must be set");
    let provider = NeteaseProvider::new(NeteaseConfig {
        cookie: Some(cookie),
        ..NeteaseConfig::default()
    })
    .unwrap();
    provider
        .recommendation_dislikes(Some("default"))
        .await
        .unwrap();
    let mut request = CollectedPodcastFeedRequest {
        account: Some("default".to_owned()),
        cursor: None,
        limit: 2,
    };
    let feed = provider.collected_podcast_feed(&request).await.unwrap();
    if let Some(cursor) = feed.next_cursor {
        request.cursor = Some(cursor);
        provider.collected_podcast_feed(&request).await.unwrap();
    }
}
