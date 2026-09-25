use super::*;
use crate::account::cloud::tests::{Frame, server};
use crate::account::following::tests::{encrypted, full, singer};
use crate::provider::library::tests::store_account;
use crate::provider::session::tests::{credential, exchange, profile, raw, read};

fn start() -> Vec<Frame> {
    vec![exchange("111", "next").into(), profile("111").into()]
}
fn request(caller: bool, limit: u32, offset: u32) -> PageRequest {
    PageRequest {
        account: (!caller).then(|| "A".into()),
        limit,
        offset,
    }
}

#[tokio::test]
async fn followed_artists_provider_slices_complete_order_for_exact_server_or_caller_identity() {
    for caller in [false, true] {
        for explicit_uid in [false, true] {
            let mut frames = start();
            frames.push(encrypted(full(vec![singer(42), singer(7), singer(9)])));
            let f = server(frames).await;
            let mut owner = KugouProvider::from_client(f.client);
            let store = store_account(&mut owner);
            let saved = read(&store, "A");
            let other = read(&store, "B");
            let provider = if caller {
                owner.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                owner
            };
            let r = request(caller, 1, 1);
            let page = if explicit_uid {
                provider.user_following_artists("111", &r).await
            } else {
                provider.account_following_artists(&r).await
            }
            .unwrap();
            assert_eq!(page.items[0].id, "7");
            assert_eq!(page.items[0].extensions["linked_user_id"], "999999");
            assert_eq!(page.pagination.total, Some(3));
            assert_eq!(page.pagination.next_offset, Some(2));
            assert_eq!(page.pagination.extensions["library_owner_id"], "111");
            assert_eq!(page.pagination.extensions["source_version"], 37);
            assert_eq!(page.pagination.extensions["complete_read"], true);
            assert_eq!(read(&store, "B"), other);
            if caller {
                assert_eq!(read(&store, "A"), saved);
                assert!(provider.take_response_credential().unwrap().is_some());
            } else {
                assert_eq!(read(&store, "A").native().session.token, "next");
            }
            assert_eq!(f.requests.await.unwrap().len(), 3);
        }
    }
}

#[tokio::test]
async fn followed_artists_provider_preserves_empty_and_beyond_end_but_validates_unsliced_rows() {
    for case in ["empty", "beyond", "late_bad"] {
        let mut frames = start();
        let rows = match case {
            "empty" => vec![],
            "beyond" => vec![singer(42)],
            _ => vec![singer(42), json!({"id":0,"name":"Invalid later row"})],
        };
        frames.push(encrypted(full(rows)));
        let f = server(frames).await;
        let mut provider = KugouProvider::from_client(f.client);
        store_account(&mut provider);
        let result = provider
            .account_following_artists(&request(false, 1, if case == "beyond" { 100 } else { 0 }))
            .await;
        if case == "late_bad" {
            assert!(result.is_err());
        } else {
            let page = result.unwrap();
            assert!(page.items.is_empty());
            assert!(!page.pagination.has_more);
            assert_eq!(
                page.pagination.total,
                Some(if case == "empty" { 0 } else { 1 })
            );
        }
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn followed_artists_provider_rejects_wrong_owner_bad_paging_and_other_clients_without_network()
 {
    let f = server(vec![]).await;
    let mut provider = KugouProvider::from_client(f.client);
    let store = store_account(&mut provider);
    for r in [
        request(false, 0, 0),
        request(false, 101, 0),
        request(false, 2, u32::MAX),
    ] {
        assert_eq!(
            provider
                .account_following_artists(&r)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        provider
            .user_following_artists("222", &request(false, 1, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        provider
            .user_following_artists("0111", &request(false, 1, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let caller = provider
        .caller_scope(&read(&store, "A").caller().unwrap())
        .unwrap();
    assert_eq!(
        caller
            .account_following_artists(&request(false, 1, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let mut session = read(&store, "A").native().session.clone();
    session.client = KugouLoginClient::Concept;
    store
        .put(
            &KugouCredential::verified(session)
                .unwrap()
                .stored("A")
                .unwrap(),
        )
        .unwrap();
    assert_eq!(
        provider
            .account_following_artists(&request(false, 1, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    store
        .put(
            &KugouCredential::verified_web(crate::web::WebSession::test_session(
                "111",
                "web-token",
            ))
            .unwrap()
            .stored("A")
            .unwrap(),
        )
        .unwrap();
    assert_eq!(
        provider
            .account_following_artists(&request(false, 1, 0))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn followed_artists_provider_business_errors_keep_rotation_and_auth_errors_suppress_it() {
    for caller in [false, true] {
        for auth in [false, true] {
            let mut frames = start();
            frames.push(raw(json!({"status":0,"error_code":if auth {20017} else {20010},"data":"secret-marker"})).into());
            let f = server(frames).await;
            let mut owner = KugouProvider::from_client(f.client);
            let store = store_account(&mut owner);
            let saved = read(&store, "A");
            let provider = if caller {
                owner.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                owner
            };
            let mut e = provider
                .account_following_artists(&request(caller, 1, 0))
                .await
                .unwrap_err();
            assert_eq!(
                e.code,
                if auth {
                    ErrorCode::AuthenticationRequired
                } else {
                    ErrorCode::UpstreamError
                }
            );
            assert_eq!(e.take_caller_credential_update().is_some(), caller && !auth);
            assert!(!format!("{e:?}").contains("secret-marker"));
            if caller {
                assert_eq!(read(&store, "A"), saved);
            }
            assert_eq!(f.requests.await.unwrap().len(), 3);
        }
    }
}

#[tokio::test]
async fn followed_artists_provider_relogin_discards_late_success_or_error_in_both_ownerships() {
    for caller in [false, true] {
        for failure in [false, true] {
            let mut frames = start();
            let mut last = if failure {
                Frame::from(raw(json!({"status":0,"error_code":20017})))
            } else {
                encrypted(full(vec![singer(7)]))
            };
            let (resume, gate) = tokio::sync::oneshot::channel();
            last.gate = Some(gate);
            frames.push(last);
            let mut f = server(frames).await;
            let mut owner = KugouProvider::from_client(f.client);
            let store = store_account(&mut owner);
            let saved = read(&store, "A");
            let provider = if caller {
                owner.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                owner
            };
            let p = provider.clone();
            let task =
                tokio::spawn(
                    async move { p.account_following_artists(&request(caller, 1, 0)).await },
                );
            for _ in 0..3 {
                f.seen.recv().await.unwrap();
            }
            let replacement = credential("111", "new-login");
            if caller {
                *provider.caller_credential.as_ref().unwrap().lock().unwrap() =
                    Some(replacement.clone());
            } else {
                store.put(&replacement.stored("A").unwrap()).unwrap();
            }
            resume.send(()).unwrap();
            let mut e = task.await.unwrap().unwrap_err();
            assert_eq!(e.code, ErrorCode::Conflict);
            assert!(e.take_caller_credential_update().is_none());
            assert_eq!(read(&store, "A"), if caller { saved } else { replacement });
            assert_eq!(f.requests.await.unwrap().len(), 3);
        }
    }
}
