use super::super::tests::{Store, received, replies, seed, setup, stored};
use super::*;
use crate::client::{
    catalog::tests::response,
    native::{
        management::contribution::tests::{flow, request, submit_at},
        submissions::Limits,
        tests as fixture,
    },
};
use std::sync::atomic::Ordering;
use tokio::sync::Notify;

#[tokio::test]
async fn native_contribution_sources_preserve_credentials_and_never_read_server_for_caller() {
    for edit in [false, true] {
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
            let bodies = flow("42", edit);
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
                .submit_playlist("101", &request((mode == "named").then_some(account), edit))
                .await
                .unwrap();
            assert!(value.accepted);
            assert_eq!(value.metadata_updated, edit);
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
async fn native_contribution_every_late_success_and_error_preserves_original_generation_and_write_history()
 {
    for edit in [false, true] {
        for caller in [false, true] {
            for boundary in 0..flow("42", edit).len() {
                for change in ["logout", "relogin", "switch"] {
                    for failure in [false, true] {
                        let store = Arc::new(Store::default());
                        let selected = seed(&store, "personal", "42", "selected-session");
                        let other = seed(&store, "other", "7", "other-session")
                            .stored("other")
                            .unwrap();
                        store.forbid_reads.store(caller, Ordering::SeqCst);
                        let gate = Arc::new(Notify::new());
                        let mut bodies = flow("42", edit);
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
                                .submit_playlist(
                                    "101",
                                    &request((!caller).then_some("personal"), edit),
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
                        if boundary >= submit_at(edit) {
                            assert_eq!(
                                e.details["submission_outcome"],
                                if boundary > submit_at(edit) || !failure {
                                    "accepted"
                                } else {
                                    "unconfirmed"
                                }
                            );
                        } else if edit && boundary >= 9 {
                            assert_eq!(e.details["submission_outcome"], "not_dispatched");
                        } else {
                            assert!(e.details.get("write_requests_dispatched").is_none());
                        }
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
async fn native_contribution_cancellation_at_every_boundary_never_continues_a_write() {
    for edit in [false, true] {
        for caller in [false, true] {
            for boundary in 0..flow("42", edit).len() {
                let store = Arc::new(Store::default());
                let selected = seed(&store, "personal", "42", "selected-session");
                let original = store.values.lock().unwrap().clone();
                store.forbid_reads.store(caller, Ordering::SeqCst);
                let gate = Arc::new(Notify::new());
                let mut bodies = flow("42", edit);
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
                        .submit_playlist("101", &request((!caller).then_some("personal"), edit))
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

#[tokio::test]
async fn native_contribution_total_deadline_keeps_each_write_stage_truthful() {
    for caller in [false, true] {
        for boundary in [0, 1, 9, 10, 15, 16, 17, 24] {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session");
            let original = store.values.lock().unwrap().clone();
            let gate = Arc::new(Notify::new());
            let mut bodies = flow("42", true);
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
            f.provider.client.native_submission_limits = Some(Limits {
                budget: Duration::from_secs(1),
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
                    .submit_playlist("101", &request((!caller).then_some("personal"), true))
                    .await
            });
            for _ in 0..=boundary {
                received(&mut f).await;
            }
            let e = task.await.unwrap().unwrap_err();
            assert_eq!(e.code, ErrorCode::UpstreamTimeout);
            assert!(e.message.contains("total deadline"));
            if boundary >= 9 {
                assert_eq!(
                    e.details["playlist_write_outcome"],
                    if boundary >= 16 {
                        "confirmed"
                    } else {
                        "unconfirmed"
                    }
                );
                assert_eq!(
                    e.details["submission_outcome"],
                    if boundary > 16 {
                        "accepted"
                    } else if boundary == 16 {
                        "unconfirmed"
                    } else {
                        "not_dispatched"
                    }
                );
            } else {
                assert!(e.details.get("write_requests_dispatched").is_none());
            }
            gate.notify_one();
            (&mut f.network.server).await.unwrap();
            assert_eq!(*f.store.values.lock().unwrap(), original);
            assert!(p.take_response_credential().unwrap().is_none());
            assert!(f.network.seen.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn native_contribution_auth_failures_only_clear_the_selected_original_account() {
    for caller in [false, true] {
        for boundary in [0, 1, 9, 10, 16, 17, 24] {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session");
            seed(&store, "other", "7", "other-session");
            let original = store.values.lock().unwrap().clone();
            store.forbid_reads.store(caller, Ordering::SeqCst);
            let mut bodies = flow("42", true);
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
                .submit_playlist("101", &request((!caller).then_some("personal"), true))
                .await
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::AuthenticationRequired);
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

#[tokio::test]
async fn native_contribution_accounts_finishing_in_reverse_order_keep_each_receipt_and_source() {
    let store = Arc::new(Store::default());
    seed(&store, "A", "42", "session-A");
    seed(&store, "B", "43", "session-B");
    let original = store.values.lock().unwrap().clone();
    let gate = Arc::new(Notify::new());
    let mut a = setup(
        flow("42", true)
            .into_iter()
            .enumerate()
            .map(|(i, b)| (b, (i == 24).then(|| gate.clone())))
            .collect(),
        store.clone(),
    )
    .await;
    let mut b = setup(replies(flow("43", false)), store).await;
    let p = a.provider.clone();
    let task =
        tokio::spawn(async move { p.submit_playlist("101", &request(Some("A"), true)).await });
    for _ in 0..25 {
        received(&mut a).await;
    }
    let second = b
        .provider
        .submit_playlist("101", &request(Some("B"), false))
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
    fixture::requests(&mut b.network, 18).await;
}
