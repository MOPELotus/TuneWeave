use super::*;

fn visibility_request(
    caller: bool,
    visibility: tuneweave_core::PlaylistVisibility,
) -> tuneweave_core::PlaylistVisibilityUpdateRequest {
    tuneweave_core::PlaylistVisibilityUpdateRequest {
        visibility,
        account: (!caller).then(|| "personal".to_owned()),
    }
}

fn created_library_reply(owner: Option<&str>, cookie: &str) -> String {
    crate::test_http::json(
        &json!({
            "status_code": 0,
            "playlists": [{
                "id": "42",
                "title": "Owned playlist",
                "count_tracks": 3,
                "owner": owner.map(|id| json!({"id": id})),
            }],
            "total_num": 1,
            "has_more": false,
        })
        .to_string(),
        Some(&format!("sessionid_ss={cookie}; Path=/")),
    )
}

fn created_library_empty_reply(cookie: &str) -> String {
    crate::test_http::json(
        r#"{"status_code":0,"playlists":[],"total_num":0,"has_more":false}"#,
        Some(&format!("sessionid_ss={cookie}; Path=/")),
    )
}

fn visibility_ack(status: i64, cookie: &str) -> String {
    crate::test_http::json(
        &json!({"status_code": status}).to_string(),
        Some(&format!("sessionid_ss={cookie}; Path=/")),
    )
}

fn account_playlist_detail_reply(is_private: Option<serde_json::Value>, cookie: &str) -> String {
    let mut body = crate::client::test_account_playlist_fixture();
    body["playlist"]["id"] = json!("42");
    body["playlist"]["owner"]["id"] = json!("123456");
    body["playlist"]["type"] = json!(0);
    match is_private {
        Some(value) => body["playlist"]["is_private"] = value,
        None => {
            body["playlist"]
                .as_object_mut()
                .unwrap()
                .remove("is_private");
        }
    }
    crate::test_http::json(
        &body.to_string(),
        Some(&format!("sessionid_ss={cookie}; Path=/")),
    )
}

fn account_playlist_detail_page_reply(is_private: bool, has_more: bool, cookie: &str) -> String {
    let mut body = crate::client::test_account_playlist_fixture();
    body["playlist"]["id"] = json!("42");
    body["playlist"]["owner"]["id"] = json!("123456");
    body["playlist"]["type"] = json!(0);
    body["playlist"]["is_private"] = json!(is_private);
    body["playlist"]["count_tracks"] = json!(3);
    body["playlist"]["resource_cnt"]["track_cnt"] = json!(3);
    body["has_more"] = json!(has_more);
    if has_more {
        body["next_cursor"] = json!("100");
    } else {
        body["media_resources"] = json!([body["media_resources"][0].clone()]);
    }
    crate::test_http::json(
        &body.to_string(),
        Some(&format!("sessionid_ss={cookie}; Path=/")),
    )
}

