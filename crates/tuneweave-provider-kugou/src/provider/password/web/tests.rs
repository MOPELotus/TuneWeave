use super::*;
use crate::provider::session::tests::{Store, credential, paused, raw, read, reply, server};
use serde_json::Value;
use std::collections::BTreeMap;
use tuneweave_core::{
    AuthBrowserChallenge, AuthChallengeBackend, AuthChallengeRequest, ChallengeMethod,
    PasswordFormat, PrincipalType,
};

const PHONE: &str = "13800000000";
const PASSWORD: &str = "synthetic-web-password";
fn input(account: &str) -> PasswordLoginRequest {
    PasswordLoginRequest {
        backend: PasswordLoginBackend::Web,
        account: account.into(),
        principal_type: PrincipalType::Username,
        principal: "synthetic-web-user".into(),
        password: PASSWORD.into(),
        password_format: PasswordFormat::Plain,
        country_code: None,
        secure_captcha: None,
    }
}
fn phone(code: u32) -> String {
    raw(json!({"status":0,"error_code":code,"data":PHONE}))
}
fn rejected(code: u32) -> String {
    raw(json!({"status":0,"error_code":code}))
}
fn cookie(uid: &str) -> String {
    reply(
        json!({"name":"KuGoo","domain":".kugou.com","path":"/","value":format!("KugooID={uid}&t=synthetic-sms-cookie&a_id=1014&NickName=Listener")}),
    )
}
fn exchange(uid: &str) -> String {
    reply(json!({})).replace("Content-Type:", &format!("Set-Cookie: KuGoo=KugooID={uid}&t=synthetic-verified&a_id=1014&NickName=Verified; Domain=.kugou.com; Path=/\r\nContent-Type:"))
}
fn submit() -> PasswordChallengeAction {
    PasswordChallengeAction::SubmitSms {
        code: "123456".into(),
    }
}
fn select(uid: &str) -> PasswordChallengeAction {
    PasswordChallengeAction::SelectAccount {
        user_id: uid.into(),
        code: "654321".into(),
    }
}
fn callback(p: &AuthBrowserChallenge) -> PasswordChallengeAction {
    PasswordChallengeAction::SubmitSmsBrowser {
        verification_id: p.verification_id.clone(),
        code: "654321".into(),
        response:
            json!({"status":1,"error_code":0,"vType":2,"verify_data":"synthetic%2Bproof%252F"})
                .to_string(),
    }
}
fn pending(p: PasswordLoginProgress) -> (ProviderPasswordChallenge, PasswordVerification) {
    let PasswordLoginProgress::Pending {
        challenge,
        verification,
    } = p
    else {
        panic!("pending expected")
    };
    (challenge, verification)
}
fn parts(wire: &str) -> (BTreeMap<String, String>, Value) {
    let (head, body) = wire.split_once("\r\n\r\n").unwrap();
    let url = url::Url::parse(&format!(
        "http://local{}",
        head.split_whitespace().nth(1).unwrap()
    ))
    .unwrap();
    let mut query: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
    let signature = query.remove("signature").unwrap();
    assert_eq!(
        signature,
        crate::signing::web_signature(
            &query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect(),
            body.as_bytes()
        )
    );
    assert!(!wire.contains(PASSWORD));
    (
        query,
        if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_str(body).unwrap()
        },
    )
}
fn ready(provider: &KugouProvider, receipt: &ProviderPasswordChallenge) {
    let mut registry = provider.qr_transactions.lock().unwrap();
    registry.sms_cooldowns.clear();
    registry
        .passwords
        .get_mut(receipt.provider_transaction_id())
        .unwrap()
        .web
        .as_mut()
        .unwrap()
        .context
        .as_mut()
        .unwrap()
        .next_send = tokio::time::Instant::now();
}

