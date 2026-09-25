use super::*;
use crate::client::artists::tests::info;
use crate::client::similar_artists::{
    INDEX_PATH, MODULE_PATH,
    tests::{index, modules},
};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tuneweave_core::StoredAccountCredential;

fn response(data: Value) -> String {
    let body = json!({"code":"000000","data":data}).to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn frames() -> Vec<String> {
    vec![
        response(info("112", None, None)),
        response(modules(json!([]), json!([]))),
        response(index()),
    ]
}
async fn read(socket: &mut tokio::net::TcpStream) -> String {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut bytes = Vec::new();
        loop {
            let mut buf = [0; 1024];
            let n = socket.read(&mut buf).await.unwrap();
            assert!(n > 0);
            bytes.extend_from_slice(&buf[..n]);
            assert!(bytes.len() < 65536);
            if let Some(end) = bytes.windows(4).position(|v| v == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]);
                let length = headers
                    .lines()
                    .find_map(|l| {
                        l.split_once(':')
                            .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                if bytes.len() >= end + 4 + length {
                    return String::from_utf8(bytes).unwrap();
                }
            }
        }
    })
    .await
    .unwrap()
}
async fn server(frames: Vec<String>) -> (MiguProvider, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        let mut seen = Vec::new();
        for frame in frames {
            let (mut socket, _) =
                tokio::time::timeout(std::time::Duration::from_secs(5), listener.accept())
                    .await
                    .unwrap()
                    .unwrap();
            seen.push(read(&mut socket).await);
            socket.write_all(frame.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
        seen
    });
    (
        MiguProvider::from_client(MiguClient::test_client().with_catalog_test_origin(origin)),
        task,
    )
}
struct NoStore;
impl AccountCredentialStore for NoStore {
    fn load_platform(&self, _: Platform) -> Result<Vec<StoredAccountCredential>> {
        panic!("similar artists read account store")
    }
    fn put(&self, _: &StoredAccountCredential) -> Result<()> {
        panic!("similar artists wrote account store")
    }
    fn remove(&self, _: Platform, _: &str) -> Result<bool> {
        panic!("similar artists removed account store")
    }
}

#[tokio::test]
async fn similar_artists_complete_identity_policy_index_chain_before_local_limit() {
    for limit in [1, 100] {
        let (mut p, seen) = server(frames()).await;
        p.credential_store = Some(Arc::new(NoStore));
        assert!(p.capabilities().contains(&Capability::SimilarArtists));
        let result = p
            .similar_artists("112", &SimilarArtistRequest::new(limit))
            .await
            .unwrap();
        assert_eq!(result.artist_ref.id(), "112");
        assert_eq!(result.requested_limit, limit);
        assert_eq!(result.artists.len(), (limit as usize).min(3));
        assert_eq!(result.extensions["upstream_count"], 3);
        assert_eq!(result.artists[0].id, "266");
        let requests = seen.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[0].starts_with("GET /pc/bmw/singer/info/v1.1?singerId=112 "));
        assert!(requests[1].starts_with(&format!("POST {MODULE_PATH} ")));
        assert_eq!(
            serde_json::from_str::<Value>(requests[1].split_once("\r\n\r\n").unwrap().1).unwrap(),
            json!({"resourceModuleQueryParam":[{"resourceId":"112","resourceType":"2002","responseType":"module"}]})
        );
        assert!(requests[2].starts_with(&format!("GET {INDEX_PATH}?singerId=112 ")));
        for request in requests {
            for header in [
                "cookie:",
                "authorization:",
                "pacmtoken:",
                "token:",
                "uid:",
                "ce:",
            ] {
                assert!(!request.to_lowercase().contains(&format!("\r\n{header}")));
            }
        }
        assert!(p.take_response_credential().unwrap().is_none());
    }
}

