use super::*;
use crate::account::cloud::tests::{Frame, server};
use crate::account::following::tests::{encrypted, full, singer};
use crate::provider::library::tests::store_account;
use crate::provider::session::tests::{credential, exchange, profile, raw, read};
use aes::{
    Aes256,
    cipher::{BlockDecryptMut, KeyIvInit, block_padding::Pkcs7},
};

fn start(ids: &[u64]) -> Vec<Frame> {
    vec![
        exchange("111", "next").into(),
        profile("111").into(),
        snapshot(ids, 37),
    ]
}
fn snapshot(ids: &[u64], version: u64) -> Frame {
    let mut value = full(ids.iter().copied().map(singer).collect());
    value["version"] = json!(version);
    encrypted(value)
}
fn ack() -> Frame {
    raw(json!({"status":1,"data":{"rank":2}})).into()
}
fn account(caller: bool) -> Option<&'static str> {
    if caller { None } else { Some("A") }
}

#[tokio::test]
async fn artist_subscription_provider_confirms_follow_and_unfollow_for_both_owners() {
    for caller in [false, true] {
        for subscribed in [false, true] {
            let mut frames = start(if subscribed { &[9, 7] } else { &[9, 42, 7] });
            frames.push(ack());
            frames.push(snapshot(if subscribed { &[42, 9, 7] } else { &[9, 7] }, 38));
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
            let result = provider
                .set_artist_subscription("42", subscribed, account(caller))
                .await
                .unwrap();
            assert_eq!(result.resource_ref.id(), "42");
            assert_eq!(result.subscribed, subscribed);
            assert_eq!(result.extensions["changed"], true);
            assert_eq!(result.extensions["source_version"], 38);
            assert_eq!(result.extensions["write_requests_dispatched"], 1);
            assert_eq!(read(&store, "B"), other);
            if caller {
                assert_eq!(read(&store, "A"), saved);
                assert!(provider.take_response_credential().unwrap().is_some());
            } else {
                assert_eq!(read(&store, "A").native().session.token, "next");
            }
            let requests = f.requests.await.unwrap();
            assert_eq!(requests.len(), 5);
            let write: serde_json::Value = serde_json::from_slice(&requests[3].body).unwrap();
            assert_eq!(write["userid"], 111);
            assert_eq!(write["singerid"], 42);
            let key = b"4032af8d61035123906e58e067140cc5";
            let mut encrypted = hex::decode(write["params"].as_str().unwrap()).unwrap();
            let decrypted = cbc::Decryptor::<Aes256>::new_from_slices(key, &key[16..])
                .unwrap()
                .decrypt_padded_mut::<Pkcs7>(&mut encrypted)
                .unwrap();
            assert_eq!(
                serde_json::from_slice::<serde_json::Value>(decrypted).unwrap(),
                json!({"singerid":42,"token":"next"})
            );
            assert!(!String::from_utf8_lossy(&requests[3].body).contains("next"));
        }
    }
}