#[tokio::test]
async fn web_secondary_sms_reuses_password_device_and_original_principal_in_all_ownership_modes() {
    for (mode, backend, principal, kind, rejection) in [
        (
            CredentialMode::Client,
            PasswordLoginBackend::Default,
            "13800000000",
            PrincipalType::Phone,
            30767,
        ),
        (
            CredentialMode::Server,
            PasswordLoginBackend::Web,
            "synthetic@example.test",
            PrincipalType::Email,
            30768,
        ),
        (
            CredentialMode::Both,
            PasswordLoginBackend::Web,
            "111",
            PrincipalType::Username,
            30768,
        ),
    ] {
        let mut f = server(vec![
            phone(rejection).into(),
            reply(json!("sent")).into(),
            cookie("111").into(),
            exchange("111").into(),
        ])
        .await;
        let store = Arc::new(Store::default());
        f.provider.credential_store = Some(store.clone());
        let account = if mode == CredentialMode::Client {
            "default"
        } else {
            "A"
        };
        let old = credential("222", "old-native");
        store.put(&old.stored(account).unwrap()).unwrap();
        let mut request = input(account);
        request.backend = backend;
        request.principal = principal.into();
        request.principal_type = kind;
        let (receipt, verification) = pending(
            f.provider
                .begin_password_login(&request, mode)
                .await
                .unwrap(),
        );
        assert!(matches!(
            verification,
            PasswordVerification::Sms {
                remaining_attempts: 5,
                ..
            }
        ));
        let serialized = serde_json::to_string(&verification).unwrap();
        assert!(!serialized.contains(PHONE) && !serialized.contains(PASSWORD));
        assert_eq!(read(&store, account), old);
        let PasswordLoginProgress::Confirmed(result) = f
            .provider
            .advance_password_login(&receipt, &submit())
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(result.profile.user_id.as_deref(), Some("111"));
        assert_eq!(result.profile.account, account);
        assert_eq!(result.credential.is_some(), mode.returns_to_caller());
        assert_eq!(read(&store, account) == old, !mode.persists_on_server());
        assert!(
            f.provider
                .advance_password_login(&receipt, &submit())
                .await
                .is_err()
        );
        let wires = f.requests.await.unwrap();
        let password = parts(&wires[0]).0;
        let (send, send_body) = parts(&wires[1]);
        let (verify, verify_body) = parts(&wires[2]);
        assert!(wires[1].starts_with("POST /v8/send_mobile_code?"));
        assert!(wires[2].starts_with("POST /v2/loginbyverifycode/?"));
        assert!(wires[1].contains("x-router: loginservice.kugou.com"));
        assert_eq!(password["mid"], send["mid"]);
        assert_eq!(password["mid"], verify["mid"]);
        assert_eq!(password["dfid"], verify["dfid"]);
        assert_eq!(send_body["businessid"], 5);
        assert_eq!(send_body["mobile"], "13********0");
        assert!(!wires[1].contains(PHONE));
        assert_eq!(verify_body["userid"], Value::String(principal.into()));
        assert_eq!(verify_body["force_login"], 0);
        assert_eq!(verify_body["support_multi"], 1);
        assert!(verify_body.get("pwd").is_none());
    }
}