#[tokio::test]
async fn playlist_visibility_update_uses_the_selected_pc_account_and_exact_wire_contract() {
    for caller in [false, true] {
        for visibility in [
            tuneweave_core::PlaylistVisibility::Public,
            tuneweave_core::PlaylistVisibility::Private,
        ] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            let other = SodaCredential::test_credential("other-secret")
                .bind_user("654321")
                .unwrap();
            fixture.put("personal", &source);
            fixture.put("other", &other);
            let (origin, server) = crate::test_http::serve(vec![
                account_reply("123456", Some("sessionid_ss=verified; Path=/")),
                created_library_reply(Some("123456"), "listed"),
                visibility_ack(0, "updated"),
                account_playlist_detail_reply(
                    Some(json!(
                        visibility == tuneweave_core::PlaylistVisibility::Private
                    )),
                    "readback",
                ),
            ])
            .await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(origin.clone());
            let provider = if caller {
                fixture
                    .provider
                    .caller_credential_scope(&caller_from(&source))
                    .unwrap()
            } else {
                fixture.provider.clone()
            };

            assert!(
                provider
                    .capabilities()
                    .contains(&Capability::PlaylistVisibilityWrite)
            );
            let result = provider
                .update_playlist_visibility("42", &visibility_request(caller, visibility))
                .await
                .unwrap();
            assert_eq!(
                result.action,
                tuneweave_core::PlaylistMutationAction::Update
            );
            assert_eq!(result.playlist_ref.id(), "42");
            let playlist = result.playlist.as_ref().unwrap();
            assert_eq!(playlist.id, "42");
            assert_eq!(playlist.extensions["owner_id"], "123456");
            assert_eq!(
                playlist.extensions["is_private"],
                visibility == tuneweave_core::PlaylistVisibility::Private
            );
            assert_eq!(result.extensions["source_user_id"], "123456");
            assert_eq!(
                result.extensions["verified_by"],
                "complete_account_playlist_detail_readback"
            );
            assert_eq!(result.extensions["visibility_verified"], true);
            assert_eq!(
                result.extensions["requested_visibility"],
                if visibility == tuneweave_core::PlaylistVisibility::Private {
                    "private"
                } else {
                    "public"
                }
            );
            assert_eq!(
                result.extensions["verified_visibility"],
                if visibility == tuneweave_core::PlaylistVisibility::Private {
                    "private"
                } else {
                    "public"
                }
            );
            assert_eq!(
                result.extensions["playlist_owner_verified_by"],
                "complete_created_library_readback"
            );

            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 4);
            assert!(requests[0].starts_with("GET /luna/pc/me?"));
            assert!(requests[0].contains("sessionid_ss=session-secret"));
            assert!(requests[1].starts_with("GET /luna/pc/me/playlist?"));
            assert!(requests[1].contains("sessionid_ss=verified"));
            assert!(requests[2].starts_with("POST /luna/pc/me/playlist/update?"));
            let lower_request = requests[2].to_ascii_lowercase();
            assert!(lower_request.contains("cookie: sessionid_ss=listed"));
            assert!(lower_request.contains("content-type: application/json"));
            let body = requests[2].split_once("\r\n\r\n").unwrap().1;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(body).unwrap(),
                json!({
                    "playlist_id": "42",
                    "is_private": visibility == tuneweave_core::PlaylistVisibility::Private,
                })
            );
            let url = origin
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
            let query = url.query_pairs().collect::<BTreeMap<_, _>>();
            assert_eq!(query["aid"], "386088");
            assert_eq!(query["app_name"], "luna_pc");
            assert_eq!(query["device_platform"], "windows");
            assert_eq!(query["version_name"], "2.1.0");
            assert_eq!(query["version_code"], "20010000");
            assert_eq!(query["channel"], "official");
            assert_eq!(query["fp"], query["device_id"]);
            assert!(!query["device_id"].is_empty());
            assert!(!query["iid"].is_empty());
            assert_ne!(query["device_id"], query["iid"]);
            assert!(!query.contains_key("user_id"));
            assert!(requests[3].starts_with("GET /luna/pc/playlist/detail?"));
            assert!(requests[3].contains("sessionid_ss=updated"));

            if caller {
                assert_eq!(
                    fixture.stored("personal").unwrap().secret(),
                    source.serialize().unwrap()
                );
                let update = provider.take_response_credential().unwrap().unwrap();
                assert!(update.secret().contains("readback"));
                assert!(!update.secret().contains("other-secret"));
            } else {
                assert!(
                    fixture
                        .stored("personal")
                        .unwrap()
                        .secret()
                        .contains("readback")
                );
            }
            assert_eq!(
                fixture.stored("other").unwrap().secret(),
                other.serialize().unwrap()
            );
        }
    }
}

#[tokio::test]
async fn playlist_visibility_write_rejects_missing_null_or_mismatched_readback() {
    for observed in [None, Some(json!(null)), Some(json!(false))] {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        fixture.put("personal", &source);
        let (origin, server) = crate::test_http::serve(vec![
            account_reply("123456", Some("sessionid_ss=verified; Path=/")),
            created_library_reply(Some("123456"), "listed"),
            visibility_ack(0, "updated"),
            account_playlist_detail_reply(observed, "readback"),
        ])
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);

        let error = fixture
            .provider
            .update_playlist_visibility(
                "42",
                &visibility_request(false, tuneweave_core::PlaylistVisibility::Private),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert_eq!(error.details["write_outcome"], "unconfirmed");
        assert_eq!(server.await.unwrap().len(), 4);
    }
}

#[tokio::test]
async fn playlist_visibility_readback_rejects_owner_mismatch() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let mut detail = crate::client::test_account_playlist_fixture();
    detail["playlist"]["id"] = json!("42");
    detail["playlist"]["owner"]["id"] = json!("654321");
    detail["playlist"]["type"] = json!(0);
    detail["playlist"]["is_private"] = json!(true);
    let detail = crate::test_http::json(&detail.to_string(), Some("sessionid_ss=readback; Path=/"));
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", Some("sessionid_ss=verified; Path=/")),
        created_library_reply(Some("123456"), "listed"),
        visibility_ack(0, "updated"),
        detail,
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);

    let error = fixture
        .provider
        .update_playlist_visibility(
            "42",
            &visibility_request(false, tuneweave_core::PlaylistVisibility::Private),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert_eq!(error.details["write_outcome"], "unconfirmed");
    assert_eq!(server.await.unwrap().len(), 4);
}

