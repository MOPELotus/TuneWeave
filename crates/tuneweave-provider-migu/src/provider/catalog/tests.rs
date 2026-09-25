use super::*;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tuneweave_core::{ErrorCode, ProviderCredential, SearchSelector, VideoSearchFilters};

fn item(kind: SearchKind, id: u32) -> Value {
    match kind {
        SearchKind::Playlist => {
            json!({"musicList":{"resourceType":"2021","musicListId":id.to_string(),"title":format!("Playlist {id}"),"musicNum":id}})
        }
        SearchKind::Artist => {
            json!({"singer":{"resourceType":"2002","singerId":id.to_string(),"singer":format!("Artist {id}"),"songNum":id}})
        }
        _ => unreachable!(),
    }
}
fn page(kind: SearchKind, start: u32, count: u32, more: bool) -> Value {
    json!({"code":"000000","data":{"hasNext":more,"seq":format!("page-{start}"),"items":(start..start+count).map(|id|item(kind,id)).collect::<Vec<_>>()}})
}
fn query(kind: SearchKind, limit: u32, offset: u32) -> SearchQuery {
    SearchQuery {
        kind,
        ..SearchQuery::tracks(" A&B / 中文?=x ", limit, offset)
    }
}
fn id(item: &SearchItem) -> &str {
    match item {
        SearchItem::Playlist(v) => &v.id,
        SearchItem::Artist(v) => &v.id,
        _ => panic!("unexpected result type"),
    }
}
fn response(value: Value) -> String {
    let body = value.to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
pub(in crate::provider) async fn server(
    responses: Vec<String>,
) -> (MiguProvider, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let request = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut bytes = Vec::new();
                loop {
                    let mut buffer = [0; 1024];
                    let count = socket.read(&mut buffer).await.unwrap();
                    assert!(count > 0);
                    bytes.extend_from_slice(&buffer[..count]);
                    assert!(bytes.len() <= 65536);
                    if bytes.windows(4).any(|s| s == b"\r\n\r\n") {
                        break;
                    }
                }
                socket.write_all(response.as_bytes()).await.unwrap();
                socket.shutdown().await.unwrap();
                String::from_utf8(bytes).unwrap()
            })
            .await
            .unwrap();
            requests.push(request);
        }
        requests
    });
    (
        MiguProvider::from_client(MiguClient::test_client().with_catalog_test_origin(origin)),
        task,
    )
}

#[tokio::test]
async fn catalogue_search_slices_across_physical_pages_and_encodes_only_supported_inputs() {
    for kind in [SearchKind::Playlist, SearchKind::Artist] {
        for (offset, limit, total) in [
            (0_u32, 1_u32, 45_u32),
            (17, 25, 45),
            (19, 100, 140),
            (40, 20, 45),
            (44, 20, 45),
            (60, 20, 45),
        ] {
            let first = offset / 20;
            let budget = ((offset % 20 + limit) as usize).div_ceil(20);
            let mut replies = Vec::new();
            for index in first..first + budget as u32 {
                let start = index * 20;
                let count = total.saturating_sub(start).min(20);
                replies.push(response(page(
                    kind,
                    start + 1,
                    count,
                    start + count < total,
                )));
                if start + count >= total {
                    break;
                }
            }
            let expected_requests = replies.len();
            let (provider, requests) = server(replies).await;
            let result = provider
                .search_catalog(&query(kind, limit, offset))
                .await
                .unwrap();
            let expected: Vec<_> = (offset..offset.saturating_add(limit).min(total))
                .map(|v| (v + 1).to_string())
                .collect();
            assert_eq!(
                result.items.iter().map(id).collect::<Vec<_>>(),
                expected.iter().map(String::as_str).collect::<Vec<_>>()
            );
            assert_eq!(result.pagination.total, None);
            assert_eq!(result.pagination.offset, offset);
            assert_eq!(result.pagination.limit, limit);
            assert_eq!(
                result.pagination.has_more,
                offset + (result.items.len() as u32) < total
            );
            assert_eq!(
                result.pagination.next_offset,
                if result.pagination.has_more {
                    Some(offset + result.items.len() as u32)
                } else {
                    None
                }
            );
            assert_eq!(
                result.pagination.extensions["upstream_pages_fetched"],
                expected_requests
            );
            assert!(expected_requests <= 6);
            let requests = requests.await.unwrap();
            assert_eq!(requests.len(), expected_requests);
            for (index, request) in requests.iter().enumerate() {
                let target = request
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                let url = url::Url::parse(&format!("https://app.c.nf.migu.cn{target}")).unwrap();
                let params: std::collections::BTreeMap<_, _> =
                    url.query_pairs().into_owned().collect();
                assert_eq!(
                    url.path(),
                    if kind == SearchKind::Artist {
                        "/bmw/search/singer/v2.0"
                    } else {
                        "/bmw/search/music-list/v1.0"
                    }
                );
                assert_eq!(params["pageNo"], (first + index as u32 + 1).to_string());
                assert_eq!(params["text"], "A&B / 中文?=x");
                assert_eq!(params.len(), if kind == SearchKind::Artist { 2 } else { 3 });
                if kind == SearchKind::Playlist {
                    assert_eq!(params["typeOrder"], "0");
                }
                let lower = request.to_ascii_lowercase();
                assert!(!lower.contains("cookie:"));
                assert!(!lower.contains("authorization:"));
                assert!(lower.contains("accept: application/json"));
            }
        }
    }
}

