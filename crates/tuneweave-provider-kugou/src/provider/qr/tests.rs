use super::*;
use crate::provider::session::tests::{
    Store, credential, exchange, paused, profile, read, reply, server,
};

const KEY: &str = "0123456789ABCDEF0123456789ABCDEF0123";

#[tokio::test]
async fn web_exchange_expiration_and_new_account_conflict_do_not_publish_cookies() {
    for expire in [false, true] {
        let body = reply(json!({})).replace("Content-Type:", "Set-Cookie: KuGoo=KugooID=111&t=late-web&a_id=1014; Domain=.kugou.com; Path=/\r\nContent-Type:");
        let (frame, resume) = paused(body);
        let mut f = server(vec![created().into(), authorized().into(), frame]).await;
        let store = Arc::new(Store::default());
        f.provider.credential_store = Some(store.clone());
        let start = f
            .provider
            .start_qr_login_with_mode(Some("web"), CredentialMode::Both)
            .await
            .unwrap();
        let worker = f.provider.clone();
        let id = start.provider_transaction_id.clone();
        let task = tokio::spawn(async move {
            worker
                .poll_qr_login_with_mode(&id, "A", CredentialMode::Both)
                .await
        });
        for _ in 0..3 {
            f.seen.recv().await.unwrap();
        }
        let winner = credential("222", "new-native");
        if expire {
            f.provider
                .qr_transactions
                .lock()
                .unwrap()
                .entries
                .get_mut(&start.provider_transaction_id)
                .unwrap()
                .deadline = Instant::now();
        } else {
            store.put(&winner.stored("A").unwrap()).unwrap();
        }
        resume.send(()).unwrap();
        let result = task.await.unwrap();
        if expire {
            assert_eq!(result.unwrap().state, AuthState::Expired);
            assert!(store.values.lock().unwrap().is_empty());
        } else {
            assert_eq!(result.unwrap_err().code, ErrorCode::Conflict);
            assert_eq!(read(&store, "A"), winner);
        }
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
}
fn created() -> String {
    reply(json!({"qrcode":KEY}))
}
fn authorized() -> String {
    reply(json!({"status":4,"userid":"111","token":"qr-original"}))
}

#[tokio::test]
async fn cancellation_during_token_exchange_prevents_the_following_profile_request() {
    let (exchange_frame, resume) = paused(exchange("111", "next"));
    let mut f = server(vec![created().into(), authorized().into(), exchange_frame]).await;
    let start = f
        .provider
        .start_qr_login_with_mode(None, CredentialMode::Client)
        .await
        .unwrap();
    let provider = f.provider.clone();
    let id = start.provider_transaction_id.clone();
    let task = tokio::spawn(async move {
        provider
            .poll_qr_login_with_mode(&id, "default", CredentialMode::Client)
            .await
    });
    for _ in 0..3 {
        f.seen.recv().await.unwrap();
    }
    assert!(
        f.provider
            .cancel_qr_login(&start.provider_transaction_id)
            .unwrap()
    );
    resume.send(()).unwrap();
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(f.requests.await.unwrap().len(), 3);
}

#[tokio::test]
async fn successful_server_login_cancels_other_transactions_already_bound_to_that_alias() {
    let mut f = server(vec![
        created().into(),
        reply(json!({"status":1})).into(),
        created().into(),
        reply(json!({"status":1})).into(),
        authorized().into(),
        exchange("111", "winner").into(),
        profile("111").into(),
    ])
    .await;
    f.provider.credential_store = Some(Arc::new(Store::default()));
    let a = f
        .provider
        .start_qr_login_with_mode(None, CredentialMode::Both)
        .await
        .unwrap();
    f.provider
        .poll_qr_login_with_mode(&a.provider_transaction_id, "A", CredentialMode::Both)
        .await
        .unwrap();
    let b = f
        .provider
        .start_qr_login_with_mode(None, CredentialMode::Both)
        .await
        .unwrap();
    f.provider
        .poll_qr_login_with_mode(&b.provider_transaction_id, "A", CredentialMode::Both)
        .await
        .unwrap();
    allow(&f.provider, &a.provider_transaction_id).await;
    assert_eq!(
        f.provider
            .poll_qr_login_with_mode(&a.provider_transaction_id, "A", CredentialMode::Both)
            .await
            .unwrap()
            .state,
        AuthState::Confirmed
    );
    assert_eq!(
        f.provider
            .poll_qr_login_with_mode(&b.provider_transaction_id, "A", CredentialMode::Both)
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    assert_eq!(f.requests.await.unwrap().len(), 7);
}
async fn allow(provider: &KugouProvider, id: &str) {
    let qr = provider.qr_transactions.lock().unwrap().entries[id]
        .qr
        .clone();
    qr.allow_test_poll().await;
}

#[tokio::test]
async fn qr_login_completes_both_native_clients_with_exact_credential_ownership() {
    for kind in ["standard", "concept"] {
        for mode in [
            CredentialMode::Server,
            CredentialMode::Client,
            CredentialMode::Both,
        ] {
            let mut f = server(vec![
                created().into(),
                authorized().into(),
                exchange("111", "verified-token").into(),
                profile("111").into(),
            ])
            .await;
            let store = Arc::new(Store::default());
            f.provider.credential_store = Some(store.clone());
            let start = f
                .provider
                .start_qr_login_with_mode(Some(kind), mode)
                .await
                .unwrap();
            assert_eq!(start.provider_transaction_id.len(), 64);
            assert!(!start.provider_transaction_id.contains(KEY));
            assert!(
                start
                    .image_data_url
                    .as_deref()
                    .is_some_and(|image| image.starts_with("data:image/svg+xml;base64,"))
            );
            let account = if mode == CredentialMode::Client {
                "default"
            } else {
                "personal"
            };
            let result = f
                .provider
                .poll_qr_login_with_mode(&start.provider_transaction_id, account, mode)
                .await
                .unwrap();
            assert_eq!(result.state, AuthState::Confirmed);
            let p = result.profile.unwrap();
            assert_eq!(p.account, account);
            assert_eq!(p.user_id.as_deref(), Some("111"));
            assert_eq!(result.credential.is_some(), mode.returns_to_caller());
            assert_eq!(
                store.values.lock().unwrap().len(),
                usize::from(mode.persists_on_server())
            );
            if let Some(caller) = result.credential {
                let c = KugouCredential::parse_caller(&caller).unwrap();
                assert_eq!(
                    c.native().session.client,
                    if kind == "standard" {
                        KugouLoginClient::Standard
                    } else {
                        KugouLoginClient::Concept
                    }
                );
                if mode == CredentialMode::Both {
                    assert_eq!(c, read(&store, account));
                }
            }
            assert!(
                f.provider
                    .poll_qr_login_with_mode(&start.provider_transaction_id, account, mode)
                    .await
                    .is_err()
            );
            assert_eq!(f.requests.await.unwrap().len(), 4);
        }
    }
}

#[tokio::test]
async fn qr_account_and_mode_bind_on_first_poll_and_waiting_never_exports_credentials() {
    let mut f = server(vec![
        created().into(),
        reply(json!({"status":1})).into(),
        reply(json!({"status":2})).into(),
        authorized().into(),
        exchange("111", "confirmed").into(),
        profile("111").into(),
    ])
    .await;
    f.provider.credential_store = Some(Arc::new(Store::default()));
    let start = f
        .provider
        .start_qr_login_with_mode(None, CredentialMode::Both)
        .await
        .unwrap();
    let id = &start.provider_transaction_id;
    assert!(
        f.provider
            .poll_qr_login_with_mode(id, "A", CredentialMode::Server)
            .await
            .is_err()
    );
    let first = f
        .provider
        .poll_qr_login_with_mode(id, "A", CredentialMode::Both)
        .await
        .unwrap();
    assert_eq!(first.state, AuthState::Waiting);
    assert!(first.credential.is_none() && first.profile.is_none());
    assert!(
        f.provider
            .poll_qr_login_with_mode(id, "B", CredentialMode::Both)
            .await
            .is_err()
    );
    assert_eq!(
        f.provider
            .poll_qr_login_with_mode(id, "A", CredentialMode::Both)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    allow(&f.provider, id).await;
    let second = f
        .provider
        .poll_qr_login_with_mode(id, "A", CredentialMode::Both)
        .await
        .unwrap();
    assert_eq!(second.state, AuthState::Scanned);
    assert!(second.credential.is_none());
    allow(&f.provider, id).await;
    assert_eq!(
        f.provider
            .poll_qr_login_with_mode(id, "A", CredentialMode::Both)
            .await
            .unwrap()
            .state,
        AuthState::Confirmed
    );
    assert_eq!(f.requests.await.unwrap().len(), 6);
}

#[tokio::test]
async fn logout_cancel_and_expiry_discard_completion_while_exchange_or_profile_is_in_flight() {
    for action in ["logout", "cancel", "expiry"] {
        for existing in [false, true] {
            let (last, resume) = paused(profile("111"));
            let mut f = server(vec![
                created().into(),
                authorized().into(),
                exchange("111", "new-token").into(),
                last,
            ])
            .await;
            let store = Arc::new(Store::default());
            if existing {
                store
                    .put(&credential("111", "old-token").stored("A").unwrap())
                    .unwrap();
            }
            f.provider.credential_store = Some(store.clone());
            let start = f
                .provider
                .start_qr_login_with_mode(None, CredentialMode::Both)
                .await
                .unwrap();
            let id = start.provider_transaction_id.clone();
            let provider = f.provider.clone();
            let task = tokio::spawn(async move {
                provider
                    .poll_qr_login_with_mode(&id, "A", CredentialMode::Both)
                    .await
            });
            for _ in 0..4 {
                f.seen.recv().await.unwrap();
            }
            assert_eq!(
                f.provider
                    .poll_qr_login_with_mode(
                        &start.provider_transaction_id,
                        "A",
                        CredentialMode::Both
                    )
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::RateLimited
            );
            match action {
                "logout" => {
                    assert_eq!(f.provider.logout("A").await.unwrap(), existing);
                }
                "cancel" => {
                    assert!(
                        f.provider
                            .cancel_qr_login(&start.provider_transaction_id)
                            .unwrap()
                    );
                }
                _ => {
                    f.provider
                        .qr_transactions
                        .lock()
                        .unwrap()
                        .entries
                        .get_mut(&start.provider_transaction_id)
                        .unwrap()
                        .deadline = Instant::now();
                }
            }
            resume.send(()).unwrap();
            let result = task.await.unwrap();
            if action == "expiry" {
                let p = result.unwrap();
                assert_eq!(p.state, AuthState::Expired);
                assert!(p.credential.is_none());
            } else {
                let mut error = result.unwrap_err();
                assert_eq!(error.code, ErrorCode::Conflict);
                assert!(error.take_caller_credential_update().is_none());
            }
            if existing && action != "logout" {
                assert_eq!(read(&store, "A").native().session.token, "old-token");
            } else {
                assert!(store.values.lock().unwrap().is_empty());
            }
            assert!(
                f.provider
                    .qr_transactions
                    .lock()
                    .unwrap()
                    .entries
                    .is_empty()
            );
            f.requests.await.unwrap();
        }
    }
}

#[tokio::test]
async fn qr_commit_uses_conditional_first_write_and_exact_previous_snapshot() {
    for existing in [false, true] {
        let (last, resume) = paused(profile("111"));
        let mut f = server(vec![
            created().into(),
            authorized().into(),
            exchange("111", "qr-next").into(),
            last,
        ])
        .await;
        let store = Arc::new(Store::default());
        if existing {
            store
                .put(&credential("111", "initial").stored("A").unwrap())
                .unwrap();
        }
        f.provider.credential_store = Some(store.clone());
        let start = f
            .provider
            .start_qr_login_with_mode(None, CredentialMode::Both)
            .await
            .unwrap();
        let id = start.provider_transaction_id;
        let provider = f.provider.clone();
        let task = tokio::spawn(async move {
            provider
                .poll_qr_login_with_mode(&id, "A", CredentialMode::Both)
                .await
        });
        for _ in 0..4 {
            f.seen.recv().await.unwrap();
        }
        let winner = credential("222", "concurrent-winner");
        store.put(&winner.stored("A").unwrap()).unwrap();
        resume.send(()).unwrap();
        let mut error = task.await.unwrap().unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert!(error.take_caller_credential_update().is_none());
        assert_eq!(read(&store, "A"), winner);
        f.requests.await.unwrap();
    }
}

#[tokio::test]
async fn cancelled_completion_future_releases_busy_state_and_consumed_transaction() {
    let (last, resume) = paused(profile("111"));
    let f = server(vec![
        created().into(),
        authorized().into(),
        exchange("111", "next").into(),
        last,
    ])
    .await;
    let mut f = f;
    let start = f
        .provider
        .start_qr_login_with_mode(None, CredentialMode::Client)
        .await
        .unwrap();
    let id = start.provider_transaction_id.clone();
    let provider = f.provider.clone();
    let task = tokio::spawn(async move {
        provider
            .poll_qr_login_with_mode(&id, "default", CredentialMode::Client)
            .await
    });
    for _ in 0..4 {
        f.seen.recv().await.unwrap();
    }
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(
        f.provider
            .qr_transactions
            .lock()
            .unwrap()
            .entries
            .is_empty()
    );
    let _ = resume.send(());
    f.requests.await.unwrap();
}

#[tokio::test]
async fn qr_failures_never_replace_existing_login_or_export_half_finished_credentials() {
    for fail_store in [false, true] {
        let last = if fail_store {
            profile("111")
        } else {
            profile("999")
        };
        let mut f = server(vec![
            created().into(),
            authorized().into(),
            exchange("111", "next").into(),
            last.into(),
        ])
        .await;
        let store = Arc::new(Store::default());
        let old = credential("111", "old");
        store.put(&old.stored("A").unwrap()).unwrap();
        store.fail.store(fail_store, Ordering::SeqCst);
        f.provider.credential_store = Some(store.clone());
        let start = f
            .provider
            .start_qr_login_with_mode(None, CredentialMode::Both)
            .await
            .unwrap();
        let mut failure = f
            .provider
            .poll_qr_login_with_mode(&start.provider_transaction_id, "A", CredentialMode::Both)
            .await
            .unwrap_err();
        assert!(failure.take_caller_credential_update().is_none());
        assert_eq!(read(&store, "A"), old);
        assert!(
            f.provider
                .qr_transactions
                .lock()
                .unwrap()
                .entries
                .is_empty()
        );
        f.requests.await.unwrap();
    }
}

#[tokio::test]
async fn qr_capacity_reservations_and_expired_entries_have_bounded_lifetimes() {
    let (first, resume) = paused(created());
    let mut f = server(vec![first, created().into()]).await;
    {
        let mut state = f.provider.qr_transactions.lock().unwrap();
        state.creating = CAPACITY - 1;
    }
    let provider = f.provider.clone();
    let task = tokio::spawn(async move {
        provider
            .start_qr_login_with_mode(None, CredentialMode::Client)
            .await
    });
    f.seen.recv().await.unwrap();
    assert_eq!(
        f.provider.qr_transactions.lock().unwrap().creating,
        CAPACITY
    );
    assert_eq!(
        f.provider
            .start_qr_login_with_mode(None, CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(
        f.provider.qr_transactions.lock().unwrap().creating,
        CAPACITY - 1
    );
    let _ = resume.send(());
    f.provider.qr_transactions.lock().unwrap().creating = 0;
    let start = f
        .provider
        .start_qr_login_with_mode(None, CredentialMode::Client)
        .await
        .unwrap();
    f.provider
        .qr_transactions
        .lock()
        .unwrap()
        .entries
        .get_mut(&start.provider_transaction_id)
        .unwrap()
        .deadline = Instant::now();
    assert_eq!(
        f.provider
            .poll_qr_login_with_mode(
                &start.provider_transaction_id,
                "default",
                CredentialMode::Client
            )
            .await
            .unwrap()
            .state,
        AuthState::Expired
    );
    assert!(
        f.provider
            .qr_transactions
            .lock()
            .unwrap()
            .entries
            .is_empty()
    );
    f.requests.await.unwrap();
}

#[tokio::test]
async fn unavailable_login_modes_fail_before_any_network_or_store_write() {
    let f = server(vec![]).await;
    assert!(f.provider.start_qr_login(None).await.is_err());
    for kind in ["password", " standard", ""] {
        assert!(
            f.provider
                .start_qr_login_with_mode(Some(kind), CredentialMode::Client)
                .await
                .is_err()
        );
    }
    assert!(f.provider.supports(Capability::PasswordLogin));
    assert!(f.provider.supports(Capability::UserMembership));
    assert!(f.provider.supports(Capability::UserMembershipClientInfo));
    assert!(
        f.provider
            .poll_qr_login_with_mode("invalid", "default", CredentialMode::Client)
            .await
            .is_err()
    );
    assert!(f.requests.await.unwrap().is_empty());
}
