use super::super::tests::{Store, received, replies, seed, setup, stored};
use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{
        playlist::{
            favorite_tests::{directory, flow},
            tests::page,
        },
        tests as fixture,
    },
};
use std::sync::atomic::Ordering;
use tokio::sync::Notify;

fn request(caller: bool) -> PageRequest {
    PageRequest {
        limit: 2,
        offset: 1,
        account: (!caller).then(|| "personal".into()),
    }
}

#[tokio::test]
async fn native_favorites_default_and_uni_sources_keep_user_and_cloud_ids_distinct() {
    for caller in [false, true] {
        for mode in [
            "default",
            "own-user",
            "uni-metadata",
            "uni-items",
            "cloud-ref",
        ] {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "default", "42", "selected-session");
            seed(&store, "other", "7", "other-session");
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
            if mode == "uni-items" {
                let page = p
                    .playlist_source_items("42", "favorite_tracks", &PageRequest::new(2, 1))
                    .await
                    .unwrap();
                assert_eq!(page.items.len(), 2);
                assert_eq!(page.items[0], page.items[1]);
                assert_eq!(page.pagination.extensions["library_section"], "favorite");
            } else {
                let metadata = match mode {
                    "default" => p.favorite_playlist(None).await,
                    "own-user" => p.user_favorite_playlist("42", None).await,
                    "uni-metadata" => p.playlist_source("42", "favorite_tracks", None).await,
                    _ => p.playlist("901", Some("default")).await,
                }
                .unwrap();
                assert_eq!(metadata.resource_ref.to_string(), "kuwo:901");
                assert_eq!(metadata.extensions["is_favorite"], true);
                assert_eq!(metadata.extensions["owner_id"], "42");
            }
            assert_eq!(*f.store.values.lock().unwrap(), original);
            assert!(p.take_response_credential().unwrap().is_none());
            fixture::requests(&mut f.network, 7).await;
        }
    }
}

#[tokio::test]
async fn native_favorites_provider_supports_exact_server_and_caller_sources() {
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
                let result = p.favorite_tracks(&request(caller)).await.unwrap();
                assert_eq!(
                    result
                        .items
                        .iter()
                        .map(|t| t.id.as_str())
                        .collect::<Vec<_>>(),
                    ["22", "22"]
                );
                assert_eq!(result.pagination.extensions["library_owner_id"], "42");
            } else {
                let result = p
                    .favorite_playlist(request(caller).account.as_deref())
                    .await
                    .unwrap();
                assert_eq!(result.track_count, Some(4));
                assert_eq!(result.extensions["library_owner_id"], "42");
            }
            assert_eq!(*f.store.values.lock().unwrap(), original);
            assert!(p.take_response_credential().unwrap().is_none());
            if caller {
                assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
            }
            for r in fixture::requests(&mut f.network, 7).await {
                assert!(!r.contains("other-session"));
            }
        }
    }
}

#[tokio::test]
async fn native_favorites_rejects_alias_uid_and_page_errors_before_network() {
    let store = Arc::new(Store::default());
    let selected = seed(&store, "personal", "42", "selected-session");
    let mut f = setup(vec![], store).await;
    assert_eq!(
        f.provider
            .favorite_playlist(Some("missing"))
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
        p.favorite_playlist(Some("personal"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    for uid in ["0", "042", "7", "42&sid=x"] {
        assert_eq!(
            f.provider
                .user_favorite_playlist(uid, Some("personal"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        assert_eq!(
            p.user_favorite_tracks(uid, &request(true))
                .await
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
    }
    for (limit, offset) in [(0, 0), (101, 0), (1, u32::MAX)] {
        let mut r = request(false);
        r.limit = limit;
        r.offset = offset;
        assert_eq!(
            f.provider.favorite_tracks(&r).await.unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    for kind in ["album", "favorite_playlists", "", "private-unknown"] {
        assert_eq!(
            f.provider
                .playlist_source("42", kind, Some("personal"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::CapabilityNotSupported
        );
    }
    fixture::requests(&mut f.network, 0).await;
}

#[tokio::test]
async fn native_favorites_each_late_success_or_error_cannot_survive_logout_or_relogin() {
    for boundary in 0..7 {
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
                let task = tokio::spawn(async move { p.favorite_tracks(&request(false)).await });
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
async fn native_favorites_auth_failure_clears_only_the_original_selected_source() {
    for caller in [false, true] {
        for boundary in 0..7 {
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
                p.favorite_tracks(&request(caller)).await.unwrap_err().code,
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
async fn native_favorites_data_errors_and_removed_lists_preserve_the_session() {
    let mut changed = directory(Some(4));
    changed["plist"][0]["info"] = json!("Changed");
    for (at, bad) in [
        (1, json_response(&json!({"errcode":0,"plist":[]}))),
        (3, response(403, "application/json", "", b"private")),
        (3, json_response(&json!({"errcode":603}))),
        (3, json_response(&page(&[22, 33], 3))),
        (6, json_response(&changed)),
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
                .favorite_tracks(&request(false))
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
async fn native_favorites_cancel_timeout_and_caller_discard_guard_every_boundary() {
    for boundary in 0..7 {
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
            let task = tokio::spawn(async move { worker.favorite_tracks(&request(caller)).await });
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
async fn native_favorites_parallel_accounts_have_distinct_content_identity() {
    let store = Arc::new(Store::default());
    seed(&store, "A", "42", "session-A");
    seed(&store, "B", "43", "session-B");
    let gate = Arc::new(Notify::new());
    let mut a = setup(
        flow()
            .into_iter()
            .enumerate()
            .map(|(i, b)| (b, (i == 6).then(|| gate.clone())))
            .collect(),
        store.clone(),
    )
    .await;
    let mut b = setup(replies(flow()), store).await;
    let p = a.provider.clone();
    let task = tokio::spawn(async move { p.favorite_playlist(Some("A")).await });
    for _ in 0..7 {
        received(&mut a).await;
    }
    let result_b = b.provider.favorite_playlist(Some("B")).await.unwrap();
    gate.notify_one();
    let result_a = task.await.unwrap().unwrap();
    assert_eq!(result_a.extensions["library_owner_id"], "42");
    assert_eq!(result_b.extensions["library_owner_id"], "43");
    assert_ne!(
        result_a.extensions["source_snapshot_id"],
        result_b.extensions["source_snapshot_id"]
    );
    fixture::requests(&mut b.network, 7).await;
}
