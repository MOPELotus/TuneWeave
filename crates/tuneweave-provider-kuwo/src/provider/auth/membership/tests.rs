use super::super::tests::{Store, received, replies, seed, setup, stored, valid};
use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{
        membership::tests::{body, encrypted, reply},
        tests as fixture,
    },
};
use std::sync::atomic::Ordering;
use tokio::sync::Notify;

#[tokio::test]
async fn native_membership_summary_and_client_info_preserve_selected_ownership_and_storage() {
    for caller in [false, true] {
        for detailed in [false, true] {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session");
            seed(&store, "default", "43", "other-session");
            let original = store.values.lock().unwrap().clone();
            store.forbid_reads.store(caller, Ordering::SeqCst);
            let mut f = setup(replies(vec![valid(), reply()]), store).await;
            let p = if caller {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let account = (!caller).then_some("personal");
            let result = if detailed {
                p.user_membership_client_info(Some("42"), account).await
            } else {
                p.user_membership(None, account).await
            };
            let summary = result.unwrap();
            assert_eq!(summary.user_ref.unwrap().id(), "42");
            assert_eq!(summary.active, Some(true));
            assert_eq!(*f.store.values.lock().unwrap(), original);
            assert!(p.take_response_credential().unwrap().is_none());
            if caller {
                assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
            }
            let req = fixture::requests(&mut f.network, 2).await;
            assert!(req[1].contains("uid=42&sid=selected-session"));
        }
    }
}

#[tokio::test]
async fn native_membership_rejects_other_users_missing_alias_and_metadata_injection_before_io() {
    let store = Arc::new(Store::default());
    let selected = seed(&store, "personal", "42", "selected-session");
    seed(&store, "invalid", "42", "session,loginUid=43");
    let f = setup(vec![], store).await;
    for detailed in [false, true] {
        for (id, account, code) in [
            (Some("43"), Some("personal"), ErrorCode::PermissionDenied),
            (None, Some("missing"), ErrorCode::AuthenticationRequired),
            (None, Some("invalid"), ErrorCode::InvalidRequest),
            (None, None, ErrorCode::AuthenticationRequired),
        ] {
            let result = if detailed {
                f.provider.user_membership_client_info(id, account).await
            } else {
                f.provider.user_membership(id, account).await
            };
            assert_eq!(result.unwrap_err().code, code);
        }
    }
    let caller = f
        .provider
        .caller_scope(&selected.caller().unwrap())
        .unwrap();
    assert_eq!(
        caller
            .user_membership(None, Some("personal"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
}

#[tokio::test]
async fn native_membership_late_success_and_errors_never_overwrite_logout_or_relogin() {
    for boundary in 0..2 {
        for logout in [false, true] {
            for failure in [false, true] {
                let store = Arc::new(Store::default());
                seed(&store, "personal", "42", "selected-session");
                let other = seed(&store, "other", "7", "other-session")
                    .stored("other")
                    .unwrap();
                let gate = Arc::new(Notify::new());
                let mut f = setup(
                    [valid(), reply()]
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
                let p = f.provider.clone();
                let task =
                    tokio::spawn(async move { p.user_membership(None, Some("personal")).await });
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
async fn native_membership_auth_failures_clear_only_the_unchanged_selected_source() {
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
            let p = if caller {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            assert_eq!(
                p.user_membership_client_info(None, (!caller).then_some("personal"))
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::AuthenticationRequired
            );
            if caller {
                assert!(
                    p.caller_credential
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
async fn native_membership_bad_business_data_is_not_logout_and_never_delivers_partial_membership() {
    let mut drift = body();
    drift["data"]["uid"] = json!(43);
    for b in [
        encrypted(&drift),
        encrypted(&json!({"meta":{"code":200},"ctime":1700000000000_u64,"data":{}})),
        encrypted(&json!({"meta":{"code":500},"data":null})),
        response(403, "application/json", "", b"private-error"),
    ] {
        let store = Arc::new(Store::default());
        let original = seed(&store, "personal", "42", "selected-session")
            .stored("personal")
            .unwrap();
        let mut f = setup(replies(vec![valid(), b]), store).await;
        assert!(
            f.provider
                .user_membership(None, Some("personal"))
                .await
                .is_err()
        );
        assert_eq!(stored(&f.store, "personal"), Some(original));
        fixture::requests(&mut f.network, 2).await;
    }
}

#[tokio::test]
async fn native_membership_cancel_and_timeout_preserve_credentials_at_both_boundaries() {
    for boundary in 0..2 {
        for cancel in [false, true] {
            let store = Arc::new(Store::default());
            let original = seed(&store, "personal", "42", "selected-session")
                .stored("personal")
                .unwrap();
            let gate = Arc::new(Notify::new());
            let mut f = setup(
                [valid(), reply()]
                    .into_iter()
                    .enumerate()
                    .map(|(i, b)| (b, (i == boundary).then(|| gate.clone())))
                    .collect(),
                store,
            )
            .await;
            let p = f.provider.clone();
            let task = tokio::spawn(async move { p.user_membership(None, Some("personal")).await });
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
            assert_eq!(stored(&f.store, "personal"), Some(original));
            assert!(f.network.seen.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn native_membership_parallel_accounts_and_caller_invalidation_cannot_exchange_data() {
    let store = Arc::new(Store::default());
    seed(&store, "A", "42", "session-A");
    seed(&store, "B", "43", "session-B");
    let gate = Arc::new(Notify::new());
    let mut a = setup(
        vec![(valid(), None), (reply(), Some(gate.clone()))],
        store.clone(),
    )
    .await;
    let mut data_b = body();
    data_b["data"]["uid"] = json!(43);
    data_b["data"]["vipmExpire"] = json!(0);
    data_b["data"]["vipLuxuryExpire"] = json!(0);
    let mut b = setup(replies(vec![valid(), encrypted(&data_b)]), store).await;
    let p = a.provider.clone();
    let task = tokio::spawn(async move { p.user_membership(None, Some("A")).await });
    received(&mut a).await;
    received(&mut a).await;
    let summary_b = b.provider.user_membership(None, Some("B")).await.unwrap();
    assert_eq!(summary_b.active, Some(false));
    assert_eq!(summary_b.user_ref.unwrap().id(), "43");
    gate.notify_one();
    assert_eq!(task.await.unwrap().unwrap().active, Some(true));
    fixture::requests(&mut b.network, 2).await;
    for boundary in 0..2 {
        let gate = Arc::new(Notify::new());
        let mut f = setup(
            [valid(), reply()]
                .into_iter()
                .enumerate()
                .map(|(i, b)| (b, (i == boundary).then(|| gate.clone())))
                .collect(),
            Arc::new(Store::default()),
        )
        .await;
        let p = f
            .provider
            .caller_scope(
                &fixture::credential_fixture("42", "caller-session")
                    .caller()
                    .unwrap(),
            )
            .unwrap();
        let same = p.clone();
        let task = tokio::spawn(async move { p.user_membership(None, None).await });
        for _ in 0..=boundary {
            received(&mut f).await;
        }
        *same.caller_credential.as_ref().unwrap().lock().unwrap() = None;
        gate.notify_one();
        assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
        assert!(
            same.caller_credential
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .is_none()
        );
    }
}
