use super::*;
use tuneweave_core::SessionRevocationState;

fn invalid() -> String {
    crate::test_http::json(r#"{"status_code":1000016}"#, None)
}
fn replies() -> Vec<String> {
    vec![
        account_reply("123456", Some("sessionid_ss=ignored-profile-cookie")),
        crate::test_http::json(r#"{"message":"success"}"#, Some("sessionid_ss=; Max-Age=0")),
        invalid(),
    ]
}
fn caller(mode: CredentialMode, credential: &ProviderCredential) -> Option<&ProviderCredential> {
    mode.returns_to_caller().then_some(credential)
}
fn alias(mode: CredentialMode) -> &'static str {
    if mode == CredentialMode::Client {
        "default"
    } else {
        "personal"
    }
}

#[tokio::test]
async fn session_revocation_all_ownerships_confirm_original_cookie_independently() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        for ack in [
            "",
            "HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n",
            "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:9/\r\nContent-Length: 0\r\n\r\n",
        ] {
            let mut f = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            let supplied = caller_from(&source);
            // both may contain an older Cookie from the same login, but must use the current server Cookie.
            let headers = reqwest::header::HeaderMap::from_iter([(
                reqwest::header::SET_COOKIE,
                "sessionid_ss=server-current".parse().unwrap(),
            )]);
            let current = source.with_response_cookies(&headers).unwrap();
            f.put("personal", &current);
            f.put("other", &source);
            let mut responses = replies();
            responses[1] = ack.into();
            let (origin, server) = crate::test_http::serve(responses).await;
            f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
            let value = f
                .provider
                .revoke_session_with_ownership(alias(mode), caller(mode, &supplied), mode)
                .await
                .unwrap();
            assert_eq!(value.state, SessionRevocationState::Invalidated);
            assert_eq!(value.removed, mode.persists_on_server());
            assert_eq!(
                value.caller_credential_discard_required,
                mode.returns_to_caller()
            );
            assert!(value.revocation_request_started);
            assert_eq!(f.stored("personal").is_none(), mode.persists_on_server());
            assert_eq!(
                f.stored("other").unwrap().secret(),
                source.serialize().unwrap()
            );
            assert!(f.provider.take_response_credential().unwrap().is_none());
            let requests = server.await.unwrap();
            assert_eq!(requests.len(), 3);
            assert!(requests[0].starts_with("GET /luna/pc/me?"));
            assert!(requests[1].starts_with("GET /passport/web/logout/?"));
            assert!(requests[2].starts_with("GET /luna/pc/me?"));
            let device = f.provider.client.login_device().unwrap();
            for request in &requests {
                let target = request
                    .lines()
                    .next()
                    .unwrap()
                    .split_whitespace()
                    .nth(1)
                    .unwrap();
                let url = url::Url::parse(&format!("https://api.qishui.com{target}")).unwrap();
                let query = url.query_pairs().collect::<BTreeMap<_, _>>();
                assert_eq!(query["device_id"], device.device_id);
                assert_eq!(query["fp"], device.device_id);
            }
            for request in requests {
                let expected = if mode.persists_on_server() {
                    "server-current"
                } else {
                    "session-secret"
                };
                assert!(request.contains(&format!("cookie: sessionid_ss={expected}\r\n")));
                assert!(!request.contains("ignored-profile-cookie"));
            }
        }
    }
}

