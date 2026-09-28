use super::*;

fn artist_page(
    artists: &[(&str, &str)],
    next_cursor: &str,
    total_num: u64,
    has_more: bool,
    cookie: Option<&str>,
) -> String {
    crate::test_http::json(
        &json!({
            "status_code": 0,
            "artists": artists
                .iter()
                .map(|(id, name)| json!({"id": id, "name": name, "count_albums": 2, "count_tracks": 7, "user": {"id": "999", "token": "discarded-artist-user-secret"}}))
                .collect::<Vec<_>>(),
            "next_cursor": next_cursor,
            "total_num": total_num,
            "has_more": has_more,
        })
        .to_string(),
        cookie,
    )
}

fn owner_provider(fixture: &SessionFixture, owner: &str, source: &SodaCredential) -> SodaProvider {
    if owner == "caller" {
        fixture
            .provider
            .caller_credential_scope(&caller_from(source))
            .unwrap()
    } else {
        fixture.provider.clone()
    }
}

#[tokio::test]
async fn account_following_artists_uses_selected_session_and_complete_mobile_readback() {
    for owner in ["personal", "caller"] {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        fixture.put("personal", &source);
        fixture.put("other", &source);
        let (origin, server) = crate::test_http::serve(vec![
            account_reply("123456", Some("sessionid_ss=verified-session; Path=/")),
            artist_page(
                &[("11", "Artist Eleven")],
                "cursor-1",
                2,
                true,
                Some("sessionid_ss=page-one-session; Path=/"),
            ),
            artist_page(
                &[("22", "Artist Twenty-Two")],
                "",
                2,
                false,
                Some("sessionid_ss=final-session; Path=/"),
            ),
        ])
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin.clone());
        let provider = owner_provider(&fixture, owner, &source);
        let request = PageRequest {
            limit: 1,
            offset: 1,
            account: (owner != "caller").then_some(owner.to_owned()),
        };

        let page = provider.account_following_artists(&request).await.unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, "22");
        assert_eq!(page.items[0].name, "Artist Twenty-Two");
        assert_eq!(page.items[0].resource_ref.to_string(), "soda:22");
        assert_eq!(page.items[0].album_count, Some(2));
        assert_eq!(page.items[0].track_count, Some(7));
        assert_eq!(page.items[0].extensions["source_user_id"], "123456");
        assert_eq!(page.items[0].extensions["authenticated"], true);
        assert_eq!(page.pagination.total, Some(2));
        assert!(!page.pagination.has_more);
        assert_eq!(page.pagination.extensions["complete_snapshot"], true);
        assert_eq!(page.pagination.extensions["upstream_pages_read"], 2);

        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 3);
        assert!(requests[1].starts_with("GET /luna/me/collection/artist?aid=386088&"));
        assert!(requests[1].contains("cookie: sessionid_ss=verified-session\r\n"));
        assert!(
            requests[1]
                .to_ascii_lowercase()
                .contains("user-agent: lunapc/3.7.0(")
        );
        assert!(requests[1].contains("x-luna-background-type: foreground\r\n"));
        assert!(requests[1].contains("x-luna-is-background-req: 0\r\n"));
        assert!(requests[1].contains("x-luna-is-local-user: 1\r\n"));
        assert!(requests[2].starts_with("GET /luna/me/collection/artist?aid=386088&"));
        assert!(requests[2].contains("cookie: sessionid_ss=page-one-session\r\n"));
        for wire in &requests[1..] {
            let target = wire
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap();
            let url = origin.join(target).unwrap();
            let query = url.query_pairs().collect::<BTreeMap<_, _>>();
            assert_eq!(query.get("count").map(|value| value.as_ref()), Some("100"));
            assert_eq!(
                query.get("app_name").map(|value| value.as_ref()),
                Some("luna_pc")
            );
            assert_eq!(
                query.get("device_platform").map(|value| value.as_ref()),
                Some("windows")
            );
        }
        let first = origin
            .join(
                requests[1]
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap(),
            )
            .unwrap();
        let second = origin
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
        assert_eq!(
            first
                .query_pairs()
                .find(|(key, _)| key == "cursor")
                .unwrap()
                .1,
            ""
        );
        assert_eq!(
            second
                .query_pairs()
                .find(|(key, _)| key == "cursor")
                .unwrap()
                .1,
            "cursor-1"
        );

        let serialized = serde_json::to_string(&page).unwrap();
        for secret in [
            "session-secret",
            "verified-session",
            "page-one-session",
            "final-session",
            "discarded-artist-user-secret",
        ] {
            assert!(!serialized.contains(secret));
        }
        assert_eq!(
            fixture.stored("other").unwrap().secret(),
            source.serialize().unwrap()
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

#[tokio::test]
async fn account_following_artists_rejects_bad_windows_before_credentials_or_io() {
    let provider = SodaProvider::new(SodaConfig::default()).unwrap();
    for request in [
        PageRequest::new(0, 0),
        PageRequest::new(101, 0),
        PageRequest::new(1, 10_001),
    ] {
        assert_eq!(
            provider
                .account_following_artists(&request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        provider
            .account_following_artists(&PageRequest::new(1, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
}

#[tokio::test]
async fn account_following_artists_rejects_unstable_totals_without_partial_success() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", None),
        artist_page(&[("11", "Artist Eleven")], "cursor-1", 2, true, None),
        artist_page(&[("22", "Artist Twenty-Two")], "", 3, false, None),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);

    let error = fixture
        .provider
        .account_following_artists(&PageRequest {
            account: Some("personal".to_owned()),
            ..PageRequest::new(25, 0)
        })
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert_eq!(server.await.unwrap().len(), 3);
}

#[tokio::test]
async fn account_following_artists_cannot_return_a_late_page_after_selected_logout() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    fixture.put("other", &source);
    let paused = crate::test_http::serve_paused_at(
        vec![
            account_reply("123456", Some("sessionid_ss=verified-session")),
            artist_page(
                &[("11", "Artist Eleven")],
                "cursor-1",
                2,
                true,
                Some("sessionid_ss=page-one-session"),
            ),
            artist_page(
                &[("22", "Artist Twenty-Two")],
                "",
                2,
                false,
                Some("sessionid_ss=late-page-session"),
            ),
        ],
        2,
    )
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(paused.origin);
    let provider = fixture.provider.clone();
    let task = tokio::spawn(async move {
        provider
            .account_following_artists(&PageRequest {
                account: Some("personal".to_owned()),
                ..PageRequest::new(25, 0)
            })
            .await
    });

    paused.arrived.await.unwrap();
    assert!(fixture.provider.logout("personal").await.unwrap());
    paused.release.send(()).unwrap();
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(paused.requests.await.unwrap().len(), 3);
    assert!(fixture.stored("personal").is_none());
    assert_eq!(
        fixture.stored("other").unwrap().secret(),
        source.serialize().unwrap()
    );
    assert!(
        fixture
            .provider
            .take_response_credential()
            .unwrap()
            .is_none()
    );
}
