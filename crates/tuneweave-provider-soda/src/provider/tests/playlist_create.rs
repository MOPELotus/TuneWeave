use super::*;

fn request(
    caller: bool,
    visibility: tuneweave_core::PlaylistVisibility,
) -> tuneweave_core::PlaylistCreateRequest {
    tuneweave_core::PlaylistCreateRequest {
        name: "新歌单".to_owned(),
        visibility,
        kind: tuneweave_core::PlaylistKind::Normal,
        account: (!caller).then(|| "personal".to_owned()),
    }
}

fn created_library_reply(id: &str, name: &str, owner: &str, cookie: &str) -> String {
    crate::test_http::json(
        &json!({
            "status_code": 0,
            "playlists": [{
                "id": id,
                "title": name,
                "owner": {"id": owner},
            }],
            "total_num": 1,
            "has_more": false,
        })
        .to_string(),
        Some(&format!("sessionid_ss={cookie}; Path=/")),
    )
}

fn account_playlist_reply(
    id: &str,
    name: &str,
    owner: &str,
    playlist_type: i64,
    track_count: usize,
    cookie: &str,
) -> String {
    let mut page = crate::client::test_account_playlist_fixture();
    page["playlist"]["id"] = json!(id);
    page["playlist"]["title"] = json!(name);
    page["playlist"]["public_title"] = json!(name);
    page["playlist"]["type"] = json!(playlist_type);
    page["playlist"]["count_tracks"] = json!(track_count);
    page["playlist"]["resource_cnt"]["track_cnt"] = json!(track_count);
    page["playlist"]["owner"]["id"] = json!(owner);
    page["media_resources"] = if track_count == 0 {
        json!([])
    } else {
        json!([page["media_resources"][0].clone()])
    };
    page["has_more"] = json!(false);
    crate::test_http::json(
        &page.to_string(),
        Some(&format!("sessionid_ss={cookie}; Path=/")),
    )
}

fn create_ack(id: &str, cookie: &str) -> String {
    crate::test_http::json(
        &json!({"status_code": 0, "playlist": {"id": id}}).to_string(),
        Some(&format!("sessionid_ss={cookie}; Path=/")),
    )
}

#[tokio::test]
async fn playlist_create_creates_only_empty_normal_playlists_for_the_selected_source() {
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
                create_ack("42", "created"),
                created_library_reply("42", "新歌单", "123456", "readback"),
                account_playlist_reply("42", "新歌单", "123456", 2, 0, "detail"),
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

            assert!(provider.capabilities().contains(&Capability::PlaylistWrite));
            let result = provider
                .create_playlist(&request(caller, visibility))
                .await
                .unwrap();
            assert_eq!(
                result.action,
                tuneweave_core::PlaylistMutationAction::Create
            );
            assert_eq!(result.playlist_ref.id(), "42");
            let playlist = result.playlist.unwrap();
            assert_eq!(playlist.id, "42");
            assert_eq!(playlist.name, "新歌单");
            assert_eq!(playlist.track_count, Some(0));
            assert_eq!(playlist.extensions["source_user_id"], "123456");
            assert!(!playlist.extensions.contains_key("visibility"));
            assert_eq!(result.extensions["source_user_id"], "123456");
            assert_eq!(result.extensions["empty_playlist_only"], true);
            assert_eq!(result.extensions["visibility_verified"], false);
            assert_eq!(
                result.extensions["requested_visibility"],
                if visibility == tuneweave_core::PlaylistVisibility::Private {
                    "private"
                } else {
                    "public"
                }
            );

            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 4);
            assert!(requests[0].starts_with("GET /luna/pc/me?"));
            assert!(requests[0].contains("sessionid_ss=session-secret"));
            assert!(requests[1].starts_with("POST /luna/pc/me/playlist?"));
            assert!(requests[1].contains("sessionid_ss=verified"));
            let body = requests[1].split_once("\r\n\r\n").unwrap().1;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(body).unwrap(),
                json!({
                    "name":"新歌单",
                    "is_private": visibility == tuneweave_core::PlaylistVisibility::Private,
                    "track_ids": [],
                })
            );
            assert!(requests[2].starts_with("GET /luna/pc/me/playlist?"));
            assert!(requests[2].contains("sessionid_ss=created"));
            assert!(requests[3].starts_with("GET /luna/pc/playlist/detail?"));
            assert!(requests[3].contains("sessionid_ss=readback"));

            if caller {
                assert_eq!(
                    fixture.stored("personal").unwrap().secret(),
                    source.serialize().unwrap()
                );
                let update = provider.take_response_credential().unwrap().unwrap();
                assert!(update.secret().contains("detail"));
                assert!(!update.secret().contains("other-secret"));
            } else {
                assert!(
                    fixture
                        .stored("personal")
                        .unwrap()
                        .secret()
                        .contains("detail")
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
async fn playlist_create_rejects_a_nonempty_complete_detail_readback() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", Some("sessionid_ss=verified; Path=/")),
        create_ack("42", "created"),
        created_library_reply("42", "新歌单", "123456", "readback"),
        account_playlist_reply("42", "新歌单", "123456", 2, 1, "detail"),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);

    let error = fixture
        .provider
        .create_playlist(&request(false, tuneweave_core::PlaylistVisibility::Private))
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert!(!error.retryable);
    assert_eq!(error.details["write_outcome"], "unconfirmed");
    assert_eq!(server.await.unwrap().len(), 4);
}

