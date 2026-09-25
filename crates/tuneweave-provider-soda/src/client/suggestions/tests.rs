use super::*;

fn envelope(sugs: serde_json::Value) -> serde_json::Value {
    json!({"status_info":{"now":1789556330u64,"now_ts_ms":1789556330206u64},"sugs":sugs})
}

#[test]
fn suggestions_preserve_keyword_order_and_known_types_without_inventing_resources() {
    let body = envelope(json!([
        {"suggestion":" 曲目 ","content_type":"track","entity":{"track":{"id":"123"}}},
        {"suggestion":"专辑","content_type":"album"},
        {"suggestion":"歌手","content_type":"artist"},
        {"suggestion":"歌单","content_type":"playlist"},
        {"suggestion":"未来类型","content_type":"future"},
        {"suggestion":"纯关键词"}
    ]));
    let parsed = parse(body.to_string().as_bytes(), "周").unwrap();
    assert_eq!(
        parsed
            .suggestions
            .iter()
            .map(|x| x.keyword.as_str())
            .collect::<Vec<_>>(),
        ["曲目", "专辑", "歌手", "歌单", "未来类型", "纯关键词"]
    );
    assert_eq!(
        parsed
            .suggestions
            .iter()
            .map(|x| x.kind)
            .collect::<Vec<_>>(),
        [
            Some(SearchKind::Track),
            Some(SearchKind::Album),
            Some(SearchKind::Artist),
            Some(SearchKind::Playlist),
            None,
            None
        ]
    );
    assert!(
        parsed
            .suggestions
            .iter()
            .all(|s| s.resource.is_none() && s.icon_url.is_none() && s.display_text.is_none())
    );
    assert!(parsed.recommendations.is_empty());
    assert_eq!(parsed.extensions["authenticated"], false);
}

#[test]
fn suggestions_require_actual_status_metadata_even_when_sugs_is_omitted() {
    let mut empty = envelope(json!([]));
    assert!(
        parse(empty.to_string().as_bytes(), "x")
            .unwrap()
            .suggestions
            .is_empty()
    );
    empty.as_object_mut().unwrap().remove("sugs");
    assert!(
        parse(empty.to_string().as_bytes(), "x")
            .unwrap()
            .suggestions
            .is_empty()
    );
    for body in [
        json!({}),
        json!({"status_code":0}),
        json!({"status_info":{}}),
        json!({"status_info":{"now":0,"now_ts_ms":0}}),
        json!({"status_info":{"now":1,"now_ts_ms":2000}}),
        json!({"status_info":{"now":u64::MAX,"now_ts_ms":u64::MAX}}),
        json!({"status_info":{"now":1,"now_ts_ms":1000},"sugs":null}),
    ] {
        assert_eq!(
            parse(body.to_string().as_bytes(), "x").unwrap_err().code,
            ErrorCode::UpstreamError
        );
    }
}

#[test]
fn suggestions_reject_business_errors_and_any_invalid_late_item() {
    let good = envelope(json!([{"suggestion":"first"}]));
    for path in ["status_code", "status_info"] {
        let mut body = good.clone();
        if path == "status_info" {
            body[path]["status_code"] = json!(1000016);
        } else {
            body[path] = json!(7);
        }
        assert!(parse(body.to_string().as_bytes(), "x").is_err());
    }
    for item in [
        json!({}),
        json!({"suggestion":42}),
        json!({"suggestion":" "}),
        json!({"suggestion":"a\nb"}),
        json!({"suggestion":"x".repeat(1025)}),
        json!({"suggestion":"last","content_type":"x".repeat(65)}),
    ] {
        let body = envelope(json!([{"suggestion":"first"},item]));
        assert!(parse(body.to_string().as_bytes(), "x").is_err());
    }
    let body = envelope(json!(vec![json!({"suggestion":"same"}); 65]));
    assert!(parse(body.to_string().as_bytes(), "x").is_err());
}

