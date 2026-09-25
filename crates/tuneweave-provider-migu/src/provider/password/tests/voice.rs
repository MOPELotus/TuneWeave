use super::*;

fn delivery() -> String {
    response(
        json!({"status":2000,"result":{"captchaId":"unsubmitted-captcha-id"}}),
        "Set-Cookie: mgnd_session_id=voice-cookie; Domain=.migu.cn; Path=/\r\n",
    )
}

fn start() -> Vec<String> {
    vec![key(0), extra(6118), delivery()]
}

fn finish() -> Vec<String> {
    vec![login(), exchange("222"), profile("222", "")]
}

fn submit(code: &str) -> PasswordChallengeAction {
    PasswordChallengeAction::SubmitVoice { code: code.into() }
}

fn allow_resend(provider: &MiguProvider, receipt: &ProviderPasswordChallenge) {
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

#[tokio::test]
async fn voice_password_contract_preserves_exact_forms_cookies_and_account_modes() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        for code in ["1234", "123456"] {
            let (client, requests) = server([start(), finish()].concat()).await;
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
            let PasswordVerification::Voice {
                masked_destination,
                remaining_attempts,
                resend_after_secs,
            } = &verification
            else {
                panic!("Expected voice verification")
            };
            assert_eq!(masked_destination, "138****8000");
            assert_eq!(*remaining_attempts, 5);
            assert!((1..=60).contains(resend_after_secs));
            assert_eq!(
                serde_json::to_value(&verification).unwrap()["method"],
                "voice"
            );
            assert!(!format!("{verification:?}").contains("138"));
            assert_eq!(read(&store, "A"), old);

            let PasswordLoginProgress::Confirmed(result) = provider
                .advance_password_login(&receipt, &submit(code))
                .await
                .unwrap()
            else {
                panic!("Expected confirmed voice login")
            };
            assert_eq!(result.profile.account, account);
            assert_eq!(result.profile.user_id.as_deref(), Some("222"));
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

            let requests = requests.await.unwrap();
            assert_eq!(requests.len(), 6);
            assert!(requests[0].starts_with("POST /password/publickey "));
            assert!(requests[1].starts_with("POST /authn "));
            assert!(requests[2].starts_with("GET /login/send/voice?"));
            assert!(requests[2].contains("mgnd_session_id=extra"));
            let target = requests[2].split_whitespace().nth(1).unwrap();
            let url = url::Url::parse(&format!("https://passport.migu.cn{target}")).unwrap();
            let fields = url.query_pairs().collect::<BTreeMap<_, _>>();
            assert_eq!(fields.len(), 3);
            assert_eq!(fields["msisdn"], "opaque+/=?%ticket");
            assert_eq!(fields["sourceID"], "220029");
            assert_eq!(fields["isAsync"], "true");
            assert!(requests[3].starts_with("POST /authn/voice/validate "));
            assert!(requests[3].contains("mgnd_session_id=voice-cookie"));
            assert!(requests[3].contains("content-type: application/x-www-form-urlencoded"));
            let body = requests[3].split_once("\r\n\r\n").unwrap().1;
            let fields = url::form_urlencoded::parse(body.as_bytes()).collect::<BTreeMap<_, _>>();
            assert_eq!(fields.len(), 6);
            assert_eq!(fields["msisdn"], "opaque+/=?%ticket");
            assert_eq!(fields["voiceCode"], code);
            assert_eq!(fields["sourceID"], "220029");
            assert_eq!(fields["appType"], "0");
            assert_eq!(fields["relayState"], "");
            assert_eq!(fields["isAsync"], "true");
            assert!(!requests[4].contains("cookie:") && !requests[5].contains("cookie:"));
            for request in &requests {
                assert!(!request.contains("unsubmitted-captcha-id"));
                assert!(!request.contains("138****8000"));
            }
            assert_eq!(
                provider
                    .advance_password_login(&receipt, &submit(code))
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::ResourceNotFound
            );
        }
    }
}

