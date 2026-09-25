use super::*;

fn owner_alias(owner: &str) -> Option<&str> {
    (owner == "personal").then_some("personal")
}

fn owned(fixture: &SessionFixture, source: &SodaCredential, owner: &str) -> SodaProvider {
    selected(fixture, source, owner == "caller")
}

fn replies(writing: bool) -> Vec<String> {
    let mut replies = vec![
        account_reply("123456", Some("sessionid_ss=verified")),
        crate::test_http::json(&chosen_directory(), Some("sessionid_ss=directory")),
    ];
    if writing {
        replies.push(crate::test_http::json(
            r#"{"status_code":0}"#,
            Some("sessionid_ss=written"),
        ));
    }
    replies.push(json_reply(
        &detail(&[], 0, None),
        Some("sessionid_ss=detail"),
    ));
    if writing {
        replies.push(json_reply(
            &track_state(Some(false)),
            Some("sessionid_ss=track"),
        ));
    }
    replies
}

#[tokio::test]
async fn favorite_lifecycle_late_write_errors_cannot_belong_to_a_replacement_login() {
    for owner in ["default", "personal", "caller"] {
        for boundary in [2, 4] {
            for failure in ["unauthorized", "business", "malformed"] {
                let mut fixture = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                for alias in ["default", "personal", "other"] {
                    fixture.put(alias, &source);
                }
                let mut responses = replies(true);
                responses.truncate(boundary + 1);
                responses[boundary] = match failure {
                    "unauthorized" => "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
                    "business" => crate::test_http::json(r#"{"status_code":99}"#, Some("sessionid_ss=poison")),
                    _ => crate::test_http::json("{", Some("sessionid_ss=poison")),
                };
                let paused = crate::test_http::serve_paused_at(responses, boundary).await;
                fixture.provider.client = fixture
                    .provider
                    .client
                    .clone()
                    .with_auth_test_origin(paused.origin);
                let provider = owned(&fixture, &source, owner);
                let worker = provider.clone();
                let task = tokio::spawn(async move {
                    worker
                        .set_track_subscription("11", false, owner_alias(owner))
                        .await
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
                    "{owner}/{boundary}/{failure}"
                );
                assert_eq!(error.details["write_outcome"], "unconfirmed");
                assert!(!error.retryable);
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
                let seen = paused.requests.await.unwrap();
                assert_eq!(seen.len(), boundary + 1);
                assert_eq!(
                    seen.iter()
                        .filter(|r| r.starts_with("POST /luna/pc/me/collection/media/delete?"))
                        .count(),
                    1
                );
            }
        }
    }
}

#[tokio::test]
async fn favorite_lifecycle_cancel_and_timeout_clear_pending_rotations_without_replaying_writes() {
    for owner in ["default", "personal", "caller"] {
        for writing in [false, true] {
            for boundary in 0..replies(writing).len() {
                for timeout in [false, true] {
                    let mut fixture = SessionFixture::new();
                    let source = test_soda_credential().bind_user("123456").unwrap();
                    for alias in ["default", "personal", "other"] {
                        fixture.put(alias, &source);
                    }
                    let mut responses = replies(writing);
                    responses.truncate(boundary + 1);
                    let paused = crate::test_http::serve_paused_at(responses, boundary).await;
                    fixture.provider.client = fixture
                        .provider
                        .client
                        .clone()
                        .with_auth_test_origin(paused.origin);
                    let provider = owned(&fixture, &source, owner);
                    let worker = provider.clone();
                    let task = tokio::spawn(async move {
                        let budget = std::time::Duration::from_secs(5);
                        if writing {
                            worker
                                .set_favorite_track_with_budget(
                                    "11",
                                    false,
                                    owner_alias(owner),
                                    budget,
                                )
                                .await
                                .map(|_| ())
                        } else {
                            worker
                                .read_favorite_snapshot_with_budget(
                                    None,
                                    owner_alias(owner),
                                    budget,
                                )
                                .await
                                .map(|_| ())
                        }
                    });
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
                            "{owner}/{writing}/{boundary}"
                        );
                        if writing && boundary >= 2 {
                            assert_eq!(error.details["write_outcome"], "unconfirmed");
                            assert!(!error.retryable);
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
async fn favorite_lifecycle_session_errors_suppress_rotations_but_business_errors_preserve_verified_updates()
 {
    for writing in [false, true] {
        for invalid_session in [false, true] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            let mut responses = replies(writing);
            let last = responses.len() - 1;
            responses[last] = crate::test_http::json(
                if invalid_session {
                    r#"{"status_code":1000016}"#
                } else {
                    r#"{"status_code":99}"#
                },
                Some("sessionid_ss=poison"),
            );
            let (origin, server) = crate::test_http::serve(responses).await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(origin);
            let provider = owned(&fixture, &source, "caller");
            let error = if writing {
                provider
                    .set_track_subscription("11", false, None)
                    .await
                    .unwrap_err()
            } else {
                provider.favorite_playlist(None).await.unwrap_err()
            };
            let update = provider.take_response_credential().unwrap();
            if invalid_session {
                assert_eq!(error.code, ErrorCode::AuthenticationRequired);
                assert!(update.is_none());
            } else {
                assert_eq!(error.code, ErrorCode::UpstreamError);
                let update = update.unwrap();
                assert!(
                    update
                        .secret()
                        .contains(if writing { "detail" } else { "directory" })
                );
                assert!(!update.secret().contains("poison"));
            }
            let seen = server.await.unwrap();
            assert_eq!(seen.len(), last + 1);
            assert_eq!(
                seen.iter()
                    .filter(|r| r.starts_with("POST /luna/pc/me/collection/media/delete?"))
                    .count(),
                usize::from(writing)
            );
        }
    }
}
