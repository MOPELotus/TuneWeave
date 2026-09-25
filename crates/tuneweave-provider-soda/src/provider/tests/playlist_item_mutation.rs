use super::*;

fn owned_created_playlist(track_count: usize, kind: i64) -> serde_json::Value {
    json!({
        "id": "42",
        "title": "Owned playlist",
        "desc": "Preserved description",
        "type": kind,
        "count_tracks": track_count,
        "owner": {"id": "123456"},
    })
}

fn created_library_reply(track_count: usize, cookie: &str) -> String {
    crate::test_http::json(
        &json!({
            "status_code": 0,
            "playlists": [owned_created_playlist(track_count, 0)],
            "total_num": 1,
            "has_more": false,
        })
        .to_string(),
        Some(&format!("sessionid_ss={cookie}; Path=/")),
    )
}

fn account_playlist_reply(ids: &[&str], cookie: &str) -> String {
    let mut page = crate::client::test_account_playlist_fixture();
    let template = page["media_resources"][0].clone();
    page["playlist"]["id"] = json!("42");
    page["playlist"]["title"] = json!("Owned playlist");
    page["playlist"]["desc"] = json!("Preserved description");
    page["playlist"]["type"] = json!(0);
    page["playlist"]["owner"]["id"] = json!("123456");
    page["playlist"]["count_tracks"] = json!(ids.len());
    page["playlist"]["resource_cnt"]["track_cnt"] = json!(ids.len());
    page["media_resources"] = ids
        .iter()
        .map(|id| {
            let mut item = template.clone();
            item["id"] = json!(id);
            item["entity"]["track_wrapper"]["track"]["id"] = json!(id);
            item
        })
        .collect();
    page["has_more"] = json!(false);
    crate::test_http::json(
        &page.to_string(),
        Some(&format!("sessionid_ss={cookie}; Path=/")),
    )
}

fn mutation_request(
    caller: bool,
    item_ids: &[&str],
) -> tuneweave_core::PlaylistItemMutationRequest {
    tuneweave_core::PlaylistItemMutationRequest {
        item_refs: item_ids
            .iter()
            .map(|id| tuneweave_core::ResourceRef::new(Platform::Soda, *id).unwrap())
            .collect(),
        kind: tuneweave_core::PlaylistItemKind::Track,
        account: (!caller).then(|| "personal".to_owned()),
    }
}

