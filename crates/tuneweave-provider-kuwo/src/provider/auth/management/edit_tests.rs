use super::super::tests::{Store, received, replies, seed, setup, stored};
use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{
        management::edit::tests::{flow, request},
        tests as fixture,
    },
};
use std::sync::atomic::Ordering;
use tokio::sync::Notify;

#[tokio::test]
async fn native_edit_default_named_and_caller_account_sources_remain_separate() {
    for mode in ["default", "named", "caller"] {
        let account = if mode == "named" {
            "personal"
        } else {
            "default"
        };
        let store = Arc::new(Store::default());
        let selected = seed(&store, account, "42", "selected-session");
        seed(&store, "other", "7", "other-session");
        let original = store.values.lock().unwrap().clone();
        store.forbid_reads.store(mode == "caller", Ordering::SeqCst);
        let r = request((mode == "named").then_some(account));
        let mut f = setup(replies(flow("42", false, &r)), store).await;
        let p = if mode == "caller" {
            f.provider
                .caller_scope(&selected.caller().unwrap())
                .unwrap()
        } else {
            f.provider.clone()
        };
        let result = p.update_playlist("101", &r).await.unwrap();
        assert_eq!(
            result.action,
            tuneweave_core::PlaylistMutationAction::Update
        );
        assert_eq!(result.extensions["library_owner_id"], "42");
        assert_eq!(*f.store.values.lock().unwrap(), original);
        if mode == "caller" {
            assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
        }
        assert!(p.take_response_credential().unwrap().is_none());
        for r in fixture::requests(&mut f.network, 7).await {
            assert!(!r.contains("other-session"));
        }
    }
}

