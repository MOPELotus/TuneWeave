use super::session::tests::{Store, credential, paused, raw, read, reply, server};
use super::*;
use crate::{KugouLoginClient, KugouQrAuthorization, web::WebSession};
use serde_json::Value;
use std::collections::BTreeMap;

mod media;

fn web(uid: &str, token: &str) -> KugouCredential {
    KugouCredential::verified_web(WebSession::test_session(uid, token)).unwrap()
}
fn cookie(uid: &str, token: &str) -> String {
    format!(
        "KuGoo=KugooID={uid}&t={token}&a_id=1014&ct=1700000000&NickName=%u6D4B%u8BD5&Pic=https://imge.kugou.com/avatar.jpg; Domain=.kugou.com; Path=/; Secure; HttpOnly"
    )
}
fn web_reply(uid: &str, token: &str) -> String {
    response_with_cookies(
        json!({"status":1,"error_code":0,"data":null}),
        &[cookie(uid, token)],
    )
}
fn response_with_cookies(body: Value, cookies: &[String]) -> String {
    let headers = cookies
        .iter()
        .map(|v| format!("Set-Cookie: {v}\r\n"))
        .collect::<String>();
    raw(body).replacen("Content-Type:", &format!("{headers}Content-Type:"), 1)
}
fn params(request: &str) -> BTreeMap<String, String> {
    let (head, body) = request.split_once("\r\n\r\n").unwrap();
    assert!(head.starts_with("POST /v1/login_by_token_get?"));
    assert!(body.is_empty());
    let target = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
    let mut params: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
    let signature = params.remove("signature").unwrap();
    let borrowed = params
        .iter()
        .map(|(k, v)| (k.as_str(), v.clone()))
        .collect();
    assert_eq!(signature, crate::signing::web_signature(&borrowed, b""));
    assert_eq!(params["appid"], "1014");
    assert_eq!(params["clientver"], "1000");
    assert_eq!(params["dev"], "web");
    assert_eq!(params["plat"], "4");
    assert_eq!(params["mid"], params["uuid"]);
    assert_eq!(params["mid"].len(), 32);
    assert_eq!(
        params["clienttime"].parse::<u64>().unwrap(),
        params["clienttime_ms"].parse::<u64>().unwrap() / 1000
    );
    assert_eq!(params.len(), 14);
    assert_eq!(params["srcappid"], "2919");
    assert_eq!(params["dfid"], "-");
    assert_eq!(params["expire_day"], "1");
    assert_eq!(params["pk"].len() % 4, 0);
    assert_eq!(params["params"].len() % 32, 0);
    assert!(
        head.to_lowercase()
            .contains("origin: https://login-user.kugou.com")
    );
    assert!(!head.to_lowercase().contains("authorization:"));
    params
}

#[tokio::test]
async fn web_qr_uses_fresh_cookie_identity_for_all_three_ownership_modes() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let mut f = server(vec![
            reply(json!({"qrcode":"0123456789ABCDEF0123456789ABCDEF0123"})).into(),
            reply(json!({"status":4,"userid":"111","token":"qr-secret"})).into(),
            web_reply("111", "fresh-web").into(),
        ])
        .await;
        let store = Arc::new(Store::default());
        let unrelated = credential("999", "unrelated-native");
        store.put(&unrelated.stored("B").unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        let account = if mode == CredentialMode::Client {
            "default"
        } else {
            "A"
        };
        let start = f
            .provider
            .start_qr_login_with_mode(Some("web"), mode)
            .await
            .unwrap();
        let result = f
            .provider
            .poll_qr_login_with_mode(&start.provider_transaction_id, account, mode)
            .await
            .unwrap();
        assert_eq!(result.state, AuthState::Confirmed);
        let profile = result.profile.unwrap();
        assert_eq!(profile.user_id.as_deref(), Some("111"));
        assert_eq!(profile.nickname.as_deref(), Some("测试"));
        assert_eq!(profile.account, account);
        assert!(profile.authenticated);
        assert_eq!(result.credential.is_some(), mode.returns_to_caller());
        if let Some(caller) = result.credential {
            assert_eq!(caller.kind, "kugou_web_v1");
            let value = KugouCredential::parse_caller(&caller).unwrap();
            assert!(matches!(value, KugouCredential::Web(_)));
            if mode == CredentialMode::Both {
                assert_eq!(value, read(&store, account));
            }
            assert!(!format!("{caller:?}").contains("fresh-web"));
        }
        assert_eq!(
            store.values.lock().unwrap().contains_key(account),
            mode.persists_on_server()
        );
        assert_eq!(read(&store, "B"), unrelated);
        assert!(
            f.provider
                .poll_qr_login_with_mode(&start.provider_transaction_id, account, mode)
                .await
                .is_err()
        );
        let requests = f.requests.await.unwrap();
        assert_eq!(requests.len(), 3);
        let p = params(&requests[2]);
        assert_eq!(p["userid"], "111");
        let url = url::Url::parse(&format!(
            "http://localhost{}",
            requests[0]
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
        ))
        .unwrap();
        let start_params: BTreeMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(start_params["mid"], p["mid"]);
        assert!(!requests[2].contains("qr-secret"));
        assert!(!requests[2].to_lowercase().contains("cookie:"));
        assert!(!requests.iter().any(|r| r.contains("unrelated-native")));
    }
}