#[tokio::test]
async fn playlist_create_rejects_detail_owned_by_another_account() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", Some("sessionid_ss=verified; Path=/")),
        create_ack("42", "created"),
        created_library_reply("42", "新歌单", "123456", "readback"),
        account_playlist_reply("42", "新歌单", "654321", 2, 0, "detail"),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);

    let error = fixture
        .provider
        .create_playlist(&request(false, tuneweave_core::PlaylistVisibility::Private))
        .await
        .unwrap_err();

    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert!(!error.retryable);
    assert_eq!(error.details["write_outcome"], "unconfirmed");
    assert_eq!(server.await.unwrap().len(), 4);
}

#[tokio::test]
async fn playlist_create_rejects_unsupported_shapes_before_credentials_or_network() {
    let provider = SodaProvider::from_client(crate::client::SodaClient::test_client());
    let mut invalid = vec![
        tuneweave_core::PlaylistCreateRequest {
            kind: tuneweave_core::PlaylistKind::Video,
            ..tuneweave_core::PlaylistCreateRequest::new("video")
        },
        tuneweave_core::PlaylistCreateRequest {
            kind: tuneweave_core::PlaylistKind::Shared,
            ..tuneweave_core::PlaylistCreateRequest::new("shared")
        },
        tuneweave_core::PlaylistCreateRequest {
            visibility: tuneweave_core::PlaylistVisibility::PlatformDefault,
            ..tuneweave_core::PlaylistCreateRequest::new("default")
        },
        tuneweave_core::PlaylistCreateRequest::new(""),
        tuneweave_core::PlaylistCreateRequest::new("x".repeat(31)),
        tuneweave_core::PlaylistCreateRequest::new("line\nbreak"),
    ];
    invalid.push(tuneweave_core::PlaylistCreateRequest::new("🎵".repeat(16)));
    invalid.push(tuneweave_core::PlaylistCreateRequest::new("歌".repeat(31)));
    for request in invalid {
        assert_eq!(
            provider.create_playlist(&request).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    for valid_name in ["valid".to_owned(), "歌".repeat(30), "🎵".repeat(15)] {
        assert_eq!(
            provider
                .create_playlist(&tuneweave_core::PlaylistCreateRequest::new(valid_name))
                .await
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
    }
}

#[tokio::test]
async fn playlist_create_errors_never_accept_unvalidated_cookies_and_expiry_clears_prior_rotation()
{
    for (status, expected, expect_rotation) in [
        (9, ErrorCode::UpstreamError, true),
        (1_000_016, ErrorCode::AuthenticationRequired, false),
    ] {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        fixture.put("personal", &source);
        let (origin, server) = crate::test_http::serve(vec![
            account_reply("123456", Some("sessionid_ss=verified; Path=/")),
            crate::test_http::json(
                &json!({"status_code": status, "playlist": {"id": "42"}}).to_string(),
                Some("sessionid_ss=poison; Path=/"),
            ),
        ])
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);
        let caller = fixture
            .provider
            .caller_credential_scope(&caller_from(&source))
            .unwrap();
        let error = caller
            .create_playlist(&request(true, tuneweave_core::PlaylistVisibility::Public))
            .await
            .unwrap_err();
        assert_eq!(error.code, expected);
        assert!(!error.retryable);
        assert_eq!(error.details["write_outcome"], "unconfirmed");
        let update = caller.take_response_credential().unwrap();
        if expect_rotation {
            let update = update.unwrap();
            assert!(update.secret().contains("verified"));
            assert!(!update.secret().contains("poison"));
        } else {
            assert!(update.is_none());
        }
        assert_eq!(server.await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn playlist_create_late_ack_cannot_cross_selected_account_generation() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let paused = crate::test_http::serve_paused_at(
        vec![
            account_reply("123456", Some("sessionid_ss=verified; Path=/")),
            create_ack("42", "poison"),
        ],
        1,
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
            .create_playlist(&request(false, tuneweave_core::PlaylistVisibility::Public))
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
    assert_eq!(paused.requests.await.unwrap().len(), 2);
}
