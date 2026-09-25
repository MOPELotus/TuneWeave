use super::tests::{Store, received, replies, seed, setup, stored, valid};
use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{profile::tests::reply, tests as fixture},
};
use std::sync::atomic::Ordering;
use tokio::sync::Notify;
use tuneweave_core::UserProfileBackend;

#[tokio::test]
async fn self_profile_reads_server_and_caller_credentials_without_writes_or_cross_account_reads() {
    for caller in [false, true] {
        let store = Arc::new(Store::default());
        let selected = seed(&store, "personal", "42", "selected-session");
        seed(&store, "default", "43", "other-session");
        let before = store.values.lock().unwrap().clone();
        store.forbid_reads.store(caller, Ordering::SeqCst);
        let mut f = setup(replies(vec![valid(), reply("42")]), store).await;
        let provider = if caller {
            f.provider
                .caller_scope(&selected.caller().unwrap())
                .unwrap()
        } else {
            f.provider.clone()
        };
        let profile = provider
            .user_profile(
                "42",
                UserProfileBackend::Modern,
                (!caller).then_some("personal"),
            )
            .await
            .unwrap();
        assert_eq!(profile.user.id, "42");
        assert_eq!(profile.user.name, "听众 + %20");
        assert_eq!(profile.level, Some(3));
        assert_eq!(*f.store.values.lock().unwrap(), before);
        assert!(provider.take_response_credential().unwrap().is_none());
        if caller {
            assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
        }
        fixture::requests(&mut f.network, 2).await;
    }
}

#[tokio::test]
async fn self_profile_invalid_identity_scope_and_metadata_are_rejected_before_io() {
    let store = Arc::new(Store::default());
    seed(&store, "personal", "42", "selected-session");
    seed(&store, "invalid", "42", "session,loginUid=43");
    let f = setup(vec![], store).await;
    for (id, backend, account, expected) in [
        (
            "43",
            UserProfileBackend::Modern,
            Some("personal"),
            ErrorCode::PermissionDenied,
        ),
        (
            "42",
            UserProfileBackend::Legacy,
            Some("personal"),
            ErrorCode::CapabilityNotSupported,
        ),
        (
            "42",
            UserProfileBackend::Modern,
            None,
            ErrorCode::AuthenticationRequired,
        ),
        (
            "42",
            UserProfileBackend::Modern,
            Some("missing"),
            ErrorCode::AuthenticationRequired,
        ),
        (
            "42",
            UserProfileBackend::Modern,
            Some("invalid"),
            ErrorCode::InvalidRequest,
        ),
    ] {
        assert_eq!(
            f.provider
                .user_profile(id, backend, account)
                .await
                .unwrap_err()
                .code,
            expected
        );
    }
}

#[tokio::test]
async fn self_profile_late_success_and_error_never_cross_logout_or_relogin_at_either_boundary() {
    for boundary in 0..2 {
        for logout in [false, true] {
            for failure in [false, true] {
                let store = Arc::new(Store::default());
                seed(&store, "personal", "42", "selected-session");
                let other = seed(&store, "other", "7", "other-session")
                    .stored("other")
                    .unwrap();
                let gate = Arc::new(Notify::new());
                let bodies = [valid(), reply("42")];
                let mut f = setup(
                    bodies
                        .into_iter()
                        .enumerate()
                        .map(|(i, b)| {
                            (
                                if failure && i == boundary {
                                    response(401, "application/json", "", b"private-error")
                                } else {
                                    b
                                },
                                (i == boundary).then(|| gate.clone()),
                            )
                        })
                        .collect(),
                    store,
                )
                .await;
                let provider = f.provider.clone();
                let task = tokio::spawn(async move {
                    provider
                        .user_profile("42", UserProfileBackend::Modern, Some("personal"))
                        .await
                });
                for _ in 0..=boundary {
                    received(&mut f).await;
                }
                if logout {
                    f.provider.logout("personal").await.unwrap();
                } else {
                    seed(&f.store, "personal", "43", "replacement-session");
                }
                let after = stored(&f.store, "personal");
                gate.notify_one();
                assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
                assert_eq!(stored(&f.store, "personal"), after);
                assert_eq!(stored(&f.store, "other"), Some(other));
                assert!(f.network.seen.try_recv().is_err());
            }
        }
    }
}

