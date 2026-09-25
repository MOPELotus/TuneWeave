use super::*;

mod lifecycle;

fn album(id: &str, count: Option<u64>) -> serde_json::Value {
    json!({"item_type":"album","album":{"id":id,"name":format!("Album {id}"),"count_tracks":count,"artists":[{"id":"31","name":"Artist"}],"state":{"is_collected":true}}})
}
fn playlist(id: &str) -> serde_json::Value {
    json!({"item_type":"playlist","playlist":{"id":id,"title":"unrelated"}})
}
fn mixed(
    items: Vec<serde_json::Value>,
    next: Option<&str>,
    total: Option<usize>,
) -> serde_json::Value {
    json!({"mixed_collections":items,"has_more":next.is_some(),"next_cursor":next,"total_num":total})
}
fn reply(value: serde_json::Value, cookie: Option<&str>) -> String {
    crate::test_http::json(&value.to_string(), cookie)
}
fn alias(caller: bool) -> Option<&'static str> {
    if caller { None } else { Some("personal") }
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

#[tokio::test]
async fn album_collections_read_full_mixed_pages_before_unified_paging_and_keep_source_isolation() {
    for caller in [false, true] {
        for user_route in [false, true] {
            for offset in [0, 1, 2, 10] {
                let mut fixture = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                fixture.put("personal", &source);
                fixture.put("other", &source);
                let (origin, server) = crate::test_http::serve(vec![
                    account_reply("123456", Some("sessionid_ss=verified")),
                    reply(
                        mixed(vec![playlist("11")], Some("one"), Some(5)),
                        Some("sessionid_ss=one"),
                    ),
                    reply(
                        mixed(
                            vec![album("11", None), playlist("44")],
                            Some("two"),
                            Some(5),
                        ),
                        Some("sessionid_ss=two"),
                    ),
                    reply(
                        mixed(
                            vec![album("22", Some(0)), album("33", Some(10))],
                            None,
                            Some(5),
                        ),
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
                    ..PageRequest::new(2, offset)
                };
                let page = if user_route {
                    provider
                        .user_favorite_albums("123456", &request)
                        .await
                        .unwrap()
                } else {
                    provider.account_albums(&request).await.unwrap()
                };
                let expected: Vec<_> = ["11", "22", "33"]
                    .into_iter()
                    .skip(offset as usize)
                    .take(2)
                    .collect();
                assert_eq!(
                    page.items
                        .iter()
                        .map(|album| album.id.as_str())
                        .collect::<Vec<_>>(),
                    expected
                );
                assert_eq!(page.pagination.total, Some(3));
                assert_eq!(page.pagination.has_more, offset == 0);
                assert_eq!(
                    page.pagination.extensions["upstream_raw_collection_count"],
                    5
                );
                assert_eq!(page.pagination.extensions["source_user_id"], "123456");
                if offset == 0 {
                    assert_eq!(page.items[0].track_count, None);
                    assert_eq!(page.items[1].track_count, Some(0));
                }
                for album in page.items {
                    assert_eq!(album.extensions["subscribed"], true);
                    assert_eq!(
                        album.artists[0].resource_ref.as_ref().unwrap().to_string(),
                        "soda:31"
                    );
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
                assert_eq!(
                    fixture.stored("other").unwrap().secret(),
                    source.serialize().unwrap()
                );
                let requests = server.await.unwrap();
                assert_eq!(requests.len(), 4);
                for (index, cookie) in ["verified", "one", "two"].into_iter().enumerate() {
                    assert!(requests[index + 1].starts_with("GET /luna/pc/me/collection/mixed?"));
                    assert!(requests[index + 1].contains("count=100"));
                    assert!(requests[index + 1].contains(&format!("sessionid_ss={cookie}")));
                }
            }
        }
    }
}

#[tokio::test]
async fn album_collection_failures_do_not_accept_bad_pages_or_silently_return_a_partial_list() {
    for caller in [false, true] {
        for mode in [
            "missing",
            "duplicate",
            "total",
            "cursor",
            "false_state",
            "malformed",
            "auth",
        ] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("personal", &source);
            let mut last = mixed(vec![album("22", Some(1))], None, Some(2));
            match mode {
                "missing" => last["mixed_collections"] = json!([]),
                "duplicate" => last["mixed_collections"][0] = album("11", Some(1)),
                "total" => last["total_num"] = json!(3),
                "cursor" => {
                    last["has_more"] = json!(true);
                    last["next_cursor"] = json!("0");
                }
                "false_state" => {
                    last["mixed_collections"][0]["album"]["state"]["is_collected"] = json!(false)
                }
                "malformed" => last["mixed_collections"][0]["album"] = json!(null),
                "auth" => {
                    last["status_code"] = json!(1000016);
                    last["mixed_collections"] = json!("bad");
                }
                _ => unreachable!(),
            }
            let (origin, server) = crate::test_http::serve(vec![
                account_reply("123456", None),
                reply(
                    mixed(vec![album("11", None)], Some("next"), Some(2)),
                    Some("sessionid_ss=accepted"),
                ),
                reply(last, Some("sessionid_ss=poison")),
            ])
            .await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(origin);
            let provider = selected(&fixture, &source, caller);
            let error = provider
                .account_albums(&PageRequest {
                    account: alias(caller).map(str::to_owned),
                    ..PageRequest::new(1, 0)
                })
                .await
                .unwrap_err();
            assert_eq!(
                error.code,
                if mode == "auth" {
                    ErrorCode::AuthenticationRequired
                } else {
                    ErrorCode::UpstreamError
                }
            );
            let current = provider
                .selected_credential(alias(caller).unwrap_or("default"))
                .unwrap()
                .unwrap()
                .0
                .serialize()
                .unwrap();
            assert!(current.contains("accepted"));
            assert!(!current.contains("poison"));
            assert_eq!(server.await.unwrap().len(), 3);
        }
    }
}

#[tokio::test]
async fn album_collection_writes_verify_complete_readback_and_preserve_each_rotation() {
    for caller in [false, true] {
        for subscribed in [false, true] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("personal", &source);
            let (origin, server) = crate::test_http::serve(vec![
                account_reply("123456", None),
                reply(json!({"status_code":0}), Some("sessionid_ss=written")),
                reply(
                    mixed(
                        vec![album(if subscribed { "11" } else { "22" }, Some(1))],
                        Some("next"),
                        Some(2),
                    ),
                    Some("sessionid_ss=read-one"),
                ),
                reply(
                    mixed(vec![playlist("11")], None, Some(2)),
                    Some("sessionid_ss=read-two"),
                ),
            ])
            .await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(origin);
            let provider = selected(&fixture, &source, caller);
            let result = provider
                .set_album_subscription("11", subscribed, alias(caller))
                .await
                .unwrap();
            assert_eq!(result.subscribed, subscribed);
            assert_eq!(result.resource_ref.to_string(), "soda:11");
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 4);
            assert!(requests[1].starts_with(if subscribed {
                "POST /luna/pc/me/collection/album?"
            } else {
                "POST /luna/pc/me/collection/album/delete?"
            }));
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(
                    requests[1].split("\r\n\r\n").nth(1).unwrap()
                )
                .unwrap(),
                json!({"album_ids":["11"]})
            );
            assert!(requests[2].contains("sessionid_ss=written"));
            assert!(requests[3].contains("sessionid_ss=read-one"));
            if caller {
                assert!(
                    provider
                        .take_response_credential()
                        .unwrap()
                        .unwrap()
                        .secret()
                        .contains("read-two")
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
                        .contains("read-two")
                );
            }
        }
    }
}