#[tokio::test]
async fn suggestions_send_only_anonymous_pc_fields_and_never_reuse_response_cookies() {
    let body = envelope(json!([{"suggestion":"周杰伦"}])).to_string();
    let (origin, server) = crate::test_http::serve(vec![
        crate::test_http::json(&body, Some("sessionid_ss=must-not-reuse; Path=/")),
        crate::test_http::json(&body, None),
    ])
    .await;
    let temp_dir = std::env::temp_dir().join(format!("soda-sug-device-{}", rand::random::<u64>()));
    std::fs::create_dir(&temp_dir).unwrap();
    let device_path = temp_dir.join("state.json");
    let client = SodaClient::new(&SodaConfig {
        device_path: Some(device_path.clone()),
        ..SodaConfig::default()
    })
    .unwrap()
    .with_auth_test_origin(origin.clone());
    for _ in 0..2 {
        assert_eq!(
            client
                .pc_search_suggestions(" 周 & 杰 ")
                .await
                .unwrap()
                .query,
            "周 & 杰"
        );
    }
    let requests = server.await.unwrap();
    let mut ids = Vec::new();
    for request in &requests {
        let line = request.lines().next().unwrap();
        assert!(line.starts_with("GET /luna/pc/sug?"));
        let url = origin
            .join(line.split_whitespace().nth(1).unwrap())
            .unwrap();
        let q = url.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(q.len(), 12);
        assert_eq!(q["q"], "周 & 杰");
        assert_eq!(q["sug_scene"], "main");
        assert_eq!(q["app_name"], "luna_pc");
        assert_eq!(q["device_platform"], "windows");
        assert_eq!(q["version_name"], "2.1.0");
        assert_eq!(q["version_code"], "20010000");
        assert!(!q["device_id"].is_empty());
        assert!(!q["iid"].is_empty());
        assert_ne!(q["device_id"], q["iid"]);
        assert_eq!(q["fp"], q["device_id"]);
        let id = q["sug_search_id"].to_string();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
        assert_eq!(hex::decode(id.replace('-', "")).unwrap().len(), 16);
        ids.push(id);
        assert!(!request.to_ascii_lowercase().contains("cookie:"));
        assert!(!request.to_ascii_lowercase().contains("authorization:"));
        assert!(!request.to_ascii_lowercase().contains("x-luna-is-login:"));
    }
    assert_ne!(ids[0], ids[1]);
    let device = client.login_device().unwrap();
    assert!(
        device_path.exists(),
        "PC suggestions need stable common device params"
    );
    for request in &requests {
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
        let q = url.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(q["device_id"], device.device_id.as_str());
        assert_eq!(q["iid"], device.install_id.as_str());
        assert_eq!(q["fp"], device.device_id.as_str());
    }
    std::fs::remove_dir_all(temp_dir).unwrap();
}

