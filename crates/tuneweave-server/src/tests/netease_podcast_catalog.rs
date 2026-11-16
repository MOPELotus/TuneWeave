use super::*;

async fn get_response(app: &Router, path: &str) -> (StatusCode, HeaderMap, Value) {
    let response = app
        .clone()
        .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let (parts, body) = response.into_parts();
    let json = serde_json::from_slice(&to_bytes(body, 10_000).await.unwrap()).unwrap();
    (parts.status, parts.headers, json)
}

#[tokio::test]
async fn podcast_tree_and_collected_feed_preserve_navigation_and_account_cursor() {
    let app = test_app_with_provider();
    let (status, _, body) = get_response(&app, "/v1/podcasts/category-tree").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["data"]["categories"][0]["children"][0],
        json!({"id":"2","name":"全部"})
    );
    assert_eq!(body["meta"]["platform"], "netease");
    assert!(body["meta"].get("account").is_none());

    let (status, headers, body) =
        get_response(&app, "/v1/podcasts/category-tree?account=listener").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(body["meta"]["account"], "listener");

    let path = "/v1/account/podcasts/collected-feed?account=listener";
    let (status, headers, body) = get_response(&app, path).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(body["meta"]["account"], "listener");
    assert_eq!(body["data"]["has_more"], true);
    assert_eq!(body["data"]["items"][0]["feed_id"], "podcast-activity-1");
    let cursor = body["data"]["next_cursor"].as_str().unwrap();
    let (status, headers, body) =
        get_response(&app, &format!("{path}&cursor={cursor}&limit=3")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(body["data"]["has_more"], false);
    assert!(body["data"]["next_cursor"].is_null());
}

#[tokio::test]
async fn recommendation_dislikes_use_selected_account_or_isolated_caller() {
    let app = test_app_with_provider();
    let path = "/v1/account/recommendation-dislikes";
    let (status, headers, body) = get_response(&app, &format!("{path}?account=listener")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(body["meta"]["account"], "listener");
    assert_eq!(body["data"]["song_ids"], json!(["7"]));
    assert_eq!(body["data"]["podcast_ids"], json!(["100"]));

    let credential = netease_caller_credential();
    let (status, body) =
        caller_json_request(app.clone(), Method::GET, path, None, &credential).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["data"]["song_ids"], json!(["8"]));
    assert!(body["meta"].get("account").is_none());
    let (status, _) = caller_json_request(
        app,
        Method::GET,
        &format!("{path}?account=listener"),
        None,
        &credential,
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn private_podcast_catalog_rejects_bad_queries_and_missing_sessions_without_cache() {
    let mut registry = ProviderRegistry::new();
    registry
        .register(
            tuneweave_provider_netease::NeteaseProvider::new(
                tuneweave_provider_netease::NeteaseConfig::default(),
            )
            .unwrap(),
        )
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Netease));
    for (path, expected) in [
        (
            "/v1/account/recommendation-dislikes",
            StatusCode::UNAUTHORIZED,
        ),
        (
            "/v1/account/podcasts/collected-feed",
            StatusCode::UNAUTHORIZED,
        ),
        (
            "/v1/account/podcasts/collected-feed?cursor=0",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/v1/account/podcasts/collected-feed?cursor=",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/v1/account/podcasts/collected-feed?cursor=-1",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/v1/account/podcasts/collected-feed?offset=1",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/v1/account/podcasts/collected-feed?limit=0",
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let (status, headers, _) = get_response(&app, path).await;
        assert_eq!(status, expected, "{path}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store", "{path}");
    }
}
