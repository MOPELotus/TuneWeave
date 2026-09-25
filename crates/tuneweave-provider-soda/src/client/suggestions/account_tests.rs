use super::*;

fn body() -> String {
    json!({"status_info":{"now":1,"now_ts_ms":1000},"sugs":[
        {"suggestion":"周杰伦","content_type":"artist","entity":{"private":"not-for-output"}},
        {"suggestion":"稻香","content_type":"track"}
    ]})
    .to_string()
}

#[tokio::test]
async fn account_suggestions_sdk_binds_uid_cookie_and_device_without_changing_anonymous_scope() {
    let (origin, server) = crate::test_http::serve(vec![
        crate::test_http::json(&body(), Some("sessionid_ss=rotated; Path=/")),
        crate::test_http::json(r#"{"status_info":{"now":1,"now_ts_ms":1000}}"#, None),
        crate::test_http::json(&body(), Some("sessionid_ss=mobile-rotated; Path=/")),
        crate::test_http::json(&body(), Some("sessionid_ss=anonymous; Path=/")),
    ])
    .await;
    let client = SodaClient::new(&SodaConfig::default())
        .unwrap()
        .with_auth_test_origin(origin.clone());
    let credential = SodaCredential::test_credential("selected-secret")
        .bind_user("123456")
        .unwrap();
    let (list, rotated) = client
        .account_search_suggestions(" 周 & 杰 ", &credential, SearchSuggestionClient::Pc)
        .await
        .unwrap();
    assert_eq!(list.query, "周 & 杰");
    assert_eq!(list.suggestions[0].keyword, "周杰伦");
    assert_eq!(list.suggestions[0].kind, Some(SearchKind::Artist));
    assert_eq!(list.suggestions[1].kind, Some(SearchKind::Track));
    assert!(list.suggestions.iter().all(|s| s.resource.is_none()));
    assert_eq!(list.extensions["authenticated"], true);
    assert_eq!(list.extensions["source_user_id"], "123456");
    assert_eq!(list.extensions["backend"], "official_pc_account_sug");
    for secret in [
        "selected-secret",
        "rotated",
        "sessionid_ss",
        "not-for-output",
    ] {
        assert!(!serde_json::to_string(&list).unwrap().contains(secret));
    }
    assert!(credential.same_login(&rotated));
    assert!(rotated.cookie_header().unwrap().contains("rotated"));
    let (empty, _) = client
        .account_search_suggestions("x", &rotated, SearchSuggestionClient::Pc)
        .await
        .unwrap();
    assert!(empty.suggestions.is_empty());
    assert_eq!(empty.extensions["authenticated"], true);
    let (mobile, mobile_rotated) = client
        .account_search_suggestions("x", &rotated, SearchSuggestionClient::Mobile)
        .await
        .unwrap();
    assert_eq!(mobile.client, SearchSuggestionClient::Mobile);
    assert_eq!(mobile.extensions["authenticated"], true);
    assert_eq!(mobile.extensions["source_user_id"], "123456");
    assert_eq!(mobile.extensions["backend"], "official_android_account_sug");
    assert!(
        mobile_rotated
            .cookie_header()
            .unwrap()
            .contains("mobile-rotated")
    );
    assert!(
        !serde_json::to_string(&mobile)
            .unwrap()
            .contains("mobile-rotated")
    );
    let public = client.pc_search_suggestions("x").await.unwrap();
    assert_eq!(public.extensions["authenticated"], false);
    assert!(!public.extensions.contains_key("source_user_id"));
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests[0].contains("sessionid_ss=selected-secret"));
    assert!(requests[1].contains("sessionid_ss=rotated"));
    assert!(requests[2].starts_with("GET /luna/sug?"));
    assert!(requests[2].contains("sessionid_ss=rotated"));
    assert!(
        requests[2]
            .to_ascii_lowercase()
            .contains("x-luna-is-login: 1")
    );
    assert!(!requests[3].to_ascii_lowercase().contains("cookie:"));
    let mut devices = Vec::new();
    for request in &requests[..2] {
        assert!(request.starts_with("GET /luna/pc/sug?"));
        assert!(!request.to_ascii_lowercase().contains("x-luna-is-login:"));
        let url = origin
            .join(
                request
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap(),
            )
            .unwrap();
        let query = url.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(query.len(), 12);
        assert_eq!(query["version_code"], "20010000");
        assert_eq!(query["sug_scene"], "main");
        assert_eq!(query["fp"], query["device_id"]);
        assert_ne!(query["device_id"], query["iid"]);
        devices.push((query["device_id"].to_string(), query["iid"].to_string()));
    }
    assert_eq!(devices[0], devices[1]);
    let mobile_url = origin
        .join(
            requests[2]
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap(),
        )
        .unwrap();
    let mobile_query = mobile_url.query_pairs().collect::<BTreeMap<_, _>>();
    assert_eq!(mobile_query.len(), 8);
    assert_eq!(mobile_query["device_platform"], "android");
    assert_eq!(mobile_query["version_code"], "100211030");
    assert!(!mobile_query.contains_key("device_id"));
    assert!(!mobile_query.contains_key("iid"));
    assert!(!mobile_query.contains_key("channel"));
}

