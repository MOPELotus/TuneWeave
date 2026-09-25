use super::super::tests::{Store, received, replies, seed, setup, stored};
use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{
        playlist::tests::{detail, directory, flow, flow_for, page},
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
async fn native_created_playlist_provider_supports_exact_server_and_caller_sources() {
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
                    ["22", "22"]
                );
                assert_eq!(result.pagination.extensions["library_owner_id"], "42");
            } else {
                let result = p
                    .playlist("101", request(caller).account.as_deref())
                    .await
                    .unwrap();
                assert_eq!(result.track_count, Some(4));
                assert_eq!(result.tags, ["流行", "安静"]);
                assert_eq!(result.extensions["editable_metadata_verified"], true);
                assert_eq!(result.extensions["library_owner_id"], "42");
            }
            assert_eq!(*f.store.values.lock().unwrap(), original);
            assert!(p.take_response_credential().unwrap().is_none());
            if caller {
                assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
            }
            for r in fixture::requests(&mut f.network, 9).await {
                assert!(!r.contains("other-session"));
            }
        }
    }
}

#[tokio::test]
async fn native_created_playlist_rejects_alias_id_and_page_errors_before_network() {
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
async fn native_created_playlist_each_late_success_or_error_cannot_survive_logout_or_relogin() {
    for boundary in 0..flow().len() {
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
async fn native_created_playlist_auth_failure_clears_only_the_original_selected_source() {
    for caller in [false, true] {
        for boundary in 0..flow().len() {
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
async fn native_created_playlist_data_errors_and_removed_playlists_preserve_the_session() {
    let mut renamed = directory(Some(4));
    renamed["plist"][0]["title"] = json!("Changed");
    for (at, bad) in [
        (1, json_response(&json!({"errcode":0,"plist":[]}))),
        (2, json_response(&detail("7", 4))),
        (7, json_response(&detail("42", 5))),
        (4, response(403, "application/json", "", b"private")),
        (4, json_response(&json!({"errcode":603}))),
        (4, json_response(&page(&[22, 33], 3))),
        (8, json_response(&renamed)),
    ] {
        let store = Arc::new(Store::default());
        let original = seed(&store, "personal", "42", "selected-session")
            .stored("personal")
            .unwrap();
        let mut bodies = flow();
        bodies[at] = bad;
        bodies.truncate(at + 1);
        if at == 1 {
            bodies.push(json_response(&json!({"result":"ok","data":[]})));
        }
        let count = bodies.len();
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
        fixture::requests(&mut f.network, count).await;
    }
}

#[tokio::test]
async fn native_created_playlist_cancel_timeout_and_caller_discard_guard_every_boundary() {
    for boundary in 0..flow().len() {
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
async fn native_created_playlist_parallel_accounts_have_distinct_content_identity() {
    let store = Arc::new(Store::default());
    seed(&store, "A", "42", "session-A");
    seed(&store, "B", "43", "session-B");
    let gate = Arc::new(Notify::new());
    let mut a = setup(
        flow()
            .into_iter()
            .enumerate()
            .map(|(i, b)| (b, (i == 8).then(|| gate.clone())))
            .collect(),
        store.clone(),
    )
    .await;
    let mut b = setup(replies(flow_for("43")), store).await;
    let p = a.provider.clone();
    let task = tokio::spawn(async move { p.playlist("101", Some("A")).await });
    for _ in 0..9 {
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
    fixture::requests(&mut b.network, 9).await;
}

#[tokio::test]
async fn native_created_playlist_uni_default_account_detects_tag_only_changes_across_calls() {
    for changed in [false, true] {
        let store = Arc::new(Store::default());
        seed(&store, "default", "42", "selected-session");
        seed(&store, "other", "7", "other-session");
        let original = store.values.lock().unwrap().clone();
        let mut second = flow();
        if changed {
            let mut m = detail("42", 4);
            m["sl_data"]["tag"] = json!("流行,新标签");
            for at in [2, 7] {
                second[at] = json_response(&m);
            }
        }
        let mut bodies = flow();
        bodies.extend(second);
        let mut f = setup(replies(bodies), store).await;
        let metadata = f
            .provider
            .playlist_source("101", "playlist", Some("default"))
            .await
            .unwrap();
        let mut r = PageRequest::new(2, 1);
        r.account = Some("default".into());
        let page = f
            .provider
            .playlist_source_items("101", "playlist", &r)
            .await
            .unwrap();
        assert_eq!(metadata.tags, ["流行", "安静"]);
        assert_eq!(
            metadata.extensions["source_snapshot_id"]
                != page.pagination.extensions["source_snapshot_id"],
            changed
        );
        assert_eq!(page.items.len(), 2);
        assert_eq!(*f.store.values.lock().unwrap(), original);
        for seen in fixture::requests(&mut f.network, 18).await {
            assert!(seen.starts_with("GET "));
            assert!(!seen.contains("other-session"));
        }
    }
}
