use super::*;
use crate::client::artist_directory::{
    LIST_PATH, TABS_PATH,
    tests::{directory, envelope, singer, tabs},
};
use crate::provider::catalog::tests::server;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tuneweave_core::{ArtistArea, ArtistCategory, ArtistGenre, StoredAccountCredential};

fn response(value: Value) -> String {
    let body = value.to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn request() -> ArtistCatalogRequest {
    ArtistCatalogRequest {
        area: ArtistArea::Chinese,
        category: ArtistCategory::Male,
        ..ArtistCatalogRequest::new()
    }
}

struct NoStore;
impl AccountCredentialStore for NoStore {
    fn load_platform(&self, _: Platform) -> Result<Vec<StoredAccountCredential>> {
        panic!("public artist directory read account credentials")
    }
    fn put(&self, _: &StoredAccountCredential) -> Result<()> {
        panic!("public artist directory wrote account credentials")
    }
    fn remove(&self, _: Platform, _: &str) -> Result<bool> {
        panic!("public artist directory removed account credentials")
    }
}

#[tokio::test]
async fn artist_directory_anonymous_subsets_use_returned_taxonomy_and_fixed_template() {
    for (area, group) in [
        (ArtistArea::Chinese, "huayu"),
        (ArtistArea::Western, "oumei"),
        (ArtistArea::JapaneseKorean, "rihan"),
    ] {
        for (category, subtype) in [
            (ArtistCategory::Male, "nan"),
            (ArtistCategory::Female, "nv"),
            (ArtistCategory::Group, "group"),
        ] {
            let (mut p, requests) = server(vec![response(tabs()), response(directory())]).await;
            p.credential_store = Some(Arc::new(NoStore));
            let r = ArtistCatalogRequest {
                area,
                category,
                ..request()
            };
            let result = p.artist_catalog(&r).await.unwrap();
            assert!(p.capabilities().contains(&Capability::ArtistCatalog));
            assert_eq!(result.area, area);
            assert_eq!(result.category, category);
            assert_eq!(result.genre, ArtistGenre::All);
            assert_eq!(result.featured_artists.len(), 2);
            assert_eq!(result.artists.len(), 2);
            assert_eq!(result.extensions["complete_read"], true);
            assert_eq!(
                result
                    .filters
                    .areas
                    .iter()
                    .map(|option| (option.id.as_str(), option.name.as_str()))
                    .collect::<Vec<_>>(),
                [
                    ("chinese", "华语"),
                    ("western", "欧美"),
                    ("japanese_korean", "日韩"),
                ]
            );
            assert_eq!(
                result.extensions["source_tab"],
                format!("{group}-{subtype}")
            );
            let requests = requests.await.unwrap();
            assert_eq!(requests.len(), 2);
            assert!(requests[0].starts_with(&format!("GET {TABS_PATH} ")));
            assert!(requests[1].starts_with(&format!(
                "GET {LIST_PATH}?tab={group}-{subtype}&templateVersion=3 "
            )));
            for request in requests {
                let request = request.to_lowercase();
                for header in [
                    "cookie:",
                    "authorization:",
                    "pacmtoken:",
                    "token:",
                    "uid:",
                    "ce:",
                    "sign:",
                ] {
                    assert!(
                        !request.contains(&format!("\r\n{header}")),
                        "unexpected {header}"
                    );
                }
                assert!(!request.contains("deviceid"));
            }
            assert!(p.take_response_credential().unwrap().is_none());
        }
    }
}

#[tokio::test]
async fn artist_directory_unsupported_defaults_and_accounts_fail_before_network() {
    let (mut p, requests) = server(vec![]).await;
    p.credential_store = Some(Arc::new(NoStore));
    for area in [
        ArtistArea::All,
        ArtistArea::Japanese,
        ArtistArea::Korean,
        ArtistArea::HongKongTaiwan,
        ArtistArea::Other,
    ] {
        let r = ArtistCatalogRequest { area, ..request() };
        assert_eq!(
            p.artist_catalog(&r).await.unwrap_err().code,
            ErrorCode::CapabilityNotSupported
        );
    }
    assert_eq!(
        p.artist_catalog(&ArtistCatalogRequest::new())
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    let r = ArtistCatalogRequest {
        category: ArtistCategory::All,
        ..request()
    };
    assert_eq!(
        p.artist_catalog(&r).await.unwrap_err().code,
        ErrorCode::CapabilityNotSupported
    );
    for genre in [ArtistGenre::Pop, ArtistGenre::Rock, ArtistGenre::Folk] {
        let r = ArtistCatalogRequest { genre, ..request() };
        assert_eq!(
            p.artist_catalog(&r).await.unwrap_err().code,
            ErrorCode::CapabilityNotSupported
        );
    }
    for account in ["", "default", "A"] {
        let r = ArtistCatalogRequest {
            account: Some(account.into()),
            ..request()
        };
        assert_eq!(
            p.artist_catalog(&r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    let credential =
        crate::credential::MiguCredential::verified("111".into(), "private-pacm".into()).unwrap();
    let caller = p.caller_scope(&credential.caller().unwrap()).unwrap();
    assert_eq!(
        caller.artist_catalog(&request()).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert!(caller.take_response_credential().unwrap().is_none());
    assert!(requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn artist_directory_changed_taxonomy_stops_before_list_without_guessing_or_retry() {
    for mutation in 0..3 {
        let mut value = tabs();
        match mutation {
            0 => value["data"]["contents"][0]["contents"][0]["txt"] = json!("欧美"),
            1 => value["data"]["contents"][1]["contents"][0]["action"] = json!("other"),
            _ => value["code"] = json!("private-upstream-error"),
        }
        let (p, requests) = server(vec![response(value)]).await;
        let error = p.artist_catalog(&request()).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!error.message.contains("private-upstream"));
        assert_eq!(requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn artist_directory_japanese_korean_requires_the_exact_combined_taxonomy() {
    // The archived tabs prove the combined region and all three subtype keys.
    // These are synthetic responses, not captured rihan live list results.
    for mutation in 0..6 {
        let mut value = tabs();
        match mutation {
            0 => {
                value["data"]["contents"]
                    .as_array_mut()
                    .unwrap()
                    .truncate(4);
            }
            1 => value["data"]["contents"][4]["contents"][0]["txt"] = json!("日本"),
            2 => value["data"]["contents"][4]["contents"][0]["txt2"] = json!("japan"),
            3 => {
                value["data"]["contents"][5]["contents"]
                    .as_array_mut()
                    .unwrap()
                    .remove(1);
            }
            4 => value["data"]["contents"][5]["contents"][1]["txt"] = json!("男"),
            _ => value["data"]["contents"][5]["contents"][1]["action"] = json!("nv&uid=private"),
        }
        let (p, requests) = server(vec![response(value)]).await;
        let r = ArtistCatalogRequest {
            area: ArtistArea::JapaneseKorean,
            category: ArtistCategory::Female,
            ..request()
        };
        let error = p.artist_catalog(&r).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError, "mutation {mutation}");
        assert!(!error.message.contains("private"));
        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with(&format!("GET {TABS_PATH} ")));
    }
}

#[tokio::test]
async fn artist_directory_japanese_korean_keeps_account_and_partial_read_boundaries() {
    let r = ArtistCatalogRequest {
        area: ArtistArea::JapaneseKorean,
        category: ArtistCategory::Group,
        ..request()
    };
    let (mut p, requests) = server(vec![]).await;
    p.credential_store = Some(Arc::new(NoStore));
    for account in ["", "default", "A"] {
        let mut selected = r.clone();
        selected.account = Some(account.into());
        assert_eq!(
            p.artist_catalog(&selected).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    let credential =
        crate::credential::MiguCredential::verified("111".into(), "private-pacm".into()).unwrap();
    let caller = p.caller_scope(&credential.caller().unwrap()).unwrap();
    assert_eq!(
        caller.artist_catalog(&r).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert!(caller.take_response_credential().unwrap().is_none());
    let mut unsupported = r.clone();
    unsupported.category = ArtistCategory::All;
    assert_eq!(
        p.artist_catalog(&unsupported).await.unwrap_err().code,
        ErrorCode::CapabilityNotSupported
    );
    unsupported = r.clone();
    unsupported.genre = ArtistGenre::Pop;
    assert_eq!(
        p.artist_catalog(&unsupported).await.unwrap_err().code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(requests.await.unwrap().is_empty());

    for mutation in 0..4 {
        let mut value = directory();
        match mutation {
            0 => value["data"]["contents"][3]["resId"] = json!("999"),
            1 => value["data"]["contents"][3] = value["data"]["contents"][2].clone(),
            2 => value["data"]["header"]["hasNext"] = json!(true),
            _ => value["code"] = json!("private-upstream-error"),
        }
        let (p, requests) = server(vec![response(tabs()), response(value)]).await;
        let error = p.artist_catalog(&r).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError, "mutation {mutation}");
        assert!(!error.message.contains("private"));
        assert_eq!(requests.await.unwrap().len(), 2);
    }
    let (p, requests) = server(vec![response(tabs()), response(envelope(vec![]))]).await;
    let result = p.artist_catalog(&r).await.unwrap();
    assert_eq!(result.area, ArtistArea::JapaneseKorean);
    assert!(result.artists.is_empty() && result.featured_artists.is_empty());
    assert_eq!(result.extensions["source_tab"], "rihan-group");
    assert_eq!(result.extensions["complete_read"], true);
    assert_eq!(requests.await.unwrap().len(), 2);
}

#[tokio::test]
async fn artist_directory_complete_view_rejects_bad_tail_and_returns_explicit_empty() {
    for mutation in 0..4 {
        let mut value = directory();
        match mutation {
            0 => value["data"]["contents"][3]["resId"] = json!("999"),
            1 => value["data"]["contents"][3] = value["data"]["contents"][2].clone(),
            2 => {
                value["data"]["header"]["nextPageUrl"] =
                    json!("https://outside.invalid/?secret=value")
            }
            _ => value["data"]["contents"][3]["txt2"] = json!(123),
        }
        let (p, requests) = server(vec![response(tabs()), response(value)]).await;
        assert_eq!(
            p.artist_catalog(&request()).await.unwrap_err().code,
            ErrorCode::UpstreamError
        );
        assert_eq!(requests.await.unwrap().len(), 2);
    }
    let (p, requests) = server(vec![response(tabs()), response(envelope(vec![]))]).await;
    let result = p.artist_catalog(&request()).await.unwrap();
    assert!(result.artists.is_empty() && result.featured_artists.is_empty());
    assert_eq!(result.extensions["complete_read"], true);
    assert_eq!(requests.await.unwrap().len(), 2);
    let many = envelope((1..=1400).map(|id| singer(&id.to_string(), "A")).collect());
    let (p, requests) = server(vec![response(tabs()), response(many)]).await;
    assert_eq!(
        p.artist_catalog(&request()).await.unwrap().artists.len(),
        1400
    );
    assert_eq!(requests.await.unwrap().len(), 2);
}

#[tokio::test]
async fn artist_directory_transport_errors_are_bounded_private_and_never_redirected() {
    let secret = "private-token-value";
    for status in [429, 503, 302] {
        let reply = format!(
            "HTTP/1.1 {status} Error\r\nContent-Length: {}\r\nLocation: https://outside.invalid/?{secret}\r\nSet-Cookie: secret={secret}\r\nConnection: close\r\n\r\n{secret}",
            secret.len()
        );
        let (p, requests) = server(vec![response(tabs()), reply]).await;
        let error = p.artist_catalog(&request()).await.unwrap_err();
        assert_eq!(
            error.code,
            if status == 429 {
                ErrorCode::RateLimited
            } else {
                ErrorCode::UpstreamError
            }
        );
        assert!(!error.message.contains(secret));
        assert!(
            !serde_json::to_string(&error.details)
                .unwrap()
                .contains(secret)
        );
        assert!(p.take_response_credential().unwrap().is_none());
        assert_eq!(requests.await.unwrap().len(), 2);
    }
    for reply in [
        response(directory()).replace("application/json; charset=utf-8", "text/html"),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2097153\r\nConnection: close\r\n\r\n".into(),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{".into(),
    ] {
        let (p, requests) = server(vec![response(tabs()), reply]).await;
        assert_eq!(p.artist_catalog(&request()).await.unwrap_err().code, ErrorCode::UpstreamError);
        assert_eq!(requests.await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn artist_directory_total_deadline_and_cancellation_cover_both_response_boundaries() {
    for stage in 0..2 {
        for body_boundary in [false, true] {
            for cancel in [false, true] {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let origin =
                    url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap()))
                        .unwrap();
                let p = MiguProvider::from_client(
                    crate::client::charts::tests::long_timeout_client(origin),
                );
                let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
                let (release_tx, release_rx) = tokio::sync::oneshot::channel();
                let server = tokio::spawn(async move {
                    for current in 0..=stage {
                        let (mut socket, _) = listener.accept().await.unwrap();
                        let mut bytes = Vec::new();
                        loop {
                            let mut buf = [0; 1024];
                            let n = socket.read(&mut buf).await.unwrap();
                            assert!(n > 0);
                            bytes.extend_from_slice(&buf[..n]);
                            assert!(bytes.len() < 65536);
                            if bytes.windows(4).any(|v| v == b"\r\n\r\n") {
                                break;
                            }
                        }
                        if current != stage {
                            socket.write_all(response(tabs()).as_bytes()).await.unwrap();
                            socket.shutdown().await.unwrap();
                        } else {
                            if body_boundary {
                                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{").await.unwrap();
                            }
                            seen_tx.send(()).unwrap();
                            release_rx.await.unwrap();
                            break;
                        }
                    }
                });
                let provider = p.clone();
                let task = tokio::spawn(async move { provider.artist_catalog(&request()).await });
                tokio::time::timeout(std::time::Duration::from_secs(5), seen_rx)
                    .await
                    .unwrap()
                    .unwrap();
                if cancel {
                    task.abort();
                    assert!(task.await.unwrap_err().is_cancelled());
                } else {
                    tokio::time::pause();
                    let result = task.await;
                    tokio::time::resume();
                    let error = result.unwrap().unwrap_err();
                    assert_eq!(error.code, ErrorCode::UpstreamTimeout);
                    assert!(error.message.contains("total time budget"));
                }
                release_tx.send(()).unwrap();
                server.await.unwrap();
                assert!(p.take_response_credential().unwrap().is_none());
            }
        }
    }
}
