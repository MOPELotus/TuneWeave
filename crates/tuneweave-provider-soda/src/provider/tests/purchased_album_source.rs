use super::*;

fn album_catalog(ids: &[(&str, &str)]) -> serde_json::Value {
    json!({
        "status_code": 0,
        "albums": ids.iter().map(|(id, name)| json!({
            "id": id,
            "name": name,
            "count_tracks": 3,
            "artists": [{"id":"456","name":"Artist","simple_display_name":""}],
            "url_cover": {
                "uri": format!("album/{id}"),
                "urls": ["https://p1-luna.douyinpic.com/img/"],
                "template_prefix": ""
            },
            "release_date": 1700000000
        })).collect::<Vec<_>>()
    })
}

fn account_album(id: &str) -> serde_json::Value {
    let mut value = crate::client::test_account_album_fixture();
    value["album_info"]["id"] = json!(id);
    for track in value["tracks"].as_array_mut().unwrap() {
        track["album"]["id"] = json!(id);
    }
    value
}

fn json_reply(value: &serde_json::Value, cookie: &str) -> String {
    crate::test_http::json(&value.to_string(), Some(cookie))
}

fn account_album_replies(ids: &[(&str, &str)], after: &[(&str, &str)]) -> Vec<String> {
    let mut replies = vec![account_reply("123456", Some("sessionid_ss=verified"))];
    replies.push(json_reply(
        &album_catalog(ids),
        "sessionid_ss=catalog-before",
    ));
    let unique = ids
        .iter()
        .map(|(id, _)| *id)
        .collect::<std::collections::BTreeSet<_>>();
    for id in unique {
        replies.push(json_reply(&account_album(id), "sessionid_ss=album-detail"));
    }
    replies.push(json_reply(
        &album_catalog(after),
        "sessionid_ss=catalog-after",
    ));
    replies
}

#[tokio::test]
async fn purchased_album_source_preserves_catalog_order_tracks_and_selected_credentials() {
    for owner in ["default", "personal", "caller"] {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        let other = test_soda_credential().bind_user("654321").unwrap();
        fixture.put("default", &source);
        fixture.put("personal", &source);
        fixture.put("other", &other);

        // The official UI deduplicates digital albums by ID, keeping the first row.
        let catalog = [("900", "First"), ("900", "Duplicate"), ("901", "Second")];
        let mut replies = account_album_replies(&catalog, &catalog);
        replies.extend(account_album_replies(&catalog, &catalog));
        let (origin, server) = crate::test_http::serve(replies).await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);
        let provider = if owner == "caller" {
            fixture
                .provider
                .caller_credential_scope(&caller_from(&source))
                .unwrap()
        } else {
            fixture.provider.clone()
        };
        let alias = (owner != "caller").then_some(owner);

        let metadata = provider
            .playlist_source("123456", "purchased_albums", alias)
            .await
            .unwrap();
        assert_eq!(metadata.name, "已购数字专辑");
        assert_eq!(metadata.track_count, Some(6));
        assert_eq!(metadata.extensions["source_album_count"], 2);
        assert_eq!(metadata.extensions["playback_entitlement_verified"], false);

        let page = provider
            .playlist_source_items(
                "123456",
                "purchased_albums",
                &PageRequest {
                    account: alias.map(str::to_owned),
                    ..PageRequest::new(3, 1)
                },
            )
            .await
            .unwrap();
        assert_eq!(page.pagination.total, Some(6));
        assert_eq!(page.pagination.next_offset, Some(4));
        assert_eq!(
            metadata.extensions["source_snapshot_id"],
            page.pagination.extensions["source_snapshot_id"]
        );
        let tracks = page
            .items
            .iter()
            .map(|item| match item {
                PlaylistPlayableItem::Track(track) => (track.id.as_str(), track.playable),
                _ => panic!("expected a track"),
            })
            .collect::<Vec<_>>();
        assert_eq!(tracks, [("22", None), ("22", None), ("11", None)]);
        let output = serde_json::to_string(&page).unwrap();
        for secret in ["sessionid_ss", "private-player-material", "private-key"] {
            assert!(!output.contains(secret));
        }
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
                    .contains("catalog-after")
            );
            assert_eq!(
                fixture.stored("default").unwrap().secret(),
                source.serialize().unwrap()
            );
        } else {
            assert!(provider.take_response_credential().unwrap().is_none());
            assert!(
                fixture
                    .stored(owner)
                    .unwrap()
                    .secret()
                    .contains("catalog-after")
            );
        }
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 10);
        assert!(requests[2].starts_with("GET /luna/pc/albums/900?"));
        assert!(requests[3].starts_with("GET /luna/pc/albums/901?"));
        assert!(
            requests[3]
                .to_ascii_lowercase()
                .contains("cookie: sessionid_ss=album-detail")
        );
    }
}

#[tokio::test]
async fn purchased_album_source_rejects_changed_directory_and_account_generation() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let changed_replies = vec![
        account_reply("123456", Some("sessionid_ss=verified")),
        json_reply(
            &album_catalog(&[("900", "First")]),
            "sessionid_ss=catalog-before",
        ),
        json_reply(&account_album("900"), "sessionid_ss=album-detail"),
        json_reply(
            &album_catalog(&[("901", "Changed")]),
            "sessionid_ss=catalog-after",
        ),
    ];
    let (origin, server) = crate::test_http::serve(changed_replies).await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let provider = fixture.provider.clone();
    let error = provider
        .playlist_source("123456", "purchased_albums", Some("personal"))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(provider.take_response_credential().unwrap().is_none());
    assert_eq!(server.await.unwrap().len(), 4);

    let mut fixture = SessionFixture::new();
    fixture.put("personal", &source);
    let replies = vec![
        account_reply("123456", Some("sessionid_ss=verified")),
        json_reply(
            &album_catalog(&[("900", "First")]),
            "sessionid_ss=catalog-before",
        ),
        json_reply(&account_album("900"), "sessionid_ss=album-detail"),
    ];
    let paused = crate::test_http::serve_paused_at(replies, 2).await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(paused.origin);
    let provider = fixture.provider.clone();
    let task = tokio::spawn(async move {
        provider
            .playlist_source("123456", "purchased_albums", Some("personal"))
            .await
    });
    paused.arrived.await.unwrap();
    fixture.put(
        "personal",
        &test_soda_credential().bind_user("123456").unwrap(),
    );
    paused.release.send(()).unwrap();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(paused.requests.await.unwrap().len(), 3);
}

#[tokio::test]
async fn purchased_album_source_handles_empty_catalog_and_rejects_anonymous_or_bad_pages() {
    let mut fixture = SessionFixture::new();
    assert_eq!(
        fixture
            .provider
            .playlist_source("123456", "purchased_albums", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );

    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", Some("sessionid_ss=verified")),
        json_reply(&album_catalog(&[]), "sessionid_ss=catalog-before"),
        json_reply(&album_catalog(&[]), "sessionid_ss=catalog-after"),
    ])
    .await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let metadata = fixture
        .provider
        .playlist_source("123456", "purchased_albums", Some("personal"))
        .await
        .unwrap();
    assert_eq!(metadata.track_count, Some(0));
    assert_eq!(metadata.cover_url, None);
    assert_eq!(server.await.unwrap().len(), 3);

    for request in [PageRequest::new(0, 0), PageRequest::new(101, 0)] {
        assert_eq!(
            fixture
                .provider
                .playlist_source_items("123456", "purchased_albums", &request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
}
