use super::super::tests::{Store, received, replies, seed, setup, stored, valid};
use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests::credential_fixture,
};
use tokio::sync::Notify;
use tuneweave_core::AccountCredentialStore;
fn invalidated() -> Vec<u8> {
    json_response(&json!({"result":"fail","reason":"error_user_invalid"}))
}
fn receipt() -> Vec<u8> {
    response(200, "text/plain", "", b"result=ok\n")
}

#[tokio::test]
async fn revocation_verifies_invalidation_and_respects_all_ownership_modes() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let store = Arc::new(Store::default());
        let source = seed(&store, "personal", "42", "sid-42").caller().unwrap();
        let f = setup(replies(vec![valid(), receipt(), invalidated()]), store).await;
        let account = if mode == CredentialMode::Client {
            "default"
        } else {
            "personal"
        };
        let caller = if mode == CredentialMode::Server {
            None
        } else {
            Some(&source)
        };
        let result = f
            .provider
            .revoke_session_with_ownership(account, caller, mode)
            .await
            .unwrap();
        assert_eq!(result.state, SessionRevocationState::Invalidated);
        assert!(result.revocation_request_started);
        assert_eq!(result.removed, mode.persists_on_server());
        assert_eq!(
            result.caller_credential_discard_required,
            mode.returns_to_caller()
        );
        assert_eq!(
            stored(&f.store, "personal").is_some(),
            !mode.persists_on_server()
        );
    }
}

#[tokio::test]
async fn revocation_no_session_and_already_invalid_do_not_send_a_write() {
    let store = Arc::new(Store::default());
    let f = setup(vec![], store.clone()).await;
    let result = f
        .provider
        .revoke_session_with_ownership("missing", None, CredentialMode::Server)
        .await
        .unwrap();
    assert_eq!(result.state, SessionRevocationState::NoStoredSession);
    assert!(!result.revocation_request_started);
    seed(&store, "personal", "42", "sid-42");
    let f = setup(replies(vec![invalidated()]), store).await;
    let result = f
        .provider
        .revoke_session_with_ownership("personal", None, CredentialMode::Server)
        .await
        .unwrap();
    assert_eq!(result.state, SessionRevocationState::AlreadyInvalid);
    assert!(result.removed);
    assert!(!result.revocation_request_started);
}

#[tokio::test]
async fn revocation_transport_auth_failure_is_not_expiry_and_preserves_unattempted_session() {
    for body in [
        response(401, "application/json", "", b"{}"),
        json_response(&json!({"result":"fail","reason":"unknown"})),
        json_response(&json!({"result":"ok","reason":"contradiction"})),
    ] {
        let store = Arc::new(Store::default());
        seed(&store, "personal", "42", "sid-42");
        let f = setup(replies(vec![body]), store).await;
        let error = f
            .provider
            .revoke_session_with_ownership("personal", None, CredentialMode::Server)
            .await
            .unwrap_err();
        assert_eq!(error.details["revocation_request_started"], false);
        assert_eq!(error.details["removed"], false);
        assert!(stored(&f.store, "personal").is_some());
    }
}

#[tokio::test]
async fn revocation_receipt_and_http_errors_require_independent_post_validation() {
    for wire in [receipt(), response(503, "text/plain", "", b"unavailable")] {
        for still_valid in [true, false] {
            let store = Arc::new(Store::default());
            seed(&store, "personal", "42", "sid-42");
            let f = setup(
                replies(vec![
                    valid(),
                    wire.clone(),
                    if still_valid { valid() } else { invalidated() },
                ]),
                store,
            )
            .await;
            let result = f
                .provider
                .revoke_session_with_ownership("personal", None, CredentialMode::Server)
                .await;
            assert!(stored(&f.store, "personal").is_none());
            if still_valid {
                let error = result.unwrap_err();
                assert_eq!(error.details["upstream_outcome"], "unconfirmed");
                assert!(!error.retryable);
            } else {
                assert_eq!(result.unwrap().state, SessionRevocationState::Invalidated);
            }
        }
    }
}

#[tokio::test]
async fn revocation_late_responses_at_each_boundary_preserve_new_login() {
    for boundary in 0..3 {
        let gate = Arc::new(Notify::new());
        let store = Arc::new(Store::default());
        seed(&store, "personal", "42", "sid-42");
        let responses = [valid(), receipt(), invalidated()]
            .into_iter()
            .enumerate()
            .map(|(i, b)| {
                (
                    b,
                    if i == boundary {
                        Some(gate.clone())
                    } else {
                        None
                    },
                )
            })
            .collect();
        let mut f = setup(responses, store).await;
        let provider = f.provider.clone();
        let task = tokio::spawn(async move {
            provider
                .revoke_session_with_ownership("personal", None, CredentialMode::Server)
                .await
        });
        for _ in 0..=boundary {
            received(&mut f).await;
        }
        seed(&f.store, "personal", "84", "new-sid");
        let replacement = stored(&f.store, "personal");
        gate.notify_one();
        assert!(task.await.unwrap().is_err());
        assert_eq!(stored(&f.store, "personal"), replacement);
    }
}

