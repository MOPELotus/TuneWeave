use super::*;
use crate::client::passport::image::tests::image_body;

fn graph(chinese: bool) -> String {
    response(
        image_body(chinese),
        "Set-Cookie: mgnd_session_last_access=graph; Path=/\r\n",
    )
}
fn rejected(code: u16) -> String {
    response(json!({"status":code}), "")
}
fn image_start(chinese: bool) -> Vec<String> {
    vec![
        key(0),
        key(1),
        rejected(if chinese { 4045 } else { 4044 }),
        graph(chinese),
    ]
}
fn answer(chinese: bool) -> AuthChallengeAction {
    AuthChallengeAction::SubmitImage {
        answer: if chinese { "汉字" } else { "42" }.into(),
    }
}
fn image_state(provider: &MiguProvider, receipt: &ProviderAuthChallenge) -> AuthImageChallenge {
    let AuthChallengeStatus::VerificationRequired { verification } =
        provider.sms_status(receipt).unwrap()
    else {
        panic!("Expected image challenge")
    };
    verification
}
fn allow_refresh(provider: &MiguProvider, receipt: &ProviderAuthChallenge) {
    provider
        .passport_transactions
        .lock()
        .unwrap()
        .entries
        .get_mut(receipt.provider_transaction_id().unwrap())
        .unwrap()
        .context
        .as_mut()
        .unwrap()
        .next_image_at = Instant::now();
}

#[tokio::test]
async fn manual_image_then_sms_preserves_binding_cookies_keys_and_each_ownership_mode() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        for chinese in [false, true] {
            let (client, requests) = server(
                [
                    image_start(chinese),
                    vec![sent(), key(1), key(0), sent()],
                    verified(),
                ]
                .concat(),
            )
            .await;
            let mut provider = MiguProvider::from_client(client);
            let (store, old) = seeded(&mut provider);
            let account = if mode == CredentialMode::Client {
                "default"
            } else {
                "A"
            };
            let receipt = provider
                .begin_auth_challenge(&request(account), mode)
                .await
                .unwrap();
            let before = {
                let store = provider.passport_transactions.lock().unwrap();
                let entry = store
                    .entries
                    .get(receipt.provider_transaction_id().unwrap())
                    .unwrap();
                (entry.created_at, entry.expires_at)
            };
            let instructions = image_state(&provider, &receipt);
            assert_eq!(instructions.remaining_attempts, ATTEMPTS);
            assert!(!format!("{instructions:?}").contains("base64"));
            assert_eq!(
                provider
                    .complete_auth_challenge(&receipt, "123456")
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidRequest
            );
            provider
                .passport_transactions
                .lock()
                .unwrap()
                .cooldowns
                .clear();
            assert!(matches!(
                provider
                    .advance_auth_challenge(&receipt, &answer(chinese))
                    .await
                    .unwrap(),
                AuthChallengeProgress::Pending(AuthChallengeStatus::Waiting)
            ));
            assert_eq!(
                provider.sms_status(&receipt).unwrap(),
                AuthChallengeStatus::Waiting
            );
            assert_eq!(read(&store, "A"), old);
            {
                let store = provider.passport_transactions.lock().unwrap();
                let entry = store
                    .entries
                    .get(receipt.provider_transaction_id().unwrap())
                    .unwrap();
                assert_eq!((entry.created_at, entry.expires_at), before);
                assert_eq!(entry.attempts, 0);
                assert!(
                    store.cooldowns[&receipt.request().principal]
                        > Instant::now() + Duration::from_secs(55)
                );
            }
            assert_eq!(
                provider
                    .advance_auth_challenge(&receipt, &AuthChallengeAction::RefreshImage)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::InvalidRequest
            );
            let AuthChallengeProgress::Confirmed(result) = provider
                .advance_auth_challenge(
                    &receipt,
                    &AuthChallengeAction::SubmitCode {
                        code: "1234".into(),
                    },
                )
                .await
                .unwrap()
            else {
                panic!("Login not confirmed")
            };
            assert_eq!(result.profile.account, account);
            assert_eq!(result.credential.is_some(), mode.returns_to_caller());
            assert_eq!(read(&store, "B"), old);
            if mode.persists_on_server() {
                assert_eq!(read(&store, "A").user_id(), "222");
            } else {
                assert_eq!(read(&store, "A"), old);
            }
            assert_eq!(
                provider.sms_status(&receipt).unwrap_err().code,
                ErrorCode::ResourceNotFound
            );
            let requests = requests.await.unwrap();
            assert_eq!(requests.len(), 12);
            assert!(requests[3].starts_with(&format!(
                "POST /captcha/graph/risk?imgcodeType=2&showType={}&sourceid=220029 ",
                if chinese { 1 } else { 0 }
            )));
            assert!(requests[4].starts_with("POST /captcha/graph/check "));
            for index in [4, 5] {
                assert!(requests[index].contains("mgnd_session_last_access=graph"));
            }
            let fields = url::form_urlencoded::parse(
                requests[4].split_once("\r\n\r\n").unwrap().1.as_bytes(),
            )
            .collect::<BTreeMap<_, _>>();
            assert_eq!(fields.len(), 4);
            assert_eq!(fields["sourceid"], "220029");
            assert_eq!(fields["imgcodeType"], "2");
            assert_eq!(fields["isAsync"], "true");
            assert_eq!(fields["captcha"], if chinese { "汉字" } else { "42" });
            let sms = requests[7]
                .lines()
                .next()
                .unwrap()
                .split_once('?')
                .unwrap()
                .1
                .split_whitespace()
                .next()
                .unwrap();
            let fields = url::form_urlencoded::parse(sms.as_bytes()).collect::<BTreeMap<_, _>>();
            assert_eq!(fields["captcha"], if chinese { "汉字" } else { "42" });
            assert_eq!(
                crate::passport::rsa::tests::decrypt(1, &fields["msisdn"]),
                b"13800138000"
            );
            let fields = url::form_urlencoded::parse(
                requests[9].split_once("\r\n\r\n").unwrap().1.as_bytes(),
            )
            .collect::<BTreeMap<_, _>>();
            assert_eq!(fields["captcha"], if chinese { "汉字" } else { "42" });
            assert!(!requests[10].contains("cookie:") && !requests[11].contains("cookie:"));
        }
    }
}

