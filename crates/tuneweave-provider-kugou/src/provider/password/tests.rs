use super::*;
use crate::provider::session::tests::{Store, credential, paused, raw, read, reply, server};
use std::collections::BTreeMap;
use tuneweave_core::{PasswordFormat, PasswordLoginProgress, PrincipalType};

fn input(account: &str) -> PasswordLoginRequest {
    PasswordLoginRequest {
        backend: Default::default(),
        account: account.into(),
        principal_type: PrincipalType::Username,
        principal: "synthetic-user".into(),
        password: "synthetic 密码+pass".into(),
        password_format: PasswordFormat::Plain,
        country_code: None,
        secure_captcha: None,
    }
}
fn success(uid: &str) -> String {
    reply(json!({"userid":uid})).replace("Content-Type:",&format!("Set-Cookie: KuGoo=KugooID={uid}&t=synthetic-password-token&a_id=1014&NickName=Listener; Domain=.kugou.com; Path=/\r\nContent-Type:"))
}
fn check_request(request: &str, expected_username: &str) {
    let (head, body) = request.split_once("\r\n\r\n").unwrap();
    assert!(body.is_empty());
    assert!(head.starts_with("POST /v1/login_by_pwd_get?"));
    let target = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
    let mut p: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
    let signature = p.remove("signature").unwrap();
    let borrowed = p.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    assert_eq!(signature, crate::signing::web_signature(&borrowed, b""));
    assert_eq!(p["appid"], "1014");
    assert_eq!(p["username"], expected_username);
    assert_eq!(p["clienttime"], p["clienttime_ms"]);
    assert_eq!(p["mid"], p["uuid"]);
    assert_eq!(p["mid"].len(), 32);
    assert_eq!(p.len(), 20);
    assert!(head.to_lowercase().contains("origin: https://m.kugou.com"));
    assert!(!head.to_lowercase().contains("cookie:"));
    for forbidden in [
        "synthetic 密码+pass",
        "old-native",
        "synthetic-password-token",
    ] {
        assert!(!request.contains(forbidden));
    }
}

#[tokio::test]
async fn password_login_supports_three_principals_and_ownership_without_saving_passwords() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        for (kind, principal, expected) in [
            (PrincipalType::Username, "synthetic-user", "synthetic-user"),
            (
                PrincipalType::Email,
                "synthetic@example.test",
                "synthetic@example.test",
            ),
            (PrincipalType::Phone, "13800000000", "13********0"),
        ] {
            let mut f = server(vec![success("111").into()]).await;
            let store = Arc::new(Store::default());
            let old = credential("222", "old-native");
            let account = if mode == CredentialMode::Client {
                "default"
            } else {
                "A"
            };
            store.put(&old.stored(account).unwrap()).unwrap();
            f.provider.credential_store = Some(store.clone());
            let mut request = input(account);
            request.principal_type = kind;
            request.principal = principal.into();
            let PasswordLoginProgress::Confirmed(result) = f
                .provider
                .begin_password_login(&request, mode)
                .await
                .unwrap()
            else {
                panic!("expected login")
            };
            assert_eq!(result.profile.user_id.as_deref(), Some("111"));
            assert_eq!(result.profile.account, account);
            assert_eq!(result.credential.is_some(), mode.returns_to_caller());
            if let Some(caller) = result.credential {
                assert!(!caller.secret().contains(&request.password));
                assert!(!caller.secret().contains(principal));
                assert!(matches!(
                    KugouCredential::parse_caller(&caller).unwrap(),
                    KugouCredential::Web(_)
                ));
                if mode == CredentialMode::Both {
                    assert_eq!(
                        KugouCredential::parse_caller(&caller).unwrap(),
                        read(&store, account)
                    );
                }
            }
            if mode == CredentialMode::Client {
                assert_eq!(read(&store, account), old);
            } else {
                assert!(!read(&store, account).same_login(&old));
            }
            assert!(
                f.provider
                    .qr_transactions
                    .lock()
                    .unwrap()
                    .passwords
                    .is_empty()
            );
            let requests = f.requests.await.unwrap();
            assert_eq!(requests.len(), 1);
            check_request(&requests[0], expected);
            if kind == PrincipalType::Phone {
                assert!(!requests[0].contains(principal));
            }
        }
    }
}

