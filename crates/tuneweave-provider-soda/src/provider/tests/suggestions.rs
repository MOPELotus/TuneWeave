use super::*;

#[tokio::test]
async fn public_suggestions_ignore_stored_sessions_and_response_cookies() {
    let mut fixture = SessionFixture::new();
    let credential = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("default", &credential);
    let before = fixture.stored("default").unwrap();
    let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
        r#"{"status_info":{"now":1,"now_ts_ms":1000},"sugs":[{"suggestion":"周杰伦"}]}"#,
        Some("sessionid_ss=unrelated; Path=/"),
    )])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let request = SearchSuggestionRequest {
        query: "周".to_owned(),
        client: SearchSuggestionClient::Pc,
        account: None,
    };
    let public = fixture.provider.search_suggestions(&request).await.unwrap();
    assert_eq!(public.suggestions[0].keyword, "周杰伦");
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].to_ascii_lowercase().contains("cookie:"));
    assert_eq!(fixture.stored("default").unwrap().secret(), before.secret());
    assert!(
        fixture
            .provider
            .take_response_credential()
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn suggestion_invalid_queries_and_web_client_fail_before_network() {
    let provider = SodaProvider::from_client(
        SodaClient::new(&SodaConfig::default())
            .unwrap()
            .with_auth_test_origin(url::Url::parse("http://127.0.0.1:1/").unwrap()),
    );
    let request = SearchSuggestionRequest {
        query: "q".to_owned(),
        client: SearchSuggestionClient::Pc,
        account: None,
    };
    for query in [" ".to_owned(), "a\nb".to_owned(), "x".repeat(1025)] {
        assert_eq!(
            provider
                .search_suggestions(&SearchSuggestionRequest {
                    query,
                    ..request.clone()
                })
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        provider
            .search_suggestions(&SearchSuggestionRequest {
                client: SearchSuggestionClient::Web,
                ..request
            })
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
}

#[tokio::test]
async fn mobile_suggestions_without_an_explicit_source_ignore_stored_accounts_and_cookies() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("default", &source);
    fixture.put("personal", &source);
    let before = fixture.stored("default").unwrap();
    let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
        r#"{"status_info":{"now":1,"now_ts_ms":1000},"sugs":[{"suggestion":"周杰伦"}]}"#,
        Some("sessionid_ss=anonymous-only; Path=/"),
    )])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let request = SearchSuggestionRequest {
        query: " 周 ".into(),
        client: SearchSuggestionClient::Mobile,
        account: None,
    };
    let result = fixture.provider.search_suggestions(&request).await.unwrap();
    assert_eq!(result.client, SearchSuggestionClient::Mobile);
    assert_eq!(result.extensions["backend"], "official_android_sug");
    assert_eq!(result.extensions["authenticated"], false);
    assert!(!result.extensions.contains_key("source_user_id"));
    let wires = server.await.unwrap();
    assert_eq!(wires.len(), 1);
    assert!(wires[0].starts_with("GET /luna/sug?"));
    assert!(!wires[0].to_ascii_lowercase().contains("cookie:"));
    assert!(wires[0].to_ascii_lowercase().contains("x-luna-is-login: 0"));
    assert_eq!(fixture.stored("default").unwrap().secret(), before.secret());
    assert!(
        fixture
            .provider
            .take_response_credential()
            .unwrap()
            .is_none()
    );
}
