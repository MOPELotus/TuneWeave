use super::super::tests::{Store, replies, seed, setup, valid};
use super::*;
use crate::client::{catalog::tests::json_response, native::tests as fixture};
use std::sync::atomic::Ordering;
use tokio::sync::Notify;
use tuneweave_core::AccountCredentialStore;

fn directory() -> serde_json::Value {
    json!({"result":"ok","uid":"42","data":[
        {"id":"810","name":"谭咏麟","AARTIST":"Alan Tam","digest":"4","szb":"0"},
        {"id":"336","name":"周杰伦","AARTIST":"Jay Chou","digest":"4","szb":"0"}
    ]})
}

fn directory_ids(ids: &[&str]) -> serde_json::Value {
    let data = ids
        .iter()
        .map(|id| {
            json!({
                "id": id,
                "name": format!("Artist {id}"),
                "digest": "4",
                "szb": "0"
            })
        })
        .collect::<Vec<_>>();
    json!({"result":"ok", "uid":"42", "data":data})
}

fn request(account: Option<&str>, limit: u32, offset: u32) -> PageRequest {
    PageRequest {
        limit,
        offset,
        account: account.map(str::to_owned),
    }
}

#[tokio::test]
async fn following_artists_reads_only_the_selected_native_account_for_server_and_caller() {
    for caller in [false, true] {
        let store = Arc::new(Store::default());
        let selected = seed(&store, "personal", "42", "selected-session");
        seed(&store, "other", "43", "other-session");
        let before = store.values.lock().unwrap().clone();
        store.forbid_reads.store(caller, Ordering::SeqCst);
        let mut fixture = setup(
            replies(vec![valid(), json_response(&directory())]),
            store.clone(),
        )
        .await;
        let provider = if caller {
            fixture
                .provider
                .caller_scope(&selected.caller().unwrap())
                .unwrap()
        } else {
            fixture.provider.clone()
        };
        let request = request((!caller).then_some("personal"), 1, 1);
        let page = if caller {
            provider.account_following_artists(&request).await
        } else {
            provider.user_following_artists("42", &request).await
        }
        .unwrap();
        assert_eq!(page.items.len(), 1);
        assert_eq!(page.items[0].id, "336");
        assert_eq!(page.pagination.total, Some(2));
        assert_eq!(page.pagination.extensions["library_owner_id"], "42");
        assert_eq!(*store.values.lock().unwrap(), before);
        assert!(provider.take_response_credential().unwrap().is_none());
        if caller {
            assert_eq!(store.reads.load(Ordering::SeqCst), 0);
        }
        let requests = fixture::requests(&mut fixture.network, 2).await;
        for (index, request) in requests.iter().enumerate() {
            assert!(!request.contains("other-session"));
            if index == 1 {
                assert!(request.contains("loginUid=42,loginSid=selected-session"));
            }
        }
    }
}

