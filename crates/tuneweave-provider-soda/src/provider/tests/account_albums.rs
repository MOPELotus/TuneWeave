use super::*;

fn alias(owner: &str) -> Option<&str> {
    if owner == "caller" { None } else { Some(owner) }
}
fn reply(body: &serde_json::Value, cookie: Option<&str>) -> String {
    crate::test_http::json(&body.to_string(), cookie)
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
async fn account_album_three_owners_share_complete_details_tracks_and_uni_sources() {
    for owner in ["default", "personal", "caller"] {
        for operation in ["album", "tracks", "uni_metadata", "uni_items"] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("other", &source);
            fixture.put("default", &source);
            fixture.put("personal", &source);
            let (origin, server) = crate::test_http::serve(vec![
                account_reply("123456", Some("sessionid_ss=verified; Path=/")),
                reply(
                    &crate::client::test_account_album_fixture(),
                    Some("sessionid_ss=final; Path=/"),
                ),
            ])
            .await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(origin);
            let provider = owner_provider(&fixture, owner, &source);
            let request = PageRequest {
                account: alias(owner).map(str::to_owned),
                ..PageRequest::new(2, 1)
            };
            let (output, extensions) = match operation {
                "album" => {
                    let a = provider.album("900", alias(owner)).await.unwrap();
                    assert_eq!(a.track_count, Some(3));
                    (json!(a), a.extensions)
                }
                "tracks" => {
                    let a = provider.album_tracks("900", &request).await.unwrap();
                    assert_eq!(
                        a.items.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
                        ["22", "22"]
                    );
                    assert!(!a.pagination.has_more);
                    assert_eq!(a.items[0].extensions["album_position"], 1);
                    (json!(a), a.pagination.extensions)
                }
                "uni_metadata" => {
                    let a = provider
                        .playlist_source("900", "album", alias(owner))
                        .await
                        .unwrap();
                    assert_eq!(a.extensions["source_type"], "album");
                    (json!(a), a.extensions)
                }
                _ => {
                    let a = provider
                        .playlist_source_items("900", "album", &request)
                        .await
                        .unwrap();
                    assert_eq!(a.items.len(), 2);
                    (json!(a), a.pagination.extensions)
                }
            };
            assert_eq!(extensions["source_user_id"], "123456");
            assert_eq!(extensions["complete_read"], true);
            assert!(
                extensions["source_snapshot_id"]
                    .as_str()
                    .unwrap()
                    .starts_with("soda_album_v1_")
            );
            for secret in [
                "session-secret",
                "sessionid_ss",
                "private-key",
                "private-player-material",
            ] {
                assert!(!output.to_string().contains(secret));
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
                assert_eq!(
                    fixture.stored("personal").unwrap().secret(),
                    source.serialize().unwrap()
                );
            } else {
                assert!(fixture.stored(owner).unwrap().secret().contains("final"));
                assert!(provider.take_response_credential().unwrap().is_none());
            }
            let seen = server.await.unwrap();
            assert_eq!(seen.len(), 2);
            assert!(seen[0].contains("sessionid_ss=session-secret"));
            assert!(seen[1].starts_with("GET /luna/pc/albums/900?"));
            assert!(seen[1].contains("sessionid_ss=verified"));
        }
    }
}

#[tokio::test]
async fn account_album_snapshot_is_stable_for_cookie_rotation_but_binds_login_and_order() {
    let mut fixture = SessionFixture::new();
    let original = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("personal", &original);
    let base = crate::client::test_account_album_fixture();
    let mut changed = base.clone();
    changed["tracks"].as_array_mut().unwrap().reverse();
    let mut replies = Vec::new();
    for i in 0..4 {
        replies.push(account_reply(
            "123456",
            Some(&format!("sessionid_ss=verified-{i}; Path=/")),
        ));
        replies.push(reply(
            if i == 3 { &changed } else { &base },
            Some(&format!("sessionid_ss=final-{i}; Path=/")),
        ));
    }
    let (origin, server) = crate::test_http::serve(replies).await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let metadata = fixture
        .provider
        .playlist_source("900", "album", Some("personal"))
        .await
        .unwrap();
    let page = fixture
        .provider
        .playlist_source_items(
            "900",
            "album",
            &PageRequest {
                account: Some("personal".to_owned()),
                ..PageRequest::new(100, 0)
            },
        )
        .await
        .unwrap();
    let first = metadata.extensions["source_snapshot_id"].clone();
    assert_eq!(first, page.pagination.extensions["source_snapshot_id"]);
    let current = fixture
        .provider
        .selected_credential("personal")
        .unwrap()
        .unwrap()
        .0;
    let new_login = SodaCredential::import_cookie_header(&current.cookie_header().unwrap())
        .unwrap()
        .bind_user("123456")
        .unwrap();
    assert!(!current.same_login(&new_login));
    fixture.put("personal", &new_login);
    let relogin = fixture
        .provider
        .album("900", Some("personal"))
        .await
        .unwrap();
    assert_ne!(first, relogin.extensions["source_snapshot_id"]);
    let reordered = fixture
        .provider
        .album("900", Some("personal"))
        .await
        .unwrap();
    assert_ne!(
        relogin.extensions["source_snapshot_id"],
        reordered.extensions["source_snapshot_id"]
    );
    assert_eq!(server.await.unwrap().len(), 8);
}