#[tokio::test]
async fn account_suggestions_sdk_requires_complete_success_before_cookie_rotation() {
    let credential = SodaCredential::test_credential("original")
        .bind_user("123456")
        .unwrap();
    let mut late_bad: serde_json::Value = serde_json::from_str(&body()).unwrap();
    late_bad["sugs"][1]["suggestion"] = json!(null);
    let responses = [
        (
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".to_owned(),
            ErrorCode::AuthenticationRequired,
        ),
        (
            crate::test_http::json(r#"{"status_code":1000016}"#, None),
            ErrorCode::AuthenticationRequired,
        ),
        (
            crate::test_http::json(r#"{"status_info":{"status_code":1000016}}"#, None),
            ErrorCode::AuthenticationRequired,
        ),
        (
            crate::test_http::json(r#"{"status_code":9}"#, None),
            ErrorCode::UpstreamError,
        ),
        (
            crate::test_http::json(
                &late_bad.to_string(),
                Some("sessionid_ss=unaccepted; Path=/"),
            ),
            ErrorCode::UpstreamError,
        ),
        (
            crate::test_http::json(&body(), Some("sessionid_ss=; Max-Age=0; Path=/")),
            ErrorCode::AuthenticationRequired,
        ),
        (
            crate::test_http::json(&body(), None).replace("application/json", "text/html"),
            ErrorCode::UpstreamError,
        ),
        (
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
                " ".repeat(MAX_BYTES + 1)
            ),
            ErrorCode::UpstreamError,
        ),
        (
            "HTTP/1.1 302 Found\r\nLocation: https://example.invalid/\r\nContent-Length: 0\r\n\r\n"
                .to_owned(),
            ErrorCode::UpstreamError,
        ),
    ];
    for client_kind in [SearchSuggestionClient::Pc, SearchSuggestionClient::Mobile] {
        for (response, code) in responses.iter() {
            let (origin, server) = crate::test_http::serve(vec![response.clone()]).await;
            let client = SodaClient::new(&SodaConfig::default())
                .unwrap()
                .with_auth_test_origin(origin);
            assert_eq!(
                client
                    .account_search_suggestions("x", &credential, client_kind)
                    .await
                    .unwrap_err()
                    .code,
                *code,
                "{client_kind:?}"
            );
            assert_eq!(server.await.unwrap().len(), 1);
            assert!(credential.cookie_header().unwrap().contains("original"));
        }
    }
}

#[tokio::test]
async fn account_suggestions_sdk_invalid_input_or_unbound_session_cannot_initialize_device() {
    let path = std::env::temp_dir().join(format!(
        "soda-suggestions-invalid-{}",
        rand::random::<u64>()
    ));
    let client = SodaClient::new(&SodaConfig {
        device_path: Some(path.clone()),
        ..SodaConfig::default()
    })
    .unwrap();
    let unbound = SodaCredential::test_credential("secret");
    let bound = unbound.clone().bind_user("123456").unwrap();
    for client_kind in [SearchSuggestionClient::Pc, SearchSuggestionClient::Mobile] {
        assert_eq!(
            client
                .account_search_suggestions("valid", &unbound, client_kind)
                .await
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
        for query in [" ".to_owned(), "a\nb".to_owned(), "x".repeat(1025)] {
            assert_eq!(
                client
                    .account_search_suggestions(&query, &bound, client_kind)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidRequest
            );
        }
    }
    assert!(!path.exists());
}
