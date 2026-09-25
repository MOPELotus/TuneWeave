use super::session::tests::{Store, credential, raw, server};
use super::*;
use crate::client::search_default::tests::payload;
use std::{
    collections::BTreeMap,
    sync::atomic::{AtomicUsize, Ordering},
};
use tuneweave_core::{SearchDefaultKeywordRequest, StoredAccountCredential};

#[derive(Default)]
struct ObservedStore {
    inner: Store,
    reads: AtomicUsize,
    writes: AtomicUsize,
}

impl AccountCredentialStore for ObservedStore {
    fn load_platform(&self, platform: Platform) -> Result<Vec<StoredAccountCredential>> {
        self.reads.fetch_add(1, Ordering::SeqCst);
        self.inner.load_platform(platform)
    }
    fn put(&self, value: &StoredAccountCredential) -> Result<()> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.put(value)
    }
    fn remove(&self, platform: Platform, account: &str) -> Result<bool> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.remove(platform, account)
    }
    fn insert_if_absent(&self, value: &StoredAccountCredential) -> Result<bool> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.insert_if_absent(value)
    }
    fn compare_exchange(
        &self,
        old: &StoredAccountCredential,
        next: Option<&StoredAccountCredential>,
    ) -> Result<bool> {
        self.writes.fetch_add(1, Ordering::SeqCst);
        self.inner.compare_exchange(old, next)
    }
}

fn request() -> SearchDefaultKeywordRequest {
    SearchDefaultKeywordRequest { account: None }
}

#[tokio::test]
async fn search_default_sdk_and_provider_sign_anonymous_web_requests_without_account_state() {
    let frame = raw(payload());
    let mut f = server(vec![
        frame.clone().into(),
        frame.clone().into(),
        frame.into(),
    ])
    .await;
    let store = Arc::new(ObservedStore::default());
    let original = credential("999", "stored-account-secret")
        .stored("default")
        .unwrap();
    store.inner.put(&original).unwrap();
    f.provider.credential_store = Some(store.clone());
    let sdk = f
        .provider
        .client
        .default_search_keyword(&request())
        .await
        .unwrap();
    let provider = f.provider.default_search_keyword(&request()).await.unwrap();
    assert_eq!(sdk, provider);
    let default = f
        .provider
        .default_search_keyword(&SearchDefaultKeywordRequest {
            account: Some("default".into()),
        })
        .await
        .unwrap();
    assert_eq!(provider, default);
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 3);
    for wire in requests {
        let target = wire
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        let url = url::Url::parse(&format!("http://fixture{target}")).unwrap();
        let mut query: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        assert!(wire.starts_with("POST "));
        assert_eq!(url.path(), "/ads.gateway/v1/search_no_focus_word");
        assert_eq!(query.len(), 8);
        assert_eq!(query["appid"], "1014");
        assert_eq!(query["srcappid"], "2919");
        assert_eq!(query["clientver"], "1000");
        assert_eq!(query["dfid"], "-");
        assert_eq!(query["mid"].len(), 32);
        assert!(query["mid"].bytes().all(|b| b.is_ascii_hexdigit()));
        let millis: u64 = query["uuid"].parse().unwrap();
        assert_eq!(query["clienttime"], ((millis + 500) / 1000).to_string());
        let signature = query.remove("signature").unwrap();
        let body = wire.split_once("\r\n\r\n").unwrap().1;
        assert_eq!(
            body,
            r#"{"userid":0,"plat":103,"m_type":0,"vip_type":0,"own_ads":{}}"#
        );
        assert_eq!(
            signature,
            crate::signing::web_signature(
                &query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect(),
                body.as_bytes()
            )
            .to_ascii_lowercase()
        );
        let lower = wire.to_ascii_lowercase();
        assert!(lower.contains("referer: https://www.kugou.com/\r\n"));
        assert!(
            lower.contains("content-type: application/x-www-form-urlencoded; charset=utf-8\r\n")
        );
        for forbidden in [
            "cookie:",
            "authorization:",
            "token=",
            "stored-account-secret",
        ] {
            assert!(!lower.contains(forbidden));
        }
    }
    assert_eq!(store.reads.load(Ordering::SeqCst), 0);
    assert_eq!(store.writes.load(Ordering::SeqCst), 0);
    assert_eq!(
        store.inner.values.lock().unwrap().get("default"),
        Some(&original)
    );
}

#[tokio::test]
async fn search_default_loopback_distinguishes_empty_success_and_business_failure() {
    for (value, expected) in [
        (
            json!({"status":1,"error_code":0,"data":{"timestamp":1790166855,"ads":[]}}),
            ErrorCode::ResourceNotFound,
        ),
        (
            json!({"status":0,"error_code":20001,"msg":"private-upstream-message"}),
            ErrorCode::UpstreamError,
        ),
        (
            json!({"status":1,"error_code":0,"data":{"ads":null}}),
            ErrorCode::UpstreamError,
        ),
    ] {
        let f = server(vec![raw(value).into()]).await;
        let error = f
            .provider
            .default_search_keyword(&request())
            .await
            .unwrap_err();
        assert_eq!(error.code, expected);
        assert!(!format!("{error:?}").contains("private-upstream-message"));
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn search_default_rejects_named_accounts_and_caller_credentials_before_network() {
    let f = server(vec![]).await;
    for account in ["", "default", "Other"] {
        let input = SearchDefaultKeywordRequest {
            account: Some(account.into()),
        };
        assert_eq!(
            f.provider
                .client
                .default_search_keyword(&input)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        if account != "default" {
            assert_eq!(
                f.provider
                    .default_search_keyword(&input)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidRequest
            );
        }
    }
    let caller = f
        .provider
        .caller_scope(&credential("111", "caller-secret").caller().unwrap())
        .unwrap();
    for account in [None, Some("default".into())] {
        assert_eq!(
            caller
                .default_search_keyword(&SearchDefaultKeywordRequest { account })
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn search_default_rejects_http_verification_redirects_and_oversized_bodies() {
    let valid = raw(payload());
    let oversized = " ".repeat(131073);
    for (frame, expected) in [
        (valid.replace("application/json", "text/html"), ErrorCode::UpstreamError),
        (valid.replace("Connection: close", "SSA-CODE: fixture\r\nConnection: close"), ErrorCode::PermissionDenied),
        ("HTTP/1.1 302 Found\r\nLocation: https://unused.invalid/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(), ErrorCode::UpstreamError),
        ("HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(), ErrorCode::RateLimited),
        ("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 131073\r\nConnection: close\r\n\r\n".into(), ErrorCode::UpstreamError),
        (format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{oversized}\r\n0\r\n\r\n", oversized.len()), ErrorCode::UpstreamError),
    ] {
        let f = server(vec![frame.into()]).await;
        assert_eq!(f.provider.default_search_keyword(&request()).await.unwrap_err().code, expected);
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
}
