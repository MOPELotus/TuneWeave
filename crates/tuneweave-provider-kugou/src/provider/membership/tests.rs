use super::*;
use crate::provider::session::tests::{
    Frame, Store, credential, exchange, paused, profile, raw, read, server,
};
use crate::{KugouLoginClient, web::WebSession};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tuneweave_core::StoredAccountCredential;

fn source(client: KugouLoginClient, uid: &str, token: &str) -> KugouCredential {
    if client == KugouLoginClient::Web {
        KugouCredential::verified_web(WebSession::test_session(uid, token)).unwrap()
    } else {
        let KugouCredential::Native(mut value) = credential(uid, token) else {
            unreachable!()
        };
        value.session.client = client;
        KugouCredential::Native(value)
    }
}
fn cookie(uid: &str, token: &str) -> String {
    format!(
        "KuGoo=KugooID={uid}&t={token}&a_id=1014&ct=1700000000&NickName=Listener; Domain=.kugou.com; Path=/; Secure; HttpOnly"
    )
}
fn with_cookie(body: serde_json::Value, uid: &str, token: &str) -> String {
    raw(body).replacen(
        "Content-Type:",
        &format!("Set-Cookie: {}\r\nContent-Type:", cookie(uid, token)),
        1,
    )
}
fn web_exchange(uid: &str, token: &str) -> String {
    with_cookie(json!({"status":1,"error_code":0,"data":null}), uid, token)
}
fn native_member(client: KugouLoginClient, uid: &str) -> String {
    if client == KugouLoginClient::Standard {
        raw(
            json!({"status":1,"errcode":0,"data":{"userid":uid,"vip_type":6,"svip_level":3,"vip_end_time":"2027-01-01 00:00:00"}}),
        )
    } else {
        raw(
            json!({"status":1,"error_code":0,"data":{"userid":uid,"vip_type":0,"busi_vip":[{"userid":uid,"busi_type":"concept","product_type":"wvip","is_vip":1,"vip_end_time":"2028-01-01 00:00:00"}]}}),
        )
    }
}
fn frames(client: KugouLoginClient) -> Vec<String> {
    frames_for(
        client,
        "111",
        "first-verified-token",
        "final-verified-token",
    )
}
fn frames_for(client: KugouLoginClient, uid: &str, first: &str, last: &str) -> Vec<String> {
    if client == KugouLoginClient::Web {
        vec![
            web_exchange(uid, first),
            raw(json!({"role":11,"vipEndTime":"2027-01-01 00:00:00"})),
            web_exchange(uid, last),
        ]
    } else {
        vec![
            exchange(uid, first),
            profile(uid),
            native_member(client, uid),
        ]
    }
}
fn clients() -> [KugouLoginClient; 3] {
    [
        KugouLoginClient::Standard,
        KugouLoginClient::Concept,
        KugouLoginClient::Web,
    ]
}