#[tokio::test]
async fn artist_subscription_provider_noop_never_dispatches_write() {
    for subscribed in [false, true] {
        let f = server(start(if subscribed { &[42, 7] } else { &[7] })).await;
        let mut provider = KugouProvider::from_client(f.client);
        store_account(&mut provider);
        let result = provider
            .set_artist_subscription("42", subscribed, Some("A"))
            .await
            .unwrap();
        assert_eq!(result.extensions["changed"], false);
        assert_eq!(result.extensions["source_version"], 37);
        assert_eq!(result.extensions["write_requests_dispatched"], 0);
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn artist_subscription_provider_requires_full_valid_snapshots_before_and_after_write() {
    for after_write in [false, true] {
        // need_update=0 is not proof that the full selected-account directory is empty.
        let invalid = encrypted(json!({"status":1,"error_code":0,"need_update":0,"version":38}));
        let mut frames = start(&[7]);
        if after_write {
            frames.push(ack());
            frames.push(invalid);
        } else {
            *frames.last_mut().unwrap() = invalid;
        }
        let f = server(frames).await;
        let mut provider = KugouProvider::from_client(f.client);
        store_account(&mut provider);
        let error = provider
            .set_artist_subscription("42", true, Some("A"))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert_eq!(error.details.get("write_outcome").is_some(), after_write);
        assert_eq!(
            f.requests.await.unwrap().len(),
            if after_write { 5 } else { 3 }
        );
    }
}

#[tokio::test]
async fn artist_subscription_provider_rejects_unproven_or_unrelated_readback_delta() {
    for (ids, version) in [
        (vec![9, 7], 38),
        (vec![42, 9, 7], 36),
        (vec![42, 7, 9], 38),
        (vec![42, 9, 7, 8], 38),
        (vec![42, 9], 38),
    ] {
        let mut frames = start(&[9, 7]);
        frames.push(ack());
        frames.push(snapshot(&ids, version));
        let f = server(frames).await;
        let mut provider = KugouProvider::from_client(f.client);
        store_account(&mut provider);
        let error = provider
            .set_artist_subscription("42", true, Some("A"))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::Conflict);
        assert_eq!(error.details["write_outcome"], "unconfirmed");
        assert_eq!(error.details["write_requests_dispatched"], 1);
        assert!(!error.retryable);
        assert_eq!(f.requests.await.unwrap().len(), 5);
    }
}

#[tokio::test]
async fn artist_subscription_provider_rejects_invalid_target_account_and_client_before_network() {
    let f = server(vec![]).await;
    let mut provider = KugouProvider::from_client(f.client);
    let store = store_account(&mut provider);
    for id in ["0", "042", "-1", "42x", "9223372036854775808"] {
        assert_eq!(
            provider
                .set_artist_subscription(id, true, Some("A"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        provider
            .set_artist_subscription("42", true, Some(""))
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
            .set_artist_subscription("42", true, Some("A"))
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
            .set_artist_subscription("42", true, Some("A"))
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
            .set_artist_subscription("42", true, Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn artist_subscription_provider_rejects_ack_and_retains_only_eligible_rotation() {
    for caller in [false, true] {
        for auth in [false, true] {
            let mut frames = start(&[7]);
            frames.push(raw(json!({"status":0,"error_code":if auth {20017} else {20010},"data":{"msg":"secret-marker"}})).into());
            let f = server(frames).await;
            let mut owner = KugouProvider::from_client(f.client);
            let store = store_account(&mut owner);
            let saved = read(&store, "A");
            let provider = if caller {
                owner.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                owner
            };
            let mut error = provider
                .set_artist_subscription("42", true, account(caller))
                .await
                .unwrap_err();
            assert_eq!(
                error.code,
                if auth {
                    ErrorCode::AuthenticationRequired
                } else {
                    ErrorCode::UpstreamError
                }
            );
            assert_eq!(error.details["write_outcome"], "unconfirmed");
            assert!(!format!("{error:?}").contains("secret-marker"));
            assert_eq!(
                error.take_caller_credential_update().is_some(),
                caller && !auth
            );
            assert!(!error.retryable);
            if caller {
                assert_eq!(read(&store, "A"), saved);
            }
            assert_eq!(f.requests.await.unwrap().len(), 4);
        }
    }
}

#[tokio::test]
async fn artist_subscription_provider_relogin_at_every_boundary_discards_late_success_or_error() {
    for caller in [false, true] {
        for boundary in [3, 4, 5] {
            for failure in [false, true] {
                let mut frames = start(&[7]);
                if boundary >= 4 {
                    frames.push(ack());
                }
                if boundary >= 5 {
                    frames.push(snapshot(&[42, 7], 38));
                }
                let (resume, gate) = tokio::sync::oneshot::channel();
                let last = frames.last_mut().unwrap();
                if failure {
                    *last = raw(json!({"status":0,"error_code":20017})).into();
                }
                last.gate = Some(gate);
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
                let task = tokio::spawn(async move {
                    p.set_artist_subscription("42", true, account(caller)).await
                });
                for _ in 0..boundary {
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
                let mut error = task.await.unwrap().unwrap_err();
                assert_eq!(error.code, ErrorCode::Conflict);
                assert!(error.take_caller_credential_update().is_none());
                if boundary > 3 {
                    assert_eq!(error.details["write_outcome"], "unconfirmed");
                }
                assert_eq!(read(&store, "A"), if caller { saved } else { replacement });
                assert_eq!(f.requests.await.unwrap().len(), boundary);
            }
        }
    }
}
