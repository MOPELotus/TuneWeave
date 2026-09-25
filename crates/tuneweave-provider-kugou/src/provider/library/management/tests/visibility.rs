use super::*;
use tuneweave_core::PlaylistVisibilityUpdateRequest;

fn visible_row(id: u64, private: bool) -> Value {
    let mut value = row(id);
    value["is_pri"] = json!(u8::from(private));
    value["is_mutual"] = json!(0);
    value
}

fn visibility(private: bool, caller: bool) -> PlaylistVisibilityUpdateRequest {
    PlaylistVisibilityUpdateRequest {
        visibility: if private {
            PlaylistVisibility::Private
        } else {
            PlaylistVisibility::Public
        },
        account: (!caller).then(|| "A".into()),
    }
}

#[tokio::test]
async fn visibility_switches_both_directions_preserving_metadata_and_exact_credential_owner() {
    for caller in [false, true] {
        for private in [false, true] {
            let before = visible_row(37, !private);
            let mut changed = visible_row(37, private);
            changed["list_ver"] = json!(4);
            let mut frames = start();
            frames.push(library(vec![before, row(3)], 9).into());
            frames.push(accepted(37, 0, 9, 10, 2));
            frames.push(library(vec![changed, row(3)], 10).into());
            let mut f = server(frames).await;
            let store = store_account(&mut f.provider);
            let saved = read(&store, "A");
            let other = read(&store, "B");
            let provider = if caller {
                f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let result = provider
                .update_playlist_visibility(REF, &visibility(private, caller))
                .await
                .unwrap();
            let playlist = result.playlist.unwrap();
            assert_eq!(playlist.extensions["is_private"], private);
            assert_eq!(playlist.extensions["is_mutual"], false);
            assert_eq!(playlist.description, "  Preserve intro\n");
            assert_eq!(playlist.tags, ["甲", "乙"]);
            assert_eq!(result.extensions["changed"], true);
            assert_eq!(result.extensions["write_requests_dispatched"], 1);
            assert_eq!(read(&store, "B"), other);
            if caller {
                assert_eq!(read(&store, "A"), saved);
                assert!(provider.take_response_credential().unwrap().is_some());
            } else {
                assert_eq!(read(&store, "A").native().session.token, "next");
            }
            let all = f.requests.await.unwrap();
            assert_eq!(all.len(), 5);
            assert!(all[3].starts_with("POST /cloudlist.service/v4/modify_list?"));
            let payload = body(&all[3]);
            assert_eq!(payload["is_pri"], u8::from(private));
            assert_eq!(payload["is_mutual"], 0);
            assert_eq!(payload["support_pub"], 1);
            assert_eq!(payload["sort"], 42);
            for omitted in ["intro", "tags", "pic", "list_create_gid", "token"] {
                assert!(payload.get(omitted).is_none(), "{omitted}");
            }
        }
    }
}

#[tokio::test]
async fn visibility_noops_do_not_write_and_unknown_or_collaborative_system_lists_are_rejected() {
    for case in [
        "noop",
        "collaborative",
        "missing_mutual",
        "missing_private",
        "missing_sort",
        "system",
        "missing_system",
    ] {
        let mut selected = visible_row(37, false);
        match case {
            "collaborative" => selected["is_mutual"] = json!(1),
            "system" => selected["is_def"] = json!(2),
            "missing_mutual" | "missing_private" | "missing_sort" | "missing_system" => {
                selected.as_object_mut().unwrap().remove(match case {
                    "missing_mutual" => "is_mutual",
                    "missing_private" => "is_pri",
                    "missing_sort" => "sort",
                    _ => "is_def",
                });
            }
            _ => {}
        }
        let mut frames = start();
        frames.push(library(vec![selected], 9).into());
        let mut f = server(frames).await;
        store_account(&mut f.provider);
        let outcome = f
            .provider
            .update_playlist_visibility(REF, &visibility(case != "noop", false))
            .await;
        if case == "noop" {
            let result = outcome.unwrap();
            assert_eq!(result.extensions["changed"], false);
            assert_eq!(result.extensions["write_requests_dispatched"], 0);
        } else {
            let error = outcome.unwrap_err();
            assert_eq!(
                error.code,
                if case == "system" {
                    ErrorCode::PermissionDenied
                } else {
                    ErrorCode::CapabilityNotSupported
                }
            );
            assert!(error.details.get("write_outcome").is_none());
        }
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn visibility_rejects_other_clients_owner_and_invalid_targets_before_network() {
    let mut f = server(vec![]).await;
    let store = store_account(&mut f.provider);
    for (id, request, code) in [
        (
            "cloudlist:222:0:37",
            visibility(true, false),
            ErrorCode::PermissionDenied,
        ),
        (
            "cloudlist:111:1:37",
            visibility(true, false),
            ErrorCode::PermissionDenied,
        ),
        (
            REF,
            PlaylistVisibilityUpdateRequest {
                visibility: PlaylistVisibility::PlatformDefault,
                account: Some("A".into()),
            },
            ErrorCode::InvalidRequest,
        ),
    ] {
        assert_eq!(
            f.provider
                .update_playlist_visibility(id, &request)
                .await
                .unwrap_err()
                .code,
            code
        );
    }
    let caller = f
        .provider
        .caller_scope(&read(&store, "A").caller().unwrap())
        .unwrap();
    assert_eq!(
        caller
            .update_playlist_visibility(REF, &visibility(true, false))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    store_client(&mut f.provider, KugouLoginClient::Concept);
    assert_eq!(
        f.provider
            .update_playlist_visibility(REF, &visibility(true, false))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    let store = f.provider.credential_store.as_ref().unwrap();
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
        f.provider
            .update_playlist_visibility(REF, &visibility(true, false))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn visibility_readback_rejects_wrong_privacy_collaboration_metadata_order_and_ack_version() {
    for case in [
        "privacy",
        "mutual",
        "name",
        "intro",
        "sort",
        "unrelated",
        "ack_version",
        "missing",
    ] {
        let mut changed = visible_row(37, true);
        let mut other = row(3);
        match case {
            "privacy" => changed["is_pri"] = json!(0),
            "mutual" => changed["is_mutual"] = json!(1),
            "name" => changed["name"] = json!("Other"),
            "intro" => changed["intro"] = json!("Cleared"),
            "sort" => changed["sort"] = json!(45),
            "unrelated" => other["name"] = json!("Other changed"),
            _ => {}
        }
        let mut frames = start();
        frames.push(library(vec![visible_row(37, false), row(3)], 9).into());
        frames.push(accepted(
            37,
            0,
            9,
            if case == "ack_version" { 11 } else { 10 },
            2,
        ));
        frames.push(
            library(
                if case == "missing" {
                    vec![other]
                } else {
                    vec![changed, other]
                },
                10,
            )
            .into(),
        );
        let mut f = server(frames).await;
        store_account(&mut f.provider);
        let e = f
            .provider
            .update_playlist_visibility(REF, &visibility(true, false))
            .await
            .unwrap_err();
        assert_eq!(e.details["write_outcome"], "unconfirmed", "{case}");
        assert_eq!(e.details["write_requests_dispatched"], 1);
        assert!(!e.retryable);
        assert_eq!(f.requests.await.unwrap().len(), 5);
    }
}

#[tokio::test]
async fn visibility_auth_and_business_failures_preserve_rotation_rules_without_auto_retry() {
    for caller in [false, true] {
        for auth in [false, true] {
            let mut frames = start();
            frames.push(library(vec![visible_row(37, false)], 9).into());
            frames.push(raw(json!({"status":0,"error_code":if auth {20017} else {20010},"data":"secret-marker"})).into());
            let mut f = server(frames).await;
            let store = store_account(&mut f.provider);
            let saved = read(&store, "A");
            let provider = if caller {
                f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let mut e = provider
                .update_playlist_visibility(REF, &visibility(true, caller))
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
            assert_eq!(e.details["write_outcome"], "unconfirmed");
            assert!(!e.retryable);
            assert!(!format!("{e:?}").contains("secret-marker"));
            assert_eq!(e.take_caller_credential_update().is_some(), caller && !auth);
            if caller {
                assert_eq!(read(&store, "A"), saved);
            }
            assert_eq!(f.requests.await.unwrap().len(), 4);
        }
    }
}

#[tokio::test]
async fn visibility_relogin_at_each_library_or_write_boundary_prevents_stale_credential_commit() {
    for caller in [false, true] {
        for boundary in ["before", "write", "after"] {
            let mut frames = start();
            if boundary != "before" {
                frames.push(library(vec![visible_row(37, false)], 9).into());
            }
            if boundary == "after" {
                frames.push(accepted(37, 0, 9, 10, 1));
            }
            let (last, resume) = paused(match boundary {
                "before" => library(vec![visible_row(37, false)], 9),
                "write" => reply(json!({"userid":111,"info":{"listid":37,"type":0,"code":1}})),
                _ => library(vec![visible_row(37, true)], 10),
            });
            frames.push(last);
            let n = frames.len();
            let mut f = server(frames).await;
            let store = store_account(&mut f.provider);
            let saved = read(&store, "A");
            let provider = if caller {
                f.provider.caller_scope(&saved.caller().unwrap()).unwrap()
            } else {
                f.provider.clone()
            };
            let p = provider.clone();
            let task = tokio::spawn(async move {
                p.update_playlist_visibility(REF, &visibility(true, caller))
                    .await
            });
            for _ in 0..n {
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
            if boundary == "before" {
                assert!(e.details.get("write_outcome").is_none());
            } else {
                assert_eq!(e.details["write_outcome"], "unconfirmed");
            }
            assert!(e.take_caller_credential_update().is_none());
            assert_eq!(read(&store, "A"), if caller { saved } else { replacement });
            assert_eq!(f.requests.await.unwrap().len(), n);
        }
    }
}
