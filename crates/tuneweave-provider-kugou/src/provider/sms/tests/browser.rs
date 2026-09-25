use super::*;
use tuneweave_core::AuthBrowserChallenge;

const PROOF: &str = "synthetic+browser-proof%2F=";
fn required(event: &str) -> String {
    raw(
        json!({"status":0,"error_code":20028,"data":format!("eventid={event}&unused=private-upstream-data")}),
    )
}
fn browser_action(prompt: &AuthBrowserChallenge) -> AuthChallengeAction {
    AuthChallengeAction::SubmitBrowser {
        verification_id: prompt.verification_id.clone(), code: CODE.into(),
        response: json!({"status":1,"vType":2,"error_code":0,"error_msg":"private-message","verify_data":"synthetic%2Bbrowser-proof%252F%3D"}).to_string(),
    }
}
fn prompt(progress: AuthChallengeProgress) -> AuthBrowserChallenge {
    let AuthChallengeProgress::Pending(AuthChallengeStatus::BrowserVerificationRequired {
        verification,
    }) = progress
    else {
        panic!("browser challenge expected")
    };
    verification
}
async fn begin_browser(
    provider: &KugouProvider,
    account: &str,
    mode: CredentialMode,
) -> (ProviderAuthChallenge, AuthBrowserChallenge) {
    let r = provider
        .begin_auth_challenge(&input(account), mode)
        .await
        .unwrap();
    let p = prompt(
        provider
            .advance_auth_challenge(&r, &submit())
            .await
            .unwrap(),
    );
    (r, p)
}

#[tokio::test]
async fn sms_browser_callback_is_header_only_and_all_ownership_modes_require_fresh_uid_exchange() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let mut f = server(vec![
            reply(json!("sent")).into(),
            required("original-event").into(),
            cookie("111").into(),
            exchange("111").into(),
        ])
        .await;
        let store = Arc::new(Store::default());
        f.provider.credential_store = Some(if mode == CredentialMode::Client {
            Arc::new(NoStore)
        } else {
            store.clone()
        });
        let alias = if mode == CredentialMode::Client {
            "default"
        } else {
            "A"
        };
        let (r, p) = begin_browser(&f.provider, alias, mode).await;
        assert_eq!(p.remaining_attempts, 4);
        assert!(store.values.lock().unwrap().is_empty());
        assert!(!format!("{p:?}").contains("original-event"));
        assert_eq!(
            f.provider.auth_challenge_status(&r).await.unwrap(),
            AuthChallengeStatus::BrowserVerificationRequired {
                verification: p.clone()
            }
        );
        for action in [
            submit(),
            select("111"),
            AuthChallengeAction::SubmitBrowser {
                verification_id: "other-receipt".into(),
                code: CODE.into(),
                response: "{}".into(),
            },
        ] {
            assert_eq!(
                f.provider
                    .advance_auth_challenge(&r, &action)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidRequest
            );
        }
        let AuthChallengeProgress::Confirmed(result) = f
            .provider
            .advance_auth_challenge(&r, &browser_action(&p))
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(result.profile.user_id.as_deref(), Some("111"));
        assert_eq!(result.credential.is_some(), mode.returns_to_caller());
        assert_eq!(
            store.values.lock().unwrap().contains_key(alias),
            mode.persists_on_server()
        );
        assert!(f.provider.auth_challenge_status(&r).await.is_err());
        let wires = f.requests.await.unwrap();
        let (initial, _) = check_wire(&wires[1], "/v2/loginbyverifycode/");
        let (continued, body) = check_wire(&wires[2], "/v2/loginbyverifycode/");
        assert_eq!(initial["mid"], continued["mid"]);
        assert_eq!(initial["dfid"], continued["dfid"]);
        assert!(p.url.contains(&format!("/1014/null/{}/", initial["mid"])));
        assert_eq!(body["force_login"], 0);
        assert_eq!(body["userid"], "");
        assert!(wires[2].contains(&format!("verifydata: {PROOF}\r\n")));
        assert!(
            !wires[2]
                .split_once("\r\n\r\n")
                .unwrap()
                .1
                .contains("browser-proof")
        );
        assert!(!wires[2].lines().next().unwrap().contains("browser-proof"));
        for i in [0, 1, 3] {
            assert!(!wires[i].to_lowercase().contains("verifydata:"));
        }
    }
}

