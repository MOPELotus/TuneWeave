use super::super::tests::{SEED, answer, image, input, pending, rejected, request_parts, success};
use super::*;
use crate::login::crypto::ExchangeCipher;
use crate::provider::session::tests::{Store, credential, paused, raw, read, reply, server};
use serde_json::Value;

fn challenge(code: i64, target: Value) -> String {
    raw(json!({"status":0,"error_code":code,"data":target}))
}
fn sent() -> String {
    reply(json!({"msg":"sent","count":1}))
}
fn submit(code: &str) -> PasswordChallengeAction {
    PasswordChallengeAction::SubmitSms { code: code.into() }
}
fn choose(user_id: &str, code: &str) -> PasswordChallengeAction {
    PasswordChallengeAction::SelectAccount {
        user_id: user_id.into(),
        code: code.into(),
    }
}
fn choices() -> String {
    reply(json!({"isreg":1,"info_list":[
        {"userid":"111","nickname":"One","pic":"https://kgimg.com/one.jpg"},
        {"userid":"222","nickname":"Two","pic":"https://kgimg.com/two.jpg"}
    ]}))
}
fn sms_pending(
    progress: PasswordLoginProgress,
    remaining: u8,
    display: &str,
) -> ProviderPasswordChallenge {
    let PasswordLoginProgress::Pending {
        challenge,
        verification,
    } = progress
    else {
        panic!("expected SMS");
    };
    let debug = format!("{verification:?}");
    let PasswordVerification::Sms {
        masked_destination,
        remaining_attempts,
        resend_after_secs,
    } = verification
    else {
        panic!("expected SMS");
    };
    assert_eq!(remaining_attempts, remaining);
    assert_eq!(masked_destination, display);
    assert!(resend_after_secs <= 31);
    assert!(!debug.contains("13800138000"));
    challenge
}
fn ready(provider: &KugouProvider, receipt: &ProviderPasswordChallenge) {
    let mut registry = provider.qr_transactions.lock().unwrap();
    let context = registry
        .passwords
        .get_mut(receipt.provider_transaction_id())
        .unwrap()
        .native
        .as_mut()
        .unwrap()
        .context
        .as_mut()
        .unwrap();
    let Some(Stage::Sms(sms)) = context.stage.as_mut() else {
        panic!("expected SMS");
    };
    sms.next_send = Instant::now();
}
fn secret(request: &str) -> (Value, Value) {
    let (mut query, body) = request_parts(request);
    let signature = query.remove("signature").unwrap();
    assert_eq!(
        signature,
        crate::signing::android_signature(
            &query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect(),
            body.as_bytes()
        )
    );
    let body: Value = serde_json::from_str(body).unwrap();
    let secret = serde_json::from_slice(
        &ExchangeCipher::for_test(SEED)
            .decrypt(body["params"].as_str().unwrap())
            .unwrap(),
    )
    .unwrap();
    (body, secret)
}

#[tokio::test]
async fn native_secondary_sms_rejects_voice_actions_without_consuming_or_sending() {
    let mut f = server(vec![
        challenge(30768, json!("111")).into(),
        sent().into(),
        success().into(),
        reply(json!({"userid":"111"})).into(),
    ])
    .await;
    f.provider.client.password_test_seed = Some(SEED.into());
    let receipt = sms_pending(
        f.provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap(),
        5,
        "账号绑定手机",
    );
    ready(&f.provider, &receipt);
    let snapshot = || {
        let registry = f.provider.qr_transactions.lock().unwrap();
        let entry = &registry.passwords[receipt.provider_transaction_id()];
        let context = entry.native.as_ref().unwrap().context.as_ref().unwrap();
        let Some(Stage::Sms(sms)) = context.stage.as_ref() else {
            panic!("expected SMS");
        };
        (entry.deadline, context.attempts, sms.sends, sms.next_send)
    };
    let before = snapshot();
    for action in [
        PasswordChallengeAction::SubmitVoice {
            code: "123456".into(),
        },
        PasswordChallengeAction::ResendVoice,
    ] {
        let error = f
            .provider
            .advance_password_login(&receipt, &action)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert!(!error.auth_challenge_consumed());
        assert_eq!(snapshot(), before);
    }
    assert!(matches!(
        f.provider
            .advance_password_login(&receipt, &submit("654321"))
            .await
            .unwrap(),
        PasswordLoginProgress::Confirmed(_)
    ));
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 4);
    assert!(requests[1].starts_with("POST /v8/send_mobile_code?"));
    assert!(requests[2].starts_with("POST /login.user/v7/login_by_verifycode?"));
    assert_eq!(secret(&requests[2]).1["code"], "654321");
}

