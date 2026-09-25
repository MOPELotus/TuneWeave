use super::super::tests::{Store, received, replies, seed, setup, stored, valid};
use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{
        library::tests::{created, flow, saved},
        tests as fixture,
    },
};
use std::sync::atomic::Ordering;
use tokio::sync::Notify;

fn request(account: Option<&str>) -> PageRequest {
    PageRequest {
        limit: 100,
        offset: 0,
        account: account.map(str::to_owned),
    }
}

#[tokio::test]
async fn native_library_provider_scopes_preserve_credentials_and_never_read_server_for_caller() {
    for caller in [false, true] {
        for section in [None, Some(Section::Created), Some(Section::Saved)] {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session");
            seed(&store, "default", "43", "other-session");
            let original = store.values.lock().unwrap().clone();
            store.forbid_reads.store(caller, Ordering::SeqCst);
            let (bodies, requests) = match section {
                Some(Section::Favorite | Section::Owned) => {
                    unreachable!("this test covers only public directory methods")
                }
                None => (flow(), 4),
                Some(Section::Created) => (vec![valid(), json_response(&created())], 2),
                Some(Section::Saved) => (
                    vec![
                        valid(),
                        json_response(&saved(0, 20)),
                        json_response(&saved(20, 1)),
                    ],
                    3,
                ),
            };
            let mut f = setup(replies(bodies), store).await;
            let p = if caller {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let request = request((!caller).then_some("personal"));
            let page = match section {
                Some(Section::Favorite | Section::Owned) => {
                    unreachable!("this test covers only public directory methods")
                }
                None => p.account_playlists(&request).await,
                Some(Section::Created) => p.user_created_playlists("42", &request).await,
                Some(Section::Saved) => p.user_favorite_playlists("42", &request).await,
            }
            .unwrap();
            assert_eq!(
                page.pagination.total,
                Some(match section {
                    Some(Section::Favorite | Section::Owned) =>
                        unreachable!("this test covers only public directory methods"),
                    None => 23,
                    Some(Section::Created) => 2,
                    Some(Section::Saved) => 21,
                })
            );
            assert_eq!(page.pagination.extensions["library_owner_id"], "42");
            assert!(
                page.items
                    .iter()
                    .all(|p| p.extensions["library_owner_id"] == "42")
            );
            assert_eq!(*f.store.values.lock().unwrap(), original);
            assert!(p.take_response_credential().unwrap().is_none());
            if caller {
                assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
            }
            let req = fixture::requests(&mut f.network, requests).await;
            for r in req {
                assert!(!r.contains("other-session"));
            }
        }
    }
}

#[tokio::test]
async fn native_library_invalid_identity_account_and_pagination_fail_before_network() {
    let store = Arc::new(Store::default());
    let selected = seed(&store, "personal", "42", "selected-session");
    seed(&store, "invalid", "42", "session,loginUid=43");
    let f = setup(vec![], store).await;
    assert_eq!(
        f.provider
            .user_created_playlists("43", &request(Some("personal")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        f.provider
            .user_favorite_playlists("43", &request(Some("personal")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    for (account, code) in [
        ("missing", ErrorCode::AuthenticationRequired),
        ("invalid", ErrorCode::InvalidRequest),
    ] {
        assert_eq!(
            f.provider
                .account_playlists(&request(Some(account)))
                .await
                .unwrap_err()
                .code,
            code
        );
    }
    let caller = f
        .provider
        .caller_scope(&selected.caller().unwrap())
        .unwrap();
    assert_eq!(
        caller
            .account_playlists(&request(Some("personal")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    for (limit, offset) in [(0, 0), (101, 0), (1, u32::MAX)] {
        let mut request = request(Some("personal"));
        request.limit = limit;
        request.offset = offset;
        assert_eq!(
            f.provider
                .account_playlists(&request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
}

#[tokio::test]
async fn native_library_late_success_and_errors_after_logout_or_relogin_never_cross_accounts() {
    for boundary in 0..4 {
        for logout in [false, true] {
            for failure in [false, true] {
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
                                if failure && i == boundary {
                                    response(401, "application/json", "", b"private-error")
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
                    tokio::spawn(
                        async move { p.account_playlists(&request(Some("personal"))).await },
                    );
                for _ in 0..=boundary {
                    received(&mut f).await;
                }
                if logout {
                    f.provider.logout("personal").await.unwrap();
                } else {
                    seed(&f.store, "personal", "43", "replacement-session");
                }
                let after = stored(&f.store, "personal");
                gate.notify_one();
                assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
                assert_eq!(stored(&f.store, "personal"), after);
                assert_eq!(stored(&f.store, "other"), Some(other));
                assert!(f.network.seen.try_recv().is_err());
            }
        }
    }
}

#[tokio::test]
async fn native_library_auth_failures_clear_only_the_unchanged_selected_owner() {
    for caller in [false, true] {
        for boundary in 0..4 {
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session");
            seed(&store, "other", "7", "other-session");
            let original = store.values.lock().unwrap().clone();
            let mut bodies = flow();
            bodies.truncate(boundary + 1);
            bodies[boundary] = response(401, "application/json", "", b"private-error");
            let mut f = setup(replies(bodies), store).await;
            let p = if caller {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            assert_eq!(
                p.account_playlists(&request((!caller).then_some("personal")))
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
async fn native_library_business_errors_and_late_duplicates_preserve_session_without_partial_data()
{
    let mut wrong_owner = saved(20, 1);
    wrong_owner["uid"] = json!(43);
    for bad in [
        json_response(&wrong_owner),
        json_response(&saved(0, 1)),
        json_response(&json!({"result":"fail","data":[]})),
        response(403, "application/json", "", b"private-error"),
    ] {
        let store = Arc::new(Store::default());
        let original = seed(&store, "personal", "42", "selected-session")
            .stored("personal")
            .unwrap();
        let mut bodies = flow();
        bodies[3] = bad;
        let mut f = setup(replies(bodies), store).await;
        let error = f
            .provider
            .account_playlists(&request(Some("personal")))
            .await
            .unwrap_err();
        assert_ne!(error.code, ErrorCode::AuthenticationRequired);
        assert_eq!(stored(&f.store, "personal"), Some(original));
        assert!(f.provider.take_response_credential().unwrap().is_none());
        fixture::requests(&mut f.network, 4).await;
    }
}

#[tokio::test]
async fn native_library_cancel_and_timeout_at_every_boundary_leave_the_original_session() {
    for boundary in 0..4 {
        for cancel in [false, true] {
            let store = Arc::new(Store::default());
            let original = seed(&store, "personal", "42", "selected-session")
                .stored("personal")
                .unwrap();
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
            let p = f.provider.clone();
            let task =
                tokio::spawn(async move { p.account_playlists(&request(Some("personal"))).await });
            for _ in 0..=boundary {
                received(&mut f).await;
            }
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            } else {
                assert_eq!(
                    task.await.unwrap().unwrap_err().code,
                    ErrorCode::UpstreamTimeout
                );
            }
            assert_eq!(stored(&f.store, "personal"), Some(original));
            assert!(f.network.seen.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn native_library_parallel_accounts_and_caller_invalidation_keep_results_separate() {
    let store = Arc::new(Store::default());
    seed(&store, "A", "42", "session-A");
    seed(&store, "B", "43", "session-B");
    let gate = Arc::new(Notify::new());
    let mut a = setup(
        flow()
            .into_iter()
            .enumerate()
            .map(|(i, b)| (b, (i == 3).then(|| gate.clone())))
            .collect(),
        store.clone(),
    )
    .await;
    let mut b = setup(replies(flow()), store).await;
    let p = a.provider.clone();
    let task = tokio::spawn(async move { p.account_playlists(&request(Some("A"))).await });
    for _ in 0..4 {
        received(&mut a).await;
    }
    let result = b
        .provider
        .account_playlists(&request(Some("B")))
        .await
        .unwrap();
    assert_eq!(result.pagination.extensions["library_owner_id"], "43");
    gate.notify_one();
    assert_eq!(
        task.await.unwrap().unwrap().pagination.extensions["library_owner_id"],
        "42"
    );
    fixture::requests(&mut b.network, 4).await;
    for boundary in 0..4 {
        let gate = Arc::new(Notify::new());
        let store = Arc::new(Store::default());
        let selected = seed(&store, "stored", "42", "selected-session");
        let original = store.values.lock().unwrap().clone();
        let mut f = setup(
            flow()
                .into_iter()
                .enumerate()
                .map(|(i, b)| (b, (i == boundary).then(|| gate.clone())))
                .collect(),
            store,
        )
        .await;
        let p = f
            .provider
            .caller_scope(&selected.caller().unwrap())
            .unwrap();
        let worker = p.clone();
        let task = tokio::spawn(async move { worker.account_playlists(&request(None)).await });
        for _ in 0..=boundary {
            received(&mut f).await;
        }
        *p.caller_credential.as_ref().unwrap().lock().unwrap() = None;
        gate.notify_one();
        assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(*f.store.values.lock().unwrap(), original);
        assert!(f.network.seen.try_recv().is_err());
    }
}
