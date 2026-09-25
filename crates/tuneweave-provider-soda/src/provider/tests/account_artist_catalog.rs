use super::*;
use std::time::Duration;
use tuneweave_core::{ArtistTrackListRequest, ArtistTrackOrder};

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
fn profile() -> serde_json::Value {
    json!({"status_info":{"now":1,"now_ts_ms":1000},"artist_info":{"id":"123","name":"Artist","count_tracks":52,"count_albums":52}})
}
fn page(kind: &str, last: bool) -> serde_json::Value {
    let mut v = json!({"status_info":{"now":1,"now_ts_ms":1000},"has_more":!last,"next_cursor":if last {"401"} else {"99"}});
    v[kind] = json!(
        (if last { 51..=52 } else { 1..=50 })
            .map(
                |i| json!({"id":i.to_string(),"name":format!("Work {i}"),"duration":1000,
        "artists":[{"id":"789","name":"Collaborator"},{"id":"123","name":"Artist"}]})
            )
            .collect::<Vec<_>>()
    );
    v
}
fn reply(v: &serde_json::Value, cookie: &str) -> String {
    crate::test_http::json(
        &v.to_string(),
        Some(&format!("sessionid_ss={cookie}; Path=/")),
    )
}
fn replies(kind: &str) -> Vec<String> {
    vec![
        account_reply("123456", Some("sessionid_ss=verified-session; Path=/")),
        reply(&profile(), "profile-session"),
        reply(&page(kind, false), "first-session"),
        reply(&page(kind, true), "last-session"),
    ]
}
async fn read(
    p: &SodaProvider,
    owner: &str,
    kind: &str,
    offset: u32,
) -> Result<(serde_json::Value, PageMeta)> {
    if kind == "tracks" {
        let p = p
            .artist_tracks(
                "123",
                &ArtistTrackListRequest {
                    limit: 2,
                    offset,
                    account: alias(owner).map(str::to_owned),
                    order: ArtistTrackOrder::PlatformDefault,
                },
            )
            .await?;
        Ok((json!(p.items), p.pagination))
    } else {
        let p = p
            .artist_albums(
                "123",
                &PageRequest {
                    limit: 2,
                    offset,
                    account: alias(owner).map(str::to_owned),
                },
            )
            .await?;
        Ok((json!(p.items), p.pagination))
    }
}

#[tokio::test]
async fn account_artist_catalog_three_sources_keep_complete_paging_identity_and_cookie_chain() {
    for owner in ["default", "personal", "caller"] {
        for kind in ["tracks", "albums"] {
            for offset in [49, 51, 100] {
                let mut f = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                for a in ["default", "personal", "other"] {
                    f.put(a, &source);
                }
                let (origin, server) = crate::test_http::serve(replies(kind)).await;
                f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
                let p = provider(&f, owner, &source);
                let (items, meta) = read(&p, owner, kind, offset).await.unwrap();
                let ids = items
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v["id"].as_str().unwrap())
                    .collect::<Vec<_>>();
                assert_eq!(
                    ids,
                    match offset {
                        49 => vec!["50", "51"],
                        51 => vec!["52"],
                        _ => vec![],
                    }
                );
                assert_eq!(meta.total, Some(52));
                assert_eq!(meta.has_more, offset == 49);
                assert_eq!(meta.next_offset, if offset == 49 { Some(51) } else { None });
                assert_eq!(meta.extensions["source_user_id"], "123456");
                assert_eq!(meta.extensions["authenticated"], true);
                assert_eq!(meta.extensions["upstream_pages"], 2);
                assert_eq!(meta.extensions["reported_total"], 52);
                assert_eq!(
                    meta.extensions["backend"],
                    format!("official_pc_account_artist_{kind}")
                );
                for item in items.as_array().unwrap() {
                    assert_eq!(item["extensions"]["source_user_id"], "123456");
                    assert_eq!(item["artists"].as_array().unwrap().len(), 2);
                }
                assert!(!items.to_string().contains("session"));
                let seen = server.await.unwrap();
                assert_eq!(seen.len(), 4);
                for (i, wire) in seen.iter().enumerate() {
                    assert!(wire.contains(
                        [
                            "sessionid_ss=session-secret",
                            "sessionid_ss=verified-session",
                            "sessionid_ss=profile-session",
                            "sessionid_ss=first-session"
                        ][i]
                    ));
                    if i > 0 {
                        let target = wire
                            .lines()
                            .next()
                            .unwrap()
                            .split_whitespace()
                            .nth(1)
                            .unwrap();
                        let url =
                            url::Url::parse(&format!("https://api.qishui.com{target}")).unwrap();
                        let q = url.query_pairs().collect::<BTreeMap<_, _>>();
                        assert_eq!(
                            url.path(),
                            if i == 1 {
                                "/luna/pc/artists/123".into()
                            } else {
                                format!("/luna/pc/artists/123/{kind}")
                            }
                        );
                        assert!(!q["device_id"].is_empty());
                        assert!(!q["iid"].is_empty());
                        assert_eq!(q["fp"], q["device_id"]);
                        if i > 1 {
                            assert_eq!(q["cursor"], if i == 2 { "" } else { "99" });
                            assert_eq!(q["count"], "50");
                        }
                        assert!(!q.contains_key("user_id"));
                    }
                }
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
                            .contains("last-session")
                    );
                    for a in ["default", "personal"] {
                        assert_eq!(f.stored(a).unwrap().secret(), source.serialize().unwrap());
                    }
                } else {
                    assert!(f.stored(owner).unwrap().secret().contains("last-session"));
                    assert!(p.take_response_credential().unwrap().is_none());
                }
            }
        }
    }
}

