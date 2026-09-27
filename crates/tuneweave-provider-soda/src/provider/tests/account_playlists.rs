use super::*;

const ID: &str = "7200303561195061287";

fn page(ids: &[&str], total: u64, next: Option<u64>) -> serde_json::Value {
    let mut value = crate::client::test_account_playlist_fixture();
    let template = value["media_resources"][0].clone();
    value["media_resources"] = ids
        .iter()
        .map(|id| {
            let mut item = template.clone();
            item["id"] = json!(id);
            item["entity"]["track_wrapper"]["track"]["id"] = json!(id);
            item
        })
        .collect();
    value["playlist"]["count_tracks"] = json!(total);
    value["playlist"]["resource_cnt"]["track_cnt"] = json!(300);
    value["has_more"] = json!(next.is_some());
    value["next_cursor"] = json!(next.unwrap_or(300).to_string());
    value
}

fn reply(page: &serde_json::Value, cookie: Option<&str>) -> String {
    crate::test_http::json(&page.to_string(), cookie)
}

fn request(caller: bool, limit: u32, offset: u32) -> PageRequest {
    PageRequest {
        account: (!caller).then(|| "personal".to_owned()),
        ..PageRequest::new(limit, offset)
    }
}

#[tokio::test]
async fn account_playlists_prove_full_filtered_snapshot_before_slicing_and_preserve_duplicates() {
    for caller in [false, true] {
        for offset in [0, 1, 4, 9] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("personal", &source);
            fixture.put("other", &source);
            let (origin, server) = crate::test_http::serve(vec![
                account_reply("123456", Some("sessionid_ss=verified")),
                reply(&page(&["11"], 4, Some(100)), Some("sessionid_ss=page-one")),
                reply(&page(&[], 4, Some(200)), Some("sessionid_ss=page-two")),
                reply(
                    &page(&["22", "22", "33"], 4, None),
                    Some("sessionid_ss=page-three"),
                ),
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
                .playlist_tracks(ID, &request(caller, 2, offset))
                .await
                .unwrap();
            let expected: Vec<_> = ["11", "22", "22", "33"]
                .into_iter()
                .skip(offset as usize)
                .take(2)
                .collect();
            assert_eq!(
                result
                    .items
                    .iter()
                    .map(|track| track.id.as_str())
                    .collect::<Vec<_>>(),
                expected
            );
            for (index, track) in result.items.iter().enumerate() {
                assert_eq!(
                    track.extensions["playlist_position"],
                    json!(offset as usize + index)
                );
            }
            assert_eq!(result.pagination.total, Some(4));
            assert_eq!(result.pagination.has_more, offset < 2);
            assert_eq!(result.pagination.extensions["upstream_pages_fetched"], 3);
            assert_eq!(
                result.pagination.extensions["upstream_raw_resource_count"],
                300
            );
            assert_eq!(result.pagination.extensions["source_user_id"], "123456");
            assert_eq!(result.pagination.extensions["complete_snapshot"], true);
            assert!(
                result.pagination.extensions["source_snapshot_id"]
                    .as_str()
                    .unwrap()
                    .starts_with("soda_playlist_v1_")
            );
            let updated = if caller {
                assert_eq!(
                    fixture.stored("personal").unwrap().secret(),
                    source.serialize().unwrap()
                );
                provider
                    .take_response_credential()
                    .unwrap()
                    .unwrap()
                    .secret()
                    .to_owned()
            } else {
                fixture.stored("personal").unwrap().secret().to_owned()
            };
            assert!(updated.contains("page-three"));
            assert_eq!(
                fixture.stored("other").unwrap().secret(),
                source.serialize().unwrap()
            );
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 4);
            for (index, cookie) in ["verified", "page-one", "page-two"].into_iter().enumerate() {
                let req = &requests[index + 1];
                assert!(req.starts_with("GET /luna/pc/playlist/detail?"));
                assert!(req.contains(&format!("cursor={}", index * 100)));
                assert!(req.contains("count=100"));
                assert!(req.contains("device_platform=windows"));
                assert!(req.contains("device_id="));
                assert!(req.contains(&format!("sessionid_ss={cookie}")));
            }
        }
    }
}

