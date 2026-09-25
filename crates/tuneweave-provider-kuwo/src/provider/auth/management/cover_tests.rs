use super::super::tests::{Store, replies, seed, stored};
use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{
        management::cover::tests::{flow, request},
        tests as fixture,
    },
};
use std::sync::atomic::Ordering;
use tokio::sync::Notify;

#[tokio::test]
async fn native_cover_default_named_and_caller_account_sources_remain_separate() {
    let boundaries = 17;
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
        let mut f = setup(replies(flow("42")), store).await;
        let p = if mode == "caller" {
            f.provider
                .caller_scope(&selected.caller().unwrap())
                .unwrap()
        } else {
            f.provider.clone()
        };
        let result = p.update_playlist_cover("101", &r).await.unwrap();
        assert_eq!(
            result.image.url.as_deref(),
            Some(crate::client::native::management::cover::tests::URL)
        );
        assert_eq!(result.extensions["write_requests_dispatched"], 2);
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

#[tokio::test]
async fn native_cover_every_late_success_and_error_observes_original_generation() {
    let boundaries = 17;
    let write_at = 7;
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
                    flow("42")
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
                let task = tokio::spawn(async move { p.update_playlist_cover("101", &r).await });
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
                assert_progress(&error, boundary);
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

#[tokio::test]
async fn native_cover_auth_expiry_clears_only_the_unchanged_selected_source() {
    let boundaries = 17;
    let write_at = 7;
    for caller in [false, true] {
        for boundary in 0..boundaries {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session");
            seed(&store, "other", "7", "other-session");
            let original = store.values.lock().unwrap().clone();
            let r = request((!caller).then_some("personal"));
            let mut bodies = flow("42");
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
            let error = p.update_playlist_cover("101", &r).await.unwrap_err();
            assert_eq!(error.code, ErrorCode::AuthenticationRequired);
            assert_progress(&error, boundary);
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

#[tokio::test]
async fn native_cover_business_error_and_readback_change_keep_credentials_without_retry() {
    let boundaries = 17;
    let write_at = 7;
    for boundary in 1..boundaries {
        let store = Arc::new(Store::default());
        seed(&store, "personal", "42", "selected-session");
        let original = store.values.lock().unwrap().clone();
        let r = request(Some("personal"));
        let mut bodies = flow("42");
        bodies[boundary] = json_response(&json!({"errcode":603,"message":"selected-session"}));
        bodies.truncate(boundary + 1);
        let mut f = setup(replies(bodies), store).await;
        let e = f
            .provider
            .update_playlist_cover("101", &r)
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

#[tokio::test]
async fn native_cover_cancel_timeout_and_caller_discard_stop_at_every_boundary() {
    let boundaries = 17;
    let write_at = 7;
    for boundary in 0..boundaries {
        for mode in ["cancel", "timeout", "caller-discard"] {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session");
            let original = store.values.lock().unwrap().clone();
            let gate = Arc::new(Notify::new());
            let caller = mode == "caller-discard";
            let r = request((!caller).then_some("personal"));
            let mut f = setup(
                flow("42")
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
            let task = tokio::spawn(async move { worker.update_playlist_cover("101", &r).await });
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
                assert_progress(&e, boundary);
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

#[tokio::test]
async fn native_cover_reverse_accounts_keep_metadata_and_mutations_separate() {
    let boundaries = 17;
    let store = Arc::new(Store::default());
    seed(&store, "A", "42", "session-A");
    seed(&store, "B", "43", "session-B");
    let gate = Arc::new(Notify::new());
    let a_request = request(Some("A"));
    let b_request = request(Some("B"));
    let mut a = setup(
        flow("42")
            .into_iter()
            .enumerate()
            .map(|(i, b)| (b, (i == boundaries - 1).then(|| gate.clone())))
            .collect(),
        store.clone(),
    )
    .await;
    let mut b = setup(replies(flow("43")), store).await;
    // A deliberately waits for B's full local conversion and network flow.
    // Timeout-specific tests retain the ordinary two-second fixture budget.
    fixture::set_request_timeout(&mut a.provider.client, Duration::from_secs(30));
    let p = a.provider.clone();
    let task = tokio::spawn(async move { p.update_playlist_cover("101", &a_request).await });
    for _ in 0..boundaries {
        received(&mut a).await;
    }
    let result_b = b
        .provider
        .update_playlist_cover("101", &b_request)
        .await
        .unwrap();
    gate.notify_one();
    let result_a = task.await.unwrap().unwrap();
    assert_eq!(result_a.extensions["library_owner_id"], "42");
    assert_eq!(result_b.extensions["library_owner_id"], "43");
    assert_eq!(result_b.playlist_ref, result_a.playlist_ref);
    for r in fixture::requests(&mut b.network, boundaries).await {
        assert!(!r.contains("session-A"));
        assert!(r.contains("session-B"));
    }
}

fn assert_progress(error: &TuneWeaveError, boundary: usize) {
    if boundary < 7 {
        assert!(error.details.get("write_outcome").is_none());
    } else {
        assert_eq!(
            error.details["write_requests_dispatched"],
            if boundary < 10 { 1 } else { 2 }
        );
        assert_eq!(error.details["upload_requests_dispatched"], 1);
        assert_eq!(
            error.details["playlist_write_requests_dispatched"],
            u8::from(boundary >= 10)
        );
        assert_eq!(
            error.details["upload_outcome"],
            if boundary > 7 {
                "confirmed"
            } else {
                "unconfirmed"
            }
        );
        assert_eq!(
            error.details["playlist_write_outcome"],
            if boundary >= 10 {
                "unconfirmed"
            } else {
                "not_dispatched"
            }
        );
    }
}

async fn setup(
    bodies: Vec<(Vec<u8>, Option<Arc<Notify>>)>,
    store: Arc<Store>,
) -> super::super::tests::Fixture {
    let network = fixture::setup_gated_with_preparation(bodies, Duration::from_secs(30)).await;
    let mut provider = KuwoProvider::from_client(network.client.clone());
    provider.credential_store = Some(store.clone());
    super::super::tests::Fixture {
        network,
        provider,
        store,
    }
}
async fn received(fixture: &mut super::super::tests::Fixture) {
    tokio::time::timeout(Duration::from_secs(30), fixture.network.seen.recv())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn native_cover_queued_image_preparation_stays_bound_to_original_selection() {
    for mode in ["logout", "replace", "cancel"] {
        for bad_image in [false, true] {
            let pause = crate::client::native::management::cover::image::pause_preparation().await;
            let store = Arc::new(Store::default());
            seed(&store, "personal", "42", "selected-session");
            let mut f = setup(vec![], store).await;
            let p = f.provider.clone();
            let mut r = request(Some("personal"));
            if bad_image {
                r.data.truncate(24);
            }
            let task = tokio::spawn(async move { p.update_playlist_cover("101", &r).await });
            tokio::time::timeout(Duration::from_secs(3), async {
                while f.store.reads.load(Ordering::SeqCst) == 0 {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            if mode == "cancel" {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                drop(pause);
            } else {
                if mode == "logout" {
                    f.provider.logout("personal").await.unwrap();
                } else {
                    seed(&f.store, "personal", "43", "replacement-session");
                }
                let expected = stored(&f.store, "personal");
                drop(pause);
                let error = task.await.unwrap().unwrap_err();
                assert_eq!(error.code, ErrorCode::Conflict);
                assert!(error.details.get("write_outcome").is_none());
                assert_eq!(stored(&f.store, "personal"), expected);
            }
            fixture::requests(&mut f.network, 0).await;
        }
    }
}
