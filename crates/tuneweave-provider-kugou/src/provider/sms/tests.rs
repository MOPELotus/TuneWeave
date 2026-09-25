use super::*;
use crate::provider::session::tests::{Store, credential, paused, raw, read, reply, server};
use serde_json::Value;
use std::collections::BTreeMap;
use tuneweave_core::{ChallengeMethod, StoredAccountCredential};

mod browser;

const PHONE: &str = "13800000000";
const CODE: &str = "123456";
fn input(account: &str) -> AuthChallengeRequest {
    AuthChallengeRequest {
        account: account.into(),
        method: ChallengeMethod::Sms,
        backend: AuthChallengeBackend::Standard,
        principal: PHONE.into(),
        country_code: Some("86".into()),
        allow_account_creation: false,
        accept_platform_policies: false,
    }
}
fn submit() -> AuthChallengeAction {
    AuthChallengeAction::SubmitCode { code: CODE.into() }
}
fn select(uid: &str) -> AuthChallengeAction {
    AuthChallengeAction::SelectAccount {
        user_id: uid.into(),
        code: CODE.into(),
    }
}
fn cookie(uid: &str) -> String {
    reply(
        json!({"name":"KuGoo","domain":".kugou.com","path":"/","value":format!("KugooID={uid}&t=synthetic-sms-cookie&a_id=1014&NickName=Listener")}),
    )
}
fn exchange(uid: &str) -> String {
    reply(json!({})).replace("Content-Type:",&format!("Set-Cookie: KuGoo=KugooID={uid}&t=synthetic-sms-verified&a_id=1014&NickName=Verified; Domain=.kugou.com; Path=/\r\nContent-Type:"))
}
fn choice_required() -> String {
    raw(json!({"status":0,"error_code":34175,"data":"private-upstream-prose"}))
}
fn choices() -> String {
    reply(json!({"info_list":[{"userid":111,"nickname":"One"},{"userid":"222","nickname":"Two"}]}))
}
fn boundary_responses(boundary: usize) -> (Vec<String>, usize) {
    match boundary {
        3 => (vec![reply(json!("sent")), choice_required(), choices()], 2),
        4 | 5 => (
            vec![
                reply(json!("sent")),
                raw(json!({"status":0,"error_code":30703})),
                cookie("111"),
                exchange("111"),
            ],
            boundary - 2,
        ),
        6 | 7 => (
            vec![
                reply(json!("sent")),
                choice_required(),
                choices(),
                cookie("222"),
                exchange("222"),
            ],
            boundary - 3,
        ),
        _ => (
            vec![reply(json!("sent")), cookie("111"), exchange("111")],
            boundary,
        ),
    }
}
fn boundary_input(account: &str, boundary: usize) -> AuthChallengeRequest {
    let mut request = input(account);
    request.allow_account_creation = matches!(boundary, 4 | 5);
    request
}
fn boundary_action(boundary: usize) -> AuthChallengeAction {
    if boundary >= 6 {
        select("222")
    } else {
        submit()
    }
}
fn check_wire(wire: &str, path: &str) -> (BTreeMap<String, String>, Value) {
    let (head, body) = wire.split_once("\r\n\r\n").unwrap();
    let target = head.split_whitespace().nth(1).unwrap();
    assert!(head.starts_with(&format!("POST {path}?")));
    let url = url::Url::parse(&format!("http://local{target}")).unwrap();
    let mut q = url.query_pairs().into_owned().collect::<BTreeMap<_, _>>();
    let signature = q.remove("signature").unwrap();
    assert_eq!(
        signature,
        crate::signing::web_signature(
            &q.iter().map(|(k, v)| (k.as_str(), v.clone())).collect(),
            body.as_bytes()
        )
    );
    assert_eq!(signature, signature.to_ascii_uppercase());
    assert_eq!(q["appid"], "1014");
    assert_eq!(q["srcappid"], "2919");
    assert!(!target.contains(PHONE) && !target.contains(CODE));
    assert!(!head.to_lowercase().contains("cookie:"));
    assert!(
        head.to_lowercase()
            .contains("content-type: text/plain;charset=utf-8")
    );
    assert_eq!(
        head.to_lowercase()
            .contains("x-router: loginservice.kugou.com"),
        path == "/v8/send_mobile_code"
    );
    (q, serde_json::from_str(body).unwrap())
}

