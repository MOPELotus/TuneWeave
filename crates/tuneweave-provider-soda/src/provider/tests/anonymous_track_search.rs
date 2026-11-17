use super::*;
use crate::test_http::{json as response, serve};
use url::Url;

fn tracks(id: u32, next: Option<u32>) -> String {
    response(
        &json!({
            "result_groups": [{
                "id": "tracks",
                "data": [{"entity": {"track": {"id": id.to_string(), "name": "track"}}}],
                "has_more": next.is_some(),
                "next_cursor": next
            }]
        })
        .to_string(),
        None,
    )
}

#[tokio::test]
async fn android_empty_restarts_search_window_with_cookie_free_signed_pc_pages() {
    let (origin, platform) = serve(vec![
        response("", None),
        tracks(101, Some(45)),
        tracks(102, None),
    ])
    .await;
    let signed = || {
        response(
            r#"{"ok":true,"headers":{"x-helios":"test-h","x-medusa":"test-m"}}"#,
            None,
        )
    };
    let (signing_origin, signer) = serve(vec![signed(), signed()]).await;
    let provider = SodaProvider::from_client(
        SodaClient::test_client()
            .with_auth_test_origin(origin)
            .with_track_search_test_signer(&signing_origin),
    );
    let page = provider
        .search(&SearchQuery::tracks("晴天", 1, 1))
        .await
        .expect("signed search");
    assert_eq!(page.items[0].resource_ref.id(), "102");
    assert_eq!(
        page.items[0].extensions["backend"],
        SIGNED_TRACK_SEARCH_BACKEND
    );
    assert_eq!(
        page.pagination.extensions["backend"],
        SIGNED_TRACK_SEARCH_BACKEND
    );
    assert_eq!(
        page.pagination.extensions["request_signature_required"],
        true
    );
    assert!(!page.pagination.has_more);

    let requests = platform.await.unwrap();
    assert!(requests[0].starts_with("GET /luna/search/track?"));
    assert!(!requests[0].contains("x-medusa:"));
    for request in &requests[1..] {
        let request = request.to_ascii_lowercase();
        assert!(request.starts_with("get /luna/pc/search/track?"));
        assert!(request.contains("\r\nx-helios: test-h\r\n"));
        assert!(request.contains("\r\nx-medusa: test-m\r\n"));
        assert!(!request.contains("\r\ncookie:"));
    }
    let mut search_ids = Vec::new();
    for (request, expected_cursor) in signer.await.unwrap().iter().zip(["0", "45"]) {
        let (_, body) = request.split_once("\r\n\r\n").unwrap();
        let payload: serde_json::Value = serde_json::from_str(body).unwrap();
        assert_eq!(payload["mode"], "native");
        let url = Url::parse(payload["url"].as_str().unwrap()).unwrap();
        assert_eq!(url.host_str(), Some("api.qishui.com"));
        let params: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(params["cursor"], expected_cursor);
        assert_eq!(params["device_id"], payload["device_id"]);
        search_ids.push(params["search_id"].clone());
        assert!(
            payload["headers"]
                .as_array()
                .unwrap()
                .iter()
                .all(|pair| { !pair[0].as_str().unwrap().eq_ignore_ascii_case("cookie") })
        );
    }
    assert_eq!(search_ids[0], search_ids[1]);
}

#[tokio::test]
async fn explicit_no_results_and_verification_do_not_use_the_signer() {
    for (reply, expected) in [
        (response(r#"{"extra":{"empty_search":1}}"#, None), None),
        (
            response("", None).replace("Connection:", "bdturing-verify: test\r\nConnection:"),
            Some(ErrorCode::CapabilityNotSupported),
        ),
    ] {
        let (origin, platform) = serve(vec![reply]).await;
        let provider = SodaProvider::from_client(
            SodaClient::test_client()
                .with_auth_test_origin(origin.clone())
                .with_track_search_test_signer(&origin),
        );
        let result = provider.search(&SearchQuery::tracks("test", 10, 0)).await;
        match expected {
            Some(code) => assert_eq!(result.unwrap_err().code, code),
            None => assert!(result.unwrap().items.is_empty()),
        }
        assert_eq!(platform.await.unwrap().len(), 1);
    }
}
