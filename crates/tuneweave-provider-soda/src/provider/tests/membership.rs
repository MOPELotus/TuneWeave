use super::*;
use crate::client::membership::tests::{body, fixture};

fn responses(member: serde_json::Value, user: &str) -> Vec<String> {
    vec![
        account_reply(user, Some("sessionid_ss=profile-current")),
        crate::test_http::json(&member.to_string(), Some("sessionid_ss=member-candidate")),
        account_reply(user, Some("sessionid_ss=final-current")),
    ]
}
fn selected(f: &SessionFixture, owner: &str, source: &SodaCredential) -> SodaProvider {
    if owner == "caller" {
        f.provider
            .caller_credential_scope(&caller_from(source))
            .unwrap()
    } else {
        f.provider.clone()
    }
}
fn account(owner: &str) -> Option<&str> {
    (owner != "caller").then_some(owner)
}

#[tokio::test]
async fn commerce_membership_three_sources_and_both_methods_use_verified_commerce_fields() {
    for owner in ["default", "personal", "caller"] {
        for client_info in [false, true] {
            for member in [
                fixture(),
                body(json!({"is_membership":false,"expire_time":0})),
                body(json!({"membership_type":"free_vip"})),
            ] {
                let mut f = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                for alias in ["default", "personal", "other"] {
                    f.put(alias, &source);
                }
                let expected = member["membership"]["is_membership"].as_bool();
                let (origin, server) = crate::test_http::serve(responses(member, "123456")).await;
                f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
                let p = selected(&f, owner, &source);
                let value = if client_info {
                    p.user_membership_client_info(Some("123456"), account(owner))
                        .await
                } else {
                    p.user_membership(None, account(owner)).await
                }
                .unwrap();
                assert_eq!(value.active, expected);
                assert_eq!(
                    value.extensions["backend"],
                    "official_pc_commerce_membership_v2"
                );
                assert_eq!(value.extensions["source_user_id"], "123456");
                let output = serde_json::to_string(&value).unwrap();
                for secret in [
                    "session-secret",
                    "profile-current",
                    "member-candidate",
                    "final-current",
                ] {
                    assert!(!output.contains(secret));
                }
                if owner == "caller" {
                    let update = p.take_response_credential().unwrap().unwrap();
                    assert_eq!(
                        parse_soda_caller_credential(&update)
                            .unwrap()
                            .cookie_header()
                            .unwrap(),
                        "sessionid_ss=final-current"
                    );
                } else {
                    assert!(f.stored(owner).unwrap().secret().contains("final-current"));
                    assert!(p.take_response_credential().unwrap().is_none());
                }
                for alias in ["default", "personal", "other"]
                    .into_iter()
                    .filter(|a| *a != owner)
                {
                    assert_eq!(
                        f.stored(alias).unwrap().secret(),
                        source.serialize().unwrap()
                    );
                }
                let req = server.await.unwrap();
                assert_eq!(req.len(), 3);
                for (r, cookie) in
                    req.iter()
                        .zip(["session-secret", "profile-current", "member-candidate"])
                {
                    assert!(r.contains(&format!("cookie: sessionid_ss={cookie}\r\n")));
                }
            }
        }
    }
}