#[tokio::test]
async fn self_profile_authentication_errors_clear_only_selected_unchanged_credential() {
    for caller in [false, true] {
        for boundary in 0..2 {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session");
            let original = store.values.lock().unwrap().clone();
            let bodies = if boundary == 0 {
                vec![json_response(
                    &json!({"result":"fail","reason":"error_user_invalid"}),
                )]
            } else {
                vec![
                    valid(),
                    response(401, "application/json", "", b"private-error"),
                ]
            };
            let mut f = setup(replies(bodies), store).await;
            let provider = if caller {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            assert_eq!(
                provider
                    .user_profile(
                        "42",
                        UserProfileBackend::Modern,
                        (!caller).then_some("personal")
                    )
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::AuthenticationRequired
            );
            if caller {
                assert!(
                    provider
                        .caller_credential
                        .as_ref()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .is_none()
                );
                assert_eq!(*f.store.values.lock().unwrap(), original);
            } else {
                assert!(stored(&f.store, "personal").is_none());
            }
            fixture::requests(&mut f.network, boundary + 1).await;
        }
    }
}

#[tokio::test]
async fn self_profile_unknown_business_errors_and_damaged_data_preserve_credentials() {
    for body in [
        fixture::encrypted(&json!({"status":500,"msg":"private-error"})),
        reply("43"),
        response(403, "application/json", "", b"private-error"),
        response(200, "text/html", "", b"broken"),
    ] {
        let store = Arc::new(Store::default());
        let selected = seed(&store, "personal", "42", "selected-session")
            .stored("personal")
            .unwrap();
        let mut f = setup(replies(vec![valid(), body]), store).await;
        assert!(
            f.provider
                .user_profile("42", UserProfileBackend::Modern, Some("personal"))
                .await
                .is_err()
        );
        assert_eq!(stored(&f.store, "personal"), Some(selected));
        fixture::requests(&mut f.network, 2).await;
    }
}

#[tokio::test]
async fn self_profile_cancel_and_timeout_preserve_the_original_credential() {
    for boundary in 0..2 {
        for cancel in [false, true] {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session")
                .stored("personal")
                .unwrap();
            let gate = Arc::new(Notify::new());
            let mut f = setup(
                [valid(), reply("42")]
                    .into_iter()
                    .enumerate()
                    .map(|(i, b)| (b, (i == boundary).then(|| gate.clone())))
                    .collect(),
                store,
            )
            .await;
            let provider = f.provider.clone();
            let task = tokio::spawn(async move {
                provider
                    .user_profile("42", UserProfileBackend::Modern, Some("personal"))
                    .await
            });
            for _ in 0..=boundary {
                received(&mut f).await;
            }
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                assert_eq!(
                    task.await.unwrap().unwrap_err().code,
                    ErrorCode::UpstreamTimeout
                );
            }
            assert_eq!(stored(&f.store, "personal"), Some(selected));
            assert!(f.network.seen.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn self_profile_concurrent_accounts_finishing_in_reverse_order_keep_their_own_identity() {
    let store = Arc::new(Store::default());
    seed(&store, "A", "42", "session-A");
    seed(&store, "B", "43", "session-B");
    let gate = Arc::new(Notify::new());
    let mut a = setup(
        vec![(valid(), None), (reply("42"), Some(gate.clone()))],
        store.clone(),
    )
    .await;
    let mut b = setup(replies(vec![valid(), reply("43")]), store.clone()).await;
    let provider = a.provider.clone();
    let task = tokio::spawn(async move {
        provider
            .user_profile("42", UserProfileBackend::Modern, Some("A"))
            .await
    });
    received(&mut a).await;
    received(&mut a).await;
    assert_eq!(
        b.provider
            .user_profile("43", UserProfileBackend::Modern, Some("B"))
            .await
            .unwrap()
            .user
            .id,
        "43"
    );
    gate.notify_one();
    assert_eq!(task.await.unwrap().unwrap().user.id, "42");
    fixture::requests(&mut b.network, 2).await;
    assert_eq!(store.values.lock().unwrap().len(), 2);
}
