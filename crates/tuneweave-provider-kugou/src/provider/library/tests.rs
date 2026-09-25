use super::*;
use crate::provider::session::tests::{
    Store, credential, exchange, paused, profile, raw, read, reply, server,
};
use serde_json::Value;

pub(super) fn row(id: u64, kind: u8) -> Value {
    json!({"listid":id,"type":kind,"name":format!("Playlist {id}"),"list_ver":3,
        "count":2,"m_count":3,"is_def":if id==1 {2} else {0}})
}
pub(super) fn page(items: Vec<Value>) -> String {
    reply(json!({"userid":111,"total_ver":9,"list_count":166,"collect_count":47,"info":items}))
}
pub(super) fn request(account: &str, limit: u32, offset: u32) -> PageRequest {
    PageRequest {
        account: Some(account.into()),
        limit,
        offset,
    }
}
pub(in crate::provider) fn store_account(provider: &mut KugouProvider) -> Arc<Store> {
    let store = Arc::new(Store::default());
    store
        .put(&credential("111", "original").stored("A").unwrap())
        .unwrap();
    store
        .put(&credential("222", "other").stored("B").unwrap())
        .unwrap();
    provider.credential_store = Some(store.clone());
    store
}

#[tokio::test]
async fn account_library_reads_all_physical_pages_then_slices_visible_rows_with_exact_alias() {
    let mut first = (1..=30).map(|id| row(id, 0)).collect::<Vec<_>>();
    first[1] = json!({"listid":2,"type":0,"is_del":1});
    let mut f = server(vec![
        exchange("111", "rotated").into(),
        profile("111").into(),
        page(first).into(),
        page(vec![row(31, 1), row(32, 1)]).into(),
    ])
    .await;
    let store = store_account(&mut f.provider);
    let other = read(&store, "B");
    let p = f
        .provider
        .account_playlists(&request("A", 4, 28))
        .await
        .unwrap();
    assert_eq!(p.pagination.total, Some(31));
    assert!(!p.pagination.has_more);
    assert_eq!(
        p.items.iter().map(|p| p.id.as_str()).collect::<Vec<_>>(),
        [
            "cloudlist:111:0:30",
            "cloudlist:111:1:31",
            "cloudlist:111:1:32"
        ]
    );
    assert_eq!(p.pagination.extensions["upstream_pages_fetched"], 2);
    assert_eq!(p.pagination.extensions["list_count"], 166);
    assert_eq!(p.pagination.extensions["collect_count"], 47);
    assert_eq!(p.pagination.extensions["deleted_rows"], 1);
    assert_eq!(read(&store, "B"), other);
    assert_eq!(read(&store, "A").native().session.token, "rotated");
    assert!(f.provider.take_response_credential().unwrap().is_none());
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 4);
    for (index, r) in requests[2..].iter().enumerate() {
        let body: Value = serde_json::from_str(r.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["page"], index + 1);
        assert_eq!(body["total_ver"], 0);
        assert_eq!(body["userid"], 111);
        assert_eq!(body["token"], "rotated");
    }
}

