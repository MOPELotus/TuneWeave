use super::*;

fn album(id: &str, name: &str) -> serde_json::Value {
    json!({
        "id": id,
        "name": name,
        "artists": [{"id":"900","name":"Artist {id}","simple_display_name":""}],
        "count_tracks": 4,
        "url_cover": {
            "uri": format!("album/{id}"),
            "urls": ["https://p1-luna.douyinpic.com/img/"],
            "template_prefix": ""
        },
        "release_date": 1700000000,
        "digital_album_info": {"sale_period":2,"count_purchased":1},
        "credential_material": "must-not-escape"
    })
}

fn albums_reply(cookie: Option<&str>) -> String {
    crate::test_http::json(
        &json!({
            "status_code": 0,
            "albums": [
                album("11", "First"),
                album("22", "Second"),
                album("22", "Duplicate with different metadata")
            ]
        })
        .to_string(),
        cookie,
    )
}

fn owner_provider(
    fixture: &SessionFixture,
    owner: &str,
    credential: &SodaCredential,
) -> SodaProvider {
    if owner == "caller" {
        fixture
            .provider
            .caller_credential_scope(&caller_from(credential))
            .unwrap()
    } else {
        fixture.provider.clone()
    }
}

#[tokio::test]
async fn purchased_album_pages_use_the_selected_identity_and_complete_single_response() {
    for owner in ["personal", "caller"] {
        for offset in [0, 1, 2, 7] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("personal", &source);
            let other = SodaCredential::test_credential("other-secret")
                .bind_user("654321")
                .unwrap();
            fixture.put("other", &other);
            let (origin, server) = crate::test_http::serve(vec![
                account_reply("123456", Some("sessionid_ss=verified-session; Path=/")),
                albums_reply(Some("sessionid_ss=final-session; Path=/")),
            ])
            .await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(origin);
            let provider = owner_provider(&fixture, owner, &source);
            let request = PageRequest {
                account: (owner == "personal").then(|| owner.to_owned()),
                ..PageRequest::new(1, offset)
            };

            let page = provider.account_digital_albums(&request).await.unwrap();
            let expected = ["11", "22"].into_iter().nth(offset as usize);
            assert_eq!(page.items.len(), if expected.is_some() { 1 } else { 0 });
            if let Some(expected) = expected {
                let album = &page.items[0];
                assert_eq!(album.id, expected);
                assert_eq!(album.platform, Platform::Soda);
                assert_eq!(album.purchased, Some(true));
                assert_eq!(album.extensions["source_user_id"], "123456");
                assert!(
                    !serde_json::to_string(album)
                        .unwrap()
                        .contains("must-not-escape")
                );
            }
            assert_eq!(page.pagination.total, Some(2));
            assert_eq!(page.pagination.offset, offset);
            assert_eq!(page.pagination.limit, 1);
            assert_eq!(page.pagination.has_more, offset == 0);
            assert_eq!(page.pagination.next_offset, (offset == 0).then_some(1));
            assert_eq!(page.pagination.extensions["complete_snapshot"], true);
            assert_eq!(page.pagination.extensions["upstream_requests"], 1);
            assert_eq!(page.pagination.extensions["source_user_id"], "123456");
            assert!(
                page.pagination.extensions["source_snapshot_id"]
                    .as_str()
                    .unwrap()
                    .starts_with("soda_digital_albums_v1_")
            );

            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 2);
            assert!(requests[0].starts_with("GET /luna/pc/me?"));
            assert!(requests[1].starts_with("GET /luna/pc/me/assets/albums?"));
            assert!(
                requests[1]
                    .to_ascii_lowercase()
                    .contains("cookie: sessionid_ss=verified-session")
            );
            assert!(requests[1].contains("cursor=&count=100"));
            assert_eq!(
                fixture.stored("other").unwrap().secret(),
                other.serialize().unwrap()
            );
            if owner == "caller" {
                assert!(
                    provider
                        .take_response_credential()
                        .unwrap()
                        .unwrap()
                        .secret()
                        .contains("final-session")
                );
                assert_eq!(
                    fixture.stored("personal").unwrap().secret(),
                    source.serialize().unwrap()
                );
            } else {
                assert!(
                    fixture
                        .stored("personal")
                        .unwrap()
                        .secret()
                        .contains("final-session")
                );
                assert!(provider.take_response_credential().unwrap().is_none());
            }
        }
    }
}

#[tokio::test]
async fn purchased_album_bad_page_and_unconfigured_session_fail_before_network() {
    let provider = SodaProvider::from_client(crate::client::SodaClient::test_client());
    for request in [
        PageRequest::new(0, 0),
        PageRequest::new(101, 0),
        PageRequest::new(1, 10_001),
    ] {
        assert_eq!(
            provider
                .account_digital_albums(&request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        provider
            .account_digital_albums(&PageRequest::new(10, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
}

#[tokio::test]
async fn purchased_album_empty_account_snapshot_is_a_confirmed_empty_page() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", Some("sessionid_ss=verified; Path=/")),
        crate::test_http::json(
            r#"{"status_code":0,"albums":[]}"#,
            Some("sessionid_ss=empty-list-read; Path=/"),
        ),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);

    let page = fixture
        .provider
        .account_digital_albums(&PageRequest {
            account: Some("personal".to_owned()),
            ..PageRequest::new(10, 0)
        })
        .await
        .unwrap();
    assert!(page.items.is_empty());
    assert_eq!(page.pagination.total, Some(0));
    assert!(!page.pagination.has_more);
    assert_eq!(page.pagination.next_offset, None);
    assert_eq!(page.pagination.extensions["complete_snapshot"], true);
    assert!(
        fixture
            .stored("personal")
            .unwrap()
            .secret()
            .contains("empty-list-read")
    );
    assert_eq!(server.await.unwrap().len(), 2);
}

#[tokio::test]
async fn purchased_album_read_rejects_wrong_uid_and_generation_change() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
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
            .account_digital_albums(&PageRequest {
                account: Some("personal".to_owned()),
                ..PageRequest::new(10, 0)
            })
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(server.await.unwrap().len(), 1);

    let paused = crate::test_http::serve_paused_at(
        vec![
            account_reply("123456", Some("sessionid_ss=verified; Path=/")),
            albums_reply(Some("sessionid_ss=unaccepted; Path=/")),
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
            .account_digital_albums(&PageRequest {
                account: Some("personal".to_owned()),
                ..PageRequest::new(10, 0)
            })
            .await
    });
    paused.arrived.await.unwrap();
    fixture.put(
        "personal",
        &SodaCredential::test_credential("new-session")
            .bind_user("123456")
            .unwrap(),
    );
    paused.release.send(()).unwrap();
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    let requests = paused.requests.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        fixture
            .provider
            .take_response_credential()
            .unwrap()
            .is_none()
    );
}
