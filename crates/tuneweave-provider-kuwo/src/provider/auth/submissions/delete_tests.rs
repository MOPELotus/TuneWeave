use super::super::tests::{Store, received, replies, seed, setup, stored};
use super::*;
use crate::client::{
    catalog::tests::response,
    native::{
        management::submission_delete::tests::{delete_at, flow, request},
        submissions::Limits,
        tests as fixture,
    },
};
use std::sync::atomic::Ordering;
use tokio::sync::Notify;

#[tokio::test]
async fn native_submission_delete_sources_preserve_credentials_and_never_read_server_for_caller() {
    for present in [false, true] {
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
            let bodies = flow("42", present);
            let count = bodies.len();
            let mut f = setup(replies(bodies), store).await;
            let p = if mode == "caller" {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let value = p
                .delete_playlist_submission_records(
                    "101",
                    &request((mode == "named").then_some(account)),
                )
                .await
                .unwrap();
            assert!(value.confirmed);
            assert_eq!(value.owned_playlist_present, present);
            assert_eq!(value.extensions["source_user_id"], "42");
            assert_eq!(*f.store.values.lock().unwrap(), original);
            assert!(p.take_response_credential().unwrap().is_none());
            if mode == "caller" {
                assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
            }
            assert!(
                fixture::requests(&mut f.network, count)
                    .await
                    .iter()
                    .all(|v| !v.contains("other-session"))
            );
        }
    }
}

