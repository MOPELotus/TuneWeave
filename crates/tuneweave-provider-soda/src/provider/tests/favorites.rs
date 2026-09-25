use super::*;

mod lifecycle;

const PLAYLIST: &str = "7200303561195061287";

fn entry(id: &str, kind: i64, title: &str) -> serde_json::Value {
    json!({"id":id,"type":kind,"title":title,"owner":{"id":"123456"}})
}

fn directory(entries: Vec<serde_json::Value>, next: Option<&str>, total: usize) -> String {
    json!({"playlists":entries,"has_more":next.is_some(),"next_cursor":next,"total_num":total})
        .to_string()
}

fn detail(ids: &[&str], total: usize, next: Option<&str>) -> serde_json::Value {
    let mut value = crate::client::test_account_playlist_fixture();
    let template = value["media_resources"][0].clone();
    value["playlist"]["type"] = json!(1);
    value["playlist"]["owner"]["id"] = json!("123456");
    value["playlist"]["title"] = json!("重命名的喜欢列表");
    value["playlist"]["count_tracks"] = json!(total);
    value["playlist"]["resource_cnt"]["track_cnt"] = json!(300);
    value["has_more"] = json!(next.is_some());
    value["next_cursor"] = json!(next.unwrap_or("300"));
    value["media_resources"] = ids
        .iter()
        .map(|id| {
            let mut item = template.clone();
            item["id"] = json!(id);
            item["entity"]["track_wrapper"]["track"]["id"] = json!(id);
            item
        })
        .collect();
    value
}

fn json_reply(value: &serde_json::Value, cookie: Option<&str>) -> String {
    crate::test_http::json(&value.to_string(), cookie)
}

fn chosen_directory() -> String {
    directory(
        vec![
            entry("99", 2, "我喜欢"),
            entry("88", 4, "抖音喜欢"),
            entry(PLAYLIST, 1, "随意改名"),
        ],
        None,
        3,
    )
}

fn track_state(collected: Option<bool>) -> serde_json::Value {
    let mut value: serde_json::Value =
        serde_json::from_slice(&crate::client::test_account_track_fixture(false)).unwrap();
    value["track"]["id"] = json!("11");
    value["track"]["state"]["is_collected"] = json!(collected);
    value
}

fn selected(fixture: &SessionFixture, source: &SodaCredential, caller: bool) -> SodaProvider {
    if caller {
        fixture
            .provider
            .caller_credential_scope(&caller_from(source))
            .unwrap()
    } else {
        fixture.provider.clone()
    }
}

fn alias(caller: bool) -> Option<&'static str> {
    if caller { None } else { Some("personal") }
}

#[tokio::test]
async fn favorite_reads_resolve_actual_typed_identity_across_full_directory_and_detail_pages() {
    for caller in [false, true] {
        for method in [
            "metadata",
            "tracks",
            "user_metadata",
            "user_tracks",
            "source",
            "source_items",
        ] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("personal", &source);
            let (origin, server) = crate::test_http::serve(vec![
                account_reply("123456", Some("sessionid_ss=verified")),
                crate::test_http::json(
                    &directory(
                        vec![entry("99", 2, "我喜欢"), entry("88", 4, "我喜欢")],
                        Some("next"),
                        3,
                    ),
                    Some("sessionid_ss=directory-one"),
                ),
                crate::test_http::json(
                    &directory(vec![entry(PLAYLIST, 1, "任意名称")], None, 3),
                    Some("sessionid_ss=directory-two"),
                ),
                json_reply(
                    &detail(&["11"], 3, Some("100")),
                    Some("sessionid_ss=detail-one"),
                ),
                json_reply(
                    &detail(&["22", "22"], 3, None),
                    Some("sessionid_ss=complete"),
                ),
            ])
            .await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(origin);
            let provider = selected(&fixture, &source, caller);
            let request = PageRequest {
                account: alias(caller).map(str::to_owned),
                ..PageRequest::new(2, 1)
            };
            let value = match method {
                "metadata" => {
                    serde_json::to_value(provider.favorite_playlist(alias(caller)).await.unwrap())
                        .unwrap()
                }
                "user_metadata" => serde_json::to_value(
                    provider
                        .user_favorite_playlist("123456", alias(caller))
                        .await
                        .unwrap(),
                )
                .unwrap(),
                "source" => serde_json::to_value(
                    provider
                        .playlist_source("123456", "favorite_tracks", alias(caller))
                        .await
                        .unwrap(),
                )
                .unwrap(),
                "tracks" => {
                    serde_json::to_value(provider.favorite_tracks(&request).await.unwrap()).unwrap()
                }
                "user_tracks" => serde_json::to_value(
                    provider
                        .user_favorite_tracks("123456", &request)
                        .await
                        .unwrap(),
                )
                .unwrap(),
                "source_items" => serde_json::to_value(
                    provider
                        .playlist_source_items("123456", "favorite_tracks", &request)
                        .await
                        .unwrap(),
                )
                .unwrap(),
                _ => unreachable!(),
            };
            if matches!(method, "metadata" | "user_metadata" | "source") {
                assert_eq!(value["id"], PLAYLIST);
                assert_eq!(value["name"], "重命名的喜欢列表");
                assert_eq!(value["track_count"], 3);
                assert_eq!(value["extensions"]["source_type"], "favorite_tracks");
            } else {
                assert_eq!(value["items"].as_array().unwrap().len(), 2);
                assert_eq!(value["pagination"]["total"], 3);
                assert_eq!(value["pagination"]["has_more"], false);
                assert_eq!(value["pagination"]["extensions"]["favorite_kind"], "soda");
            }
            if caller {
                assert!(
                    provider
                        .take_response_credential()
                        .unwrap()
                        .unwrap()
                        .secret()
                        .contains("complete")
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
                        .contains("complete")
                );
            }
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 5);
            assert!(requests[3].contains(&format!("playlist_id={PLAYLIST}")));
            assert!(requests[3].contains("sessionid_ss=directory-two"));
            assert!(requests[4].contains("sessionid_ss=detail-one"));
        }
    }
}

