use super::*;
use crate::client::player_info::encrypted_tests as synthetic;

fn replies(preview: bool, content: bool) -> Vec<Vec<u8>> {
    let (body, info) = synthetic::fixture(preview);
    let mut out = vec![
        account_reply("123456", Some("sessionid_ss=verified")).into_bytes(),
        crate::test_http::json(&body.to_string(), Some("sessionid_ss=media-current")).into_bytes(),
        crate::test_http::json(&info.to_string(), Some("sessionid_ss=info-must-not-rotate"))
            .into_bytes(),
    ];
    if content {
        out.push(synthetic::audio_reply());
    }
    out
}

fn provider(f: &SessionFixture, owner: &str, source: &SodaCredential) -> SodaProvider {
    if owner == "caller" {
        f.provider
            .caller_credential_scope(&caller_from(source))
            .unwrap()
    } else {
        f.provider.clone()
    }
}

#[tokio::test]
async fn secondary_cenc_three_sources_deliver_bound_content_and_preserve_rights_and_rotation() {
    for owner in ["default", "personal", "caller"] {
        for preview in [false, true] {
            for operation in 0..4 {
                let mut f = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                for alias in ["default", "personal", "other"] {
                    f.put(alias, &source);
                }
                let (origin, server) =
                    crate::test_http::serve_bytes(replies(preview, operation == 2)).await;
                f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
                let p = provider(&f, owner, &source);
                let result =
                    secondary_media_result(&p, operation, (owner != "caller").then_some(owner))
                        .await
                        .unwrap();
                match operation {
                    0 => assert_eq!(result["trial"].is_object(), preview),
                    1 => {
                        assert_eq!(result["available"], !preview);
                        assert_eq!(result["url"].is_null(), preview);
                    }
                    2 => {
                        let bytes: Vec<u8> =
                            serde_json::from_value(result["bytes"].clone()).unwrap();
                        synthetic::assert_decrypted(&bytes);
                        assert_eq!(result["content_type"], "audio/mp4");
                    }
                    _ => {
                        assert_eq!(result["playable"], !preview);
                        assert_eq!(result["extensions"]["preview_available"], preview);
                    }
                }
                let serialized = result.to_string();
                for secret in [
                    synthetic::SPADE,
                    synthetic::KID,
                    "info-must-not-rotate",
                    "cdn-must-not-rotate",
                    "player-secret",
                ] {
                    assert!(!serialized.contains(secret));
                }
                if operation < 2
                    && let Some(url) = result["url"].as_str()
                {
                    assert!(url.starts_with("/v1/tracks/soda:"));
                    assert_eq!(url.contains("account="), owner != "caller");
                }
                if owner == "caller" {
                    let updated = p.take_response_credential().unwrap().unwrap();
                    assert_eq!(
                        parse_soda_caller_credential(&updated)
                            .unwrap()
                            .cookie_header()
                            .unwrap(),
                        "sessionid_ss=media-current"
                    );
                } else {
                    assert!(f.stored(owner).unwrap().secret().contains("media-current"));
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
                let requests = server.await.unwrap();
                assert_eq!(requests.len(), if operation == 2 { 4 } else { 3 });
                assert!(requests[0].contains("cookie: sessionid_ss=session-secret\r\n"));
                assert!(requests[1].contains("cookie: sessionid_ss=verified\r\n"));
                for r in &requests[2..] {
                    assert!(!r.to_ascii_lowercase().contains("cookie:") && !r.contains("twc1_"));
                }
            }
        }
    }
}

#[tokio::test]
async fn secondary_cenc_every_late_success_or_failure_observes_original_login_generation() {
    for owner in ["default", "personal", "caller"] {
        for action in 0..3 {
            if owner == "caller" && action == 0 {
                continue;
            }
            for boundary in 0..4 {
                for failure in [false, true] {
                    let mut f = SessionFixture::new();
                    let source = test_soda_credential().bind_user("123456").unwrap();
                    f.put(owner, &source);
                    let mut responses = replies(false, true);
                    responses.truncate(boundary + 1);
                    if failure {
                        responses[boundary] = b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec();
                    }
                    let paused = crate::test_http::serve_bytes_paused_at(responses, boundary).await;
                    f.provider.client = f
                        .provider
                        .client
                        .clone()
                        .with_auth_test_origin(paused.origin);
                    let p = provider(&f, owner, &source);
                    let task_provider = p.clone();
                    let task = tokio::spawn(async move {
                        secondary_media_result(
                            &task_provider,
                            2,
                            (owner != "caller").then_some(owner),
                        )
                        .await
                    });
                    paused.arrived.await.unwrap();
                    let replacement = SodaCredential::test_credential(if action == 1 {
                        "session-secret"
                    } else {
                        "replacement-secret"
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
                    let error = task.await.unwrap().unwrap_err();
                    assert_eq!(
                        error.code,
                        ErrorCode::Conflict,
                        "owner={owner} boundary={boundary} action={action} failure={failure}"
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