#[tokio::test]
async fn sms_browser_preserves_selected_account_and_explicit_registration_branch() {
    for selection in [false, true] {
        let mut frames = vec![reply(json!("sent")).into()];
        if selection {
            frames.extend([choice_required().into(), choices().into()]);
        } else {
            frames.push(raw(json!({"status":0,"error_code":30703})).into());
        }
        let uid = if selection { "222" } else { "111" };
        frames.extend([
            required("branch-event").into(),
            cookie(uid).into(),
            exchange(uid).into(),
        ]);
        let f = server(frames).await;
        let mut request = input("default");
        request.allow_account_creation = !selection;
        let r = f
            .provider
            .begin_auth_challenge(&request, CredentialMode::Client)
            .await
            .unwrap();
        let initial = f
            .provider
            .advance_auth_challenge(&r, &submit())
            .await
            .unwrap();
        let p = if selection {
            assert!(matches!(
                initial,
                AuthChallengeProgress::Pending(
                    AuthChallengeStatus::AccountSelectionRequired { .. }
                )
            ));
            prompt(
                f.provider
                    .advance_auth_challenge(&r, &select(uid))
                    .await
                    .unwrap(),
            )
        } else {
            prompt(initial)
        };
        assert!(matches!(
            f.provider
                .advance_auth_challenge(&r, &browser_action(&p))
                .await
                .unwrap(),
            AuthChallengeProgress::Confirmed(_)
        ));
        let wires = f.requests.await.unwrap();
        let (_, b) = check_wire(&wires[wires.len() - 2], "/v2/loginbyverifycode/");
        assert_eq!(b["force_login"], u8::from(!selection));
        assert_eq!(b["userid"], if selection { json!(222) } else { json!("") });
    }
}

#[tokio::test]
async fn sms_browser_then_registration_or_selection_never_reuses_proof_on_followup_requests() {
    for registration in [false, true] {
        let mut frames = vec![
            reply(json!("sent")).into(),
            required("before-branch").into(),
        ];
        if registration {
            frames.extend([
                raw(json!({"status":0,"error_code":30703})).into(),
                cookie("111").into(),
                exchange("111").into(),
            ]);
        } else {
            frames.extend([
                choice_required().into(),
                choices().into(),
                cookie("222").into(),
                exchange("222").into(),
            ]);
        }
        let f = server(frames).await;
        let mut request = input("default");
        request.allow_account_creation = registration;
        let r = f
            .provider
            .begin_auth_challenge(&request, CredentialMode::Client)
            .await
            .unwrap();
        let p = prompt(
            f.provider
                .advance_auth_challenge(&r, &submit())
                .await
                .unwrap(),
        );
        let result = f
            .provider
            .advance_auth_challenge(&r, &browser_action(&p))
            .await
            .unwrap();
        if registration {
            assert!(matches!(result, AuthChallengeProgress::Confirmed(_)));
        } else {
            assert!(matches!(
                result,
                AuthChallengeProgress::Pending(
                    AuthChallengeStatus::AccountSelectionRequired { .. }
                )
            ));
            assert!(matches!(
                f.provider
                    .advance_auth_challenge(&r, &select("222"))
                    .await
                    .unwrap(),
                AuthChallengeProgress::Confirmed(_)
            ));
        }
        let wires = f.requests.await.unwrap();
        assert!(wires[2].contains("verifydata:"));
        assert!(
            wires
                .iter()
                .enumerate()
                .all(|(i, w)| i == 2 || !w.contains("verifydata:"))
        );
    }
}