#[tokio::test]
async fn native_secondary_sms_phone_and_uid_retry_resend_then_commit_all_ownership_modes() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        for (target, display) in [("13800138000", "138*****000"), ("111", "账号绑定手机")] {
            let mut f = server(vec![
                challenge(30768, json!(target)).into(),
                sent().into(),
                rejected(20021).into(),
                sent().into(),
                success().into(),
                reply(json!({"userid":"111"})).into(),
            ])
            .await;
            f.provider.client.password_test_seed = Some(SEED.into());
            let account = if mode == CredentialMode::Client {
                "default"
            } else {
                "A"
            };
            let store = Arc::new(Store::default());
            let old = credential("222", "old-token");
            store.put(&old.stored(account).unwrap()).unwrap();
            f.provider.credential_store = Some(store.clone());
            let receipt = sms_pending(
                f.provider
                    .begin_password_login(&input(account), mode)
                    .await
                    .unwrap(),
                5,
                display,
            );
            let deadline = f.provider.qr_transactions.lock().unwrap().passwords
                [receipt.provider_transaction_id()]
            .deadline;
            assert_eq!(read(&store, account), old);
            for action in [
                submit("not-a-code"),
                answer("A7b9", "wrong-path"),
                choose("111", "123456"),
                PasswordChallengeAction::ResendSms,
            ] {
                let error = f
                    .provider
                    .advance_password_login(&receipt, &action)
                    .await
                    .unwrap_err();
                assert!(matches!(
                    error.code,
                    ErrorCode::InvalidRequest | ErrorCode::RateLimited
                ));
                assert!(!error.auth_challenge_consumed());
            }
            assert_eq!(
                sms_pending(
                    f.provider
                        .advance_password_login(&receipt, &submit("123456"))
                        .await
                        .unwrap(),
                    4,
                    display
                ),
                receipt
            );
            ready(&f.provider, &receipt);
            assert_eq!(
                sms_pending(
                    f.provider
                        .advance_password_login(&receipt, &PasswordChallengeAction::ResendSms)
                        .await
                        .unwrap(),
                    4,
                    display
                ),
                receipt
            );
            assert_eq!(
                f.provider.qr_transactions.lock().unwrap().passwords
                    [receipt.provider_transaction_id()]
                .deadline,
                deadline
            );
            let PasswordLoginProgress::Confirmed(result) = f
                .provider
                .advance_password_login(&receipt, &submit("654321"))
                .await
                .unwrap()
            else {
                panic!("expected confirmed");
            };
            assert_eq!(result.profile.user_id.as_deref(), Some("111"));
            assert_eq!(result.profile.account, account);
            assert_eq!(result.credential.is_some(), mode.returns_to_caller());
            assert_eq!(
                read(&store, account).same_login(&old),
                !mode.persists_on_server()
            );
            assert_eq!(
                f.provider
                    .advance_password_login(&receipt, &submit("654321"))
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::ResourceNotFound
            );
            let requests = f.requests.await.unwrap();
            assert_eq!(requests.len(), 6);
            assert!(requests[1].starts_with("POST /v8/send_mobile_code?"));
            assert!(requests[2].starts_with("POST /login.user/v7/login_by_verifycode?"));
            let mid = request_parts(&requests[0]).0["mid"].clone();
            for index in [1, 2, 3, 4] {
                assert_eq!(request_parts(&requests[index]).0["mid"], mid);
                assert_eq!(request_parts(&requests[index]).0["clientver"], "20809");
                let (body, secret) = secret(&requests[index]);
                for name in ["pwd", "force", "data", "verifycode", "token"] {
                    assert!(secret.get(name).is_none());
                }
                assert!(body.get("force_login").is_none());
                if [1, 3].contains(&index) {
                    assert_eq!(body["businessid"], 5);
                    assert_eq!(
                        body["clienttime_ms"].as_u64().unwrap().to_string(),
                        request_parts(&requests[index]).0["clienttime"]
                    );
                    if target == "111" {
                        assert_eq!(body["userid"], 111);
                        assert_eq!(secret["mobile"], "");
                    } else {
                        assert_eq!(body["mobile"], display);
                        assert_eq!(secret["mobile"], target);
                    }
                } else {
                    assert_eq!(secret["mobile"], target);
                    assert_eq!(secret["code"], if index == 2 { "123456" } else { "654321" });
                    assert!(secret.get("mobile_data").is_some());
                    assert!(secret.get("clienttime_ms").is_none());
                }
                assert!(!requests[index].contains("first-password"));
                assert!(!requests[index].contains("13800138000"));
            }
        }
    }
}