#[tokio::test]
async fn voice_after_image_keeps_original_receipt_and_expiry() {
    let (client, requests) = server(
        [
            image_start(false),
            vec![ok(), key(1), extra(6118), delivery()],
            finish(),
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
    let expiry = provider.passport_transactions.lock().unwrap().passwords
        [receipt.provider_transaction_id()]
    .expires_at;
    let (same, verification) = pending(
        provider
            .advance_password_login(&receipt, &answer(false))
            .await
            .unwrap(),
    );
    assert_eq!(same, receipt);
    assert!(matches!(verification, PasswordVerification::Voice { .. }));
    assert_eq!(
        provider.passport_transactions.lock().unwrap().passwords[receipt.provider_transaction_id()]
            .expires_at,
        expiry
    );
    assert!(matches!(
        provider
            .advance_password_login(&receipt, &submit("1234"))
            .await
            .unwrap(),
        PasswordLoginProgress::Confirmed(_)
    ));
    requests.await.unwrap();
}

#[tokio::test]
async fn voice_actions_and_resends_are_bounded_without_extending_the_original_lease() {
    let (client, requests) = server([start(), vec![delivery(), delivery()]].concat()).await;
    let provider = MiguProvider::from_client(client);
    let (receipt, _) = pending(
        provider
            .begin_password_login(&request("default"), CredentialMode::Client)
            .await
            .unwrap(),
    );
    let expiry = provider.passport_transactions.lock().unwrap().passwords
        [receipt.provider_transaction_id()]
    .expires_at;
    for action in [
        submit(""),
        submit("12345"),
        submit("１２３４"),
        PasswordChallengeAction::SubmitSms {
            code: "1234".into(),
        },
        PasswordChallengeAction::ResendSms,
        PasswordChallengeAction::RefreshImage,
    ] {
        let e = provider
            .advance_password_login(&receipt, &action)
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidRequest);
        assert!(!e.auth_challenge_consumed());
    }
    assert_eq!(
        provider
            .advance_password_login(&receipt, &PasswordChallengeAction::ResendVoice)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    for _ in 1..SECONDARY_SENDS {
        allow_resend(&provider, &receipt);
        let (same, verification) = pending(
            provider
                .advance_password_login(&receipt, &PasswordChallengeAction::ResendVoice)
                .await
                .unwrap(),
        );
        assert_eq!(same, receipt);
        assert!(matches!(
            verification,
            PasswordVerification::Voice {
                remaining_attempts: 5,
                ..
            }
        ));
        assert_eq!(
            provider.passport_transactions.lock().unwrap().passwords
                [receipt.provider_transaction_id()]
            .expires_at,
            expiry
        );
    }
    allow_resend(&provider, &receipt);
    assert_eq!(
        provider
            .advance_password_login(&receipt, &PasswordChallengeAction::ResendVoice)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    provider
        .passport_transactions
        .lock()
        .unwrap()
        .passwords
        .get_mut(receipt.provider_transaction_id())
        .unwrap()
        .expires_at = Instant::now();
    assert_eq!(
        provider
            .advance_password_login(&receipt, &submit("1234"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    let requests = requests.await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.starts_with("GET /login/send/voice?"))
            .count(),
        3
    );
}

#[tokio::test]
async fn voice_rejections_malformed_success_and_uid_mismatch_consume_without_persistence() {
    for replies in [
        vec![response(
            json!({"status":4005,"message":"private-code-1234 opaque-ticket 13800138000"}),
            "",
        )],
        vec![extra(6118)],
        vec![response(json!({"status":4016}), "")],
        vec![response(json!({"status":2000,"result":{}}), "")],
        vec![response(json!({"status":2000,"result":{"token":""}}), "")],
        vec![
            login(),
            exchange("222"),
            profile("333", "pacmtoken: do-not-persist\r\n"),
        ],
    ] {
        let (client, requests) = server([start(), replies].concat()).await;
        let mut provider = MiguProvider::from_client(client);
        let (store, old) = seeded(&mut provider);
        let (receipt, _) = pending(
            provider
                .begin_password_login(&request("A"), CredentialMode::Both)
                .await
                .unwrap(),
        );
        let mut e = provider
            .advance_password_login(&receipt, &submit("1234"))
            .await
            .unwrap_err();
        assert!(e.auth_challenge_consumed());
        assert!(e.take_caller_credential_update().is_none());
        assert!(e.details.get("remaining_attempts").is_none());
        let public = format!("{} {}", e.message, e.details);
        for secret in [
            "private-code-1234",
            "opaque-ticket",
            "13800138000",
            "do-not-persist",
        ] {
            assert!(!public.contains(secret));
        }
        assert_eq!(read(&store, "A"), old);
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
                .advance_password_login(&receipt, &submit("1234"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::ResourceNotFound
        );
        requests.await.unwrap();
    }
}

#[tokio::test]
async fn voice_delivery_failure_preserves_cooldown_and_never_publishes_a_challenge() {
    for resend in [false, true] {
        let mut replies = if resend {
            start()
        } else {
            vec![key(0), extra(6118)]
        };
        replies.extend([response(json!({"status":4005}), ""), key(1), extra(6103)]);
        let (client, requests) = server(replies).await;
        let mut provider = MiguProvider::from_client(client);
        let (store, old) = seeded(&mut provider);
        let e = if resend {
            let (receipt, _) = pending(
                provider
                    .begin_password_login(&request("A"), CredentialMode::Both)
                    .await
                    .unwrap(),
            );
            allow_resend(&provider, &receipt);
            let e = provider
                .advance_password_login(&receipt, &PasswordChallengeAction::ResendVoice)
                .await
                .unwrap_err();
            assert!(e.auth_challenge_consumed());
            e
        } else {
            provider
                .begin_password_login(&request("A"), CredentialMode::Both)
                .await
                .unwrap_err()
        };
        assert_eq!(e.code, ErrorCode::UpstreamError);
        assert!(
            provider
                .passport_transactions
                .lock()
                .unwrap()
                .passwords
                .is_empty()
        );
        // A new password attempt cannot evade delivery cooldown by changing to SMS.
        assert_eq!(
            provider
                .begin_password_login(&request("A"), CredentialMode::Both)
                .await
                .unwrap_err()
                .code,
            ErrorCode::RateLimited
        );
        assert_eq!(read(&store, "A"), old);
        requests.await.unwrap();
    }
}

#[tokio::test]
async fn voice_identity_is_validated_before_delivery_and_sms_actions_cannot_be_substituted() {
    for identity in [
        json!({}),
        json!({"msisdnRSA":"ticket","msisdnHide":"13800138000"}),
        json!({"msisdnRSA":"ticket\n","msisdnHide":"138****8000"}),
    ] {
        let (client, requests) = server(vec![
            key(0),
            response(json!({"status":6118,"result":identity}), ""),
        ])
        .await;
        let provider = MiguProvider::from_client(client);
        let e = provider
            .begin_password_login(&request("default"), CredentialMode::Client)
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::UpstreamError);
        assert!(
            provider
                .passport_transactions
                .lock()
                .unwrap()
                .passwords
                .is_empty()
        );
        assert!(!format!("{e:?}").contains("ticket"));
        assert!(!format!("{e:?}").contains("13800138000"));
        assert_eq!(requests.await.unwrap().len(), 2);
    }
    let (client, requests) = server(secondary_start()).await;
    let provider = MiguProvider::from_client(client);
    let (receipt, _) = pending(
        provider
            .begin_password_login(&request("default"), CredentialMode::Client)
            .await
            .unwrap(),
    );
    for action in [submit("1234"), PasswordChallengeAction::ResendVoice] {
        let e = provider
            .advance_password_login(&receipt, &action)
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidRequest);
        assert!(!e.auth_challenge_consumed());
    }
    assert_eq!(requests.await.unwrap().len(), 4);
}

#[tokio::test]
async fn voice_network_boundaries_reject_replaced_generation_expiry_and_cancellation() {
    for condition in ["replace", "expire", "cancel", "timeout"] {
        for stage in 1..=6 {
            let replies = [start(), finish()].concat();
            let (mut provider, seen, release, server) = gated(replies[..stage].to_vec()).await;
            if condition == "timeout" {
                provider.client = provider
                    .client
                    .with_session_test_timeout(Duration::from_millis(300));
            }
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
            let worker = provider.clone();
            let task = tokio::spawn(async move {
                if let Some(receipt) = receipt {
                    worker
                        .advance_password_login(&receipt, &submit("1234"))
                        .await
                } else {
                    worker
                        .begin_password_login(&request("A"), CredentialMode::Both)
                        .await
                }
            });
            tokio::time::timeout(Duration::from_secs(5), seen)
                .await
                .unwrap()
                .unwrap();
            // Same UID and token, different login generation must still be rejected.
            let expected = if condition == "replace" {
                let new = MiguCredential::verified("111".into(), "old-token".into()).unwrap();
                store.put(&stored("A", &new)).unwrap();
                new
            } else {
                old.clone()
            };
            if condition == "cancel" {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                server.abort();
                let _ = server.await;
                drop(release);
            } else if condition == "timeout" {
                let e = tokio::time::timeout(Duration::from_secs(5), task)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap_err();
                assert_eq!(e.code, ErrorCode::UpstreamTimeout);
                if stage > 3 {
                    assert!(e.auth_challenge_consumed());
                }
                server.abort();
                let _ = server.await;
                drop(release);
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
                tokio::time::timeout(Duration::from_secs(5), server)
                    .await
                    .unwrap()
                    .unwrap();
                let e = tokio::time::timeout(Duration::from_secs(5), task)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap_err();
                assert_eq!(
                    e.code,
                    if condition == "replace" {
                        ErrorCode::Conflict
                    } else {
                        ErrorCode::ResourceNotFound
                    },
                    "{condition}/{stage}"
                );
                if stage > 3 {
                    assert!(e.auth_challenge_consumed());
                }
            }
            assert_eq!(read(&store, "A"), expected);
            assert_eq!(read(&store, "B"), old);
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