#[tokio::test]
async fn sms_browser_invalid_callbacks_keep_pending_but_new_challenges_rotate_binding_and_share_attempts()
 {
    let mut frames = vec![reply(json!("sent")).into()];
    frames.extend((0..5).map(|_| required("same-event").into()));
    let f = server(frames).await;
    let (r, mut p) = begin_browser(&f.provider, "default", CredentialMode::Client).await;
    for bad in [
        "{}".into(),
        json!({"status":0,"vType":2,"error_code":0,"verify_data":"x"}).to_string(),
        json!({"status":1,"vType":2,"error_code":0,"verify_data":"%0d%0aevil"}).to_string(),
    ] {
        let action = AuthChallengeAction::SubmitBrowser {
            verification_id: p.verification_id.clone(),
            code: CODE.into(),
            response: bad,
        };
        let e = f
            .provider
            .advance_auth_challenge(&r, &action)
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidRequest);
        assert!(!e.auth_challenge_consumed());
        assert_eq!(
            f.provider.auth_challenge_status(&r).await.unwrap(),
            AuthChallengeStatus::BrowserVerificationRequired {
                verification: p.clone()
            }
        );
    }
    for remaining in (1..=3).rev() {
        let old = browser_action(&p);
        let next = prompt(f.provider.advance_auth_challenge(&r, &old).await.unwrap());
        assert_ne!(p.verification_id, next.verification_id);
        assert_eq!(next.remaining_attempts, remaining);
        assert_eq!(
            f.provider
                .advance_auth_challenge(&r, &old)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        p = next;
    }
    assert!(
        f.provider
            .advance_auth_challenge(&r, &browser_action(&p))
            .await
            .unwrap_err()
            .auth_challenge_consumed()
    );
    assert!(f.provider.auth_challenge_status(&r).await.is_err());
    assert_eq!(f.requests.await.unwrap().len(), 6);
}

#[tokio::test]
async fn sms_browser_wrong_code_discards_callback_and_returns_to_original_selection() {
    for selected in [false, true] {
        let mut frames = vec![reply(json!("sent")).into()];
        if selected {
            frames.extend([choice_required().into(), choices().into()]);
        }
        frames.extend([
            required("wrong-code-event").into(),
            raw(json!({"status":0,"error_code":20021})).into(),
            cookie(if selected { "222" } else { "111" }).into(),
            exchange(if selected { "222" } else { "111" }).into(),
        ]);
        let f = server(frames).await;
        let r = f
            .provider
            .begin_auth_challenge(&input("default"), CredentialMode::Client)
            .await
            .unwrap();
        if selected {
            f.provider
                .advance_auth_challenge(&r, &submit())
                .await
                .unwrap();
        }
        let action = if selected { select("222") } else { submit() };
        let p = prompt(
            f.provider
                .advance_auth_challenge(&r, &action)
                .await
                .unwrap(),
        );
        let e = f
            .provider
            .advance_auth_challenge(&r, &browser_action(&p))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::AuthenticationRequired);
        assert!(!e.auth_challenge_consumed());
        let status = f.provider.auth_challenge_status(&r).await.unwrap();
        assert_eq!(
            matches!(status, AuthChallengeStatus::AccountSelectionRequired { .. }),
            selected
        );
        assert_eq!(
            f.provider
                .advance_auth_challenge(&r, &browser_action(&p))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        assert!(matches!(
            f.provider
                .advance_auth_challenge(&r, &action)
                .await
                .unwrap(),
            AuthChallengeProgress::Confirmed(_)
        ));
        let wires = f.requests.await.unwrap();
        assert!(!wires[wires.len() - 2].contains("verifydata:"));
    }
}

