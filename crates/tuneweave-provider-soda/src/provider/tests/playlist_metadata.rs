use super::*;

fn rename_request(caller: bool, name: &str) -> tuneweave_core::PlaylistUpdateRequest {
    metadata_request(caller, Some(name), None)
}

fn metadata_request(
    caller: bool,
    name: Option<&str>,
    description: Option<&str>,
) -> tuneweave_core::PlaylistUpdateRequest {
    tuneweave_core::PlaylistUpdateRequest {
        name: name.map(str::to_owned),
        description: description.map(str::to_owned),
        tags: None,
        variant: tuneweave_core::PlaylistMetadataUpdateVariant::Default,
        account: (!caller).then(|| "personal".to_owned()),
    }
}

fn created_library_reply(name: &str, description: &str, cookie: &str) -> String {
    crate::test_http::json(
        &json!({
            "status_code": 0,
            "playlists": [{
                "id": "42",
                "title": name,
                "desc": description,
                "count_tracks": 3,
                "owner": {"id": "123456"},
            }],
            "total_num": 1,
            "has_more": false,
        })
        .to_string(),
        Some(&format!("sessionid_ss={cookie}; Path=/")),
    )
}

fn rename_ack(status: i64, cookie: &str) -> String {
    crate::test_http::json(
        &json!({"status_code": status}).to_string(),
        Some(&format!("sessionid_ss={cookie}; Path=/")),
    )
}

