use super::*;
use crate::provider::catalog::tests::server;
use std::collections::BTreeMap;

mod atomic_store;

#[derive(Default)]
pub(in crate::provider) struct Store(Mutex<BTreeMap<String, StoredAccountCredential>>);
impl AccountCredentialStore for Store {
    fn load_platform(&self, platform: Platform) -> Result<Vec<StoredAccountCredential>> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .values()
            .filter(|v| v.platform == platform)
            .cloned()
            .collect())
    }
    fn put(&self, credential: &StoredAccountCredential) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(credential.account.clone(), credential.clone());
        Ok(())
    }
    fn insert_if_absent(&self, credential: &StoredAccountCredential) -> Result<bool> {
        let mut values = self.0.lock().unwrap();
        if values.contains_key(&credential.account) {
            return Ok(false);
        }
        values.insert(credential.account.clone(), credential.clone());
        Ok(true)
    }
    fn remove(&self, _: Platform, account: &str) -> Result<bool> {
        Ok(self.0.lock().unwrap().remove(account).is_some())
    }
    fn compare_exchange(
        &self,
        expected: &StoredAccountCredential,
        next: Option<&StoredAccountCredential>,
    ) -> Result<bool> {
        let mut values = self.0.lock().unwrap();
        if values.get(&expected.account) != Some(expected) {
            return Ok(false);
        }
        if let Some(next) = next {
            values.insert(expected.account.clone(), next.clone());
        } else {
            values.remove(&expected.account);
        }
        Ok(true)
    }
}

fn reply(body: serde_json::Value, headers: &str) -> String {
    let body = body.to_string();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}Connection: close\r\n\r\n{body}",
        body.len()
    )
}
pub(in crate::provider) fn profile(uid: &str, headers: &str) -> String {
    reply(
        json!({"code":"000000","data":{"userId":uid,"nickName":"Listener","smallIcon":"https://d.musicapp.migu.cn/avatar.png","msisdn":"do-not-retain-phone","usessionId":"do-not-retain-session","province":"do-not-retain-location"}}),
        headers,
    )
}
fn check(token: &str) -> String {
    reply(json!({"code":"000000"}), &format!("pacmtoken: {token}\r\n"))
}
fn request(account: &str, token: &str) -> CredentialImportRequest {
    CredentialImportRequest {
        account: account.into(),
        credential: ImportedCredential::Cookie {
            value: format!("other=discard; pacmtoken={token}"),
        },
    }
}
pub(in crate::provider) fn stored(
    account: &str,
    value: &MiguCredential,
) -> StoredAccountCredential {
    StoredAccountCredential::new(Platform::Migu, account, KIND, value.serialize().unwrap()).unwrap()
}
pub(in crate::provider) fn read(store: &Store, account: &str) -> MiguCredential {
    MiguCredential::parse(store.0.lock().unwrap().get(account).unwrap().secret()).unwrap()
}

#[tokio::test]
async fn import_verifies_rotated_pacm_and_uid_before_applying_ownership() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let (mut provider, requests) = server(vec![check("checked"), profile("111", "Set-Cookie: pacmtoken=profile-current; Domain=.migu.cn; Path=/; Secure; HttpOnly\r\n")]).await;
        let store = Arc::new(Store::default());
        provider.credential_store = Some(store.clone());
        let account = if mode == CredentialMode::Client {
            "default"
        } else {
            "A"
        };
        let result = provider
            .import_credential(&request(account, "imported"), mode)
            .await
            .unwrap();
        assert!(result.profile.authenticated);
        assert_eq!(result.profile.account, account);
        assert_eq!(result.profile.user_id.as_deref(), Some("111"));
        let serialized = serde_json::to_string(&result.profile).unwrap();
        for secret in ["checked", "profile-current", "do-not-retain", "usessionId"] {
            assert!(!serialized.contains(secret));
        }
        assert_eq!(result.credential.is_some(), mode.returns_to_caller());
        assert_eq!(
            !store.0.lock().unwrap().is_empty(),
            mode.persists_on_server()
        );
        if let Some(caller) = result.credential {
            let caller = MiguCredential::parse_caller(&caller).unwrap();
            assert_eq!(caller.token(), "profile-current");
            if mode == CredentialMode::Both {
                assert_eq!(caller, read(&store, account));
            }
        }
        let requests = requests.await.unwrap();
        assert!(requests[0].starts_with("GET /mgateway/api/checkPacMtoken HTTP/1.1"));
        assert!(requests[0].contains("pacmtoken: imported\r\n"));
        assert!(requests[1].starts_with("GET /user/h5/user-info/v1.0 HTTP/1.1"));
        assert!(requests[1].contains("pacmtoken: checked\r\n"));
        assert!(requests.iter().all(|v| !v.contains("cookie:")
            && !v.contains("other=discard")
            && !v.contains("uid:")));
    }
}