#[tokio::test]
async fn sms_browser_failed_authorization_identity_reflection_and_store_failures_never_confirm() {
    for case in 0..7 {
        let mut frames = vec![
            reply(json!("sent")).into(),
            required("failure-event").into(),
        ];
        match case {
            0 => frames.push(raw(json!({"status":0,"error_code":30703})).into()),
            1 => frames.push(raw(json!({"status":0,"error_code":20020})).into()),
            2 => frames
                .push(raw(json!({"status":0,"error_code":20028,"data":"missing-event"})).into()),
            3 => frames.push(
                cookie("111")
                    .replace("Content-Type:", "SSA-CODE: private-ssa\r\nContent-Type:")
                    .into(),
            ),
            4 => frames
                .push(raw(json!({"status":1,"error_code":0,"data":{"nickname":PROOF}})).into()),
            _ => frames.extend([
                cookie("111").into(),
                exchange(if case == 5 { "222" } else { "111" }).into(),
            ]),
        }
        let mut f = server(frames).await;
        let store = Arc::new(Store::default());
        f.provider.credential_store = Some(store.clone());
        let (r, p) = begin_browser(&f.provider, "A", CredentialMode::Both).await;
        if case == 6 {
            store.fail.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        let e = f
            .provider
            .advance_auth_challenge(&r, &browser_action(&p))
            .await
            .unwrap_err();
        assert!(e.auth_challenge_consumed(), "case {case}");
        assert!(!format!("{e:?}").contains(PROOF));
        assert!(store.values.lock().unwrap().is_empty());
        assert!(f.provider.auth_challenge_status(&r).await.is_err());
        f.requests.await.unwrap();
    }
}

#[tokio::test]
async fn sms_browser_each_resumed_network_boundary_checks_logout_relogin_and_late_errors() {
    for boundary in 0..2 {
        for change in 0..3 {
            for failure in [false, true] {
                let response = if boundary == 0 {
                    cookie("111")
                } else {
                    exchange("111")
                };
                let (gate, release) = paused(if failure {
                    response.replace("200 OK", "401 Unauthorized")
                } else {
                    response
                });
                let mut frames = vec![reply(json!("sent")).into(), required("late-event").into()];
                if boundary == 1 {
                    frames.push(cookie("111").into());
                }
                frames.push(gate);
                let mut f = server(frames).await;
                let store = Arc::new(Store::default());
                let old = credential("333", "old-session");
                store.put(&old.stored("A").unwrap()).unwrap();
                f.provider.credential_store = Some(store.clone());
                let (r, p) = begin_browser(&f.provider, "A", CredentialMode::Both).await;
                let provider = f.provider.clone();
                let task = tokio::spawn(async move {
                    provider
                        .advance_auth_challenge(&r, &browser_action(&p))
                        .await
                });
                for _ in 0..3 + boundary {
                    f.seen.recv().await.unwrap();
                }
                if change == 0 {
                    f.provider.logout("A").await.unwrap();
                } else {
                    store
                        .put(
                            &credential(if change == 1 { "333" } else { "444" }, "new-session")
                                .stored("A")
                                .unwrap(),
                        )
                        .unwrap();
                }
                release.send(()).unwrap();
                let e = task.await.unwrap().unwrap_err();
                assert!(e.auth_challenge_consumed());
                assert!(matches!(
                    e.code,
                    ErrorCode::Conflict | ErrorCode::AuthenticationRequired
                ));
                if change == 0 {
                    assert!(store.values.lock().unwrap().is_empty());
                } else {
                    assert_eq!(
                        read(&store, "A").user_id(),
                        if change == 1 { "333" } else { "444" }
                    );
                }
                assert!(f.provider.qr_transactions.lock().unwrap().sms.is_empty());
                assert_eq!(f.requests.await.unwrap().len(), 3 + boundary);
            }
        }
    }
}

#[tokio::test]
async fn sms_browser_cancellation_total_timeout_and_original_ttl_consume_pending_work() {
    for boundary in 0..2 {
        for cancel in [false, true] {
            let (gate, release) = paused(if boundary == 0 {
                cookie("111")
            } else {
                exchange("111")
            });
            let mut frames = vec![reply(json!("sent")).into(), required("cancel-event").into()];
            if boundary == 1 {
                frames.push(cookie("111").into());
            }
            frames.push(gate);
            let mut f = server(frames).await;
            f.provider.client.http = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap();
            let (r, p) = begin_browser(&f.provider, "default", CredentialMode::Client).await;
            let provider = f.provider.clone();
            let task = tokio::spawn(async move {
                provider
                    .advance_auth_challenge(&r, &browser_action(&p))
                    .await
            });
            for _ in 0..3 + boundary {
                f.seen.recv().await.unwrap();
            }
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                tokio::time::pause();
                let result = task.await;
                tokio::time::resume();
                assert_eq!(
                    result.unwrap().unwrap_err().code,
                    ErrorCode::UpstreamTimeout
                );
            }
            let _ = release.send(());
            f.requests.await.unwrap();
            assert!(f.provider.qr_transactions.lock().unwrap().sms.is_empty());
        }
    }
    let f = server(vec![reply(json!("sent")).into()]).await;
    let r = f
        .provider
        .begin_auth_challenge(&input("default"), CredentialMode::Client)
        .await
        .unwrap();
    assert_eq!(f.requests.await.unwrap().len(), 1);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(290)).await;
    tokio::time::resume();
    // Start the second transport after advancing time: the fixture's five-second
    // accept timeout is not part of the original authentication deadline.
    let mut second = server(vec![required("ttl-event").into()]).await;
    second.provider.qr_transactions = f.provider.qr_transactions.clone();
    let p = prompt(
        second
            .provider
            .advance_auth_challenge(&r, &submit())
            .await
            .unwrap(),
    );
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(11)).await;
    tokio::time::resume();
    assert!(
        second
            .provider
            .advance_auth_challenge(&r, &browser_action(&p))
            .await
            .unwrap_err()
            .auth_challenge_consumed()
    );
    assert_eq!(second.requests.await.unwrap().len(), 1);
}

