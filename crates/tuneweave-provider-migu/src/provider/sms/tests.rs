use super::*;
use crate::{
    client::passport::tests::{exchange, key, login, response, server},
    credential::MiguCredential,
    provider::session::tests::{Store, gated, profile, read, stored},
};
use tuneweave_core::{ChallengeMethod, PasswordFormat, PasswordLoginRequest, PrincipalType};

fn request(account: &str) -> AuthChallengeRequest {
    AuthChallengeRequest {
        allow_account_creation: false,
        accept_platform_policies: false,
        account: account.into(),
        method: ChallengeMethod::Sms,
        backend: AuthChallengeBackend::Standard,
        principal: "13800138000".into(),
        country_code: Some("86".into()),
    }
}
fn sent() -> String {
    response(json!({"status":2000}), "")
}
fn start_responses() -> Vec<String> {
    vec![key(0), key(1), sent()]
}
fn verified() -> Vec<String> {
    vec![
        key(1),
        login(),
        exchange("222"),
        profile("222", "pacmtoken: profile-final\r\n"),
    ]
}
fn seeded(provider: &mut MiguProvider) -> (Arc<Store>, MiguCredential) {
    let store = Arc::new(Store::default());
    let old = MiguCredential::verified("111".into(), "old-token".into()).unwrap();
    store.put(&stored("A", &old)).unwrap();
    store.put(&stored("B", &old)).unwrap();
    provider.credential_store = Some(store.clone());
    (store, old)
}

#[tokio::test]
async fn sms_verifies_music_identity_before_committing_each_ownership_mode() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let (client, requests) = server([start_responses(), verified()].concat()).await;
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
        assert_eq!(read(&store, "A"), old);
        let code = if mode == CredentialMode::Client {
            "1234"
        } else {
            "123456"
        };
        let result = provider
            .complete_auth_challenge(&receipt, code)
            .await
            .unwrap();
        assert_eq!(result.profile.user_id.as_deref(), Some("222"));
        assert_eq!(result.credential.is_some(), mode.returns_to_caller());
        if mode.persists_on_server() {
            assert_eq!(read(&store, "A").token(), "profile-final");
            assert!(!read(&store, "A").same_login(&old));
        } else {
            assert_eq!(read(&store, "A"), old);
        }
        if let Some(credential) = result.credential {
            let issued = MiguCredential::parse_caller(&credential).unwrap();
            assert_eq!(issued.token(), "profile-final");
            if mode == CredentialMode::Both {
                assert_eq!(issued, read(&store, "A"));
            }
        }
        assert_eq!(read(&store, "B"), old);
        assert_eq!(
            provider
                .complete_auth_challenge(&receipt, "123456")
                .await
                .unwrap_err()
                .code,
            ErrorCode::ResourceNotFound
        );
        assert!(
            provider
                .passport_transactions
                .lock()
                .unwrap()
                .entries
                .is_empty()
        );
        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 7);
        assert!(!requests[5].contains("cookie:") && !requests[6].contains("cookie:"));
        assert!(requests[5].contains("deviceid:"));
        assert!(requests[6].contains("pacmtoken: music-token\r\n"));
    }
}