#[tokio::test]
async fn web_refresh_preserves_generation_and_both_selects_latest_cookie() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let mut f = server(vec![web_reply("111", "rotated-web").into()]).await;
        let store = Arc::new(Store::default());
        let old = web("111", "old-web");
        let KugouCredential::Web(old_web) = &old else {
            unreachable!()
        };
        let mut next = WebSession::test_session("111", "server-current");
        next.device = old_web.session.device.clone();
        let current = old.rotate_web(next).unwrap();
        let account = if mode == CredentialMode::Client {
            "default"
        } else {
            "A"
        };
        store.put(&current.stored(account).unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        let caller = old.caller().unwrap();
        let result = f
            .provider
            .refresh_session_with_ownership(
                account,
                mode.returns_to_caller().then_some(&caller),
                mode,
            )
            .await
            .unwrap();
        assert_eq!(result.credential.is_some(), mode.returns_to_caller());
        assert_eq!(result.profile.account, account);
        if let Some(caller) = result.credential {
            let value = KugouCredential::parse_caller(&caller).unwrap();
            assert!(old.same_login(&value));
            if mode == CredentialMode::Both {
                assert_eq!(value, read(&store, account));
            }
        }
        if mode == CredentialMode::Client {
            assert_eq!(read(&store, account), current);
        } else {
            assert_ne!(read(&store, account), current);
        }
        let requests = f.requests.await.unwrap();
        params(&requests[0]);
        assert!(requests[0].contains(if mode == CredentialMode::Client {
            "t=old-web"
        } else {
            "t=server-current"
        }));
        assert!(!requests[0].contains("rotated-web"));
    }
}

#[tokio::test]
async fn web_caller_scope_reads_own_identity_and_never_inherits_server_state() {
    let mut f = server(vec![web_reply("222", "new-caller").into()]).await;
    let store = Arc::new(Store::default());
    let native = credential("111", "server-secret");
    store.put(&native.stored("A").unwrap()).unwrap();
    f.provider.credential_store = Some(store.clone());
    let caller = web("222", "caller-only").caller().unwrap();
    let scoped = f.provider.caller_scope(&caller).unwrap();
    assert!(scoped.session_profile("A").await.is_err());
    let profile = scoped
        .read_user_profile("222", tuneweave_core::UserProfileBackend::Modern, None)
        .await
        .unwrap();
    assert_eq!(profile.user.id, "222");
    assert_eq!(profile.user.name, "测试");
    assert!(scoped.response_credential.lock().unwrap().is_some());
    assert_eq!(read(&store, "A"), native);
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].contains("server-secret"));
    assert!(scoped.require_public_source().is_err());
    let result = f
        .provider
        .logout_with_ownership("default", Some(&caller), CredentialMode::Client)
        .await
        .unwrap();
    assert!(result.caller_credential_discard_required);
    assert!(!result.removed);
}

