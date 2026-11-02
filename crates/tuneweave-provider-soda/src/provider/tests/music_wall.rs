use super::*;
fn wall_reply(other: bool, cookie: &str) -> String {
    crate::test_http::json(&if other {json!({"status_code":0,"user_brief":{"id":"789"},"user_listen_duration":{"total_duration":12,"week_duration":2}})}
        else {json!({"status_code":0,"listen_duration":null,"tracks":null})}.to_string(), Some(cookie))
}
#[tokio::test]
async fn music_wall_uses_bound_account_and_native_self_or_user_route() {
    for owner in ["personal", "caller"] {
        for other in [false, true] {
            let mut f = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            f.put("personal", &source);
            f.put("other", &source);
            let (origin, requests) = crate::test_http::serve(vec![
                account_reply("123456", Some("sessionid_ss=verified")),
                wall_reply(other, "sessionid_ss=final"),
            ])
            .await;
            f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
            let provider = if owner == "caller" {
                f.provider
                    .caller_credential_scope(&caller_from(&source))
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let result = provider
                .music_wall(other.then_some("789"), (owner != "caller").then_some(owner))
                .await
                .unwrap();
            assert_eq!(
                result["user_ref"],
                if other { "soda:789" } else { "soda:123456" }
            );
            assert!(result["duration_unit"].is_null());
            let requests = requests.await.unwrap();
            assert_eq!(requests.len(), 2);
            assert!(requests[1].contains("sessionid_ss=verified"));
            assert!(requests[1].starts_with(if other {
                "GET /luna/users/789/music_wall?"
            } else {
                "POST /luna/me/music_wall?"
            }));
            if !other {
                assert!(requests[1].contains("\"schema_extra\":\"\""));
            }
            assert_eq!(
                f.stored("other").unwrap().secret(),
                source.serialize().unwrap()
            );
            if owner == "caller" {
                assert!(
                    provider
                        .take_response_credential()
                        .unwrap()
                        .unwrap()
                        .secret()
                        .contains("final")
                );
            } else {
                assert!(f.stored("personal").unwrap().secret().contains("final"));
            }
        }
    }
}
#[tokio::test]
async fn music_wall_late_response_does_not_cross_login_generation() {
    let mut f = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    f.put("personal", &source);
    let paused = crate::test_http::serve_paused_at(
        vec![
            account_reply("123456", None),
            wall_reply(false, "sessionid_ss=poison"),
        ],
        1,
    )
    .await;
    f.provider.client = f
        .provider
        .client
        .clone()
        .with_auth_test_origin(paused.origin);
    let provider = f.provider.clone();
    let task = tokio::spawn(async move { provider.music_wall(None, Some("personal")).await });
    paused.arrived.await.unwrap();
    let replacement = test_soda_credential().bind_user("999999").unwrap();
    f.put("personal", &replacement);
    paused.release.send(()).unwrap();
    assert!(task.await.unwrap().is_err());
    paused.requests.await.unwrap();
    assert_eq!(
        f.stored("personal").unwrap().secret(),
        replacement.serialize().unwrap()
    );
    assert!(f.provider.take_response_credential().unwrap().is_none());
}