#[tokio::test]
async fn membership_all_clients_use_only_selected_default_named_or_caller_identity() {
    for client in clients() {
        for owner in ["default", "A", "caller"] {
            for detailed in [false, true] {
                let mut fixture =
                    server(frames(client).into_iter().map(Frame::from).collect()).await;
                let store = Arc::new(Store::default());
                let original = source(client, "111", "original-private-token");
                let unrelated = credential("999", "unrelated-private-token");
                store.put(&unrelated.stored("B").unwrap()).unwrap();
                fixture.provider.credential_store = Some(store.clone());
                let provider = if owner == "caller" {
                    fixture
                        .provider
                        .caller_scope(&original.caller().unwrap())
                        .unwrap()
                } else {
                    store.put(&original.stored(owner).unwrap()).unwrap();
                    fixture.provider.clone()
                };
                let alias = if owner == "caller" { None } else { Some(owner) };
                let member = if detailed {
                    provider
                        .user_membership_client_info(Some("111"), alias)
                        .await
                } else {
                    provider.user_membership(None, alias).await
                }
                .unwrap();
                assert_eq!(member.user_ref.unwrap().id(), "111");
                assert_eq!(member.active, Some(client != KugouLoginClient::Concept));
                assert_eq!(member.extensions["summary_scope"], "main_membership");
                assert_eq!(read(&store, "B"), unrelated);
                let update = provider.take_response_credential().unwrap();
                assert_eq!(update.is_some(), owner == "caller");
                if owner == "caller" {
                    assert_eq!(store.values.lock().unwrap().len(), 1);
                } else {
                    assert!(read(&store, owner).same_login(&original));
                    assert_ne!(read(&store, owner), original);
                }
                let requests = fixture.requests.await.unwrap();
                assert_eq!(requests.len(), 3);
                assert!(
                    !requests
                        .iter()
                        .any(|s| s.contains("unrelated-private-token"))
                );
                let member_request = &requests[if client == KugouLoginClient::Web {
                    1
                } else {
                    2
                }];
                assert!(member_request.starts_with("GET "));
                assert!(member_request.ends_with("\r\n\r\n"));
                if client == KugouLoginClient::Web {
                    assert!(member_request.starts_with("GET /recharge/roleinfo?"));
                    assert!(member_request.contains("t=first-verified-token"));
                    assert!(!member_request.contains("clienttoken="));
                    assert!(!member_request.contains("signature="));
                } else {
                    assert!(!member_request.to_lowercase().contains("cookie:"));
                    let path = member_request.split_whitespace().nth(1).unwrap();
                    let url = url::Url::parse(&format!("http://localhost{path}")).unwrap();
                    let q = url
                        .query_pairs()
                        .into_owned()
                        .collect::<std::collections::BTreeMap<_, _>>();
                    if client == KugouLoginClient::Standard {
                        assert_eq!(q["appid"], "1005");
                        assert_eq!(q["clientappid"], "1005");
                        assert_eq!(q["kugouid"], "111");
                        assert_eq!(q["clienttoken"], "first-verified-token");
                        assert!(!q.contains_key("busi_type"));
                        assert!(member_request.to_lowercase().contains("kg-tid: 524"));
                    } else {
                        assert_eq!(q["busi_type"], "concept");
                        assert_eq!(q["opt_product_types"], "dvip,qvip,wvip");
                        assert_eq!(q["userid"], "111");
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn membership_wrong_requested_user_or_missing_source_fails_before_network() {
    let provider = KugouProvider::new(KugouConfig::default()).unwrap();
    for client in clients() {
        let scoped = provider
            .caller_scope(
                &source(client, "111", "fixture-secret-token")
                    .caller()
                    .unwrap(),
            )
            .unwrap();
        for (id, code) in [
            ("222", ErrorCode::PermissionDenied),
            ("01", ErrorCode::InvalidRequest),
            ("", ErrorCode::InvalidRequest),
        ] {
            assert_eq!(
                scoped
                    .user_membership(Some(id), None)
                    .await
                    .unwrap_err()
                    .code,
                code
            );
        }
        assert_eq!(
            scoped
                .user_membership(None, Some("named"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        provider.user_membership(None, None).await.unwrap_err().code,
        ErrorCode::AuthenticationRequired
    );
}

#[tokio::test]
async fn membership_each_success_and_error_boundary_rejects_logout_relogin_and_account_replacement()
{
    for client in clients() {
        for boundary in 0..3 {
            for failure in [false, true] {
                for mutation in 0..3 {
                    let mut replies = frames(client);
                    replies.truncate(boundary + 1);
                    let body = if failure {
                        "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".into()
                    } else {
                        replies.pop().unwrap()
                    };
                    if failure {
                        replies.pop();
                    }
                    let (gate, release) = paused(body);
                    let mut fs = replies.into_iter().map(Frame::from).collect::<Vec<_>>();
                    fs.push(gate);
                    let mut fixture = server(fs).await;
                    let store = Arc::new(Store::default());
                    let original = source(client, "111", "original-private-token");
                    store.put(&original.stored("A").unwrap()).unwrap();
                    fixture.provider.credential_store = Some(store.clone());
                    let p = fixture.provider.clone();
                    let task =
                        tokio::spawn(async move { p.user_membership(None, Some("A")).await });
                    for _ in 0..=boundary {
                        fixture.seen.recv().await.unwrap();
                    }
                    let replacement = match mutation {
                        0 => None,
                        1 => Some(source(client, "111", "first-verified-token")),
                        _ => Some(source(client, "222", "replacement-private-token")),
                    };
                    if let Some(v) = &replacement {
                        store.put(&v.stored("A").unwrap()).unwrap();
                    } else {
                        store.remove(Platform::Kugou, "A").unwrap();
                    }
                    release.send(()).unwrap();
                    let mut error = task.await.unwrap().unwrap_err();
                    assert_eq!(error.code, ErrorCode::Conflict);
                    assert!(error.take_caller_credential_update().is_none());
                    assert!(
                        fixture
                            .provider
                            .take_response_credential()
                            .unwrap()
                            .is_none()
                    );
                    if let Some(v) = replacement {
                        assert_eq!(read(&store, "A"), v);
                    } else {
                        assert!(!store.values.lock().unwrap().contains_key("A"));
                    }
                    assert_eq!(fixture.requests.await.unwrap().len(), boundary + 1);
                }
            }
        }
    }
}

#[tokio::test]
async fn membership_cancellation_and_total_timeout_clear_pending_updates_at_each_boundary() {
    for client in clients() {
        for boundary in 0..3 {
            for cancel in [true, false] {
                let mut replies = frames(client);
                replies.truncate(boundary + 1);
                let (gate, release) = paused(replies.pop().unwrap());
                let mut fs = replies.into_iter().map(Frame::from).collect::<Vec<_>>();
                fs.push(gate);
                let mut fixture = server(fs).await;
                let scoped = fixture
                    .provider
                    .caller_scope(
                        &source(client, "111", "original-private-token")
                            .caller()
                            .unwrap(),
                    )
                    .unwrap();
                let p = scoped.clone();
                let task = tokio::spawn(async move {
                    p.read_membership(None, None, Duration::from_secs(2)).await
                });
                for _ in 0..=boundary {
                    fixture.seen.recv().await.unwrap();
                }
                if cancel {
                    task.abort();
                    assert!(task.await.unwrap_err().is_cancelled());
                } else {
                    tokio::time::pause();
                    tokio::time::advance(Duration::from_secs(2)).await;
                    let mut error = task.await.unwrap().unwrap_err();
                    assert_eq!(error.code, ErrorCode::UpstreamTimeout);
                    assert!(error.take_caller_credential_update().is_none());
                    tokio::time::resume();
                }
                assert!(scoped.take_response_credential().unwrap().is_none());
                let _ = release.send(());
                fixture.requests.await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn membership_web_cookie_candidate_requires_final_independent_uid_and_commits_only_verified_cookie()
 {
    for mismatch in [false, true] {
        let mut fixture = server(vec![
            web_exchange("111", "first-verified-token").into(),
            with_cookie(json!({"role":11}), "111", "candidate-private-token").into(),
            web_exchange(if mismatch { "222" } else { "111" }, "final-verified-token").into(),
        ])
        .await;
        let store = Arc::new(Store::default());
        let original = source(KugouLoginClient::Web, "111", "original-private-token");
        store.put(&original.stored("A").unwrap()).unwrap();
        fixture.provider.credential_store = Some(store.clone());
        let result = fixture.provider.user_membership(None, Some("A")).await;
        if mismatch {
            assert!(result.is_err());
            assert!(
                !serde_json::to_string(&store.values.lock().unwrap().get("A"))
                    .unwrap()
                    .contains("candidate-private-token")
            );
        } else {
            assert!(result.is_ok());
            let KugouCredential::Web(v) = read(&store, "A") else {
                unreachable!()
            };
            assert_eq!(v.session.media_token().unwrap(), "final-verified-token");
        }
        let requests = fixture.requests.await.unwrap();
        assert!(requests[2].contains("t=candidate-private-token"));
    }
}

#[tokio::test]
async fn membership_wrong_member_identity_and_auth_denial_invalidate_only_selected_account() {
    for client in clients() {
        let mut replies = frames(client);
        let index = if client == KugouLoginClient::Web {
            1
        } else {
            2
        };
        replies.truncate(index + 1);
        replies[index] = if client == KugouLoginClient::Web {
            raw(json!({"errno":105,"error_code":20017}))
        } else {
            native_member(client, "999")
        };
        let mut fixture = server(replies.into_iter().map(Frame::from).collect()).await;
        let store = Arc::new(Store::default());
        store
            .put(
                &source(client, "111", "original-private-token")
                    .stored("A")
                    .unwrap(),
            )
            .unwrap();
        let other = credential("999", "unrelated-private-token");
        store.put(&other.stored("B").unwrap()).unwrap();
        fixture.provider.credential_store = Some(store.clone());
        assert_eq!(
            fixture
                .provider
                .user_membership(None, Some("A"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
        assert!(!store.values.lock().unwrap().contains_key("A"));
        assert_eq!(read(&store, "B"), other);
        fixture.requests.await.unwrap();
    }
}

#[tokio::test]
async fn membership_store_failure_never_releases_response_or_credential() {
    for boundary in 0..3 {
        let mut replies = frames(KugouLoginClient::Standard);
        replies.truncate(boundary + 1);
        let (gate, release) = paused(replies.pop().unwrap());
        let mut fs = replies.into_iter().map(Frame::from).collect::<Vec<_>>();
        fs.push(gate);
        let mut fixture = server(fs).await;
        let store = Arc::new(Store::default());
        store
            .put(
                &credential("111", "original-private-token")
                    .stored("A")
                    .unwrap(),
            )
            .unwrap();
        fixture.provider.credential_store = Some(store.clone());
        let p = fixture.provider.clone();
        let task = tokio::spawn(async move { p.user_membership(None, Some("A")).await });
        for _ in 0..=boundary {
            fixture.seen.recv().await.unwrap();
        }
        store.fail.store(true, Ordering::SeqCst);
        release.send(()).unwrap();
        let mut error = task.await.unwrap().unwrap_err();
        assert_eq!(error.code, ErrorCode::InternalError);
        assert!(error.take_caller_credential_update().is_none());
        assert!(
            fixture
                .provider
                .take_response_credential()
                .unwrap()
                .is_none()
        );
        fixture.requests.await.unwrap();
    }
}

#[tokio::test]
async fn membership_rejects_private_material_from_original_and_rotated_credentials() {
    for secret in [
        "original-private-token",
        "first-verified-token",
        "final-verified-token",
    ] {
        let fixture = server(vec![
            web_exchange("111", "first-verified-token").into(),
            raw(json!({"role":11,"vipEndTime":secret})).into(),
            web_exchange("111", "final-verified-token").into(),
        ])
        .await;
        let scoped = fixture
            .provider
            .caller_scope(
                &source(KugouLoginClient::Web, "111", "original-private-token")
                    .caller()
                    .unwrap(),
            )
            .unwrap();
        assert_eq!(
            scoped.user_membership(None, None).await.unwrap_err().code,
            ErrorCode::UpstreamError
        );
        assert!(scoped.take_response_credential().unwrap().is_none());
        fixture.requests.await.unwrap();
    }
}

#[tokio::test]
async fn membership_transport_limits_and_unknown_failures_do_not_retry_or_change_source() {
    let cases = [
        (
            raw(json!({"status":1,"errcode":0,"data":{}})).replace("application/json", "text/html"),
            ErrorCode::UpstreamError,
        ),
        (
            "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 9999\r\nContent-Length: 0\r\n\r\n"
                .into(),
            ErrorCode::RateLimited,
        ),
        (
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1048577\r\n\r\n"
                .into(),
            ErrorCode::UpstreamError,
        ),
        (
            raw(json!({"status":0,"errcode":99999,"data":{}})),
            ErrorCode::UpstreamError,
        ),
    ];
    for (body, code) in cases {
        let fixture = server(vec![
            exchange("111", "first-verified-token").into(),
            profile("111").into(),
            body.into(),
        ])
        .await;
        let scoped = fixture
            .provider
            .caller_scope(
                &credential("111", "original-private-token")
                    .caller()
                    .unwrap(),
            )
            .unwrap();
        let mut error = scoped.user_membership(None, None).await.unwrap_err();
        assert_eq!(error.code, code);
        if code == ErrorCode::RateLimited {
            assert_eq!(error.details["retry_after_secs"], 300);
        }
        // The prior exchange was independently verified, so its typed error update
        // remains usable without leaking it into the ordinary membership JSON.
        assert!(error.take_caller_credential_update().is_some());
        assert!(scoped.take_response_credential().unwrap().is_none());
        assert_eq!(fixture.requests.await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn membership_reverse_account_completion_keeps_each_identity_and_update_separate() {
    for client in clients() {
        for caller in [false, true] {
            let mut replies = frames(client);
            let (gate, release) = paused(replies.pop().unwrap());
            let mut a_frames = replies.into_iter().map(Frame::from).collect::<Vec<_>>();
            a_frames.push(gate);
            let mut a = server(a_frames).await;
            let mut b = server(
                frames_for(
                    client,
                    "222",
                    "first-verified-token-b",
                    "final-verified-token-b",
                )
                .into_iter()
                .map(Frame::from)
                .collect(),
            )
            .await;
            let store = Arc::new(Store::default());
            let original_a = source(client, "111", "original-private-token-a");
            let original_b = source(client, "222", "original-private-token-b");
            store.put(&original_a.stored("A").unwrap()).unwrap();
            store.put(&original_b.stored("B").unwrap()).unwrap();
            a.provider.credential_store = Some(store.clone());
            b.provider.credential_store = Some(store.clone());
            let provider_a = if caller {
                a.provider
                    .caller_scope(&original_a.caller().unwrap())
                    .unwrap()
            } else {
                a.provider.clone()
            };
            let provider_b = if caller {
                b.provider
                    .caller_scope(&original_b.caller().unwrap())
                    .unwrap()
            } else {
                b.provider.clone()
            };
            let p = provider_a.clone();
            let task_a = tokio::spawn(async move {
                p.user_membership(None, if caller { None } else { Some("A") })
                    .await
            });
            for _ in 0..3 {
                a.seen.recv().await.unwrap();
            }
            let member_b = provider_b
                .user_membership(None, if caller { None } else { Some("B") })
                .await
                .unwrap();
            assert_eq!(member_b.user_ref.unwrap().id(), "222");
            assert!(!task_a.is_finished());
            release.send(()).unwrap();
            assert_eq!(task_a.await.unwrap().unwrap().user_ref.unwrap().id(), "111");
            for (p, alias, original) in [
                (&provider_a, "A", &original_a),
                (&provider_b, "B", &original_b),
            ] {
                let update = p.take_response_credential().unwrap();
                if caller {
                    let accepted = KugouCredential::parse_caller(&update.unwrap()).unwrap();
                    assert_eq!(accepted.user_id(), original.user_id());
                    assert!(accepted.same_login(original));
                    assert_eq!(read(&store, alias), *original);
                } else {
                    assert!(update.is_none());
                    assert!(read(&store, alias).same_login(original));
                    assert_ne!(read(&store, alias), *original);
                }
            }
            let requests_a = a.requests.await.unwrap();
            let requests_b = b.requests.await.unwrap();
            assert!(!requests_a.iter().any(|s| s.contains("token-b")));
            assert!(!requests_b.iter().any(|s| s.contains("token-a")));
        }
    }
}

#[tokio::test]
async fn membership_web_rejects_untrusted_cookie_candidates_without_verifying_or_exporting_them() {
    let valid = cookie("111", "candidate-private-token");
    let cases = [
        (
            valid.replace("Domain=.kugou.com; ", ""),
            ErrorCode::CapabilityNotSupported,
        ),
        (
            valid.replace(".kugou.com", ".example.com"),
            ErrorCode::UpstreamError,
        ),
        (
            valid.replace("Path=/;", "Path=/v1;"),
            ErrorCode::CapabilityNotSupported,
        ),
        (
            format!("{valid}; Max-Age=0"),
            ErrorCode::AuthenticationRequired,
        ),
        (
            format!("{valid}; Domain=.kugou.com"),
            ErrorCode::UpstreamError,
        ),
        (
            format!("{valid}\r\nSet-Cookie: {valid}"),
            ErrorCode::UpstreamError,
        ),
        (
            cookie("222", "candidate-private-token"),
            ErrorCode::AuthenticationRequired,
        ),
    ];
    for (cookie_header, code) in cases {
        let body = raw(json!({"role":11})).replacen(
            "Content-Type:",
            &format!("Set-Cookie: {cookie_header}\r\nContent-Type:"),
            1,
        );
        let fixture = server(vec![
            web_exchange("111", "first-verified-token").into(),
            body.into(),
        ])
        .await;
        let scoped = fixture
            .provider
            .caller_scope(
                &source(KugouLoginClient::Web, "111", "original-private-token")
                    .caller()
                    .unwrap(),
            )
            .unwrap();
        let mut error = scoped.user_membership(None, None).await.unwrap_err();
        assert_eq!(error.code, code);
        let update = error.take_caller_credential_update();
        if code == ErrorCode::AuthenticationRequired {
            assert!(update.is_none());
        } else {
            let KugouCredential::Web(accepted) =
                KugouCredential::parse_caller(&update.unwrap()).unwrap()
            else {
                unreachable!()
            };
            assert_eq!(
                accepted.session.media_token().unwrap(),
                "first-verified-token"
            );
        }
        assert!(scoped.take_response_credential().unwrap().is_none());
        assert_eq!(fixture.requests.await.unwrap().len(), 2);
    }
}

#[derive(Default)]
struct UnreadableStore;
impl AccountCredentialStore for UnreadableStore {
    fn load_platform(&self, _: Platform) -> Result<Vec<StoredAccountCredential>> {
        Err(session::state_error())
    }
    fn put(&self, _: &StoredAccountCredential) -> Result<()> {
        panic!("unexpected membership store write")
    }
    fn remove(&self, _: Platform, _: &str) -> Result<bool> {
        panic!("unexpected membership store removal")
    }
}

#[tokio::test]
async fn membership_store_read_failure_stops_server_but_never_affects_caller_scope() {
    for client in clients() {
        let mut fixture = server(frames(client).into_iter().map(Frame::from).collect()).await;
        fixture.provider.credential_store = Some(Arc::new(UnreadableStore));
        let mut error = fixture
            .provider
            .user_membership(None, Some("A"))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::InternalError);
        assert!(error.take_caller_credential_update().is_none());
        assert!(fixture.seen.try_recv().is_err());
        let scoped = fixture
            .provider
            .caller_scope(
                &source(client, "111", "original-private-token")
                    .caller()
                    .unwrap(),
            )
            .unwrap();
        scoped.user_membership(None, None).await.unwrap();
        assert!(scoped.take_response_credential().unwrap().is_some());
        assert_eq!(fixture.requests.await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn membership_native_rejects_raw_and_encoded_token_vip_token_and_t1_reflections() {
    for client in [KugouLoginClient::Standard, KugouLoginClient::Concept] {
        for secret in ["original-token+private", "vip-token+private", "t1+private"] {
            for encoded in [false, true] {
                let KugouCredential::Native(mut original) =
                    source(client, "111", "original-token+private")
                else {
                    unreachable!()
                };
                original.session.vip_token = Some("vip-token+private".into());
                original.session.t1 = Some("t1+private".into());
                let reflected: String = if encoded {
                    url::form_urlencoded::byte_serialize(secret.as_bytes()).collect()
                } else {
                    secret.into()
                };
                let member = if client == KugouLoginClient::Standard {
                    raw(
                        json!({"status":1,"errcode":0,"data":{"userid":"111","vip_type":1,"vip_end_time":reflected}}),
                    )
                } else {
                    raw(
                        json!({"status":1,"error_code":0,"data":{"userid":"111","vip_type":1,"vip_end_time":reflected}}),
                    )
                };
                let fixture = server(vec![
                    exchange("111", "first-verified-token").into(),
                    profile("111").into(),
                    member.into(),
                ])
                .await;
                let scoped = fixture
                    .provider
                    .caller_scope(&KugouCredential::Native(original).caller().unwrap())
                    .unwrap();
                let error = scoped.user_membership(None, None).await.unwrap_err();
                assert_eq!(error.code, ErrorCode::UpstreamError);
                assert!(!format!("{error:?}").contains(secret));
                assert!(scoped.take_response_credential().unwrap().is_none());
                assert_eq!(fixture.requests.await.unwrap().len(), 3);
            }
        }
    }
}
