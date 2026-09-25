use super::*;

fn owner_alias(owner: &str) -> Option<&str> {
    (owner == "personal").then_some("personal")
}

fn responses(operation: &str) -> Vec<String> {
    let mut responses = vec![account_reply("123456", Some("sessionid_ss=verified"))];
    if operation == "read" {
        responses.push(reply(
            mixed(vec![album("11", None)], Some("next"), Some(2)),
            Some("sessionid_ss=page-one"),
        ));
        responses.push(reply(
            mixed(vec![album("22", None)], None, Some(2)),
            Some("sessionid_ss=complete"),
        ));
    } else {
        responses.push(reply(
            json!({"status_code":0}),
            Some("sessionid_ss=written"),
        ));
        responses.push(reply(
            mixed(vec![album("11", None)], None, Some(1)),
            Some("sessionid_ss=first-confirmed"),
        ));
        if operation == "batch" {
            responses.push(reply(json!({"status_code":0}), Some("sessionid_ss=second")));
            responses.push(reply(
                mixed(vec![album("11", None), album("22", None)], None, Some(2)),
                Some("sessionid_ss=second-confirmed"),
            ));
        }
    }
    responses
}

async fn run_with_budget(provider: &SodaProvider, operation: &str, owner: &str) -> Result<()> {
    let budget = std::time::Duration::from_secs(5);
    match operation {
        "read" => provider
            .read_album_collections_with_budget(
                None,
                &PageRequest {
                    account: owner_alias(owner).map(str::to_owned),
                    ..PageRequest::new(20, 0)
                },
                budget,
            )
            .await
            .map(|_| ()),
        "write" => provider
            .change_album_collection_with_budget("11", true, owner_alias(owner), budget)
            .await
            .map(|_| ()),
        "batch" => provider
            .change_album_collections_with_budget(
                &["11".into(), "22".into(), "33".into()],
                true,
                owner_alias(owner),
                budget,
            )
            .await
            .map(|_| ()),
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn album_lifecycle_cancel_and_total_timeout_clear_updates_and_keep_confirmed_batch_prefix() {
    for owner in ["default", "personal", "caller"] {
        for (operation, boundaries) in [
            ("read", vec![0, 2]),
            ("write", vec![0, 1, 2]),
            ("batch", vec![0, 3, 4]),
        ] {
            for boundary in boundaries {
                for timeout in [false, true] {
                    let mut fixture = SessionFixture::new();
                    let source = test_soda_credential().bind_user("123456").unwrap();
                    for name in ["default", "personal", "other"] {
                        fixture.put(name, &source);
                    }
                    let mut replies = responses(operation);
                    replies.truncate(boundary + 1);
                    let paused = crate::test_http::serve_paused_at(replies, boundary).await;
                    fixture.provider.client = fixture
                        .provider
                        .client
                        .clone()
                        .with_auth_test_origin(paused.origin);
                    let provider = selected(&fixture, &source, owner == "caller");
                    let worker = provider.clone();
                    let task =
                        tokio::spawn(
                            async move { run_with_budget(&worker, operation, owner).await },
                        );
                    paused.arrived.await.unwrap();
                    let accepted = provider
                        .selected_credential(owner_alias(owner).unwrap_or("default"))
                        .unwrap()
                        .unwrap()
                        .0;
                    assert!(accepted.same_login(&source));
                    if timeout {
                        tokio::time::pause();
                        tokio::time::advance(std::time::Duration::from_secs(6)).await;
                        let error = task.await.unwrap().unwrap_err();
                        tokio::time::resume();
                        assert_eq!(
                            error.code,
                            ErrorCode::UpstreamTimeout,
                            "{owner}/{operation}/{boundary}"
                        );
                        if operation != "read" && boundary > 0 {
                            assert_eq!(error.details["write_outcome"], "unconfirmed");
                            assert!(!error.retryable);
                            if operation == "batch" {
                                assert_eq!(error.details["completed_refs"], json!(["soda:11"]));
                                assert_eq!(error.details["failed_ref"], "soda:22");
                                assert_eq!(error.details["remaining_refs"], json!(["soda:33"]));
                            }
                        } else {
                            assert!(error.details.get("write_outcome").is_none());
                            assert!(error.retryable);
                        }
                    } else {
                        task.abort();
                        assert!(task.await.unwrap_err().is_cancelled());
                    }
                    paused.requests.abort();
                    assert!(paused.requests.await.unwrap_err().is_cancelled());
                    assert!(provider.take_response_credential().unwrap().is_none());
                    assert_eq!(
                        provider
                            .selected_credential(owner_alias(owner).unwrap_or("default"))
                            .unwrap()
                            .unwrap()
                            .0,
                        accepted
                    );
                    assert_eq!(
                        fixture.stored("other").unwrap().secret(),
                        source.serialize().unwrap()
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn album_lifecycle_authentication_errors_suppress_only_pending_delivery_of_verified_rotation()
{
    for (operation, boundary, retained) in [
        ("read", 2, "page-one"),
        ("write", 1, "verified"),
        ("batch", 3, "first-confirmed"),
    ] {
        for authentication in [false, true] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            let mut replies = responses(operation);
            replies.truncate(boundary + 1);
            replies[boundary] = reply(
                json!({"status_code":if authentication { 1000016 } else { 99 }}),
                Some("sessionid_ss=poison"),
            );
            let (origin, server) = crate::test_http::serve(replies).await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(origin);
            let provider = selected(&fixture, &source, true);
            let error = run_with_budget(&provider, operation, "caller")
                .await
                .unwrap_err();
            assert_eq!(
                error.code,
                if authentication {
                    ErrorCode::AuthenticationRequired
                } else {
                    ErrorCode::UpstreamError
                }
            );
            let update = provider.take_response_credential().unwrap();
            if authentication {
                assert!(update.is_none());
            } else {
                let update = update.unwrap();
                assert!(update.secret().contains(retained));
                assert!(!update.secret().contains("poison"));
            }
            assert!(
                provider
                    .selected_credential("default")
                    .unwrap()
                    .unwrap()
                    .0
                    .serialize()
                    .unwrap()
                    .contains(retained)
            );
            assert_eq!(server.await.unwrap().len(), boundary + 1);
        }
    }
}

#[tokio::test]
async fn album_lifecycle_late_write_errors_belong_to_original_login_and_stop_batches() {
    for owner in ["default", "personal", "caller"] {
        for (operation, boundary) in [("write", 1), ("batch", 3)] {
            for failure in ["unauthorized", "business"] {
                let mut fixture = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                for name in ["default", "personal", "other"] {
                    fixture.put(name, &source);
                }
                let mut replies = responses(operation);
                replies.truncate(boundary + 1);
                replies[boundary] = if failure == "unauthorized" {
                    "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .into()
                } else {
                    reply(json!({"status_code":99}), Some("sessionid_ss=poison"))
                };
                let paused = crate::test_http::serve_paused_at(replies, boundary).await;
                fixture.provider.client = fixture
                    .provider
                    .client
                    .clone()
                    .with_auth_test_origin(paused.origin);
                let provider = selected(&fixture, &source, owner == "caller");
                let worker = provider.clone();
                let task = tokio::spawn(async move {
                    if operation == "write" {
                        worker
                            .set_album_subscription("11", true, owner_alias(owner))
                            .await
                            .map(|_| ())
                    } else {
                        worker
                            .set_album_subscriptions(
                                &["11".into(), "22".into(), "33".into()],
                                true,
                                owner_alias(owner),
                            )
                            .await
                            .map(|_| ())
                    }
                });
                paused.arrived.await.unwrap();
                let current = provider
                    .selected_credential(owner_alias(owner).unwrap_or("default"))
                    .unwrap()
                    .unwrap()
                    .0;
                let cookie = current.cookie_header().unwrap();
                let replacement =
                    SodaCredential::test_credential(cookie.strip_prefix("sessionid_ss=").unwrap())
                        .bind_user("123456")
                        .unwrap();
                if owner == "caller" {
                    *provider.caller_credential.as_ref().unwrap().lock().unwrap() =
                        replacement.clone();
                } else {
                    fixture.put(owner, &replacement);
                }
                paused.release.send(()).unwrap();
                let error = task.await.unwrap().unwrap_err();
                assert_eq!(
                    error.code,
                    ErrorCode::Conflict,
                    "{owner}/{operation}/{failure}"
                );
                assert_eq!(error.details["write_outcome"], "unconfirmed");
                assert!(!error.retryable);
                if operation == "batch" {
                    assert_eq!(error.details["completed_refs"], json!(["soda:11"]));
                    assert_eq!(error.details["failed_ref"], "soda:22");
                    assert_eq!(error.details["remaining_refs"], json!(["soda:33"]));
                }
                assert!(provider.take_response_credential().unwrap().is_none());
                assert_eq!(
                    provider
                        .selected_credential(owner_alias(owner).unwrap_or("default"))
                        .unwrap()
                        .unwrap()
                        .0,
                    replacement
                );
                assert_eq!(
                    fixture.stored("other").unwrap().secret(),
                    source.serialize().unwrap()
                );
                let requests = paused.requests.await.unwrap();
                assert_eq!(requests.len(), boundary + 1);
                assert_eq!(
                    requests.iter().filter(|r| r.starts_with("POST ")).count(),
                    if operation == "batch" { 2 } else { 1 }
                );
            }
        }
    }
}

#[tokio::test]
async fn saved_playlist_removal_requires_counted_directory_without_unknown_collection_types() {
    for owner in ["default", "personal", "caller"] {
        for (items, total, confirmed) in [
            (vec![], None, false),
            (vec![json!({"item_type":"future_playlist"})], Some(1), false),
            (vec![json!({"item_type":"album"})], Some(1), true),
            (vec![], Some(0), true),
        ] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            for name in ["default", "personal"] {
                fixture.put(name, &source);
            }
            let (origin, server) = crate::test_http::serve(vec![
                account_reply("123456", None),
                reply(json!({"status_code":0}), Some("sessionid_ss=written")),
                reply(mixed(items, None, total), Some("sessionid_ss=readback")),
            ])
            .await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(origin);
            let provider = selected(&fixture, &source, owner == "caller");
            let result = provider
                .set_playlist_subscription("11", false, owner_alias(owner))
                .await;
            if confirmed {
                assert!(!result.unwrap().subscribed);
            } else {
                let error = result.unwrap_err();
                assert_eq!(error.code, ErrorCode::UpstreamError);
                assert_eq!(error.details["write_outcome"], "unconfirmed");
                assert!(!error.retryable);
            }
            assert_eq!(server.await.unwrap().len(), 3);
        }
    }
}
