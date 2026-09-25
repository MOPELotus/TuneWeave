use super::*;

fn imported(account: &str) -> CredentialImportRequest {
    CredentialImportRequest {
        account: account.into(),
        credential: ImportedCredential::Cookie {
            value: "sessionid_ss=incoming-secret".into(),
        },
    }
}

#[tokio::test]
async fn auth_generation_repro_late_import_cannot_overwrite_logout_or_relogin() {
    for logout in [true, false] {
        let mut f = SessionFixture::new();
        let old = test_soda_credential().bind_user("123456").unwrap();
        f.put("personal", &old);
        let paused = crate::test_http::serve_paused(account_reply("654321", None)).await;
        f.provider.client = f
            .provider
            .client
            .clone()
            .with_auth_test_origin(paused.origin);
        let p = f.provider.clone();
        let task = tokio::spawn(async move {
            p.import_credential(&imported("personal"), CredentialMode::Both)
                .await
        });
        paused.arrived.await.unwrap();
        let replacement = SodaCredential::test_credential("new-login-secret")
            .bind_user("123456")
            .unwrap();
        if logout {
            f.provider.logout("personal").await.unwrap();
        } else {
            f.put("personal", &replacement);
        }
        paused.release.send(()).unwrap();
        let result = task.await.unwrap();
        assert_eq!(paused.requests.await.unwrap().len(), 1);
        assert_eq!(result.err().map(|e| e.code), Some(ErrorCode::Conflict));
        if logout {
            assert!(f.stored("personal").is_none());
        } else {
            assert_eq!(
                f.stored("personal").unwrap().secret(),
                replacement.serialize().unwrap()
            );
        }
    }
}

#[tokio::test]
async fn auth_generation_repro_first_import_cannot_survive_same_provider_logout() {
    let mut f = SessionFixture::new();
    let paused = crate::test_http::serve_paused(account_reply("123456", None)).await;
    f.provider.client = f
        .provider
        .client
        .clone()
        .with_auth_test_origin(paused.origin);
    let p = f.provider.clone();
    let task = tokio::spawn(async move {
        p.import_credential(&imported("personal"), CredentialMode::Server)
            .await
    });
    paused.arrived.await.unwrap();
    assert!(!f.provider.logout("personal").await.unwrap());
    paused.release.send(()).unwrap();
    let result = task.await.unwrap();
    paused.requests.await.unwrap();
    assert_eq!(result.err().map(|e| e.code), Some(ErrorCode::Conflict));
    assert!(f.stored("personal").is_none());
}

#[tokio::test]
async fn auth_generation_repro_late_session_errors_observe_new_login() {
    for refresh in [false, true] {
        let mut f = SessionFixture::new();
        f.put(
            "personal",
            &test_soda_credential().bind_user("123456").unwrap(),
        );
        let paused = crate::test_http::serve_paused(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".into(),
        )
        .await;
        f.provider.client = f
            .provider
            .client
            .clone()
            .with_auth_test_origin(paused.origin);
        let p = f.provider.clone();
        let task = tokio::spawn(async move {
            if refresh {
                p.refresh_session("personal").await.map(|_| ())
            } else {
                p.session_profile("personal").await.map(|_| ())
            }
        });
        paused.arrived.await.unwrap();
        let replacement = SodaCredential::test_credential("replacement-secret")
            .bind_user("654321")
            .unwrap();
        f.put("personal", &replacement);
        paused.release.send(()).unwrap();
        let result = task.await.unwrap();
        paused.requests.await.unwrap();
        assert_eq!(result.err().map(|e| e.code), Some(ErrorCode::Conflict));
        assert_eq!(
            f.stored("personal").unwrap().secret(),
            replacement.serialize().unwrap()
        );
    }
}