#[tokio::test]
async fn similar_artists_invalid_sources_accounts_and_limits_are_rejected_before_io() {
    let (mut p, seen) = server(vec![]).await;
    p.credential_store = Some(Arc::new(NoStore));
    for id in ["", "0", "0112", "112&x=1"] {
        assert_eq!(
            p.similar_artists(id, &SimilarArtistRequest::new(1))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for limit in [0, 101, u32::MAX] {
        assert_eq!(
            p.similar_artists("112", &SimilarArtistRequest::new(limit))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let mut request = SimilarArtistRequest::new(1);
    request.account = Some("A".into());
    assert_eq!(
        p.similar_artists("112", &request).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    let c =
        crate::credential::MiguCredential::verified("111".into(), "private-pacm".into()).unwrap();
    let caller = p.caller_scope(&c.caller().unwrap()).unwrap();
    assert_eq!(
        caller
            .similar_artists("112", &SimilarArtistRequest::new(1))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(seen.await.unwrap().is_empty());
}

#[tokio::test]
async fn similar_artists_policy_denial_identity_and_tail_errors_never_return_partial_or_retry() {
    for (policy, code) in [
        (
            modules(json!([]), json!(["similarSinger"])),
            ErrorCode::PermissionDenied,
        ),
        (
            json!({"resourceModuleQueryList":[]}),
            ErrorCode::UpstreamError,
        ),
        (
            json!({"resourceModuleQueryList":[{"resourceId":"999","resourceType":"2002"}]}),
            ErrorCode::UpstreamError,
        ),
    ] {
        let (p, seen) = server(vec![frames()[0].clone(), response(policy)]).await;
        assert_eq!(
            p.similar_artists("112", &SimilarArtistRequest::new(1))
                .await
                .unwrap_err()
                .code,
            code
        );
        assert_eq!(seen.await.unwrap().len(), 2);
    }
    let mut bad = index();
    bad["contents"][2]["contents"][2]["resType"] = json!("2");
    let (p, seen) = server(vec![
        frames()[0].clone(),
        frames()[1].clone(),
        response(bad),
    ])
    .await;
    assert_eq!(
        p.similar_artists("112", &SimilarArtistRequest::new(1))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(seen.await.unwrap().len(), 3);
    let (p, seen) = server(vec![response(info("999", None, None))]).await;
    assert_eq!(
        p.similar_artists("112", &SimilarArtistRequest::new(1))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(seen.await.unwrap().len(), 1);
    let secret = "private-upstream-value";
    for stage in 0..3 {
        for status in [302, 429, 503] {
            let mut replies = frames()[..stage].to_vec();
            replies.push(format!("HTTP/1.1 {status} Error\r\nContent-Length: {}\r\nLocation: https://outside.invalid/{secret}\r\nSet-Cookie: token={secret}\r\nConnection: close\r\n\r\n{secret}",secret.len()));
            let (p, seen) = server(replies).await;
            let error = p
                .similar_artists("112", &SimilarArtistRequest::new(1))
                .await
                .unwrap_err();
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
            assert_eq!(seen.await.unwrap().len(), stage + 1);
        }
    }
}

#[tokio::test]
async fn similar_artists_policy_and_index_require_bounded_successful_json() {
    for stage in [1, 2] {
        for raw in [
            frames()[stage].replace("application/json", "text/html"),
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 2097153\r\nConnection: close\r\n\r\n".into(),
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1\r\nConnection: close\r\n\r\n{".into(),
            response(json!({})),
        ] {
            let mut replies = frames()[..stage].to_vec();
            replies.push(raw);
            let (p, seen) = server(replies).await;
            assert_eq!(
                p.similar_artists("112", &SimilarArtistRequest::new(1))
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::UpstreamError
            );
            assert_eq!(seen.await.unwrap().len(), stage + 1);
        }
    }
    let mut replies = frames();
    replies[1] = response(modules(json!(["similarSinger"]), json!(["similarSinger"])));
    let mut empty = index();
    empty["contents"][2]["contents"] = json!([]);
    replies[2] = response(empty);
    let (p, seen) = server(replies).await;
    assert!(
        p.similar_artists("112", &SimilarArtistRequest::new(1))
            .await
            .unwrap()
            .artists
            .is_empty()
    );
    assert_eq!(seen.await.unwrap().len(), 3);
}

#[tokio::test]
async fn similar_artists_total_deadline_and_cancellation_cover_policy_and_index_body() {
    for stage in [1, 2] {
        for cancel in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let origin =
                url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
            let p = MiguProvider::from_client(crate::client::charts::tests::long_timeout_client(
                origin,
            ));
            let (tx, rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                for frame in &frames()[..stage] {
                    let (mut socket, _) = listener.accept().await.unwrap();
                    read(&mut socket).await;
                    socket.write_all(frame.as_bytes()).await.unwrap();
                    socket.shutdown().await.unwrap();
                }
                let (mut socket, _) = listener.accept().await.unwrap();
                read(&mut socket).await;
                socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 10000\r\nConnection: close\r\n\r\n{").await.unwrap();
                tx.send(()).unwrap();
                release_rx.await.unwrap();
            });
            let provider = p.clone();
            let task = tokio::spawn(async move {
                provider
                    .similar_artists("112", &SimilarArtistRequest::new(1))
                    .await
            });
            tokio::time::timeout(std::time::Duration::from_secs(5), rx)
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