#[tokio::test]
async fn commerce_membership_invalid_selection_rejects_before_any_io() {
    let f = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    f.put("personal", &source);
    for (id, alias, code) in [
        (Some("0123"), Some("personal"), ErrorCode::InvalidRequest),
        (
            Some("654321"),
            Some("personal"),
            ErrorCode::PermissionDenied,
        ),
        (None, Some("missing"), ErrorCode::AuthenticationRequired),
        (None, None, ErrorCode::AuthenticationRequired),
    ] {
        assert_eq!(
            f.provider
                .user_membership(id, alias)
                .await
                .unwrap_err()
                .code,
            code
        );
    }
    let p = selected(&f, "caller", &source);
    assert_eq!(
        p.user_membership(None, Some("personal"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(p.take_response_credential().unwrap().is_none());
}

#[tokio::test]
async fn commerce_membership_never_commits_unverified_cookie_or_exports_partial_results() {
    for owner in ["default", "personal", "caller"] {
        for mutation in 0..7 {
            let mut f = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            f.put("personal", &source);
            f.put("default", &source);
            f.put("other", &source);
            let mut value = fixture();
            if mutation == 4 {
                value["membership"]["membership_type"] = json!("session-secret");
            }
            if mutation == 5 {
                value["membership"]["membership_type"] = json!("final-current");
            }
            if mutation == 6 {
                value["membership"] = json!({});
            }
            let mut replies = responses(value, "123456");
            let expected = match mutation {
                0 => {
                    replies[2] = account_reply("654321", Some("sessionid_ss=foreign-secret"));
                    ErrorCode::UpstreamError
                }
                1 => {
                    replies[2] = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".into();
                    ErrorCode::AuthenticationRequired
                }
                2 => {
                    replies[1] = crate::test_http::json(
                        r#"{"status_code":1000016}"#,
                        Some("sessionid_ss=bad-cookie"),
                    );
                    replies.truncate(2);
                    ErrorCode::AuthenticationRequired
                }
                3 => {
                    replies[1] = crate::test_http::json(r#"{"status_code":5}"#, None);
                    replies.truncate(2);
                    ErrorCode::UpstreamError
                }
                6 => {
                    replies.truncate(2);
                    ErrorCode::UpstreamError
                }
                _ => ErrorCode::UpstreamError,
            };
            let expected_requests = replies.len();
            let (origin, server) = crate::test_http::serve(replies).await;
            f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
            let p = selected(&f, owner, &source);
            let error = p.user_membership(None, account(owner)).await.unwrap_err();
            assert_eq!(error.code, expected, "mutation {mutation}");
            assert!(p.take_response_credential().unwrap().is_none());
            let current = if owner == "caller" {
                p.caller_credential
                    .as_ref()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .clone()
            } else {
                SodaCredential::parse(f.stored(owner).unwrap().secret()).unwrap()
            };
            assert_eq!(
                current.cookie_header().unwrap(),
                "sessionid_ss=profile-current"
            );
            assert_eq!(
                f.stored("other").unwrap().secret(),
                source.serialize().unwrap()
            );
            assert_eq!(server.await.unwrap().len(), expected_requests);
        }
    }
}

#[tokio::test]
async fn commerce_membership_every_late_success_and_error_observes_original_login() {
    for owner in ["default", "personal", "caller"] {
        for action in 0..3 {
            if owner == "caller" && action == 0 {
                continue;
            }
            for boundary in 0..3 {
                for failure in [false, true] {
                    let mut f = SessionFixture::new();
                    let source = test_soda_credential().bind_user("123456").unwrap();
                    f.put(owner, &source);
                    let mut replies = responses(fixture(), "123456");
                    replies.truncate(boundary + 1);
                    if failure {
                        replies[boundary] =
                            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".into();
                    }
                    let paused = crate::test_http::serve_paused_at(replies, boundary).await;
                    f.provider.client = f
                        .provider
                        .client
                        .clone()
                        .with_auth_test_origin(paused.origin);
                    let p = selected(&f, owner, &source);
                    let task_p = p.clone();
                    let task =
                        tokio::spawn(
                            async move { task_p.user_membership(None, account(owner)).await },
                        );
                    paused.arrived.await.unwrap();
                    let previous = if owner == "caller" {
                        p.caller_credential
                            .as_ref()
                            .unwrap()
                            .lock()
                            .unwrap()
                            .clone()
                    } else {
                        SodaCredential::parse(f.stored(owner).unwrap().secret()).unwrap()
                    };
                    let current_value = previous
                        .cookie_header()
                        .unwrap()
                        .split_once('=')
                        .unwrap()
                        .1
                        .to_owned();
                    let replacement = SodaCredential::test_credential(if action == 1 {
                        &current_value
                    } else {
                        "new-account-secret"
                    })
                    .bind_user(if action == 2 { "654321" } else { "123456" })
                    .unwrap();
                    if owner == "caller" {
                        *p.caller_credential.as_ref().unwrap().lock().unwrap() =
                            replacement.clone();
                    } else if action == 0 {
                        f.store.remove(Platform::Soda, owner).unwrap();
                    } else {
                        f.put(owner, &replacement);
                    }
                    paused.release.send(()).unwrap();
                    let err = task.await.unwrap().unwrap_err();
                    assert_eq!(
                        err.code,
                        ErrorCode::Conflict,
                        "{owner}/{action}/{boundary}/{failure}"
                    );
                    assert!(p.take_response_credential().unwrap().is_none());
                    if owner == "caller" {
                        assert_eq!(
                            *p.caller_credential.as_ref().unwrap().lock().unwrap(),
                            replacement
                        );
                    } else if action == 0 {
                        assert!(f.stored(owner).is_none());
                    } else {
                        assert_eq!(
                            f.stored(owner).unwrap().secret(),
                            replacement.serialize().unwrap()
                        );
                    }
                    assert_eq!(paused.requests.await.unwrap().len(), boundary + 1);
                }
            }
        }
    }
}

#[tokio::test]
async fn commerce_membership_cancel_and_timeout_discard_pending_updates() {
    for boundary in 0..3 {
        for timeout in [false, true] {
            let mut f = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            f.put("default", &source);
            let mut replies = responses(fixture(), "123456");
            replies.truncate(boundary + 1);
            let paused = crate::test_http::serve_paused_at(replies, boundary).await;
            f.provider.client = f
                .provider
                .client
                .clone()
                .with_auth_test_origin(paused.origin);
            let p = selected(&f, "caller", &source);
            let task_p = p.clone();
            let task = tokio::spawn(async move {
                task_p
                    .read_membership(
                        None,
                        None,
                        std::time::Duration::from_secs(if timeout { 2 } else { 45 }),
                    )
                    .await
            });
            paused.arrived.await.unwrap();
            if timeout {
                assert_eq!(
                    task.await.unwrap().unwrap_err().code,
                    ErrorCode::UpstreamTimeout
                );
            } else {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
            }
            assert!(p.take_response_credential().unwrap().is_none());
            assert_eq!(
                f.stored("default").unwrap().secret(),
                source.serialize().unwrap()
            );
            paused.release.send(()).unwrap();
            assert_eq!(paused.requests.await.unwrap().len(), boundary + 1);
        }
    }
}

#[tokio::test]
async fn commerce_membership_two_accounts_finish_in_reverse_without_sharing_results() {
    let f = SessionFixture::new();
    let first = test_soda_credential().bind_user("123456").unwrap();
    let second = SodaCredential::test_credential("second-secret")
        .bind_user("654321")
        .unwrap();
    f.put("first", &first);
    f.put("second", &second);
    let named = |member, user, suffix: &str| {
        responses(member, user)
            .into_iter()
            .map(|r| {
                r.replace("profile-current", &format!("profile-{suffix}"))
                    .replace("member-candidate", &format!("member-{suffix}"))
                    .replace("final-current", &format!("final-{suffix}"))
            })
            .collect()
    };
    let paused = crate::test_http::serve_paused_at(named(fixture(), "123456", "first"), 2).await;
    let mut p = f.provider.clone();
    p.client = p.client.clone().with_auth_test_origin(paused.origin);
    let task = tokio::spawn(async move { p.user_membership(None, Some("first")).await });
    paused.arrived.await.unwrap();
    let (origin, server) = crate::test_http::serve(named(
        body(json!({"is_membership":false})),
        "654321",
        "second",
    ))
    .await;
    let mut p = f.provider.clone();
    p.client = p.client.clone().with_auth_test_origin(origin);
    let b = p.user_membership(None, Some("second")).await.unwrap();
    assert_eq!(b.active, Some(false));
    assert_eq!(b.user_ref.unwrap().id(), "654321");
    paused.release.send(()).unwrap();
    let a = task.await.unwrap().unwrap();
    assert_eq!(a.active, Some(true));
    assert_eq!(a.user_ref.unwrap().id(), "123456");
    assert!(f.stored("first").unwrap().secret().contains("123456"));
    assert!(f.stored("second").unwrap().secret().contains("654321"));
    assert!(f.stored("first").unwrap().secret().contains("final-first"));
    assert!(
        f.stored("second")
            .unwrap()
            .secret()
            .contains("final-second")
    );
    assert_eq!(server.await.unwrap().len(), 3);
    assert_eq!(paused.requests.await.unwrap().len(), 3);
}
