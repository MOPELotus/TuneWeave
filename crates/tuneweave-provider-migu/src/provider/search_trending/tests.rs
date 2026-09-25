use super::*;
use crate::client::search_trending::{PATH, tests::reply};
use crate::provider::catalog::tests::server;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tuneweave_core::{SearchTrendingDetail, StoredAccountCredential};

fn request(detail: SearchTrendingDetail) -> SearchTrendingRequest {
    SearchTrendingRequest {
        detail,
        account: None,
    }
}

fn response(value: Value) -> String {
    let body = value.to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

struct NoStore;
impl AccountCredentialStore for NoStore {
    fn load_platform(&self, _: Platform) -> Result<Vec<StoredAccountCredential>> {
        panic!("public hot-word read touched account store")
    }
    fn put(&self, _: &StoredAccountCredential) -> Result<()> {
        panic!("public hot-word read wrote account store")
    }
    fn remove(&self, _: Platform, _: &str) -> Result<bool> {
        panic!("public hot-word read removed account store")
    }
}

#[tokio::test]
async fn pc_search_trending_uses_one_anonymous_fixed_request_in_both_detail_modes() {
    for detail in [SearchTrendingDetail::Brief, SearchTrendingDetail::Full] {
        let (mut provider, seen) = server(vec![response(reply())]).await;
        provider.credential_store = Some(Arc::new(NoStore));
        assert!(
            provider
                .capabilities()
                .contains(&Capability::SearchTrending)
        );
        let result = provider.trending_searches(&request(detail)).await.unwrap();
        assert_eq!(result.detail, detail);
        assert_eq!(result.entries.len(), 4);
        let requests = seen.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(requests[0].starts_with(&format!("GET {PATH} HTTP/1.1\r\n")));
        let lower = requests[0].to_lowercase();
        for header in [
            "cookie:",
            "authorization:",
            "pacmtoken:",
            "token:",
            "uid:",
            "ce:",
            "sign:",
        ] {
            assert!(!lower.contains(&format!("\r\n{header}")));
        }
        assert!(!lower.contains("deviceid"));
        assert!(provider.take_response_credential().unwrap().is_none());
    }
}

#[tokio::test]
async fn pc_search_trending_refuses_selected_and_caller_accounts_before_io() {
    let (mut provider, seen) = server(vec![]).await;
    provider.credential_store = Some(Arc::new(NoStore));
    for account in ["", "default", "A"] {
        let mut r = request(SearchTrendingDetail::Full);
        r.account = Some(account.into());
        assert_eq!(
            provider.trending_searches(&r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    let credential =
        crate::credential::MiguCredential::verified("111".into(), "private-pacm".into()).unwrap();
    let caller = provider
        .caller_scope(&credential.caller().unwrap())
        .unwrap();
    assert_eq!(
        caller
            .trending_searches(&request(SearchTrendingDetail::Brief))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(caller.take_response_credential().unwrap().is_none());
    assert!(seen.await.unwrap().is_empty());
}

#[tokio::test]
async fn pc_search_trending_rejects_business_transport_and_body_failures_without_fallback() {
    let secret = "private-upstream-value";
    let mut cases = vec![
        (response(json!({"code":"111111","info":secret,"data":{"hotWordItemList":[{"word":"Never"}]}})), ErrorCode::UpstreamError),
        (response(json!({"code":"000000","data":{}})), ErrorCode::UpstreamError),
        (response(reply()).replace("application/json; charset=utf-8", "text/html"), ErrorCode::UpstreamError),
        ("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 262145\r\nConnection: close\r\n\r\n".into(), ErrorCode::UpstreamError),
        ("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1\r\nConnection: close\r\n\r\n{".into(), ErrorCode::UpstreamError),
    ];
    for status in [302, 429, 503] {
        cases.push((format!("HTTP/1.1 {status} Error\r\nContent-Length: {}\r\nLocation: https://outside.invalid/{secret}\r\nSet-Cookie: token={secret}\r\nConnection: close\r\n\r\n{secret}",secret.len()), if status==429 {ErrorCode::RateLimited} else {ErrorCode::UpstreamError}));
    }
    for (raw, code) in cases {
        let (provider, seen) = server(vec![raw]).await;
        let error = provider
            .trending_searches(&request(SearchTrendingDetail::Full))
            .await
            .unwrap_err();
        assert_eq!(error.code, code);
        assert!(!error.message.contains(secret));
        assert!(
            !serde_json::to_string(&error.details)
                .unwrap()
                .contains(secret)
        );
        assert!(provider.take_response_credential().unwrap().is_none());
        assert_eq!(seen.await.unwrap().len(), 1);
    }
    let (provider, seen) = server(vec![response(
        json!({"code":"000000","data":{"hotWordItemList":[]}}),
    )])
    .await;
    assert!(
        provider
            .trending_searches(&request(SearchTrendingDetail::Full))
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    assert_eq!(seen.await.unwrap().len(), 1);
}

#[tokio::test]
async fn pc_search_trending_total_deadline_and_cancellation_cover_headers_and_body() {
    for body_boundary in [false, true] {
        for cancel in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin =
                url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
            let p = MiguProvider::from_client(crate::client::charts::tests::long_timeout_client(
                origin,
            ));
            let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
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
                if body_boundary {
                    socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n{").await.unwrap();
                }
                seen_tx.send(()).unwrap();
                release_rx.await.unwrap();
            });
            let provider = p.clone();
            let task = tokio::spawn(async move {
                provider
                    .trending_searches(&request(SearchTrendingDetail::Full))
                    .await
            });
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