#[tokio::test]
async fn catalogue_search_rejects_repeated_or_incomplete_pages_without_returning_a_prefix() {
    for kind in [SearchKind::Playlist, SearchKind::Artist] {
        for failure in [
            "repeat",
            "short",
            "empty_more",
            "missing_end",
            "wrong_kind",
            "business",
            "transport",
        ] {
            let second = match failure {
                "repeat" => response(page(kind, 1, 20, true)),
                "short" => response(page(kind, 21, 19, true)),
                "empty_more" => response(page(kind, 21, 0, true)),
                "missing_end" => response(json!({"code":"000000","data":{"items":[]}})),
                "wrong_kind" => response(page(
                    if kind == SearchKind::Artist {
                        SearchKind::Playlist
                    } else {
                        SearchKind::Artist
                    },
                    21,
                    20,
                    false,
                )),
                "business" => response(
                    json!({"code":"123456","data":{"hasNext":false},"info":"not exported"}),
                ),
                "transport" => {
                    "HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .to_owned()
                }
                _ => unreachable!(),
            };
            let (provider, requests) =
                server(vec![response(page(kind, 1, 20, true)), second]).await;
            let error = provider
                .search_catalog(&query(kind, 40, 0))
                .await
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::UpstreamError, "{failure}");
            assert!(!format!("{error:?}").contains("not exported"));
            assert_eq!(requests.await.unwrap().len(), 2);
        }
    }
}