#[tokio::test]
async fn failed_password_login_preserves_old_accounts_and_requires_explicit_verification() {
    for frame in [
        raw(json!({"status":0,"error_code":30701,"data":"synthetic 密码+pass"})),
        raw(json!({"status":0,"error_code":30767,"data":"13800000000"})),
        reply(json!({"userid":"111"})),
        success("111").replace("KugooID=111", "KugooID=222"),
        success("111").replace("200 OK", "401 Unauthorized"),
    ] {
        let mut f = server(vec![frame.into()]).await;
        let store = Arc::new(Store::default());
        let old = credential("222", "old-native");
        store.put(&old.stored("A").unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        let mut error = f
            .provider
            .password_login_with_mode(&input("A"), CredentialMode::Both)
            .await
            .unwrap_err();
        assert!(error.take_caller_credential_update().is_none());
        assert!(!format!("{error:?}").contains("13800000000"));
        assert_eq!(read(&store, "A"), old);
        assert!(
            f.provider
                .qr_transactions
                .lock()
                .unwrap()
                .passwords
                .is_empty()
        );
        f.requests.await.unwrap();
    }
}

#[tokio::test]
async fn password_logout_and_expiration_discard_late_success_and_failure() {
    for existing in [false, true] {
        for failure in [false, true] {
            for expire in [false, true] {
                let (frame, resume) = paused(if failure {
                    raw(json!({"status":0,"error_code":30701,"data":null}))
                } else {
                    success("111")
                });
                let mut f = server(vec![frame]).await;
                let store = Arc::new(Store::default());
                let old = credential("222", "old-native");
                if existing {
                    store.put(&old.stored("A").unwrap()).unwrap();
                }
                f.provider.credential_store = Some(store.clone());
                let worker = f.provider.clone();
                let task = tokio::spawn(async move {
                    worker
                        .password_login_with_mode(&input("A"), CredentialMode::Both)
                        .await
                });
                f.seen.recv().await.unwrap();
                if expire {
                    for value in f
                        .provider
                        .qr_transactions
                        .lock()
                        .unwrap()
                        .passwords
                        .values_mut()
                    {
                        value.deadline = Instant::now();
                    }
                } else {
                    f.provider
                        .logout_with_ownership("A", None, CredentialMode::Server)
                        .await
                        .unwrap();
                }
                let requests_before = store.values.lock().unwrap().clone();
                resume.send(()).unwrap();
                assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
                assert_eq!(*store.values.lock().unwrap(), requests_before);
                assert!(
                    f.provider
                        .qr_transactions
                        .lock()
                        .unwrap()
                        .passwords
                        .is_empty()
                );
                f.requests.await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn independent_password_logins_use_conditional_first_write_and_preserve_the_winner() {
    for existing in [false, true] {
        let (frame_a, resume_a) = paused(success("111"));
        let (frame_b, resume_b) = paused(success("222"));
        let mut a = server(vec![frame_a]).await;
        let mut b = server(vec![frame_b]).await;
        let store = Arc::new(Store::default());
        if existing {
            store
                .put(&credential("333", "old-native").stored("A").unwrap())
                .unwrap();
        }
        a.provider.credential_store = Some(store.clone());
        b.provider.credential_store = Some(store.clone());
        let wa = a.provider.clone();
        let wb = b.provider.clone();
        let ta = tokio::spawn(async move {
            wa.password_login_with_mode(&input("A"), CredentialMode::Both)
                .await
        });
        let tb = tokio::spawn(async move {
            wb.password_login_with_mode(&input("A"), CredentialMode::Both)
                .await
        });
        a.seen.recv().await.unwrap();
        b.seen.recv().await.unwrap();
        resume_a.send(()).unwrap();
        let winner = ta.await.unwrap().unwrap();
        resume_b.send(()).unwrap();
        assert_eq!(tb.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(
            read(&store, "A"),
            KugouCredential::parse_caller(winner.credential.as_ref().unwrap()).unwrap()
        );
        a.requests.await.unwrap();
        b.requests.await.unwrap();
    }
}

#[tokio::test]
async fn password_request_abort_releases_shared_capacity_and_input_rejection_never_connects() {
    let (frame, resume) = paused(success("111"));
    let mut f = server(vec![frame]).await;
    let worker = f.provider.clone();
    let task = tokio::spawn(async move {
        worker
            .password_login_with_mode(&input("default"), CredentialMode::Client)
            .await
    });
    f.seen.recv().await.unwrap();
    assert_eq!(
        f.provider.qr_transactions.lock().unwrap().passwords.len(),
        1
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(
        f.provider
            .qr_transactions
            .lock()
            .unwrap()
            .passwords
            .is_empty()
    );
    resume.send(()).unwrap();
    f.requests.await.unwrap();
    let f = server(vec![]).await;
    let mut bad = input("default");
    bad.password_format = PasswordFormat::Md5;
    assert!(
        f.provider
            .password_login_with_mode(&bad, CredentialMode::Client)
            .await
            .is_err()
    );
    assert!(
        f.provider
            .password_login_with_mode(&input("A"), CredentialMode::Client)
            .await
            .is_err()
    );
    assert!(f.provider.password_login(&input("A")).await.is_err());
    let scoped = f
        .provider
        .caller_scope(&credential("111", "caller").caller().unwrap())
        .unwrap();
    assert!(
        scoped
            .password_login_with_mode(&input("default"), CredentialMode::Client)
            .await
            .is_err()
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn password_sdk_returns_only_a_verified_caller_session() {
    let f = server(vec![success("111").into()]).await;
    assert!(
        f.provider
            .client
            .login_web_password(&input("A"))
            .await
            .is_err()
    );
    let result = f
        .provider
        .client
        .login_web_password(&input("default"))
        .await
        .unwrap();
    assert!(result.profile.authenticated);
    assert_eq!(result.profile.account, "default");
    assert_eq!(result.credential.unwrap().kind, "kugou_web_v1");
    assert_eq!(f.requests.await.unwrap().len(), 1);
}

#[tokio::test]
async fn in_flight_password_and_qr_logins_share_capacity_and_abort_releases_one_slot() {
    let created = reply(json!({"qrcode":"0123456789ABCDEF0123456789ABCDEF0123"}));
    let (frame, resume) = paused(success("111"));
    let mut frames = (0..127).map(|_| created.clone().into()).collect::<Vec<_>>();
    frames.push(frame);
    frames.push(created.into());
    let mut f = server(frames).await;
    for _ in 0..127 {
        f.provider
            .start_qr_login_with_mode(Some("web"), CredentialMode::Client)
            .await
            .unwrap();
        f.seen.recv().await.unwrap();
    }
    let worker = f.provider.clone();
    let task = tokio::spawn(async move {
        worker
            .password_login_with_mode(&input("default"), CredentialMode::Client)
            .await
    });
    f.seen.recv().await.unwrap();
    assert_eq!(
        f.provider
            .start_qr_login_with_mode(Some("web"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    assert_eq!(
        f.provider
            .password_login_with_mode(&input("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    resume.send(()).unwrap();
    f.provider
        .start_qr_login_with_mode(Some("web"), CredentialMode::Client)
        .await
        .unwrap();
    assert_eq!(f.requests.await.unwrap().len(), 129);
}

#[tokio::test]
async fn successful_qr_and_password_logins_cancel_each_others_bound_attempts() {
    for password_wins in [false, true] {
        let (password_frame, resume) = paused(success("111"));
        let mut a = server(vec![password_frame]).await;
        let mut qr_frames =
            vec![reply(json!({"qrcode":"0123456789ABCDEF0123456789ABCDEF0123"})).into()];
        if password_wins {
            qr_frames.push(reply(json!({"status":1})).into());
        } else {
            qr_frames.push(reply(json!({"status":4,"userid":"111","token":"qr-token"})).into());
            qr_frames.push(success("111").into());
        }
        let mut b = server(qr_frames).await;
        let store = Arc::new(Store::default());
        a.provider.credential_store = Some(store.clone());
        b.provider.credential_store = Some(store.clone());
        b.provider.qr_transactions = a.provider.qr_transactions.clone();
        let start = b
            .provider
            .start_qr_login_with_mode(Some("web"), CredentialMode::Both)
            .await
            .unwrap();
        let worker = a.provider.clone();
        let task = tokio::spawn(async move {
            worker
                .password_login_with_mode(&input("A"), CredentialMode::Both)
                .await
        });
        a.seen.recv().await.unwrap();
        let result = b
            .provider
            .poll_qr_login_with_mode(&start.provider_transaction_id, "A", CredentialMode::Both)
            .await
            .unwrap();
        if password_wins {
            assert_eq!(result.state, AuthState::Waiting);
            resume.send(()).unwrap();
            task.await.unwrap().unwrap();
            assert_eq!(
                b.provider
                    .poll_qr_login_with_mode(
                        &start.provider_transaction_id,
                        "A",
                        CredentialMode::Both
                    )
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::ResourceNotFound
            );
        } else {
            assert_eq!(result.state, AuthState::Confirmed);
            resume.send(()).unwrap();
            assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
        }
        assert_eq!(read(&store, "A").user_id(), "111");
        a.requests.await.unwrap();
        b.requests.await.unwrap();
    }
}

#[tokio::test]
async fn password_timeout_and_store_failure_never_export_a_session_or_retry() {
    for timeout in [false, true] {
        let (frame, resume) = paused(success("111"));
        let mut f = server(vec![frame]).await;
        if timeout {
            f.provider.client.http = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .retry(reqwest::retry::never())
                .timeout(Duration::from_millis(100))
                .build()
                .unwrap();
        }
        let store = Arc::new(Store::default());
        let old = credential("222", "old-native");
        store.put(&old.stored("A").unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        let worker = f.provider.clone();
        let task = tokio::spawn(async move {
            worker
                .password_login_with_mode(&input("A"), CredentialMode::Both)
                .await
        });
        f.seen.recv().await.unwrap();
        if !timeout {
            store.fail.store(true, std::sync::atomic::Ordering::SeqCst);
            resume.send(()).unwrap();
            let mut error = task.await.unwrap().unwrap_err();
            assert_eq!(error.code, ErrorCode::InternalError);
            assert!(error.take_caller_credential_update().is_none());
        } else {
            let mut error = task.await.unwrap().unwrap_err();
            assert_eq!(error.code, ErrorCode::UpstreamTimeout);
            assert!(error.take_caller_credential_update().is_none());
            resume.send(()).unwrap();
        }
        assert_eq!(read(&store, "A"), old);
        assert!(
            f.provider
                .qr_transactions
                .lock()
                .unwrap()
                .passwords
                .is_empty()
        );
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
}