#[tokio::test]
async fn web_secondary_selection_and_browser_preserve_numeric_uid_and_rotate_proof() {
    let f = server(vec![phone(30768).into(), reply(json!("sent")).into(), rejected(34175).into(),
        reply(json!({"info_list":[{"userid":111,"nickname":"One"},{"userid":"222","nickname":"Two"}]})).into(),
        raw(json!({"status":0,"error_code":20028,"data":"eventid=first"})).into(),
        raw(json!({"status":0,"error_code":20028,"data":"eventid=second"})).into(),
        cookie("222").into(), exchange("222").into()]).await;
    let (receipt, _) = pending(
        f.provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap(),
    );
    let (_, v) = pending(
        f.provider
            .advance_password_login(&receipt, &submit())
            .await
            .unwrap(),
    );
    assert!(matches!(
        v,
        PasswordVerification::AccountSelection {
            remaining_attempts: 4,
            ..
        }
    ));
    assert_eq!(
        f.provider
            .advance_password_login(&receipt, &select("333"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let (
        _,
        PasswordVerification::SmsBrowser {
            verification: first,
        },
    ) = pending(
        f.provider
            .advance_password_login(&receipt, &select("222"))
            .await
            .unwrap(),
    )
    else {
        panic!()
    };
    assert_eq!(first.remaining_attempts, 3);
    let wrong_kind = PasswordChallengeAction::SubmitBrowser {
        verification_id: first.verification_id.clone(),
        response: "{}".into(),
        password: PASSWORD.into(),
    };
    assert_eq!(
        f.provider
            .advance_password_login(&receipt, &wrong_kind)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let (
        _,
        PasswordVerification::SmsBrowser {
            verification: second,
        },
    ) = pending(
        f.provider
            .advance_password_login(&receipt, &callback(&first))
            .await
            .unwrap(),
    )
    else {
        panic!()
    };
    assert_ne!(first.verification_id, second.verification_id);
    assert_eq!(second.remaining_attempts, 2);
    assert_eq!(
        f.provider
            .advance_password_login(&receipt, &callback(&first))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(matches!(
        f.provider
            .advance_password_login(&receipt, &callback(&second))
            .await
            .unwrap(),
        PasswordLoginProgress::Confirmed(_)
    ));
    let wires = f.requests.await.unwrap();
    assert!(wires[3].starts_with("POST /v3/check_mobile?"));
    assert_eq!(parts(&wires[2]).1["userid"], "synthetic-web-user");
    for index in [4, 5, 6] {
        let (_, body) = parts(&wires[index]);
        assert_eq!(body["userid"], 222);
        assert_eq!(body["force_login"], 0);
        assert_eq!(body["code"], "654321");
    }
    for index in [5, 6] {
        assert!(wires[index].contains("verifydata: synthetic+proof%2F\r\n"));
        assert!(
            !wires[index]
                .split_once("\r\n\r\n")
                .unwrap()
                .1
                .contains("synthetic+proof")
        );
    }
    for index in [0, 1, 2, 3, 4, 7] {
        assert!(!wires[index].contains("verifydata:"));
    }
}

#[tokio::test]
async fn web_secondary_resend_preserves_answer_budget_and_original_deadline() {
    let f = server(vec![
        phone(30767).into(),
        reply(json!("sent")).into(),
        rejected(20021).into(),
        reply(json!("resent")).into(),
        rejected(20021).into(),
        rejected(20021).into(),
        rejected(20021).into(),
        rejected(20021).into(),
    ])
    .await;
    let (receipt, _) = pending(
        f.provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap(),
    );
    let deadline = f.provider.qr_transactions.lock().unwrap().passwords
        [receipt.provider_transaction_id()]
    .deadline;
    assert_eq!(
        f.provider
            .advance_password_login(&receipt, &PasswordChallengeAction::ResendSms)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    assert_eq!(
        f.provider
            .advance_password_login(&receipt, &submit())
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    ready(&f.provider, &receipt);
    let (_, v) = pending(
        f.provider
            .advance_password_login(&receipt, &PasswordChallengeAction::ResendSms)
            .await
            .unwrap(),
    );
    assert!(matches!(
        v,
        PasswordVerification::Sms {
            remaining_attempts: 4,
            ..
        }
    ));
    assert_eq!(
        f.provider.qr_transactions.lock().unwrap().passwords[receipt.provider_transaction_id()]
            .deadline,
        deadline
    );
    for _ in 0..4 {
        assert!(
            f.provider
                .advance_password_login(&receipt, &submit())
                .await
                .is_err()
        );
    }
    assert!(
        f.provider
            .qr_transactions
            .lock()
            .unwrap()
            .passwords
            .is_empty()
    );
    assert!(
        f.provider
            .advance_password_login(&receipt, &submit())
            .await
            .is_err()
    );
    let wires = f.requests.await.unwrap();
    assert_eq!(parts(&wires[1]).0["mid"], parts(&wires[3]).0["mid"]);
}

#[tokio::test]
async fn web_secondary_shares_cooldown_and_retires_receipts_across_sms_entry_points() {
    let f = server(vec![
        phone(30768).into(),
        reply(json!("sent")).into(),
        reply(json!("sent-again")).into(),
        phone(30767).into(),
    ])
    .await;
    let (receipt, _) = pending(
        f.provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap(),
    );
    let sms = AuthChallengeRequest {
        account: "default".into(),
        method: ChallengeMethod::Sms,
        backend: AuthChallengeBackend::Standard,
        principal: PHONE.into(),
        country_code: Some("86".into()),
        allow_account_creation: false,
        accept_platform_policies: false,
    };
    assert_eq!(
        f.provider
            .begin_auth_challenge(&sms, CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    ready(&f.provider, &receipt);
    let ordinary = f
        .provider
        .begin_auth_challenge(&sms, CredentialMode::Client)
        .await
        .unwrap();
    assert!(
        f.provider
            .advance_password_login(&receipt, &submit())
            .await
            .is_err()
    );
    assert_eq!(
        f.provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    assert_eq!(
        f.provider.auth_challenge_status(&ordinary).await.unwrap(),
        AuthChallengeStatus::Waiting
    );
    assert_eq!(f.requests.await.unwrap().len(), 4);
}

#[tokio::test]
async fn web_secondary_late_cookie_exchange_cannot_replace_changed_account_or_survive_cancellation()
{
    for cancel in [false, true] {
        let (frame, release) = paused(exchange("111"));
        let mut f = server(vec![
            phone(30768).into(),
            reply(json!("sent")).into(),
            cookie("111").into(),
            frame,
        ])
        .await;
        let store = Arc::new(Store::default());
        let old = credential("222", "original");
        store.put(&old.stored("A").unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        let provider = Arc::new(f.provider);
        let (receipt, _) = pending(
            provider
                .begin_password_login(&input("A"), CredentialMode::Both)
                .await
                .unwrap(),
        );
        let task = {
            let provider = provider.clone();
            let receipt = receipt.clone();
            tokio::spawn(async move { provider.advance_password_login(&receipt, &submit()).await })
        };
        for _ in 0..4 {
            f.seen.recv().await.unwrap();
        }
        let replacement = credential("333", "replacement");
        if cancel {
            task.abort();
        } else {
            store.put(&replacement.stored("A").unwrap()).unwrap();
        }
        let _ = release.send(());
        if cancel {
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
        }
        assert_eq!(read(&store, "A"), if cancel { old } else { replacement });
        assert!(
            provider
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
async fn web_secondary_rejects_malformed_phone_and_never_registers_or_substitutes_sms_for_other_challenges()
 {
    for value in [
        Value::Null,
        json!(111),
        json!("138*****000"),
        json!("13800000000 private"),
        json!("+8613800000000"),
    ] {
        let f = server(vec![
            raw(json!({"status":0,"error_code":30768,"data":value})).into(),
        ])
        .await;
        let error = f
            .provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!format!("{error:?}").contains("private"));
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
    for code in [20020, 30791, 30798] {
        let f = server(vec![rejected(code).into()]).await;
        assert_eq!(
            f.provider
                .begin_password_login(&input("default"), CredentialMode::Client)
                .await
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
    let f = server(vec![
        phone(30768).into(),
        reply(json!("sent")).into(),
        rejected(30703).into(),
    ])
    .await;
    let (receipt, _) = pending(
        f.provider
            .begin_password_login(&input("default"), CredentialMode::Client)
            .await
            .unwrap(),
    );
    assert_eq!(
        f.provider
            .advance_password_login(&receipt, &submit())
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert!(
        f.provider
            .qr_transactions
            .lock()
            .unwrap()
            .passwords
            .is_empty()
    );
    let wires = f.requests.await.unwrap();
    assert_eq!(wires.len(), 3);
    assert_eq!(parts(&wires[2]).1["force_login"], 0);
}