#[tokio::test]
async fn native_secondary_sms_selects_only_offered_accounts_with_a_fresh_code() {
    let mut f = server(vec![
        challenge(34216, json!("13800138000")).into(),
        sent().into(),
        rejected(34175).into(),
        choices().into(),
        success().into(),
        reply(json!({"userid":"111"})).into(),
    ])
    .await;
    f.provider.client.password_test_seed = Some(SEED.into());
    let receipt = sms_pending(
        f.provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap(),
        5,
        "138*****000",
    );
    let PasswordLoginProgress::Pending {
        challenge: again,
        verification:
            PasswordVerification::AccountSelection {
                accounts,
                remaining_attempts,
            },
    } = f
        .provider
        .advance_password_login(&receipt, &submit("123456"))
        .await
        .unwrap()
    else {
        panic!("expected account choices");
    };
    assert_eq!(again, receipt);
    assert_eq!(remaining_attempts, 4);
    assert_eq!(
        accounts
            .iter()
            .map(|a| a.user_id.as_str())
            .collect::<Vec<_>>(),
        vec!["111", "222"]
    );
    for action in [
        choose("333", "654321"),
        submit("654321"),
        choose("111", "bad"),
    ] {
        let error = f
            .provider
            .advance_password_login(&receipt, &action)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InvalidRequest);
        assert!(!error.auth_challenge_consumed());
    }
    assert!(matches!(
        f.provider
            .advance_password_login(&receipt, &choose("111", "654321"))
            .await
            .unwrap(),
        PasswordLoginProgress::Confirmed(_)
    ));
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 6);
    assert!(requests[3].starts_with("POST /v4/check_mobile?"));
    let (body, proof) = secret(&requests[3]);
    assert_eq!(body["businessid"], 5);
    assert_eq!(proof["code"], "123456");
    let (body, proof) = secret(&requests[4]);
    assert_eq!(body["userid"], "111");
    assert_eq!(proof["code"], "654321");
    assert_eq!(proof["mobile"], "13800138000");
}

#[tokio::test]
async fn native_secondary_sms_preserves_image_attempts_and_does_not_turn_bad_sms_into_an_image() {
    let mut f = server(vec![
        rejected(30709).into(),
        image("private-key").into(),
        challenge(30768, json!("111")).into(),
        sent().into(),
        rejected(20020).into(),
        success().into(),
        reply(json!({"userid":"111"})).into(),
    ])
    .await;
    f.provider.client.password_test_seed = Some(SEED.into());
    let receipt = pending(
        f.provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap(),
        5,
    );
    let deadline = f.provider.qr_transactions.lock().unwrap().passwords
        [receipt.provider_transaction_id()]
    .deadline;
    assert_eq!(
        sms_pending(
            f.provider
                .advance_password_login(&receipt, &answer("A7b9", "second-password"))
                .await
                .unwrap(),
            4,
            "账号绑定手机"
        ),
        receipt
    );
    assert_eq!(
        sms_pending(
            f.provider
                .advance_password_login(&receipt, &submit("123456"))
                .await
                .unwrap(),
            3,
            "账号绑定手机"
        ),
        receipt
    );
    assert_eq!(
        f.provider.qr_transactions.lock().unwrap().passwords[receipt.provider_transaction_id()]
            .deadline,
        deadline
    );
    assert!(matches!(
        f.provider
            .advance_password_login(&receipt, &submit("654321"))
            .await
            .unwrap(),
        PasswordLoginProgress::Confirmed(_)
    ));
    assert_eq!(f.requests.await.unwrap().len(), 7);
}

#[tokio::test]
async fn native_secondary_sms_mismatched_uid_and_selected_account_never_commit() {
    for selection in [false, true] {
        let mut frames = vec![
            challenge(30768, json!(if selection { "13800138000" } else { "333" })).into(),
            sent().into(),
        ];
        if selection {
            frames.extend([rejected(34175).into(), choices().into()]);
        }
        frames.push(success().into());
        let mut f = server(frames).await;
        f.provider.client.password_test_seed = Some(SEED.into());
        let store = Arc::new(Store::default());
        let old = credential("222", "old-token");
        store.put(&old.stored("A").unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        let receipt = sms_pending(
            f.provider
                .begin_password_login(&input("A"), CredentialMode::Both)
                .await
                .unwrap(),
            5,
            if selection {
                "138*****000"
            } else {
                "账号绑定手机"
            },
        );
        if selection {
            assert!(matches!(
                f.provider
                    .advance_password_login(&receipt, &submit("123456"))
                    .await
                    .unwrap(),
                PasswordLoginProgress::Pending {
                    verification: PasswordVerification::AccountSelection { .. },
                    ..
                }
            ));
        }
        let action = if selection {
            choose("222", "123456")
        } else {
            submit("123456")
        };
        let error = f
            .provider
            .advance_password_login(&receipt, &action)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::PermissionDenied);
        assert!(error.auth_challenge_consumed());
        assert_eq!(read(&store, "A"), old);
        assert!(
            f.provider
                .qr_transactions
                .lock()
                .unwrap()
                .passwords
                .is_empty()
        );
        assert_eq!(
            f.requests.await.unwrap().len(),
            if selection { 5 } else { 3 }
        );
    }
}