#[tokio::test]
async fn playlist_metadata_rename_uses_selected_account_and_confirms_preserved_fields() {
    for caller in [false, true] {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        let other = SodaCredential::test_credential("other-secret")
            .bind_user("654321")
            .unwrap();
        fixture.put("personal", &source);
        fixture.put("other", &other);
        let (origin, server) = crate::test_http::serve(vec![
            account_reply("123456", Some("sessionid_ss=verified; Path=/")),
            created_library_reply("Before title", "preserved description", "before"),
            rename_ack(0, "updated"),
            created_library_reply("After title", "preserved description", "readback"),
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

        let result = provider
            .update_playlist("42", &rename_request(caller, "After title"))
            .await
            .unwrap();
        assert_eq!(
            result.action,
            tuneweave_core::PlaylistMutationAction::Update
        );
        assert_eq!(result.playlist_ref.id(), "42");
        let updated = result.playlist.unwrap();
        assert_eq!(updated.name, "After title");
        assert_eq!(updated.description, "preserved description");
        assert_eq!(updated.track_count, Some(3));
        assert_eq!(updated.extensions["owner_id"], "123456");
        assert_eq!(updated.extensions["source_user_id"], "123456");
        assert_eq!(
            result.extensions["verified_by"],
            "complete_created_library_readback"
        );
        assert_eq!(result.extensions["updated_fields"], json!(["name"]));
        assert_eq!(result.extensions["write_requests_dispatched"], 1);

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests[0].starts_with("GET /luna/pc/me?"));
        assert!(requests[0].contains("sessionid_ss=session-secret"));
        assert!(requests[1].starts_with("GET /luna/pc/me/playlist?"));
        assert!(requests[1].contains("sessionid_ss=verified"));
        assert!(requests[2].starts_with("POST /luna/pc/me/playlist/update?"));
        assert!(requests[2].contains("sessionid_ss=before"));
        let body = requests[2].split_once("\r\n\r\n").unwrap().1;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            json!({"playlist_id":"42", "name":"After title"})
        );
        assert!(requests[3].starts_with("GET /luna/pc/me/playlist?"));
        assert!(requests[3].contains("sessionid_ss=updated"));
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

#[tokio::test]
async fn playlist_metadata_description_updates_and_clears_only_supplied_fields_for_selected_sources()
 {
    for caller in [false, true] {
        for (name, description) in [
            (None, Some("Changed description\nsecond line")),
            (Some("After title"), Some("Changed description")),
            (None, Some("")),
        ] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("personal", &source);
            let after_name = name.unwrap_or("Before title");
            let after_description = description.unwrap();
            let (origin, server) = crate::test_http::serve(vec![
                account_reply("123456", Some("sessionid_ss=verified; Path=/")),
                created_library_reply("Before title", "Before description", "before"),
                rename_ack(0, "updated"),
                created_library_reply(after_name, after_description, "readback"),
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

            let result = provider
                .update_playlist("42", &metadata_request(caller, name, description))
                .await
                .unwrap();
            let updated = result.playlist.unwrap();
            assert_eq!(updated.name, after_name);
            assert_eq!(updated.description, after_description);
            assert_eq!(updated.track_count, Some(3));
            assert_eq!(updated.extensions["owner_id"], "123456");
            assert_eq!(
                result.extensions["updated_fields"],
                if name.is_some() {
                    json!(["name", "description"])
                } else {
                    json!(["description"])
                }
            );

            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 4);
            assert!(requests[2].starts_with("POST /luna/pc/me/playlist/update?"));
            assert!(requests[2].contains("sessionid_ss=before"));
            let body = requests[2].split_once("\r\n\r\n").unwrap().1;
            let mut expected_body = json!({"playlist_id":"42"});
            if let Some(name) = name {
                expected_body["name"] = json!(name);
            }
            expected_body["description"] = json!(after_description);
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(body).unwrap(),
                expected_body
            );
            assert!(requests[3].starts_with("GET /luna/pc/me/playlist?"));
            assert!(requests[3].contains("sessionid_ss=updated"));

            if caller {
                let update = provider.take_response_credential().unwrap().unwrap();
                assert!(update.secret().contains("readback"));
            } else {
                assert!(
                    fixture
                        .stored("personal")
                        .unwrap()
                        .secret()
                        .contains("readback")
                );
            }
        }
    }
}

#[tokio::test]
async fn playlist_metadata_rejects_unsupported_shapes_before_io() {
    let provider = SodaProvider::from_client(crate::client::SodaClient::test_client());
    let mut invalid = vec![
        tuneweave_core::PlaylistUpdateRequest {
            name: Some("title".to_owned()),
            tags: Some(vec!["tag".to_owned()]),
            ..tuneweave_core::PlaylistUpdateRequest::new()
        },
        tuneweave_core::PlaylistUpdateRequest {
            name: Some("title".to_owned()),
            variant: tuneweave_core::PlaylistMetadataUpdateVariant::Individual,
            ..tuneweave_core::PlaylistUpdateRequest::new()
        },
    ];
    for request in invalid.drain(..) {
        assert_eq!(
            provider
                .update_playlist("42", &request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::CapabilityNotSupported
        );
    }
    assert_eq!(
        provider
            .update_playlist("42", &tuneweave_core::PlaylistUpdateRequest::new(),)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    for name in [
        String::new(),
        "x".repeat(31),
        "🎵".repeat(16),
        "line\nbreak".to_owned(),
    ] {
        let error = provider
            .update_playlist("42", &rename_request(false, &name))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }
    for valid in ["valid".to_owned(), "歌".repeat(30), "🎵".repeat(15)] {
        assert_eq!(
            provider
                .update_playlist("42", &rename_request(false, &valid))
                .await
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
    }
    for (name, description) in [
        (None, Some("description only")),
        (Some("title"), Some("description")),
        (None, Some("")),
        (Some("title"), Some("")),
    ] {
        assert_eq!(
            provider
                .update_playlist("42", &metadata_request(false, name, description))
                .await
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
    }
}

#[tokio::test]
async fn playlist_metadata_rename_requires_explicit_created_owner_before_dispatch() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", Some("sessionid_ss=verified; Path=/")),
        crate::test_http::json(
            r#"{"status_code":0,"playlists":[{"id":"42","title":"Foreign","desc":"x","owner":{"id":"654321"}}],"total_num":1,"has_more":false}"#,
            Some("sessionid_ss=listed; Path=/"),
        ),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let error = fixture
        .provider
        .update_playlist("42", &rename_request(false, "Not mine"))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert!(
        server
            .await
            .unwrap()
            .iter()
            .all(|request| { !request.starts_with("POST /luna/pc/me/playlist/update?") })
    );
}

#[tokio::test]
async fn playlist_metadata_rename_failed_ack_keeps_only_previously_verified_cookie() {
    for (status, expected, preserve_rotation) in [
        (9, ErrorCode::UpstreamError, true),
        (1_000_016, ErrorCode::AuthenticationRequired, false),
    ] {
        let source = test_soda_credential().bind_user("123456").unwrap();
        let (origin, server) = crate::test_http::serve(vec![
            account_reply("123456", Some("sessionid_ss=verified; Path=/")),
            created_library_reply("Before title", "description", "before"),
            rename_ack(status, "poison"),
        ])
        .await;
        let provider = SodaProvider::from_client(
            crate::client::SodaClient::new(&SodaConfig::default())
                .unwrap()
                .with_auth_test_origin(origin),
        )
        .caller_credential_scope(&caller_from(&source))
        .unwrap();
        let error = provider
            .update_playlist("42", &rename_request(true, "After title"))
            .await
            .unwrap_err();
        assert_eq!(error.code, expected);
        assert!(!error.retryable);
        assert_eq!(error.details["write_outcome"], "unconfirmed");
        let update = provider.take_response_credential().unwrap();
        if preserve_rotation {
            let update = update.unwrap();
            assert!(update.secret().contains("before"));
            assert!(!update.secret().contains("poison"));
        } else {
            assert!(update.is_none());
        }
        assert_eq!(server.await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn playlist_metadata_rename_readback_mismatch_is_unconfirmed_and_never_retried() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", Some("sessionid_ss=verified; Path=/")),
        created_library_reply("Before title", "description", "before"),
        rename_ack(0, "updated"),
        created_library_reply("Unexpected title", "description", "readback"),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let error = fixture
        .provider
        .update_playlist("42", &rename_request(false, "After title"))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert!(!error.retryable);
    assert_eq!(error.details["write_outcome"], "unconfirmed");
    let requests = server.await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.starts_with("POST /luna/pc/me/playlist/update?"))
            .count(),
        1
    );
}

#[tokio::test]
async fn playlist_metadata_rename_late_ack_cannot_cross_selected_account_generation() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let paused = crate::test_http::serve_paused_at(
        vec![
            account_reply("123456", Some("sessionid_ss=verified; Path=/")),
            created_library_reply("Before title", "description", "before"),
            rename_ack(0, "poison"),
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
            .update_playlist("42", &rename_request(false, "After title"))
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