#[tokio::test]
async fn catalogue_search_retains_positions_and_accepts_explicit_empty_end_pages() {
    for kind in [SearchKind::Playlist, SearchKind::Artist] {
        let mut value = page(kind, 1, 3, false);
        value["data"]["items"][2] = item(kind, 1);
        let (provider, requests) = server(vec![response(value)]).await;
        let page = provider.search_catalog(&query(kind, 20, 0)).await.unwrap();
        assert_eq!(
            page.items.iter().map(id).collect::<Vec<_>>(),
            ["1", "2", "1"]
        );
        assert_eq!(requests.await.unwrap().len(), 1);
        let (provider, requests) = server(vec![response(
            json!({"code":"000000","data":{"hasNext":false,"seq":"end"}}),
        )])
        .await;
        let page = provider.search_catalog(&query(kind, 20, 40)).await.unwrap();
        assert!(page.items.is_empty());
        assert!(!page.pagination.has_more);
        assert!(page.pagination.next_offset.is_none());
        assert_eq!(page.pagination.total, None);
        assert_eq!(requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn catalogue_transport_preserves_status_errors_and_rejects_redirects_or_non_json() {
    for (reply,expected) in [
        ("HTTP/1.1 429 Too Many Requests\r\nContent-Type: text/html\r\nContent-Length: 0\r\n\r\n".to_owned(),ErrorCode::RateLimited),
        ("HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/never\r\nContent-Length: 0\r\n\r\n".to_owned(),ErrorCode::UpstreamError),
        (response(page(SearchKind::Playlist,1,1,false)).replace("application/json; charset=utf-8","text/html"),ErrorCode::UpstreamError),
        ("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 8388609\r\nConnection: close\r\n\r\n".to_owned(),ErrorCode::UpstreamError),
    ] {
        let (provider,requests)=server(vec![reply]).await;let error=provider.search_catalog(&query(SearchKind::Playlist,20,0)).await.unwrap_err();assert_eq!(error.code,expected);assert_eq!(requests.await.unwrap().len(),1);
    }
}

#[tokio::test]
async fn catalogue_search_rejects_accounts_and_invalid_options_before_network() {
    let provider = MiguProvider::new(MiguConfig::default()).unwrap();
    for kind in [SearchKind::Playlist, SearchKind::Artist] {
        for mutation in 0..12 {
            let mut query = query(kind, 20, 0);
            match mutation {
                0 => query.account = Some("personal".to_owned()),
                1 => query.variant = SearchVariant::Legacy,
                2 => query.highlight = true,
                3 => query.search_id = Some("old".to_owned()),
                4 => query.selectors.push(SearchSelector {
                    id: 1,
                    name: "test".to_owned(),
                    selector_type: 1,
                    extensions: Extensions::new(),
                }),
                5 => query.video_filters = Some(VideoSearchFilters::default()),
                6 => query.limit = 0,
                7 => query.limit = 101,
                8 => query.offset = u32::MAX,
                9 => query.query = " ".to_owned(),
                10 => query.query = "x".repeat(513),
                11 => query.query = "query\n".to_owned() + "another",
                _ => unreachable!(),
            }
            assert_eq!(
                provider.search_catalog(&query).await.unwrap_err().code,
                ErrorCode::InvalidRequest,
                "{mutation}"
            );
        }
        assert_eq!(
            provider.search(&query(kind, 20, 0)).await.unwrap_err().code,
            ErrorCode::CapabilityNotSupported
        );
    }
    for kind in [SearchKind::Mixed, SearchKind::User] {
        assert_eq!(
            provider
                .search_catalog(&query(kind, 20, 0))
                .await
                .unwrap_err()
                .code,
            ErrorCode::CapabilityNotSupported
        );
    }
    let credential =
        ProviderCredential::new(Platform::Migu, "test", "caller-secret", None).unwrap();
    assert!(provider.with_caller_credential(&credential).is_err());
}

#[tokio::test]
#[ignore = "requires public Migu HTTPS endpoints; no account or audio"]
async fn live_public_catalog_search_checks_paging_and_playlist_identity() {
    let provider = MiguProvider::new(MiguConfig::default()).unwrap();
    let mut request = query(SearchKind::Playlist, 25, 17);
    request.query = "周杰伦".to_owned();
    let page = provider.search_catalog(&request).await.unwrap();
    assert_eq!(page.items.len(), 25);
    assert_eq!(page.pagination.offset, 17);
    assert_eq!(page.pagination.next_offset, Some(42));
    assert_eq!(page.pagination.extensions["upstream_pages_fetched"], 3);
    let SearchItem::Playlist(found) = &page.items[0] else {
        panic!("playlist expected")
    };
    let detail = provider.playlist(&found.id, None).await.unwrap();
    assert_eq!(found.resource_ref, detail.resource_ref);
    assert_eq!(found.name, detail.name);
    request.kind = SearchKind::Artist;
    request.offset = 0;
    request.limit = 100;
    let page = provider.search_catalog(&request).await.unwrap();
    assert!(!page.items.is_empty());
    assert!(
        page.items
            .iter()
            .all(|v| matches!(v, SearchItem::Artist(_)))
    );
    assert!(page.items.iter().any(|v| id(v) == "112"));
    assert!(page.pagination.total.is_none());
}