#[tokio::test]
async fn native_submission_delete_every_late_success_and_error_preserves_original_generation_and_write_history()
 {
    for present in [false, true] {
        for caller in [false, true] {
            for boundary in 0..flow("42", present).len() {
                for change in ["logout", "relogin", "switch"] {
                    for failure in [false, true] {
                        let store = Arc::new(Store::default());
                        let selected = seed(&store, "personal", "42", "selected-session");
                        let other = seed(&store, "other", "7", "other-session")
                            .stored("other")
                            .unwrap();
                        store.forbid_reads.store(caller, Ordering::SeqCst);
                        let gate = Arc::new(Notify::new());
                        let mut bodies = flow("42", present);
                        bodies.truncate(boundary + 1);
                        if failure {
                            bodies[boundary] = response(401, "application/json", "", b"private");
                        }
                        let mut f = setup(
                            bodies
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
                        let task = tokio::spawn(async move {
                            worker
                                .delete_playlist_submission_records(
                                    "101",
                                    &request((!caller).then_some("personal")),
                                )
                                .await
                        });
                        for _ in 0..=boundary {
                            received(&mut f).await;
                        }
                        if caller {
                            *p.caller_credential.as_ref().unwrap().lock().unwrap() = match change {
                                "logout" => None,
                                "relogin" => {
                                    Some(fixture::credential_fixture("42", "selected-session"))
                                }
                                _ => Some(fixture::credential_fixture("43", "replacement-session")),
                            };
                        } else if change == "logout" {
                            p.logout("personal").await.unwrap();
                        } else {
                            seed(
                                &f.store,
                                "personal",
                                if change == "switch" { "43" } else { "42" },
                                if change == "switch" {
                                    "replacement-session"
                                } else {
                                    "selected-session"
                                },
                            );
                        }
                        let server_after = f.store.values.lock().unwrap().clone();
                        let caller_after = p
                            .caller_credential
                            .as_ref()
                            .map(|c| c.lock().unwrap().clone());
                        gate.notify_one();
                        let e = task.await.unwrap().unwrap_err();
                        assert_eq!(e.code, ErrorCode::Conflict);
                        assert_progress(&e, boundary, present, failure);
                        assert_eq!(*f.store.values.lock().unwrap(), server_after);
                        assert_eq!(stored(&f.store, "other"), Some(other));
                        assert_eq!(
                            p.caller_credential
                                .as_ref()
                                .map(|c| c.lock().unwrap().clone()),
                            caller_after
                        );
                        assert!(p.take_response_credential().unwrap().is_none());
                        (&mut f.network.server).await.unwrap();
                        assert!(f.network.seen.try_recv().is_err());
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn native_submission_delete_cancellation_at_every_boundary_never_continues_a_write() {
    for present in [false, true] {
        for caller in [false, true] {
            for boundary in 0..flow("42", present).len() {
                let store = Arc::new(Store::default());
                let selected = seed(&store, "personal", "42", "selected-session");
                let original = store.values.lock().unwrap().clone();
                store.forbid_reads.store(caller, Ordering::SeqCst);
                let gate = Arc::new(Notify::new());
                let mut bodies = flow("42", present);
                bodies.truncate(boundary + 1);
                let mut f = setup(
                    bodies
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
                let task = tokio::spawn(async move {
                    worker
                        .delete_playlist_submission_records(
                            "101",
                            &request((!caller).then_some("personal")),
                        )
                        .await
                });
                for _ in 0..=boundary {
                    received(&mut f).await;
                }
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                gate.notify_one();
                (&mut f.network.server).await.unwrap();
                assert_eq!(*f.store.values.lock().unwrap(), original);
                assert!(p.take_response_credential().unwrap().is_none());
                assert!(f.network.seen.try_recv().is_err());
            }
        }
    }
}

fn assert_progress(e: &TuneWeaveError, boundary: usize, present: bool, failure: bool) {
    let dispatched = delete_at(present);
    if boundary < dispatched {
        assert!(e.details.get("write_requests_dispatched").is_none());
    } else {
        assert_eq!(e.details["write_requests_dispatched"], 1);
        assert_eq!(
            e.details["record_delete_outcome"],
            if boundary > dispatched + 2 {
                "confirmed"
            } else if boundary > dispatched || !failure {
                "acknowledged"
            } else {
                "unconfirmed"
            }
        );
        assert!(!e.retryable);
    }
}

#[tokio::test]
async fn native_submission_delete_total_deadline_is_deterministic_at_every_network_boundary() {
    for present in [false, true] {
        for caller in [false, true] {
            for boundary in 0..flow("42", present).len() {
                let store = Arc::new(Store::default());
                let selected = seed(&store, "personal", "42", "selected-session");
                let original = store.values.lock().unwrap().clone();
                store.forbid_reads.store(caller, Ordering::SeqCst);
                let gate = Arc::new(Notify::new());
                let mut bodies = flow("42", present);
                bodies.truncate(boundary + 1);
                let mut f = setup(
                    bodies
                        .into_iter()
                        .enumerate()
                        .map(|(i, b)| (b, (i == boundary).then(|| gate.clone())))
                        .collect(),
                    store,
                )
                .await;
                fixture::set_request_timeout(&mut f.provider.client, Duration::from_secs(20));
                f.provider.client.native_submission_limits = Some(Limits {
                    budget: Duration::from_secs(15),
                    ..Limits::default()
                });
                let p = if caller {
                    f.provider
                        .caller_scope(&selected.caller().unwrap())
                        .unwrap()
                } else {
                    f.provider.clone()
                };
                let worker = p.clone();
                let task = tokio::spawn(async move {
                    worker
                        .delete_playlist_submission_records(
                            "101",
                            &request((!caller).then_some("personal")),
                        )
                        .await
                });
                for _ in 0..=boundary {
                    received(&mut f).await;
                }
                tokio::time::pause();
                let result = task.await;
                tokio::time::resume();
                let e = result.unwrap().unwrap_err();
                assert_eq!(e.code, ErrorCode::UpstreamTimeout);
                assert!(e.message.contains("total deadline"));
                assert_progress(&e, boundary, present, true);
                gate.notify_one();
                (&mut f.network.server).await.unwrap();
                assert_eq!(*f.store.values.lock().unwrap(), original);
                assert!(p.take_response_credential().unwrap().is_none());
                assert!(f.network.seen.try_recv().is_err());
            }
        }
    }
}

#[tokio::test]
async fn native_submission_delete_auth_failures_clear_only_the_original_selected_source() {
    for present in [false, true] {
        for caller in [false, true] {
            for boundary in 0..flow("42", present).len() {
                let store = Arc::new(Store::default());
                let selected = seed(&store, "personal", "42", "selected-session");
                seed(&store, "other", "7", "other-session");
                let original = store.values.lock().unwrap().clone();
                store.forbid_reads.store(caller, Ordering::SeqCst);
                let mut bodies = flow("42", present);
                bodies.truncate(boundary + 1);
                bodies[boundary] = response(401, "application/json", "", b"private");
                let mut f = setup(replies(bodies), store).await;
                let p = if caller {
                    f.provider
                        .caller_scope(&selected.caller().unwrap())
                        .unwrap()
                } else {
                    f.provider.clone()
                };
                let e = p
                    .delete_playlist_submission_records(
                        "101",
                        &request((!caller).then_some("personal")),
                    )
                    .await
                    .unwrap_err();
                assert_eq!(e.code, ErrorCode::AuthenticationRequired);
                assert_progress(&e, boundary, present, true);
                if caller {
                    assert_eq!(*f.store.values.lock().unwrap(), original);
                    assert!(
                        p.caller_credential
                            .as_ref()
                            .unwrap()
                            .lock()
                            .unwrap()
                            .is_none()
                    );
                    assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
                } else {
                    assert!(stored(&f.store, "personal").is_none());
                    assert!(stored(&f.store, "other").is_some());
                }
                assert!(p.take_response_credential().unwrap().is_none());
                fixture::requests(&mut f.network, boundary + 1).await;
            }
        }
    }
}

#[tokio::test]
async fn native_submission_delete_accounts_finishing_in_reverse_keep_separate_receipts() {
    let store = Arc::new(Store::default());
    seed(&store, "A", "42", "session-A");
    seed(&store, "B", "43", "session-B");
    let original = store.values.lock().unwrap().clone();
    let gate = Arc::new(Notify::new());
    let mut a = setup(
        flow("42", true)
            .into_iter()
            .enumerate()
            .map(|(i, b)| (b, (i == 19).then(|| gate.clone())))
            .collect(),
        store.clone(),
    )
    .await;
    let mut b = setup(replies(flow("43", false)), store).await;
    let p = a.provider.clone();
    let task = tokio::spawn(async move {
        p.delete_playlist_submission_records("101", &request(Some("A")))
            .await
    });
    for _ in 0..20 {
        received(&mut a).await;
    }
    let second = b
        .provider
        .delete_playlist_submission_records("101", &request(Some("B")))
        .await
        .unwrap();
    gate.notify_one();
    let first = task.await.unwrap().unwrap();
    assert_eq!(first.extensions["source_user_id"], "42");
    assert_eq!(second.extensions["source_user_id"], "43");
    assert_ne!(
        first.extensions["records_snapshot_id"],
        second.extensions["records_snapshot_id"]
    );
    assert_eq!(*a.store.values.lock().unwrap(), original);
    (&mut a.network.server).await.unwrap();
    fixture::requests(&mut b.network, 10).await;
}
