use super::*;

fn alias(owner: &str) -> Option<&str> {
    match owner {
        "caller" | "default" => None,
        _ => Some(owner),
    }
}

fn selected(fixture: &SessionFixture, owner: &str, source: &SodaCredential) -> SodaProvider {
    if owner == "caller" {
        fixture
            .provider
            .caller_credential_scope(&caller_from(source))
            .unwrap()
    } else {
        fixture.provider.clone()
    }
}

fn responses(write: bool) -> Vec<String> {
    let mut responses = vec![account_reply("123456", Some("sessionid_ss=verified"))];
    if write {
        responses.push(crate::test_http::json(
            r#"{"status_code":0}"#,
            Some("sessionid_ss=written"),
        ));
    } else {
        responses.extend([
            crate::test_http::json(r#"{"playlists":[{"id":"11","title":"first"}],"has_more":true,"next_cursor":"created-next","total_num":2}"#, Some("sessionid_ss=created-first")),
            crate::test_http::json(r#"{"playlists":[{"id":"12","title":"second"}],"has_more":false,"total_num":2}"#, Some("sessionid_ss=created-last")),
        ]);
    }
    responses.extend([
        crate::test_http::json(r#"{"mixed_collections":[{"item_type":"playlist","playlist":{"id":"21","title":"saved"}}],"has_more":true,"next_cursor":"saved-next","total_num":2}"#, Some("sessionid_ss=saved-first")),
        crate::test_http::json(r#"{"mixed_collections":[{"item_type":"playlist","playlist":{"id":"22","title":"last"}}],"has_more":false,"total_num":2}"#, Some("sessionid_ss=saved-last")),
    ]);
    responses
}

async fn operation(provider: &SodaProvider, owner: &str, write: bool) -> Result<()> {
    if write {
        provider
            .set_playlist_subscription("21", true, alias(owner))
            .await
            .map(|_| ())
    } else {
        provider
            .account_playlists(&PageRequest {
                limit: 1,
                offset: 2,
                account: alias(owner).map(str::to_owned),
            })
            .await
            .map(|_| ())
    }
}

async fn late_responses(write: bool) {
    for owner in ["default", "personal", "caller"] {
        for boundary in 0..responses(write).len() {
            for response_kind in ["success", "unauthorized", "business_error", "malformed"] {
                for change in ["same_cookie_login", "other_user", "logout"] {
                    if owner == "caller" && change == "logout" {
                        continue;
                    }
                    let mut fixture = SessionFixture::new();
                    let source = test_soda_credential().bind_user("123456").unwrap();
                    for account in ["default", "personal", "other"] {
                        fixture.put(account, &source);
                    }
                    let mut replies = responses(write);
                    replies.truncate(boundary + 1);
                    replies[boundary] = match response_kind {
                        "unauthorized" => "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_owned(),
                        "business_error" => crate::test_http::json(r#"{"status_code":99}"#, Some("sessionid_ss=unaccepted")),
                        "malformed" => crate::test_http::json("{", Some("sessionid_ss=unaccepted")),
                        _ => replies[boundary].clone(),
                    };
                    let paused = crate::test_http::serve_paused_at(replies, boundary).await;
                    fixture.provider.client = fixture
                        .provider
                        .client
                        .clone()
                        .with_auth_test_origin(paused.origin.clone());
                    let provider = selected(&fixture, owner, &source);
                    let worker = provider.clone();
                    let task = tokio::spawn(async move { operation(&worker, owner, write).await });
                    paused.arrived.await.unwrap();
                    let old = provider
                        .selected_credential(alias(owner).unwrap_or("default"))
                        .unwrap()
                        .unwrap()
                        .0;
                    let cookie = old.cookie_header().unwrap();
                    let replacement = if change == "same_cookie_login" {
                        SodaCredential::test_credential(
                            cookie.strip_prefix("sessionid_ss=").unwrap(),
                        )
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
                    let error = task.await.unwrap().unwrap_err();
                    assert_eq!(
                        error.code,
                        ErrorCode::Conflict,
                        "{owner}/{write}/{boundary}/{response_kind}/{change}"
                    );
                    if write && boundary > 0 {
                        assert_eq!(error.details["write_outcome"], "unconfirmed");
                        assert!(!error.retryable);
                    } else {
                        assert!(error.details.get("write_outcome").is_none());
                    }
                    assert!(provider.take_response_credential().unwrap().is_none());
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
                    for untouched in ["default", "personal", "other"]
                        .into_iter()
                        .filter(|a| *a != owner)
                    {
                        assert_eq!(
                            fixture.stored(untouched).unwrap().secret(),
                            source.serialize().unwrap()
                        );
                    }
                    let requests = paused.requests.await.unwrap();
                    assert_eq!(requests.len(), boundary + 1);
                    assert_eq!(
                        requests.iter().filter(|r| r.starts_with("POST ")).count(),
                        usize::from(write && boundary > 0)
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn library_directory_late_success_and_errors_belong_to_the_original_login() {
    late_responses(false).await;
}

#[tokio::test]
async fn library_collection_late_success_and_errors_belong_to_the_original_login() {
    late_responses(true).await;
}

async fn interrupted_operations(write: bool) {
    for owner in ["default", "personal", "caller"] {
        for boundary in 0..responses(write).len() {
            for timeout in [false, true] {
                let mut fixture = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                for account in ["default", "personal", "other"] {
                    fixture.put(account, &source);
                }
                let mut replies = responses(write);
                replies.truncate(boundary + 1);
                let paused = crate::test_http::serve_paused_at(replies, boundary).await;
                fixture.provider.client = fixture
                    .provider
                    .client
                    .clone()
                    .with_auth_test_origin(paused.origin.clone());
                let provider = selected(&fixture, owner, &source);
                let worker = provider.clone();
                let task = tokio::spawn(async move {
                    let budget = std::time::Duration::from_secs(5);
                    if write {
                        worker
                            .change_playlist_collection("21", true, alias(owner), budget)
                            .await
                            .map(|_| ())
                    } else {
                        worker
                            .read_library_playlists(
                                &PageRequest {
                                    limit: 1,
                                    offset: 2,
                                    account: alias(owner).map(str::to_owned),
                                },
                                budget,
                            )
                            .await
                            .map(|_| ())
                    }
                });
                paused.arrived.await.unwrap();
                let accepted = provider
                    .selected_credential(alias(owner).unwrap_or("default"))
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
                        "{owner}/{write}/{boundary}"
                    );
                    if write && boundary > 0 {
                        assert!(!error.retryable);
                        assert_eq!(error.details["write_outcome"], "unconfirmed");
                    } else {
                        assert!(error.retryable);
                        assert!(error.details.get("write_outcome").is_none());
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
                        .selected_credential(alias(owner).unwrap_or("default"))
                        .unwrap()
                        .unwrap()
                        .0,
                    accepted
                );
                for untouched in ["default", "personal", "other"]
                    .into_iter()
                    .filter(|a| *a != owner)
                {
                    assert_eq!(
                        fixture.stored(untouched).unwrap().secret(),
                        source.serialize().unwrap()
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn library_directory_cancellation_and_total_timeout_clear_pending_updates() {
    interrupted_operations(false).await;
}

#[tokio::test]
async fn library_collection_cancellation_and_total_timeout_clear_pending_updates() {
    interrupted_operations(true).await;
}

#[tokio::test]
async fn shared_collection_identity_helper_rejects_late_authentication_errors() {
    for owner in ["default", "personal", "caller"] {
        for favorites in [false, true] {
            let mut fixture = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            fixture.put("default", &source);
            fixture.put("personal", &source);
            let paused = crate::test_http::serve_paused(crate::test_http::json(
                r#"{"status_code":1000016}"#,
                None,
            ))
            .await;
            fixture.provider.client = fixture
                .provider
                .client
                .clone()
                .with_auth_test_origin(paused.origin.clone());
            let provider = selected(&fixture, owner, &source);
            let worker = provider.clone();
            let task = tokio::spawn(async move {
                if favorites {
                    worker.favorite_playlist(alias(owner)).await.map(|_| ())
                } else {
                    worker
                        .account_albums(&PageRequest {
                            limit: 1,
                            offset: 0,
                            account: alias(owner).map(str::to_owned),
                        })
                        .await
                        .map(|_| ())
                }
            });
            paused.arrived.await.unwrap();
            let replacement = test_soda_credential().bind_user("123456").unwrap();
            if owner == "caller" {
                *provider.caller_credential.as_ref().unwrap().lock().unwrap() = replacement.clone();
            } else {
                fixture.put(owner, &replacement);
            }
            paused.release.send(()).unwrap();
            assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
            assert!(provider.take_response_credential().unwrap().is_none());
            assert_eq!(
                provider
                    .selected_credential(alias(owner).unwrap_or("default"))
                    .unwrap()
                    .unwrap()
                    .0,
                replacement
            );
            assert_eq!(paused.requests.await.unwrap().len(), 1);
        }
    }
}