#[tokio::test]
async fn session_revocation_absent_invalid_and_wrong_ownership_do_not_dispatch_logout() {
    let mut f = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    let supplied = caller_from(&source);
    let absent = f
        .provider
        .revoke_session_with_ownership("missing", None, CredentialMode::Server)
        .await
        .unwrap();
    assert_eq!(absent.state, SessionRevocationState::NoStoredSession);
    assert!(!absent.revocation_request_started && !absent.removed);
    for (name, credential, mode) in [
        ("personal", Some(&supplied), CredentialMode::Client),
        ("personal", None, CredentialMode::Both),
        ("personal", Some(&supplied), CredentialMode::Server),
    ] {
        assert_eq!(
            f.provider
                .revoke_session_with_ownership(name, credential, mode)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    f.put(
        "personal",
        &test_soda_credential().bind_user("123456").unwrap(),
    );
    assert_eq!(
        f.provider
            .revoke_session_with_ownership("personal", Some(&supplied), CredentialMode::Both)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        f.put("personal", &source);
        let (origin, server) = crate::test_http::serve(vec![invalid()]).await;
        f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
        let value = f
            .provider
            .revoke_session_with_ownership(alias(mode), caller(mode, &supplied), mode)
            .await
            .unwrap();
        assert_eq!(value.state, SessionRevocationState::AlreadyInvalid);
        assert!(!value.revocation_request_started);
        assert_eq!(value.removed, mode.persists_on_server());
        assert_eq!(server.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn session_revocation_cookie_deletion_and_bad_readback_never_prove_success() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        for post in [
            account_reply("123456", Some("sessionid_ss=; Max-Age=0")),
            account_reply("654321", None),
            crate::test_http::json("{}", None),
        ] {
            let mut f = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            let supplied = caller_from(&source);
            f.put("personal", &source);
            f.put("other", &source);
            let mut responses = replies();
            responses[2] = post;
            let (origin, server) = crate::test_http::serve(responses).await;
            f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
            let error = f
                .provider
                .revoke_session_with_ownership(alias(mode), caller(mode, &supplied), mode)
                .await
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::UpstreamError);
            assert_eq!(error.details["upstream_outcome"], "unconfirmed");
            assert_eq!(error.details["removed"], mode.persists_on_server());
            assert_eq!(
                error.details["caller_credential_discard_required"],
                mode.returns_to_caller()
            );
            assert!(!error.retryable);
            assert!(!format!("{error:?}").contains("session-secret"));
            assert_eq!(f.stored("personal").is_none(), mode.persists_on_server());
            assert!(f.stored("other").is_some());
            assert!(f.provider.take_response_credential().unwrap().is_none());
            assert_eq!(server.await.unwrap().len(), 3);
        }
    }
}

#[tokio::test]
async fn session_revocation_late_success_and_error_cannot_clear_a_new_login() {
    for mode in [CredentialMode::Server, CredentialMode::Both] {
        for boundary in 0..3 {
            for action in 0..4 {
                for failed in [false, true] {
                    let mut f = SessionFixture::new();
                    let source = test_soda_credential().bind_user("123456").unwrap();
                    let supplied = caller_from(&source);
                    f.put("personal", &source);
                    f.put("other", &source);
                    let mut responses = replies();
                    responses.truncate(boundary + 1);
                    if failed {
                        responses[boundary] =
                            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".into();
                    }
                    let paused = crate::test_http::serve_paused_at(responses, boundary).await;
                    f.provider.client = f
                        .provider
                        .client
                        .clone()
                        .with_auth_test_origin(paused.origin);
                    let p = f.provider.clone();
                    let task = tokio::spawn(async move {
                        p.revoke_session_with_ownership("personal", caller(mode, &supplied), mode)
                            .await
                    });
                    paused.arrived.await.unwrap();
                    let headers = reqwest::header::HeaderMap::from_iter([(
                        reqwest::header::SET_COOKIE,
                        "sessionid_ss=same-login-rotation".parse().unwrap(),
                    )]);
                    let replacement = if action == 3 {
                        source.with_response_cookies(&headers).unwrap()
                    } else {
                        SodaCredential::test_credential("session-secret")
                            .bind_user(if action == 2 { "654321" } else { "123456" })
                            .unwrap()
                    };
                    if action == 0 {
                        f.store.remove(Platform::Soda, "personal").unwrap();
                    } else {
                        f.put("personal", &replacement);
                    }
                    paused.release.send(()).unwrap();
                    let error = task.await.unwrap().unwrap_err();
                    assert_eq!(
                        error.code,
                        ErrorCode::Conflict,
                        "{mode:?}/{boundary}/{action}/{failed}"
                    );
                    if action == 0 || (action == 3 && boundary > 0) {
                        assert!(f.stored("personal").is_none());
                    } else {
                        assert_eq!(
                            f.stored("personal").unwrap().secret(),
                            replacement.serialize().unwrap()
                        );
                    }
                    assert_eq!(
                        f.stored("other").unwrap().secret(),
                        source.serialize().unwrap()
                    );
                    assert_eq!(error.details["revocation_request_started"], boundary > 0);
                    assert!(!error.retryable);
                    assert_eq!(paused.requests.await.unwrap().len(), boundary + 1);
                }
            }
        }
    }
}

#[tokio::test]
async fn session_revocation_cancel_and_timeout_clear_only_dispatched_original_login() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        for boundary in 0..3 {
            for timeout in [false, true] {
                let mut f = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                let supplied = caller_from(&source);
                f.put("personal", &source);
                f.put("other", &source);
                let mut responses = replies();
                responses.truncate(boundary + 1);
                let paused = crate::test_http::serve_paused_at(responses, boundary).await;
                f.provider.client = f
                    .provider
                    .client
                    .clone()
                    .with_auth_test_origin(paused.origin);
                let p = f.provider.clone();
                *p.response_credential.lock().unwrap() = Some(supplied.clone());
                let budget = std::time::Duration::from_secs(if timeout { 2 } else { 45 });
                let task = tokio::spawn(async move {
                    p.revoke_owned_session(alias(mode), caller(mode, &supplied), mode, budget)
                        .await
                });
                paused.arrived.await.unwrap();
                if timeout {
                    let error = task.await.unwrap().unwrap_err();
                    assert_eq!(error.code, ErrorCode::UpstreamTimeout);
                    assert_eq!(error.details["revocation_request_started"], boundary > 0);
                    assert_eq!(
                        error.details["caller_credential_discard_required"],
                        boundary > 0 && mode.returns_to_caller()
                    );
                    assert!(!error.retryable);
                } else {
                    task.abort();
                    assert!(task.await.unwrap_err().is_cancelled());
                }
                paused.requests.abort();
                let _ = paused.requests.await;
                assert_eq!(
                    f.stored("personal").is_none(),
                    boundary > 0 && mode.persists_on_server()
                );
                assert_eq!(
                    f.stored("other").unwrap().secret(),
                    source.serialize().unwrap()
                );
                assert!(f.provider.take_response_credential().unwrap().is_none());
            }
        }
    }
}