#[tokio::test]
async fn native_secondary_sms_last_answer_and_send_limits_do_not_retry_automatically() {
    let mut frames = vec![challenge(30768, json!("111")).into(), sent().into()];
    frames.extend((0..5).map(|_| rejected(20021).into()));
    let f = server(frames).await;
    let receipt = sms_pending(
        f.provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap(),
        5,
        "账号绑定手机",
    );
    for remaining in (1..5).rev() {
        sms_pending(
            f.provider
                .advance_password_login(&receipt, &submit("123456"))
                .await
                .unwrap(),
            remaining,
            "账号绑定手机",
        );
    }
    let error = f
        .provider
        .advance_password_login(&receipt, &submit("123456"))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::RateLimited);
    assert!(error.auth_challenge_consumed());
    assert_eq!(f.requests.await.unwrap().len(), 7);

    let mut frames = vec![challenge(30768, json!("111")).into()];
    frames.extend((0..5).map(|_| sent().into()));
    let f = server(frames).await;
    let receipt = sms_pending(
        f.provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap(),
        5,
        "账号绑定手机",
    );
    for _ in 1..5 {
        ready(&f.provider, &receipt);
        sms_pending(
            f.provider
                .advance_password_login(&receipt, &PasswordChallengeAction::ResendSms)
                .await
                .unwrap(),
            5,
            "账号绑定手机",
        );
    }
    ready(&f.provider, &receipt);
    let error = f
        .provider
        .advance_password_login(&receipt, &PasswordChallengeAction::ResendSms)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::RateLimited);
    assert!(!error.auth_challenge_consumed());
    assert_eq!(f.requests.await.unwrap().len(), 6);
}

#[tokio::test]
async fn native_secondary_sms_logout_during_verify_prevents_account_lookup_and_cancellation_releases_state()
 {
    let (frame, resume) = paused(rejected(34175));
    let mut f = server(vec![
        challenge(30768, json!("13800138000")).into(),
        sent().into(),
        frame,
    ])
    .await;
    let store = Arc::new(Store::default());
    store
        .put(&credential("222", "old-token").stored("A").unwrap())
        .unwrap();
    f.provider.credential_store = Some(store.clone());
    let receipt = sms_pending(
        f.provider
            .begin_password_login(&input("A"), CredentialMode::Both)
            .await
            .unwrap(),
        5,
        "138*****000",
    );
    f.seen.recv().await.unwrap();
    f.seen.recv().await.unwrap();
    let worker = f.provider.clone();
    let task = tokio::spawn(async move {
        worker
            .advance_password_login(&receipt, &submit("123456"))
            .await
    });
    f.seen.recv().await.unwrap();
    f.provider
        .logout_with_ownership("A", None, CredentialMode::Server)
        .await
        .unwrap();
    resume.send(()).unwrap();
    assert!(task.await.unwrap().is_err());
    assert!(store.values.lock().unwrap().is_empty());
    assert_eq!(f.requests.await.unwrap().len(), 3);

    let (frame, resume) = paused(sent());
    let mut f = server(vec![challenge(30768, json!("111")).into(), frame]).await;
    let worker = f.provider.clone();
    let task = tokio::spawn(async move {
        worker
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
    });
    f.seen.recv().await.unwrap();
    f.seen.recv().await.unwrap();
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
    assert_eq!(f.requests.await.unwrap().len(), 2);
}

#[tokio::test]
async fn native_secondary_sms_rejects_unknown_or_malformed_targets_without_sending() {
    for (code, target) in [
        (30767, json!("13800138000")),
        (30768, json!({"mobile":"13800138000"})),
        (30768, json!("138*****000")),
        (34216, json!(-1)),
        (30768, json!("9223372036854775808")),
    ] {
        let f = server(vec![challenge(code, target).into()]).await;
        let error = f
            .provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap_err();
        assert!(!format!("{error:?}").contains("13800138000"));
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