#[tokio::test]
async fn revocation_cancel_and_timeout_after_send_clean_only_selected_login() {
    for cancel in [false, true] {
        let gate = Arc::new(Notify::new());
        let store = Arc::new(Store::default());
        seed(&store, "personal", "42", "sid-42");
        let mut f = setup(vec![(valid(), None), (receipt(), Some(gate))], store).await;
        let provider = f.provider.clone();
        let task = tokio::spawn(async move {
            provider
                .revoke_owned(
                    "personal",
                    None,
                    CredentialMode::Server,
                    Duration::from_millis(300),
                )
                .await
        });
        received(&mut f).await;
        received(&mut f).await;
        if cancel {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            assert_eq!(
                task.await.unwrap().unwrap_err().code,
                ErrorCode::UpstreamTimeout
            );
        }
        assert!(stored(&f.store, "personal").is_none());
    }
}

#[tokio::test]
async fn revocation_foreign_caller_rejected_before_any_network_or_cleanup() {
    let store = Arc::new(Store::default());
    seed(&store, "personal", "42", "sid-42");
    let f = setup(vec![], store).await;
    let foreign = credential_fixture("84", "other-sid").caller().unwrap();
    assert!(
        f.provider
            .revoke_session_with_ownership("personal", Some(&foreign), CredentialMode::Both)
            .await
            .is_err()
    );
    assert!(stored(&f.store, "personal").is_some());
}

#[tokio::test]
async fn revocation_post_http_401_is_unconfirmed_and_cleanup_failure_is_reported() {
    for fail_cleanup in [false, true] {
        let store = Arc::new(Store::default());
        seed(&store, "personal", "42", "sid-42");
        store
            .fail_write
            .store(fail_cleanup, std::sync::atomic::Ordering::SeqCst);
        let f = setup(
            replies(vec![
                valid(),
                receipt(),
                if fail_cleanup {
                    invalidated()
                } else {
                    response(401, "application/json", "", b"{}")
                },
            ]),
            store,
        )
        .await;
        let error = f
            .provider
            .revoke_session_with_ownership("personal", None, CredentialMode::Server)
            .await
            .unwrap_err();
        assert_eq!(error.details["revocation_request_started"], true);
        assert_eq!(
            error.details["upstream_outcome"],
            if fail_cleanup {
                "invalidated"
            } else {
                "unconfirmed"
            }
        );
        assert_eq!(
            error.details["local_cleanup"],
            if fail_cleanup { "failed" } else { "removed" }
        );
        assert_eq!(stored(&f.store, "personal").is_some(), fail_cleanup);
    }
}

#[tokio::test]
async fn revocation_cancel_during_preflight_keeps_credentials() {
    let store = Arc::new(Store::default());
    seed(&store, "personal", "42", "sid-42");
    let mut f = setup(vec![(valid(), Some(Arc::new(Notify::new())))], store).await;
    let provider = f.provider.clone();
    let task = tokio::spawn(async move {
        provider
            .revoke_session_with_ownership("personal", None, CredentialMode::Server)
            .await
    });
    received(&mut f).await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(stored(&f.store, "personal").is_some());
}

#[tokio::test]
async fn revocation_of_old_sid_never_removes_a_rotated_sid_in_the_same_generation() {
    let gate = Arc::new(Notify::new());
    let store = Arc::new(Store::default());
    let original = seed(&store, "personal", "42", "sid-42");
    let mut value: serde_json::Value = serde_json::to_value(&original).unwrap();
    value["session_id"] = json!("rotated-sid-42");
    let rotated: crate::client::native::credential::NativeCredential =
        serde_json::from_value(value).unwrap();
    assert!(original.same_login(&rotated));
    let mut f = setup(
        vec![(valid(), None), (receipt(), Some(gate.clone()))],
        store.clone(),
    )
    .await;
    let provider = f.provider.clone();
    let task = tokio::spawn(async move {
        provider
            .revoke_session_with_ownership("personal", None, CredentialMode::Server)
            .await
    });
    received(&mut f).await;
    received(&mut f).await;
    store.put(&rotated.stored("personal").unwrap()).unwrap();
    let expected = stored(&f.store, "personal");
    gate.notify_one();
    assert!(task.await.unwrap().is_err());
    assert_eq!(stored(&f.store, "personal"), expected);
}
