use super::*;
use crate::client::search_suggestions::{PATH, tests::reply};
use crate::provider::catalog::tests::server;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tuneweave_core::StoredAccountCredential;

fn request(query: &str) -> SearchSuggestionRequest {
    SearchSuggestionRequest {
        query: query.into(),
        client: SearchSuggestionClient::Pc,
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
        panic!("public suggestion touched account store")
    }
    fn put(&self, _: &StoredAccountCredential) -> Result<()> {
        panic!("public suggestion wrote account store")
    }
    fn remove(&self, _: Platform, _: &str) -> Result<bool> {
        panic!("public suggestion removed account store")
    }
}

#[tokio::test]
async fn pc_search_suggestions_use_exact_official_query_vectors_without_credentials() {
    // Independent vectors execute the extracted official Ut JS and WHATWG URL
    // serializer offline; '#'/'&' have two escapes, spaces use %20, not '+'.
    for (query, wire) in [
        ("周杰伦", "%E5%91%A8%E6%9D%B0%E4%BC%A6"),
        (
            " A&B # 中文?=x / ' \" ",
            "A%2526B%20%2523%20%E4%B8%AD%E6%96%87?=x%20/%20%27%20%22",
        ),
        ("A=B?C", "A=B?C"),
    ] {
        let (mut p, seen) = server(vec![response(reply())]).await;
        p.credential_store = Some(Arc::new(NoStore));
        assert!(p.capabilities().contains(&Capability::SearchSuggestions));
        let result = p.search_suggestions(&request(query)).await.unwrap();
        assert_eq!(result.query, query.trim());
        assert_eq!(result.suggestions.len(), 5);
        let requests = seen.await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(
            requests[0].starts_with(&format!("GET {PATH}?text={wire} ")),
            "{}",
            requests[0].lines().next().unwrap()
        );
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
        assert!(p.take_response_credential().unwrap().is_none());
    }
}

#[tokio::test]
async fn pc_search_suggestions_refuse_unproved_clients_queries_and_account_sources_before_io() {
    let (mut p, seen) = server(vec![]).await;
    p.credential_store = Some(Arc::new(NoStore));
    for query in ["", " ", "A+B", "A%26B", "A\nB", &"a".repeat(1025)] {
        assert_eq!(
            p.search_suggestions(&request(query))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for client in [SearchSuggestionClient::Web, SearchSuggestionClient::Mobile] {
        let mut r = request("query");
        r.client = client;
        assert_eq!(
            p.search_suggestions(&r).await.unwrap_err().code,
            ErrorCode::CapabilityNotSupported
        );
    }
    for account in ["", "default", "A"] {
        let mut r = request("query");
        r.account = Some(account.into());
        assert_eq!(
            p.search_suggestions(&r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    let credential =
        crate::credential::MiguCredential::verified("111".into(), "private-pacm".into()).unwrap();
    let caller = p.caller_scope(&credential.caller().unwrap()).unwrap();
    assert_eq!(
        caller
            .search_suggestions(&request("query"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(caller.take_response_credential().unwrap().is_none());
    assert!(seen.await.unwrap().is_empty());
}

#[tokio::test]
async fn pc_search_suggestions_business_failure_never_falls_back_to_hot_or_empty_results() {
    for value in [
        json!({"code":"111111","info":"private-error"}),
        json!({"code":"000000","data":{"singerList":[{"singerName":"ok"}],"songList":[{}]}}),
    ] {
        let (p, seen) = server(vec![response(value)]).await;
        let error = p.search_suggestions(&request("query")).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!error.message.contains("private-error"));
        assert_eq!(seen.await.unwrap().len(), 1);
    }
    let (p, seen) = server(vec![response(
        json!({"code":"000000","data":{"songList":[]}}),
    )])
    .await;
    let result = p.search_suggestions(&request("query")).await.unwrap();
    assert!(result.suggestions.is_empty() && result.recommendations.is_empty());
    assert_eq!(seen.await.unwrap().len(), 1);
}

#[tokio::test]
async fn pc_search_suggestions_bound_transport_and_do_not_reflect_or_follow_error_responses() {
    let secret = "private-upstream-value";
    for status in [429, 503, 302] {
        let raw = format!(
            "HTTP/1.1 {status} Error\r\nContent-Length: {}\r\nLocation: https://outside.invalid/{secret}\r\nSet-Cookie: token={secret}\r\nConnection: close\r\n\r\n{secret}",
            secret.len()
        );
        let (p, seen) = server(vec![raw]).await;
        let error = p.search_suggestions(&request("query")).await.unwrap_err();
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
        assert_eq!(seen.await.unwrap().len(), 1);
    }
    for raw in [
        response(reply()).replace("application/json; charset=utf-8", "text/html"),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1048577\r\nConnection: close\r\n\r\n".into(),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1\r\nConnection: close\r\n\r\n{".into(),
    ] {
        let (p, seen) = server(vec![raw]).await;
        assert_eq!(p.search_suggestions(&request("query")).await.unwrap_err().code, ErrorCode::UpstreamError);
        assert_eq!(seen.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn pc_search_suggestions_total_deadline_and_cancellation_cover_headers_and_body() {
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
            let task =
                tokio::spawn(async move { provider.search_suggestions(&request("query")).await });
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