#[test]
fn credentials_bound_identity_generation_and_secrets_have_strict_boundaries() {
    let a = MiguCredential::verified("111".into(), "secret".into()).unwrap();
    let b = MiguCredential::verified("111".into(), "secret".into()).unwrap();
    assert!(!a.same_login(&b));
    assert!(a.same_login(&a.rotate("next".into()).unwrap()));
    assert_eq!(
        MiguCredential::parse_caller(&a.caller().unwrap()).unwrap(),
        a
    );
    assert!(!format!("{a:?}").contains("secret"));
    for input in [
        "",
        "pacmtoken=",
        "pacmtoken=null",
        "pacmtoken=undefined",
        "pacmtoken=a;pacmtoken=b",
        "pacmtoken=a\r\nx=b",
        "pacmtoken=a b",
        "pacmtoken=\"a\"",
        "pacmtoken=中文",
        "uid=111",
    ] {
        assert!(import_cookie(input).is_err(), "{input:?}");
    }
    assert!(import_cookie(&format!("pacmtoken={}", "x".repeat(16_385))).is_err());
    let value = serde_json::to_value(&a).unwrap();
    for (key, replacement) in [
        ("version", json!(2)),
        ("generation", json!("bad")),
        ("user_id", json!("")),
        ("user_id", json!("a/b")),
        ("pacm", json!("bad\n")),
        ("unexpected", json!(true)),
    ] {
        let mut bad = value.clone();
        bad[key] = replacement;
        assert!(MiguCredential::parse(&bad.to_string()).is_err());
    }
    for (platform, kind, expiry) in [
        (Platform::Soda, KIND, None),
        (Platform::Migu, "wrong", None),
        (Platform::Migu, KIND, Some(u64::MAX)),
    ] {
        assert!(
            MiguCredential::parse_caller(
                &ProviderCredential::new(platform, kind, a.serialize().unwrap(), expiry).unwrap()
            )
            .is_err()
        );
    }
}

#[tokio::test]
async fn later_failure_retains_only_the_latest_identity_bound_rotation() {
    let old = MiguCredential::verified("111".into(), "old".into()).unwrap();
    let malformed = [
        json!({"code":"000000"}),
        json!({"code":"000000","data":{}}),
        json!({"code":"000000","data":{"userId":"0"}}),
        json!({"code":"299999","data":{"userId":"111"},"info":"secret upstream message"}),
        json!({"code":"000000","data":{"userId":"111","nickName":"bad\n"}}),
    ];
    for (index, body) in malformed.into_iter().enumerate() {
        let (mut provider, requests) = server(vec![
            check("check-rotated"),
            reply(body, "pacmtoken: response-rotated\r\n"),
        ])
        .await;
        let store = Arc::new(Store::default());
        store.put(&stored("A", &old)).unwrap();
        provider.credential_store = Some(store.clone());
        let mut err = provider
            .refresh_session_with_ownership("A", None, CredentialMode::Both)
            .await
            .unwrap_err();
        assert_eq!(err.code, ErrorCode::UpstreamError);
        assert!(!format!("{err:?}").contains("secret upstream"));
        let expected = old
            .rotate(
                if index == 4 {
                    "response-rotated"
                } else {
                    "check-rotated"
                }
                .into(),
            )
            .unwrap();
        assert_eq!(read(&store, "A"), expected);
        assert_eq!(
            MiguCredential::parse_caller(&err.take_caller_credential_update().unwrap()).unwrap(),
            expected
        );
        assert!(err.take_caller_credential_update().is_none());
        assert!(provider.take_response_credential().unwrap().is_none());
        requests.await.unwrap();
    }
    let (provider, requests) =
        server(vec![profile("222", "pacmtoken: wrong-user-token\r\n")]).await;
    let caller = provider.caller_scope(&old.caller().unwrap()).unwrap();
    assert!(
        !caller
            .session_profile("default")
            .await
            .unwrap()
            .authenticated
    );
    assert!(caller.take_response_credential().unwrap().is_none());
    assert_eq!(
        *caller.caller_credential.as_ref().unwrap().lock().unwrap(),
        old
    );
    requests.await.unwrap();
}

#[tokio::test]
async fn caller_reads_deliver_only_latest_verified_rotation_and_clear_it_on_auth_failure() {
    let (provider, requests) = server(vec![
        profile("111", "pacmtoken: first\r\n"),
        profile("111", "pacmtoken: latest\r\n"),
        reply(json!({"code":"290001"}), "pacmtoken: invalid\r\n"),
    ])
    .await;
    let source = MiguCredential::verified("111".into(), "initial".into()).unwrap();
    let scope = provider.caller_scope(&source.caller().unwrap()).unwrap();
    scope.session_profile("default").await.unwrap();
    scope.session_profile("default").await.unwrap();
    assert_eq!(
        MiguCredential::parse_caller(&scope.response_credential.lock().unwrap().clone().unwrap())
            .unwrap()
            .token(),
        "latest"
    );
    assert!(
        !scope
            .session_profile("default")
            .await
            .unwrap()
            .authenticated
    );
    assert!(scope.take_response_credential().unwrap().is_none());
    let requests = requests.await.unwrap();
    for (request, token) in requests.iter().zip(["initial", "first", "latest"]) {
        assert!(request.contains(&format!("pacmtoken: {token}\r\n")));
    }
}