#[tokio::test]
async fn sms_login_requires_independent_exchange_and_preserves_all_three_ownership_modes() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let mut f = server(vec![
            reply(json!("ok")).into(),
            cookie("111").into(),
            exchange("111").into(),
        ])
        .await;
        let store = Arc::new(Store::default());
        let alias = if mode == CredentialMode::Client {
            "default"
        } else {
            "A"
        };
        let old = credential("333", "synthetic-previous");
        store.put(&old.stored(alias).unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        let receipt = f
            .provider
            .begin_auth_challenge(&input(alias), mode)
            .await
            .unwrap();
        assert_eq!(read(&store, alias), old);
        assert_eq!(
            f.provider.auth_challenge_status(&receipt).await.unwrap(),
            AuthChallengeStatus::Waiting
        );
        let AuthChallengeProgress::Confirmed(r) = f
            .provider
            .advance_auth_challenge(&receipt, &submit())
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(r.profile.user_id.as_deref(), Some("111"));
        assert_eq!(r.profile.nickname.as_deref(), Some("Verified"));
        assert_eq!(r.profile.account, alias);
        assert_eq!(r.credential.is_some(), mode.returns_to_caller());
        if mode == CredentialMode::Client {
            assert_eq!(read(&store, alias), old);
        } else {
            assert!(!read(&store, alias).same_login(&old));
        }
        if mode == CredentialMode::Both {
            assert_eq!(
                read(&store, alias),
                KugouCredential::parse_caller(r.credential.as_ref().unwrap()).unwrap()
            );
        }
        assert!(
            f.provider
                .auth_challenge_status(&receipt)
                .await
                .unwrap_err()
                .auth_challenge_consumed()
        );
        let wires = f.requests.await.unwrap();
        assert_eq!(wires.len(), 3);
        let (a, b) = check_wire(&wires[0], "/v8/send_mobile_code");
        assert_eq!(b["businessid"], 5);
        assert_eq!(b["mobile"], "13********0");
        assert!(!wires[0].contains(PHONE));
        assert_eq!(a["clienttime"], b["clienttime_ms"].to_string());
        let (c, b) = check_wire(&wires[1], "/v2/loginbyverifycode/");
        assert_eq!(a["mid"], c["mid"]);
        assert_eq!(
            c["uuid"].parse::<u64>().unwrap() / 1000,
            c["clienttime"].parse::<u64>().unwrap()
        );
        assert_eq!(b["force_login"], 0);
        assert_eq!(b["support_multi"], 1);
        assert_eq!(b["mobile"], PHONE);
        assert_eq!(b["code"], CODE);
        assert!(b["userid"].as_str().unwrap().is_empty());
        assert!(wires[2].starts_with("POST /v1/login_by_token_get?"));
        assert!(wires[2].contains("userid=111"));
        assert!(wires[2].contains("synthetic-sms-cookie"));
    }
}

#[tokio::test]
async fn sms_multi_account_flow_keeps_original_receipt_and_requires_an_offered_uid() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let mut f = server(vec![
            reply(json!("sent")).into(),
            choice_required().into(),
            choices().into(),
            cookie("222").into(),
            exchange("222").into(),
        ])
        .await;
        let store = Arc::new(Store::default());
        f.provider.credential_store = Some(store.clone());
        let alias = if mode == CredentialMode::Client {
            "default"
        } else {
            "A"
        };
        let receipt = f
            .provider
            .begin_auth_challenge(&input(alias), mode)
            .await
            .unwrap();
        let AuthChallengeProgress::Pending(status) = f
            .provider
            .advance_auth_challenge(&receipt, &submit())
            .await
            .unwrap()
        else {
            panic!()
        };
        let AuthChallengeStatus::AccountSelectionRequired { accounts } = &status else {
            panic!()
        };
        assert_eq!(
            accounts
                .iter()
                .map(|a| a.user_id.as_str())
                .collect::<Vec<_>>(),
            ["111", "222"]
        );
        assert!(store.values.lock().unwrap().is_empty());
        assert_eq!(
            f.provider.auth_challenge_status(&receipt).await.unwrap(),
            status
        );
        for action in [
            submit(),
            select("999"),
            select("0222"),
            AuthChallengeAction::RefreshImage,
        ] {
            assert_eq!(
                f.provider
                    .advance_auth_challenge(&receipt, &action)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidRequest
            );
            assert_eq!(
                f.provider.auth_challenge_status(&receipt).await.unwrap(),
                status
            );
        }
        let AuthChallengeProgress::Confirmed(r) = f
            .provider
            .advance_auth_challenge(&receipt, &select("222"))
            .await
            .unwrap()
        else {
            panic!()
        };
        assert_eq!(r.profile.user_id.as_deref(), Some("222"));
        let wires = f.requests.await.unwrap();
        assert_eq!(wires.len(), 5);
        let (a, _) = check_wire(&wires[0], "/v8/send_mobile_code");
        for (i, path) in [
            (1, "/v2/loginbyverifycode/"),
            (2, "/v3/check_mobile"),
            (3, "/v2/loginbyverifycode/"),
        ] {
            let (q, b) = check_wire(&wires[i], path);
            assert_eq!(q["mid"], a["mid"]);
            assert_eq!(b["code"], CODE);
            if i == 2 {
                assert_eq!(b["businessid"], 5);
                assert_eq!(b["query"], json!({"duration":1,"p_grade":1}));
            }
            if i == 3 {
                assert_eq!(b["userid"], 222);
                assert_eq!(b["force_login"], 0);
            }
        }
    }
}

