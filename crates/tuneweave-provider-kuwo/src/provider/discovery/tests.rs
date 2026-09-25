use super::super::*;
use crate::client::{
    catalog::{
        CatalogKind,
        tests::{body, home, json_response, requests, response, setup, setup_with_gates},
    },
    native::tests as native_fixture,
};
use std::time::Duration;
use tokio::sync::Notify;
use tuneweave_core::{
    AccountCredentialStore, ErrorCode, SearchSuggestionClient, SearchSuggestionRequest,
    SearchTrendingDetail, SearchTrendingRequest, StoredAccountCredential,
};

fn suggest(query: &str) -> SearchSuggestionRequest {
    SearchSuggestionRequest {
        query: query.into(),
        client: SearchSuggestionClient::Web,
        account: None,
    }
}
fn trending() -> SearchTrendingRequest {
    SearchTrendingRequest {
        detail: SearchTrendingDetail::Full,
        account: None,
    }
}
fn envelope(rows: serde_json::Value) -> serde_json::Value {
    json!({"code":200,"success":true,"data":rows})
}
fn suggestions() -> serde_json::Value {
    envelope(json!([
        "RELWORD=测试 & + / ? = 第二行\r\nSNUM=12345\r\nRNUM=1000\r\nTYPE=0\r\nOPAQUE=private-not-for-export",
        "RELWORD=Artist &lt;name&gt;\nTYPE=9",
        "RELWORD=Artist &lt;name&gt;"
    ]))
}
fn assert_wire(wire: &str, key: &str) -> String {
    let url = url::Url::parse(&format!(
        "https://www.kuwo.cn{}",
        wire.split_whitespace().nth(1).unwrap()
    ))
    .unwrap();
    assert_eq!(url.path(), "/openapi/v1/www/search/searchKey");
    let pairs = url
        .query_pairs()
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(pairs.len(), 5);
    assert_eq!(pairs["key"], key);
    assert_eq!(pairs["httpsStatus"], "1");
    assert_eq!(pairs["plat"], "web_www");
    assert_eq!(pairs["from"], "");
    assert_eq!(pairs["reqId"].len(), 36);
    let lower = wire.to_ascii_lowercase();
    for header in ["cookie:", "secret:", "authorization:"] {
        assert!(!lower.contains(header));
    }
    assert!(lower.contains("referer: https://www.kuwo.cn/\r\n"));
    pairs["reqId"].to_string()
}

#[tokio::test]
async fn discovery_suggestions_sdk_and_provider_preserve_text_order_and_unknown_kind() {
    for sdk in [false, true] {
        let mut f = setup(vec![json_response(&suggestions())]).await;
        let request = suggest("  测试 & + / ? =  ");
        let result = if sdk {
            f.provider.client.search_suggestions(&request).await
        } else {
            f.provider.search_suggestions(&request).await
        }
        .unwrap();
        assert_eq!(result.query, "测试 & + / ? =");
        assert_eq!(result.client, SearchSuggestionClient::Web);
        assert_eq!(result.suggestions.len(), 3);
        assert_eq!(result.suggestions[0].keyword, "测试 & + / ? = 第二行");
        assert_eq!(result.suggestions[1].keyword, "Artist &lt;name&gt;");
        assert_eq!(result.suggestions[2].keyword, result.suggestions[1].keyword);
        assert!(result.suggestions.iter().all(|entry| entry.kind.is_none()
            && entry.resource.is_none()
            && entry.icon_url.is_none()
            && entry.display_text.is_none()));
        assert_eq!(result.suggestions[0].extensions["upstream_type"], 0);
        assert_eq!(result.suggestions[0].extensions["upstream_snum"], 12345);
        assert_eq!(result.suggestions[0].extensions["upstream_rnum"], 1000);
        assert_eq!(result.suggestions[1].extensions["upstream_type"], 9);
        assert!(result.suggestions[2].extensions.is_empty());
        assert!(result.recommendations.is_empty());
        assert_eq!(result.extensions["recommendations_available"], false);
        let encoded = serde_json::to_string(&result).unwrap();
        assert!(!encoded.contains("private-not-for-export"));
        assert!(!result.extensions.contains_key("complete_snapshot"));
        assert_wire(&requests(&mut f, 1).await[0], "测试 & + / ? =");
    }
}