#[tokio::test]
async fn account_playlists_reject_changed_or_incomplete_pages_without_accepting_their_cookies() {
    for caller in [false, true] {
        for mutation in [
            "total",
            "raw",
            "updated",
            "owner",
            "sort",
            "order_end",
            "auth",
            "id",
        ] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("personal", &source);
            let mut last = page(&["22"], 2, None);
            match mutation {
                "total" => last["playlist"]["count_tracks"] = json!(3),
                "raw" => last["playlist"]["resource_cnt"]["track_cnt"] = json!(301),
                "updated" => last["playlist"]["update_time"] = json!(1_785_333_244),
                "owner" => last["playlist"]["owner"]["id"] = json!("222"),
                "sort" => last["playlist"]["current_sort_type"] = json!(4),
                "order_end" => last["media_resources"] = json!([]),
                "auth" => last["status_code"] = json!(1_000_016),
                "id" => last["playlist"]["id"] = json!("222"),
                _ => unreachable!(),
            }
            let (origin, server) = crate::test_http::serve(vec![
                account_reply("123456", Some("sessionid_ss=verified")),
                reply(
                    &page(&["11"], 2, Some(100)),
                    Some("sessionid_ss=accepted-page"),
                ),
                reply(&last, Some("sessionid_ss=poison")),
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
            let error = provider
                .playlist_tracks(ID, &request(caller, 1, 0))
                .await
                .unwrap_err();
            assert_eq!(
                error.code,
                if mutation == "auth" {
                    ErrorCode::AuthenticationRequired
                } else {
                    ErrorCode::UpstreamError
                },
                "{mutation}"
            );
            let current = provider
                .selected_credential(if caller { "default" } else { "personal" })
                .unwrap()
                .unwrap()
                .0
                .serialize()
                .unwrap();
            assert!(current.contains("accepted-page"));
            assert!(!current.contains("poison"));
            assert_eq!(server.await.unwrap().len(), 3);
        }
    }
}

#[tokio::test]
async fn account_playlists_metadata_and_pages_share_revision_but_never_mix_users_or_order() {
    let mut fingerprints = Vec::new();
    for (user, ids) in [
        ("123456", vec!["11", "22", "22"]),
        ("123456", vec!["11", "22", "22"]),
        ("654321", vec!["11", "22", "22"]),
        ("123456", vec!["22", "11", "22"]),
    ] {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user(user).unwrap();
        let (origin, server) = crate::test_http::serve(vec![
            account_reply(user, Some("sessionid_ss=first-read")),
            reply(&page(&ids, 3, None), None),
            account_reply(user, Some("sessionid_ss=second-read")),
            reply(&page(&ids, 3, None), None),
        ])
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);
        let provider = fixture
            .provider
            .caller_credential_scope(&caller_from(&source))
            .unwrap();
        let metadata = provider.playlist(ID, None).await.unwrap();
        let items = provider
            .playlist_tracks(ID, &PageRequest::new(100, 0))
            .await
            .unwrap();
        assert_eq!(metadata.track_count, Some(3));
        assert_eq!(metadata.extensions["owner_id"], "2186250840705864");
        assert_eq!(metadata.extensions["source_user_id"], user);
        assert_eq!(
            metadata.extensions["source_snapshot_id"],
            items.pagination.extensions["source_snapshot_id"]
        );
        fingerprints.push(metadata.extensions["source_snapshot_id"].clone());
        assert_eq!(server.await.unwrap().len(), 4);
    }
    assert_eq!(fingerprints[0], fingerprints[1]);
    assert_ne!(fingerprints[0], fingerprints[2]);
    assert_ne!(fingerprints[0], fingerprints[3]);
}