#[tokio::test]
async fn following_artists_rejects_foreign_accounts_and_bad_pages_before_network() {
    let store = Arc::new(Store::default());
    seed(&store, "personal", "42", "selected-session");
    let mut fixture = setup(vec![], store).await;
    assert_eq!(
        fixture
            .provider
            .user_following_artists("43", &request(Some("personal"), 10, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    for (account, code) in [
        (Some("missing"), ErrorCode::AuthenticationRequired),
        (Some("personal"), ErrorCode::InvalidRequest),
    ] {
        if account == Some("personal") {
            let caller = fixture
                .provider
                .caller_scope(
                    &fixture::credential_fixture("42", "selected-session")
                        .caller()
                        .unwrap(),
                )
                .unwrap();
            assert_eq!(
                caller
                    .account_following_artists(&request(account, 10, 0))
                    .await
                    .unwrap_err()
                    .code,
                code
            );
        } else {
            assert_eq!(
                fixture
                    .provider
                    .account_following_artists(&request(account, 10, 0))
                    .await
                    .unwrap_err()
                    .code,
                code
            );
        }
    }
    for request in [
        request(Some("personal"), 0, 0),
        request(Some("personal"), 101, 0),
        request(Some("personal"), 1, u32::MAX),
    ] {
        assert_eq!(
            fixture
                .provider
                .account_following_artists(&request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(fixture::requests(&mut fixture.network, 0).await.is_empty());
}

#[tokio::test]
async fn following_artists_discards_late_directory_response_after_logout() {
    let store = Arc::new(Store::default());
    seed(&store, "personal", "42", "selected-session");
    let gate = Arc::new(Notify::new());
    let mut fixture = setup(
        vec![
            (valid(), None),
            (json_response(&directory()), Some(gate.clone())),
        ],
        store.clone(),
    )
    .await;
    let provider = fixture.provider.clone();
    let task = tokio::spawn(async move {
        provider
            .account_following_artists(&request(Some("personal"), 10, 0))
            .await
    });
    super::super::tests::received(&mut fixture).await;
    super::super::tests::received(&mut fixture).await;
    store.remove(Platform::Kuwo, "personal").unwrap();
    gate.notify_one();
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    assert!(!store.values.lock().unwrap().contains_key("personal"));
}

#[tokio::test]
async fn artist_subscription_changes_only_the_selected_account_and_confirms_full_readback() {
    for caller in [false, true] {
        for subscribed in [true, false] {
            let before = if subscribed {
                vec!["810", "336"]
            } else {
                vec!["810", "999", "336"]
            };
            let after = if subscribed {
                vec!["810", "336", "999"]
            } else {
                vec!["810", "336"]
            };
            let store = Arc::new(Store::default());
            let selected = seed(&store, "personal", "42", "selected-session");
            seed(&store, "other", "43", "other-session");
            let stored_before = store.values.lock().unwrap().clone();
            store.forbid_reads.store(caller, Ordering::SeqCst);
            let mut fixture = setup(
                replies(vec![
                    valid(),
                    json_response(&directory_ids(&before)),
                    json_response(&json!({"result":"ok"})),
                    json_response(&directory_ids(&after)),
                ]),
                store.clone(),
            )
            .await;
            let provider = if caller {
                fixture
                    .provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                fixture.provider.clone()
            };
            let result = provider
                .set_artist_subscription("999", subscribed, (!caller).then_some("personal"))
                .await
                .unwrap();
            assert_eq!(
                result.resource_ref,
                ResourceRef::new(Platform::Kuwo, "999").unwrap()
            );
            assert_eq!(result.subscribed, subscribed);
            assert_eq!(result.extensions["source_user_id"], "42");
            assert_eq!(result.extensions["write_performed"], true);
            assert_eq!(result.extensions["write_requests_dispatched"], 1);
            assert_eq!(result.extensions["atomic"], false);
            assert_eq!(*store.values.lock().unwrap(), stored_before);
            assert!(provider.take_response_credential().unwrap().is_none());
            if caller {
                assert_eq!(store.reads.load(Ordering::SeqCst), 0);
            }

            let requests = fixture::requests(&mut fixture.network, 4).await;
            assert!(
                requests
                    .iter()
                    .all(|request| !request.contains("other-session"))
            );
            let write = &requests[2];
            assert!(write.starts_with("GET "));
            assert!(write.contains("loginUid=42,loginSid=selected-session"));
            let target = write.split_whitespace().nth(1).unwrap();
            let url = url::Url::parse(&format!("https://fixture.test{target}")).unwrap();
            let query = url.query_pairs().collect::<BTreeMap<_, _>>();
            assert_eq!(
                query["type"],
                if subscribed {
                    "click_like"
                } else {
                    "cancel_like"
                }
            );
            assert_eq!(query["uid"], "42");
            assert_eq!(query["digest"], "4");
            assert_eq!(query["sid"], "999");
            assert_eq!(query["loginSid"], "selected-session");
            assert_eq!(query["newver"], "3");
        }
    }
}

#[tokio::test]
async fn artist_subscription_is_idempotent_and_invalid_ids_fail_before_network() {
    let store = Arc::new(Store::default());
    seed(&store, "personal", "42", "selected-session");
    let mut fixture = setup(
        replies(vec![
            valid(),
            json_response(&directory_ids(&["810", "999"])),
        ]),
        store.clone(),
    )
    .await;
    let result = fixture
        .provider
        .set_artist_subscription("999", true, Some("personal"))
        .await
        .unwrap();
    assert_eq!(result.extensions["write_performed"], false);
    assert_eq!(result.extensions["write_requests_dispatched"], 0);
    let requests = fixture::requests(&mut fixture.network, 2).await;
    assert!(
        requests
            .iter()
            .all(|request| !request.contains("type=click_like"))
    );

    let mut fixture = setup(vec![], store).await;
    assert_eq!(
        fixture
            .provider
            .set_artist_subscription("0810", true, Some("personal"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(fixture::requests(&mut fixture.network, 0).await.is_empty());
}

#[tokio::test]
async fn artist_subscription_readback_mismatch_is_unconfirmed_without_retry() {
    let store = Arc::new(Store::default());
    seed(&store, "personal", "42", "selected-session");
    let mut fixture = setup(
        replies(vec![
            valid(),
            json_response(&directory_ids(&["810"])),
            json_response(&json!({"result":"ok"})),
            json_response(&directory_ids(&["810", "336"])),
        ]),
        store,
    )
    .await;
    let error = fixture
        .provider
        .set_artist_subscription("999", true, Some("personal"))
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(error.details["write_outcome"], "unconfirmed");
    assert_eq!(error.details["write_requests_dispatched"], 1);
    assert_eq!(error.details["automatic_retry"], false);
    fixture::requests(&mut fixture.network, 4).await;
}

#[tokio::test]
async fn artist_subscription_discards_a_late_write_after_selected_account_changes() {
    let store = Arc::new(Store::default());
    seed(&store, "personal", "42", "selected-session");
    let gate = Arc::new(Notify::new());
    let mut fixture = setup(
        vec![
            (valid(), None),
            (json_response(&directory_ids(&["810"])), None),
            (json_response(&json!({"result":"ok"})), Some(gate.clone())),
        ],
        store.clone(),
    )
    .await;
    let provider = fixture.provider.clone();
    let task = tokio::spawn(async move {
        provider
            .set_artist_subscription("999", true, Some("personal"))
            .await
    });
    for _ in 0..3 {
        super::super::tests::received(&mut fixture).await;
    }
    seed(&store, "personal", "42", "replacement-session");
    gate.notify_one();
    let error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert_eq!(error.details["write_outcome"], "unconfirmed");
    assert_eq!(error.details["write_requests_dispatched"], 1);
}
