use super::*;

fn created_library_reply(track_count: usize, cookie: &str) -> String {
    crate::test_http::json(
        &json!({
            "status_code": 0,
            "playlists": [{
                "id": "42",
                "title": "Owned playlist",
                "desc": "Preserved description",
                "type": 2,
                "count_tracks": track_count,
                "owner": {"id": "123456"}
            }],
            "total_num": 1,
            "has_more": false
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
    page["playlist"]["type"] = json!(2);
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

fn order_request(caller: bool, track_ids: &[&str]) -> tuneweave_core::PlaylistTrackOrderRequest {
    tuneweave_core::PlaylistTrackOrderRequest {
        track_refs: track_ids
            .iter()
            .map(|id| tuneweave_core::ResourceRef::new(Platform::Soda, *id).unwrap())
            .collect(),
        account: (!caller).then(|| "personal".to_owned()),
    }
}

fn sort_ack(cookie: &str) -> String {
    crate::test_http::json(
        &json!({ "status_code": 0 }).to_string(),
        Some(&format!("sessionid_ss={cookie}; Path=/")),
    )
}

#[tokio::test]
async fn soda_manual_track_order_uses_selected_session_and_confirms_full_readback() {
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
            created_library_reply(3, "created-before"),
            account_playlist_reply(&["11", "12", "11"], "tracks-before"),
            sort_ack("sorted"),
            account_playlist_reply(&["11", "11", "12"], "tracks-after"),
            created_library_reply(3, "created-after"),
        ])
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);
        let provider = if caller {
            fixture
                .provider
                .caller_credential_scope(&caller_from(&source))
                .unwrap()
        } else {
            fixture.provider.clone()
        };
        let request = order_request(caller, &["11", "11", "12"]);
        let result = provider
            .reorder_playlist_tracks("42", &request)
            .await
            .unwrap();

        assert_eq!(result.playlist_ref.id(), "42");
        assert_eq!(result.track_refs, request.track_refs);
        assert!(
            result
                .snapshot_id
                .as_deref()
                .unwrap()
                .starts_with("soda_playlist_v1_")
        );
        assert_eq!(
            result.extensions["backend"],
            "official_android_playlist_media_sort"
        );
        assert_eq!(
            result.extensions["verified_by"],
            "sort_ack_and_complete_playlist_and_created_library_readback"
        );
        assert_eq!(result.extensions["source_user_id"], "123456");
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
        assert!(
            requests[3]
                .starts_with("POST /luna/me/playlist/media/sort?aid=386088&app_name=luna_pc")
        );
        assert!(requests[3].contains("content-type: application/json; charset=utf-8"));
        assert!(requests[3].contains("x-luna-background-type: foreground"));
        assert!(requests[3].contains("x-luna-is-background-req: 0"));
        assert!(requests[3].contains("x-luna-is-local-user: 1"));
        assert!(requests[3].contains("x-ss-stub: "));
        assert!(requests[3].contains("sessionid_ss=tracks-before"));
        let body = requests[3].split_once("\r\n\r\n").unwrap().1;
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(body).unwrap(),
            json!({
                "playlist_id": "42",
                "media": [
                    {"id":"11", "type":"track"},
                    {"id":"11", "type":"track"},
                    {"id":"12", "type":"track"}
                ]
            })
        );
        assert!(requests[4].starts_with("GET /luna/pc/playlist/detail?"));
        assert!(requests[4].contains("sessionid_ss=sorted"));
        assert!(requests[5].starts_with("GET /luna/pc/me/playlist?"));
        assert!(requests[5].contains("sessionid_ss=tracks-after"));

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

#[tokio::test]
async fn soda_manual_track_order_requires_an_exact_full_occurrence_multiset_before_write() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", Some("sessionid_ss=verified; Path=/")),
        created_library_reply(3, "created-before"),
        account_playlist_reply(&["11", "12", "11"], "tracks-before"),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let error = fixture
        .provider
        .reorder_playlist_tracks("42", &order_request(false, &["11", "12", "12"]))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.details.get("write_outcome").is_none());
    assert_eq!(server.await.unwrap().len(), 3);
}