#[tokio::test]
async fn discovery_trending_preserves_keyword_positions_in_both_views_and_sdk() {
    let mut seen_ids = BTreeSet::new();
    for sdk in [false, true] {
        for detail in [SearchTrendingDetail::Brief, SearchTrendingDetail::Full] {
            let mut f = setup(vec![json_response(&envelope(json!([
                "First", "重复", "First"
            ])))])
            .await;
            let request = SearchTrendingRequest {
                detail,
                account: None,
            };
            let result = if sdk {
                f.provider.client.trending_searches(&request).await
            } else {
                f.provider.trending_searches(&request).await
            }
            .unwrap();
            assert_eq!(result.detail, detail);
            assert_eq!(
                result
                    .entries
                    .iter()
                    .map(|r| (r.rank, r.keyword.as_str()))
                    .collect::<Vec<_>>(),
                [(1, "First"), (2, "重复"), (3, "First")]
            );
            assert!(result.entries.iter().all(|r| r.description.is_none()
                && r.score.is_none()
                && r.icon_type.is_none()
                && r.icon_url.is_none()
                && r.target_url.is_none()));
            assert_eq!(result.extensions["metadata_scope"], "keywords_only");
            assert_eq!(result.extensions["rank_scope"], "upstream_response_order");
            assert!(!result.extensions.contains_key("complete_snapshot"));
            assert!(seen_ids.insert(assert_wire(&requests(&mut f, 1).await[0], "")));
        }
    }
}

#[tokio::test]
async fn discovery_explicit_empty_is_success_for_both_endpoints_without_fixed_row_count() {
    for count in [0, 1, 21, 100] {
        let mut f = setup(vec![
            json_response(&envelope(json!(vec!["RELWORD=A"; count]))),
            json_response(&envelope(json!(vec!["A"; count]))),
        ])
        .await;
        assert_eq!(
            f.provider
                .search_suggestions(&suggest("NoMatch"))
                .await
                .unwrap()
                .suggestions
                .len(),
            count
        );
        assert_eq!(
            f.provider
                .trending_searches(&trending())
                .await
                .unwrap()
                .entries
                .len(),
            count
        );
        requests(&mut f, 2).await;
    }
}

#[tokio::test]
async fn discovery_envelope_rejects_missing_shape_contradictions_and_too_many_rows() {
    let cases = [
        json!({}),
        json!({"code":200}),
        json!({"code":200,"data":null}),
        json!({"code":200,"data":{}}),
        json!({"code":200,"data":[7]}),
        json!({"code":200,"data":[null]}),
        json!({"code":"200","data":[]}),
        json!({"code":200,"success":false,"data":[],"msg":"do-not-export"}),
        json!({"code":200,"success":"true","data":[]}),
        json!({"code":2001,"data":[]}),
        json!({"code":-1,"data":[],"msg":"do-not-export"}),
        envelope(json!(vec!["A"; 101])),
    ];
    for suggestions in [false, true] {
        for value in &cases {
            let mut f = setup(vec![json_response(value)]).await;
            let error = if suggestions {
                f.provider
                    .search_suggestions(&suggest("A"))
                    .await
                    .unwrap_err()
            } else {
                f.provider.trending_searches(&trending()).await.unwrap_err()
            };
            assert_eq!(error.code, ErrorCode::UpstreamError);
            assert_eq!(error.platform, Some(Platform::Kuwo));
            assert!(!format!("{error:?}").contains("do-not-export"));
            requests(&mut f, 1).await;
        }
    }
}

