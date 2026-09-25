use super::*;
use crate::client::artist_catalog::account_detail::fixture;
use std::time::Duration;

fn alias(owner: &str) -> Option<&str> {
    if owner == "caller" { None } else { Some(owner) }
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
fn detail_reply(v: &serde_json::Value) -> String {
    crate::test_http::json(
        &v.to_string(),
        Some("sessionid_ss=artist-final-session; Path=/"),
    )
}
fn replies() -> Vec<String> {
    vec![
        account_reply(
            "123456",
            Some("sessionid_ss=artist-verified-session; Path=/"),
        ),
        detail_reply(&fixture()),
    ]
}
async fn read(p: &SodaProvider, owner: &str, overview: bool) -> Result<serde_json::Value> {
    if overview {
        Ok(json!(p.artist_overview("123", alias(owner)).await?))
    } else {
        Ok(json!(p.artist("123", alias(owner)).await?))
    }
}

#[tokio::test]
async fn account_artist_detail_three_owners_keep_profile_preview_and_account_identity_separate() {
    for owner in ["default", "personal", "caller"] {
        for overview in [false, true] {
            let mut f = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            for a in ["default", "personal", "other"] {
                f.put(a, &source);
            }
            let (origin, server) = crate::test_http::serve(replies()).await;
            f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
            let p = provider(&f, owner, &source);
            let output = read(&p, owner, overview).await.unwrap();
            let artist = if overview { &output["artist"] } else { &output };
            assert_eq!(artist["id"], "123");
            assert_eq!(artist["track_count"], 3);
            assert_eq!(artist["album_count"], 7);
            assert_eq!(artist["extensions"]["source_user_id"], "123456");
            assert_eq!(artist["extensions"]["linked_user_id"], "456");
            assert_eq!(
                artist["extensions"]["backend"],
                "official_pc_account_artist_detail"
            );
            assert_eq!(artist["extensions"]["account_state"]["is_collected"], true);
            if overview {
                assert_eq!(output["featured_tracks"].as_array().unwrap().len(), 2);
                assert_eq!(output["has_more_tracks"], true);
                assert_eq!(output["extensions"]["preview_scope"], "hot_tracks");
                assert_eq!(
                    output["featured_tracks"][0]["extensions"]["source_user_id"],
                    "123456"
                );
            }
            for secret in [
                "session-secret",
                "artist-verified-session",
                "artist-final-session",
                "ignored-linked-user-secret",
            ] {
                assert!(!output.to_string().contains(secret));
            }
            let seen = server.await.unwrap();
            assert_eq!(seen.len(), 2);
            assert!(seen[0].contains("sessionid_ss=session-secret"));
            assert!(seen[1].contains("sessionid_ss=artist-verified-session"));
            assert!(seen[1].starts_with("GET /luna/pc/artists/123?"));
            assert!(!seen[1].contains("user_id="));
            assert_eq!(
                f.stored("other").unwrap().secret(),
                source.serialize().unwrap()
            );
            if owner == "caller" {
                assert!(
                    p.take_response_credential()
                        .unwrap()
                        .unwrap()
                        .secret()
                        .contains("artist-final-session")
                );
                for a in ["default", "personal"] {
                    assert_eq!(f.stored(a).unwrap().secret(), source.serialize().unwrap());
                }
            } else {
                assert!(
                    f.stored(owner)
                        .unwrap()
                        .secret()
                        .contains("artist-final-session")
                );
                assert!(p.take_response_credential().unwrap().is_none());
            }
        }
    }
}

#[tokio::test]
async fn account_artist_detail_every_late_success_and_error_respects_relogin_replacement_or_logout()
{
    for owner in ["default", "personal", "caller"] {
        for boundary in 0..2 {
            for change in ["same_cookie", "other_user", "logout"] {
                for failure in [false, true] {
                    if owner == "caller" && change == "logout" {
                        continue;
                    }
                    let mut f = SessionFixture::new();
                    let source = test_soda_credential().bind_user("123456").unwrap();
                    for a in ["default", "personal", "other"] {
                        f.put(a, &source);
                    }
                    let mut frames = replies();
                    frames.truncate(boundary + 1);
                    if failure {
                        frames[boundary]="HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into();
                    }
                    let paused = crate::test_http::serve_paused_at(frames, boundary).await;
                    f.provider.client = f
                        .provider
                        .client
                        .clone()
                        .with_auth_test_origin(paused.origin.clone());
                    let p = provider(&f, owner, &source);
                    let worker = p.clone();
                    let task = tokio::spawn(async move { read(&worker, owner, true).await });
                    paused.arrived.await.unwrap();
                    let replacement = if change == "same_cookie" {
                        SodaCredential::test_credential(if boundary == 0 {
                            "session-secret"
                        } else {
                            "artist-verified-session"
                        })
                        .bind_user("123456")
                        .unwrap()
                    } else {
                        SodaCredential::test_credential("replacement-session")
                            .bind_user("654321")
                            .unwrap()
                    };
                    if owner == "caller" {
                        *p.caller_credential.as_ref().unwrap().lock().unwrap() =
                            replacement.clone();
                    } else if change == "logout" {
                        f.store.remove(Platform::Soda, owner).unwrap();
                    } else {
                        f.put(owner, &replacement);
                    }
                    paused.release.send(()).unwrap();
                    assert_eq!(
                        task.await.unwrap().unwrap_err().code,
                        ErrorCode::Conflict,
                        "{owner}/{boundary}/{change}/{failure}"
                    );
                    assert!(p.take_response_credential().unwrap().is_none());
                    assert_eq!(paused.requests.await.unwrap().len(), boundary + 1);
                    assert_eq!(
                        f.stored("other").unwrap().secret(),
                        source.serialize().unwrap()
                    );
                    if change == "logout" {
                        assert!(f.stored(owner).is_none());
                    } else {
                        assert_eq!(
                            p.selected_credential(alias(owner).unwrap_or("default"))
                                .unwrap()
                                .unwrap()
                                .0,
                            replacement
                        );
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn account_artist_detail_cancel_total_timeout_or_invalid_response_preserves_only_accepted_rotations()
 {
    for owner in ["personal", "caller"] {
        for boundary in 0..2 {
            for action in ["cancel", "timeout", "invalid"] {
                let mut f = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                f.put("personal", &source);
                let mut frames = replies();
                frames.truncate(boundary + 1);
                if action == "invalid" {
                    frames[boundary] = detail_reply(&json!({}));
                }
                let paused = crate::test_http::serve_bytes_paused_at_with_hold_timeout(
                    frames.into_iter().map(String::into_bytes).collect(),
                    boundary,
                    Duration::from_secs(60),
                )
                .await;
                f.provider.client = f
                    .provider
                    .client
                    .clone()
                    .with_auth_test_origin(paused.origin.clone());
                let p = provider(&f, owner, &source);
                let worker = p.clone();
                let task = tokio::spawn(async move {
                    worker
                        .read_account_artist(
                            "123",
                            alias(owner),
                            Duration::from_secs(if action == "timeout" { 15 } else { 60 }),
                        )
                        .await
                });
                tokio::time::timeout(Duration::from_secs(10), paused.arrived)
                    .await
                    .unwrap()
                    .unwrap();
                if action == "cancel" {
                    task.abort();
                    assert!(task.await.unwrap_err().is_cancelled());
                } else if action == "timeout" {
                    tokio::time::pause();
                    let error = task.await.unwrap().unwrap_err();
                    tokio::time::resume();
                    assert_eq!(error.code, ErrorCode::UpstreamTimeout);
                    assert!(error.message.contains("total time budget"));
                } else {
                    paused.release.send(()).unwrap();
                    assert_eq!(
                        task.await.unwrap().unwrap_err().code,
                        ErrorCode::UpstreamError
                    );
                }
                assert!(p.take_response_credential().unwrap().is_none());
                let current = p
                    .selected_credential(alias(owner).unwrap_or("default"))
                    .unwrap()
                    .unwrap()
                    .0;
                assert!(current.cookie_header().unwrap().contains(if boundary == 0 {
                    "session-secret"
                } else {
                    "artist-verified-session"
                }));
                if action == "invalid" {
                    paused.requests.await.unwrap();
                } else {
                    paused.requests.abort();
                    assert!(paused.requests.await.unwrap_err().is_cancelled());
                }
            }
        }
    }
}

#[tokio::test]
async fn account_artist_detail_validates_all_preview_metadata_and_initial_secret_before_publishing_final_cookie()
 {
    for case in [
        "initial_secret",
        "current_secret",
        "new_secret",
        "wrong_artist",
        "bad_tail",
        "false_complete",
    ] {
        for overview in [false, true] {
            let mut f = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            let mut v = fixture();
            match case {
                "initial_secret" => {
                    v["artist_info"]["artist_profile"]["intro"] = json!("session-secret")
                }
                "current_secret" => v["artist_info"]["name"] = json!("artist-verified-session"),
                "new_secret" => v["hot_tracks"][1]["name"] = json!("artist-final-session"),
                "wrong_artist" => v["artist_info"]["id"] = json!("456"),
                "bad_tail" => v["hot_tracks"][1]["artists"][0]["id"] = json!("789"),
                _ => v["has_more_tracks"] = json!(false),
            }
            let mut frames = replies();
            frames[1] = detail_reply(&v);
            let (origin, server) = crate::test_http::serve(frames).await;
            f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
            let p = provider(&f, "caller", &source);
            let error = read(&p, "caller", overview).await.unwrap_err();
            assert_eq!(error.code, ErrorCode::UpstreamError, "{case}");
            assert!(p.take_response_credential().unwrap().is_none());
            assert!(
                p.selected_credential("default")
                    .unwrap()
                    .unwrap()
                    .0
                    .cookie_header()
                    .unwrap()
                    .contains("artist-verified-session")
            );
            assert_eq!(server.await.unwrap().len(), 2);
        }
    }
}

struct NoStore;
impl AccountCredentialStore for NoStore {
    fn load_platform(&self, _: Platform) -> Result<Vec<StoredAccountCredential>> {
        panic!("caller read server store")
    }
    fn put(&self, _: &StoredAccountCredential) -> Result<()> {
        panic!("caller wrote server store")
    }
    fn remove(&self, _: Platform, _: &str) -> Result<bool> {
        panic!("caller removed server store")
    }
}
struct FailedSave {
    inner: Arc<tuneweave_core::FileAccountCredentialStore>,
    calls: std::sync::atomic::AtomicUsize,
    fail_at: usize,
}
impl AccountCredentialStore for FailedSave {
    fn load_platform(&self, p: Platform) -> Result<Vec<StoredAccountCredential>> {
        self.inner.load_platform(p)
    }
    fn put(&self, c: &StoredAccountCredential) -> Result<()> {
        self.inner.put(c)
    }
    fn remove(&self, p: Platform, a: &str) -> Result<bool> {
        self.inner.remove(p, a)
    }
    fn compare_exchange(
        &self,
        expected: &StoredAccountCredential,
        replacement: Option<&StoredAccountCredential>,
    ) -> Result<bool> {
        if self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1 == self.fail_at {
            return Err(TuneWeaveError::new(
                ErrorCode::InternalError,
                "fixture credential save failed",
            ));
        }
        self.inner.compare_exchange(expected, replacement)
    }
}

#[tokio::test]
async fn account_artist_detail_caller_never_uses_server_store_and_failed_conditional_saves_stop_the_read()
 {
    let source = test_soda_credential().bind_user("123456").unwrap();
    let (origin, server) = crate::test_http::serve(replies()).await;
    let mut p = SodaProvider::new(SodaConfig {
        credential_store: Some(Arc::new(NoStore)),
        ..SodaConfig::default()
    })
    .unwrap();
    p.client = p.client.with_auth_test_origin(origin);
    let p = p.caller_credential_scope(&caller_from(&source)).unwrap();
    read(&p, "caller", true).await.unwrap();
    assert_eq!(server.await.unwrap().len(), 2);
    for fail_at in [1, 2] {
        let mut f = SessionFixture::new();
        f.put("personal", &source);
        f.provider.credential_store = Some(Arc::new(FailedSave {
            inner: Arc::clone(&f.store),
            calls: std::sync::atomic::AtomicUsize::new(0),
            fail_at,
        }));
        let mut frames = replies();
        frames.truncate(fail_at);
        let (origin, server) = crate::test_http::serve(frames).await;
        f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
        assert_eq!(
            read(&f.provider, "personal", true).await.unwrap_err().code,
            ErrorCode::InternalError
        );
        assert!(f.provider.take_response_credential().unwrap().is_none());
        assert_eq!(server.await.unwrap().len(), fail_at);
        assert!(
            f.stored("personal")
                .unwrap()
                .secret()
                .contains(if fail_at == 1 {
                    "session-secret"
                } else {
                    "artist-verified-session"
                })
        );
    }
}

#[tokio::test]
async fn account_artist_detail_transport_errors_challenges_and_missing_sources_never_fall_back_to_public()
 {
    let mut f = SessionFixture::new();
    f.provider.client = f
        .provider
        .client
        .clone()
        .with_auth_test_origin(url::Url::parse("http://127.0.0.1:1/").unwrap());
    assert_eq!(
        read(&f.provider, "personal", true).await.unwrap_err().code,
        ErrorCode::AuthenticationRequired
    );
    for id in ["0", "0123", "123/path", "18446744073709551616"] {
        assert_eq!(
            f.provider
                .artist(id, Some("personal"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for (last,code) in [
        ("HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),ErrorCode::AuthenticationRequired),
        ("HTTP/1.1 302 Found\r\nLocation: https://example.invalid/artist-verified-session\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),ErrorCode::UpstreamError),
        ("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".into(),ErrorCode::UpstreamError),
        ("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 8388609\r\nConnection: close\r\n\r\n{}".into(),ErrorCode::UpstreamError),
        ("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nbdturing-verify: private-challenge\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".into(),ErrorCode::CapabilityNotSupported),
        (detail_reply(&json!({"status_code":1000016})),ErrorCode::AuthenticationRequired),
        (detail_reply(&json!({"status_code":1000004})),ErrorCode::UpstreamError),
    ] {
        let mut f=SessionFixture::new();let source=test_soda_credential().bind_user("123456").unwrap();let mut frames=replies();frames[1]=last;let (origin,server)=crate::test_http::serve(frames).await;f.provider.client=f.provider.client.clone().with_auth_test_origin(origin);let p=provider(&f,"caller",&source);let error=read(&p,"caller",false).await.unwrap_err();assert_eq!(error.code,code);for secret in ["artist-verified-session","private-challenge","example.invalid"] {assert!(!format!("{error:?}").contains(secret));}assert!(p.take_response_credential().unwrap().is_none());assert_eq!(server.await.unwrap().len(),2);
    }
}

#[tokio::test]
async fn account_artist_detail_reverse_account_completion_preserves_each_source_and_preview() {
    let mut f = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    let other = SodaCredential::test_credential("other-input-session")
        .bind_user("654321")
        .unwrap();
    f.put("personal", &source);
    f.put("other", &other);
    let paused = crate::test_http::serve_paused_at(replies(), 1).await;
    f.provider.client = f
        .provider
        .client
        .clone()
        .with_auth_test_origin(paused.origin.clone());
    let worker = f.provider.clone();
    let task = tokio::spawn(async move { read(&worker, "personal", true).await });
    paused.arrived.await.unwrap();
    let (origin, server) = crate::test_http::serve(
        replies()
            .into_iter()
            .map(|r| {
                r.replace("123456", "654321")
                    .replace("sessionid_ss=", "sessionid_ss=other-")
            })
            .collect(),
    )
    .await;
    let mut p = f.provider.clone();
    p.client = p.client.with_auth_test_origin(origin);
    let b = read(&p, "other", true).await.unwrap();
    assert_eq!(b["artist"]["extensions"]["source_user_id"], "654321");
    paused.release.send(()).unwrap();
    let a = task.await.unwrap().unwrap();
    assert_eq!(a["artist"]["extensions"]["source_user_id"], "123456");
    assert_eq!(
        a["artist"]["extensions"]["linked_user_id"],
        b["artist"]["extensions"]["linked_user_id"]
    );
    assert!(
        f.stored("personal")
            .unwrap()
            .secret()
            .contains("artist-final-session")
    );
    assert!(
        f.stored("other")
            .unwrap()
            .secret()
            .contains("other-artist-final-session")
    );
    server.await.unwrap();
    paused.requests.await.unwrap();
}
