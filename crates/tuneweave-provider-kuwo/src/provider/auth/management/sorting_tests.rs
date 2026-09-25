use super::super::tests::{Store, received, replies, seed, setup, stored};
use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{
        management::sorting::tests::{case, flow, request},
        tests as fixture,
    },
};
use std::sync::atomic::Ordering;
use tokio::sync::Notify;

#[tokio::test]
async fn native_track_sort_default_named_and_caller_account_sources_remain_separate() {
    for favorite in [false, true] {
        let (id, boundaries, _) = case(favorite);
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
            let mut f = setup(replies(flow("42", favorite)), store).await;
            let p = if mode == "caller" {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let result = p.reorder_playlist_tracks(id, &r).await.unwrap();
            assert_eq!(result.track_refs, r.track_refs);
            assert_eq!(result.playlist_ref.id(), id);
            assert_eq!(result.extensions["library_owner_id"], "42");
            assert_eq!(*f.store.values.lock().unwrap(), original);
            if mode == "caller" {
                assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
            }
            assert!(p.take_response_credential().unwrap().is_none());
            for r in fixture::requests(&mut f.network, boundaries).await {
                assert!(!r.contains("other-session"));
            }
        }
    }
}

#[tokio::test]
async fn native_track_sort_every_late_success_and_error_observes_original_generation() {
    for favorite in [false, true] {
        let (id, boundaries, write_at) = case(favorite);
        for boundary in 0..boundaries {
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
                        flow("42", favorite)
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
                    let task = tokio::spawn(async move { p.reorder_playlist_tracks(id, &r).await });
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
                    assert_eq!(
                        error.details.get("write_outcome").is_some(),
                        boundary >= write_at
                    );
                    if boundary >= write_at {
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
}

#[tokio::test]
async fn native_track_sort_auth_expiry_clears_only_the_unchanged_selected_source() {
    for favorite in [false, true] {
        let (id, boundaries, write_at) = case(favorite);
        for caller in [false, true] {
            for boundary in 0..boundaries {
                let store = Arc::new(Store::default());
                let selected = seed(&store, "personal", "42", "selected-session");
                seed(&store, "other", "7", "other-session");
                let original = store.values.lock().unwrap().clone();
                let r = request((!caller).then_some("personal"));
                let mut bodies = flow("42", favorite);
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
                let error = p.reorder_playlist_tracks(id, &r).await.unwrap_err();
                assert_eq!(error.code, ErrorCode::AuthenticationRequired);
                assert_eq!(
                    error.details.get("write_outcome").is_some(),
                    boundary >= write_at
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
}

#[tokio::test]
async fn native_track_sort_business_error_and_readback_change_keep_credentials_without_retry() {
    for favorite in [false, true] {
        let (id, boundaries, write_at) = case(favorite);
        for boundary in 1..boundaries {
            let store = Arc::new(Store::default());
            seed(&store, "personal", "42", "selected-session");
            let original = store.values.lock().unwrap().clone();
            let r = request(Some("personal"));
            let mut bodies = flow("42", favorite);
            bodies[boundary] = json_response(&json!({"errcode":603,"message":"selected-session"}));
            bodies.truncate(boundary + 1);
            let mut f = setup(replies(bodies), store).await;
            let e = f
                .provider
                .reorder_playlist_tracks(id, &r)
                .await
                .unwrap_err();
            assert_eq!(e.code, ErrorCode::UpstreamError);
            assert_eq!(
                e.details.get("write_outcome").is_some(),
                boundary >= write_at
            );
            assert_eq!(*f.store.values.lock().unwrap(), original);
            assert!(!format!("{e:?}").contains("selected-session"));
            fixture::requests(&mut f.network, boundary + 1).await;
        }
    }
}

#[tokio::test]
async fn native_track_sort_cancel_timeout_and_caller_discard_stop_at_every_boundary() {
    for favorite in [false, true] {
        let (id, boundaries, write_at) = case(favorite);
        for boundary in 0..boundaries {
            for mode in ["cancel", "timeout", "caller-discard"] {
                let store = Arc::new(Store::default());
                let selected = seed(&store, "personal", "42", "selected-session");
                let original = store.values.lock().unwrap().clone();
                let gate = Arc::new(Notify::new());
                let caller = mode == "caller-discard";
                let r = request((!caller).then_some("personal"));
                let mut f = setup(
                    flow("42", favorite)
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
                let task =
                    tokio::spawn(async move { worker.reorder_playlist_tracks(id, &r).await });
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
                    assert_eq!(
                        e.details.get("write_outcome").is_some(),
                        boundary >= write_at
                    );
                    if boundary >= write_at {
                        assert!(!e.retryable);
                    }
                }
                assert_eq!(*f.store.values.lock().unwrap(), original);
                assert!(f.network.seen.try_recv().is_err());
            }
        }
    }
}

#[tokio::test]
async fn native_track_sort_reverse_accounts_keep_metadata_and_mutations_separate() {
    for favorite in [false, true] {
        let (id, boundaries, _) = case(favorite);
        let store = Arc::new(Store::default());
        seed(&store, "A", "42", "session-A");
        seed(&store, "B", "43", "session-B");
        let gate = Arc::new(Notify::new());
        let a_request = request(Some("A"));
        let b_request = request(Some("B"));
        let mut a = setup(
            flow("42", favorite)
                .into_iter()
                .enumerate()
                .map(|(i, b)| (b, (i == boundaries - 1).then(|| gate.clone())))
                .collect(),
            store.clone(),
        )
        .await;
        let mut b = setup(replies(flow("43", favorite)), store).await;
        let p = a.provider.clone();
        let task = tokio::spawn(async move { p.reorder_playlist_tracks(id, &a_request).await });
        for _ in 0..boundaries {
            received(&mut a).await;
        }
        let result_b = b
            .provider
            .reorder_playlist_tracks(id, &b_request)
            .await
            .unwrap();
        gate.notify_one();
        let result_a = task.await.unwrap().unwrap();
        assert_eq!(result_a.extensions["library_owner_id"], "42");
        assert_eq!(result_b.extensions["library_owner_id"], "43");
        assert_eq!(result_a.extensions["cloud_track_count"], json!(4));
        assert_eq!(
            result_b.extensions["cloud_track_count"],
            result_a.extensions["cloud_track_count"]
        );
        for r in fixture::requests(&mut b.network, boundaries).await {
            assert!(!r.contains("session-A"));
            assert!(r.contains("session-B"));
        }
    }
}