#[tokio::test]
async fn favorites_reject_ambiguous_missing_foreign_or_changed_identity_before_accepting_detail_cookies()
 {
    for variant in ["missing", "duplicate", "type", "owner"] {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        let items = match variant {
            "missing" => vec![entry("99", 2, "我喜欢")],
            "duplicate" => vec![entry(PLAYLIST, 1, "one"), entry("88", 1, "two")],
            _ => vec![entry(PLAYLIST, 1, "actual")],
        };
        let mut responses = vec![
            account_reply("123456", None),
            crate::test_http::json(
                &directory(items.clone(), None, items.len()),
                Some("sessionid_ss=directory"),
            ),
        ];
        if matches!(variant, "type" | "owner") {
            let mut last = detail(&[], 0, None);
            if variant == "type" {
                last["playlist"]["type"] = json!(4);
            } else {
                last["playlist"]["owner"]["id"] = json!("654321");
            }
            responses.push(json_reply(&last, Some("sessionid_ss=poison")));
        }
        let expected_count = responses.len();
        let (origin, server) = crate::test_http::serve(responses).await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);
        let provider = selected(&fixture, &source, true);
        let error = provider.favorite_playlist(None).await.unwrap_err();
        assert_eq!(
            error.code,
            if variant == "missing" {
                ErrorCode::ResourceNotFound
            } else {
                ErrorCode::UpstreamError
            }
        );
        let current = provider
            .selected_credential("default")
            .unwrap()
            .unwrap()
            .0
            .serialize()
            .unwrap();
        assert!(current.contains("directory"));
        assert!(!current.contains("poison"));
        assert_eq!(server.await.unwrap().len(), expected_count);
    }
    let fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    let provider = selected(&fixture, &source, true);
    assert_eq!(
        provider
            .user_favorite_playlist("654321", None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        provider
            .user_favorite_tracks("01", &PageRequest::new(20, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        fixture
            .provider
            .favorite_playlist(Some("missing"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
}

#[tokio::test]
async fn favorite_writes_read_back_every_page_and_update_only_the_selected_source() {
    for caller in [false, true] {
        for subscribed in [false, true] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("personal", &source);
            let last = if subscribed {
                vec!["11", "22"]
            } else {
                vec!["22", "22"]
            };
            let mut replies = vec![
                account_reply("123456", None),
                crate::test_http::json(&chosen_directory(), Some("sessionid_ss=directory")),
                crate::test_http::json(r#"{"status_code":0}"#, Some("sessionid_ss=written")),
                json_reply(
                    &detail(&["33"], 3, Some("100")),
                    Some("sessionid_ss=page-one"),
                ),
                json_reply(&detail(&last, 3, None), Some("sessionid_ss=complete")),
            ];
            if !subscribed {
                replies.push(json_reply(
                    &track_state(Some(false)),
                    Some("sessionid_ss=complete-state"),
                ));
            }
            let (origin, server) = crate::test_http::serve(replies).await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(origin);
            let provider = selected(&fixture, &source, caller);
            let result = provider
                .set_track_subscription("11", subscribed, alias(caller))
                .await
                .unwrap();
            assert_eq!(result.subscribed, subscribed);
            assert_eq!(result.resource_ref.to_string(), "soda:11");
            assert_eq!(
                result.extensions["favorite_playlist_ref"],
                format!("soda:{PLAYLIST}")
            );
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), if subscribed { 5 } else { 6 });
            assert!(requests[2].starts_with(if subscribed {
                "POST /luna/pc/me/collection/media?"
            } else {
                "POST /luna/pc/me/collection/media/delete?"
            }));
            let body: serde_json::Value =
                serde_json::from_str(requests[2].split("\r\n\r\n").nth(1).unwrap()).unwrap();
            assert_eq!(
                body,
                json!({"media":[{"type":"track","id":"11"}],"scene":""})
            );
            assert!(requests[3].contains("sessionid_ss=written"));
            assert!(requests[4].contains("sessionid_ss=page-one"));
            if !subscribed {
                assert!(requests[5].starts_with("POST /luna/pc/track_v2?"));
                assert!(requests[5].contains("sessionid_ss=complete"));
            }
            if caller {
                assert!(
                    provider
                        .take_response_credential()
                        .unwrap()
                        .unwrap()
                        .secret()
                        .contains("complete")
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
                        .contains("complete")
                );
            }
        }
    }
}

#[tokio::test]
async fn favorite_write_failure_never_retries_or_reports_unverified_success() {
    for variant in ["write", "readback", "mismatch", "late_page"] {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        let mut responses = vec![
            account_reply("123456", None),
            crate::test_http::json(&chosen_directory(), Some("sessionid_ss=directory")),
        ];
        if variant == "write" {
            responses.push(crate::test_http::json(
                r#"{"status_code":7}"#,
                Some("sessionid_ss=poison"),
            ));
        } else {
            responses.push(crate::test_http::json(
                r#"{"status_code":0}"#,
                Some("sessionid_ss=written"),
            ));
            match variant {
                "readback" => responses.push(crate::test_http::json(
                    r#"{"status_code":1000016}"#,
                    Some("sessionid_ss=poison"),
                )),
                "mismatch" => responses.push(json_reply(
                    &detail(&[], 0, None),
                    Some("sessionid_ss=readback"),
                )),
                "late_page" => {
                    responses.push(json_reply(
                        &detail(&["11"], 2, Some("100")),
                        Some("sessionid_ss=page-one"),
                    ));
                    responses.push(json_reply(
                        &detail(&[], 2, None),
                        Some("sessionid_ss=poison"),
                    ));
                }
                _ => unreachable!(),
            }
        }
        let count = responses.len();
        let (origin, server) = crate::test_http::serve(responses).await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);
        let provider = selected(&fixture, &source, true);
        let error = provider
            .set_track_subscription("11", true, None)
            .await
            .unwrap_err();
        assert_eq!(error.details["write_outcome"], "unconfirmed");
        assert!(!error.retryable);
        let current = provider
            .selected_credential("default")
            .unwrap()
            .unwrap()
            .0
            .serialize()
            .unwrap();
        assert!(!current.contains("poison"));
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), count);
        assert_eq!(
            requests.iter().filter(|r| r.starts_with("POST ")).count(),
            1
        );
    }
}