#[tokio::test]
async fn playlist_track_mutations_use_the_selected_source_and_confirm_ordered_readback() {
    for caller in [false, true] {
        for (action, request_ids, before, after) in [
            (
                tuneweave_core::PlaylistItemMutationAction::Add,
                vec!["13", "14"],
                vec!["11", "12"],
                vec!["11", "12", "13", "14"],
            ),
            (
                tuneweave_core::PlaylistItemMutationAction::Remove,
                vec!["12", "13"],
                vec!["11", "12", "13", "12"],
                vec!["11"],
            ),
        ] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            let other = SodaCredential::test_credential("other-secret")
                .bind_user("654321")
                .unwrap();
            fixture.put("personal", &source);
            fixture.put("other", &other);
            let before_refs = before.to_vec();
            let after_refs = after.to_vec();
            let (origin, server) = crate::test_http::serve(vec![
                account_reply("123456", Some("sessionid_ss=verified; Path=/")),
                created_library_reply(before.len(), "created-before"),
                account_playlist_reply(&before_refs, "tracks-before"),
                crate::test_http::json(
                    r#"{"status_code":0}"#,
                    Some("sessionid_ss=written; Path=/"),
                ),
                account_playlist_reply(&after_refs, "tracks-after"),
                created_library_reply(after.len(), "created-after"),
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
                .mutate_playlist_items("42", action, &mutation_request(caller, &request_ids))
                .await
                .unwrap();
            assert_eq!(result.playlist_ref.id(), "42");
            assert_eq!(
                result.item_refs,
                mutation_request(caller, &request_ids).item_refs
            );
            assert_eq!(result.kind, tuneweave_core::PlaylistItemKind::Track);
            assert_eq!(result.action, action);
            assert_eq!(result.cloud_track_count, Some(after.len() as u64));
            assert!(
                result
                    .snapshot_id
                    .as_deref()
                    .unwrap()
                    .starts_with("soda_playlist_v1_")
            );
            assert_eq!(result.extensions["source_user_id"], "123456");
            assert_eq!(
                result.extensions["verified_by"],
                "complete_before_after_playlist_and_created_library_readback"
            );
            assert_eq!(result.extensions["existing_track_order_preserved"], true);
            assert_eq!(result.extensions["write_requests_dispatched"], 1);
            assert_eq!(result.extensions["atomic"], false);

            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 6);
            assert!(requests[0].starts_with("GET /luna/pc/me?"));
            assert!(requests[0].contains("sessionid_ss=session-secret"));
            assert!(requests[1].starts_with("GET /luna/pc/me/playlist?"));
            assert!(requests[1].contains("sessionid_ss=verified"));
            assert!(requests[2].starts_with("GET /luna/pc/playlist/detail?"));
            assert!(requests[2].contains("sessionid_ss=created-before"));
            let expected_path = match action {
                tuneweave_core::PlaylistItemMutationAction::Add => {
                    "/luna/pc/me/playlist/media/append?"
                }
                tuneweave_core::PlaylistItemMutationAction::Remove => {
                    "/luna/pc/me/playlist/media/delete?"
                }
            };
            assert!(requests[3].starts_with(&format!("POST {expected_path}")));
            let body = requests[3].split_once("\r\n\r\n").unwrap().1;
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(body).unwrap(),
                json!({
                    "playlist_id":"42",
                    "media": request_ids.iter().map(|id| json!({"id":id,"type":"track"})).collect::<Vec<_>>(),
                })
            );
            assert!(requests[3].contains("sessionid_ss=tracks-before"));
            assert!(requests[4].starts_with("GET /luna/pc/playlist/detail?"));
            assert!(requests[4].contains("sessionid_ss=written"));
            assert!(requests[5].starts_with("GET /luna/pc/me/playlist?"));
            assert!(requests[5].contains("sessionid_ss=tracks-after"));

            let write_url = origin
                .join(
                    requests[3]
                        .lines()
                        .next()
                        .unwrap()
                        .split_whitespace()
                        .nth(1)
                        .unwrap(),
                )
                .unwrap();
            let query = write_url.query_pairs().collect::<BTreeMap<_, _>>();
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

            if caller {
                assert_eq!(
                    fixture.stored("personal").unwrap().secret(),
                    source.serialize().unwrap()
                );
                let response = provider.take_response_credential().unwrap().unwrap();
                assert!(response.secret().contains("created-after"));
                assert!(!response.secret().contains("other-secret"));
            } else {
                assert!(
                    fixture
                        .stored("personal")
                        .unwrap()
                        .secret()
                        .contains("created-after")
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
async fn playlist_track_mutation_rejects_unsupported_inputs_before_network() {
    let provider = SodaProvider::from_client(crate::client::SodaClient::test_client());
    let item = tuneweave_core::ResourceRef::new(Platform::Soda, "11").unwrap();
    for (request, code) in [
        (
            tuneweave_core::PlaylistItemMutationRequest::new(
                vec![item.clone()],
                tuneweave_core::PlaylistItemKind::Video,
            ),
            ErrorCode::CapabilityNotSupported,
        ),
        (
            tuneweave_core::PlaylistItemMutationRequest::new(
                Vec::new(),
                tuneweave_core::PlaylistItemKind::Track,
            ),
            ErrorCode::InvalidRequest,
        ),
        (
            tuneweave_core::PlaylistItemMutationRequest::new(
                vec![item.clone(), item.clone()],
                tuneweave_core::PlaylistItemKind::Track,
            ),
            ErrorCode::InvalidRequest,
        ),
        (
            tuneweave_core::PlaylistItemMutationRequest::new(
                vec![tuneweave_core::ResourceRef::new(Platform::Migu, "11").unwrap()],
                tuneweave_core::PlaylistItemKind::Track,
            ),
            ErrorCode::InvalidRequest,
        ),
        (
            tuneweave_core::PlaylistItemMutationRequest::new(
                (1..=101)
                    .map(|id| {
                        tuneweave_core::ResourceRef::new(Platform::Soda, id.to_string()).unwrap()
                    })
                    .collect(),
                tuneweave_core::PlaylistItemKind::Track,
            ),
            ErrorCode::InvalidRequest,
        ),
    ] {
        assert_eq!(
            provider
                .mutate_playlist_items(
                    "42",
                    tuneweave_core::PlaylistItemMutationAction::Add,
                    &request
                )
                .await
                .unwrap_err()
                .code,
            code
        );
    }
    let maximum_batch = tuneweave_core::PlaylistItemMutationRequest::new(
        (1..=100)
            .map(|id| tuneweave_core::ResourceRef::new(Platform::Soda, id.to_string()).unwrap())
            .collect(),
        tuneweave_core::PlaylistItemKind::Track,
    );
    assert_eq!(
        provider
            .mutate_playlist_items(
                "42",
                tuneweave_core::PlaylistItemMutationAction::Add,
                &maximum_batch,
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired,
        "a 100-track batch passes local validation before account selection"
    );
}

#[tokio::test]
async fn playlist_track_mutation_requires_an_owned_ordinary_created_playlist() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    for kind in [1, 4] {
        let (origin, server) = crate::test_http::serve(vec![
            account_reply("123456", None),
            crate::test_http::json(
                &json!({
                    "status_code": 0,
                    "playlists": [owned_created_playlist(2, kind)],
                    "total_num": 1,
                    "has_more": false,
                })
                .to_string(),
                None,
            ),
        ])
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);
        assert_eq!(
            fixture
                .provider
                .mutate_playlist_items(
                    "42",
                    tuneweave_core::PlaylistItemMutationAction::Add,
                    &mutation_request(false, &["11"]),
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::CapabilityNotSupported
        );
        assert_eq!(server.await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn playlist_track_mutation_readback_failure_is_non_retryable_and_never_reposts() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", Some("sessionid_ss=verified; Path=/")),
        created_library_reply(2, "created-before"),
        account_playlist_reply(&["11", "12"], "tracks-before"),
        crate::test_http::json(r#"{"status_code":0}"#, Some("sessionid_ss=written; Path=/")),
        account_playlist_reply(&["11", "12"], "tracks-after"),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let error = fixture
        .provider
        .mutate_playlist_items(
            "42",
            tuneweave_core::PlaylistItemMutationAction::Add,
            &mutation_request(false, &["13"]),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert!(!error.retryable);
    assert_eq!(error.details["write_outcome"], "unconfirmed");
    assert_eq!(error.details["operation"], "playlist_track_mutation");
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 5);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.starts_with("POST /luna/pc/me/playlist/media/append?"))
            .count(),
        1
    );
}