#[tokio::test]
async fn sms_registration_is_never_implicit_and_opt_in_permits_only_one_confirmed_retry() {
    for allow in [false, true] {
        let rejected = raw(json!({"status":0,"error_code":30703,"data":PHONE}));
        let mut replies = vec![reply(json!("sent")).into(), rejected.into()];
        if allow {
            replies.extend([cookie("111").into(), exchange("111").into()]);
        }
        let f = server(replies).await;
        let mut request = input("default");
        request.allow_account_creation = allow;
        let receipt = f
            .provider
            .begin_auth_challenge(&request, CredentialMode::Client)
            .await
            .unwrap();
        let result = f.provider.advance_auth_challenge(&receipt, &submit()).await;
        if allow {
            assert!(matches!(
                result.unwrap(),
                AuthChallengeProgress::Confirmed(_)
            ));
        } else {
            let e = result.unwrap_err();
            assert_eq!(e.code, ErrorCode::PermissionDenied);
            assert!(e.auth_challenge_consumed());
            assert!(!format!("{e:?}").contains(PHONE));
        }
        let wires = f.requests.await.unwrap();
        let (_, b) = check_wire(&wires[1], "/v2/loginbyverifycode/");
        assert_eq!(b["force_login"], 0);
        if allow {
            let (_, b) = check_wire(&wires[2], "/v2/loginbyverifycode/");
            assert_eq!(b["force_login"], 1);
        } else {
            assert_eq!(wires.len(), 2);
        }
    }
    let f = server(vec![
        reply(json!("sent")).into(),
        raw(json!({"status":0,"error_code":30703})).into(),
        raw(json!({"status":0,"error_code":30703})).into(),
    ])
    .await;
    let mut request = input("default");
    request.allow_account_creation = true;
    let r = f
        .provider
        .begin_auth_challenge(&request, CredentialMode::Client)
        .await
        .unwrap();
    assert!(
        f.provider
            .advance_auth_challenge(&r, &submit())
            .await
            .unwrap_err()
            .auth_challenge_consumed()
    );
    assert_eq!(f.requests.await.unwrap().len(), 3);
}

#[tokio::test]
async fn sms_wrong_code_has_five_attempts_and_expired_or_ambiguous_errors_consume_the_receipt() {
    let mut frames = vec![reply(json!("sent")).into()];
    frames.extend((0..5).map(|_| raw(json!({"status":0,"error_code":20021,"data":CODE})).into()));
    let f = server(frames).await;
    let r = f
        .provider
        .begin_auth_challenge(&input("default"), CredentialMode::Client)
        .await
        .unwrap();
    for i in 1..=5 {
        let e = f
            .provider
            .advance_auth_challenge(&r, &submit())
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::AuthenticationRequired);
        assert_eq!(e.auth_challenge_consumed(), i == 5);
        assert!(!format!("{e:?}").contains(CODE));
    }
    assert!(
        f.provider
            .advance_auth_challenge(&r, &submit())
            .await
            .unwrap_err()
            .auth_challenge_consumed()
    );
    assert_eq!(f.requests.await.unwrap().len(), 6);
    for bad in [
        raw(json!({"status":0,"error_code":20020})),
        raw(json!({"status":0,"error_code":20028,"data":CODE})),
        cookie("111").replace("200 OK", "500 Internal Server Error"),
        cookie("111").replace("application/json", "image/jpeg"),
        cookie("111").replace(
            "Content-Type:",
            "SSA-CODE: synthetic-event\r\nContent-Type:",
        ),
        cookie("111").replace(".kugou.com", "foreign.invalid"),
    ] {
        let f = server(vec![reply(json!("sent")).into(), bad.into()]).await;
        let r = f
            .provider
            .begin_auth_challenge(&input("default"), CredentialMode::Client)
            .await
            .unwrap();
        let e = f
            .provider
            .advance_auth_challenge(&r, &submit())
            .await
            .unwrap_err();
        assert!(e.auth_challenge_consumed());
        assert!(!format!("{e:?}").contains(CODE));
        assert!(f.provider.auth_challenge_status(&r).await.is_err());
        assert_eq!(f.requests.await.unwrap().len(), 2);
    }
}