#[tokio::test]
async fn favorite_reads_and_writes_cannot_cross_login_generations_after_directory_resolution() {
    for caller in [false, true] {
        for operation in ["read", "like", "unlike"] {
            let writing = operation != "read";
            let subscribed = operation == "like";
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("personal", &source);
            let mut replies = vec![
                account_reply("123456", None),
                crate::test_http::json(&chosen_directory(), None),
            ];
            if writing {
                replies.push(crate::test_http::json(r#"{"status_code":0}"#, None));
            }
            if operation == "unlike" {
                replies.push(json_reply(&detail(&[], 0, None), None));
                replies.push(json_reply(&track_state(Some(false)), None));
            } else {
                replies.push(json_reply(&detail(&["11"], 1, None), None));
            }
            let last = replies.len() - 1;
            let paused = crate::test_http::serve_paused_at(replies, last).await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(paused.origin);
            let provider = selected(&fixture, &source, caller);
            let pending_provider = provider.clone();
            let pending = tokio::spawn(async move {
                if writing {
                    pending_provider
                        .set_track_subscription("11", subscribed, alias(caller))
                        .await
                        .map(|_| ())
                } else {
                    pending_provider
                        .favorite_playlist(alias(caller))
                        .await
                        .map(|_| ())
                }
            });
            paused.arrived.await.unwrap();
            let replacement = SodaCredential::test_credential("new-login")
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
                    .selected_credential(alias(caller).unwrap_or("default"))
                    .unwrap()
                    .unwrap()
                    .0,
                replacement
            );
            assert_eq!(paused.requests.await.unwrap().len(), last + 1);
        }
    }
}

#[tokio::test]
async fn unlike_requires_explicit_false_state_even_when_target_is_absent_from_visible_tracks() {
    for collected in [None, Some(true)] {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        let (origin, server) = crate::test_http::serve(vec![
            account_reply("123456", None),
            crate::test_http::json(&chosen_directory(), None),
            crate::test_http::json(r#"{"status_code":0}"#, Some("sessionid_ss=written")),
            json_reply(&detail(&[], 0, None), Some("sessionid_ss=playlist")),
            json_reply(&track_state(collected), Some("sessionid_ss=track-state")),
        ])
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);
        let provider = selected(&fixture, &source, true);
        let error = provider
            .set_track_subscription("11", false, None)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert_eq!(error.details["write_outcome"], "unconfirmed");
        assert!(
            provider
                .take_response_credential()
                .unwrap()
                .unwrap()
                .secret()
                .contains("track-state")
        );
        assert_eq!(server.await.unwrap().len(), 5);
    }
}
