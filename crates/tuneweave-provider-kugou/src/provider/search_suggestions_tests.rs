use super::session::tests::{Store, credential, server};
use super::*;
use crate::client::search_suggestions::tests::{jsonp, payload};
use std::collections::BTreeMap;
use tuneweave_core::{SearchSuggestionClient, SearchSuggestionRequest};

fn request(query: &str) -> SearchSuggestionRequest {
    SearchSuggestionRequest {
        query: query.into(),
        client: SearchSuggestionClient::Web,
        account: None,
    }
}

fn reply(body: &str, mime: &str, extra: &str) -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",
        body.len()
    )
}

#[tokio::test]
async fn search_suggestions_sdk_and_provider_use_the_anonymous_header_request() {
    let frame = reply(&jsonp(&payload()), "text/plain; charset=utf-8", "");
    let mut f = server(vec![frame.clone().into(), frame.into()]).await;
    let store = Arc::new(Store::default());
    let original = credential("999", "stored-account-secret")
        .stored("default")
        .unwrap();
    store.put(&original).unwrap();
    f.provider.credential_store = Some(store.clone());
    let query = " A <B> 'C' \"D\" & % + 歌曲 ";
    let sdk = f
        .provider
        .client
        .search_suggestions(&request(query))
        .await
        .unwrap();
    let provider = f
        .provider
        .search_suggestions(&request(query))
        .await
        .unwrap();
    assert_eq!(sdk, provider);
    assert_eq!(provider.query, query.trim());
    assert_eq!(provider.suggestions.len(), 4);
    for wire in f.requests.await.unwrap() {
        let target = wire
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap();
        let url = url::Url::parse(&format!("http://fixture{target}")).unwrap();
        let params: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        assert!(wire.starts_with("GET "));
        assert_eq!(url.path(), "/getSearchTip");
        assert_eq!(
            params,
            BTreeMap::from([
                ("MusicTipCount".into(), "5".into()),
                ("MVTipCount".into(), "2".into()),
                ("albumcount".into(), "2".into()),
                ("keyword".into(), "A&nbsp;&lt;B&gt;&nbsp;&#39;C&#39;&nbsp;&quot;D&quot;&nbsp;&&nbsp;%&nbsp;+&nbsp;歌曲".into()),
                ("callback".into(), "tuneweaveKugouSearchTip".into()),
            ])
        );
        let lower = wire.to_ascii_lowercase();
        assert!(lower.contains("referer: https://www.kugou.com/\r\n"));
        for forbidden in [
            "cookie:",
            "authorization:",
            "token=",
            "signature=",
            "stored-account-secret",
        ] {
            assert!(!lower.contains(forbidden));
        }
        assert_eq!(wire.split_once("\r\n\r\n").unwrap().1, "");
    }
    assert_eq!(store.values.lock().unwrap().get("default"), Some(&original));
}

#[tokio::test]
async fn search_suggestions_reject_accounts_clients_and_bad_queries_before_network() {
    let f = server(vec![]).await;
    let mut invalid = vec![
        request(""),
        request("  "),
        request("line\nbreak"),
        request(&"a".repeat(513)),
    ];
    for client in [SearchSuggestionClient::Pc, SearchSuggestionClient::Mobile] {
        invalid.push(SearchSuggestionRequest {
            client,
            ..request("Q")
        });
    }
    invalid.push(SearchSuggestionRequest {
        account: Some("default".into()),
        ..request("Q")
    });
    for request in invalid {
        assert_eq!(
            f.provider
                .client
                .search_suggestions(&request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider
                .search_suggestions(&request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let caller = f
        .provider
        .caller_scope(&credential("111", "caller-secret").caller().unwrap())
        .unwrap();
    assert_eq!(
        caller
            .search_suggestions(&request("Q"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn search_suggestions_reject_transport_errors_redirects_and_executable_wrappers() {
    let valid = jsonp(&payload());
    for (frame, expected) in [
        (reply(&valid, "text/html", ""), ErrorCode::UpstreamError),
        (reply(&valid, "text/plain", "SSA-CODE: fixture\r\n"), ErrorCode::PermissionDenied),
        (reply(&format!("{valid};alert(1)"), "text/javascript", ""), ErrorCode::UpstreamError),
        ("HTTP/1.1 302 Found\r\nLocation: https://unused.invalid/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(), ErrorCode::UpstreamError),
        ("HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(), ErrorCode::RateLimited),
        ("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 131073\r\nConnection: close\r\n\r\n".into(), ErrorCode::UpstreamError),
    ] {
        let f = server(vec![frame.into()]).await;
        let error = f.provider.search_suggestions(&request("Q")).await.unwrap_err();
        assert_eq!(error.code, expected);
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
#[ignore = "requires anonymous official KuGou Web suggestions"]
async fn live_search_suggestions_returns_only_web_header_keywords() {
    let provider = KugouProvider::new(KugouConfig::default()).unwrap();
    let result = provider
        .search_suggestions(&request("周杰伦"))
        .await
        .unwrap();
    assert!(!result.suggestions.is_empty());
    assert!(result.suggestions.len() <= 7);
    assert!(result.suggestions.iter().all(|row| matches!(
        row.kind,
        Some(SearchKind::Track | SearchKind::Mv)
    ) && row.resource.is_none()));
    assert!(result.recommendations.is_empty());
}
