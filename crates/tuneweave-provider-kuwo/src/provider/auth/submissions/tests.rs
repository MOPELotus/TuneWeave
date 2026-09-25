use super::super::tests::{Store, received, replies, seed, setup, stored};
use super::*;
use crate::client::catalog::tests::{json_response, response};
use crate::client::native::{
    submissions::{Limits, tests::flow},
    tests as fixture,
};
use std::sync::atomic::Ordering;
use tokio::sync::Notify;

fn request(account: Option<&str>) -> PageRequest {
    PageRequest {
        limit: 2,
        offset: 5,
        account: account.map(str::to_owned),
    }
}

#[tokio::test]
async fn native_submissions_sources_preserve_credentials_and_caller_never_reads_server() {
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
        let mut f = setup(replies(flow("42", 8)), store).await;
        let p = if mode == "caller" {
            f.provider
                .caller_scope(&selected.caller().unwrap())
                .unwrap()
        } else {
            f.provider.clone()
        };
        let result = p
            .account_playlist_submissions(&request((mode == "named").then_some(account)))
            .await
            .unwrap();
        assert_eq!(result.pagination.total, Some(8));
        assert_eq!(result.items[0].playlist_ref.id(), "106");
        assert_eq!(result.pagination.extensions["source_user_id"], "42");
        assert_eq!(*f.store.values.lock().unwrap(), original);
        assert!(p.take_response_credential().unwrap().is_none());
        if mode == "caller" {
            assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
        }
        assert!(
            fixture::requests(&mut f.network, 5)
                .await
                .iter()
                .all(|r| !r.contains("other-session"))
        );
    }
}

#[tokio::test]
async fn native_submissions_late_success_and_errors_never_cross_logout_relogin_or_account_switch() {
    for caller in [false, true] {
        for boundary in 0..5 {
            for change in ["logout", "same-session-relogin", "switch"] {
                for failure in [false, true] {
                    let store = Arc::new(Store::default());
                    let selected = seed(&store, "personal", "42", "selected-session");
                    let other = seed(&store, "other", "7", "other-session")
                        .stored("other")
                        .unwrap();
                    store.forbid_reads.store(caller, Ordering::SeqCst);
                    let gate = Arc::new(Notify::new());
                    let mut bodies = flow("42", 8);
                    bodies.truncate(boundary + 1);
                    if failure {
                        bodies[boundary] =
                            response(401, "application/json", "", b"private-failure");
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
                            .account_playlist_submissions(&request((!caller).then_some("personal")))
                            .await
                    });
                    for _ in 0..=boundary {
                        received(&mut f).await;
                    }
                    if caller {
                        let next = match change {
                            "logout" => None,
                            "same-session-relogin" => {
                                Some(fixture::credential_fixture("42", "selected-session"))
                            }
                            _ => Some(fixture::credential_fixture("43", "replacement-session")),
                        };
                        *p.caller_credential.as_ref().unwrap().lock().unwrap() = next;
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
                    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
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

#[tokio::test]
async fn native_submissions_auth_failures_only_clear_the_selected_unchanged_source() {
    for caller in [false, true] {
        for boundary in 0..5 {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session");
            seed(&store, "other", "7", "other-session");
            let original = store.values.lock().unwrap().clone();
            let mut bodies = flow("42", 8);
            bodies.truncate(boundary + 1);
            bodies[boundary] = if boundary == 0 {
                response(401, "application/json", "", b"private-failure")
            } else {
                json_response(&json!({"code":-1001,"msg":"auth fail"}))
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
                p.account_playlist_submissions(&request((!caller).then_some("personal")))
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
                assert_eq!(stored(&f.store, "other"), original.get("other").cloned());
            }
            fixture::requests(&mut f.network, boundary + 1).await;
        }
    }
}

#[tokio::test]
async fn native_submissions_cancel_and_total_deadline_preserve_each_source_at_every_boundary() {
    for caller in [false, true] {
        for boundary in 0..5 {
            for cancel in [false, true] {
                let store = Arc::new(Store::default());
                let selected = seed(&store, "personal", "42", "selected-session");
                let original = store.values.lock().unwrap().clone();
                let gate = Arc::new(Notify::new());
                let mut bodies = flow("42", 8);
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
                // Longer than ordinary local scheduling; shorter than the fixture's 2s per-request timeout.
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
                        .account_playlist_submissions(&request((!caller).then_some("personal")))
                        .await
                });
                for _ in 0..=boundary {
                    received(&mut f).await;
                }
                if cancel {
                    task.abort();
                    assert!(task.await.unwrap_err().is_cancelled());
                } else {
                    let e = task.await.unwrap().unwrap_err();
                    assert_eq!(e.code, ErrorCode::UpstreamTimeout);
                    assert!(e.message.contains("total deadline"));
                }
                gate.notify_one();
                (&mut f.network.server).await.unwrap();
                assert_eq!(*f.store.values.lock().unwrap(), original);
                assert!(p.take_response_credential().unwrap().is_none());
                if caller {
                    assert_eq!(
                        p.caller_credential
                            .as_ref()
                            .unwrap()
                            .lock()
                            .unwrap()
                            .as_ref(),
                        Some(&selected)
                    );
                }
                assert!(f.network.seen.try_recv().is_err());
            }
        }
    }
}

#[tokio::test]
async fn native_submissions_parallel_accounts_keep_results_and_snapshots_separate() {
    let store = Arc::new(Store::default());
    seed(&store, "A", "42", "session-A");
    seed(&store, "B", "43", "session-B");
    let gate = Arc::new(Notify::new());
    let mut a = setup(
        flow("42", 8)
            .into_iter()
            .enumerate()
            .map(|(i, b)| (b, (i == 4).then(|| gate.clone())))
            .collect(),
        store.clone(),
    )
    .await;
    let mut b = setup(replies(flow("43", 8)), store).await;
    let p = a.provider.clone();
    let task =
        tokio::spawn(async move { p.account_playlist_submissions(&request(Some("A"))).await });
    for _ in 0..5 {
        received(&mut a).await;
    }
    let result_b = b
        .provider
        .account_playlist_submissions(&request(Some("B")))
        .await
        .unwrap();
    gate.notify_one();
    let result_a = task.await.unwrap().unwrap();
    assert_eq!(result_a.items[0].owner_id, "42");
    assert_eq!(result_b.items[0].owner_id, "43");
    assert_ne!(
        result_a.pagination.extensions["source_snapshot_id"],
        result_b.pagination.extensions["source_snapshot_id"]
    );
    (&mut a.network.server).await.unwrap();
    assert!(
        fixture::requests(&mut b.network, 5)
            .await
            .iter()
            .all(|r| !r.contains("session-A"))
    );
}

#[tokio::test]
async fn native_submissions_missing_alias_and_invalid_source_fail_before_network() {
    let store = Arc::new(Store::default());
    let selected = seed(&store, "personal", "42", "selected-session");
    let mut f = setup(vec![], store).await;
    assert_eq!(
        f.provider
            .account_playlist_submissions(&request(Some("missing")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    let caller = f
        .provider
        .caller_scope(&selected.caller().unwrap())
        .unwrap();
    assert_eq!(
        caller
            .account_playlist_submissions(&request(Some("personal")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    fixture::requests(&mut f.network, 0).await;
}
