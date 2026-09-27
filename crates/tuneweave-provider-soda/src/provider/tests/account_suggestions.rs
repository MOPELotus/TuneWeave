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

fn request_for_client(owner: &str, client: SearchSuggestionClient) -> SearchSuggestionRequest {
    SearchSuggestionRequest {
        query: " 周 ".to_owned(),
        client,
        account: alias(owner).map(str::to_owned),
    }
}
fn request_for(owner: &str) -> SearchSuggestionRequest {
    request_for_client(owner, SearchSuggestionClient::Pc)
}
fn suggestions_body() -> serde_json::Value {
    json!({"status_info":{"now":1,"now_ts_ms":1000},"sugs":[
        {"suggestion":"周杰伦","content_type":"artist","entity":{"secret":"not-for-output"}},
        {"suggestion":"周杰伦稻香","content_type":"track"}
    ]})
}

#[tokio::test]
async fn pc_and_mobile_account_suggestions_accept_only_selected_source_and_final_rotation() {
    for client in [SearchSuggestionClient::Pc, SearchSuggestionClient::Mobile] {
        for owner in ["default", "personal", "caller"] {
            for empty in [false, true] {
                let mut fixture = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                for account in ["default", "personal", "other"] {
                    fixture.put(account, &source);
                }
                let mut body = suggestions_body();
                if empty {
                    body.as_object_mut().unwrap().remove("sugs");
                }
                let (origin, server) = crate::test_http::serve(vec![
                    account_reply("123456", Some("sessionid_ss=verified; Path=/")),
                    reply(&body, Some("sessionid_ss=final; Path=/")),
                ])
                .await;
                fixture.provider.client = fixture
                    .provider
                    .client
                    .clone()
                    .with_auth_test_origin(origin);
                let provider = owner_provider(&fixture, owner, &source);
                let result = provider
                    .search_suggestions(&request_for_client(owner, client))
                    .await
                    .unwrap();
                assert_eq!(result.query, "周");
                assert_eq!(result.client, client);
                assert_eq!(result.suggestions.len(), if empty { 0 } else { 2 });
                assert_eq!(result.extensions["source_user_id"], "123456");
                assert_eq!(result.extensions["authenticated"], true);
                assert_eq!(
                    result.extensions["backend"],
                    if client == SearchSuggestionClient::Pc {
                        "official_pc_account_sug"
                    } else {
                        "official_android_account_sug"
                    }
                );
                for secret in ["session-secret", "sessionid_ss", "final", "not-for-output"] {
                    assert!(!serde_json::to_string(&result).unwrap().contains(secret));
                }
                for untouched in ["default", "personal", "other"]
                    .into_iter()
                    .filter(|a| *a != owner)
                {
                    assert_eq!(
                        fixture.stored(untouched).unwrap().secret(),
                        source.serialize().unwrap()
                    );
                }
                if owner == "caller" {
                    assert!(
                        provider
                            .take_response_credential()
                            .unwrap()
                            .unwrap()
                            .secret()
                            .contains("final")
                    );
                } else {
                    assert!(fixture.stored(owner).unwrap().secret().contains("final"));
                    assert!(provider.take_response_credential().unwrap().is_none());
                }
                let requests = server.await.unwrap();
                assert_eq!(requests.len(), 2);
                assert!(requests[0].starts_with("GET /luna/pc/me?aid=386088&app_name=luna_pc"));
                assert!(requests[0].contains("sessionid_ss=session-secret"));
                assert!(
                    requests[1].starts_with(if client == SearchSuggestionClient::Pc {
                        "GET /luna/pc/sug?"
                    } else {
                        "GET /luna/sug?"
                    })
                );
                assert!(requests[1].contains("sessionid_ss=verified"));
                if client == SearchSuggestionClient::Mobile {
                    assert!(
                        requests[1]
                            .to_ascii_lowercase()
                            .contains("x-luna-is-login: 1")
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn account_suggestions_late_success_and_failure_observe_source_generation_at_both_boundaries()
{
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
                        reply(&suggestions_body(), Some("sessionid_ss=late; Path=/"))
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
                    let pending = tokio::spawn(async move {
                        worker.search_suggestions(&request_for(owner)).await
                    });
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
async fn account_suggestions_cancel_timeout_and_bad_response_cannot_issue_partial_credentials() {
    for owner in ["default", "personal", "caller"] {
        for boundary in 0..2 {
            for action in ["cancel", "timeout", "invalid"] {
                let mut fixture = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                for account in ["default", "personal", "other"] {
                    fixture.put(account, &source);
                }
                let failure = crate::test_http::json(
                    r#"{"status_code":9}"#,
                    Some("sessionid_ss=unaccepted; Path=/"),
                );
                let replies = if boundary == 0 {
                    vec![failure]
                } else {
                    vec![
                        account_reply("123456", Some("sessionid_ss=verified; Path=/")),
                        failure,
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
                let task = tokio::spawn(async move {
                    worker
                        .read_account_suggestions(
                            &request_for(owner),
                            std::time::Duration::from_secs(if action == "timeout" {
                                2
                            } else {
                                45
                            }),
                        )
                        .await
                });
                paused.arrived.await.unwrap();
                if action == "cancel" {
                    task.abort();
                    assert!(task.await.unwrap_err().is_cancelled());
                } else if action == "timeout" {
                    assert_eq!(
                        task.await.unwrap().unwrap_err().code,
                        ErrorCode::UpstreamTimeout
                    );
                } else {
                    paused.release.send(()).unwrap();
                    assert_eq!(
                        task.await.unwrap().unwrap_err().code,
                        ErrorCode::UpstreamError
                    );
                }
                if action != "invalid" {
                    paused.requests.abort();
                    assert!(paused.requests.await.unwrap_err().is_cancelled());
                } else {
                    assert_eq!(paused.requests.await.unwrap().len(), boundary + 1);
                }
                assert!(provider.take_response_credential().unwrap().is_none());
                for untouched in ["default", "personal", "other"]
                    .into_iter()
                    .filter(|a| *a != owner)
                {
                    assert_eq!(
                        fixture.stored(untouched).unwrap().secret(),
                        source.serialize().unwrap()
                    );
                }
                let selected = provider
                    .selected_credential(alias(owner).unwrap_or("default"))
                    .unwrap()
                    .unwrap()
                    .0;
                assert!(selected.same_login(&source));
                assert!(
                    selected
                        .cookie_header()
                        .unwrap()
                        .contains(if boundary == 0 {
                            "session-secret"
                        } else {
                            "verified"
                        })
                );
            }
        }
    }
}

#[tokio::test]
async fn mobile_account_suggestions_reject_late_responses_after_login_replacement() {
    for failure in [false, true] {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        fixture.put("personal", &source);
        fixture.put(
            "other",
            &SodaCredential::test_credential("other-secret")
                .bind_user("777777")
                .unwrap(),
        );
        let response = if failure {
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".to_owned()
        } else {
            reply(&suggestions_body(), Some("sessionid_ss=late; Path=/"))
        };
        let paused = crate::test_http::serve_paused_at(
            vec![
                account_reply("123456", Some("sessionid_ss=verified; Path=/")),
                response,
            ],
            1,
        )
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(paused.origin.clone());
        let worker = fixture.provider.clone();
        let pending = tokio::spawn(async move {
            worker
                .search_suggestions(&request_for_client(
                    "personal",
                    SearchSuggestionClient::Mobile,
                ))
                .await
        });
        paused.arrived.await.unwrap();
        let replacement = SodaCredential::test_credential("replacement-secret")
            .bind_user("654321")
            .unwrap();
        fixture.put("personal", &replacement);
        paused.release.send(()).unwrap();
        assert_eq!(
            pending.await.unwrap().unwrap_err().code,
            ErrorCode::Conflict,
            "failure={failure}"
        );
        assert!(
            fixture
                .provider
                .take_response_credential()
                .unwrap()
                .is_none()
        );
        assert_eq!(
            fixture.stored("personal").unwrap().secret(),
            replacement.serialize().unwrap()
        );
        assert!(
            fixture
                .stored("other")
                .unwrap()
                .secret()
                .contains("other-secret")
        );
        assert_eq!(paused.requests.await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn mobile_account_suggestions_cancellation_timeout_and_bad_response_discard_pending_updates()
{
    for action in ["cancel", "timeout", "invalid"] {
        let mut fixture = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        fixture.put("personal", &source);
        let failure = crate::test_http::json(
            r#"{"status_code":9}"#,
            Some("sessionid_ss=unaccepted; Path=/"),
        );
        let paused = crate::test_http::serve_paused_at(
            vec![
                account_reply("123456", Some("sessionid_ss=verified; Path=/")),
                failure,
            ],
            1,
        )
        .await;
        fixture.provider.client = fixture
            .provider
            .client
            .clone()
            .with_auth_test_origin(paused.origin.clone());
        let worker = fixture.provider.clone();
        let task = tokio::spawn(async move {
            worker
                .read_account_suggestions(
                    &request_for_client("personal", SearchSuggestionClient::Mobile),
                    std::time::Duration::from_secs(if action == "timeout" { 2 } else { 45 }),
                )
                .await
        });
        paused.arrived.await.unwrap();
        if action == "cancel" {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else if action == "timeout" {
            assert_eq!(
                task.await.unwrap().unwrap_err().code,
                ErrorCode::UpstreamTimeout
            );
        } else {
            paused.release.send(()).unwrap();
            assert_eq!(
                task.await.unwrap().unwrap_err().code,
                ErrorCode::UpstreamError
            );
        }
        if action == "invalid" {
            assert_eq!(paused.requests.await.unwrap().len(), 2);
        } else {
            paused.requests.abort();
            assert!(paused.requests.await.unwrap_err().is_cancelled());
        }
        assert!(
            fixture
                .provider
                .take_response_credential()
                .unwrap()
                .is_none()
        );
        assert!(
            fixture
                .stored("personal")
                .unwrap()
                .secret()
                .contains("verified")
        );
        assert!(
            !fixture
                .stored("personal")
                .unwrap()
                .secret()
                .contains("unaccepted")
        );
    }
}

#[tokio::test]
async fn account_suggestions_invalid_input_missing_sources_and_foreign_alias_fail_before_io() {
    let mut fixture = SessionFixture::new();
    fixture.provider.client = fixture
        .provider
        .client
        .clone()
        .with_auth_test_origin(url::Url::parse("http://127.0.0.1:1/").unwrap());
    let source = test_soda_credential().bind_user("123456").unwrap();
    fixture.put("default", &source);
    let caller = owner_provider(&fixture, "caller", &source);
    for provider in [&fixture.provider, &caller] {
        for query in [" ".to_owned(), "a\nb".to_owned(), "x".repeat(1025)] {
            assert_eq!(
                provider
                    .search_suggestions(&SearchSuggestionRequest {
                        query,
                        ..request_for("default")
                    })
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidRequest
            );
        }
        for client in [SearchSuggestionClient::Web] {
            assert_eq!(
                provider
                    .search_suggestions(&SearchSuggestionRequest {
                        client,
                        ..request_for("default")
                    })
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::CapabilityNotSupported
            );
        }
        assert!(provider.take_response_credential().unwrap().is_none());
    }
    assert_eq!(
        fixture
            .provider
            .search_suggestions(&request_for("missing"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert_eq!(
        fixture
            .provider
            .search_suggestions(&request_for_client(
                "missing",
                SearchSuggestionClient::Mobile
            ))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert_eq!(
        fixture
            .provider
            .search_suggestions(&request_for(""))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        caller
            .search_suggestions(&request_for("personal"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        caller
            .search_suggestions(&request_for_client(
                "personal",
                SearchSuggestionClient::Mobile
            ))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        fixture.stored("default").unwrap().secret(),
        source.serialize().unwrap()
    );
}
