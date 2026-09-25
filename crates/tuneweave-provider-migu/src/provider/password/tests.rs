use super::*;
use crate::{
    client::passport::{
        image::tests::image_body,
        tests::{exchange, key, login, request, response, server},
    },
    credential::MiguCredential,
    provider::session::tests::{Store, gated, profile, read, stored},
};
use std::collections::BTreeMap;

mod voice;

fn ok() -> String {
    response(json!({"status":2000}), "")
}
fn graph(chinese: bool) -> String {
    response(
        image_body(chinese),
        "Set-Cookie: mgnd_session_last_access=graph; Path=/\r\n",
    )
}
fn extra(code: u16) -> String {
    response(
        json!({"status":code,"result":{"msisdnRSA":"opaque+/=?%ticket","msisdnHide":"138****8000"}}),
        "Set-Cookie: mgnd_session_id=extra; Domain=.migu.cn; Path=/\r\n",
    )
}
fn image_start(chinese: bool) -> Vec<String> {
    vec![
        key(0),
        extra(if chinese { 4045 } else { 4044 }),
        graph(chinese),
    ]
}
fn secondary_start() -> Vec<String> {
    vec![key(0), extra(6103), key(1), ok()]
}
fn image_to_secondary() -> Vec<String> {
    vec![ok(), key(1), extra(6103), key(0), ok()]
}
fn confirmed() -> Vec<String> {
    vec![
        key(1),
        login(),
        exchange("222"),
        profile("222", "pacmtoken: final-profile\r\n"),
    ]
}
fn answer(chinese: bool) -> PasswordChallengeAction {
    PasswordChallengeAction::SubmitImage {
        answer: if chinese { "汉字" } else { "42" }.into(),
        password: "resubmitted-secret".into(),
    }
}
fn seeded(provider: &mut MiguProvider) -> (Arc<Store>, MiguCredential) {
    let store = Arc::new(Store::default());
    let old = MiguCredential::verified("111".into(), "old-token".into()).unwrap();
    for account in ["A", "B"] {
        store.put(&stored(account, &old)).unwrap();
    }
    provider.credential_store = Some(store.clone());
    (store, old)
}
fn pending(result: PasswordLoginProgress) -> (ProviderPasswordChallenge, PasswordVerification) {
    let PasswordLoginProgress::Pending {
        challenge,
        verification,
    } = result
    else {
        panic!("Expected pending password verification")
    };
    (challenge, verification)
}
fn assert_pending_image(verification: PasswordVerification, chinese: bool, remaining: u8) {
    let PasswordVerification::Image { image } = verification else {
        panic!("Expected image")
    };
    assert_eq!(
        image.answer_kind,
        if chinese {
            AuthImageAnswerKind::Chinese
        } else {
            AuthImageAnswerKind::Arithmetic
        }
    );
    assert_eq!(image.remaining_attempts, remaining);
}

#[tokio::test]
async fn ordinary_begin_password_preserves_single_step_result_for_all_modes() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let (client, requests) =
            server(vec![key(0), login(), exchange("222"), profile("222", "")]).await;
        let mut provider = MiguProvider::from_client(client);
        let (store, old) = seeded(&mut provider);
        let account = if mode == CredentialMode::Client {
            "default"
        } else {
            "A"
        };
        let PasswordLoginProgress::Confirmed(result) = provider
            .begin_password_login(&request(account), mode)
            .await
            .unwrap()
        else {
            panic!("Expected confirmation")
        };
        assert_eq!(result.profile.account, account);
        assert_eq!(result.credential.is_some(), mode.returns_to_caller());
        assert_eq!(read(&store, "B"), old);
        assert!(
            provider
                .passport_transactions
                .lock()
                .unwrap()
                .passwords
                .is_empty()
        );
        requests.await.unwrap();
    }
}