#[tokio::test]
async fn auth_generation_repro_qr_identity_confirmation_cannot_revive_logged_out_alias() {
    let mut f = SessionFixture::new();
    f.put(
        "personal",
        &test_soda_credential().bind_user("123456").unwrap(),
    );
    let paused=crate::test_http::serve_paused_at(vec![
        crate::test_http::json(r#"{"message":"success","data":{"token":"01234567890123456789012345678901234","qrcode":"iVBORw0KGgo="}}"#,None),
        crate::test_http::json(r#"{"data":{"status":"confirmed","error_code":0}}"#,Some("sessionid_ss=qr-secret")),
        account_reply("654321",None),
    ],2).await;
    f.provider.client = f
        .provider
        .client
        .clone()
        .with_auth_test_origin(paused.origin);
    let start = f
        .provider
        .start_qr_login_with_mode(None, CredentialMode::Both)
        .await
        .unwrap();
    let p = f.provider.clone();
    let task = tokio::spawn(async move {
        p.poll_qr_login_with_mode(
            &start.provider_transaction_id,
            "personal",
            CredentialMode::Both,
        )
        .await
    });
    paused.arrived.await.unwrap();
    f.provider.logout("personal").await.unwrap();
    paused.release.send(()).unwrap();
    let result = task.await.unwrap();
    paused.requests.await.unwrap();
    assert_eq!(result.err().map(|e| e.code), Some(ErrorCode::Conflict));
    assert!(f.stored("personal").is_none());
}

fn qr_create() -> String {
    crate::test_http::json(
        r#"{"message":"success","data":{"token":"01234567890123456789012345678901234","qrcode":"iVBORw0KGgo="}}"#,
        None,
    )
}
fn qr_confirm() -> String {
    crate::test_http::json(
        r#"{"data":{"status":"confirmed","error_code":0}}"#,
        Some("sessionid_ss=qr-secret"),
    )
}

#[tokio::test]
async fn auth_generation_independent_providers_first_and_replacement_imports_use_cas() {
    for existing in [false, true] {
        for denied in [false, true] {
            let mut f = SessionFixture::new();
            if existing {
                f.put(
                    "personal",
                    &test_soda_credential().bind_user("123456").unwrap(),
                );
            }
            let reply = if denied {
                crate::test_http::json(r#"{"status_code":1000016}"#, None)
            } else {
                account_reply("123456", None)
            };
            let slow = crate::test_http::serve_paused(reply).await;
            f.provider.client = f.provider.client.clone().with_auth_test_origin(slow.origin);
            let p = f.provider.clone();
            let pending = tokio::spawn(async move {
                p.import_credential(&imported("personal"), CredentialMode::Both)
                    .await
            });
            slow.arrived.await.unwrap();
            let (origin, fast) = crate::test_http::serve(vec![account_reply("123456", None)]).await;
            // Independently constructed provider, sharing only the file store.
            let mut other = SodaProvider::new(SodaConfig {
                credential_store: Some(f.store.clone()),
                ..SodaConfig::default()
            })
            .unwrap();
            other.client = other.client.clone().with_auth_test_origin(origin);
            let winner = other
                .import_credential(&imported("personal"), CredentialMode::Both)
                .await
                .unwrap();
            fast.await.unwrap();
            slow.release.send(()).unwrap();
            assert_eq!(
                pending.await.unwrap().unwrap_err().code,
                ErrorCode::Conflict
            );
            slow.requests.await.unwrap();
            assert_eq!(
                f.stored("personal").unwrap().secret(),
                winner.credential.unwrap().secret()
            );
            assert!(f.provider.take_response_credential().unwrap().is_none());
        }
    }
}

#[tokio::test]
async fn auth_generation_imports_keep_unrelated_aliases_independent() {
    let mut f = SessionFixture::new();
    let slow = crate::test_http::serve_paused(account_reply("123456", None)).await;
    f.provider.client = f.provider.client.clone().with_auth_test_origin(slow.origin);
    let p = f.provider.clone();
    let pending = tokio::spawn(async move {
        p.import_credential(&imported("first"), CredentialMode::Both)
            .await
    });
    slow.arrived.await.unwrap();
    let (origin, fast) = crate::test_http::serve(vec![account_reply("654321", None)]).await;
    let mut other = f.provider.clone();
    other.client = other.client.clone().with_auth_test_origin(origin);
    other
        .import_credential(&imported("second"), CredentialMode::Server)
        .await
        .unwrap();
    fast.await.unwrap();
    f.provider.logout("third").await.unwrap();
    slow.release.send(()).unwrap();
    assert!(pending.await.unwrap().unwrap().credential.is_some());
    slow.requests.await.unwrap();
    assert!(f.stored("first").is_some() && f.stored("second").is_some());
}

#[tokio::test]
async fn auth_generation_absent_revocation_cancels_first_import() {
    let mut f = SessionFixture::new();
    let paused = crate::test_http::serve_paused(account_reply("123456", None)).await;
    f.provider.client = f
        .provider
        .client
        .clone()
        .with_auth_test_origin(paused.origin);
    let p = f.provider.clone();
    let pending = tokio::spawn(async move {
        p.import_credential(&imported("personal"), CredentialMode::Both)
            .await
    });
    paused.arrived.await.unwrap();
    let result = f
        .provider
        .revoke_session_with_ownership("personal", None, CredentialMode::Server)
        .await
        .unwrap();
    assert_eq!(
        result.state,
        tuneweave_core::SessionRevocationState::NoStoredSession
    );
    paused.release.send(()).unwrap();
    assert_eq!(
        pending.await.unwrap().unwrap_err().code,
        ErrorCode::Conflict
    );
    paused.requests.await.unwrap();
    assert!(f.stored("personal").is_none());
}

struct UnreadableStore;
impl AccountCredentialStore for UnreadableStore {
    fn load_platform(&self, _: Platform) -> Result<Vec<StoredAccountCredential>> {
        panic!("client auth read server store")
    }
    fn put(&self, _: &StoredAccountCredential) -> Result<()> {
        panic!("client auth wrote server store")
    }
    fn remove(&self, _: Platform, _: &str) -> Result<bool> {
        panic!("client auth removed server store")
    }
}

#[tokio::test]
async fn auth_generation_client_import_and_qr_never_access_server_accounts() {
    let mut p = SodaProvider::new(SodaConfig {
        credential_store: Some(Arc::new(UnreadableStore)),
        ..SodaConfig::default()
    })
    .unwrap();
    let (origin, server) = crate::test_http::serve(vec![
        account_reply("123456", None),
        qr_create(),
        qr_confirm(),
        account_reply("654321", None),
    ])
    .await;
    p.client = p.client.clone().with_auth_test_origin(origin);
    let imported = p
        .import_credential(&imported("default"), CredentialMode::Client)
        .await
        .unwrap();
    let start = p
        .start_qr_login_for_account(None, "default", CredentialMode::Client)
        .await
        .unwrap();
    p.logout_with_ownership(
        "default",
        imported.credential.as_ref(),
        CredentialMode::Client,
    )
    .await
    .unwrap();
    let result = p
        .poll_qr_login_with_mode(
            &start.provider_transaction_id,
            "default",
            CredentialMode::Client,
        )
        .await
        .unwrap();
    assert_eq!(result.state, AuthState::Confirmed);
    assert_eq!(result.profile.unwrap().user_id.as_deref(), Some("654321"));
    assert!(result.credential.is_some());
    assert_eq!(server.await.unwrap().len(), 4);
}

#[tokio::test]
async fn auth_generation_qr_creation_is_cancelled_before_transaction_is_exposed() {
    for denied in [false, true] {
        let mut f = SessionFixture::new();
        let response = if denied {
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".into()
        } else {
            qr_create()
        };
        let paused = crate::test_http::serve_paused(response).await;
        f.provider.client = f
            .provider
            .client
            .clone()
            .with_auth_test_origin(paused.origin);
        let p = f.provider.clone();
        let pending = tokio::spawn(async move {
            p.start_qr_login_for_account(None, "personal", CredentialMode::Both)
                .await
        });
        paused.arrived.await.unwrap();
        f.provider.logout("personal").await.unwrap();
        paused.release.send(()).unwrap();
        assert_eq!(
            pending.await.unwrap().unwrap_err().code,
            ErrorCode::Conflict
        );
        paused.requests.await.unwrap();
        assert!(f.stored("personal").is_none());
    }
}

#[tokio::test]
async fn auth_generation_bound_qr_keeps_alias_while_legacy_qr_rejects_changed_snapshot() {
    for bound in [false, true] {
        let mut f = SessionFixture::new();
        let responses = if bound {
            vec![qr_create(), qr_confirm(), account_reply("123456", None)]
        } else {
            vec![qr_create()]
        };
        let (origin, server) = crate::test_http::serve(responses).await;
        f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
        let start = if bound {
            f.provider
                .start_qr_login_for_account(None, "personal", CredentialMode::Both)
                .await
        } else {
            f.provider
                .start_qr_login_with_mode(None, CredentialMode::Both)
                .await
        }
        .unwrap();
        f.put(
            "other",
            &test_soda_credential().bind_user("654321").unwrap(),
        );
        if bound {
            assert_eq!(
                f.provider
                    .poll_qr_login_with_mode(
                        &start.provider_transaction_id,
                        "wrong",
                        CredentialMode::Both
                    )
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidRequest
            );
        }
        let result = f
            .provider
            .poll_qr_login_with_mode(
                &start.provider_transaction_id,
                "personal",
                CredentialMode::Both,
            )
            .await;
        if bound {
            assert_eq!(result.unwrap().state, AuthState::Confirmed);
        } else {
            assert_eq!(result.unwrap_err().code, ErrorCode::Conflict);
        }
        assert!(f.stored("other").is_some());
        assert_eq!(f.stored("personal").is_some(), bound);
        server.await.unwrap();
    }
}

#[tokio::test]
async fn auth_generation_qr_created_before_relogin_cannot_bind_the_new_session() {
    for bound in [false, true] {
        let mut f = SessionFixture::new();
        f.put(
            "personal",
            &test_soda_credential().bind_user("123456").unwrap(),
        );
        let (origin, server) = crate::test_http::serve(vec![qr_create()]).await;
        f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
        let start = if bound {
            f.provider
                .start_qr_login_for_account(None, "personal", CredentialMode::Both)
                .await
        } else {
            f.provider
                .start_qr_login_with_mode(None, CredentialMode::Both)
                .await
        }
        .unwrap();
        // Same UID and Cookie, but a distinct login generation.
        let replacement = test_soda_credential().bind_user("123456").unwrap();
        f.put("personal", &replacement);
        assert_eq!(
            f.provider
                .poll_qr_login_with_mode(
                    &start.provider_transaction_id,
                    "personal",
                    CredentialMode::Both
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            f.stored("personal").unwrap().secret(),
            replacement.serialize().unwrap()
        );
        assert_eq!(server.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn auth_generation_qr_poll_and_each_mfa_write_reject_logout_during_await() {
    for boundary in [1, 2, 3] {
        for denied in [false, true] {
            let mut f = SessionFixture::new();
            let mfa = String::from_utf8(crate::mfa::tests::sms_fixture()).unwrap();
            let mut replies = if boundary == 1 {
                vec![qr_create(), qr_confirm()]
            } else {
                vec![
                    qr_create(),
                    crate::test_http::json(&mfa, Some("passport_mfa_token=mfa-secret")),
                    crate::test_http::json(
                        r#"{"message":"success","data":{"retry_time":60}}"#,
                        None,
                    ),
                    crate::test_http::json(
                        r#"{"message":"success","data":{"ticket":"ticket-secret"}}"#,
                        None,
                    ),
                ]
            };
            replies.truncate(boundary + 1);
            if denied {
                replies[boundary] = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".into();
            }
            let paused = crate::test_http::serve_paused_at(replies, boundary).await;
            f.provider.client = f
                .provider
                .client
                .clone()
                .with_auth_test_origin(paused.origin);
            let start = f
                .provider
                .start_qr_login_for_account(None, "personal", CredentialMode::Both)
                .await
                .unwrap();
            let id = start.provider_transaction_id;
            if boundary > 1 {
                assert_eq!(
                    f.provider
                        .poll_qr_login_with_mode(&id, "personal", CredentialMode::Both)
                        .await
                        .unwrap()
                        .state,
                    AuthState::VerificationRequired
                );
            }
            if boundary > 2 {
                f.provider
                    .verify_qr_login(
                        &id,
                        "personal",
                        CredentialMode::Both,
                        &tuneweave_core::QrVerificationAction::SendSms,
                    )
                    .await
                    .unwrap();
            }
            let p = f.provider.clone();
            let pending = tokio::spawn(async move {
                match boundary {
                    1 => {
                        p.poll_qr_login_with_mode(&id, "personal", CredentialMode::Both)
                            .await
                    }
                    2 => {
                        p.verify_qr_login(
                            &id,
                            "personal",
                            CredentialMode::Both,
                            &tuneweave_core::QrVerificationAction::SendSms,
                        )
                        .await
                    }
                    _ => {
                        p.verify_qr_login(
                            &id,
                            "personal",
                            CredentialMode::Both,
                            &tuneweave_core::QrVerificationAction::SubmitSms {
                                code: "864209".into(),
                            },
                        )
                        .await
                    }
                }
            });
            paused.arrived.await.unwrap();
            f.provider.logout("personal").await.unwrap();
            paused.release.send(()).unwrap();
            assert_eq!(
                pending.await.unwrap().unwrap_err().code,
                ErrorCode::Conflict
            );
            assert_eq!(paused.requests.await.unwrap().len(), boundary + 1);
            assert!(f.stored("personal").is_none());
            assert!(f.provider.take_response_credential().unwrap().is_none());
        }
    }
}

#[tokio::test]
async fn auth_generation_cancellation_consumes_qr_and_never_saves_import() {
    for qr in [false, true] {
        let mut f = SessionFixture::new();
        let replies = if qr {
            vec![qr_create(), qr_confirm()]
        } else {
            vec![account_reply("123456", None)]
        };
        let paused = crate::test_http::serve_paused_at(replies, usize::from(qr)).await;
        f.provider.client = f
            .provider
            .client
            .clone()
            .with_auth_test_origin(paused.origin);
        let id = if qr {
            Some(
                f.provider
                    .start_qr_login_for_account(None, "personal", CredentialMode::Both)
                    .await
                    .unwrap()
                    .provider_transaction_id,
            )
        } else {
            None
        };
        let task_id = id.clone();
        let p = f.provider.clone();
        let pending = tokio::spawn(async move {
            if let Some(id) = task_id {
                p.poll_qr_login_with_mode(&id, "personal", CredentialMode::Both)
                    .await
                    .map(|_| ())
            } else {
                p.import_credential(&imported("personal"), CredentialMode::Both)
                    .await
                    .map(|_| ())
            }
        });
        paused.arrived.await.unwrap();
        pending.abort();
        assert!(pending.await.unwrap_err().is_cancelled());
        paused.requests.abort();
        assert!(paused.requests.await.unwrap_err().is_cancelled());
        if let Some(id) = id {
            assert_eq!(
                f.provider
                    .poll_qr_login_with_mode(&id, "personal", CredentialMode::Both)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidRequest
            );
        }
        assert!(f.stored("personal").is_none());
        assert!(f.provider.take_response_credential().unwrap().is_none());
        // Every reservation is released; this still admits all 128 slots.
        let leases: Vec<_> = (0..128)
            .map(|_| {
                f.provider
                    .authentication_lease(
                        Some("personal"),
                        CredentialMode::Both,
                        std::time::Instant::now() + std::time::Duration::from_secs(30),
                    )
                    .unwrap()
            })
            .collect();
        assert_eq!(leases.len(), 128);
    }
}

#[tokio::test]
async fn auth_generation_capacity_and_deadline_apply_before_network_and_publication() {
    let f = SessionFixture::new();
    let leases: Vec<_> = (0..128)
        .map(|_| {
            f.provider
                .authentication_lease(
                    Some("personal"),
                    CredentialMode::Both,
                    std::time::Instant::now() + std::time::Duration::from_secs(30),
                )
                .unwrap()
        })
        .collect();
    assert_eq!(
        f.provider
            .import_credential(&imported("personal"), CredentialMode::Both)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    drop(leases);
    let mut f = f;
    let paused = crate::test_http::serve_paused(account_reply("123456", None)).await;
    f.provider.client = f
        .provider
        .client
        .clone()
        .with_auth_test_origin(paused.origin);
    let p = f.provider.clone();
    let pending = tokio::spawn(async move {
        p.finish_authentication(
            "personal",
            &test_soda_credential(),
            CredentialMode::Both,
            Some(std::time::Instant::now() + std::time::Duration::from_secs(2)),
        )
        .await
    });
    paused.arrived.await.unwrap();
    assert_eq!(
        pending.await.unwrap().unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    paused.requests.abort();
    assert!(paused.requests.await.unwrap_err().is_cancelled());
    assert!(f.stored("personal").is_none());
}

#[tokio::test]
async fn auth_generation_qr_deadline_survives_network_wait_and_returns_expired() {
    let mut f = SessionFixture::new();
    let paused = crate::test_http::serve_paused_at(vec![qr_create(), qr_confirm()], 1).await;
    f.provider.client = f
        .provider
        .client
        .clone()
        .with_auth_test_origin(paused.origin);
    let start = f
        .provider
        .start_qr_login_for_account(None, "personal", CredentialMode::Both)
        .await
        .unwrap();
    let id = start.provider_transaction_id;
    {
        let mut access = f
            .provider
            .qr_transactions
            .access(&id, "personal", CredentialMode::Both)
            .await
            .unwrap();
        access
            .authentication
            .as_mut()
            .unwrap()
            .shorten_deadline(std::time::Instant::now() + std::time::Duration::from_secs(2))
            .unwrap();
    }
    let p = f.provider.clone();
    let task_id = id.clone();
    let pending = tokio::spawn(async move {
        p.poll_qr_login_with_mode(&task_id, "personal", CredentialMode::Both)
            .await
    });
    paused.arrived.await.unwrap();
    let result = pending.await.unwrap().unwrap();
    assert_eq!(result.state, AuthState::Expired);
    assert!(result.profile.is_none() && result.credential.is_none());
    paused.requests.abort();
    assert!(paused.requests.await.unwrap_err().is_cancelled());
    assert!(f.stored("personal").is_none());
    assert!(f.provider.qr_transactions.expires_at(&id).is_err());
}

#[tokio::test]
async fn auth_generation_session_errors_preserve_new_server_and_caller_generations() {
    for caller_owned in [false, true] {
        for denied in [false, true] {
            let mut f = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            f.put("personal", &source);
            let reply = if denied {
                crate::test_http::json(r#"{"status_code":1000016}"#, Some("sessionid_ss=poison"))
            } else {
                crate::test_http::json("{", None)
            };
            let paused = crate::test_http::serve_paused(reply).await;
            f.provider.client = f
                .provider
                .client
                .clone()
                .with_auth_test_origin(paused.origin);
            let p = if caller_owned {
                f.provider
                    .caller_credential_scope(&caller_from(&source))
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let changed_scope = p.clone();
            let pending = tokio::spawn(async move {
                p.session_profile(if caller_owned { "default" } else { "personal" })
                    .await
            });
            paused.arrived.await.unwrap();
            let replacement = test_soda_credential().bind_user("123456").unwrap();
            if caller_owned {
                *changed_scope
                    .caller_credential
                    .as_ref()
                    .unwrap()
                    .lock()
                    .unwrap() = replacement.clone();
            } else {
                f.put("personal", &replacement);
            }
            paused.release.send(()).unwrap();
            assert_eq!(
                pending.await.unwrap().unwrap_err().code,
                ErrorCode::Conflict
            );
            paused.requests.await.unwrap();
            assert!(changed_scope.take_response_credential().unwrap().is_none());
            if caller_owned {
                assert_eq!(
                    *changed_scope
                        .caller_credential
                        .as_ref()
                        .unwrap()
                        .lock()
                        .unwrap(),
                    replacement
                );
            } else {
                assert_eq!(
                    f.stored("personal").unwrap().secret(),
                    replacement.serialize().unwrap()
                );
            }
        }
    }
}

#[tokio::test]
async fn auth_generation_expired_qr_stays_expired_after_capacity_pruning() {
    let mut f = SessionFixture::new();
    let (origin, server) = crate::test_http::serve(vec![qr_create()]).await;
    f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
    let start = f
        .provider
        .start_qr_login_for_account(None, "personal", CredentialMode::Both)
        .await
        .unwrap();
    let id = start.provider_transaction_id;
    {
        let mut access = f
            .provider
            .qr_transactions
            .access(&id, "personal", CredentialMode::Both)
            .await
            .unwrap();
        assert_eq!(
            access
                .authentication
                .as_mut()
                .unwrap()
                .shorten_deadline(std::time::Instant::now())
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let _next = f
        .provider
        .authentication_lease(
            Some("other"),
            CredentialMode::Both,
            std::time::Instant::now() + std::time::Duration::from_secs(30),
        )
        .unwrap();
    let result = f
        .provider
        .poll_qr_login_with_mode(&id, "personal", CredentialMode::Both)
        .await
        .unwrap();
    assert_eq!(result.state, AuthState::Expired);
    assert!(result.profile.is_none() && result.credential.is_none());
    assert!(f.stored("personal").is_none());
    assert_eq!(server.await.unwrap().len(), 1);
}

#[tokio::test]
async fn auth_generation_server_logout_discards_only_matching_idle_qr_challenges() {
    let mut f = SessionFixture::new();
    let personal = test_soda_credential().bind_user("123456").unwrap();
    let other = SodaCredential::test_credential("other-secret")
        .bind_user("654321")
        .unwrap();
    f.put("personal", &personal);
    f.put("other", &other);
    let (origin, server) =
        crate::test_http::serve(vec![qr_create(), qr_create(), qr_create(), qr_create()]).await;
    f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);

    let personal_qr = f
        .provider
        .start_qr_login_for_account(None, "personal", CredentialMode::Server)
        .await
        .unwrap()
        .provider_transaction_id;
    let other_qr = f
        .provider
        .start_qr_login_for_account(None, "other", CredentialMode::Server)
        .await
        .unwrap()
        .provider_transaction_id;
    let legacy_qr = f
        .provider
        .start_qr_login_with_mode(None, CredentialMode::Server)
        .await
        .unwrap()
        .provider_transaction_id;
    let client_qr = f
        .provider
        .start_qr_login_for_account(None, "default", CredentialMode::Client)
        .await
        .unwrap()
        .provider_transaction_id;
    assert_eq!(server.await.unwrap().len(), 4);

    assert!(f.provider.logout("personal").await.unwrap());
    assert!(
        f.provider
            .qr_transactions
            .access(&personal_qr, "personal", CredentialMode::Server)
            .await
            .is_err()
    );
    assert!(
        f.provider
            .qr_transactions
            .access(&legacy_qr, "personal", CredentialMode::Server)
            .await
            .is_err()
    );
    let other_access = f
        .provider
        .qr_transactions
        .access(&other_qr, "other", CredentialMode::Server)
        .await
        .unwrap();
    drop(other_access);
    let client_access = f
        .provider
        .qr_transactions
        .access(&client_qr, "default", CredentialMode::Client)
        .await
        .unwrap();
    drop(client_access);
    assert!(f.stored("personal").is_none());
    assert!(f.stored("other").is_some());
}

#[tokio::test]
async fn auth_generation_inflight_qr_logout_cancels_lease_before_readback_can_publish() {
    let mut f = SessionFixture::new();
    let personal = test_soda_credential().bind_user("123456").unwrap();
    let other = SodaCredential::test_credential("other-secret")
        .bind_user("654321")
        .unwrap();
    f.put("personal", &personal);
    f.put("other", &other);
    let paused = crate::test_http::serve_paused_at(vec![qr_create(), qr_confirm()], 1).await;
    f.provider.client = f
        .provider
        .client
        .clone()
        .with_auth_test_origin(paused.origin);
    let id = f
        .provider
        .start_qr_login_for_account(None, "personal", CredentialMode::Server)
        .await
        .unwrap()
        .provider_transaction_id;
    let p = f.provider.clone();
    let poll_id = id.clone();
    let poll = tokio::spawn(async move {
        p.poll_qr_login_with_mode(&poll_id, "personal", CredentialMode::Server)
            .await
    });
    paused.arrived.await.unwrap();

    assert!(f.provider.logout("personal").await.unwrap());
    assert!(f.stored("personal").is_none());
    assert!(f.stored("other").is_some());
    paused.release.send(()).unwrap();
    assert_eq!(poll.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(paused.requests.await.unwrap().len(), 2);
    assert!(
        f.provider
            .qr_transactions
            .access(&id, "personal", CredentialMode::Server)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn auth_generation_session_revocation_discards_matching_idle_qr_challenge() {
    let mut f = SessionFixture::new();
    let personal = test_soda_credential().bind_user("123456").unwrap();
    let other = SodaCredential::test_credential("other-secret")
        .bind_user("654321")
        .unwrap();
    f.put("personal", &personal);
    f.put("other", &other);
    let (origin, server) = crate::test_http::serve(vec![
        qr_create(),
        account_reply("123456", None),
        crate::test_http::json(r#"{"message":"success"}"#, None),
        crate::test_http::json(r#"{"status_code":1000016}"#, None),
    ])
    .await;
    f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
    let id = f
        .provider
        .start_qr_login_for_account(None, "personal", CredentialMode::Server)
        .await
        .unwrap()
        .provider_transaction_id;

    let revoked = f
        .provider
        .revoke_session_with_ownership("personal", None, CredentialMode::Server)
        .await
        .unwrap();
    assert_eq!(
        revoked.state,
        tuneweave_core::SessionRevocationState::Invalidated
    );
    assert!(revoked.removed);
    assert!(f.stored("personal").is_none());
    assert!(f.stored("other").is_some());
    assert!(
        f.provider
            .qr_transactions
            .access(&id, "personal", CredentialMode::Server)
            .await
            .is_err()
    );
    assert_eq!(server.await.unwrap().len(), 4);
}
