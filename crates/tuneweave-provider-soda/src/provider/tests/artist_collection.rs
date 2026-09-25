use super::*;

fn artist_page(ids: &[&str], next: &str, total: usize, has_more: bool) -> String {
    crate::test_http::json(
        &json!({
            "status_code": 0,
            "artists": ids.iter().map(|id| json!({"id": id, "name": format!("Artist {id}")})).collect::<Vec<_>>(),
            "next_cursor": next,
            "total_num": total,
            "has_more": has_more,
        })
        .to_string(),
        None,
    )
}

#[tokio::test]
async fn artist_subscription_uses_selected_account_and_confirms_complete_readback() {
    for caller in [false, true] {
        for subscribed in [false, true] {
            let mut fixture = SessionFixture::new();
            let source = SodaCredential::test_credential("artist-selected-session")
                .bind_user("123456")
                .unwrap();
            fixture.put("personal", &source);
            fixture.put("other", &source);
            let before = if subscribed { &[][..] } else { &["11"][..] };
            let after = if subscribed { &["11"][..] } else { &[][..] };
            let method_path = if subscribed {
                "/luna/me/collection/artist"
            } else {
                "/luna/me/collection/artist/delete"
            };
            let ack = if subscribed {
                json!({"status_code":0,"artist_ids":["11"],"collect_artist_status":{"11":1}})
            } else {
                json!({"status_code":0,"deleted_artists":["11"],"deleted_artist_status":{"11":0}})
            };
            let (origin, server) = crate::test_http::serve(vec![
                account_reply("123456", Some("sessionid_ss=verified")),
                artist_page(before, "", before.len(), false),
                crate::test_http::json(&ack.to_string(), Some("sessionid_ss=write-rotated")),
                artist_page(after, "", after.len(), false),
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
            let result = provider
                .change_artist_collection_with_budget(
                    "11",
                    subscribed,
                    (!caller).then_some("personal"),
                    std::time::Duration::from_secs(5),
                )
                .await
                .unwrap();
            assert_eq!(result.subscribed, subscribed);
            assert_eq!(result.resource_ref.id(), "11");
            assert_eq!(result.extensions["source_user_id"], "123456");
            assert_eq!(result.extensions["write_performed"], true);

            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 4);
            assert!(requests[2].starts_with(&format!("POST {method_path} ")));
            assert!(requests[2].contains("cookie: sessionid_ss=verified\r\n"));
            assert!(requests[2].contains("x-luna-api-version: 2023-01-04\r\n"));
            assert!(requests[2].contains("x-luna-is-login: 1\r\n"));
            let body = requests[2].split("\r\n\r\n").nth(1).unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(body).unwrap(),
                json!({"artist_ids":["11"]})
            );
            assert!(requests[3].starts_with("GET /luna/me/collection/artist?count=100 "));
            assert!(requests[3].contains("cookie: sessionid_ss=write-rotated\r\n"));
        }
    }
}

#[tokio::test]
async fn artist_subscription_is_idempotent_and_requires_an_explicit_selected_account() {
    let provider = SodaProvider::new(SodaConfig::default()).unwrap();
    assert_eq!(
        provider
            .set_artist_subscription("01", true, None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        provider
            .set_artist_subscription("11", true, None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );

    let mut fixture = SessionFixture::new();
    let source = SodaCredential::test_credential("artist-noop-session")
        .bind_user("123456")
        .unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", None),
        artist_page(&["11"], "", 1, false),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let result = fixture
        .provider
        .change_artist_collection_with_budget(
            "11",
            true,
            Some("personal"),
            std::time::Duration::from_secs(5),
        )
        .await
        .unwrap();
    assert_eq!(result.extensions["write_performed"], false);
    assert_eq!(server.await.unwrap().len(), 2);
}

#[tokio::test]
async fn artist_subscription_rejects_selected_uid_mismatch_before_collection_or_write() {
    let mut fixture = SessionFixture::new();
    let source = SodaCredential::test_credential("artist-wrong-user-session")
        .bind_user("123456")
        .unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![account_reply("654321", None)]).await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    assert_eq!(
        fixture
            .provider
            .set_artist_subscription("11", true, Some("personal"))
            .await
            .unwrap_err()
            .code,
        // The saved credential is bound to 123456, so an account endpoint
        // responding as 654321 is rejected as an upstream identity change.
        ErrorCode::UpstreamError
    );
    assert_eq!(server.await.unwrap().len(), 1);
}

#[tokio::test]
async fn artist_collection_snapshot_requires_advancing_cursors_and_a_stable_complete_total() {
    let mut fixture = SessionFixture::new();
    let source = SodaCredential::test_credential("artist-pages-session")
        .bind_user("123456")
        .unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        crate::test_http::json(
            r#"{"status_code":0,"artists":[{"id":"11","name":"Artist 11"}],"next_cursor":"next","total_num":2,"has_more":true}"#,
            Some("sessionid_ss=page-one"),
        ),
        crate::test_http::json(
            r#"{"status_code":0,"artists":[{"id":"22","name":"Artist 22"}],"next_cursor":"","total_num":2,"has_more":false}"#,
            None,
        ),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let (mut source, mut stored) = fixture
        .provider
        .selected_credential("personal")
        .unwrap()
        .unwrap();
    let snapshot = fixture
        .provider
        .saved_artist_collection_snapshot(Some("personal"), &mut source, &mut stored)
        .await
        .unwrap();
    assert_eq!(snapshot.total, 2);
    assert_eq!(snapshot.pages, 2);
    assert_eq!(
        snapshot.ids,
        BTreeSet::from(["11".to_owned(), "22".to_owned()])
    );
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].starts_with("GET /luna/me/collection/artist?count=100 "));
    assert!(requests[1].starts_with("GET /luna/me/collection/artist?cursor=next&count=100 "));
    assert!(requests[1].contains("cookie: sessionid_ss=page-one\r\n"));

    let mut fixture = SessionFixture::new();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        crate::test_http::json(
            r#"{"status_code":0,"artists":[{"id":"11","name":"Artist 11"}],"next_cursor":"next","total_num":2,"has_more":true}"#,
            None,
        ),
        crate::test_http::json(
            r#"{"status_code":0,"artists":[{"id":"22","name":"Artist 22"}],"next_cursor":"","total_num":3,"has_more":false}"#,
            None,
        ),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let (mut source, mut stored) = fixture
        .provider
        .selected_credential("personal")
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture
            .provider
            .saved_artist_collection_snapshot(Some("personal"), &mut source, &mut stored)
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(server.await.unwrap().len(), 2);
}