#[tokio::test]
async fn own_created_collected_metadata_and_out_of_range_reads_keep_namespaces_and_counts() {
    for section in [None, Some(0), Some(1)] {
        let mut f = server(vec![
            exchange("111", "next").into(),
            profile("111").into(),
            page(vec![row(1, 0), row(1, 1), row(3, 0)]).into(),
        ])
        .await;
        store_account(&mut f.provider);
        let r = request("A", 1, 0);
        let p = match section {
            None => f.provider.account_playlists(&r).await,
            Some(0) => f.provider.user_created_playlists("111", &r).await,
            _ => f.provider.user_favorite_playlists("111", &r).await,
        }
        .unwrap();
        assert_eq!(
            p.pagination.total,
            Some(match section {
                None => 3,
                Some(0) => 2,
                _ => 1,
            })
        );
        assert_eq!(
            p.items[0].id,
            if section == Some(1) {
                "cloudlist:111:1:1"
            } else {
                "cloudlist:111:0:1"
            }
        );
        assert_eq!(
            p.pagination.next_offset,
            if section == Some(1) { None } else { Some(1) }
        );
        f.requests.await.unwrap();
    }
    for (items, offset, total) in [(vec![], 0, 0), (vec![row(1, 0)], 99, 1)] {
        let mut f = server(vec![
            exchange("111", "next").into(),
            profile("111").into(),
            page(items).into(),
        ])
        .await;
        store_account(&mut f.provider);
        let p = f
            .provider
            .account_playlists(&request("A", 10, offset))
            .await
            .unwrap();
        assert!(p.items.is_empty());
        assert_eq!(p.pagination.total, Some(total));
        assert!(!p.pagination.has_more);
        f.requests.await.unwrap();
    }
    for exists in [false, true] {
        let mut frames = vec![
            exchange("111", "next").into(),
            profile("111").into(),
            page(vec![row(1, 0)]).into(),
        ];
        if exists {
            frames.push(reply(json!({"list_ver":3,"count":0,"info":[]})).into());
            frames.push(page(vec![row(1, 0)]).into());
        }
        let mut f = server(frames).await;
        store_account(&mut f.provider);
        let result = f
            .provider
            .playlist(
                if exists {
                    "cloudlist:111:0:1"
                } else {
                    "cloudlist:111:1:1"
                },
                Some("A"),
            )
            .await;
        if exists {
            assert_eq!(
                result.unwrap().extensions["system_playlist"],
                "liked_tracks"
            );
        } else {
            assert_eq!(result.unwrap_err().code, ErrorCode::ResourceNotFound);
        }
        f.requests.await.unwrap();
    }
}

#[tokio::test]
async fn full_sized_terminal_page_requires_empty_confirmation_and_version_drift_is_rejected() {
    let first = (1..=30).map(|id| row(id, 0)).collect::<Vec<_>>();
    for failure in [None, Some("total_ver"), Some("list_count"), Some("repeat")] {
        let mut next =
            json!({"userid":111,"total_ver":9,"list_count":166,"collect_count":47,"info":[]});
        if failure == Some("repeat") {
            next["info"] = json!([row(1, 0)]);
        } else if let Some(field) = failure {
            next[field] = json!(1000);
        }
        let mut f = server(vec![
            exchange("111", "next").into(),
            profile("111").into(),
            page(first.clone()).into(),
            reply(next).into(),
        ])
        .await;
        store_account(&mut f.provider);
        let result = f.provider.account_playlists(&request("A", 1, 0)).await;
        if failure.is_none() {
            let p = result.unwrap();
            assert_eq!(p.pagination.total, Some(30));
            assert_eq!(p.pagination.extensions["upstream_pages_fetched"], 2);
        } else {
            assert_eq!(result.unwrap_err().code, ErrorCode::Conflict);
        }
        assert_eq!(f.requests.await.unwrap().len(), 4);
    }
}

#[tokio::test]
async fn library_caller_read_returns_rotation_and_never_reads_or_updates_stored_accounts() {
    let mut f = server(vec![
        exchange("111", "caller-next").into(),
        profile("111").into(),
        page(vec![row(1, 0)]).into(),
    ])
    .await;
    let store = store_account(&mut f.provider);
    let before = store.values.lock().unwrap().clone();
    let source = credential("111", "caller-old");
    let scoped = f.provider.caller_scope(&source.caller().unwrap()).unwrap();
    let p = scoped
        .account_playlists(&PageRequest::new(10, 0))
        .await
        .unwrap();
    assert_eq!(p.items[0].id, "cloudlist:111:0:1");
    let updated =
        KugouCredential::parse_caller(&scoped.take_response_credential().unwrap().unwrap())
            .unwrap();
    assert!(source.same_login(&updated));
    assert_eq!(updated.native().session.token, "caller-next");
    assert_eq!(*store.values.lock().unwrap(), before);
    f.requests.await.unwrap();
}