struct NoStore;
impl AccountCredentialStore for NoStore {
    fn load_platform(&self, _: Platform) -> Result<Vec<StoredAccountCredential>> {
        panic!("client read accounts")
    }
    fn put(&self, _: &StoredAccountCredential) -> Result<()> {
        panic!("client saved account")
    }
    fn remove(&self, _: Platform, _: &str) -> Result<bool> {
        panic!("client removed account")
    }
}
#[tokio::test]
async fn sms_invalid_inputs_caller_scopes_and_shared_cooldowns_reject_without_extra_sends() {
    let mut f = server(vec![reply(json!("sent")).into()]).await;
    f.provider.credential_store = Some(Arc::new(NoStore));
    let mut bad = vec![];
    for phone in ["", "bad", "+8613800000000"] {
        let mut r = input("default");
        r.principal = phone.into();
        bad.push(r);
    }
    let mut r = input("default");
    r.backend = AuthChallengeBackend::Middle;
    bad.push(r);
    let mut r = input("default");
    r.country_code = Some("1".into());
    bad.push(r);
    bad.push(input("named"));
    for request in bad {
        assert!(
            f.provider
                .begin_auth_challenge(&request, CredentialMode::Client)
                .await
                .is_err()
        );
    }
    let caller = f
        .provider
        .with_caller_credential(&credential("111", "caller-synthetic").caller().unwrap())
        .unwrap();
    assert!(
        caller
            .begin_auth_challenge(&input("default"), CredentialMode::Client)
            .await
            .is_err()
    );
    let receipt = f
        .provider
        .begin_auth_challenge(&input("default"), CredentialMode::Client)
        .await
        .unwrap();
    assert_eq!(
        f.provider
            .clone()
            .begin_auth_challenge(&input("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    assert!(f.provider.cancel_sms_login(&receipt).unwrap());
    assert_eq!(
        f.provider
            .begin_auth_challenge(&input("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    assert_eq!(f.requests.await.unwrap().len(), 1);
}

#[tokio::test]
async fn sms_every_late_success_and_error_is_bound_to_original_account_and_pending_login() {
    for boundary in 0..8 {
        for failed in [false, true] {
            for replaced in [false, true] {
                let (mut replies, point) = boundary_responses(boundary);
                replies.truncate(point + 1);
                if failed {
                    replies[point] = raw(json!({"status":0,"error_code":999,"data":PHONE}));
                }
                let (gate, release) = paused(replies.pop().unwrap());
                let mut frames = replies.into_iter().map(Into::into).collect::<Vec<_>>();
                frames.push(gate);
                let mut f = server(frames).await;
                let store = Arc::new(Store::default());
                let old = credential("333", "old-login");
                store.put(&old.stored("A").unwrap()).unwrap();
                f.provider.credential_store = Some(store.clone());
                let provider = f.provider.clone();
                let task = if point == 0 {
                    tokio::spawn(async move {
                        provider
                            .begin_auth_challenge(
                                &boundary_input("A", boundary),
                                CredentialMode::Server,
                            )
                            .await
                            .map(|_| ())
                    })
                } else {
                    let r = f
                        .provider
                        .begin_auth_challenge(
                            &boundary_input("A", boundary),
                            CredentialMode::Server,
                        )
                        .await
                        .unwrap();
                    if boundary >= 6 {
                        assert!(matches!(
                            f.provider
                                .advance_auth_challenge(&r, &submit())
                                .await
                                .unwrap(),
                            AuthChallengeProgress::Pending(_)
                        ));
                    }
                    tokio::spawn(async move {
                        provider
                            .advance_auth_challenge(&r, &boundary_action(boundary))
                            .await
                            .map(|_| ())
                    })
                };
                for _ in 0..=point {
                    f.seen.recv().await.unwrap();
                }
                if replaced {
                    store
                        .put(&credential("444", "replacement-login").stored("A").unwrap())
                        .unwrap();
                } else {
                    f.provider.logout("A").await.unwrap();
                }
                release.send(()).unwrap();
                let e = task.await.unwrap().unwrap_err();
                assert!(matches!(
                    e.code,
                    ErrorCode::Conflict | ErrorCode::AuthenticationRequired
                ));
                assert!(e.auth_challenge_consumed());
                assert!(f.provider.qr_transactions.lock().unwrap().sms.is_empty());
                if replaced {
                    assert_eq!(read(&store, "A").user_id(), "444");
                } else {
                    assert!(store.values.lock().unwrap().is_empty());
                }
                assert_eq!(f.requests.await.unwrap().len(), point + 1);
            }
        }
    }
}

#[tokio::test]
async fn sms_cancellation_and_total_deadline_stop_each_network_boundary() {
    for boundary in 0..8 {
        for cancel in [false, true] {
            let (mut replies, point) = boundary_responses(boundary);
            replies.truncate(point + 1);
            let (gate, release) = paused(replies.pop().unwrap());
            let mut frames = replies.into_iter().map(Into::into).collect::<Vec<_>>();
            frames.push(gate);
            let mut f = server(frames).await;
            f.provider.client.http = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap();
            let provider = f.provider.clone();
            let task = if point == 0 {
                tokio::spawn(async move {
                    provider
                        .begin_auth_challenge(
                            &boundary_input("default", boundary),
                            CredentialMode::Client,
                        )
                        .await
                        .map(|_| ())
                })
            } else {
                let r = f
                    .provider
                    .begin_auth_challenge(
                        &boundary_input("default", boundary),
                        CredentialMode::Client,
                    )
                    .await
                    .unwrap();
                if boundary >= 6 {
                    assert!(matches!(
                        f.provider
                            .advance_auth_challenge(&r, &submit())
                            .await
                            .unwrap(),
                        AuthChallengeProgress::Pending(_)
                    ));
                }
                tokio::spawn(async move {
                    provider
                        .advance_auth_challenge(&r, &boundary_action(boundary))
                        .await
                        .map(|_| ())
                })
            };
            for _ in 0..=point {
                f.seen.recv().await.unwrap();
            }
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                tokio::time::pause();
                let r = task.await;
                tokio::time::resume();
                let e = r.unwrap().unwrap_err();
                assert_eq!(e.code, ErrorCode::UpstreamTimeout);
                assert!(e.message.contains("total time budget"));
            }
            let _ = release.send(());
            assert_eq!(f.requests.await.unwrap().len(), point + 1);
            assert!(f.provider.qr_transactions.lock().unwrap().sms.is_empty());
            assert_eq!(
                f.provider
                    .begin_auth_challenge(
                        &boundary_input("default", boundary),
                        CredentialMode::Client
                    )
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::RateLimited
            );
        }
    }
}

#[tokio::test]
async fn sms_empty_alias_logout_and_store_failure_do_not_publish_a_login() {
    for fail_store in [false, true] {
        let (gate, release) = paused(if fail_store {
            exchange("111")
        } else {
            cookie("111")
        });
        let mut frames = vec![reply(json!("sent")).into()];
        if fail_store {
            frames.push(cookie("111").into());
        }
        frames.push(gate);
        let mut f = server(frames).await;
        let store = Arc::new(Store::default());
        f.provider.credential_store = Some(store.clone());
        let r = f
            .provider
            .begin_auth_challenge(&input("A"), CredentialMode::Both)
            .await
            .unwrap();
        let provider = f.provider.clone();
        let task =
            tokio::spawn(async move { provider.advance_auth_challenge(&r, &submit()).await });
        for _ in 0..if fail_store { 3 } else { 2 } {
            f.seen.recv().await.unwrap();
        }
        if fail_store {
            store.fail.store(true, std::sync::atomic::Ordering::SeqCst);
        } else {
            assert!(!f.provider.logout("A").await.unwrap());
        }
        release.send(()).unwrap();
        let e = task.await.unwrap().unwrap_err();
        assert!(e.auth_challenge_consumed());
        assert!(store.values.lock().unwrap().is_empty());
        assert!(f.provider.qr_transactions.lock().unwrap().sms.is_empty());
        f.requests.await.unwrap();
    }
}

#[tokio::test]
async fn sms_selected_and_exchanged_identities_must_match_before_any_credential_is_published() {
    for exchanged_mismatch in [false, true] {
        let mut frames = vec![
            reply(json!("sent")).into(),
            choice_required().into(),
            choices().into(),
        ];
        frames.push(cookie(if exchanged_mismatch { "222" } else { "111" }).into());
        if exchanged_mismatch {
            frames.push(exchange("333").into());
        }
        let mut f = server(frames).await;
        let store = Arc::new(Store::default());
        let old = credential("444", "previous-identity");
        store.put(&old.stored("A").unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        let r = f
            .provider
            .begin_auth_challenge(&input("A"), CredentialMode::Both)
            .await
            .unwrap();
        assert!(matches!(
            f.provider
                .advance_auth_challenge(&r, &submit())
                .await
                .unwrap(),
            AuthChallengeProgress::Pending(_)
        ));
        let e = f
            .provider
            .advance_auth_challenge(&r, &select("222"))
            .await
            .unwrap_err();
        assert!(e.auth_challenge_consumed());
        assert_eq!(read(&store, "A"), old);
        assert!(f.provider.auth_challenge_status(&r).await.is_err());
        assert_eq!(
            f.requests.await.unwrap().len(),
            if exchanged_mismatch { 5 } else { 4 }
        );
    }
}

#[tokio::test]
async fn sms_different_accounts_can_finish_in_reverse_order_with_separate_devices_and_credentials()
{
    let (gate, release) = paused(exchange("111"));
    let mut a = server(vec![
        reply(json!("sent")).into(),
        cookie("111").into(),
        gate,
    ])
    .await;
    let mut b = server(vec![
        reply(json!("sent")).into(),
        cookie("222").into(),
        exchange("222").into(),
    ])
    .await;
    let store = Arc::new(Store::default());
    a.provider.credential_store = Some(store.clone());
    b.provider.credential_store = Some(store.clone());
    b.provider.qr_transactions = a.provider.qr_transactions.clone();
    let mut input_b = input("B");
    input_b.principal = "13800000001".into();
    let receipt_a = a
        .provider
        .begin_auth_challenge(&input("A"), CredentialMode::Both)
        .await
        .unwrap();
    let receipt_b = b
        .provider
        .begin_auth_challenge(&input_b, CredentialMode::Both)
        .await
        .unwrap();
    let provider_a = a.provider.clone();
    let task_a = tokio::spawn(async move {
        provider_a
            .advance_auth_challenge(&receipt_a, &submit())
            .await
    });
    for _ in 0..3 {
        a.seen.recv().await.unwrap();
    }
    let AuthChallengeProgress::Confirmed(result_b) = b
        .provider
        .advance_auth_challenge(&receipt_b, &submit())
        .await
        .unwrap()
    else {
        panic!()
    };
    assert_eq!(read(&store, "B").user_id(), "222");
    assert!(!store.values.lock().unwrap().contains_key("A"));
    release.send(()).unwrap();
    let AuthChallengeProgress::Confirmed(result_a) = task_a.await.unwrap().unwrap() else {
        panic!()
    };
    for (account, result, uid) in [("A", result_a, "111"), ("B", result_b, "222")] {
        assert_eq!(result.profile.account, account);
        assert_eq!(result.profile.user_id.as_deref(), Some(uid));
        assert_eq!(
            read(&store, account),
            KugouCredential::parse_caller(result.credential.as_ref().unwrap()).unwrap()
        );
    }
    let a_wire = a.requests.await.unwrap();
    let b_wire = b.requests.await.unwrap();
    let mid = |wire: &str| {
        url::Url::parse(&format!(
            "http://local{}",
            wire.split_whitespace().nth(1).unwrap()
        ))
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "mid")
        .unwrap()
        .1
        .into_owned()
    };
    assert_ne!(mid(&a_wire[0]), mid(&b_wire[0]));
    assert_eq!(mid(&a_wire[0]), mid(&a_wire[1]));
    assert_eq!(mid(&b_wire[0]), mid(&b_wire[1]));
    assert!(b_wire[1].contains("13800000001"));
    assert!(a.provider.qr_transactions.lock().unwrap().sms.is_empty());
}

#[tokio::test]
async fn sms_receipt_ownership_capacity_ttl_and_new_delivery_keep_the_original_limits() {
    let f = server(vec![reply(json!("sent")).into()]).await;
    let r = f
        .provider
        .begin_auth_challenge(&input("default"), CredentialMode::Client)
        .await
        .unwrap();
    for change in 0..4 {
        let mut request = input("default");
        match change {
            0 => request.principal = "13800000001".into(),
            1 => request.allow_account_creation = true,
            2 => request.country_code = Some("+86".into()),
            _ => request.backend = AuthChallengeBackend::Middle,
        }
        let forged = ProviderAuthChallenge::stateful(
            Platform::Kugou,
            request,
            CredentialMode::Client,
            r.provider_transaction_id().unwrap().into(),
        )
        .unwrap();
        assert!(
            f.provider
                .advance_auth_challenge(&forged, &submit())
                .await
                .is_err()
        );
        assert_eq!(
            f.provider.auth_challenge_status(&r).await.unwrap(),
            AuthChallengeStatus::Waiting
        );
    }
    {
        let mut registry = f.provider.qr_transactions.lock().unwrap();
        for index in 0..127 {
            registry.sms.insert(
                format!("reserved-{index}"),
                Entry {
                    challenge: r.clone(),
                    deadline: Instant::now() + Duration::from_secs(300),
                    previous: None,
                    context: None,
                },
            );
        }
    }
    let mut other = input("default");
    other.principal = "13800000001".into();
    assert_eq!(
        f.provider
            .begin_auth_challenge(&other, CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    f.provider
        .qr_transactions
        .lock()
        .unwrap()
        .sms
        .retain(|id, _| id == r.provider_transaction_id().unwrap());
    assert_eq!(f.requests.await.unwrap().len(), 1);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    let mut second = server(vec![reply(json!("sent")).into()]).await;
    second.provider.qr_transactions = f.provider.qr_transactions.clone();
    let newer = second
        .provider
        .begin_auth_challenge(&input("default"), CredentialMode::Client)
        .await
        .unwrap();
    assert!(
        f.provider
            .auth_challenge_status(&r)
            .await
            .unwrap_err()
            .auth_challenge_consumed()
    );
    assert_eq!(second.requests.await.unwrap().len(), 1);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(301)).await;
    tokio::time::resume();
    assert!(
        f.provider
            .advance_auth_challenge(&newer, &submit())
            .await
            .unwrap_err()
            .auth_challenge_consumed()
    );
    assert!(f.provider.qr_transactions.lock().unwrap().sms.is_empty());
}

#[tokio::test]
async fn sms_failed_sends_keep_cooldown_and_wire_parsing_rejects_ambiguous_or_oversized_data() {
    let f = server(vec![raw(json!({"status":0,"error_code":20028})).into()]).await;
    assert!(
        f.provider
            .begin_auth_challenge(&input("default"), CredentialMode::Client)
            .await
            .is_err()
    );
    assert!(f.provider.qr_transactions.lock().unwrap().sms.is_empty());
    assert_eq!(
        f.provider
            .begin_auth_challenge(&input("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    assert_eq!(f.requests.await.unwrap().len(), 1);
    for bad in [
        // Data stays RawValue until typed decoding: duplicate cookie fields must not collapse.
        r#"{"status":1,"error_code":0,"data":{"name":"KuGoo","domain":".kugou.com","path":"/","value":"KugooID=111&t=one","value":"KugooID=222&t=two"}}"#.to_owned(),
        json!({"status":1,"error_code":0,"data":"x".repeat(1024 * 1024)}).to_string(),
    ] {
        let wire = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain;charset=UTF-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{bad}", bad.len());
        let f = server(vec![reply(json!("sent")).into(), wire.into()]).await;
        let receipt = f.provider.begin_auth_challenge(&input("default"), CredentialMode::Client).await.unwrap();
        assert!(f.provider.advance_auth_challenge(&receipt, &submit()).await.unwrap_err().auth_challenge_consumed());
        assert_eq!(f.requests.await.unwrap().len(), 2);
    }
    for mime in ["text/plain;charset=UTF-8", "text/html; charset=utf-8"] {
        let f = server(vec![
            reply(json!("sent"))
                .replace("application/json", mime)
                .into(),
            cookie("111").replace("application/json", mime).into(),
            exchange("111").into(),
        ])
        .await;
        let receipt = f
            .provider
            .begin_auth_challenge(&input("default"), CredentialMode::Client)
            .await
            .unwrap();
        assert!(matches!(
            f.provider
                .advance_auth_challenge(&receipt, &submit())
                .await
                .unwrap(),
            AuthChallengeProgress::Confirmed(_)
        ));
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn sms_choice_retries_share_code_attempts_and_never_return_an_unusable_pending_state() {
    for exhausted in [false, true] {
        let wrong = raw(json!({"status":0,"error_code":20021}));
        let mut frames = vec![reply(json!("sent")).into()];
        if exhausted {
            frames.extend((0..4).map(|_| wrong.clone().into()));
        }
        frames.extend([choice_required().into(), choices().into()]);
        if !exhausted {
            frames.extend([wrong.into(), cookie("222").into(), exchange("222").into()]);
        }
        let f = server(frames).await;
        let receipt = f
            .provider
            .begin_auth_challenge(&input("default"), CredentialMode::Client)
            .await
            .unwrap();
        if exhausted {
            for _ in 0..4 {
                assert!(
                    !f.provider
                        .advance_auth_challenge(&receipt, &submit())
                        .await
                        .unwrap_err()
                        .auth_challenge_consumed()
                );
            }
            assert!(
                f.provider
                    .advance_auth_challenge(&receipt, &submit())
                    .await
                    .unwrap_err()
                    .auth_challenge_consumed()
            );
            assert!(f.provider.auth_challenge_status(&receipt).await.is_err());
            assert_eq!(f.requests.await.unwrap().len(), 7);
        } else {
            // The legacy method keeps a pending selection usable through the newer API.
            let error = f
                .provider
                .complete_auth_challenge(&receipt, CODE)
                .await
                .unwrap_err();
            assert!(!error.auth_challenge_consumed());
            let pending = f.provider.auth_challenge_status(&receipt).await.unwrap();
            assert!(matches!(
                pending,
                AuthChallengeStatus::AccountSelectionRequired { .. }
            ));
            assert!(
                !f.provider
                    .advance_auth_challenge(&receipt, &select("222"))
                    .await
                    .unwrap_err()
                    .auth_challenge_consumed()
            );
            assert_eq!(
                f.provider.auth_challenge_status(&receipt).await.unwrap(),
                pending
            );
            assert!(matches!(
                f.provider
                    .advance_auth_challenge(&receipt, &select("222"))
                    .await
                    .unwrap(),
                AuthChallengeProgress::Confirmed(_)
            ));
            assert_eq!(f.requests.await.unwrap().len(), 6);
        }
    }
}

#[tokio::test]
async fn sms_upstream_delivery_retry_after_extends_the_shared_phone_cooldown() {
    let rejected = reply(json!({}))
        .replace("200 OK", "429 Too Many Requests")
        .replace("Content-Type:", "Retry-After: 180\r\nContent-Type:");
    let f = server(vec![rejected.into()]).await;
    let error = f
        .provider
        .begin_auth_challenge(&input("default"), CredentialMode::Client)
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::RateLimited);
    assert_eq!(error.details["retry_after_secs"], 180);
    assert_eq!(f.requests.await.unwrap().len(), 1);
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(61)).await;
    tokio::time::resume();
    assert_eq!(
        f.provider
            .clone()
            .begin_auth_challenge(&input("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(121)).await;
    tokio::time::resume();
    let mut next = server(vec![reply(json!("sent")).into()]).await;
    next.provider.qr_transactions = f.provider.qr_transactions.clone();
    let receipt = next
        .provider
        .begin_auth_challenge(&input("default"), CredentialMode::Client)
        .await
        .unwrap();
    assert!(next.provider.cancel_sms_login(&receipt).unwrap());
    assert_eq!(next.requests.await.unwrap().len(), 1);
}

#[tokio::test]
async fn sms_account_storage_read_failure_consumes_the_receipt_at_status_and_network_boundaries() {
    struct FailingRead {
        store: Arc<Store>,
        fail: Arc<std::sync::atomic::AtomicBool>,
    }
    impl AccountCredentialStore for FailingRead {
        fn load_platform(&self, platform: Platform) -> Result<Vec<StoredAccountCredential>> {
            if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
                Err(state_error())
            } else {
                self.store.load_platform(platform)
            }
        }
        fn put(&self, value: &StoredAccountCredential) -> Result<()> {
            self.store.put(value)
        }
        fn remove(&self, platform: Platform, account: &str) -> Result<bool> {
            self.store.remove(platform, account)
        }
    }
    for boundary in 0..3 {
        let mut frames = vec![reply(json!("sent")).into()];
        let release = if boundary > 0 {
            if boundary == 2 {
                frames.push(cookie("111").into());
            }
            let (gate, release) = paused(if boundary == 2 {
                exchange("111")
            } else {
                cookie("111")
            });
            frames.push(gate);
            Some(release)
        } else {
            None
        };
        let mut f = server(frames).await;
        let store = Arc::new(Store::default());
        let old = credential("333", "previous-unmodified");
        store.put(&old.stored("A").unwrap()).unwrap();
        let fail = Arc::new(std::sync::atomic::AtomicBool::new(false));
        f.provider.credential_store = Some(Arc::new(FailingRead {
            store: store.clone(),
            fail: fail.clone(),
        }));
        let receipt = f
            .provider
            .begin_auth_challenge(&input("A"), CredentialMode::Both)
            .await
            .unwrap();
        let error = if let Some(release) = release {
            let provider = f.provider.clone();
            let r = receipt.clone();
            let task =
                tokio::spawn(async move { provider.advance_auth_challenge(&r, &submit()).await });
            for _ in 0..=boundary {
                f.seen.recv().await.unwrap();
            }
            fail.store(true, std::sync::atomic::Ordering::SeqCst);
            release.send(()).unwrap();
            task.await.unwrap().unwrap_err()
        } else {
            fail.store(true, std::sync::atomic::Ordering::SeqCst);
            f.provider
                .auth_challenge_status(&receipt)
                .await
                .unwrap_err()
        };
        assert!(error.auth_challenge_consumed());
        assert_eq!(read(&store, "A"), old);
        assert!(f.provider.qr_transactions.lock().unwrap().sms.is_empty());
        fail.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(
            f.provider
                .advance_auth_challenge(&receipt, &submit())
                .await
                .unwrap_err()
                .auth_challenge_consumed()
        );
        assert_eq!(f.requests.await.unwrap().len(), boundary + 1);
    }
}