#[tokio::test]
async fn playlist_visibility_readback_rejects_cross_page_state_changes() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", Some("sessionid_ss=verified; Path=/")),
        created_library_reply(Some("123456"), "listed"),
        visibility_ack(0, "updated"),
        account_playlist_detail_page_reply(true, true, "first-detail"),
        account_playlist_detail_page_reply(false, false, "second-detail"),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);

    let error = fixture
        .provider
        .update_playlist_visibility(
            "42",
            &visibility_request(false, tuneweave_core::PlaylistVisibility::Private),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert_eq!(error.details["write_outcome"], "unconfirmed");
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 5);
    assert!(requests[3].starts_with("GET /luna/pc/playlist/detail?"));
    assert!(requests[3].contains("sessionid_ss=updated"));
    assert!(requests[4].contains("sessionid_ss=first-detail"));
}

#[tokio::test]
async fn playlist_visibility_update_requires_complete_owned_library_before_dispatch() {
    for listing in [
        created_library_empty_reply("listed"),
        created_library_reply(None, "listed"),
    ] {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        fixture.put("personal", &source);
        let (origin, server) = crate::test_http::serve(vec![
            account_reply("123456", Some("sessionid_ss=verified; Path=/")),
            listing,
        ])
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);
        let error = fixture
            .provider
            .update_playlist_visibility(
                "42",
                &visibility_request(false, tuneweave_core::PlaylistVisibility::Private),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::PermissionDenied);
        assert!(
            server
                .await
                .unwrap()
                .iter()
                .all(|request| { !request.starts_with("POST /luna/pc/me/playlist/update?") })
        );
    }
}

#[tokio::test]
async fn playlist_visibility_update_validates_target_before_credentials_or_network() {
    let provider = SodaProvider::from_client(crate::client::SodaClient::test_client());
    for (id, visibility) in [
        ("042", tuneweave_core::PlaylistVisibility::Private),
        ("42", tuneweave_core::PlaylistVisibility::PlatformDefault),
    ] {
        assert_eq!(
            provider
                .update_playlist_visibility(id, &visibility_request(false, visibility))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        provider
            .update_playlist_visibility(
                "42",
                &visibility_request(false, tuneweave_core::PlaylistVisibility::Private),
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
}

#[tokio::test]
async fn playlist_visibility_update_rejects_failed_ack_without_accepting_its_cookie() {
    for (status, expected, expect_rotation) in [
        (9, ErrorCode::UpstreamError, true),
        (1_000_016, ErrorCode::AuthenticationRequired, false),
    ] {
        let source = test_soda_credential().bind_user("123456").unwrap();
        let (origin, server) = crate::test_http::serve(vec![
            account_reply("123456", Some("sessionid_ss=verified; Path=/")),
            created_library_reply(Some("123456"), "listed"),
            visibility_ack(status, "poison"),
        ])
        .await;
        // Construct a caller-scoped fixture from the same selected login so the wire
        // source remains isolated from any server-side aliases.
        let provider = SodaProvider::from_client(
            crate::client::SodaClient::new(&SodaConfig::default())
                .unwrap()
                .with_auth_test_origin(origin),
        )
        .caller_credential_scope(&caller_from(&source))
        .unwrap();
        let error = provider
            .update_playlist_visibility(
                "42",
                &visibility_request(true, tuneweave_core::PlaylistVisibility::Private),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, expected);
        assert!(!error.retryable);
        assert_eq!(error.details["write_outcome"], "unconfirmed");
        let update = provider.take_response_credential().unwrap();
        if expect_rotation {
            let update = update.unwrap();
            assert!(update.secret().contains("listed"));
            assert!(!update.secret().contains("poison"));
        } else {
            assert!(update.is_none());
        }
        assert_eq!(server.await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn playlist_visibility_update_late_ack_cannot_cross_selected_account_generation() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let paused = crate::test_http::serve_paused_at(
        vec![
            account_reply("123456", Some("sessionid_ss=verified; Path=/")),
            created_library_reply(Some("123456"), "listed"),
            visibility_ack(0, "poison"),
        ],
        2,
    )
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(paused.origin.clone());
    let provider = fixture.provider.clone();
    let task = tokio::spawn(async move {
        provider
            .update_playlist_visibility(
                "42",
                &visibility_request(false, tuneweave_core::PlaylistVisibility::Private),
            )
            .await
    });
    paused.arrived.await.unwrap();
    let replacement = SodaCredential::test_credential("new-login")
        .bind_user("123456")
        .unwrap();
    fixture.put("personal", &replacement);
    paused.release.send(()).unwrap();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(!error.retryable);
    assert_eq!(error.details["write_outcome"], "unconfirmed");
    assert_eq!(
        fixture.stored("personal").unwrap().secret(),
        replacement.serialize().unwrap()
    );
    assert!(
        fixture
            .provider
            .take_response_credential()
            .unwrap()
            .is_none()
    );
    assert_eq!(paused.requests.await.unwrap().len(), 3);
}
