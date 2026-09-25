use super::*;

#[test]
fn account_album_preserves_order_duplicates_and_positions_without_player_material() {
    let page = parse(test_fixture().to_string().as_bytes(), "900").unwrap();
    assert_eq!(page.album.track_count, Some(3));
    assert_eq!(
        page.tracks
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>(),
        ["11", "22", "22"]
    );
    for (i, t) in page.tracks.iter().enumerate() {
        assert_eq!(t.extensions["album_position"], i);
        assert_eq!(t.extensions["backend"], BACKEND);
    }
    let output = json!({"album":page.album,"tracks":page.tracks}).to_string();
    for secret in [
        "private-player-material",
        "private-key",
        "video_model",
        "url_player_info",
        "is_collected",
    ] {
        assert!(!output.contains(secret));
    }
    let mut empty = test_fixture();
    empty["album_info"]["count_tracks"] = json!(0);
    empty["tracks"] = json!([]);
    assert!(
        parse(empty.to_string().as_bytes(), "900")
            .unwrap()
            .tracks
            .is_empty()
    );
}

#[test]
fn account_album_requires_counts_complete_identity_and_valid_late_items() {
    for (path, value) in [
        ("/album_info/id", json!("901")),
        ("/album_info/count_tracks", json!(2)),
        ("/tracks/2/album/id", json!("901")),
        ("/tracks/2/id", json!("bad")),
        ("/tracks/2/name", json!("")),
        ("/tracks/2/media_type", json!("video")),
        ("/has_more", json!(true)),
        ("/status_info/now", json!(0)),
        ("/status_info/now_ts_ms", json!(2000)),
        ("/album_info/hasError", json!(true)),
    ] {
        let mut value_json = test_fixture();
        if path == "/has_more" {
            value_json["has_more"] = value;
        } else if path == "/album_info/hasError" {
            value_json["album_info"]["hasError"] = value;
        } else {
            *value_json.pointer_mut(path).unwrap() = value;
        }
        assert_eq!(
            parse(value_json.to_string().as_bytes(), "900")
                .err()
                .unwrap()
                .code,
            ErrorCode::UpstreamError,
            "{path}"
        );
    }
    for field in ["tracks", "album_info", "status_info"] {
        let mut body = test_fixture();
        body.as_object_mut().unwrap().remove(field);
        assert!(parse(body.to_string().as_bytes(), "900").is_err());
    }
    let mut body = test_fixture();
    body["album_info"]
        .as_object_mut()
        .unwrap()
        .remove("count_tracks");
    assert!(parse(body.to_string().as_bytes(), "900").is_err());
    body = test_fixture();
    body["tracks"] = json!(vec![body["tracks"][0].clone(); 10001]);
    body["album_info"]["count_tracks"] = json!(10001);
    assert!(parse(body.to_string().as_bytes(), "900").is_err());
}

#[test]
fn account_album_business_failures_cannot_turn_into_empty_success() {
    for code in [1_000_016, 1_000_005, 9] {
        for nested in [false, true] {
            let value = if nested {
                json!({"status_info":{"status_code":code}})
            } else {
                json!({"status_code":code})
            };
            assert_eq!(
                parse(value.to_string().as_bytes(), "900")
                    .err()
                    .unwrap()
                    .code,
                if code == 1_000_016 {
                    ErrorCode::AuthenticationRequired
                } else {
                    ErrorCode::UpstreamError
                }
            );
        }
    }
    for body in [
        json!({}),
        json!({"status_code":0}),
        json!({"status_info":{"now":1,"now_ts_ms":1000}}),
    ] {
        assert!(parse(body.to_string().as_bytes(), "900").is_err());
    }
}

#[tokio::test]
async fn account_album_sdk_sends_selected_cookie_and_rotates_only_after_complete_validation() {
    let body = test_fixture().to_string();
    let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
        &body,
        Some("sessionid_ss=rotated; Path=/"),
    )])
    .await;
    let client = SodaClient::new(&SodaConfig::default())
        .unwrap()
        .with_auth_test_origin(origin.clone());
    let credential = SodaCredential::test_credential("selected-secret")
        .bind_user("123456")
        .unwrap();
    let result = client.account_album("900", &credential).await.unwrap();
    assert!(
        result
            .credential
            .cookie_header()
            .unwrap()
            .contains("rotated")
    );
    assert!(result.credential.same_login(&credential));
    assert!(
        credential
            .cookie_header()
            .unwrap()
            .contains("selected-secret")
    );
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 1);
    let req = &requests[0];
    assert!(req.starts_with("GET /luna/pc/albums/900?"));
    assert!(req.contains("sessionid_ss=selected-secret"));
    let url = origin
        .join(
            req.lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap(),
        )
        .unwrap();
    let q = url.query_pairs().collect::<BTreeMap<_, _>>();
    assert_eq!(q["ignore_tracks"], "false");
    assert_eq!(q["version_code"], "20010000");
    assert_eq!(q["app_name"], "luna_pc");
    assert_ne!(q["device_id"], q["iid"]);
    assert!(!q.contains_key("user_id"));
    assert!(!q.contains_key("cursor"));
}

#[tokio::test]
async fn account_album_sdk_rejects_transport_and_cookie_failure_without_retry() {
    let credential = SodaCredential::test_credential("selected-secret")
        .bind_user("123456")
        .unwrap();
    let body = test_fixture().to_string();
    for response in [
        "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".to_owned(),
        "HTTP/1.1 302 Found\r\nLocation: https://example.invalid/\r\nContent-Length: 0\r\n\r\n"
            .to_owned(),
        crate::test_http::json(&body, None).replace("application/json", "text/html"),
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            MAX_API_RESPONSE_BYTES + 1
        ),
        crate::test_http::json(&body, Some("sessionid_ss=; Max-Age=0; Path=/")),
    ] {
        let (origin, server) = crate::test_http::serve(vec![response]).await;
        let client = SodaClient::new(&SodaConfig::default())
            .unwrap()
            .with_auth_test_origin(origin);
        assert!(client.account_album("900", &credential).await.is_err());
        assert_eq!(server.await.unwrap().len(), 1);
        assert!(
            credential
                .cookie_header()
                .unwrap()
                .contains("selected-secret")
        );
    }
}

#[tokio::test]
async fn account_album_sdk_requires_valid_identity_and_bound_session_before_device_io() {
    let path = std::env::temp_dir().join(format!("soda-album-device-{}", rand::random::<u64>()));
    let client = SodaClient::new(&SodaConfig {
        device_path: Some(path.clone()),
        ..SodaConfig::default()
    })
    .unwrap();
    let unbound = SodaCredential::test_credential("secret");
    assert_eq!(
        client
            .account_album("900", &unbound)
            .await
            .err()
            .unwrap()
            .code,
        ErrorCode::AuthenticationRequired
    );
    let bound = unbound.bind_user("123456").unwrap();
    for id in ["0", "0900", " 900", "900/track"] {
        assert_eq!(
            client.account_album(id, &bound).await.err().unwrap().code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(!path.exists());
}