#[tokio::test]
async fn password_image_and_secondary_sms_use_original_cookies_exact_fields_and_bound_ownership() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let (client, requests) =
            server([image_start(true), image_to_secondary(), confirmed()].concat()).await;
        let mut provider = MiguProvider::from_client(client);
        let (store, old) = seeded(&mut provider);
        let account = if mode == CredentialMode::Client {
            "default"
        } else {
            "A"
        };
        let (receipt, verification) = pending(
            provider
                .begin_password_login(&request(account), mode)
                .await
                .unwrap(),
        );
        assert_pending_image(verification, true, 5);
        let original_identity = receipt.identity().clone();
        let expiry = provider.passport_transactions.lock().unwrap().passwords
            [receipt.provider_transaction_id()]
        .expires_at;
        assert_eq!(read(&store, "A"), old);
        let (next, verification) = pending(
            provider
                .advance_password_login(&receipt, &answer(true))
                .await
                .unwrap(),
        );
        assert_eq!(next, receipt);
        assert_eq!(next.identity(), &original_identity);
        let PasswordVerification::Sms {
            masked_destination,
            remaining_attempts,
            resend_after_secs,
        } = verification
        else {
            panic!("Expected secondary SMS")
        };
        assert_eq!(masked_destination, "138****8000");
        assert_eq!(remaining_attempts, 5);
        assert!((1..=60).contains(&resend_after_secs));
        assert_eq!(
            provider.passport_transactions.lock().unwrap().passwords
                [receipt.provider_transaction_id()]
            .expires_at,
            expiry
        );
        assert_eq!(read(&store, "A"), old);
        let PasswordLoginProgress::Confirmed(result) = provider
            .advance_password_login(
                &receipt,
                &PasswordChallengeAction::SubmitSms {
                    code: "1234".into(),
                },
            )
            .await
            .unwrap()
        else {
            panic!("Expected confirmation")
        };
        assert_eq!(result.profile.account, account);
        assert_eq!(result.credential.is_some(), mode.returns_to_caller());
        if mode.persists_on_server() {
            assert_eq!(read(&store, "A").user_id(), "222");
        } else {
            assert_eq!(read(&store, "A"), old);
        }
        assert_eq!(read(&store, "B"), old);
        assert!(
            provider
                .passport_transactions
                .lock()
                .unwrap()
                .passwords
                .is_empty()
        );
        assert_eq!(
            provider
                .advance_password_login(&receipt, &answer(true))
                .await
                .unwrap_err()
                .code,
            ErrorCode::ResourceNotFound
        );
        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 12);
        assert!(
            requests[2]
                .starts_with("POST /captcha/graph/risk?imgcodeType=1&showType=1&sourceid=220029 ")
        );
        assert!(requests[3].starts_with("POST /captcha/graph/check "));
        assert!(requests[3].contains("mgnd_session_id=extra"));
        assert!(requests[4].contains("mgnd_session_last_access=graph"));
        let fields =
            url::form_urlencoded::parse(requests[3].split_once("\r\n\r\n").unwrap().1.as_bytes())
                .collect::<BTreeMap<_, _>>();
        assert_eq!(fields.len(), 4);
        assert_eq!(fields["imgcodeType"], "1");
        assert_eq!(fields["captcha"], "汉字");
        let fields =
            url::form_urlencoded::parse(requests[5].split_once("\r\n\r\n").unwrap().1.as_bytes())
                .collect::<BTreeMap<_, _>>();
        assert_eq!(fields.len(), 10);
        assert_eq!(fields["imgcodeType"], "1");
        assert_eq!(fields["captcha"], "汉字");
        assert_eq!(
            crate::passport::rsa::tests::decrypt(1, &fields["enpassword"]),
            b"resubmitted-secret"
        );
        assert_eq!(
            crate::passport::rsa::tests::decrypt(1, &fields["loginID"]),
            b"test-listener"
        );
        assert!(requests[7].starts_with("GET /login/second/password?"));
        let query = requests[7]
            .lines()
            .next()
            .unwrap()
            .split_once('?')
            .unwrap()
            .1
            .split_whitespace()
            .next()
            .unwrap();
        let fields = url::form_urlencoded::parse(query.as_bytes()).collect::<BTreeMap<_, _>>();
        assert_eq!(fields.len(), 5);
        assert_eq!(fields["msisdn"], "opaque+/=?%ticket");
        assert_eq!(fields["sourceID"], "220029");
        assert_eq!(fields["fingerPrint"], "");
        assert_eq!(fields["fingerPrintDetail"], "");
        assert!(requests[9].starts_with("POST /authn/second/dynamicpassword "));
        let fields =
            url::form_urlencoded::parse(requests[9].split_once("\r\n\r\n").unwrap().1.as_bytes())
                .collect::<BTreeMap<_, _>>();
        assert_eq!(fields.len(), 6);
        assert_eq!(fields["msisdn"], "opaque+/=?%ticket");
        assert_eq!(fields["appType"], "0");
        assert_eq!(fields["relayState"], "");
        assert_eq!(
            crate::passport::rsa::tests::decrypt(1, &fields["secondPassword"]),
            b"1234"
        );
        assert!(!fields.contains_key("captchaId"));
        assert!(!requests[10].contains("cookie:") && !requests[11].contains("cookie:"));
        assert!(
            requests
                .iter()
                .all(|v| !v.contains("138****8000") && !v.contains("resubmitted-secret"))
        );
    }
}