#[tokio::test]
async fn web_sdk_rejects_wrong_cookie_identity_missing_cookie_and_transport_failures() {
    let success = json!({"status":1,"error_code":0,"data":null});
    let cases = vec![
        (raw(success.clone()), ErrorCode::UpstreamError),
        (web_reply("222", "wrong-user"), ErrorCode::Conflict),
        (
            response_with_cookies(
                success.clone(),
                &[cookie("111", "one"), cookie("111", "two")],
            ),
            ErrorCode::UpstreamError,
        ),
        (
            response_with_cookies(
                success.clone(),
                &[format!("{}; Max-Age=0", cookie("111", "removed"))],
            ),
            ErrorCode::AuthenticationRequired,
        ),
        (
            response_with_cookies(
                json!({"status":0,"error_code":20017,"data":"private-error"}),
                &[cookie("111", "ignored")],
            ),
            ErrorCode::UpstreamError,
        ),
        (
            web_reply("111", "never-issued").replacen(
                "Content-Type:",
                "SSA-CODE: 1\r\nContent-Type:",
                1,
            ),
            ErrorCode::PermissionDenied,
        ),
        (
            web_reply("111", "never-issued")
                .replacen("200 OK", "302 Found", 1)
                .replacen(
                    "Content-Type:",
                    "Location: https://evil.com/\r\nContent-Type:",
                    1,
                ),
            ErrorCode::UpstreamError,
        ),
        (
            web_reply("111", "never-issued").replacen("application/json", "text/html", 1),
            ErrorCode::UpstreamError,
        ),
        (
            raw(json!({"status":0,"error_code":0})).replacen("200 OK", "429 Too Many Requests", 1),
            ErrorCode::RateLimited,
        ),
    ];
    for (frame, code) in cases {
        let f = server(vec![frame.into()]).await;
        let session = WebSession::test_session("111", "qr-secret");
        let authorization = KugouQrAuthorization::test_authorization(
            KugouLoginClient::Web,
            session.device,
            "111".into(),
            "qr-secret".into(),
        );
        let mut error = f
            .provider
            .client
            .complete_qr_login(authorization)
            .await
            .unwrap_err();
        assert_eq!(error.code, code);
        assert!(error.take_caller_credential_update().is_none());
        let debug = format!("{error:?}");
        for secret in ["qr-secret", "private-error", "never-issued"] {
            assert!(!debug.contains(secret));
        }
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn web_late_refresh_success_and_error_cannot_revive_replaced_or_logged_out_accounts() {
    for caller_owned in [false, true] {
        for rejected in [false, true] {
            for logout in [false, true] {
                let body = if rejected {
                    raw(json!({"status":0,"error_code":20017,"data":null}))
                } else {
                    web_reply("111", "late-web")
                };
                let (frame, resume) = paused(body);
                let mut f = server(vec![frame]).await;
                let store = Arc::new(Store::default());
                let old = web("111", "old-web");
                store.put(&old.stored("A").unwrap()).unwrap();
                f.provider.credential_store = Some(store.clone());
                let selected = if caller_owned {
                    f.provider.caller_scope(&old.caller().unwrap()).unwrap()
                } else {
                    f.provider.clone()
                };
                let worker = selected.clone();
                let task = tokio::spawn(async move {
                    worker
                        .session_profile(if caller_owned { "default" } else { "A" })
                        .await
                });
                f.seen.recv().await.unwrap();
                let replacement = credential("222", "new-native");
                if caller_owned {
                    *selected.caller_credential.as_ref().unwrap().lock().unwrap() =
                        (!logout).then_some(replacement.clone());
                } else if logout {
                    store.values.lock().unwrap().remove("A");
                } else {
                    store.put(&replacement.stored("A").unwrap()).unwrap();
                }
                resume.send(()).unwrap();
                let error = task.await.unwrap().unwrap_err();
                assert_eq!(error.code, ErrorCode::Conflict);
                assert!(selected.response_credential.lock().unwrap().is_none());
                if caller_owned {
                    assert_eq!(
                        *selected.caller_credential.as_ref().unwrap().lock().unwrap(),
                        (!logout).then_some(replacement)
                    );
                } else if logout {
                    assert!(!store.values.lock().unwrap().contains_key("A"));
                } else {
                    assert_eq!(read(&store, "A"), replacement);
                }
                f.requests.await.unwrap();
            }
        }
    }
}

#[tokio::test]
async fn web_qr_cancellation_discards_the_completed_cookie_exchange() {
    let (frame, resume) = paused(web_reply("111", "late-web"));
    let mut f = server(vec![
        reply(json!({"qrcode":"0123456789ABCDEF0123456789ABCDEF0123"})).into(),
        reply(json!({"status":4,"userid":"111","token":"qr-secret"})).into(),
        frame,
    ])
    .await;
    let store = Arc::new(Store::default());
    f.provider.credential_store = Some(store.clone());
    let start = f
        .provider
        .start_qr_login_with_mode(Some("web"), CredentialMode::Both)
        .await
        .unwrap();
    let provider = f.provider.clone();
    let id = start.provider_transaction_id.clone();
    let task = tokio::spawn(async move {
        provider
            .poll_qr_login_with_mode(&id, "A", CredentialMode::Both)
            .await
    });
    for _ in 0..3 {
        f.seen.recv().await.unwrap();
    }
    f.provider
        .cancel_qr_login(&start.provider_transaction_id)
        .unwrap();
    resume.send(()).unwrap();
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    assert!(store.values.lock().unwrap().is_empty());
    assert_eq!(f.requests.await.unwrap().len(), 3);
}

#[test]
fn web_credential_kind_fields_and_generation_are_strictly_separate_from_native() {
    let original = web("111", "web-secret");
    let native = credential("111", "native-secret");
    assert!(!original.same_login(&native));
    assert!(!original.same_login(&web("111", "web-secret")));
    let caller = original.caller().unwrap();
    assert_eq!(KugouCredential::parse_caller(&caller).unwrap(), original);
    for (kind, secret) in [
        ("kugou_native_v1", caller.secret().to_owned()),
        ("kugou_web_v1", native.caller().unwrap().into_secret()),
        (
            "kugou_web_v1",
            caller
                .secret()
                .replacen("\"version\":1", "\"version\":1,\"version\":1", 1),
        ),
        (
            "kugou_web_v1",
            caller.secret().replacen(
                "\"user_id\":\"111\"",
                "\"user_id\":\"111\",\"user_id\":\"111\"",
                1,
            ),
        ),
        (
            "kugou_web_v1",
            caller.secret().replace("KugooID=111", "KugooID=222"),
        ),
        (
            "kugou_web_v1",
            caller
                .secret()
                .replacen("\"cookie\":{", "\"cookie\":{\"unknown\":1,", 1),
        ),
    ] {
        let bad = ProviderCredential::new(Platform::Kugou, kind, secret, None).unwrap();
        assert!(KugouCredential::parse_caller(&bad).is_err());
    }
    assert!(
        original
            .rotate_web(WebSession::test_session("111", "another-device"))
            .is_err()
    );
    assert!(!format!("{original:?}").contains("web-secret"));
}

#[tokio::test]
async fn expired_web_cookies_invalidate_only_the_selected_owner_without_network() {
    for caller_owned in [false, true] {
        let mut f = server(vec![]).await;
        let store = Arc::new(Store::default());
        let expired = web("111", "expired-web").caller().unwrap();
        let secret = expired
            .secret()
            .replace("\"expires\":null", "\"expires\":0");
        assert_ne!(secret, expired.secret());
        let expired =
            ProviderCredential::new(Platform::Kugou, "kugou_web_v1", secret, None).unwrap();
        let expired = KugouCredential::parse_caller(&expired).unwrap();
        store.put(&expired.stored("A").unwrap()).unwrap();
        let other = credential("222", "other-native");
        store.put(&other.stored("B").unwrap()).unwrap();
        f.provider.credential_store = Some(store.clone());
        let provider = if caller_owned {
            f.provider.caller_scope(&expired.caller().unwrap()).unwrap()
        } else {
            f.provider.clone()
        };
        let profile = provider
            .session_profile(if caller_owned { "default" } else { "A" })
            .await
            .unwrap();
        assert!(!profile.authenticated);
        assert!(provider.response_credential.lock().unwrap().is_none());
        assert_eq!(store.values.lock().unwrap().contains_key("A"), caller_owned);
        assert_eq!(read(&store, "B"), other);
        if caller_owned {
            assert!(
                provider
                    .caller_credential
                    .unwrap()
                    .lock()
                    .unwrap()
                    .is_none()
            );
        }
        assert!(f.requests.await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn web_sdk_refresh_and_cross_kind_rejections_preserve_the_original_login() {
    let f = server(vec![web_reply("111", "sdk-updated").into()]).await;
    let source = web("111", "sdk-original");
    let result = f
        .provider
        .client
        .refresh_web_login(&source.caller().unwrap())
        .await
        .unwrap();
    let next = KugouCredential::parse_caller(result.credential.as_ref().unwrap()).unwrap();
    assert!(source.same_login(&next));
    assert_eq!(result.profile.user_id.as_deref(), Some("111"));
    assert!(
        f.provider
            .client
            .refresh_native_login(&source.caller().unwrap())
            .await
            .is_err()
    );
    assert!(
        f.provider
            .client
            .refresh_web_login(&credential("111", "native").caller().unwrap())
            .await
            .is_err()
    );
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 1);
    params(&requests[0]);
    assert!(requests[0].contains("t=sdk-original"));
}