#[tokio::test]
async fn album_collection_removal_never_infers_absence_from_unknown_or_incomplete_readback() {
    for mode in [
        "write",
        "missing_total",
        "unknown_kind",
        "still_collected",
        "incomplete",
        "auth",
    ] {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        let mut replies = vec![account_reply("123456", None)];
        if mode == "write" {
            replies.push(reply(json!({"status_code":7}), Some("sessionid_ss=poison")));
        } else {
            replies.push(reply(
                json!({"status_code":0}),
                Some("sessionid_ss=written"),
            ));
            let last = match mode {
                "missing_total" => mixed(vec![], None, None),
                "unknown_kind" => mixed(vec![json!({"item_type":"new_kind"})], None, Some(1)),
                "still_collected" => mixed(vec![album("11", Some(1))], None, Some(1)),
                "incomplete" => mixed(vec![], None, Some(1)),
                "auth" => json!({"status_code":1000016}),
                _ => unreachable!(),
            };
            replies.push(reply(last, Some("sessionid_ss=last")));
        }
        let count = replies.len();
        let (origin, server) = crate::test_http::serve(replies).await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(origin);
        let provider = selected(&fixture, &source, true);
        let error = provider
            .set_album_subscription("11", false, None)
            .await
            .unwrap_err();
        assert_eq!(error.details["write_outcome"], "unconfirmed");
        assert!(!error.retryable);
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), count);
        assert_eq!(
            requests.iter().filter(|r| r.starts_with("POST ")).count(),
            1
        );
    }
}