#[tokio::test]
async fn account_album_late_success_and_failure_observe_source_generation_at_both_boundaries() {
    for owner in ["default", "personal", "caller"] {
        for boundary in 0..2 {
            for change in ["same_cookie_login", "other_user", "logout"] {
                for failure in [false, true] {
                    if owner == "caller" && change == "logout" {
                        continue;
                    }
                    let mut fixture = SessionFixture::new();
                    let source = test_soda_credential().bind_user("123456").unwrap();
                    fixture.put("default", &source);
                    fixture.put("personal", &source);
                    fixture.put("other", &source);
                    let success = if boundary == 0 {
                        account_reply("123456", Some("sessionid_ss=verified; Path=/"))
                    } else {
                        reply(
                            &crate::client::test_account_album_fixture(),
                            Some("sessionid_ss=late; Path=/"),
                        )
                    };
                    let last = if failure {
                        "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".to_owned()
                    } else {
                        success
                    };
                    let replies = if boundary == 0 {
                        vec![last]
                    } else {
                        vec![
                            account_reply("123456", Some("sessionid_ss=verified; Path=/")),
                            last,
                        ]
                    };
                    let paused = crate::test_http::serve_paused_at(replies, boundary).await;
                    fixture.provider.client = fixture
                        .provider
                        .client
                        .clone()
                        .with_auth_test_origin(paused.origin.clone());
                    let provider = owner_provider(&fixture, owner, &source);
                    let worker = provider.clone();
                    let pending =
                        tokio::spawn(async move { worker.album("900", alias(owner)).await });
                    paused.arrived.await.unwrap();
                    let replacement = if change == "same_cookie_login" {
                        SodaCredential::test_credential(if boundary == 0 {
                            "session-secret"
                        } else {
                            "verified"
                        })
                        .bind_user("123456")
                        .unwrap()
                    } else {
                        SodaCredential::test_credential("replacement")
                            .bind_user("654321")
                            .unwrap()
                    };
                    if owner == "caller" {
                        *provider.caller_credential.as_ref().unwrap().lock().unwrap() =
                            replacement.clone();
                    } else if change == "logout" {
                        fixture.store.remove(Platform::Soda, owner).unwrap();
                    } else {
                        fixture.put(owner, &replacement);
                    }
                    paused.release.send(()).unwrap();
                    assert_eq!(
                        pending.await.unwrap().unwrap_err().code,
                        ErrorCode::Conflict,
                        "{owner}/{boundary}/{change}/{failure}"
                    );
                    assert!(provider.take_response_credential().unwrap().is_none());
                    assert_eq!(
                        fixture.stored("other").unwrap().secret(),
                        source.serialize().unwrap()
                    );
                    if change == "logout" {
                        assert!(fixture.stored(owner).is_none());
                    } else {
                        assert_eq!(
                            provider
                                .selected_credential(alias(owner).unwrap_or("default"))
                                .unwrap()
                                .unwrap()
                                .0,
                            replacement
                        );
                    }
                    assert_eq!(paused.requests.await.unwrap().len(), boundary + 1);
                }
            }
        }
    }
}

#[tokio::test]
async fn account_album_failure_or_cancel_cannot_issue_a_partial_caller_update() {
    for cancel in [false, true] {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        fixture.put("other", &source);
        let mut bad = crate::client::test_account_album_fixture();
        bad["tracks"][2]["album"]["id"] = json!("901");
        let paused = crate::test_http::serve_paused_at(
            vec![
                account_reply("123456", Some("sessionid_ss=verified; Path=/")),
                reply(&bad, Some("sessionid_ss=bad; Path=/")),
            ],
            1,
        )
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(paused.origin.clone());
        let provider = owner_provider(&fixture, "caller", &source);
        let worker = provider.clone();
        let task = tokio::spawn(async move { worker.album("900", None).await });
        paused.arrived.await.unwrap();
        if cancel {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            paused.requests.abort();
            assert!(paused.requests.await.unwrap_err().is_cancelled());
        } else {
            paused.release.send(()).unwrap();
            assert_eq!(
                task.await.unwrap().unwrap_err().code,
                ErrorCode::UpstreamError
            );
            assert_eq!(paused.requests.await.unwrap().len(), 2);
        }
        assert!(provider.take_response_credential().unwrap().is_none());
        assert_eq!(
            fixture.stored("other").unwrap().secret(),
            source.serialize().unwrap()
        );
        assert!(
            provider
                .selected_credential("default")
                .unwrap()
                .unwrap()
                .0
                .cookie_header()
                .unwrap()
                .contains("verified")
        );
    }
}

#[tokio::test]
async fn account_album_input_and_missing_sources_fail_before_network_or_store_updates() {
    let mut fixture = SessionFixture::new();
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(url::Url::parse("http://127.0.0.1:1/").unwrap());
    for id in ["0", "0900", "900/x"] {
        assert_eq!(
            fixture
                .provider
                .album(id, Some("default"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        fixture
            .provider
            .album("900", Some("default"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    for (limit, offset) in [(0, 0), (101, 0), (100, u32::MAX)] {
        assert_eq!(
            fixture
                .provider
                .album_tracks(
                    "900",
                    &PageRequest {
                        limit,
                        offset,
                        account: Some("default".to_owned())
                    }
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("default", &source);
    let caller = owner_provider(&fixture, "caller", &source);
    assert_eq!(
        caller
            .album("900", Some("personal"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(
        fixture
            .provider
            .read_account_album("900", None)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        fixture.stored("default").unwrap().secret(),
        source.serialize().unwrap()
    );
}