#[tokio::test]
async fn account_artist_catalog_every_network_success_and_error_checks_original_generation() {
    for owner in ["default", "personal", "caller"] {
        for kind in ["tracks", "albums"] {
            for boundary in 0..4 {
                for change in ["same_cookie_login", "other_user", "logout"] {
                    for failure in [false, true] {
                        if owner == "caller" && change == "logout" {
                            continue;
                        }
                        let mut f = SessionFixture::new();
                        let source = test_soda_credential().bind_user("123456").unwrap();
                        for a in ["default", "personal", "other"] {
                            f.put(a, &source);
                        }
                        let mut frames = replies(kind);
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
                        let task =
                            tokio::spawn(async move { read(&worker, owner, kind, 49).await });
                        paused.arrived.await.unwrap();
                        let replacement = if change == "same_cookie_login" {
                            SodaCredential::test_credential(
                                [
                                    "session-secret",
                                    "verified-session",
                                    "profile-session",
                                    "first-session",
                                ][boundary],
                            )
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
                            "{owner}/{kind}/{boundary}/{change}/{failure}"
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
}

#[tokio::test]
async fn account_artist_catalog_cancel_total_deadline_and_invalid_pages_clear_pending_updates() {
    for owner in ["personal", "caller"] {
        for boundary in 0..4 {
            for action in ["cancel", "timeout", "invalid"] {
                let mut f = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                f.put("personal", &source);
                let mut frames = replies("tracks");
                frames.truncate(boundary + 1);
                if action == "invalid" {
                    frames[boundary] = reply(&json!({}), "unaccepted-session");
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
                        .read_account_artist_catalogue::<Track>(
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
                assert!(
                    !current
                        .cookie_header()
                        .unwrap()
                        .contains("unaccepted-session")
                );
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
async fn account_artist_catalog_rejects_secrets_from_original_intermediate_and_later_sessions_even_outside_window()
 {
    for kind in ["tracks", "albums"] {
        for secret in [
            "session-secret",
            "verified-session",
            "profile-session",
            "first-session",
            "last-session",
        ] {
            let mut f = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            let mut frames = replies(kind);
            let mut first = page(kind, false);
            first[kind][0]["name"] = json!(format!("reflected {secret}"));
            frames[2] = reply(&first, "first-session");
            let (origin, server) = crate::test_http::serve(frames).await;
            f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
            let p = provider(&f, "caller", &source);
            let error = read(&p, "caller", kind, 51).await.unwrap_err();
            assert_eq!(error.code, ErrorCode::UpstreamError);
            assert!(!format!("{error:?}").contains(secret));
            assert!(p.take_response_credential().unwrap().is_none());
            assert_eq!(server.await.unwrap().len(), 4);
        }
    }
}

#[tokio::test]
async fn account_artist_catalog_rejects_late_malformed_identity_counts_cursors_and_refuses_new_cookie()
 {
    for kind in ["tracks", "albums"] {
        for bad in [
            "duplicate",
            "credit",
            "count",
            "cursor",
            "missing",
            "business",
            "auth",
        ] {
            let mut f = SessionFixture::new();
            let source = test_soda_credential().bind_user("123456").unwrap();
            let mut last = page(kind, true);
            match bad {
                "duplicate" => last[kind][1]["id"] = json!("1"),
                "credit" => last[kind][1]["artists"][1]["id"] = json!("456"),
                "count" => {
                    last[kind].as_array_mut().unwrap().pop();
                }
                "cursor" => last["has_more"] = json!(true),
                "missing" => {
                    last.as_object_mut().unwrap().remove("next_cursor");
                }
                "business" => last = json!({"status_code":7}),
                _ => last = json!({"status_code":1000016}),
            }
            if bad == "cursor" {
                last["next_cursor"] = json!("99");
            }
            let mut frames = replies(kind);
            frames[3] = reply(&last, "unaccepted-session");
            let (origin, server) = crate::test_http::serve(frames).await;
            f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
            let p = provider(&f, "caller", &source);
            assert_eq!(
                read(&p, "caller", kind, 0).await.unwrap_err().code,
                if bad == "auth" {
                    ErrorCode::AuthenticationRequired
                } else {
                    ErrorCode::UpstreamError
                }
            );
            assert!(p.take_response_credential().unwrap().is_none());
            assert!(
                !p.selected_credential("default")
                    .unwrap()
                    .unwrap()
                    .0
                    .cookie_header()
                    .unwrap()
                    .contains("unaccepted")
            );
            assert_eq!(server.await.unwrap().len(), 4);
        }
    }
}

struct NoStore;
impl AccountCredentialStore for NoStore {
    fn load_platform(&self, _: Platform) -> Result<Vec<StoredAccountCredential>> {
        panic!("unexpected store read")
    }
    fn put(&self, _: &StoredAccountCredential) -> Result<()> {
        panic!("unexpected store write")
    }
    fn remove(&self, _: Platform, _: &str) -> Result<bool> {
        panic!("unexpected store removal")
    }
}

#[tokio::test]
async fn account_artist_catalog_transport_challenges_and_failures_never_retry_or_export_partial_updates()
 {
    let failures = [
        ("HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(), ErrorCode::AuthenticationRequired),
        ("HTTP/1.1 302 Found\r\nLocation: https://example.invalid/session-secret\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(), ErrorCode::UpstreamError),
        ("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 14\r\nConnection: close\r\n\r\nsession-secret".into(), ErrorCode::UpstreamError),
        ("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nbdturing-verify: private-challenge\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".into(), ErrorCode::CapabilityNotSupported),
        ("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 8388609\r\nConnection: close\r\n\r\n{}".into(), ErrorCode::UpstreamError),
        (reply(&json!({"status_code":1000016}),"rejected-session"), ErrorCode::AuthenticationRequired),
    ];
    for kind in ["tracks", "albums"] {
        for boundary in [1, 3] {
            for (last, code) in &failures {
                let mut f = SessionFixture::new();
                let source = test_soda_credential().bind_user("123456").unwrap();
                let mut frames = replies(kind);
                frames.truncate(boundary + 1);
                frames[boundary] = last.clone();
                let (origin, server) = crate::test_http::serve(frames).await;
                f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
                let p = provider(&f, "caller", &source);
                let error = read(&p, "caller", kind, 49).await.unwrap_err();
                assert_eq!(error.code, *code, "{kind}/{boundary}");
                for secret in [
                    "session-secret",
                    "private-challenge",
                    "example.invalid",
                    "rejected-session",
                ] {
                    assert!(!format!("{error:?}").contains(secret));
                }
                assert!(p.take_response_credential().unwrap().is_none());
                assert_eq!(server.await.unwrap().len(), boundary + 1);
            }
        }
    }
}

#[tokio::test]
async fn account_artist_catalog_caller_never_touches_server_storage_and_invalid_sources_stop_before_io()
 {
    for kind in ["tracks", "albums"] {
        let source = test_soda_credential().bind_user("123456").unwrap();
        let (origin, server) = crate::test_http::serve(replies(kind)).await;
        let mut p = SodaProvider::new(SodaConfig {
            credential_store: Some(Arc::new(NoStore)),
            ..SodaConfig::default()
        })
        .unwrap();
        p.client = p.client.with_auth_test_origin(origin);
        let p = p.caller_credential_scope(&caller_from(&source)).unwrap();
        read(&p, "caller", kind, 0).await.unwrap();
        assert_eq!(server.await.unwrap().len(), 4);
    }
    let mut f = SessionFixture::new();
    f.provider.client = f
        .provider
        .client
        .clone()
        .with_auth_test_origin(url::Url::parse("http://127.0.0.1:1/").unwrap());
    for id in ["0", "0123", "123/x", "18446744073709551616"] {
        assert_eq!(
            f.provider
                .artist_albums(
                    id,
                    &PageRequest {
                        account: Some("personal".into()),
                        ..PageRequest::new(2, 0)
                    }
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        read(&f.provider, "personal", "tracks", 0)
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
}

#[tokio::test]
async fn account_artist_catalog_reverse_accounts_do_not_mix_catalogues_or_rotations() {
    let mut f = SessionFixture::new();
    let a = test_soda_credential().bind_user("123456").unwrap();
    let b = SodaCredential::test_credential("other-input")
        .bind_user("654321")
        .unwrap();
    f.put("personal", &a);
    f.put("other", &b);
    let paused = crate::test_http::serve_paused_at(replies("tracks"), 3).await;
    f.provider.client = f
        .provider
        .client
        .clone()
        .with_auth_test_origin(paused.origin.clone());
    let worker = f.provider.clone();
    let task = tokio::spawn(async move { read(&worker, "personal", "tracks", 49).await });
    paused.arrived.await.unwrap();
    let (origin, server) = crate::test_http::serve(
        replies("albums")
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
    let (_, meta) = read(&p, "other", "albums", 49).await.unwrap();
    assert_eq!(meta.extensions["source_user_id"], "654321");
    paused.release.send(()).unwrap();
    let (_, meta) = task.await.unwrap().unwrap();
    assert_eq!(meta.extensions["source_user_id"], "123456");
    assert!(
        f.stored("personal")
            .unwrap()
            .secret()
            .contains("last-session")
    );
    assert!(
        f.stored("other")
            .unwrap()
            .secret()
            .contains("other-last-session")
    );
    server.await.unwrap();
    paused.requests.await.unwrap();
}