#[tokio::test]
async fn invalid_sms_inputs_do_not_send_and_forged_receipts_do_not_consume_the_original() {
    let (client, requests) = server(start_responses()).await;
    let mut provider = MiguProvider::from_client(client);
    seeded(&mut provider);
    let mut unsupported = request("A");
    unsupported.allow_account_creation = true;
    assert_eq!(
        provider
            .begin_auth_challenge(&unsupported, CredentialMode::Server)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    for (phone, country, backend, account, mode) in [
        (
            "",
            "86",
            AuthChallengeBackend::Standard,
            "A",
            CredentialMode::Server,
        ),
        (
            "23800138000",
            "86",
            AuthChallengeBackend::Standard,
            "A",
            CredentialMode::Server,
        ),
        (
            "13800138000",
            "1",
            AuthChallengeBackend::Standard,
            "A",
            CredentialMode::Server,
        ),
        (
            "13800138000",
            "86",
            AuthChallengeBackend::Middle,
            "A",
            CredentialMode::Server,
        ),
        (
            "13800138000",
            "86",
            AuthChallengeBackend::Standard,
            "A",
            CredentialMode::Client,
        ),
    ] {
        let mut r = request(account);
        r.principal = phone.into();
        r.country_code = Some(country.into());
        r.backend = backend;
        assert_eq!(
            provider
                .begin_auth_challenge(&r, mode)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(
        provider
            .passport_transactions
            .lock()
            .unwrap()
            .entries
            .is_empty()
    );
    let receipt = provider
        .begin_auth_challenge(&request("A"), CredentialMode::Server)
        .await
        .unwrap();
    for (platform, request, mode) in [
        (Platform::Migu, request("B"), CredentialMode::Server),
        (Platform::Migu, request("A"), CredentialMode::Both),
        (Platform::Qq, request("A"), CredentialMode::Server),
    ] {
        let forged = ProviderAuthChallenge::stateful(
            platform,
            request,
            mode,
            receipt.provider_transaction_id().unwrap().into(),
        )
        .unwrap();
        assert_eq!(
            provider
                .complete_auth_challenge(&forged, "123456")
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for code in ["", "12345", "1234567", "12 456", "１２３４５６"] {
        assert_eq!(
            provider
                .complete_auth_challenge(&receipt, code)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    {
        let store = provider.passport_transactions.lock().unwrap();
        let entry = store
            .entries
            .get(receipt.provider_transaction_id().unwrap())
            .unwrap();
        assert_eq!(entry.attempts, 0);
        assert!(entry.context.is_some());
    }
    assert_eq!(requests.await.unwrap().len(), 3);
}

#[tokio::test]
async fn sms_rejections_use_five_bounded_attempts_and_preserve_existing_accounts() {
    let mut replies = start_responses();
    for _ in 0..ATTEMPTS {
        replies.extend([
            key(1),
            response(
                json!({"status":4005}),
                "Set-Cookie: mgnd_session_id=rotated; Domain=.migu.cn; Path=/\r\n",
            ),
        ]);
    }
    let (client, requests) = server(replies).await;
    let mut provider = MiguProvider::from_client(client);
    let (store, old) = seeded(&mut provider);
    let receipt = provider
        .begin_auth_challenge(&request("A"), CredentialMode::Both)
        .await
        .unwrap();
    for attempt in 1..=ATTEMPTS {
        let mut error = provider
            .complete_auth_challenge(&receipt, "000000")
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::AuthenticationRequired);
        assert_eq!(error.details["remaining_attempts"], ATTEMPTS - attempt);
        assert_eq!(error.auth_challenge_consumed(), attempt == ATTEMPTS);
        assert!(error.take_caller_credential_update().is_none());
        assert_eq!(read(&store, "A"), old);
    }
    assert_eq!(
        provider
            .complete_auth_challenge(&receipt, "123456")
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    assert!(
        provider
            .passport_transactions
            .lock()
            .unwrap()
            .entries
            .is_empty()
    );
    assert_eq!(requests.await.unwrap().len(), 3 + 2 * usize::from(ATTEMPTS));
}

#[tokio::test]
async fn failed_sms_auth_exchange_or_profile_is_consumed_without_partial_credentials() {
    for tail in [
        vec![response(json!({"status":5000}), "")],
        vec![key(1), response(json!({"status":4999}), "")],
        vec![key(1), response(json!({"status":6123}), "")],
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
        let (client, requests) = server([start_responses(), tail].concat()).await;
        let mut provider = MiguProvider::from_client(client);
        let (store, old) = seeded(&mut provider);
        let receipt = provider
            .begin_auth_challenge(&request("A"), CredentialMode::Both)
            .await
            .unwrap();
        let mut error = provider
            .complete_auth_challenge(&receipt, "123456")
            .await
            .unwrap_err();
        assert!(error.auth_challenge_consumed());
        assert!(error.take_caller_credential_update().is_none());
        assert_eq!(read(&store, "A"), old);
        assert!(
            provider
                .passport_transactions
                .lock()
                .unwrap()
                .entries
                .is_empty()
        );
        assert_eq!(
            provider
                .complete_auth_challenge(&receipt, "123456")
                .await
                .unwrap_err()
                .code,
            ErrorCode::ResourceNotFound
        );
        requests.await.unwrap();
    }
}

#[tokio::test]
async fn failed_or_cancelled_sms_send_retains_cooldown_but_no_transaction() {
    let (client, requests) = server(vec![response(json!({"status":5000}), "")]).await;
    let provider = MiguProvider::from_client(client);
    assert_eq!(
        provider
            .begin_auth_challenge(&request("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert!(
        provider
            .passport_transactions
            .lock()
            .unwrap()
            .entries
            .is_empty()
    );
    assert_eq!(
        provider
            .begin_auth_challenge(&request("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    requests.await.unwrap();

    let (provider, seen, release, server) = gated(start_responses()).await;
    let provider = Arc::new(provider);
    let worker = provider.clone();
    let task = tokio::spawn(async move {
        worker
            .begin_auth_challenge(&request("default"), CredentialMode::Client)
            .await
    });
    tokio::time::timeout(Duration::from_secs(3), seen)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(
        provider
            .passport_transactions
            .lock()
            .unwrap()
            .entries
            .is_empty()
    );
    assert_eq!(
        provider
            .begin_auth_challenge(&request("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    server.abort();
    drop(release);
}

#[tokio::test]
async fn same_phone_after_cooldown_keeps_independent_receipts_and_original_expiry() {
    let (client, requests) =
        server([start_responses(), start_responses(), verified()].concat()).await;
    let provider = MiguProvider::from_client(client);
    let first = provider
        .begin_auth_challenge(&request("default"), CredentialMode::Client)
        .await
        .unwrap();
    assert_eq!(
        provider
            .begin_auth_challenge(&request("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    provider
        .passport_transactions
        .lock()
        .unwrap()
        .cooldowns
        .clear();
    let second = provider
        .begin_auth_challenge(&request("default"), CredentialMode::Client)
        .await
        .unwrap();
    assert_ne!(
        first.provider_transaction_id(),
        second.provider_transaction_id()
    );
    provider
        .passport_transactions
        .lock()
        .unwrap()
        .entries
        .get_mut(first.provider_transaction_id().unwrap())
        .unwrap()
        .expires_at = Instant::now();
    assert_eq!(
        provider
            .complete_auth_challenge(&first, "123456")
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    assert!(
        provider
            .complete_auth_challenge(&second, "123456")
            .await
            .unwrap()
            .profile
            .authenticated
    );
    requests.await.unwrap();
}

#[tokio::test]
async fn sms_capacity_is_checked_before_key_fetch_and_legacy_entry_points_are_explicitly_unsupported()
 {
    let (client, requests) = server(vec![]).await;
    let provider = MiguProvider::from_client(client);
    for index in 0..CAPACITY {
        let id = format!("receipt-{index}");
        let receipt = ProviderAuthChallenge::stateful(
            Platform::Migu,
            request("default"),
            CredentialMode::Client,
            id.clone(),
        )
        .unwrap();
        provider
            .passport_transactions
            .lock()
            .unwrap()
            .entries
            .insert(
                id,
                Entry {
                    receipt,
                    created_at: Instant::now(),
                    expires_at: Instant::now() + TTL,
                    attempts: 0,
                    context: None,
                },
            );
    }
    assert_eq!(
        provider
            .begin_auth_challenge(&request("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    assert_eq!(
        provider
            .start_auth_challenge(&request("default"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert_eq!(
        provider
            .verify_auth_challenge(&request("default"), "123456")
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn sms_cannot_overwrite_a_changed_alias_at_any_async_authentication_boundary() {
    for stage in 0..5 {
        let mut replies = start_responses();
        if stage > 0 {
            replies.extend(verified().into_iter().take(stage));
        }
        let (mut provider, seen, release, server) = gated(replies).await;
        let (store, old) = seeded(&mut provider);
        let provider = Arc::new(provider);
        let receipt = if stage == 0 {
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
                    .complete_auth_challenge(&receipt, "123456")
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
        let new = MiguCredential::verified("111".into(), old.token().into()).unwrap();
        store.put(&stored("A", &new)).unwrap();
        release.send(()).unwrap();
        assert_eq!(
            task.await.unwrap().unwrap_err().code,
            ErrorCode::Conflict,
            "stage {stage}"
        );
        assert_eq!(read(&store, "A"), new);
        assert_eq!(read(&store, "B"), old);
        assert!(
            provider
                .passport_transactions
                .lock()
                .unwrap()
                .entries
                .is_empty()
        );
        server.await.unwrap();
    }
}

#[tokio::test]
async fn logout_or_new_password_login_cancels_pending_sms_for_the_selected_server_alias() {
    for password in [false, true] {
        let mut replies = start_responses();
        if password {
            replies.extend([key(0), login(), exchange("222"), profile("222", "")]);
        }
        let (client, requests) = server(replies).await;
        let mut provider = MiguProvider::from_client(client);
        provider.credential_store = Some(Arc::new(Store::default()));
        let receipt = provider
            .begin_auth_challenge(&request("A"), CredentialMode::Server)
            .await
            .unwrap();
        if password {
            provider
                .password_login(&PasswordLoginRequest {
                    backend: Default::default(),
                    account: "A".into(),
                    principal_type: PrincipalType::Phone,
                    principal: "13800138000".into(),
                    password: "password".into(),
                    password_format: PasswordFormat::Plain,
                    country_code: None,
                    secure_captcha: None,
                })
                .await
                .unwrap();
        } else {
            assert!(!provider.logout("A").await.unwrap());
        }
        assert_eq!(
            provider
                .complete_auth_challenge(&receipt, "123456")
                .await
                .unwrap_err()
                .code,
            ErrorCode::ResourceNotFound
        );
        requests.await.unwrap();
    }
}

#[tokio::test]
async fn sms_expiration_and_timeout_stop_each_network_stage_without_a_partial_login() {
    for expire in [false, true] {
        for stage in 1..=7 {
            let replies = [start_responses(), verified()].concat();
            let (mut provider, seen, release, server) = gated(replies[..stage].to_vec()).await;
            provider.client = provider
                .client
                .with_session_test_timeout(Duration::from_millis(300));
            let (store, old) = seeded(&mut provider);
            let provider = Arc::new(provider);
            let receipt = if stage <= 3 {
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
                        .complete_auth_challenge(&receipt, "123456")
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
            let error = if expire {
                for entry in provider
                    .passport_transactions
                    .lock()
                    .unwrap()
                    .entries
                    .values_mut()
                {
                    entry.expires_at = Instant::now();
                }
                release.send(()).unwrap();
                server.await.unwrap();
                task.await.unwrap().unwrap_err()
            } else {
                let error = task.await.unwrap().unwrap_err();
                server.abort();
                drop(release);
                error
            };
            assert_eq!(
                error.code,
                if expire {
                    ErrorCode::ResourceNotFound
                } else {
                    ErrorCode::UpstreamTimeout
                },
                "stage {stage}"
            );
            assert_eq!(read(&store, "A"), old);
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
async fn concurrent_or_cancelled_sms_verification_cannot_replay_the_transaction() {
    let (mut provider, seen, release, server) =
        gated([start_responses(), verified()].concat()).await;
    let (store, old) = seeded(&mut provider);
    let provider = Arc::new(provider);
    let receipt = provider
        .begin_auth_challenge(&request("A"), CredentialMode::Both)
        .await
        .unwrap();
    let worker = provider.clone();
    let selected = receipt.clone();
    let task =
        tokio::spawn(async move { worker.complete_auth_challenge(&selected, "123456").await });
    tokio::time::timeout(Duration::from_secs(3), seen)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        provider
            .complete_auth_challenge(&receipt, "123456")
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
            .complete_auth_challenge(&receipt, "123456")
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    assert_eq!(read(&store, "A"), old);
}

#[tokio::test]
async fn new_login_on_another_alias_preserves_pending_sms_and_client_scope_cannot_start_it() {
    let replies = [
        start_responses(),
        vec![key(0), login(), exchange("333"), profile("333", "")],
        verified(),
    ]
    .concat();
    let (client, requests) = server(replies).await;
    let mut provider = MiguProvider::from_client(client);
    let (store, old) = seeded(&mut provider);
    let caller = provider.caller_scope(&old.caller().unwrap()).unwrap();
    assert_eq!(
        caller
            .begin_auth_challenge(&request("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let receipt = provider
        .begin_auth_challenge(&request("A"), CredentialMode::Both)
        .await
        .unwrap();
    provider
        .password_login(&PasswordLoginRequest {
            backend: Default::default(),
            account: "B".into(),
            principal_type: PrincipalType::Phone,
            principal: "13900139000".into(),
            password: "password".into(),
            password_format: PasswordFormat::Plain,
            country_code: None,
            secure_captcha: None,
        })
        .await
        .unwrap();
    assert_eq!(read(&store, "A"), old);
    assert_eq!(read(&store, "B").user_id(), "333");
    provider
        .complete_auth_challenge(&receipt, "123456")
        .await
        .unwrap();
    assert_eq!(read(&store, "A").user_id(), "222");
    assert_eq!(read(&store, "B").user_id(), "333");
    requests.await.unwrap();
}

mod image;