#[tokio::test]
async fn failed_refresh_obeys_ownership_and_does_not_reuse_unverified_response_tokens() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        for (last, expected) in [
            (reply(json!({"code":"299999"}), "pacmtoken: unverified\r\n"), "checked"),
            ("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 1\r\nConnection: close\r\n\r\n{".into(), "checked"),
            (profile("111", "pacmtoken: conflict-a\r\npacmtoken: conflict-b\r\n"), "checked"),
            (reply(json!({"code":"000000","data":{"userId":"111","nickName":123}}), "pacmtoken: presented\r\n"), "presented"),
        ] {
            let old = MiguCredential::verified("111".into(), "original".into()).unwrap();
            let (mut provider, requests) = server(vec![check("checked"), last]).await;
            let store = Arc::new(Store::default());
            let account = if mode == CredentialMode::Client { "default" } else { "A" };
            store.put(&stored(account, &old)).unwrap();
            provider.credential_store = Some(store.clone());
            let source = old.caller().unwrap();
            let mut error = provider.refresh_session_with_ownership(account, (mode != CredentialMode::Server).then_some(&source), mode).await.unwrap_err();
            assert_eq!(error.code, ErrorCode::UpstreamError);
            let current = old.rotate(expected.into()).unwrap();
            assert_eq!(read(&store, account), if mode.persists_on_server() { current.clone() } else { old });
            let update = error.take_caller_credential_update();
            assert_eq!(update.is_some(), mode.returns_to_caller());
            if let Some(update) = update {
                assert_eq!(MiguCredential::parse_caller(&update).unwrap(), current);
            }
            assert!(provider.take_response_credential().unwrap().is_none());
            let requests = requests.await.unwrap();
            assert!(requests[1].contains("pacmtoken: checked\r\n"));
        }
    }
}

#[tokio::test]
async fn invalidated_refresh_removes_only_the_selected_server_snapshot_and_exports_nothing() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        for responses in [
            vec![reply(json!({"code":"850001"}), "pacmtoken: rejected\r\n")],
            vec![
                check("checked"),
                reply(json!({"code":"290001"}), "pacmtoken: rejected\r\n"),
            ],
            vec![
                check("checked"),
                profile("222", "pacmtoken: other-user\r\n"),
            ],
        ] {
            let old = MiguCredential::verified("111".into(), "original".into()).unwrap();
            let account = if mode == CredentialMode::Client {
                "default"
            } else {
                "A"
            };
            let (mut provider, requests) = server(responses).await;
            let store = Arc::new(Store::default());
            store.put(&stored(account, &old)).unwrap();
            store.put(&stored("untouched", &old)).unwrap();
            provider.credential_store = Some(store.clone());
            let source = old.caller().unwrap();
            let mut error = provider
                .refresh_session_with_ownership(
                    account,
                    (mode != CredentialMode::Server).then_some(&source),
                    mode,
                )
                .await
                .unwrap_err();
            assert_eq!(error.code, ErrorCode::AuthenticationRequired);
            assert!(error.take_caller_credential_update().is_none());
            assert_eq!(
                store.0.lock().unwrap().contains_key(account),
                !mode.persists_on_server()
            );
            assert_eq!(read(&store, "untouched"), old);
            assert!(provider.take_response_credential().unwrap().is_none());
            requests.await.unwrap();
        }
    }
}

#[tokio::test]
async fn failed_first_check_and_failed_import_never_issue_a_partial_login() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let account = if mode == CredentialMode::Client {
            "default"
        } else {
            "A"
        };
        for import in [true, false] {
            let old = MiguCredential::verified("111".into(), "original".into()).unwrap();
            let responses = if import {
                vec![
                    check("checked"),
                    reply(
                        json!({"code":"000000","data":{"userId":"111","nickName":123}}),
                        "pacmtoken: partial-login\r\n",
                    ),
                ]
            } else {
                vec![reply(json!({"code":"299999"}), "pacmtoken: unverified\r\n")]
            };
            let (mut provider, requests) = server(responses).await;
            let store = Arc::new(Store::default());
            store.put(&stored(account, &old)).unwrap();
            provider.credential_store = Some(store.clone());
            let source = old.caller().unwrap();
            let mut error = if import {
                provider
                    .import_credential(&request(account, "imported"), mode)
                    .await
            } else {
                provider
                    .refresh_session_with_ownership(
                        account,
                        (mode != CredentialMode::Server).then_some(&source),
                        mode,
                    )
                    .await
            }
            .unwrap_err();
            assert_eq!(error.code, ErrorCode::UpstreamError);
            assert!(error.take_caller_credential_update().is_none());
            assert_eq!(read(&store, account), old);
            assert!(provider.take_response_credential().unwrap().is_none());
            requests.await.unwrap();
        }
    }
}

