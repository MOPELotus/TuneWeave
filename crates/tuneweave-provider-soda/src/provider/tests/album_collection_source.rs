use super::*;

fn directory(ids: &[&str]) -> serde_json::Value {
    json!({"mixed_collections": ids.iter().map(|id| json!({
        "item_type":"album", "album":{"id":id,"name":format!("Album {id}"),
        "count_tracks":3,"state":{"is_collected":true}}
    })).collect::<Vec<_>>(), "has_more":false,"total_num":ids.len()})
}
fn album(id: &str) -> serde_json::Value {
    let mut value = crate::client::test_account_album_fixture();
    value["album_info"]["id"] = json!(id);
    for track in value["tracks"].as_array_mut().unwrap() {
        track["album"]["id"] = json!(id);
    }
    value
}
fn reply(value: &serde_json::Value, cookie: &str) -> String {
    crate::test_http::json(&value.to_string(), Some(cookie))
}
fn read_replies(ids: &[&str]) -> Vec<String> {
    let mut replies = vec![
        account_reply("123456", Some("sessionid_ss=verified")),
        reply(&directory(ids), "sessionid_ss=directory"),
    ];
    for id in ids {
        replies.push(reply(&album(id), "sessionid_ss=album"));
    }
    replies.push(reply(&directory(ids), "sessionid_ss=final"));
    replies
}

#[tokio::test]
async fn collected_album_source_preserves_order_duplicates_paging_and_credential_ownership() {
    for owner in ["default", "personal", "caller"] {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        for alias in ["default", "personal", "other"] {
            fixture.put(alias, &source);
        }
        let mut replies = read_replies(&["900", "901"]);
        replies.extend(read_replies(&["900", "901"]));
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
            .playlist_source("123456", "collected_albums", alias)
            .await
            .unwrap();
        let page = provider
            .playlist_source_items(
                "123456",
                "collected_albums",
                &PageRequest {
                    account: alias.map(str::to_owned),
                    ..PageRequest::new(4, 1)
                },
            )
            .await
            .unwrap();
        assert_eq!(metadata.track_count, Some(6));
        assert_eq!(metadata.extensions["source_album_count"], 2);
        assert_eq!(page.pagination.total, Some(6));
        assert_eq!(page.pagination.next_offset, Some(5));
        assert_eq!(
            metadata.extensions["source_snapshot_id"],
            page.pagination.extensions["source_snapshot_id"]
        );
        let songs: Vec<_> = page
            .items
            .iter()
            .map(|item| match item {
                PlaylistPlayableItem::Track(track) => (
                    track.id.as_str(),
                    track
                        .album
                        .as_ref()
                        .unwrap()
                        .resource_ref
                        .as_ref()
                        .unwrap()
                        .id(),
                ),
                _ => panic!("expected a song"),
            })
            .collect();
        assert_eq!(
            songs,
            [("22", "900"), ("22", "900"), ("11", "901"), ("22", "901")]
        );
        let output = serde_json::to_string(&page).unwrap();
        for secret in ["sessionid_ss", "private-player-material", "private-key"] {
            assert!(!output.contains(secret));
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
                    .contains("final")
            );
            assert_eq!(
                fixture.stored("default").unwrap().secret(),
                source.serialize().unwrap()
            );
        } else {
            assert!(provider.take_response_credential().unwrap().is_none());
            assert!(fixture.stored(owner).unwrap().secret().contains("final"));
        }
        let seen = server.await.unwrap();
        assert_eq!(seen.len(), 10);
        assert!(seen[2].starts_with("GET /luna/pc/albums/900?"));
        assert!(seen[3].starts_with("GET /luna/pc/albums/901?"));
        assert!(seen[3].contains("sessionid_ss=album"));
    }
}

#[tokio::test]
async fn collected_album_source_rejects_changed_incomplete_or_unclassified_directories() {
    for case in [
        "changed",
        "missing_count",
        "unknown_kind",
        "incomplete_album",
    ] {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        let mut first = directory(&["900"]);
        if case == "missing_count" {
            first.as_object_mut().unwrap().remove("total_num");
        }
        if case == "unknown_kind" {
            first["mixed_collections"][0]["item_type"] = json!("future_album");
        }
        let mut replies = vec![
            account_reply("123456", Some("sessionid_ss=verified")),
            reply(&first, "sessionid_ss=directory"),
        ];
        if case == "changed" || case == "incomplete_album" {
            let mut data = album("900");
            if case == "incomplete_album" {
                data["has_more"] = json!(true);
            }
            replies.push(reply(&data, "sessionid_ss=album"));
        }
        if case == "changed" {
            replies.push(reply(&directory(&[]), "sessionid_ss=final"));
        }
        let expected = replies.len();
        let (origin, server) = crate::test_http::serve(replies).await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);
        let provider = fixture
            .provider
            .caller_credential_scope(&caller_from(&source))
            .unwrap();
        let error = provider
            .playlist_source("123456", "collected_albums", None)
            .await
            .unwrap_err();
        assert_eq!(
            error.code,
            if case == "changed" {
                ErrorCode::Conflict
            } else {
                ErrorCode::UpstreamError
            }
        );
        assert!(provider.take_response_credential().unwrap().is_none());
        assert_eq!(server.await.unwrap().len(), expected);
    }
}

#[tokio::test]
async fn collected_album_source_empty_and_input_validation_do_not_fabricate_tracks() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    for id in ["", "0123", "-1"] {
        assert_eq!(
            fixture
                .provider
                .playlist_source(id, "collected_albums", Some("personal"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        fixture
            .provider
            .playlist_source("654321", "collected_albums", Some("personal"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    let mut request = PageRequest::new(0, 0);
    request.account = Some("personal".into());
    assert_eq!(
        fixture
            .provider
            .playlist_source_items("123456", "collected_albums", &request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let (origin, server) = crate::test_http::serve(read_replies(&[])).await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    request.limit = 100;
    let page = fixture
        .provider
        .playlist_source_items("123456", "collected_albums", &request)
        .await
        .unwrap();
    assert!(page.items.is_empty());
    assert_eq!(page.pagination.total, Some(0));
    assert!(!page.pagination.has_more);
    assert_eq!(server.await.unwrap().len(), 3);
}

#[tokio::test]
async fn collected_album_source_discards_late_album_response_after_account_replacement() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &source);
    let mut replies = read_replies(&["900"]);
    replies.truncate(3);
    let paused = crate::test_http::serve_paused_at(replies, 2).await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(paused.origin);
    let provider = fixture.provider.clone();
    let task = tokio::spawn(async move {
        provider
            .playlist_source("123456", "collected_albums", Some("personal"))
            .await
    });
    paused.arrived.await.unwrap();
    let replacement = SodaCredential::test_credential("replacement")
        .bind_user("654321")
        .unwrap();
    fixture.put("personal", &replacement);
    paused.release.send(()).unwrap();
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(
        fixture.stored("personal").unwrap().secret(),
        replacement.serialize().unwrap()
    );
    assert_eq!(paused.requests.await.unwrap().len(), 3);
}