#[tokio::test]
async fn sms_browser_receipts_cannot_cross_accounts_and_can_finish_in_reverse_order() {
    let (gate, release) = paused(exchange("111"));
    let mut a = server(vec![
        reply(json!("sent")).into(),
        required("event-a").into(),
        cookie("111").into(),
        gate,
    ])
    .await;
    let mut b = server(vec![
        reply(json!("sent")).into(),
        required("event-b").into(),
        cookie("222").into(),
        exchange("222").into(),
    ])
    .await;
    let store = Arc::new(Store::default());
    a.provider.credential_store = Some(store.clone());
    b.provider.credential_store = Some(store.clone());
    b.provider.qr_transactions = a.provider.qr_transactions.clone();
    let (ra, pa) = begin_browser(&a.provider, "A", CredentialMode::Both).await;
    let mut request = input("B");
    request.principal = "13800000001".into();
    let rb = b
        .provider
        .begin_auth_challenge(&request, CredentialMode::Both)
        .await
        .unwrap();
    let pb = prompt(
        b.provider
            .advance_auth_challenge(&rb, &submit())
            .await
            .unwrap(),
    );
    assert_ne!(pa.verification_id, pb.verification_id);
    assert_eq!(
        b.provider
            .advance_auth_challenge(&rb, &browser_action(&pa))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let provider = a.provider.clone();
    let task = tokio::spawn(async move {
        provider
            .advance_auth_challenge(&ra, &browser_action(&pa))
            .await
    });
    for _ in 0..4 {
        a.seen.recv().await.unwrap();
    }
    assert!(matches!(
        b.provider
            .advance_auth_challenge(&rb, &browser_action(&pb))
            .await
            .unwrap(),
        AuthChallengeProgress::Confirmed(_)
    ));
    assert_eq!(read(&store, "B").user_id(), "222");
    assert!(!store.values.lock().unwrap().contains_key("A"));
    release.send(()).unwrap();
    assert!(matches!(
        task.await.unwrap().unwrap(),
        AuthChallengeProgress::Confirmed(_)
    ));
    assert_eq!(read(&store, "A").user_id(), "111");
    assert_eq!(a.requests.await.unwrap().len(), 4);
    assert_eq!(b.requests.await.unwrap().len(), 4);
}