#[tokio::test]
async fn image_attempts_and_refreshes_are_bounded_without_spending_sms_attempts() {
    let mut replies = image_start(false);
    for _ in 0..ATTEMPTS {
        replies.push(graph(false));
    }
    for attempt in 1..=ATTEMPTS {
        replies.push(rejected(4002));
        if attempt < ATTEMPTS {
            replies.push(graph(false));
        }
    }
    let (client, requests) = server(replies).await;
    let provider = MiguProvider::from_client(client);
    let receipt = provider
        .begin_auth_challenge(&request("default"), CredentialMode::Client)
        .await
        .unwrap();
    assert_eq!(
        provider
            .advance_auth_challenge(&receipt, &AuthChallengeAction::RefreshImage)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    for _ in 0..ATTEMPTS {
        allow_refresh(&provider, &receipt);
        provider
            .advance_auth_challenge(&receipt, &AuthChallengeAction::RefreshImage)
            .await
            .unwrap();
    }
    allow_refresh(&provider, &receipt);
    assert_eq!(
        provider
            .advance_auth_challenge(&receipt, &AuthChallengeAction::RefreshImage)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    for value in ["", "-1", "00", "汉字", "123456"] {
        assert_eq!(
            provider
                .advance_auth_challenge(
                    &receipt,
                    &AuthChallengeAction::SubmitImage {
                        answer: value.into()
                    }
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        image_state(&provider, &receipt).remaining_attempts,
        ATTEMPTS
    );
    for attempt in 1..=ATTEMPTS {
        let result = provider
            .advance_auth_challenge(&receipt, &answer(false))
            .await;
        if attempt == ATTEMPTS {
            assert!(result.unwrap_err().auth_challenge_consumed());
        } else {
            assert!(matches!(
                result.unwrap(),
                AuthChallengeProgress::Pending(AuthChallengeStatus::VerificationRequired { .. })
            ));
            assert_eq!(
                image_state(&provider, &receipt).remaining_attempts,
                ATTEMPTS - attempt
            );
            assert_eq!(
                provider
                    .passport_transactions
                    .lock()
                    .unwrap()
                    .entries
                    .get(receipt.provider_transaction_id().unwrap())
                    .unwrap()
                    .attempts,
                0
            );
        }
    }
    assert!(
        provider
            .passport_transactions
            .lock()
            .unwrap()
            .entries
            .is_empty()
    );
    assert_eq!(
        requests.await.unwrap().len(),
        4 + usize::from(ATTEMPTS) * 3 - 1
    );
}

#[tokio::test]
async fn changed_image_requirement_is_returned_on_the_same_receipt_without_resending_otp() {
    let (client, requests) = server(
        [
            image_start(false),
            vec![
                sent(),
                key(1),
                key(0),
                rejected(4045),
                graph(true),
                sent(),
                key(0),
                key(1),
                sent(),
            ],
            verified(),
        ]
        .concat(),
    )
    .await;
    let provider = MiguProvider::from_client(client);
    let receipt = provider
        .begin_auth_challenge(&request("default"), CredentialMode::Client)
        .await
        .unwrap();
    provider
        .advance_auth_challenge(&receipt, &answer(false))
        .await
        .unwrap();
    assert_eq!(
        image_state(&provider, &receipt).answer_kind,
        AuthImageAnswerKind::Chinese
    );
    assert_eq!(
        image_state(&provider, &receipt).remaining_attempts,
        ATTEMPTS - 1
    );
    provider
        .advance_auth_challenge(&receipt, &answer(true))
        .await
        .unwrap();
    provider
        .complete_auth_challenge(&receipt, "123456")
        .await
        .unwrap();
    assert_eq!(requests.await.unwrap().len(), 17);
}

#[tokio::test]
async fn image_required_during_sms_verification_retains_transaction_without_storing_otp() {
    let (client, requests) = server(
        [
            start_responses(),
            vec![key(1), rejected(4002), graph(false), sent()],
            verified(),
        ]
        .concat(),
    )
    .await;
    let provider = MiguProvider::from_client(client);
    let receipt = provider
        .begin_auth_challenge(&request("default"), CredentialMode::Client)
        .await
        .unwrap();
    let error = provider
        .complete_auth_challenge(&receipt, "123456")
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    assert!(!error.auth_challenge_consumed());
    image_state(&provider, &receipt);
    provider
        .advance_auth_challenge(&receipt, &answer(false))
        .await
        .unwrap();
    provider
        .complete_auth_challenge(&receipt, "654321")
        .await
        .unwrap();
    let requests = requests.await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|v| v.starts_with("GET /login/dynamicpassword?"))
            .count(),
        1
    );
    assert_eq!(requests.len(), 11);
}

#[tokio::test]
async fn image_receipt_cannot_change_phone_alias_mode_or_provider_and_expiry_is_final() {
    let (client, requests) = server(image_start(false)).await;
    let mut provider = MiguProvider::from_client(client);
    let (store, old) = seeded(&mut provider);
    let receipt = provider
        .begin_auth_challenge(&request("A"), CredentialMode::Both)
        .await
        .unwrap();
    for variant in 0..4 {
        let mut request = receipt.request().clone();
        let mut mode = receipt.credential_mode();
        let mut platform = Platform::Migu;
        match variant {
            0 => request.principal = "13900139000".into(),
            1 => request.account = "B".into(),
            2 => mode = CredentialMode::Server,
            _ => platform = Platform::Qq,
        }
        let forged = ProviderAuthChallenge::stateful(
            platform,
            request,
            mode,
            receipt.provider_transaction_id().unwrap().into(),
        )
        .unwrap();
        assert_eq!(
            provider
                .advance_auth_challenge(&forged, &answer(false))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        assert!(provider.sms_status(&forged).is_err());
    }
    image_state(&provider, &receipt);
    provider
        .passport_transactions
        .lock()
        .unwrap()
        .entries
        .get_mut(receipt.provider_transaction_id().unwrap())
        .unwrap()
        .expires_at = Instant::now();
    assert_eq!(
        provider
            .advance_auth_challenge(&receipt, &answer(false))
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    assert_eq!(read(&store, "A"), old);
    requests.await.unwrap();
}

#[tokio::test]
async fn image_network_failure_and_owner_changes_consume_without_partial_login_at_every_boundary() {
    for condition in ["expire", "replace", "timeout"] {
        // Initial image, manual graph check, each key, and resumed SMS send.
        for stage in 4..=8 {
            let replies = [image_start(false), vec![sent(), key(1), key(0), sent()]].concat();
            let (mut provider, seen, release, server) = gated(replies[..stage].to_vec()).await;
            provider.client = provider
                .client
                .with_session_test_timeout(Duration::from_millis(300));
            let (store, old) = seeded(&mut provider);
            let provider = Arc::new(provider);
            let receipt = if stage == 4 {
                None
            } else {
                Some(
                    provider
                        .begin_auth_challenge(&request("A"), CredentialMode::Both)
                        .await
                        .unwrap(),
                )
            };
            let worker = provider.clone();
            let task = tokio::spawn(async move {
                if let Some(receipt) = receipt {
                    worker
                        .advance_auth_challenge(&receipt, &answer(false))
                        .await
                        .map(|_| ())
                } else {
                    worker
                        .begin_auth_challenge(&request("A"), CredentialMode::Both)
                        .await
                        .map(|_| ())
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
            let error = if condition == "timeout" {
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
                        .entries
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
                error.code,
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
                    .entries
                    .is_empty()
            );
        }
    }
}

#[tokio::test]
async fn concurrent_image_actions_are_exclusive_and_cancellation_consumes_the_context() {
    for refresh in [false, true] {
        let (mut provider, seen, release, server) = gated(
            [
                image_start(false),
                vec![if refresh { graph(false) } else { sent() }],
            ]
            .concat(),
        )
        .await;
        let (store, old) = seeded(&mut provider);
        let provider = Arc::new(provider);
        let receipt = provider
            .begin_auth_challenge(&request("A"), CredentialMode::Both)
            .await
            .unwrap();
        allow_refresh(&provider, &receipt);
        let worker = provider.clone();
        let selected = receipt.clone();
        let task = tokio::spawn(async move {
            worker
                .advance_auth_challenge(
                    &selected,
                    &if refresh {
                        AuthChallengeAction::RefreshImage
                    } else {
                        answer(false)
                    },
                )
                .await
        });
        tokio::time::timeout(Duration::from_secs(3), seen)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            provider
                .advance_auth_challenge(&receipt, &answer(false))
                .await
                .unwrap_err()
                .code,
            ErrorCode::Conflict
        );
        assert_eq!(
            provider.sms_status(&receipt).unwrap_err().code,
            ErrorCode::Conflict
        );
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        server.abort();
        drop(release);
        assert_eq!(
            provider.sms_status(&receipt).unwrap_err().code,
            ErrorCode::ResourceNotFound
        );
        assert_eq!(read(&store, "A"), old);
    }
}

#[tokio::test]
async fn generic_image_rejection_preserves_the_current_chinese_graph_type() {
    let (client, requests) =
        server([image_start(true), vec![rejected(4002), graph(true)]].concat()).await;
    let provider = MiguProvider::from_client(client);
    let receipt = provider
        .begin_auth_challenge(&request("default"), CredentialMode::Client)
        .await
        .unwrap();
    provider
        .advance_auth_challenge(&receipt, &answer(true))
        .await
        .unwrap();
    assert_eq!(
        image_state(&provider, &receipt).answer_kind,
        AuthImageAnswerKind::Chinese
    );
    assert!(
        requests.await.unwrap()[5]
            .starts_with("POST /captcha/graph/risk?imgcodeType=2&showType=1&sourceid=220029 ")
    );
}