#[tokio::test]
async fn album_collection_batches_keep_one_source_and_report_confirmed_prefix_before_stopping() {
    for caller in [false, true] {
        for fails in [false, true] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("personal", &source);
            let mut replies = vec![
                account_reply("123456", Some("sessionid_ss=verified")),
                reply(json!({"status_code":0}), Some("sessionid_ss=first-write")),
                reply(
                    mixed(vec![album("11", None)], None, Some(1)),
                    Some("sessionid_ss=first-confirmed"),
                ),
            ];
            if fails {
                replies.push(reply(json!({"status_code":7}), Some("sessionid_ss=poison")));
            } else {
                replies.push(reply(
                    json!({"status_code":0}),
                    Some("sessionid_ss=second-write"),
                ));
                replies.push(reply(
                    mixed(vec![album("11", None), album("22", Some(1))], None, Some(2)),
                    Some("sessionid_ss=second-confirmed"),
                ));
            }
            let (origin, server) = crate::test_http::serve(replies).await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(origin);
            let provider = selected(&fixture, &source, caller);
            let ids = if fails {
                vec!["11".to_owned(), "22".to_owned(), "33".to_owned()]
            } else {
                vec!["11".to_owned(), "22".to_owned()]
            };
            let result = provider
                .set_album_subscriptions(&ids, true, alias(caller))
                .await;
            if fails {
                let error = result.unwrap_err();
                assert_eq!(error.details["completed_refs"], json!(["soda:11"]));
                assert_eq!(error.details["failed_ref"], "soda:22");
                assert_eq!(error.details["remaining_refs"], json!(["soda:33"]));
                assert_eq!(error.details["atomic"], false);
            } else {
                let results = result.unwrap();
                assert_eq!(results.len(), 2);
                assert_eq!(results[1].resource_ref.to_string(), "soda:22");
            }
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), if fails { 4 } else { 5 });
            assert!(requests[3].starts_with("POST /luna/pc/me/collection/album?"));
            assert!(requests[3].contains("sessionid_ss=first-confirmed"));
            assert_eq!(
                requests
                    .iter()
                    .filter(|r| r.starts_with("GET /luna/pc/me?"))
                    .count(),
                1
            );
            let current = provider
                .selected_credential(alias(caller).unwrap_or("default"))
                .unwrap()
                .unwrap()
                .0
                .serialize()
                .unwrap();
            assert!(!current.contains("poison"));
            assert!(current.contains(if fails {
                "first-confirmed"
            } else {
                "second-confirmed"
            }));
        }
    }
}

#[tokio::test]
async fn album_collections_validate_all_inputs_before_network_or_any_batch_write() {
    let fixture = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    let provider = selected(&fixture, &source, true);
    for ids in [
        vec![],
        vec!["11".to_owned(), "01".to_owned()],
        vec!["11".to_owned(); 101],
    ] {
        assert_eq!(
            provider
                .set_album_subscriptions(&ids, true, None)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        provider
            .user_favorite_albums("654321", &PageRequest::new(20, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        provider
            .user_favorite_albums("01", &PageRequest::new(20, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        provider
            .account_albums(&PageRequest::new(101, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        fixture
            .provider
            .account_albums(&PageRequest::new(20, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
}

#[tokio::test]
async fn album_collections_late_readback_cannot_cross_server_or_caller_login_generations() {
    for caller in [false, true] {
        for operation in ["read", "write", "batch"] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("personal", &source);
            let mut replies = vec![account_reply("123456", None)];
            if operation != "read" {
                replies.push(reply(json!({"status_code":0}), None));
            }
            replies.push(reply(mixed(vec![album("11", None)], None, Some(1)), None));
            let last = replies.len() - 1;
            let paused = crate::test_http::serve_paused_at(replies, last).await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(paused.origin);
            let provider = selected(&fixture, &source, caller);
            let task_provider = provider.clone();
            let pending = tokio::spawn(async move {
                match operation {
                    "read" => task_provider
                        .account_albums(&PageRequest {
                            account: alias(caller).map(str::to_owned),
                            ..PageRequest::new(20, 0)
                        })
                        .await
                        .map(|_| ()),
                    "write" => task_provider
                        .set_album_subscription("11", true, alias(caller))
                        .await
                        .map(|_| ()),
                    "batch" => task_provider
                        .set_album_subscriptions(
                            &["11".to_owned(), "22".to_owned()],
                            true,
                            alias(caller),
                        )
                        .await
                        .map(|_| ()),
                    _ => unreachable!(),
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