#[tokio::test]
async fn native_edit_every_late_success_and_error_observes_original_generation() {
    for boundary in 0..7 {
        for fail in [false, true] {
            for logout in [false, true] {
                let store = Arc::new(Store::default());
                seed(&store, "personal", "42", "selected-session");
                let other = seed(&store, "other", "7", "other-session")
                    .stored("other")
                    .unwrap();
                let gate = Arc::new(Notify::new());
                let r = request(Some("personal"));
                let mut f = setup(
                    flow("42", false, &r)
                        .into_iter()
                        .enumerate()
                        .map(|(i, b)| {
                            (
                                if fail && i == boundary {
                                    response(401, "application/json", "", b"private")
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
                let task = tokio::spawn(async move { p.update_playlist("101", &r).await });
                for _ in 0..=boundary {
                    received(&mut f).await;
                }
                if logout {
                    f.provider.logout("personal").await.unwrap();
                } else {
                    seed(&f.store, "personal", "43", "replacement-session");
                }
                let expected = stored(&f.store, "personal");
                gate.notify_one();
                let error = task.await.unwrap().unwrap_err();
                assert_eq!(error.code, ErrorCode::Conflict);
                assert_eq!(error.details.get("write_outcome").is_some(), boundary >= 4);
                if boundary >= 4 {
                    assert!(!error.retryable);
                    assert_eq!(error.details["write_outcome"], "unconfirmed");
                }
                assert_eq!(stored(&f.store, "personal"), expected);
                assert_eq!(stored(&f.store, "other"), Some(other));
                assert!(f.network.seen.try_recv().is_err());
            }
        }
    }
}

#[tokio::test]
async fn native_edit_auth_expiry_clears_only_the_unchanged_selected_source() {
    for caller in [false, true] {
        for boundary in 0..7 {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session");
            seed(&store, "other", "7", "other-session");
            let original = store.values.lock().unwrap().clone();
            let r = request((!caller).then_some("personal"));
            let mut bodies = flow("42", false, &r);
            bodies[boundary] = response(401, "application/json", "", b"private");
            bodies.truncate(boundary + 1);
            let mut f = setup(replies(bodies), store).await;
            let p = if caller {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let error = p.update_playlist("101", &r).await.unwrap_err();
            assert_eq!(error.code, ErrorCode::AuthenticationRequired);
            assert_eq!(error.details.get("write_outcome").is_some(), boundary >= 4);
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
                assert_eq!(stored(&f.store, "other"), original.get("other").cloned());
            }
            fixture::requests(&mut f.network, boundary + 1).await;
        }
    }
}

#[tokio::test]
async fn native_edit_business_error_and_readback_change_keep_credentials_without_retry() {
    for boundary in [2, 3, 4, 5, 6] {
        let store = Arc::new(Store::default());
        seed(&store, "personal", "42", "selected-session");
        let original = store.values.lock().unwrap().clone();
        let r = request(Some("personal"));
        let mut bodies = flow("42", false, &r);
        bodies[boundary] = json_response(&json!({"errcode":603,"message":"selected-session"}));
        bodies.truncate(boundary + 1);
        let mut f = setup(replies(bodies), store).await;
        let e = f.provider.update_playlist("101", &r).await.unwrap_err();
        assert_eq!(e.code, ErrorCode::UpstreamError);
        assert_eq!(e.details.get("write_outcome").is_some(), boundary >= 4);
        assert_eq!(*f.store.values.lock().unwrap(), original);
        assert!(!format!("{e:?}").contains("selected-session"));
        fixture::requests(&mut f.network, boundary + 1).await;
    }
}

#[tokio::test]
async fn native_edit_cancel_timeout_and_caller_discard_stop_at_every_boundary() {
    for boundary in 0..7 {
        for mode in ["cancel", "timeout", "caller-discard"] {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session");
            let original = store.values.lock().unwrap().clone();
            let gate = Arc::new(Notify::new());
            let caller = mode == "caller-discard";
            let r = request((!caller).then_some("personal"));
            let mut f = setup(
                flow("42", false, &r)
                    .into_iter()
                    .enumerate()
                    .map(|(i, b)| (b, (i == boundary).then(|| gate.clone())))
                    .collect(),
                store,
            )
            .await;
            let p = if caller {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let worker = p.clone();
            let task = tokio::spawn(async move { worker.update_playlist("101", &r).await });
            for _ in 0..=boundary {
                received(&mut f).await;
            }
            if mode == "cancel" {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                if caller {
                    *p.caller_credential.as_ref().unwrap().lock().unwrap() = None;
                    gate.notify_one();
                }
                let e = task.await.unwrap().unwrap_err();
                assert_eq!(
                    e.code,
                    if caller {
                        ErrorCode::Conflict
                    } else {
                        ErrorCode::UpstreamTimeout
                    }
                );
                assert_eq!(e.details.get("write_outcome").is_some(), boundary >= 4);
                if boundary >= 4 {
                    assert!(!e.retryable);
                }
            }
            assert_eq!(*f.store.values.lock().unwrap(), original);
            assert!(f.network.seen.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn native_edit_reverse_accounts_keep_metadata_and_mutations_separate() {
    let store = Arc::new(Store::default());
    seed(&store, "A", "42", "session-A");
    seed(&store, "B", "43", "session-B");
    let gate = Arc::new(Notify::new());
    let a_request = request(Some("A"));
    let b_request = request(Some("B"));
    let mut a = setup(
        flow("42", false, &a_request)
            .into_iter()
            .enumerate()
            .map(|(i, b)| (b, (i == 6).then(|| gate.clone())))
            .collect(),
        store.clone(),
    )
    .await;
    let mut b = setup(replies(flow("43", true, &b_request)), store).await;
    let p = a.provider.clone();
    let task = tokio::spawn(async move { p.update_playlist("101", &a_request).await });
    for _ in 0..7 {
        received(&mut a).await;
    }
    let result_b = b.provider.update_playlist("101", &b_request).await.unwrap();
    gate.notify_one();
    let result_a = task.await.unwrap().unwrap();
    assert_eq!(result_a.extensions["library_owner_id"], "42");
    assert_eq!(result_b.extensions["library_owner_id"], "43");
    assert_eq!(result_a.playlist.unwrap().extensions["is_public"], false);
    assert_eq!(result_b.playlist.unwrap().extensions["is_public"], true);
    for r in fixture::requests(&mut b.network, 7).await {
        assert!(!r.contains("session-A"));
        assert!(r.contains("session-B"));
    }
}

#[tokio::test]
async fn native_submission_metadata_rejection_preserves_each_account_source() {
    for mode in ["default", "named", "caller"] {
        let account = if mode == "named" {
            "personal"
        } else {
            "default"
        };
        let store = Arc::new(Store::default());
        let selected = seed(&store, account, "42", "selected-session");
        seed(&store, "other", "7", "other-session");
        let original = store.values.lock().unwrap().clone();
        store.forbid_reads.store(mode == "caller", Ordering::SeqCst);
        let r = request((mode == "named").then_some(account));
        let mut bodies = flow("42", true, &r);
        let mut m = crate::client::native::management::edit::tests::metadata("42", None);
        m["sl_data"]["igsl"] = json!("1");
        bodies[2] = json_response(&m);
        bodies.truncate(3);
        let mut f = setup(replies(bodies), store).await;
        let p = if mode == "caller" {
            f.provider
                .caller_scope(&selected.caller().unwrap())
                .unwrap()
        } else {
            f.provider.clone()
        };
        let error = p.update_playlist("101", &r).await.unwrap_err();
        assert_eq!(error.code, ErrorCode::CapabilityNotSupported);
        assert!(error.details.get("write_outcome").is_none());
        assert_eq!(*f.store.values.lock().unwrap(), original);
        assert!(p.take_response_credential().unwrap().is_none());
        if mode == "caller" {
            assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
        }
        assert!(
            fixture::requests(&mut f.network, 3)
                .await
                .iter()
                .all(|r| r.starts_with("GET "))
        );
    }
}