#[tokio::test]
async fn discovery_suggestions_do_not_silently_drop_bad_rows_or_parse_records_as_resources() {
    let mut cases = vec![
        "".into(),
        "plain keyword".into(),
        "TYPE=0".into(),
        "RELWORD=  ".into(),
        "RELWORD=A\nRELWORD=B".into(),
        "RELWORD=A\nTYPE=0\nTYPE=1".into(),
        "RELWORD=A\nTYPE=-1".into(),
        "RELWORD=A\nTYPE=+1".into(),
        "RELWORD=A\nSNUM=".into(),
        "RELWORD=A\nRNUM=18446744073709551616".into(),
        "RELWORD=A\n\nTYPE=0".into(),
        "RELWORD=A\n=X".into(),
        "RELWORD=A\nBad field=x".into(),
        "RELWORD=A\nOPAQUE=x\u{0000}".into(),
        "RELWORD=A\nTYPE=0\rRNUM=1".into(),
        "RELWORD=A\tB".into(),
        format!("RELWORD={}", "😀".repeat(257)),
        format!("RELWORD=A\nOPAQUE={}", "X".repeat(4096)),
    ];
    cases.push(format!(
        "RELWORD=A\n{}",
        (0..32)
            .map(|i| format!("FIELD_{i}=x"))
            .collect::<Vec<_>>()
            .join("\n")
    ));
    for row in cases {
        let mut f = setup(vec![json_response(&envelope(json!([
            "RELWORD=Valid",
            row
        ])))])
        .await;
        assert_eq!(
            f.provider
                .search_suggestions(&suggest("A"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        requests(&mut f, 1).await;
    }
}

#[tokio::test]
async fn discovery_trending_rejects_invalid_keywords_instead_of_returning_partial_results() {
    for word in [
        "".to_string(),
        "   ".into(),
        "A\nB".into(),
        "A\u{0000}".into(),
        "😀".repeat(257),
    ] {
        let mut f = setup(vec![json_response(&envelope(json!(["Valid", word])))]).await;
        assert_eq!(
            f.provider
                .trending_searches(&trending())
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        requests(&mut f, 1).await;
    }
}

struct NoAccountAccess;
impl AccountCredentialStore for NoAccountAccess {
    fn load_platform(&self, _: Platform) -> Result<Vec<StoredAccountCredential>> {
        panic!("public search discovery read accounts")
    }
    fn put(&self, _: &StoredAccountCredential) -> Result<()> {
        panic!("public search discovery wrote accounts")
    }
    fn remove(&self, _: Platform, _: &str) -> Result<bool> {
        panic!("public search discovery removed accounts")
    }
}

#[tokio::test]
async fn discovery_invalid_inputs_and_account_sources_fail_before_any_io() {
    let mut f = setup(vec![
        json_response(&envelope(json!([]))),
        json_response(&envelope(json!([]))),
    ])
    .await;
    f.provider.credential_store = Some(Arc::new(NoAccountAccess));
    for query in [
        "".to_owned(),
        " \u{3000} ".into(),
        "A\nB".into(),
        "\tA".into(),
        "x\u{0000}".into(),
        "A".repeat(129),
        "😀".repeat(65),
    ] {
        let request = suggest(&query);
        assert_eq!(
            f.provider
                .search_suggestions(&request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider
                .client
                .search_suggestions(&request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for client in [SearchSuggestionClient::Mobile, SearchSuggestionClient::Pc] {
        let mut request = suggest("A");
        request.client = client;
        assert_eq!(
            f.provider
                .search_suggestions(&request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider
                .client
                .search_suggestions(&request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for account in ["", "default", "selected"] {
        let mut s = suggest("A");
        s.account = Some(account.into());
        let mut t = trending();
        t.account = Some(account.into());
        assert_eq!(
            f.provider.search_suggestions(&s).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider.trending_searches(&t).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider
                .client
                .search_suggestions(&s)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            f.provider
                .client
                .trending_searches(&t)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let credential = native_fixture::credential_fixture("42", "private-discovery-session")
        .caller()
        .unwrap();
    let caller = f.provider.with_caller_credential(&credential).unwrap();
    assert_eq!(
        caller
            .search_suggestions(&suggest("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert_eq!(
        caller
            .trending_searches(&trending())
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(caller.take_response_credential().unwrap().is_none());
    assert!(f.seen.try_recv().is_err());
    f.provider.search_suggestions(&suggest("A")).await.unwrap();
    f.provider.trending_searches(&trending()).await.unwrap();
    let seen = requests(&mut f, 2).await;
    assert_wire(&seen[0], "A");
    assert_wire(&seen[1], "");
    assert!(
        seen.iter()
            .all(|wire| !wire.contains("private-discovery-session"))
    );
}

#[tokio::test]
async fn discovery_query_boundary_matches_web_utf16_and_only_trims_outer_whitespace() {
    for query in ["A".repeat(128), "😀".repeat(64), "一 二 = ? & +".into()] {
        let mut f = setup(vec![json_response(&envelope(json!([])))]).await;
        let result = f
            .provider
            .search_suggestions(&suggest(&format!(" \u{3000}{query}\u{3000} ")))
            .await
            .unwrap();
        assert_eq!(result.query, query);
        assert_wire(&requests(&mut f, 1).await[0], &query);
    }
}

#[tokio::test]
async fn discovery_neither_uses_nor_poison_signed_catalogue_cookies() {
    let public_reply = response(
        200,
        "application/json",
        "Set-Cookie: Hm_Iuvt_cdb524f42f23cer9b268564v7y735ewrq2324=poison-discovery-cookie; Path=/\r\n",
        &serde_json::to_vec(&envelope(json!([]))).unwrap(),
    );
    let mut f = setup(vec![
        home(),
        json_response(&body(CatalogKind::Album, 1, 0)),
        public_reply.clone(),
        public_reply,
        json_response(&body(CatalogKind::Album, 1, 0)),
    ])
    .await;
    f.provider
        .client
        .search_catalog_page(CatalogKind::Album, "A", 1)
        .await
        .unwrap();
    f.provider.search_suggestions(&suggest("A")).await.unwrap();
    f.provider.trending_searches(&trending()).await.unwrap();
    f.provider
        .client
        .search_catalog_page(CatalogKind::Album, "A", 1)
        .await
        .unwrap();
    let seen = requests(&mut f, 5).await;
    assert_wire(&seen[2], "A");
    assert_wire(&seen[3], "");
    assert!(seen[1].contains("anonymousCatalogueCookie123456"));
    assert!(seen[4].contains("anonymousCatalogueCookie123456"));
    assert!(
        seen.iter()
            .all(|wire| !wire.contains("poison-discovery-cookie"))
    );
}

#[tokio::test]
async fn discovery_transport_failures_and_malformed_json_do_not_retry_or_refresh_sessions() {
    let cases = [
        (
            response(403, "text/html", "", b"private-upstream-body"),
            ErrorCode::UpstreamError,
        ),
        (
            response(429, "application/json", "", b"private-upstream-body"),
            ErrorCode::RateLimited,
        ),
        (
            response(500, "application/json", "", b"private-upstream-body"),
            ErrorCode::UpstreamError,
        ),
        (
            response(
                302,
                "text/html",
                "Location: https://foreign.invalid/\r\n",
                b"",
            ),
            ErrorCode::UpstreamError,
        ),
        (
            response(200, "text/html", "", b"{}"),
            ErrorCode::UpstreamError,
        ),
        (
            response(200, "application/json", "", b"private-upstream-body"),
            ErrorCode::UpstreamError,
        ),
        (
            response(
                200,
                "application/json",
                "",
                b"{\"code\":200,\"data\":[]}{\"extra\":true}",
            ),
            ErrorCode::UpstreamError,
        ),
    ];
    for suggestions in [false, true] {
        for (wire, code) in &cases {
            let mut f = setup(vec![wire.clone()]).await;
            let error = if suggestions {
                f.provider
                    .search_suggestions(&suggest("A"))
                    .await
                    .unwrap_err()
            } else {
                f.provider.trending_searches(&trending()).await.unwrap_err()
            };
            assert_eq!(error.code, *code);
            assert!(!format!("{error:?}").contains("private-upstream-body"));
            requests(&mut f, 1).await;
        }
    }
}

#[tokio::test]
async fn discovery_size_budget_covers_declared_and_streamed_response_bodies() {
    let oversized = vec![b' '; 256 * 1024 + 1];
    let mut streamed = format!("HTTP/1.1 200 Fixture\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n", oversized.len()).into_bytes();
    streamed.extend_from_slice(&oversized);
    streamed.extend_from_slice(b"\r\n0\r\n\r\n");
    for reply in [response(200, "application/json", "", &oversized), streamed] {
        let mut f = setup(vec![reply]).await;
        let error = f.provider.trending_searches(&trending()).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(error.message.contains("size limit"));
        requests(&mut f, 1).await;
    }
}

#[tokio::test]
async fn discovery_total_budget_and_cancellation_cover_sdk_and_provider() {
    for sdk in [false, true] {
        for suggestions in [false, true] {
            for cancel in [false, true] {
                let gate = Arc::new(Notify::new());
                let mut f = setup_with_gates(vec![(
                    json_response(&envelope(json!([]))),
                    Some(gate.clone()),
                )])
                .await;
                native_fixture::set_request_timeout(
                    &mut f.provider.client,
                    Duration::from_secs(60),
                );
                let provider = f.provider.clone();
                let task = tokio::spawn(async move {
                    match (sdk, suggestions) {
                        (true, true) => provider
                            .client
                            .search_suggestions(&suggest("A"))
                            .await
                            .map(|_| ()),
                        (true, false) => provider
                            .client
                            .trending_searches(&trending())
                            .await
                            .map(|_| ()),
                        (false, true) => {
                            provider.search_suggestions(&suggest("A")).await.map(|_| ())
                        }
                        (false, false) => provider.trending_searches(&trending()).await.map(|_| ()),
                    }
                });
                tokio::time::timeout(Duration::from_secs(5), f.seen.recv())
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
                gate.notify_one();
                (&mut f.server).await.unwrap();
                assert!(f.seen.try_recv().is_err());
                assert!(f.provider.take_response_credential().unwrap().is_none());
            }
        }
    }
}
