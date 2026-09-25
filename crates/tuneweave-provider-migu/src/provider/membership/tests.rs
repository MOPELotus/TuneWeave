use super::*;
use crate::credential::MiguCredential;
use crate::{
    client::membership::tests::card,
    provider::{
        catalog::tests::server,
        session::tests::{Store, gated, profile, read, stored},
    },
};
use std::time::Duration;

fn reply(data: serde_json::Value, token: &str) -> String {
    let body = json!({"code":"000000","data":data}).to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\npacmtoken: {token}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}
fn replies() -> Vec<String> {
    vec![
        profile("111", "pacmtoken: p1\r\n"),
        reply(
            json!({"userId":"111","memberCards":[card("baijinhuiyuan","baijinhuiyuan","20261001")]}),
            "center",
        ),
        profile("111", "pacmtoken: p2\r\n"),
        reply(
            json!([{"memberTypeName":"Member","iconUrl":"https://d.musicapp.migu.cn/icon.png","actionUrl":"do-not-export"}]),
            "icons",
        ),
        profile("111", "pacmtoken: p3\r\n"),
    ]
}
fn detailed_replies() -> Vec<String> {
    let mut result = replies();
    result.push(reply(json!({"userId":"111","mediaMemberIdentities":[{"identityFullPinYin":"video_bundle","name":"Other media","payType":"01","validTime":"20261101","desc":"do-not-export"}]}), "media"));
    result.push(profile("111", "pacmtoken: p4\r\n"));
    result
}
fn setup(provider: &mut MiguProvider) -> (Arc<Store>, MiguCredential, MiguCredential) {
    let a = MiguCredential::verified("111".into(), "initial".into()).unwrap();
    let b = MiguCredential::verified("222".into(), "other".into()).unwrap();
    let store = Arc::new(Store::default());
    store.put(&stored("A", &a)).unwrap();
    store.put(&stored("B", &b)).unwrap();
    provider.credential_store = Some(store.clone());
    (store, a, b)
}

#[tokio::test]
async fn membership_uses_selected_identity_and_verifies_rotations_in_server_and_caller_modes() {
    for caller in [false, true] {
        for client_backend in [false, true] {
            let (mut provider, requests) = server(if client_backend {
                detailed_replies()
            } else {
                replies()
            })
            .await;
            let (store, original, other) = setup(&mut provider);
            let account = if caller {
                provider = provider.caller_scope(&original.caller().unwrap()).unwrap();
                "default"
            } else {
                "A"
            };
            let result = if client_backend {
                provider
                    .user_membership_client_info(Some("111"), Some(account))
                    .await
            } else {
                provider.user_membership(None, Some(account)).await
            }
            .unwrap();
            assert_eq!(result.active, Some(true));
            assert_eq!(result.user_ref.unwrap().id(), "111");
            assert_eq!(result.expires_at.as_deref(), Some("2026-10-01"));
            assert_eq!(
                result.icon_url.as_deref(),
                Some("https://d.musicapp.migu.cn/icon.png")
            );
            let serialized = serde_json::to_string(&result.extensions).unwrap();
            for secret in ["initial", "do-not-export", "pacmtoken"] {
                assert!(!serialized.contains(secret));
            }
            let update = provider.take_response_credential().unwrap();
            if caller {
                assert_eq!(
                    MiguCredential::parse_caller(&update.unwrap())
                        .unwrap()
                        .token(),
                    if client_backend { "p4" } else { "p3" }
                );
                assert_eq!(read(&store, "A"), original);
            } else {
                assert!(update.is_none());
                assert_eq!(
                    read(&store, "A").token(),
                    if client_backend { "p4" } else { "p3" }
                );
            }
            assert_eq!(read(&store, "B"), other);
            let requests = requests.await.unwrap();
            assert_eq!(requests.len(), if client_backend { 7 } else { 5 });
            assert_eq!(
                result.extensions.contains_key("media_member_identities"),
                client_backend
            );
            if client_backend {
                assert!(requests[5].starts_with("GET /user/i/member/identity/v1.0 HTTP/1.1"));
            }
            for (request, token) in requests
                .iter()
                .zip(["initial", "p1", "center", "p2", "icons", "p3", "media"])
            {
                assert!(request.contains(&format!("pacmtoken: {token}\r\n")));
                assert!(!request.contains("cookie:"));
                assert_eq!(request.matches("\r\nreferer:").count(), 1);
            }
            assert!(requests[1].starts_with("GET /user/member/center/v3.0 HTTP/1.1"));
            assert!(requests[3].starts_with("GET /pc/open/api/member/icon/v1.0 HTTP/1.1"));
            for (index, channel) in [(1, "014021I"), (3, "014X031")] {
                assert!(requests[index].contains(&format!("channel: {channel}\r\n")));
                assert!(requests[index].contains("deviceid:"));
                assert!(requests[index].contains("platform: H5\r\n"));
            }
            assert!(requests[1].contains("referer: https://h5.nf.migu.cn/\r\n"));
            assert!(requests[3].contains("referer: https://music.migu.cn/\r\n"));
        }
    }
}