#[tokio::test]
async fn library_errors_preserve_accepted_rotation_except_on_expiry_or_identity_conflict() {
    for caller in [false, true] {
        for (response, expected) in [
            (
                raw(json!({"status":0,"error_code":20010,"data":"do-not-export"})),
                ErrorCode::UpstreamError,
            ),
            (
                raw(json!({"status":0,"error_code":20017,"data":null})),
                ErrorCode::AuthenticationRequired,
            ),
            (
                reply(json!({"userid":222,"total_ver":9,"info":[]})),
                ErrorCode::Conflict,
            ),
        ] {
            let mut f = server(vec![
                exchange("111", "next").into(),
                profile("111").into(),
                response.into(),
            ])
            .await;
            let store = store_account(&mut f.provider);
            let old = read(&store, "A");
            let provider = if caller {
                f.provider.caller_scope(&old.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let mut e = provider
                .account_playlists(&request(if caller { "default" } else { "A" }, 10, 0))
                .await
                .unwrap_err();
            assert_eq!(e.code, expected);
            assert!(!format!("{e:?}").contains("do-not-export"));
            let exports = caller && expected == ErrorCode::UpstreamError;
            assert_eq!(e.take_caller_credential_update().is_some(), exports);
            assert_eq!(
                provider.take_response_credential().unwrap().is_some(),
                exports
            );
            if caller {
                assert_eq!(read(&store, "A"), old);
            } else if expected == ErrorCode::AuthenticationRequired {
                assert!(!store.values.lock().unwrap().contains_key("A"));
            } else {
                assert_eq!(read(&store, "A").native().session.token, "next");
            }
            f.requests.await.unwrap();
        }
    }
}

#[tokio::test]
async fn logout_relogin_and_concurrent_rotation_defeat_late_library_success_and_auth_failure() {
    for action in ["logout", "relogin", "rotate"] {
        for success in [false, true] {
            let last = if success {
                page(vec![row(1, 0)])
            } else {
                raw(json!({"status":0,"error_code":20017,"data":null}))
            };
            let (last, resume) = paused(last);
            let mut f = server(vec![
                exchange("111", "next").into(),
                profile("111").into(),
                last,
            ])
            .await;
            let store = store_account(&mut f.provider);
            let provider = f.provider.clone();
            let task =
                tokio::spawn(async move { provider.account_playlists(&request("A", 10, 0)).await });
            for _ in 0..3 {
                f.seen.recv().await.unwrap();
            }
            let replacement = if action == "rotate" {
                let old = read(&store, "A");
                let mut session = old.native().session.clone();
                session.token = "concurrent-next".into();
                KugouCredential::Native(old.native().rotate(session).unwrap())
            } else {
                credential("222", "relogin")
            };
            if action == "logout" {
                f.provider.logout("A").await.unwrap();
            } else {
                store.put(&replacement.stored("A").unwrap()).unwrap();
            }
            resume.send(()).unwrap();
            let mut error = task.await.unwrap().unwrap_err();
            assert_eq!(error.code, ErrorCode::Conflict);
            assert!(error.take_caller_credential_update().is_none());
            if action != "logout" {
                assert_eq!(read(&store, "A"), replacement);
            } else {
                assert!(!store.values.lock().unwrap().contains_key("A"));
            }
            f.requests.await.unwrap();
        }
    }
}

#[tokio::test]
async fn native_library_rejects_web_other_user_bad_references_and_invalid_pages_before_network() {
    let mut f = server(vec![]).await;
    let store = store_account(&mut f.provider);
    assert_eq!(
        f.provider
            .user_created_playlists("222", &request("A", 10, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        f.provider
            .account_playlists(&request("missing", 10, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    let scoped = f
        .provider
        .caller_scope(&read(&store, "A").caller().unwrap())
        .unwrap();
    assert_eq!(
        scoped
            .account_playlists(&request("A", 10, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    for (limit, offset) in [(0, 0), (101, 0), (100, u32::MAX)] {
        assert_eq!(
            f.provider
                .account_playlists(&request("A", limit, offset))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for reference in [
        "cloudlist:111:0:01",
        "cloudlist:0111:0:1",
        "cloudlist:111:2:1",
        "cloudlist:111:0:1:extra",
        "cloudlist:111:0:0",
    ] {
        assert!(f.provider.playlist(reference, Some("A")).await.is_err());
    }
    assert_eq!(
        f.provider
            .playlist("cloudlist:222:0:1", Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        f.provider
            .playlist_tracks("cloudlist:222:0:1", &request("A", 10, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    let web =
        KugouCredential::verified_web(crate::web::WebSession::test_session("111", "web-cookie"))
            .unwrap();
    store.put(&web.stored("W").unwrap()).unwrap();
    assert_eq!(
        f.provider
            .user_created_playlists("111", &request("W", 10, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert_eq!(read(&store, "W"), web);
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn library_page_budget_rejects_truncated_full_read_even_for_a_one_item_window() {
    let mut frames = vec![exchange("111", "next").into(), profile("111").into()];
    for page_index in 0..MAX_PAGES {
        frames.push(
            page(
                (1..=PAGE_SIZE)
                    .map(|n| row(u64::from(page_index) * PAGE_SIZE as u64 + n as u64, 0))
                    .collect(),
            )
            .into(),
        );
    }
    let mut f = server(frames).await;
    store_account(&mut f.provider);
    assert_eq!(
        f.provider
            .account_playlists(&request("A", 1, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(f.requests.await.unwrap().len(), MAX_PAGES as usize + 2);
}

#[tokio::test]
async fn caller_replacement_during_library_read_clears_late_rotation_without_touching_server() {
    for invalidated in [false, true] {
        let (last, resume) = paused(page(vec![row(1, 0)]));
        let mut f = server(vec![
            exchange("111", "rotated").into(),
            profile("111").into(),
            last,
        ])
        .await;
        let store = store_account(&mut f.provider);
        let saved = read(&store, "A");
        let caller = f.provider.caller_scope(&saved.caller().unwrap()).unwrap();
        let scoped = caller.clone();
        let task =
            tokio::spawn(async move { scoped.account_playlists(&PageRequest::new(10, 0)).await });
        for _ in 0..3 {
            f.seen.recv().await.unwrap();
        }
        let replacement = (!invalidated).then(|| credential("222", "replacement"));
        *caller.caller_credential.as_ref().unwrap().lock().unwrap() = replacement.clone();
        resume.send(()).unwrap();
        let mut failure = task.await.unwrap().unwrap_err();
        assert_eq!(failure.code, ErrorCode::Conflict);
        assert!(failure.take_caller_credential_update().is_none());
        assert!(caller.take_response_credential().unwrap().is_none());
        assert_eq!(
            *caller.caller_credential.as_ref().unwrap().lock().unwrap(),
            replacement
        );
        assert_eq!(read(&store, "A"), saved);
        f.requests.await.unwrap();
    }
}

#[tokio::test]
async fn native_library_stops_before_business_reads_when_exchange_persistence_or_profile_fails() {
    use std::sync::atomic::Ordering;
    for failure in ["exchange", "store", "profile"] {
        let frames = match failure {
            "exchange" => vec![raw(json!({"status":0,"error_code":20017,"data":null})).into()],
            "store" => vec![exchange("111", "next").into()],
            _ => vec![exchange("111", "next").into(), profile("222").into()],
        };
        let mut f = server(frames).await;
        let store = store_account(&mut f.provider);
        let original = read(&store, "A");
        if failure == "store" {
            store.fail.store(true, Ordering::SeqCst);
        }
        let mut error = f
            .provider
            .account_playlists(&request("A", 10, 0))
            .await
            .unwrap_err();
        assert_eq!(
            error.code,
            match failure {
                "exchange" => ErrorCode::AuthenticationRequired,
                "store" => ErrorCode::InternalError,
                _ => ErrorCode::Conflict,
            }
        );
        assert!(error.take_caller_credential_update().is_none());
        if failure == "store" {
            assert_eq!(read(&store, "A"), original);
        }
        let calls = f.requests.await.unwrap();
        assert_eq!(calls.len(), if failure == "profile" { 2 } else { 1 });
        assert!(calls.iter().all(|r| !r.contains("get_all_list")));
    }
}