#[tokio::test]
async fn session_revocation_preflight_failure_keeps_source_and_never_sends_logout() {
    for response in [
        account_reply("654321", None),
        crate::test_http::json("{}", None),
        "HTTP/1.1 503 Unavailable\r\nContent-Length: 0\r\n\r\n".into(),
    ] {
        let mut f = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        f.put("personal", &source);
        let path = f.root.join("login-device.json");
        let (origin, server) = crate::test_http::serve(vec![response]).await;
        f.provider.client = SodaClient::new(&SodaConfig {
            device_path: Some(path.clone()),
            ..Default::default()
        })
        .unwrap()
        .with_auth_test_origin(origin);
        let error = f
            .provider
            .revoke_session_with_ownership("personal", None, CredentialMode::Server)
            .await
            .unwrap_err();
        assert_eq!(error.details["upstream_outcome"], "not_attempted");
        assert_eq!(error.details["removed"], false);
        assert_eq!(error.details["revocation_request_started"], false);
        assert_eq!(
            f.stored("personal").unwrap().secret(),
            source.serialize().unwrap()
        );
        assert!(path.exists());
        assert_eq!(server.await.unwrap().len(), 1);
    }
}

struct FailingCleanupStore {
    inner: Arc<tuneweave_core::FileAccountCredentialStore>,
    conflict: bool,
    calls: std::sync::atomic::AtomicUsize,
}
impl AccountCredentialStore for FailingCleanupStore {
    fn load_platform(&self, p: Platform) -> Result<Vec<StoredAccountCredential>> {
        self.inner.load_platform(p)
    }
    fn put(&self, _: &StoredAccountCredential) -> Result<()> {
        panic!("revocation must not save credentials")
    }
    fn remove(&self, _: Platform, _: &str) -> Result<bool> {
        panic!("revocation must use conditional removal")
    }
    fn compare_exchange(
        &self,
        _: &StoredAccountCredential,
        replacement: Option<&StoredAccountCredential>,
    ) -> Result<bool> {
        assert!(replacement.is_none());
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if self.conflict {
            Ok(false)
        } else {
            Err(TuneWeaveError::new(
                ErrorCode::InternalError,
                "storage unavailable",
            ))
        }
    }
}