#[tokio::test]
async fn both_failure_returns_checked_server_token_even_if_no_new_rotation_occurs() {
    let caller = MiguCredential::verified("111".into(), "caller-old".into()).unwrap();
    let latest = caller.rotate("server-latest".into()).unwrap();
    let (mut provider, requests) = server(vec![
        check("server-latest"),
        reply(json!({"code":"299999"}), ""),
    ])
    .await;
    let store = Arc::new(Store::default());
    store.put(&stored("A", &latest)).unwrap();
    provider.credential_store = Some(store.clone());
    let mut error = provider
        .refresh_session_with_ownership("A", Some(&caller.caller().unwrap()), CredentialMode::Both)
        .await
        .unwrap_err();
    assert_eq!(
        MiguCredential::parse_caller(&error.take_caller_credential_update().unwrap()).unwrap(),
        latest
    );
    assert_eq!(read(&store, "A"), latest);
    assert!(
        requests
            .await
            .unwrap()
            .iter()
            .all(|request| request.contains("pacmtoken: server-latest\r\n"))
    );
}

#[tokio::test]
async fn profile_presentation_failure_retains_verified_rotation_for_the_next_read() {
    for caller in [false, true] {
        let source = MiguCredential::verified("111".into(), "old".into()).unwrap();
        let (mut provider, requests) = server(vec![
            reply(
                json!({"code":"000000","data":{"userId":"111","nickName":123}}),
                "pacmtoken: next\r\n",
            ),
            profile("111", ""),
        ])
        .await;
        let store = Arc::new(Store::default());
        store.put(&stored("default", &source)).unwrap();
        provider.credential_store = Some(store.clone());
        let provider = if caller {
            provider.caller_scope(&source.caller().unwrap()).unwrap()
        } else {
            provider
        };
        assert_eq!(
            provider.session_profile("default").await.unwrap_err().code,
            ErrorCode::UpstreamError
        );
        let expected = source.rotate("next".into()).unwrap();
        if caller {
            assert_eq!(
                MiguCredential::parse_caller(
                    &provider.take_response_credential().unwrap().unwrap()
                )
                .unwrap(),
                expected
            );
            assert_eq!(read(&store, "default"), source);
        } else {
            assert_eq!(read(&store, "default"), expected);
            assert!(provider.take_response_credential().unwrap().is_none());
        }
        assert!(
            provider
                .session_profile("default")
                .await
                .unwrap()
                .authenticated
        );
        assert!(requests.await.unwrap()[1].contains("pacmtoken: next\r\n"));
    }
}

#[tokio::test]
async fn profile_timeout_keeps_check_rotation_in_each_ownership_mode() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let source = MiguCredential::verified("111".into(), "original".into()).unwrap();
        let account = if mode == CredentialMode::Client {
            "default"
        } else {
            "A"
        };
        let (mut provider, seen, release, task) =
            gated(vec![check("checked"), profile("111", "")]).await;
        provider.client = provider
            .client
            .with_session_test_timeout(std::time::Duration::from_millis(300));
        let store = Arc::new(Store::default());
        store.put(&stored(account, &source)).unwrap();
        provider.credential_store = Some(store.clone());
        let worker = tokio::spawn(async move {
            let caller = source.caller().unwrap();
            provider
                .refresh_session_with_ownership(
                    account,
                    (mode != CredentialMode::Server).then_some(&caller),
                    mode,
                )
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(3), seen)
            .await
            .unwrap()
            .unwrap();
        let mut error = worker.await.unwrap().unwrap_err();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        drop(release);
        assert_eq!(error.code, ErrorCode::UpstreamTimeout);
        let update = error.take_caller_credential_update();
        assert_eq!(update.is_some(), mode.returns_to_caller());
        if let Some(update) = update {
            assert_eq!(
                MiguCredential::parse_caller(&update).unwrap().token(),
                "checked"
            );
        }
        assert_eq!(
            read(&store, account).token(),
            if mode.persists_on_server() {
                "checked"
            } else {
                "original"
            }
        );
    }
}

#[tokio::test]
async fn late_refresh_failures_cannot_restore_removed_or_concurrently_rotated_sessions() {
    for change in ["login", "rotation", "logout"] {
        for auth_failure in [false, true] {
            let source = MiguCredential::verified("111".into(), "old".into()).unwrap();
            let (mut provider, seen, release, server) = gated(vec![
                check("checked"),
                reply(
                    json!({"code":if auth_failure {"290001"} else {"299999"}}),
                    "",
                ),
            ])
            .await;
            let store = Arc::new(Store::default());
            store.put(&stored("A", &source)).unwrap();
            provider.credential_store = Some(store.clone());
            let task = tokio::spawn(async move {
                provider
                    .refresh_session_with_ownership("A", None, CredentialMode::Both)
                    .await
            });
            tokio::time::timeout(std::time::Duration::from_secs(3), seen)
                .await
                .unwrap()
                .unwrap();
            assert_eq!(read(&store, "A").token(), "checked");
            let expected = match change {
                "login" => Some(MiguCredential::verified("111".into(), "checked".into()).unwrap()),
                "rotation" => Some(source.rotate("concurrent".into()).unwrap()),
                _ => None,
            };
            if let Some(expected) = &expected {
                store.put(&stored("A", expected)).unwrap();
            } else {
                store.remove(Platform::Migu, "A").unwrap();
            }
            release.send(()).unwrap();
            let mut error = task.await.unwrap().unwrap_err();
            assert_eq!(error.code, ErrorCode::Conflict);
            assert!(error.take_caller_credential_update().is_none());
            assert_eq!(
                store.0.lock().unwrap().contains_key("A"),
                expected.is_some()
            );
            if let Some(expected) = expected {
                assert_eq!(read(&store, "A"), expected);
            }
            server.await.unwrap();
        }
    }
}