#[tokio::test]
async fn soda_manual_track_order_reports_unconfirmed_without_retry_after_bad_ack() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", Some("sessionid_ss=verified; Path=/")),
        created_library_reply(3, "created-before"),
        account_playlist_reply(&["11", "12", "11"], "tracks-before"),
        crate::test_http::json(r#"{"status_code":7}"#, None),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let error = fixture
        .provider
        .reorder_playlist_tracks("42", &order_request(false, &["11", "11", "12"]))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert!(!error.retryable);
    assert_eq!(error.details["write_outcome"], "unconfirmed");
    assert_eq!(error.details["write_requests_dispatched"], 1);
    assert_eq!(server.await.unwrap().len(), 4);
}

#[tokio::test]
async fn soda_manual_track_order_reports_unconfirmed_when_complete_playlist_readback_differs() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", Some("sessionid_ss=verified; Path=/")),
        created_library_reply(3, "created-before"),
        account_playlist_reply(&["11", "12", "11"], "tracks-before"),
        sort_ack("sorted"),
        account_playlist_reply(&["11", "12", "11"], "tracks-after"),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let error = fixture
        .provider
        .reorder_playlist_tracks("42", &order_request(false, &["11", "11", "12"]))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert!(!error.retryable);
    assert_eq!(error.details["write_outcome"], "unconfirmed");
    assert_eq!(error.details["write_requests_dispatched"], 1);
    assert_eq!(server.await.unwrap().len(), 5);
}

#[tokio::test]
async fn soda_manual_track_order_requires_created_library_id_set_to_remain_unchanged() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let changed_created =
        created_library_reply(3, "created-after").replace("\"id\":\"42\"", "\"id\":\"43\"");
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", Some("sessionid_ss=verified; Path=/")),
        created_library_reply(3, "created-before"),
        account_playlist_reply(&["11", "12", "11"], "tracks-before"),
        sort_ack("sorted"),
        account_playlist_reply(&["11", "11", "12"], "tracks-after"),
        changed_created,
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let error = fixture
        .provider
        .reorder_playlist_tracks("42", &order_request(false, &["11", "11", "12"]))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert!(!error.retryable);
    assert_eq!(error.details["write_outcome"], "unconfirmed");
    assert_eq!(server.await.unwrap().len(), 6);
}

#[tokio::test]
async fn soda_manual_track_order_does_not_write_when_current_complete_order_already_matches() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", Some("sessionid_ss=verified; Path=/")),
        created_library_reply(3, "created-before"),
        account_playlist_reply(&["11", "11", "12"], "tracks-before"),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let request = order_request(false, &["11", "11", "12"]);
    let result = fixture
        .provider
        .reorder_playlist_tracks("42", &request)
        .await
        .unwrap();
    assert_eq!(result.track_refs, request.track_refs);
    assert_eq!(result.extensions["no_op"], true);
    assert_eq!(result.extensions["write_requests_dispatched"], 0);
    assert_eq!(server.await.unwrap().len(), 3);
}

#[tokio::test]
async fn soda_manual_track_order_rejects_late_ack_after_selected_account_generation_changes() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let paused = crate::test_http::serve_paused_at(
        vec![
            account_reply("123456", Some("sessionid_ss=verified; Path=/")),
            created_library_reply(3, "created-before"),
            account_playlist_reply(&["11", "12", "11"], "tracks-before"),
            sort_ack("sorted"),
        ],
        3,
    )
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(paused.origin.clone());
    let provider = fixture.provider.clone();
    let request = order_request(false, &["11", "11", "12"]);
    let task = tokio::spawn(async move { provider.reorder_playlist_tracks("42", &request).await });
    paused.arrived.await.unwrap();
    fixture.put(
        "personal",
        &SodaCredential::test_credential("replacement-session")
            .bind_user("123456")
            .unwrap(),
    );
    paused.release.send(()).unwrap();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(error.details["write_outcome"], "unconfirmed");
    assert!(!error.retryable);
    assert_eq!(paused.requests.await.unwrap().len(), 4);
    assert!(
        fixture
            .stored("personal")
            .unwrap()
            .secret()
            .contains("replacement-session")
    );
}
