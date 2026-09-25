use super::super::tests::{Store, received, replies, seed, setup, stored};
use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{
        playlist::collected_tests::{content, directory, flow as collected_flow, page},
        tests as fixture,
    },
};
use std::sync::atomic::Ordering;
use tokio::sync::Notify;

fn flow() -> Vec<Vec<u8>> {
    collected_flow(true)
}

#[tokio::test]
async fn native_collected_playlist_absent_or_changed_created_source_never_falls_back_to_public() {
    let store = Arc::new(Store::default());
    seed(&store, "personal", "42", "selected-session");
    let mut f = setup(
        replies(vec![
            json_response(&json!({"result":"ok"})),
            json_response(&json!({"errcode":0,"plist":[]})),
            json_response(&json!({"result":"ok","data":[]})),
        ]),
        store,
    )
    .await;
    assert_eq!(
        f.provider
            .playlist("101", Some("personal"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    fixture::requests(&mut f.network, 3).await;
    // A created playlist is selected first. A later disappearance must not switch
    // to a collected or public interpretation of the same ID.
    let store = Arc::new(Store::default());
    seed(&store, "personal", "42", "selected-session");
    let mut bodies = crate::client::native::playlist::tests::flow();
    bodies[8] = json_response(&json!({"errcode":0,"plist":[]}));
    let mut f = setup(replies(bodies), store).await;
    assert_eq!(
        f.provider
            .playlist("101", Some("personal"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    fixture::requests(&mut f.network, 9).await;
}

fn request(caller: bool) -> PageRequest {
    PageRequest {
        limit: 2,
        offset: 99,
        account: (!caller).then(|| "personal".into()),
    }
}

#[tokio::test]
async fn native_collected_playlist_provider_supports_exact_server_and_caller_sources() {
    for caller in [false, true] {
        for tracks in [false, true] {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session");
            seed(&store, "default", "43", "other-session");
            let original = store.values.lock().unwrap().clone();
            store.forbid_reads.store(caller, Ordering::SeqCst);
            let mut f = setup(replies(flow()), store).await;
            let p = if caller {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            if tracks {
                let result = p.playlist_tracks("101", &request(caller)).await.unwrap();
                assert_eq!(
                    result
                        .items
                        .iter()
                        .map(|t| t.id.as_str())
                        .collect::<Vec<_>>(),
                    ["100", "100"]
                );
                assert_eq!(result.pagination.extensions["library_owner_id"], "42");
            } else {
                let result = p
                    .playlist("101", request(caller).account.as_deref())
                    .await
                    .unwrap();
                assert_eq!(result.track_count, Some(101));
                assert_eq!(result.extensions["library_owner_id"], "42");
            }
            assert_eq!(*f.store.values.lock().unwrap(), original);
            assert!(p.take_response_credential().unwrap().is_none());
            if caller {
                assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
            }
            for r in fixture::requests(&mut f.network, 8).await {
                assert!(!r.contains("other-session"));
            }
        }
    }
}

#[tokio::test]
async fn native_collected_playlist_rejects_alias_id_and_page_errors_before_network() {
    let store = Arc::new(Store::default());
    let selected = seed(&store, "personal", "42", "selected-session");
    let mut f = setup(vec![], store).await;
    assert_eq!(
        f.provider
            .playlist("101", Some("missing"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    let p = f
        .provider
        .caller_scope(&selected.caller().unwrap())
        .unwrap();
    assert_eq!(
        p.playlist("101", Some("personal")).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    for (limit, offset) in [(0, 0), (101, 0), (1, u32::MAX)] {
        let mut r = request(false);
        r.limit = limit;
        r.offset = offset;
        assert_eq!(
            f.provider
                .playlist_tracks("101", &r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for id in ["0", "0101", "101&sid=x"] {
        assert_eq!(
            f.provider
                .playlist(id, Some("personal"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    fixture::requests(&mut f.network, 0).await;
}

#[tokio::test]
async fn native_collected_playlist_each_late_success_or_error_cannot_survive_logout_or_relogin() {
    for boundary in 0..8 {
        for logout in [false, true] {
            for fail in [false, true] {
                let store = Arc::new(Store::default());
                seed(&store, "personal", "42", "selected-session");
                let other = seed(&store, "other", "7", "other-session")
                    .stored("other")
                    .unwrap();
                let gate = Arc::new(Notify::new());
                let mut f = setup(
                    flow()
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
                    tokio::spawn(async move { p.playlist_tracks("101", &request(false)).await });
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
                assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
                assert_eq!(stored(&f.store, "personal"), expected);
                assert_eq!(stored(&f.store, "other"), Some(other));
                assert!(f.network.seen.try_recv().is_err());
            }
        }
    }
}

#[tokio::test]
async fn native_collected_playlist_auth_failure_clears_only_the_original_selected_source() {
    for caller in [false, true] {
        for boundary in 0..8 {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session");
            seed(&store, "other", "7", "other-session");
            let original = store.values.lock().unwrap().clone();
            let mut bodies = flow();
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
            assert_eq!(
                p.playlist_tracks("101", &request(caller))
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
async fn native_collected_playlist_data_errors_and_removed_playlists_preserve_the_session() {
    let mut renamed = directory(Some(101));
    renamed["data"][0]["name"] = json!("Changed");
    for (at, bad) in [
        (2, json_response(&json!({"result":"ok","data":[]}))),
        (4, response(403, "text/plain", "", b"private")),
        (4, content(&json!({"code":1}))),
        (4, content(&page(&[100], 1, 102))),
        (7, json_response(&renamed)),
    ] {
        let store = Arc::new(Store::default());
        let original = seed(&store, "personal", "42", "selected-session")
            .stored("personal")
            .unwrap();
        let mut bodies = flow();
        bodies[at] = bad;
        bodies.truncate(at + 1);
        let mut f = setup(replies(bodies), store).await;
        assert_ne!(
            f.provider
                .playlist_tracks("101", &request(false))
                .await
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
        assert_eq!(stored(&f.store, "personal"), Some(original));
        assert!(f.provider.take_response_credential().unwrap().is_none());
        fixture::requests(&mut f.network, at + 1).await;
    }
}

#[tokio::test]
async fn native_collected_playlist_cancel_timeout_and_caller_discard_guard_every_boundary() {
    for boundary in 0..8 {
        for mode in ["cancel", "timeout", "caller-discard"] {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session");
            let original = store.values.lock().unwrap().clone();
            let gate = Arc::new(Notify::new());
            let mut f = setup(
                flow()
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
            let task =
                tokio::spawn(async move { worker.playlist_tracks("101", &request(caller)).await });
            for _ in 0..=boundary {
                received(&mut f).await;
            }
            match mode {
                "cancel" => {
                    task.abort();
                    assert!(task.await.unwrap_err().is_cancelled());
                }
                "timeout" => assert_eq!(
                    task.await.unwrap().unwrap_err().code,
                    ErrorCode::UpstreamTimeout
                ),
                _ => {
                    *p.caller_credential.as_ref().unwrap().lock().unwrap() = None;
                    gate.notify_one();
                    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
                }
            }
            assert_eq!(*f.store.values.lock().unwrap(), original);
            assert!(f.network.seen.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn native_collected_playlist_parallel_accounts_have_distinct_content_identity() {
    let store = Arc::new(Store::default());
    seed(&store, "A", "42", "session-A");
    seed(&store, "B", "43", "session-B");
    let gate = Arc::new(Notify::new());
    let mut a = setup(
        flow()
            .into_iter()
            .enumerate()
            .map(|(i, b)| (b, (i == 7).then(|| gate.clone())))
            .collect(),
        store.clone(),
    )
    .await;
    let mut b = setup(replies(flow()), store).await;
    let p = a.provider.clone();
    let task = tokio::spawn(async move { p.playlist("101", Some("A")).await });
    for _ in 0..8 {
        received(&mut a).await;
    }
    let result_b = b.provider.playlist("101", Some("B")).await.unwrap();
    gate.notify_one();
    let result_a = task.await.unwrap().unwrap();
    assert_eq!(result_a.extensions["library_owner_id"], "42");
    assert_eq!(result_b.extensions["library_owner_id"], "43");
    assert_ne!(
        result_a.extensions["source_snapshot_id"],
        result_b.extensions["source_snapshot_id"]
    );
    fixture::requests(&mut b.network, 8).await;
}