#[tokio::test]
async fn mobile_suggestions_use_the_android_endpoint_without_devices_or_session_cookies() {
    let body = envelope(json!([
        {"suggestion":"歌手","content_type":"artist","entity":{"artist":{"id":"123"}}},
        {"suggestion":"歌曲","content_type":"track"},
        {"suggestion":"歌曲","content_type":"future"}
    ]))
    .to_string();
    let (origin, server) = crate::test_http::serve(vec![
        crate::test_http::json(&body, Some("sessionid_ss=not-for-mobile; Path=/")),
        crate::test_http::json(&envelope(json!([])).to_string(), None),
    ])
    .await;
    let root = std::env::temp_dir().join(format!("soda-mobile-sug-{}", rand::random::<u64>()));
    let client = SodaClient::new(&SodaConfig {
        device_path: Some(root.clone()),
        ..SodaConfig::default()
    })
    .unwrap()
    .with_auth_test_origin(origin.clone());
    let result = client.mobile_search_suggestions(" 周 & +? ").await.unwrap();
    assert_eq!(result.client, SearchSuggestionClient::Mobile);
    assert_eq!(result.query, "周 & +?");
    assert_eq!(result.extensions["backend"], "official_android_sug");
    assert_eq!(result.extensions["authenticated"], false);
    assert_eq!(result.suggestions.len(), 3);
    assert_eq!(result.suggestions[0].kind, Some(SearchKind::Artist));
    assert_eq!(result.suggestions[1].kind, Some(SearchKind::Track));
    assert_eq!(result.suggestions[2].kind, None);
    assert_eq!(result.suggestions[1].keyword, result.suggestions[2].keyword);
    assert!(result.suggestions.iter().all(|s| s.resource.is_none()));
    assert!(result.recommendations.is_empty());
    assert!(
        client
            .mobile_search_suggestions("empty")
            .await
            .unwrap()
            .suggestions
            .is_empty()
    );
    let requests = server.await.unwrap();
    let mut ids = Vec::new();
    for (index, wire) in requests.iter().enumerate() {
        assert!(wire.starts_with("GET /luna/sug?"));
        let url = origin
            .join(wire.split_whitespace().nth(1).unwrap())
            .unwrap();
        let query = url.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(query.len(), 8);
        assert_eq!(query["q"], if index == 0 { "周 & +?" } else { "empty" });
        assert_eq!(query["sug_scene"], "main");
        assert_eq!(query["aid"], "386088");
        assert_eq!(query["app_name"], "luna");
        assert_eq!(query["device_platform"], "android");
        assert_eq!(query["version_name"], "21.1.0");
        assert_eq!(query["version_code"], "100211030");
        let id = query["sug_search_id"].to_string();
        assert_eq!(id.len(), 36);
        assert_eq!(&id[14..15], "4");
        assert!(matches!(&id[19..20], "8" | "9" | "a" | "b"));
        ids.push(id);
        for header in ["cookie:", "authorization:"] {
            assert!(!wire.to_ascii_lowercase().contains(header));
        }
    }
    assert_ne!(ids[0], ids[1]);
    assert!(!root.exists());
}

#[tokio::test]
async fn suggestions_reject_http_redirect_mime_and_unbounded_bodies_without_fallback() {
    let good = envelope(json!([])).to_string();
    let cases = [
        ("HTTP/1.1 302 Found\r\nLocation: https://example.invalid/secret\r\nContent-Length: 0\r\n\r\n".to_owned(),ErrorCode::UpstreamError),
        ("HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n".to_owned(),ErrorCode::RateLimited),
        (crate::test_http::json(&good,None).replace("application/json","text/html"),ErrorCode::UpstreamError),
        (format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",MAX_BYTES+1),ErrorCode::UpstreamError),
        (format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}"," ".repeat(MAX_BYTES+1)),ErrorCode::UpstreamError),
    ];
    for (index, (response, code)) in cases.into_iter().enumerate() {
        for mobile in [false, true] {
            let (origin, server) = crate::test_http::serve(vec![response.clone()]).await;
            let client = SodaClient::new(&SodaConfig::default())
                .unwrap()
                .with_auth_test_origin(origin);
            let result = if mobile {
                client.mobile_search_suggestions("x").await
            } else {
                client.pc_search_suggestions("x").await
            };
            let error = result.unwrap_err();
            assert_eq!(error.code, code, "{index}, mobile={mobile}");
            if index >= 3 {
                assert!(error.message.contains("size limit"));
            }
            assert_eq!(server.await.unwrap().len(), 1);
        }
    }
}

#[tokio::test]
#[ignore = "requires official anonymous Soda PC suggestions; no account is used"]
async fn live_pc_suggestions_accept_results_without_assuming_rare_queries_are_empty() {
    let client = SodaClient::new(&SodaConfig::default()).unwrap();
    let found = client.pc_search_suggestions("周杰伦").await.unwrap();
    assert!(!found.suggestions.is_empty());
    assert!(found.suggestions.len() <= MAX_SUGGESTIONS);
    // Suggestions can change between requests, even for a rare query. The captured
    // omitted-sugs response is tested deterministically above, not assumed live.
    let rare = client
        .pc_search_suggestions("tuneweavezzzz987654321")
        .await
        .unwrap();
    assert!(rare.suggestions.len() <= MAX_SUGGESTIONS);
    assert!(rare.suggestions.iter().all(|s| valid_text(&s.keyword)));
}
