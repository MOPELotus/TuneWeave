use super::*;

fn created_library_reply(playlists: serde_json::Value, total: usize, cookie: &str) -> String {
    crate::test_http::json(
        &json!({
            "status_code": 0,
            "playlists": playlists,
            "total_num": total,
            "has_more": false,
        })
        .to_string(),
        Some(&format!("sessionid_ss={cookie}; Path=/")),
    )
}

fn owned_playlist(owner: Option<&str>) -> serde_json::Value {
    let mut playlist = json!({
        "id": "42",
        "title": "Owned playlist",
        "count_tracks": 2,
    });
    if let Some(owner) = owner {
        playlist["owner"] = json!({"id": owner});
    }
    playlist
}

fn retained_playlist() -> serde_json::Value {
    json!({
        "id": "99",
        "title": "Unchanged playlist",
        "count_tracks": 1,
        "owner": {"id": "123456"},
    })
}

#[tokio::test]
async fn playlist_delete_uses_the_selected_account_and_confirms_exact_created_directory() {
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
            created_library_reply(
                json!([owned_playlist(Some("123456")), retained_playlist()]),
                2,
                "listed",
            ),
            crate::test_http::json(r#"{"status_code":0}"#, Some("sessionid_ss=deleted; Path=/")),
            created_library_reply(json!([retained_playlist()]), 1, "readback"),
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
            .delete_playlists(&tuneweave_core::PlaylistDeleteRequest {
                playlist_refs: vec![
                    tuneweave_core::ResourceRef::new(Platform::Soda, "42").unwrap(),
                ],
                account: (!caller).then(|| "personal".to_owned()),
            })
            .await
            .unwrap();
        assert_eq!(result.playlist_refs.len(), 1);
        assert_eq!(result.playlist_refs[0].id(), "42");
        assert_eq!(result.extensions["source_user_id"], "123456");
        assert_eq!(
            result.extensions["verified_by"],
            "complete_created_library_readback"
        );
        assert_eq!(result.extensions["write_requests_dispatched"], 1);
        assert_eq!(result.extensions["atomic"], false);

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests[0].starts_with("GET /luna/pc/me?"));
        assert!(requests[0].contains("sessionid_ss=session-secret"));
        assert!(requests[1].starts_with("GET /luna/pc/me/playlist?"));
        assert!(requests[1].contains("sessionid_ss=verified"));
        assert!(requests[2].starts_with("POST /luna/pc/me/playlist/delete?"));
        assert!(requests[2].contains("sessionid_ss=listed"));
        let body = requests[2].split_once("\r\n\r\n").unwrap().1;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            json!({"playlist_ids":["42"]})
        );
        assert!(requests[3].starts_with("GET /luna/pc/me/playlist?"));
        assert!(requests[3].contains("sessionid_ss=deleted"));

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
        assert_eq!(query["version_name"], "3.7.0");
        assert_eq!(query["version_code"], "30070000");
        assert_eq!(query["channel"], "official");
        assert_eq!(query["fp"], query["device_id"]);
        assert!(!query["device_id"].is_empty());
        assert_eq!(query["iid"], "");
        assert!(!query.contains_key("install_id"));

        if caller {
            assert_eq!(
                fixture.stored("personal").unwrap().secret(),
                source.serialize().unwrap()
            );
            let response = provider.take_response_credential().unwrap().unwrap();
            assert!(response.secret().contains("readback"));
            assert!(!response.secret().contains("other-secret"));
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
async fn playlist_delete_rejects_unowned_targets_and_batches_before_write() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", None),
        created_library_reply(json!([owned_playlist(None)]), 1, "listed"),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let request =
        |playlist_refs: Vec<tuneweave_core::ResourceRef>| tuneweave_core::PlaylistDeleteRequest {
            playlist_refs,
            account: Some("personal".to_owned()),
        };
    let id = tuneweave_core::ResourceRef::new(Platform::Soda, "42").unwrap();
    assert_eq!(
        fixture
            .provider
            .delete_playlists(&request(vec![id.clone()]))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        fixture
            .provider
            .delete_playlists(&request(vec![id.clone(), id.clone()]))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert_eq!(server.await.unwrap().len(), 2);
}

#[tokio::test]
async fn playlist_delete_unconfirmed_readback_is_non_retryable() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", None),
        created_library_reply(json!([owned_playlist(Some("123456"))]), 1, "listed"),
        crate::test_http::json(r#"{"status_code":0}"#, None),
        created_library_reply(json!([owned_playlist(Some("123456"))]), 1, "still-present"),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let error = fixture
        .provider
        .delete_playlists(&tuneweave_core::PlaylistDeleteRequest {
            playlist_refs: vec![tuneweave_core::ResourceRef::new(Platform::Soda, "42").unwrap()],
            account: Some("personal".to_owned()),
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert!(!error.retryable);
    assert_eq!(error.details["write_outcome"], "unconfirmed");
    assert_eq!(server.await.unwrap().len(), 4);
}

#[tokio::test]
async fn playlist_delete_rejects_invalid_shape_before_credentials_or_network() {
    let provider = SodaProvider::from_client(crate::client::SodaClient::test_client());
    for playlist_refs in [
        Vec::new(),
        vec![tuneweave_core::ResourceRef::new(Platform::Soda, "042").unwrap()],
        vec![tuneweave_core::ResourceRef::new(Platform::Migu, "42").unwrap()],
    ] {
        let error = provider
            .delete_playlists(&tuneweave_core::PlaylistDeleteRequest {
                playlist_refs,
                account: None,
            })
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
    }
}