#[tokio::test]
async fn scoped_accounts_reject_public_media_and_alias_fallback_before_network() {
    let provider = MiguProvider::new(MiguConfig::default()).unwrap();
    let source = MiguCredential::verified("111".into(), "initial".into()).unwrap();
    let scoped = provider.caller_scope(&source.caller().unwrap()).unwrap();
    assert!(scoped.session_profile("A").await.is_err());
    assert!(
        scoped
            .search(&SearchQuery::tracks("x", 1, 0))
            .await
            .is_err()
    );
    assert!(scoped.album("1", None).await.is_err());
    assert!(scoped.digital_album("1", None).await.is_err());
    assert!(
        scoped
            .album_tracks("1", &PageRequest::new(1, 0))
            .await
            .is_err()
    );
    assert!(
        scoped
            .digital_album_tracks("1", &PageRequest::new(1, 0))
            .await
            .is_err()
    );
    assert!(scoped.track("1", None).await.is_err());
    let track = Track::new(
        tuneweave_core::ResourceRef::new(Platform::Migu, "1").unwrap(),
        "Test",
    );
    assert!(
        scoped
            .stream(&track, &StreamRequest::default())
            .await
            .is_err()
    );
    assert!(
        scoped
            .download(&track, &StreamRequest::default())
            .await
            .is_err()
    );
    assert!(
        scoped
            .track_availability("1", &TrackAvailabilityRequest::default())
            .await
            .is_err()
    );
    assert!(
        scoped
            .lyrics_with_options("1", &LyricsRequest::default())
            .await
            .is_err()
    );
    assert!(scoped.lyrics("1", None).await.is_err());
    assert!(scoped.playlist("1", None).await.is_err());
    assert!(
        scoped
            .playlist_tracks("1", &PageRequest::new(1, 0))
            .await
            .is_err()
    );
    assert!(
        provider
            .refresh_session_with_ownership("default", None, CredentialMode::Client)
            .await
            .is_err()
    );
    assert!(
        provider
            .refresh_session_with_ownership(
                "default",
                Some(&source.caller().unwrap()),
                CredentialMode::Server
            )
            .await
            .is_err()
    );
    assert!(
        provider
            .import_credential(&request("A", "token"), CredentialMode::Client)
            .await
            .is_err()
    );
    assert!(
        provider
            .import_credential(&request("A", "token"), CredentialMode::Server)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn both_refresh_uses_latest_same_login_server_token_and_logout_preserves_other_accounts() {
    let initial = MiguCredential::verified("111".into(), "old".into()).unwrap();
    let latest = initial.rotate("server-latest".into()).unwrap();
    let other = MiguCredential::verified("222".into(), "other-account".into()).unwrap();
    let (mut provider, requests) = server(vec![
        check("checked"),
        profile("111", "pacmtoken: refreshed\r\n"),
        reply(
            json!({"code":"000000"}),
            "Set-Cookie: pacmtoken=; Max-Age=0\r\n",
        ),
    ])
    .await;
    let store = Arc::new(Store::default());
    store.put(&stored("A", &latest)).unwrap();
    store.put(&stored("B", &other)).unwrap();
    provider.credential_store = Some(store.clone());
    let result = provider
        .refresh_session_with_ownership("A", Some(&initial.caller().unwrap()), CredentialMode::Both)
        .await
        .unwrap();
    let caller = result.credential.unwrap();
    assert_eq!(
        MiguCredential::parse_caller(&caller).unwrap(),
        read(&store, "A")
    );
    assert!(result.profile.extensions["refreshed"].as_bool().unwrap());
    let result = provider
        .logout_with_ownership("A", Some(&caller), CredentialMode::Both)
        .await
        .unwrap();
    assert!(result.removed && result.caller_credential_discard_required);
    assert!(!store.0.lock().unwrap().contains_key("A"));
    assert_eq!(read(&store, "B"), other);
    let requests = requests.await.unwrap();
    assert!(requests[0].contains("pacmtoken: server-latest\r\n"));
    assert!(requests[2].starts_with("GET /mgateway/api/clearPacMtoken HTTP/1.1"));
    assert!(requests[2].contains("pacmtoken: refreshed\r\n"));
    assert!(!provider.logout("missing").await.unwrap());
}

#[tokio::test]
async fn client_refresh_and_logout_never_touch_a_server_alias_and_failures_preserve_it() {
    let source = MiguCredential::verified("111".into(), "client".into()).unwrap();
    let (mut provider, requests) = server(vec![
        check("new"),
        profile("111", ""),
        reply(json!({"code":"850001"}), ""),
    ])
    .await;
    let store = Arc::new(Store::default());
    store.put(&stored("default", &source)).unwrap();
    provider.credential_store = Some(store.clone());
    let result = provider
        .refresh_session_with_ownership(
            "default",
            Some(&source.caller().unwrap()),
            CredentialMode::Client,
        )
        .await
        .unwrap();
    assert_eq!(read(&store, "default"), source);
    let result = provider
        .logout_with_ownership(
            "default",
            result.credential.as_ref(),
            CredentialMode::Client,
        )
        .await
        .unwrap();
    assert!(!result.removed && result.caller_credential_discard_required);
    assert_eq!(read(&store, "default"), source);
    requests.await.unwrap();
    let (mut provider, requests) = server(vec![reply(json!({"code":"299999"}), "")]).await;
    provider.credential_store = Some(store.clone());
    assert!(provider.logout("default").await.is_err());
    assert_eq!(read(&store, "default"), source);
    requests.await.unwrap();
    let other = MiguCredential::verified("111".into(), "client".into()).unwrap();
    assert_eq!(
        provider
            .logout_with_ownership(
                "default",
                Some(&other.caller().unwrap()),
                CredentialMode::Both
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
}

#[tokio::test]
async fn invalid_rotations_and_transport_failures_are_distinct_and_never_save_tokens() {
    let source = MiguCredential::verified("111".into(), "old".into()).unwrap();
    for (headers, code) in [
        ("pacmtoken: a\r\npacmtoken: b\r\n", ErrorCode::UpstreamError),
        (
            "pacmtoken: a\r\nSet-Cookie: pacmtoken=b; Path=/\r\n",
            ErrorCode::UpstreamError,
        ),
        (
            "Set-Cookie: pacmtoken=x; Max-Age=0\r\n",
            ErrorCode::AuthenticationRequired,
        ),
        (
            "Set-Cookie: pacmtoken=x; Expires=Thu, 01 Jan 1970 00:00:00 GMT\r\n",
            ErrorCode::AuthenticationRequired,
        ),
        (
            "Set-Cookie: pacmtoken=x; Max-Age=bad\r\n",
            ErrorCode::UpstreamError,
        ),
    ] {
        let (provider, requests) = server(vec![profile("111", headers)]).await;
        let err = provider
            .client
            .account_profile("A", source.token(), Some(source.user_id()))
            .await
            .unwrap_err();
        assert_eq!(err.code, code);
        requests.await.unwrap();
    }
    let (provider,requests)=server(vec![profile("111","Set-Cookie: pacmtoken=foreign; Domain=other.test; Path=/\r\nSet-Cookie: pacmtoken=wrong-path; Path=/elsewhere\r\nSet-Cookie: pacmtoken=current; Domain=.migu.cn; Path=/; Max-Age=60; Expires=Thu, 01 Jan 1970 00:00:00 GMT\r\n")]).await;
    assert_eq!(
        provider
            .client
            .account_profile("A", source.token(), Some(source.user_id()))
            .await
            .unwrap()
            .token,
        "current"
    );
    requests.await.unwrap();
    for (reply, code) in [
        (
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n",
            ErrorCode::AuthenticationRequired,
        ),
        (
            "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\n\r\n",
            ErrorCode::PermissionDenied,
        ),
        (
            "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\n\r\n",
            ErrorCode::RateLimited,
        ),
        (
            "HTTP/1.1 302 Found\r\nLocation: https://other.test/\r\nContent-Length: 0\r\n\r\n",
            ErrorCode::UpstreamError,
        ),
        (
            "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\n\r\n{}",
            ErrorCode::UpstreamError,
        ),
        (
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 65537\r\n\r\n",
            ErrorCode::UpstreamError,
        ),
    ] {
        let (provider, requests) = server(vec![reply.into()]).await;
        assert_eq!(
            provider.client.check_pacm("old").await.unwrap_err().code,
            code
        );
        requests.await.unwrap();
    }
}

pub(in crate::provider) async fn gated(
    responses: Vec<String>,
) -> (
    MiguProvider,
    tokio::sync::oneshot::Receiver<()>,
    tokio::sync::oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let origin = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let (seen_tx, seen) = tokio::sync::oneshot::channel();
    let (release, release_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        let mut seen_tx = Some(seen_tx);
        let mut release_rx = Some(release_rx);
        let count = responses.len();
        for (index, response) in responses.into_iter().enumerate() {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            loop {
                let mut buf = [0; 1024];
                let n = socket.read(&mut buf).await.unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buf[..n]);
                assert!(bytes.len() < 65536);
                if let Some(end) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                    let headers = std::str::from_utf8(&bytes[..end]).unwrap();
                    let length = headers
                        .lines()
                        .find_map(|line| {
                            line.split_once(':')
                                .filter(|(key, _)| key.eq_ignore_ascii_case("content-length"))
                                .map(|(_, value)| value.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + length {
                        break;
                    }
                }
            }
            if index + 1 == count {
                seen_tx.take().unwrap().send(()).unwrap();
                release_rx.take().unwrap().await.unwrap();
            }
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        }
    });
    (
        MiguProvider::from_client(MiguClient::test_client().with_catalog_test_origin(origin)),
        seen,
        release,
        task,
    )
}

#[tokio::test]
async fn late_reads_refreshes_and_logouts_cannot_overwrite_or_export_a_replacement_login() {
    for operation in ["read", "read_failure", "refresh", "logout", "import"] {
        let source = MiguCredential::verified("111".into(), "old".into()).unwrap();
        // Even an identical PACM value must not make a new generation equal to the old one.
        let newer = MiguCredential::verified("111".into(), "old".into()).unwrap();
        let responses = match operation {
            "read" => vec![profile("111", "")],
            "read_failure" => vec![reply(json!({"code":"290001"}), "")],
            "refresh" | "import" => vec![check("checked"), profile("111", "pacmtoken: late\r\n")],
            _ => vec![reply(json!({"code":"000000"}), "")],
        };
        let (mut provider, seen, release, server) = gated(responses).await;
        let store = Arc::new(Store::default());
        store.put(&stored("A", &source)).unwrap();
        provider.credential_store = Some(store.clone());
        let provider = Arc::new(provider);
        let worker = provider.clone();
        let task = tokio::spawn(async move {
            match operation {
                "read" | "read_failure" => worker.session_profile("A").await.map(|_| ()),
                "refresh" => worker
                    .refresh_session_with_ownership("A", None, CredentialMode::Both)
                    .await
                    .map(|_| ()),
                "import" => worker
                    .import_credential(&request("A", "new-import"), CredentialMode::Both)
                    .await
                    .map(|_| ()),
                _ => worker.logout("A").await.map(|_| ()),
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(3), seen)
            .await
            .unwrap()
            .unwrap();
        store.put(&stored("A", &newer)).unwrap();
        release.send(()).unwrap();
        assert_eq!(
            task.await.unwrap().unwrap_err().code,
            ErrorCode::Conflict,
            "{operation}"
        );
        assert_eq!(read(&store, "A"), newer);
        assert!(provider.take_response_credential().unwrap().is_none());
        server.await.unwrap();
    }
    for replace in [true, false] {
        let source = MiguCredential::verified("111".into(), "old".into()).unwrap();
        let (provider, seen, release, server) =
            gated(vec![profile("111", "pacmtoken: late\r\n")]).await;
        let scope = Arc::new(provider.caller_scope(&source.caller().unwrap()).unwrap());
        let worker = scope.clone();
        let task = tokio::spawn(async move { worker.session_profile("default").await });
        tokio::time::timeout(std::time::Duration::from_secs(3), seen)
            .await
            .unwrap()
            .unwrap();
        let expected = if replace {
            MiguCredential::verified("111".into(), "replacement".into()).unwrap()
        } else {
            source.rotate("concurrent".into()).unwrap()
        };
        *scope.caller_credential.as_ref().unwrap().lock().unwrap() = expected.clone();
        release.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(
            *scope.caller_credential.as_ref().unwrap().lock().unwrap(),
            expected
        );
        assert!(scope.take_response_credential().unwrap().is_none());
        server.await.unwrap();
    }
}

#[tokio::test]
async fn password_login_verifies_exchange_identity_before_applying_ownership() {
    use crate::client::passport::tests as fixture;
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let account = if mode == CredentialMode::Client {
            "default"
        } else {
            "A"
        };
        let old = MiguCredential::verified("111".into(), "old".into()).unwrap();
        let (client, requests) = fixture::server(vec![
            fixture::key(0),
            fixture::login(),
            fixture::exchange("222"),
            profile("222", "pacmtoken: final-music\r\n"),
        ])
        .await;
        let mut provider = MiguProvider::from_client(client);
        let store = Arc::new(Store::default());
        store.put(&stored(account, &old)).unwrap();
        store.put(&stored("untouched", &old)).unwrap();
        provider.credential_store = Some(store.clone());
        let result = provider
            .password_login_with_mode(&fixture::request(account), mode)
            .await
            .unwrap();
        assert!(result.profile.authenticated);
        assert_eq!(result.profile.user_id.as_deref(), Some("222"));
        assert_eq!(result.credential.is_some(), mode.returns_to_caller());
        assert_eq!(read(&store, "untouched"), old);
        let next = if mode.persists_on_server() {
            read(&store, account)
        } else {
            assert_eq!(read(&store, account), old);
            MiguCredential::parse_caller(result.credential.as_ref().unwrap()).unwrap()
        };
        assert!(!next.same_login(&old));
        assert_eq!(next.token(), "final-music");
        if let Some(credential) = result.credential {
            assert_eq!(MiguCredential::parse_caller(&credential).unwrap(), next);
        }
        let profile_json = serde_json::to_string(&result.profile).unwrap();
        for secret in [
            "keep-private",
            "music-token",
            "final-music",
            "passport-only",
            "discard-phone",
            "discard-usession",
        ] {
            assert!(!profile_json.contains(secret));
        }
        let requests = requests.await.unwrap();
        assert_eq!(requests.len(), 4);
        assert!(requests[3].contains("pacmtoken: music-token\r\n"));
        assert!(!requests[3].contains("passport-only"));
    }
}

#[tokio::test]
async fn failed_password_login_never_overwrites_or_issues_an_unverified_account() {
    use crate::client::passport::tests as fixture;
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        for (last, code) in [
            (
                profile("333", "pacmtoken: wrong-user\r\n"),
                ErrorCode::AuthenticationRequired,
            ),
            (
                reply(
                    json!({"code":"000000","data":{"userId":"222","nickName":123}}),
                    "pacmtoken: partial\r\n",
                ),
                ErrorCode::UpstreamError,
            ),
            (
                reply(json!({"code":"290001"}), ""),
                ErrorCode::AuthenticationRequired,
            ),
        ] {
            let account = if mode == CredentialMode::Client {
                "default"
            } else {
                "A"
            };
            let old = MiguCredential::verified("111".into(), "old".into()).unwrap();
            let (client, requests) = fixture::server(vec![
                fixture::key(0),
                fixture::login(),
                fixture::exchange("222"),
                last,
            ])
            .await;
            let mut provider = MiguProvider::from_client(client);
            let store = Arc::new(Store::default());
            store.put(&stored(account, &old)).unwrap();
            provider.credential_store = Some(store.clone());
            let mut error = provider
                .password_login_with_mode(&fixture::request(account), mode)
                .await
                .unwrap_err();
            assert_eq!(error.code, code);
            assert!(error.take_caller_credential_update().is_none());
            assert_eq!(read(&store, account), old);
            assert!(provider.take_response_credential().unwrap().is_none());
            requests.await.unwrap();
        }
    }
}

#[tokio::test]
async fn password_input_validation_and_late_completion_protect_existing_sessions() {
    use crate::client::passport::tests as fixture;
    let provider = MiguProvider::from_client(
        MiguClient::test_client()
            .with_catalog_test_origin(url::Url::parse("http://127.0.0.1:9/").unwrap()),
    );
    for field in [
        "alias", "empty", "hashed", "captcha", "country", "control", "phone",
    ] {
        let mut request = fixture::request("default");
        match field {
            "alias" => request.account = "A".into(),
            "empty" => request.password.clear(),
            "hashed" => request.password_format = tuneweave_core::PasswordFormat::Md5,
            "captcha" => request.secure_captcha = Some("{}".into()),
            "country" => request.country_code = Some("1".into()),
            "control" => request.principal = "name\n".into(),
            _ => request.principal_type = tuneweave_core::PrincipalType::Phone,
        }
        assert_eq!(
            provider
                .password_login_with_mode(&request, CredentialMode::Client)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    let (mut provider, seen, release, server) = gated(vec![
        fixture::key(0),
        fixture::login(),
        fixture::exchange("222"),
        profile("222", ""),
    ])
    .await;
    let old = MiguCredential::verified("111".into(), "old".into()).unwrap();
    let newer = MiguCredential::verified("333".into(), "newer".into()).unwrap();
    let store = Arc::new(Store::default());
    store.put(&stored("A", &old)).unwrap();
    provider.credential_store = Some(store.clone());
    let task = tokio::spawn(async move {
        provider
            .password_login_with_mode(&fixture::request("A"), CredentialMode::Both)
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(3), seen)
        .await
        .unwrap()
        .unwrap();
    store.put(&stored("A", &newer)).unwrap();
    release.send(()).unwrap();
    let mut error = task.await.unwrap().unwrap_err();
    assert_eq!(error.code, ErrorCode::Conflict);
    assert!(error.take_caller_credential_update().is_none());
    assert_eq!(read(&store, "A"), newer);
    server.await.unwrap();
}

#[tokio::test]
async fn password_timeout_at_every_stage_preserves_the_previous_account() {
    use crate::client::passport::tests as fixture;
    for stage in 1..=4 {
        let replies = [
            fixture::key(0),
            fixture::login(),
            fixture::exchange("222"),
            profile("222", ""),
        ];
        let (mut provider, seen, release, server) = gated(replies[..stage].to_vec()).await;
        provider.client = provider
            .client
            .with_session_test_timeout(std::time::Duration::from_millis(300));
        let old = MiguCredential::verified("111".into(), "previous".into()).unwrap();
        let store = Arc::new(Store::default());
        store.put(&stored("A", &old)).unwrap();
        provider.credential_store = Some(store.clone());
        let task = tokio::spawn(async move {
            provider
                .password_login_with_mode(&fixture::request("A"), CredentialMode::Both)
                .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(3), seen)
            .await
            .unwrap()
            .unwrap();
        let mut error = task.await.unwrap().unwrap_err();
        server.abort();
        assert!(server.await.unwrap_err().is_cancelled());
        drop(release);
        assert_eq!(error.code, ErrorCode::UpstreamTimeout, "stage {stage}");
        assert!(error.take_caller_credential_update().is_none());
        assert_eq!(read(&store, "A"), old);
    }
}