#[tokio::test]
async fn session_revocation_cleanup_failure_keeps_remote_proof_without_claiming_removal() {
    for conflict in [false, true] {
        let mut f = SessionFixture::new();
        let source = test_soda_credential().bind_user("123456").unwrap();
        f.put("personal", &source);
        let store = Arc::new(FailingCleanupStore {
            inner: f.store.clone(),
            conflict,
            calls: std::sync::atomic::AtomicUsize::new(0),
        });
        f.provider.credential_store = Some(store.clone());
        let (origin, server) = crate::test_http::serve(replies()).await;
        f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
        let error = f
            .provider
            .revoke_session_with_ownership(
                "personal",
                Some(&caller_from(&source)),
                CredentialMode::Both,
            )
            .await
            .unwrap_err();
        assert_eq!(
            error.code,
            if conflict {
                ErrorCode::Conflict
            } else {
                ErrorCode::InternalError
            }
        );
        assert_eq!(error.details["upstream_outcome"], "invalidated");
        assert_eq!(error.details["local_cleanup"], "failed");
        assert_eq!(error.details["removed"], false);
        assert_eq!(error.details["caller_credential_discard_required"], true);
        assert!(!error.retryable);
        assert_eq!(
            store.calls.load(std::sync::atomic::Ordering::SeqCst),
            if conflict { 3 } else { 1 }
        );
        assert_eq!(
            f.stored("personal").unwrap().secret(),
            source.serialize().unwrap()
        );
        assert_eq!(server.await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn session_revocation_two_accounts_finishing_in_reverse_remain_isolated() {
    let f = SessionFixture::new();
    let a = test_soda_credential().bind_user("123456").unwrap();
    let b = SodaCredential::test_credential("second-secret")
        .bind_user("654321")
        .unwrap();
    f.put("first", &a);
    f.put("second", &b);
    f.put("other", &a);
    let first = crate::test_http::serve_paused_at(replies(), 2).await;
    let second = crate::test_http::serve_paused_at(
        vec![
            account_reply("654321", None),
            crate::test_http::json("{}", None),
            invalid(),
        ],
        2,
    )
    .await;
    let mut pa = f.provider.clone();
    pa.client = pa.client.with_auth_test_origin(first.origin);
    let mut pb = f.provider.clone();
    pb.client = pb.client.with_auth_test_origin(second.origin);
    let ta = tokio::spawn(async move {
        pa.revoke_session_with_ownership("first", None, CredentialMode::Server)
            .await
    });
    first.arrived.await.unwrap();
    let tb = tokio::spawn(async move {
        pb.revoke_session_with_ownership("second", None, CredentialMode::Server)
            .await
    });
    second.arrived.await.unwrap();
    second.release.send(()).unwrap();
    assert!(tb.await.unwrap().unwrap().removed);
    assert!(f.stored("first").is_some());
    assert!(f.stored("second").is_none());
    first.release.send(()).unwrap();
    assert!(ta.await.unwrap().unwrap().removed);
    assert!(f.stored("first").is_none());
    assert!(f.stored("other").is_some());
    for request in first.requests.await.unwrap() {
        assert!(request.contains("cookie: sessionid_ss=session-secret\r\n"));
        assert!(!request.contains("second-secret"));
    }
    for request in second.requests.await.unwrap() {
        assert!(request.contains("cookie: sessionid_ss=second-secret\r\n"));
        assert!(!request.contains("session-secret"));
    }
}