#[tokio::test]
async fn password_graph_can_finish_directly_and_generic_rejection_preserves_chinese_type() {
    let (client, requests) = server(
        [
            image_start(true),
            vec![
                extra(4002),
                graph(true),
                ok(),
                key(1),
                login(),
                exchange("222"),
                profile("222", ""),
            ],
        ]
        .concat(),
    )
    .await;
    let provider = MiguProvider::from_client(client);
    let (receipt, _) = pending(
        provider
            .begin_password_login(&request("default"), CredentialMode::Client)
            .await
            .unwrap(),
    );
    let (same, verification) = pending(
        provider
            .advance_password_login(&receipt, &answer(true))
            .await
            .unwrap(),
    );
    assert_eq!(same, receipt);
    assert_pending_image(verification, true, 4);
    assert!(matches!(
        provider
            .advance_password_login(&receipt, &answer(true))
            .await
            .unwrap(),
        PasswordLoginProgress::Confirmed(_)
    ));
    let requests = requests.await.unwrap();
    assert!(requests[4].contains("showType=1"));
}

#[tokio::test]
async fn password_graph_attempts_refreshes_and_invalid_actions_do_not_spend_sms_budget() {
    let mut replies = image_start(false);
    for _ in 0..ATTEMPTS {
        replies.push(graph(false));
    }
    for n in 1..=ATTEMPTS {
        replies.push(extra(4002));
        if n < ATTEMPTS {
            replies.push(graph(false));
        }
    }
    let (client, requests) = server(replies).await;
    let provider = MiguProvider::from_client(client);
    let (receipt, _) = pending(
        provider
            .begin_password_login(&request("default"), CredentialMode::Client)
            .await
            .unwrap(),
    );
    for action in [
        PasswordChallengeAction::SubmitImage {
            answer: "wrong".into(),
            password: "secret".into(),
        },
        PasswordChallengeAction::SubmitImage {
            answer: "42".into(),
            password: "".into(),
        },
        PasswordChallengeAction::SubmitSms {
            code: "123456".into(),
        },
        PasswordChallengeAction::ResendSms,
    ] {
        assert_eq!(
            provider
                .advance_password_login(&receipt, &action)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        provider
            .advance_password_login(&receipt, &PasswordChallengeAction::RefreshImage)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    for _ in 0..ATTEMPTS {
        provider
            .passport_transactions
            .lock()
            .unwrap()
            .passwords
            .get_mut(receipt.provider_transaction_id())
            .unwrap()
            .context
            .as_mut()
            .unwrap()
            .next_image_at = Instant::now();
        provider
            .advance_password_login(&receipt, &PasswordChallengeAction::RefreshImage)
            .await
            .unwrap();
    }
    assert_eq!(
        provider
            .advance_password_login(&receipt, &PasswordChallengeAction::RefreshImage)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    for n in 1..=ATTEMPTS {
        let result = provider
            .advance_password_login(&receipt, &answer(false))
            .await;
        if n == ATTEMPTS {
            assert!(result.unwrap_err().auth_challenge_consumed());
        } else {
            assert_pending_image(pending(result.unwrap()).1, false, ATTEMPTS - n);
        }
    }
    assert!(
        provider
            .passport_transactions
            .lock()
            .unwrap()
            .passwords
            .is_empty()
    );
    requests.await.unwrap();
}

#[tokio::test]
async fn secondary_sms_resends_and_wrong_codes_are_bounded_and_keep_rotated_cookies() {
    let mut replies = secondary_start();
    for _ in 1..SECONDARY_SENDS {
        replies.extend([key(0), ok()]);
    }
    for _ in 0..ATTEMPTS {
        replies.extend([
            key(1),
            response(
                json!({"status":4005}),
                "Set-Cookie: mgnd_session_id=rejected; Domain=.migu.cn; Path=/\r\n",
            ),
        ]);
    }
    let (client, requests) = server(replies).await;
    let provider = MiguProvider::from_client(client);
    let (receipt, _) = pending(
        provider
            .begin_password_login(&request("default"), CredentialMode::Client)
            .await
            .unwrap(),
    );
    assert_eq!(
        provider
            .advance_password_login(&receipt, &PasswordChallengeAction::ResendSms)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    for _ in 1..SECONDARY_SENDS {
        {
            let mut state = provider.passport_transactions.lock().unwrap();
            state.secondary_cooldowns.clear();
            state
                .passwords
                .get_mut(receipt.provider_transaction_id())
                .unwrap()
                .context
                .as_mut()
                .unwrap()
                .next_secondary_at = Instant::now();
        }
        let (same, _) = pending(
            provider
                .advance_password_login(&receipt, &PasswordChallengeAction::ResendSms)
                .await
                .unwrap(),
        );
        assert_eq!(same, receipt);
    }
    assert_eq!(
        provider
            .advance_password_login(&receipt, &PasswordChallengeAction::ResendSms)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    for code in ["", "12345", "１２３４"] {
        assert_eq!(
            provider
                .advance_password_login(
                    &receipt,
                    &PasswordChallengeAction::SubmitSms { code: code.into() }
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for n in 1..=ATTEMPTS {
        let e = provider
            .advance_password_login(
                &receipt,
                &PasswordChallengeAction::SubmitSms {
                    code: "0000".into(),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::AuthenticationRequired);
        assert_eq!(e.details["remaining_attempts"], ATTEMPTS - n);
        assert_eq!(e.auth_challenge_consumed(), n == ATTEMPTS);
    }
    assert!(
        provider
            .passport_transactions
            .lock()
            .unwrap()
            .passwords
            .is_empty()
    );
    let requests = requests.await.unwrap();
    assert!(requests[10].contains("mgnd_session_id=rejected"));
}

#[tokio::test]
async fn malformed_secondary_identity_unknown_steps_and_failed_send_never_publish_a_transaction() {
    for (code, identity) in [
        (6103, json!({})),
        (
            6103,
            json!({"msisdnRSA":"ticket","msisdnHide":"13800138000"}),
        ),
        (
            6103,
            json!({"msisdnRSA":"ticket\n","msisdnHide":"138****8000"}),
        ),
        (6123, json!({})),
        (6119, json!({})),
        (6118, json!({})),
        (4049, json!({})),
    ] {
        let (client, requests) = server(vec![
            key(0),
            response(json!({"status":code,"result":identity}), ""),
        ])
        .await;
        let mut provider = MiguProvider::from_client(client);
        let (store, old) = seeded(&mut provider);
        let e = provider
            .begin_password_login(&request("A"), CredentialMode::Both)
            .await
            .unwrap_err();
        for secret in ["13800138000", "ticket"] {
            assert!(!e.to_string().contains(secret));
        }
        assert!(
            provider
                .passport_transactions
                .lock()
                .unwrap()
                .passwords
                .is_empty()
        );
        assert_eq!(read(&store, "A"), old);
        requests.await.unwrap();
    }
    let (client, requests) = server(vec![
        key(0),
        extra(6103),
        key(1),
        extra(5000),
        key(0),
        extra(6103),
    ])
    .await;
    let provider = MiguProvider::from_client(client);
    assert_eq!(
        provider
            .begin_password_login(&request("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(
        provider
            .begin_password_login(&request("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    assert!(
        provider
            .passport_transactions
            .lock()
            .unwrap()
            .passwords
            .is_empty()
    );
    requests.await.unwrap();
}

#[tokio::test]
async fn password_receipts_cannot_change_identity_ownership_or_platform() {
    let (client, requests) = server(image_start(false)).await;
    let mut provider = MiguProvider::from_client(client);
    let (store, old) = seeded(&mut provider);
    let (receipt, _) = pending(
        provider
            .begin_password_login(&request("A"), CredentialMode::Both)
            .await
            .unwrap(),
    );
    for variant in 0..5 {
        let mut identity = receipt.identity().clone();
        let mut mode = receipt.credential_mode();
        let mut platform = Platform::Migu;
        match variant {
            0 => identity.account = "B".into(),
            1 => identity.principal = "other".into(),
            2 => identity.country_code = Some("+86".into()),
            3 => mode = CredentialMode::Server,
            _ => platform = Platform::Qq,
        }
        let forged = ProviderPasswordChallenge::new(
            platform,
            identity,
            mode,
            receipt.provider_transaction_id().into(),
        )
        .unwrap();
        assert_eq!(
            provider
                .advance_password_login(&forged, &answer(false))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let caller = provider.caller_scope(&old.caller().unwrap()).unwrap();
    assert_eq!(
        caller
            .advance_password_login(&receipt, &answer(false))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(read(&store, "A"), old);
    assert_eq!(
        provider
            .passport_transactions
            .lock()
            .unwrap()
            .passwords
            .len(),
        1
    );
    requests.await.unwrap();
}

#[tokio::test]
async fn password_capacity_is_shared_with_sms_and_released_when_expired() {
    let (client, requests) = server(image_start(false)).await;
    let provider = MiguProvider::from_client(client);
    {
        let mut state = provider.passport_transactions.lock().unwrap();
        for n in 0..CAPACITY {
            let now = Instant::now();
            let id = n.to_string();
            let receipt = ProviderPasswordChallenge::new(
                Platform::Migu,
                PasswordLoginIdentity::from(&request("default")),
                CredentialMode::Client,
                id.clone(),
            )
            .unwrap();
            state.passwords.insert(
                id,
                Entry {
                    receipt,
                    created_at: now,
                    expires_at: now + TTL,
                    context: None,
                },
            );
        }
    }
    assert_eq!(
        provider
            .begin_password_login(&request("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    assert_eq!(
        provider
            .begin_auth_challenge(
                &tuneweave_core::AuthChallengeRequest {
                    allow_account_creation: false,
                    accept_platform_policies: false,
                    account: "default".into(),
                    method: tuneweave_core::ChallengeMethod::Sms,
                    backend: tuneweave_core::AuthChallengeBackend::Standard,
                    principal: "13800138000".into(),
                    country_code: None
                },
                CredentialMode::Client
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    for entry in provider
        .passport_transactions
        .lock()
        .unwrap()
        .passwords
        .values_mut()
    {
        entry.expires_at = Instant::now();
    }
    provider
        .begin_password_login(&request("default"), CredentialMode::Client)
        .await
        .unwrap();
    requests.await.unwrap();
}

#[tokio::test]
async fn password_challenge_expiry_timeout_and_replaced_alias_stop_every_network_boundary() {
    for condition in ["expire", "replace", "timeout"] {
        for stage in 1..=12 {
            let replies = [image_start(true), image_to_secondary(), confirmed()].concat();
            let (mut provider, seen, release, server) = gated(replies[..stage].to_vec()).await;
            provider.client = provider
                .client
                .with_session_test_timeout(Duration::from_millis(300));
            let (store, old) = seeded(&mut provider);
            let provider = Arc::new(provider);
            let receipt = if stage > 3 {
                Some(
                    pending(
                        provider
                            .begin_password_login(&request("A"), CredentialMode::Both)
                            .await
                            .unwrap(),
                    )
                    .0,
                )
            } else {
                None
            };
            if stage > 8 {
                provider
                    .advance_password_login(receipt.as_ref().unwrap(), &answer(true))
                    .await
                    .unwrap();
            }
            let worker = provider.clone();
            let task = tokio::spawn(async move {
                if stage <= 3 {
                    worker
                        .begin_password_login(&request("A"), CredentialMode::Both)
                        .await
                } else if stage <= 8 {
                    worker
                        .advance_password_login(receipt.as_ref().unwrap(), &answer(true))
                        .await
                } else {
                    worker
                        .advance_password_login(
                            receipt.as_ref().unwrap(),
                            &PasswordChallengeAction::SubmitSms {
                                code: "123456".into(),
                            },
                        )
                        .await
                }
            });
            tokio::time::timeout(Duration::from_secs(3), seen)
                .await
                .unwrap()
                .unwrap();
            let expected = if condition == "replace" {
                let new = MiguCredential::verified("333".into(), "replacement".into()).unwrap();
                store.put(&stored("A", &new)).unwrap();
                new
            } else {
                old
            };
            let e = if condition == "timeout" {
                let e = task.await.unwrap().unwrap_err();
                server.abort();
                drop(release);
                e
            } else {
                if condition == "expire" {
                    for entry in provider
                        .passport_transactions
                        .lock()
                        .unwrap()
                        .passwords
                        .values_mut()
                    {
                        entry.expires_at = Instant::now();
                    }
                }
                release.send(()).unwrap();
                server.await.unwrap();
                task.await.unwrap().unwrap_err()
            };
            assert_eq!(
                e.code,
                match condition {
                    "replace" => ErrorCode::Conflict,
                    "expire" => ErrorCode::ResourceNotFound,
                    _ => ErrorCode::UpstreamTimeout,
                },
                "{condition}/{stage}"
            );
            assert_eq!(read(&store, "A"), expected);
            assert!(
                provider
                    .passport_transactions
                    .lock()
                    .unwrap()
                    .passwords
                    .is_empty()
            );
        }
    }
}

#[tokio::test]
async fn password_actions_are_exclusive_and_cancellation_consumes_the_receipt() {
    for image in [true, false] {
        let replies = if image {
            [image_start(false), vec![ok()]].concat()
        } else {
            [secondary_start(), vec![key(1)]].concat()
        };
        let (mut provider, seen, release, server) = gated(replies).await;
        let (store, old) = seeded(&mut provider);
        let provider = Arc::new(provider);
        let (receipt, _) = pending(
            provider
                .begin_password_login(&request("A"), CredentialMode::Both)
                .await
                .unwrap(),
        );
        let action = if image {
            answer(false)
        } else {
            PasswordChallengeAction::SubmitSms {
                code: "123456".into(),
            }
        };
        let worker = provider.clone();
        let selected = receipt.clone();
        let work_action = action.clone();
        let task =
            tokio::spawn(
                async move { worker.advance_password_login(&selected, &work_action).await },
            );
        tokio::time::timeout(Duration::from_secs(3), seen)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            provider
                .advance_password_login(&receipt, &action)
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        server.abort();
        drop(release);
        assert_eq!(
            provider
                .advance_password_login(&receipt, &action)
                .await
                .unwrap_err()
                .code,
            ErrorCode::ResourceNotFound
        );
        assert_eq!(read(&store, "A"), old);
    }
}

#[tokio::test]
async fn logout_and_new_login_cancel_only_matching_server_password_receipts() {
    for logout in [false, true] {
        let end = if logout {
            vec![response(json!({"code":"000000"}), "")]
        } else {
            vec![key(0), login(), exchange("333"), profile("333", "")]
        };
        let (client, requests) = server(
            [
                image_start(false),
                image_start(false),
                image_start(false),
                end,
            ]
            .concat(),
        )
        .await;
        let mut provider = MiguProvider::from_client(client);
        let (store, old) = seeded(&mut provider);
        let (a, _) = pending(
            provider
                .begin_password_login(&request("A"), CredentialMode::Both)
                .await
                .unwrap(),
        );
        let (b, _) = pending(
            provider
                .begin_password_login(&request("B"), CredentialMode::Server)
                .await
                .unwrap(),
        );
        let (c, _) = pending(
            provider
                .begin_password_login(&request("default"), CredentialMode::Client)
                .await
                .unwrap(),
        );
        if logout {
            provider.logout("A").await.unwrap();
        } else {
            provider
                .password_login_with_mode(&request("A"), CredentialMode::Both)
                .await
                .unwrap();
        }
        {
            let state = provider.passport_transactions.lock().unwrap();
            assert!(!state.passwords.contains_key(a.provider_transaction_id()));
            assert!(state.passwords.contains_key(b.provider_transaction_id()));
            assert!(state.passwords.contains_key(c.provider_transaction_id()));
        }
        assert_eq!(read(&store, "B"), old);
        requests.await.unwrap();
    }
}

#[tokio::test]
async fn failed_secondary_completion_never_exports_or_persists_a_partial_music_session() {
    for tail in [
        vec![key(1), extra(6123)],
        vec![
            key(1),
            login(),
            response(json!({"code":"000000","data":{"userId":"222"}}), ""),
        ],
        vec![key(1), login(), exchange("222"), profile("333", "")],
        vec![
            key(1),
            login(),
            exchange("222"),
            response(
                json!({"code":"000000","data":{"userId":"222","nickName":[]}}),
                "",
            ),
        ],
    ] {
        let (client, requests) = server([secondary_start(), tail].concat()).await;
        let mut provider = MiguProvider::from_client(client);
        let (store, old) = seeded(&mut provider);
        let (receipt, _) = pending(
            provider
                .begin_password_login(&request("A"), CredentialMode::Both)
                .await
                .unwrap(),
        );
        let mut e = provider
            .advance_password_login(
                &receipt,
                &PasswordChallengeAction::SubmitSms {
                    code: "1234".into(),
                },
            )
            .await
            .unwrap_err();
        assert!(e.auth_challenge_consumed());
        assert!(e.take_caller_credential_update().is_none());
        assert_eq!(read(&store, "A"), old);
        assert!(
            provider
                .passport_transactions
                .lock()
                .unwrap()
                .passwords
                .is_empty()
        );
        requests.await.unwrap();
    }
}