#[tokio::test]
async fn membership_rejects_missing_or_other_accounts_without_network_and_keeps_base_scope() {
    let (mut provider, requests) = server(vec![]).await;
    let (_, original, _) = setup(&mut provider);
    assert_eq!(
        provider
            .user_membership(None, Some("missing"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert_eq!(
        provider
            .user_membership(Some("222"), Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(
        provider
            .user_membership(Some("bad id"), Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let caller = provider.caller_scope(&original.caller().unwrap()).unwrap();
    assert_eq!(
        caller
            .user_membership(None, Some("A"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(provider.take_response_credential().unwrap().is_none());
    assert!(requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn membership_rotation_is_retained_after_field_failure_but_unverified_tokens_are_never_exported()
 {
    for failed_identity in [false, true] {
        for caller in [false, true] {
            let mut responses = replies();
            responses.truncate(3);
            responses[1] = reply(json!({"memberCards":[]}), "unverified");
            responses[2] = profile(
                if failed_identity { "222" } else { "111" },
                "pacmtoken: verified\r\n",
            );
            let (mut provider, requests) = server(responses).await;
            let (store, original, other) = setup(&mut provider);
            let account = if caller {
                provider = provider.caller_scope(&original.caller().unwrap()).unwrap();
                "default"
            } else {
                "A"
            };
            let mut error = provider
                .user_membership(None, Some(account))
                .await
                .unwrap_err();
            let update = provider.take_response_credential().unwrap();
            if failed_identity {
                assert_eq!(error.code, ErrorCode::AuthenticationRequired);
                assert!(update.is_none());
                assert!(error.take_caller_credential_update().is_none());
                if !caller {
                    assert!(
                        !store
                            .load_platform(Platform::Migu)
                            .unwrap()
                            .iter()
                            .any(|v| v.account == "A")
                    );
                }
            } else {
                assert_eq!(error.code, ErrorCode::UpstreamError);
                if caller {
                    assert_eq!(
                        MiguCredential::parse_caller(&update.unwrap())
                            .unwrap()
                            .token(),
                        "verified"
                    );
                    assert_eq!(
                        MiguCredential::parse_caller(
                            &error.take_caller_credential_update().unwrap()
                        )
                        .unwrap()
                        .token(),
                        "verified"
                    );
                } else {
                    assert_eq!(read(&store, "A").token(), "verified");
                    assert!(update.is_none());
                }
            }
            if caller {
                assert_eq!(read(&store, "A"), original);
            }
            assert_eq!(read(&store, "B"), other);
            assert_eq!(requests.await.unwrap().len(), 3);
        }
    }
}

#[tokio::test]
async fn membership_late_results_cannot_overwrite_relogin_or_revive_a_removed_alias() {
    for remove in [false, true] {
        for stage in 1..=7 {
            let (mut provider, seen, release, server) =
                gated(detailed_replies()[..stage].to_vec()).await;
            let (store, _, other) = setup(&mut provider);
            let task =
                tokio::spawn(
                    async move { provider.user_membership_client_info(None, Some("A")).await },
                );
            tokio::time::timeout(Duration::from_secs(5), seen)
                .await
                .unwrap()
                .unwrap();
            let replacement = MiguCredential::verified("333".into(), "replacement".into()).unwrap();
            if remove {
                store.remove(Platform::Migu, "A").unwrap();
            } else {
                store.put(&stored("A", &replacement)).unwrap();
            }
            release.send(()).unwrap();
            let mut error = tokio::time::timeout(Duration::from_secs(5), task)
                .await
                .unwrap()
                .unwrap()
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::Conflict, "stage {stage}");
            assert!(error.take_caller_credential_update().is_none());
            if remove {
                assert!(
                    !store
                        .load_platform(Platform::Migu)
                        .unwrap()
                        .iter()
                        .any(|v| v.account == "A")
                );
            } else {
                assert_eq!(read(&store, "A"), replacement);
            }
            assert_eq!(read(&store, "B"), other);
            server.await.unwrap();
        }
    }
}

#[tokio::test]
async fn membership_timeouts_at_each_boundary_preserve_only_verified_tokens() {
    for stage in 1..=7 {
        let (mut provider, seen, release, server) =
            gated(detailed_replies()[..stage].to_vec()).await;
        let (_, original, _) = setup(&mut provider);
        provider.client = provider
            .client
            .with_session_test_timeout(Duration::from_millis(300));
        let provider = Arc::new(provider.caller_scope(&original.caller().unwrap()).unwrap());
        let worker = provider.clone();
        let task =
            tokio::spawn(async move { worker.user_membership_client_info(None, None).await });
        tokio::time::timeout(Duration::from_secs(5), seen)
            .await
            .unwrap()
            .unwrap();
        let mut error = tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap()
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamTimeout);
        let expected = match stage {
            1 => None,
            2 | 3 => Some("p1"),
            4 | 5 => Some("p2"),
            6 | 7 => Some("p3"),
            _ => unreachable!(),
        };
        assert_eq!(
            provider
                .take_response_credential()
                .unwrap()
                .map(|c| MiguCredential::parse_caller(&c).unwrap().token().to_owned())
                .as_deref(),
            expected
        );
        assert_eq!(
            error
                .take_caller_credential_update()
                .map(|c| MiguCredential::parse_caller(&c).unwrap().token().to_owned())
                .as_deref(),
            expected
        );
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        drop(release);
    }
}

#[tokio::test]
async fn membership_transport_rejects_wrong_identity_failed_codes_mime_redirects_and_oversize() {
    for response in [reply(json!({"userId":"222","memberCards":[]}),"wrong-user"),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 17\r\nConnection: close\r\n\r\n{\"code\":\"290001\"}".into(),
        "HTTP/1.1 302 Found\r\nLocation: https://evil.invalid/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 262145\r\nConnection: close\r\n\r\n".into()] {
        let (mut provider,requests)=server(vec![profile("111","pacmtoken: p1\r\n"),response]).await;let (_,original,_)=setup(&mut provider);
        let provider=provider.caller_scope(&original.caller().unwrap()).unwrap();
        let error=provider.user_membership(None,None).await.unwrap_err();
        let update=provider.take_response_credential().unwrap();
        if error.code==ErrorCode::AuthenticationRequired {assert!(update.is_none());} else {assert_eq!(MiguCredential::parse_caller(&update.unwrap()).unwrap().token(),"p1");}
        assert_eq!(requests.await.unwrap().len(),2);
    }
}

#[tokio::test]
async fn detailed_membership_failure_preserves_verified_steps_and_rejects_foreign_identity() {
    for foreign_uid in [false, true] {
        let mut responses = detailed_replies();
        responses[5] = reply(
            json!({"userId":if foreign_uid {"222"} else {"111"}}),
            "unverified",
        );
        if foreign_uid {
            responses.truncate(6);
        }
        let (mut provider, requests) = server(responses).await;
        let (_, original, _) = setup(&mut provider);
        let provider = provider.caller_scope(&original.caller().unwrap()).unwrap();
        let mut failure = provider
            .user_membership_client_info(None, None)
            .await
            .unwrap_err();
        let update = provider.take_response_credential().unwrap();
        if foreign_uid {
            assert_eq!(failure.code, ErrorCode::AuthenticationRequired);
            assert!(update.is_none());
            assert!(failure.take_caller_credential_update().is_none());
        } else {
            assert_eq!(failure.code, ErrorCode::UpstreamError);
            assert_eq!(
                MiguCredential::parse_caller(&update.unwrap())
                    .unwrap()
                    .token(),
                "p4"
            );
            assert_eq!(
                MiguCredential::parse_caller(&failure.take_caller_credential_update().unwrap())
                    .unwrap()
                    .token(),
                "p4"
            );
        }
        assert_eq!(
            requests.await.unwrap().len(),
            if foreign_uid { 6 } else { 7 }
        );
    }
}
