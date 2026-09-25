use super::super::tests::{Store, received, replies, seed, setup, stored};
use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{
        management::tests::{create, delete, flow},
        tests as fixture,
    },
};
use serde_json::Value;
use std::sync::atomic::Ordering;
use tokio::sync::Notify;

async fn run(p: &KuwoProvider, is_create: bool, account: Option<&str>) -> Result<Value> {
    if is_create {
        p.create_playlist(&create(account))
            .await
            .map(|v| serde_json::to_value(v).unwrap())
    } else {
        p.delete_playlists(&delete(&["101", "102"], account))
            .await
            .map(|v| serde_json::to_value(v).unwrap())
    }
}

#[tokio::test]
async fn native_management_provider_default_named_and_caller_sources_stay_isolated() {
    for is_create in [true, false] {
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
            let mut f = setup(replies(flow(is_create)), store).await;
            let p = if mode == "caller" {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let result = run(&p, is_create, (mode == "named").then_some(account))
                .await
                .unwrap();
            assert_eq!(result["extensions"]["confirmed"], true);
            assert_eq!(result["extensions"]["library_owner_id"], "42");
            assert_eq!(*f.store.values.lock().unwrap(), original);
            if mode == "caller" {
                assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
            }
            assert!(p.take_response_credential().unwrap().is_none());
            for request in fixture::requests(&mut f.network, 4).await {
                assert!(!request.contains("other-session"));
            }
        }
    }
}

#[tokio::test]
async fn native_management_provider_rejects_missing_or_mixed_sources_without_network() {
    let store = Arc::new(Store::default());
    let selected = seed(&store, "personal", "42", "selected-session");
    let mut f = setup(vec![], store).await;
    let caller = f
        .provider
        .caller_scope(&selected.caller().unwrap())
        .unwrap();
    for is_create in [true, false] {
        assert_eq!(
            run(&f.provider, is_create, None).await.unwrap_err().code,
            ErrorCode::AuthenticationRequired
        );
        assert_eq!(
            run(&caller, is_create, Some("personal"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    fixture::requests(&mut f.network, 0).await;
}

#[tokio::test]
async fn native_management_every_late_success_and_failure_observes_logout_or_relogin() {
    for is_create in [true, false] {
        for boundary in 0..4 {
            for fail in [false, true] {
                for logout in [false, true] {
                    let store = Arc::new(Store::default());
                    seed(&store, "personal", "42", "selected-session");
                    let other = seed(&store, "other", "7", "other-session")
                        .stored("other")
                        .unwrap();
                    let gate = Arc::new(Notify::new());
                    let mut f = setup(
                        flow(is_create)
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
                    let task =
                        tokio::spawn(async move { run(&p, is_create, Some("personal")).await });
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
                    assert_eq!(error.details.get("write_outcome").is_some(), boundary >= 2);
                    if boundary >= 2 {
                        assert_eq!(error.details["write_outcome"], "unconfirmed");
                        assert!(!error.retryable);
                    }
                    assert_eq!(stored(&f.store, "personal"), expected);
                    assert_eq!(stored(&f.store, "other"), Some(other));
                    assert!(f.network.seen.try_recv().is_err());
                }
            }
        }
    }
}

#[tokio::test]
async fn native_management_auth_expiry_clears_only_original_owner_and_preserves_write_uncertainty()
{
    for is_create in [true, false] {
        for caller in [false, true] {
            for boundary in 0..4 {
                let store = Arc::new(Store::default());
                let selected = seed(&store, "personal", "42", "selected-session");
                seed(&store, "other", "7", "other-session");
                let original = store.values.lock().unwrap().clone();
                let mut bodies = flow(is_create);
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
                let error = run(&p, is_create, (!caller).then_some("personal"))
                    .await
                    .unwrap_err();
                assert_eq!(error.code, ErrorCode::AuthenticationRequired);
                assert_eq!(error.details.get("write_outcome").is_some(), boundary >= 2);
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
}

#[tokio::test]
async fn native_management_business_rejection_keeps_credentials_and_never_retries_writes() {
    for is_create in [true, false] {
        let store = Arc::new(Store::default());
        seed(&store, "personal", "42", "selected-session");
        let original = store.values.lock().unwrap().clone();
        let mut bodies = flow(is_create);
        bodies[2] = json_response(&json!({"errcode":603,"message":"selected-session"}));
        bodies.truncate(3);
        let mut f = setup(replies(bodies), store).await;
        let error = run(&f.provider, is_create, Some("personal"))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert_eq!(error.details["write_outcome"], "unconfirmed");
        assert!(!error.retryable);
        assert_eq!(*f.store.values.lock().unwrap(), original);
        assert!(!format!("{error:?}").contains("selected-session"));
        fixture::requests(&mut f.network, 3).await;
    }
}

#[tokio::test]
async fn native_management_cancel_timeout_and_caller_discard_never_trigger_followup_writes() {
    for is_create in [true, false] {
        for boundary in 0..4 {
            for mode in ["cancel", "timeout", "caller-discard"] {
                let store = Arc::new(Store::default());
                let selected = seed(&store, "personal", "42", "selected-session");
                let original = store.values.lock().unwrap().clone();
                let gate = Arc::new(Notify::new());
                let mut f = setup(
                    flow(is_create)
                        .into_iter()
                        .enumerate()
                        .map(|(i, b)| (b, (i == boundary).then(|| gate.clone())))
                        .collect(),
                    store,
                )
                .await;
                let caller = mode == "caller-discard";
                let p = if caller {
                    f.provider
                        .caller_scope(&selected.caller().unwrap())
                        .unwrap()
                } else {
                    f.provider.clone()
                };
                let worker = p.clone();
                let task = tokio::spawn(async move {
                    run(&worker, is_create, (!caller).then_some("personal")).await
                });
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
                    let error = task.await.unwrap().unwrap_err();
                    assert_eq!(
                        error.code,
                        if caller {
                            ErrorCode::Conflict
                        } else {
                            ErrorCode::UpstreamTimeout
                        }
                    );
                    assert_eq!(error.details.get("write_outcome").is_some(), boundary >= 2);
                    if boundary >= 2 {
                        assert!(!error.retryable);
                    }
                }
                assert_eq!(*f.store.values.lock().unwrap(), original);
                assert!(f.network.seen.try_recv().is_err());
            }
        }
    }
}

#[tokio::test]
async fn native_management_reverse_account_completion_keeps_both_readbacks_independent() {
    let store = Arc::new(Store::default());
    seed(&store, "A", "42", "session-A");
    seed(&store, "B", "43", "session-B");
    let gate = Arc::new(Notify::new());
    let mut a = setup(
        flow(true)
            .into_iter()
            .enumerate()
            .map(|(i, b)| (b, (i == 3).then(|| gate.clone())))
            .collect(),
        store.clone(),
    )
    .await;
    let mut b = setup(replies(flow(false)), store).await;
    let p = a.provider.clone();
    let task = tokio::spawn(async move { run(&p, true, Some("A")).await });
    for _ in 0..4 {
        received(&mut a).await;
    }
    let result_b = run(&b.provider, false, Some("B")).await.unwrap();
    gate.notify_one();
    let result_a = task.await.unwrap().unwrap();
    assert_eq!(result_a["extensions"]["library_owner_id"], "42");
    assert_eq!(result_b["extensions"]["library_owner_id"], "43");
    for r in fixture::requests(&mut b.network, 4).await {
        assert!(!r.contains("session-A"));
        assert!(r.contains("session-B"));
    }
    assert!(stored(&a.store, "A").is_some());
    assert!(stored(&a.store, "B").is_some());
}