#[tokio::test]
async fn account_playlists_late_page_cannot_replace_a_new_login_generation() {
    for caller in [false, true] {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        fixture.put("personal", &source);
        let paused = crate::test_http::serve_paused_at(
            vec![
                account_reply("123456", None),
                reply(&page(&["11"], 1, None), None),
            ],
            1,
        )
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(paused.origin);
        let provider = if caller {
            fixture
                .provider
                .caller_credential_scope(&caller_from(&source))
                .unwrap()
        } else {
            fixture.provider.clone()
        };
        let pending_provider = provider.clone();
        let pending = tokio::spawn(async move {
            pending_provider
                .playlist_tracks(ID, &request(caller, 100, 0))
                .await
        });
        paused.arrived.await.unwrap();
        let replacement = SodaCredential::test_credential("new-generation")
            .bind_user("123456")
            .unwrap();
        if caller {
            *provider.caller_credential.as_ref().unwrap().lock().unwrap() = replacement.clone();
        } else {
            fixture.put("personal", &replacement);
        }
        paused.release.send(()).unwrap();
        assert_eq!(
            pending.await.unwrap().unwrap_err().code,
            ErrorCode::Conflict
        );
        assert!(provider.take_response_credential().unwrap().is_none());
        assert_eq!(
            provider
                .selected_credential(if caller { "default" } else { "personal" })
                .unwrap()
                .unwrap()
                .0,
            replacement
        );
        assert_eq!(paused.requests.await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn account_playlists_bound_filtered_pages_and_never_report_truncated_success() {
    let mut fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    let mut responses = vec![account_reply("123456", None)];
    for index in 0..MAX_UPSTREAM_PLAYLIST_PAGES {
        responses.push(reply(&page(&[], 1, Some(u64::from(index + 1) * 100)), None));
    }
    let (origin, server) = crate::test_http::serve(responses).await;
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(origin);
    let provider = fixture
        .provider
        .caller_credential_scope(&caller_from(&source))
        .unwrap();
    let error = provider.playlist(ID, None).await.unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert!(error.message.contains("bounded upstream page count"));
    assert_eq!(
        server.await.unwrap().len(),
        MAX_UPSTREAM_PLAYLIST_PAGES as usize + 1
    );
}

#[tokio::test]
async fn account_playlist_source_is_required_and_account_album_cannot_fall_back_anonymously() {
    let fixture = SessionFixture::new();
    assert_eq!(
        fixture
            .provider
            .playlist(ID, Some("missing"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    let mut provider = fixture
        .provider
        .caller_credential_scope(&caller_from(
            &test_soda_credential().bind_user("123456").unwrap(),
        ))
        .unwrap();
    assert!(provider.playlist(ID, Some("foreign")).await.is_err());
    let (origin, server) = crate::test_http::serve(vec![
        crate::test_http::json(r#"{"status_code":1000016}"#, None),
        crate::test_http::json(r#"{"status_code":1000016}"#, None),
    ])
    .await;
    provider.client = provider.client.clone().with_auth_test_origin(origin);
    assert_eq!(
        provider
            .album("7528799183039825936", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert_eq!(
        provider
            .album_tracks("7528799183039825936", &PageRequest::new(10, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|r| r.starts_with("GET /luna/pc/me?aid=386088&app_name=luna_pc"))
    );
}

#[tokio::test]
async fn account_playlist_late_identity_and_page_errors_cannot_cross_login_generation() {
    for caller in [false, true] {
        for boundary in 0..3 {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("personal", &source);
            let mut replies = vec![
                account_reply("123456", Some("sessionid_ss=verified")),
                crate::test_http::json(
                    &page(&["100"], 2, Some(100)).to_string(),
                    Some("sessionid_ss=page-one"),
                ),
                crate::test_http::json(&page(&["101"], 2, None).to_string(), None),
            ];
            replies.truncate(boundary + 1);
            replies[boundary] =
                "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .into();
            let paused = crate::test_http::serve_paused_at(replies, boundary).await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(paused.origin.clone());
            let provider = if caller {
                fixture
                    .provider
                    .caller_credential_scope(&caller_from(&source))
                    .unwrap()
            } else {
                fixture.provider.clone()
            };
            let worker = provider.clone();
            let task =
                tokio::spawn(
                    async move { worker.playlist_tracks(ID, &request(caller, 1, 0)).await },
                );
            paused.arrived.await.unwrap();
            let alias = if caller { "default" } else { "personal" };
            let current = provider.selected_credential(alias).unwrap().unwrap().0;
            let replacement = SodaCredential::test_credential(
                current
                    .cookie_header()
                    .unwrap()
                    .strip_prefix("sessionid_ss=")
                    .unwrap(),
            )
            .bind_user("123456")
            .unwrap();
            assert!(!replacement.same_login(&current));
            if caller {
                *provider.caller_credential.as_ref().unwrap().lock().unwrap() = replacement.clone();
            } else {
                fixture.put(alias, &replacement);
            }
            paused.release.send(()).unwrap();
            assert_eq!(
                task.await.unwrap().unwrap_err().code,
                ErrorCode::Conflict,
                "caller={caller}, boundary={boundary}"
            );
            assert!(provider.take_response_credential().unwrap().is_none());
            assert_eq!(
                provider.selected_credential(alias).unwrap().unwrap().0,
                replacement
            );
            assert_eq!(paused.requests.await.unwrap().len(), boundary + 1);
        }
    }
}

#[tokio::test]
async fn account_playlist_cancel_and_total_timeout_clear_only_pending_delivery() {
    for owner in ["default", "personal", "caller"] {
        for boundary in 0..3 {
            for timeout in [false, true] {
                let mut fixture = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                for account in ["default", "personal", "other"] {
                    fixture.put(account, &source);
                }
                let mut replies = vec![
                    account_reply("123456", Some("sessionid_ss=verified")),
                    reply(&page(&["100"], 2, Some(100)), Some("sessionid_ss=page-one")),
                    reply(&page(&["101"], 2, None), None),
                ];
                replies.truncate(boundary + 1);
                let paused = crate::test_http::serve_paused_at(replies, boundary).await;
                fixture.provider.client = fixture
                    .provider
                    .client
                    .clone()
                    .with_auth_test_origin(paused.origin.clone());
                let provider = if owner == "caller" {
                    fixture
                        .provider
                        .caller_credential_scope(&caller_from(&source))
                        .unwrap()
                } else {
                    fixture.provider.clone()
                };
                let worker = provider.clone();
                let task = tokio::spawn(async move {
                    worker
                        .read_account_playlist_with_budget(
                            ID,
                            (owner != "caller").then_some(owner),
                            std::time::Duration::from_secs(5),
                        )
                        .await
                        .map(|s| s.is_some())
                });
                paused.arrived.await.unwrap();
                let alias = if owner == "caller" { "default" } else { owner };
                let accepted = provider.selected_credential(alias).unwrap().unwrap().0;
                assert!(accepted.same_login(&source));
                if timeout {
                    tokio::time::pause();
                    tokio::time::advance(std::time::Duration::from_secs(6)).await;
                    let result = task.await.unwrap();
                    tokio::time::resume();
                    let error = result.unwrap_err();
                    assert_eq!(error.code, ErrorCode::UpstreamTimeout, "{owner}/{boundary}");
                    assert!(error.retryable);
                } else {
                    task.abort();
                    assert!(task.await.unwrap_err().is_cancelled());
                }
                paused.requests.abort();
                assert!(paused.requests.await.unwrap_err().is_cancelled());
                assert!(provider.take_response_credential().unwrap().is_none());
                assert_eq!(
                    provider.selected_credential(alias).unwrap().unwrap().0,
                    accepted
                );
                for account in ["default", "personal", "other"]
                    .into_iter()
                    .filter(|a| *a != owner)
                {
                    assert_eq!(
                        fixture.stored(account).unwrap().secret(),
                        source.serialize().unwrap()
                    );
                }
            }
        }
    }
}
