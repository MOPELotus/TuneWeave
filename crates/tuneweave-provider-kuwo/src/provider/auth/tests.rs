use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests::{self as fixture, credential_fixture, encrypted},
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::Notify;
use tuneweave_core::{AccountCredentialStore, UserProfileBackend};

#[derive(Default)]
pub(super) struct Store {
    pub(super) values: Mutex<BTreeMap<String, StoredAccountCredential>>,
    pub(super) reads: AtomicUsize,
    pub(super) forbid_reads: AtomicBool,
    pub(super) reject_write: AtomicBool,
    pub(super) fail_write: AtomicBool,
}
impl AccountCredentialStore for Store {
    fn load_platform(&self, platform: Platform) -> Result<Vec<StoredAccountCredential>> {
        assert_eq!(platform, Platform::Kuwo);
        self.reads.fetch_add(1, Ordering::SeqCst);
        if self.forbid_reads.load(Ordering::SeqCst) {
            return Err(state_error());
        }
        Ok(self.values.lock().unwrap().values().cloned().collect())
    }
    fn put(&self, value: &StoredAccountCredential) -> Result<()> {
        self.values
            .lock()
            .unwrap()
            .insert(value.account.clone(), value.clone());
        Ok(())
    }
    fn remove(&self, platform: Platform, account: &str) -> Result<bool> {
        assert_eq!(platform, Platform::Kuwo);
        Ok(self.values.lock().unwrap().remove(account).is_some())
    }
    fn insert_if_absent(&self, value: &StoredAccountCredential) -> Result<bool> {
        if self.fail_write.load(Ordering::SeqCst) {
            return Err(state_error());
        }
        let mut values = self.values.lock().unwrap();
        if self.reject_write.load(Ordering::SeqCst) || values.contains_key(&value.account) {
            return Ok(false);
        }
        values.insert(value.account.clone(), value.clone());
        Ok(true)
    }
    fn compare_exchange(
        &self,
        old: &StoredAccountCredential,
        new: Option<&StoredAccountCredential>,
    ) -> Result<bool> {
        if self.fail_write.load(Ordering::SeqCst) {
            return Err(state_error());
        }
        let mut values = self.values.lock().unwrap();
        if self.reject_write.load(Ordering::SeqCst) || values.get(&old.account) != Some(old) {
            return Ok(false);
        }
        if let Some(new) = new {
            values.insert(new.account.clone(), new.clone());
        } else {
            values.remove(&old.account);
        }
        Ok(true)
    }
}
pub(super) struct Fixture {
    pub(super) network: fixture::Fixture,
    pub(super) provider: KuwoProvider,
    pub(super) store: Arc<Store>,
}
pub(super) async fn setup(
    replies: Vec<(Vec<u8>, Option<Arc<Notify>>)>,
    store: Arc<Store>,
) -> Fixture {
    let network = fixture::setup_gated(replies).await;
    let mut provider = KuwoProvider::from_client(network.client.clone());
    provider.credential_store = Some(store.clone());
    Fixture {
        network,
        provider,
        store,
    }
}
pub(super) fn request(account: &str) -> PasswordLoginRequest {
    PasswordLoginRequest {
        backend: Default::default(),
        account: account.into(),
        principal_type: tuneweave_core::PrincipalType::Username,
        principal: "synthetic-user".into(),
        password: "synthetic-password".into(),
        password_format: tuneweave_core::PasswordFormat::Plain,
        country_code: None,
        secure_captcha: None,
    }
}
pub(super) fn device() -> Vec<u8> {
    json_response(&json!({"code":200,"success":true,"data":{"appuid":"1234567890"}}))
}
pub(super) fn login(uid: u64, sid: &str) -> Vec<u8> {
    encrypted(&json!({"result":"succ","sid":sid,"userInfo":{"uid":uid,"nickName":"Listener"}}))
}
pub(super) fn valid() -> Vec<u8> {
    json_response(&json!({"result":"ok"}))
}
pub(super) fn replies(bodies: Vec<Vec<u8>>) -> Vec<(Vec<u8>, Option<Arc<Notify>>)> {
    bodies.into_iter().map(|v| (v, None)).collect()
}
pub(super) fn stored(store: &Store, account: &str) -> Option<StoredAccountCredential> {
    store.values.lock().unwrap().get(account).cloned()
}
pub(super) fn seed(store: &Store, account: &str, uid: &str, sid: &str) -> NativeCredential {
    let value = credential_fixture(uid, sid);
    store.put(&value.stored(account).unwrap()).unwrap();
    value
}
pub(super) async fn received(fixture: &mut Fixture) {
    tokio::time::timeout(Duration::from_secs(3), fixture.network.seen.recv())
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn password_login_honors_all_ownership_modes_and_returns_only_committed_credentials() {
    for (kind, principal) in [
        (tuneweave_core::PrincipalType::Username, "synthetic-user"),
        (tuneweave_core::PrincipalType::Phone, "13800138000"),
        (
            tuneweave_core::PrincipalType::Email,
            "Listener+Tag@Example.invalid",
        ),
    ] {
        for mode in [
            CredentialMode::Server,
            CredentialMode::Client,
            CredentialMode::Both,
        ] {
            let account = if mode == CredentialMode::Client {
                "default"
            } else {
                "personal"
            };
            let store = Arc::new(Store::default());
            let unrelated = seed(&store, "other", "7", "other-session")
                .stored("other")
                .unwrap();
            if mode == CredentialMode::Client {
                store.forbid_reads.store(true, Ordering::SeqCst);
            }
            let mut fixture = setup(
                replies(vec![device(), login(42, "new-session"), valid()]),
                store,
            )
            .await;
            let mut input = request(account);
            input.principal_type = kind;
            input.principal = principal.into();
            if kind == tuneweave_core::PrincipalType::Phone {
                input.country_code = Some("+86".into());
            }
            let result = fixture
                .provider
                .password_login_with_mode(&input, mode)
                .await
                .unwrap();
            assert_eq!(result.profile.account, account);
            assert_eq!(result.profile.user_id.as_deref(), Some("42"));
            assert!(result.profile.authenticated);
            assert_eq!(result.credential.is_some(), mode.returns_to_caller());
            assert_eq!(
                stored(&fixture.store, account).is_some(),
                mode.persists_on_server()
            );
            if mode == CredentialMode::Both {
                assert_eq!(
                    stored(&fixture.store, account).unwrap().secret(),
                    result.credential.as_ref().unwrap().secret()
                );
            }
            if mode == CredentialMode::Client {
                assert_eq!(fixture.store.reads.load(Ordering::SeqCst), 0);
            }
            assert_eq!(stored(&fixture.store, "other"), Some(unrelated));
            assert!(
                fixture
                    .provider
                    .auth_registry
                    .lock()
                    .unwrap()
                    .attempts
                    .is_empty()
            );
            fixture::requests(&mut fixture.network, 3).await;
        }
    }
}

#[tokio::test]
async fn refresh_preserves_login_generation_and_checks_replacement_sid_before_publishing() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let store = Arc::new(Store::default());
        let account = if mode == CredentialMode::Client {
            "default"
        } else {
            "personal"
        };
        let original = if mode.persists_on_server() {
            seed(&store, account, "42", "old-session")
        } else {
            credential_fixture("42", "old-session")
        };
        let source = original.caller().unwrap();
        if mode == CredentialMode::Client {
            store.forbid_reads.store(true, Ordering::SeqCst);
        }
        let mut fixture = setup(replies(vec![login(42, "rotated-session"), valid()]), store).await;
        let result = fixture
            .provider
            .refresh_session_with_ownership(
                account,
                (mode != CredentialMode::Server).then_some(&source),
                mode,
            )
            .await
            .unwrap();
        let next = if mode.returns_to_caller() {
            NativeCredential::parse(result.credential.as_ref().unwrap()).unwrap()
        } else {
            NativeCredential::parse_stored(&stored(&fixture.store, account).unwrap()).unwrap()
        };
        assert!(original.same_login(&next));
        assert_eq!(next.input().unwrap().session_id(), "rotated-session");
        if mode == CredentialMode::Both {
            assert_eq!(
                result.credential.unwrap().secret(),
                stored(&fixture.store, account).unwrap().secret()
            );
        }
        if mode == CredentialMode::Client {
            assert_eq!(fixture.store.reads.load(Ordering::SeqCst), 0);
        }
        let seen = fixture::requests(&mut fixture.network, 2).await;
        assert!(seen[1].contains("sid=rotated-session"));
    }
}

#[tokio::test]
async fn failed_login_and_refresh_never_replace_the_prior_alias_or_export_a_credential() {
    for failure_at in 0..3 {
        let store = Arc::new(Store::default());
        let original = seed(&store, "personal", "42", "old-session")
            .stored("personal")
            .unwrap();
        let mut bodies = vec![device(), login(42, "candidate-session"), valid()];
        bodies[failure_at] = response(503, "application/json", "", b"{}");
        bodies.truncate(failure_at + 1);
        let mut fixture = setup(replies(bodies), store).await;
        let mut error = fixture
            .provider
            .password_login_with_mode(&request("personal"), CredentialMode::Both)
            .await
            .unwrap_err();
        assert!(error.take_caller_credential_update().is_none());
        assert_eq!(stored(&fixture.store, "personal"), Some(original));
        assert!(
            fixture
                .provider
                .auth_registry
                .lock()
                .unwrap()
                .attempts
                .is_empty()
        );
        fixture::requests(&mut fixture.network, failure_at + 1).await;
    }
    let store = Arc::new(Store::default());
    let original = seed(&store, "personal", "42", "old-session");
    let mut fixture = setup(
        replies(vec![
            login(42, "candidate-session"),
            json_response(&json!({"result":"fail","reason":"error_user_invalid"})),
        ]),
        store,
    )
    .await;
    let mut error = fixture
        .provider
        .refresh_session_with_ownership(
            "personal",
            Some(&original.caller().unwrap()),
            CredentialMode::Both,
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    assert!(error.take_caller_credential_update().is_none());
    assert_eq!(
        stored(&fixture.store, "personal"),
        Some(original.stored("personal").unwrap())
    );
    fixture::requests(&mut fixture.network, 2).await;
}

#[tokio::test]
async fn failed_conditional_persistence_never_delivers_a_success_or_overwrites_existing_state() {
    for existing in [false, true] {
        for fail in [false, true] {
            let store = Arc::new(Store::default());
            if existing {
                seed(&store, "personal", "42", "old-session");
            }
            let before = stored(&store, "personal");
            store.reject_write.store(!fail, Ordering::SeqCst);
            store.fail_write.store(fail, Ordering::SeqCst);
            let mut fixture = setup(
                replies(vec![device(), login(42, "new-session"), valid()]),
                store,
            )
            .await;
            let mut error = fixture
                .provider
                .password_login_with_mode(&request("personal"), CredentialMode::Both)
                .await
                .unwrap_err();
            assert_eq!(
                error.code,
                if fail {
                    ErrorCode::InternalError
                } else {
                    ErrorCode::Conflict
                }
            );
            assert!(error.take_caller_credential_update().is_none());
            assert_eq!(stored(&fixture.store, "personal"), before);
            fixture::requests(&mut fixture.network, 3).await;
        }
    }
}

#[tokio::test]
async fn every_login_boundary_rejects_replacement_logout_expiry_and_late_errors() {
    for boundary in 0..3 {
        for action in ["replace", "logout", "expire"] {
            for late_error in [false, true] {
                let store = Arc::new(Store::default());
                seed(&store, "personal", "42", "old-session");
                let gate = Arc::new(Notify::new());
                let mut bodies = vec![device(), login(42, "candidate-session"), valid()];
                if late_error {
                    bodies[boundary] = response(503, "application/json", "", b"{}");
                }
                bodies.truncate(boundary + 1);
                let mut responses = replies(bodies);
                responses[boundary].1 = Some(gate.clone());
                let mut fixture = setup(responses, store).await;
                let provider = fixture.provider.clone();
                let task = tokio::spawn(async move {
                    provider
                        .password_login_with_mode(&request("personal"), CredentialMode::Both)
                        .await
                });
                for _ in 0..=boundary {
                    received(&mut fixture).await;
                }
                match action {
                    "replace" => {
                        seed(&fixture.store, "personal", "43", "replacement-session");
                    }
                    "logout" => {
                        assert!(fixture.provider.logout("personal").await.unwrap());
                    }
                    _ => {
                        for attempt in fixture
                            .provider
                            .auth_registry
                            .lock()
                            .unwrap()
                            .attempts
                            .values_mut()
                        {
                            attempt.deadline = Instant::now() - Duration::from_secs(1);
                        }
                    }
                }
                let expected = stored(&fixture.store, "personal");
                gate.notify_one();
                let mut error = task.await.unwrap().unwrap_err();
                assert_eq!(error.code, ErrorCode::Conflict);
                assert!(error.take_caller_credential_update().is_none());
                assert_eq!(stored(&fixture.store, "personal"), expected);
                (&mut fixture.network.server).await.unwrap();
                assert!(fixture.network.seen.try_recv().is_err());
            }
        }
    }
}

#[tokio::test]
async fn concurrent_provider_instances_cannot_both_publish_the_same_first_alias() {
    for existing in [false, true] {
        let store = Arc::new(Store::default());
        if existing {
            seed(&store, "personal", "41", "prior-session");
        }
        let a = setup(
            replies(vec![device(), login(42, "session-a"), valid()]),
            store.clone(),
        )
        .await;
        let b = setup(
            replies(vec![device(), login(43, "session-b"), valid()]),
            store.clone(),
        )
        .await;
        let request = request("personal");
        let (a, b) = tokio::join!(
            a.provider
                .password_login_with_mode(&request, CredentialMode::Both),
            b.provider
                .password_login_with_mode(&request, CredentialMode::Both)
        );
        let (winner, loser) = match (a, b) {
            (Ok(winner), Err(loser)) | (Err(loser), Ok(winner)) => (winner, loser),
            _ => panic!("exactly one concurrent login must publish"),
        };
        assert_eq!(loser.code, ErrorCode::Conflict);
        assert_eq!(
            winner.credential.unwrap().secret(),
            stored(&store, "personal").unwrap().secret()
        );
    }
}

#[tokio::test]
async fn caller_identity_is_separate_from_server_aliases_and_public_music_operations() {
    let store = Arc::new(Store::default());
    seed(&store, "default", "43", "server-session");
    store.forbid_reads.store(true, Ordering::SeqCst);
    let caller = credential_fixture("42", "caller-session").caller().unwrap();
    let mut fixture = setup(
        replies(vec![
            valid(),
            valid(),
            crate::client::native::profile::tests::reply("42"),
        ]),
        store,
    )
    .await;
    let scoped = fixture.provider.caller_scope(&caller).unwrap();
    let profile = scoped.session_profile("default").await.unwrap();
    assert_eq!(profile.user_id.as_deref(), Some("42"));
    assert!(profile.nickname.is_none());
    assert_eq!(
        scoped.session_profile("personal").await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        scoped
            .user_profile("43", UserProfileBackend::Modern, None)
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    let user = scoped
        .user_profile("42", UserProfileBackend::Modern, None)
        .await
        .unwrap();
    assert_eq!(user.user.id, "42");
    assert_eq!(user.user.name, "听众 + %20");
    macro_rules! blocked {
        ($future:expr) => {
            assert_eq!(
                $future.await.unwrap_err().code,
                ErrorCode::CapabilityNotSupported
            )
        };
    }
    blocked!(scoped.search(&SearchQuery::tracks("song", 1, 0)));
    blocked!(scoped.search_catalog(&SearchQuery::tracks("song", 1, 0)));
    blocked!(scoped.lyrics("123", None));
    // Selected-account ordinary created playlists are now covered by
    // auth::playlist tests; the remaining public operations still reject callers.
    blocked!(scoped.artist("123", None));
    blocked!(scoped.artist_overview("123", None));
    blocked!(scoped.album("123", None));
    blocked!(scoped.album_tracks("123", &PageRequest::new(1, 0)));
    blocked!(scoped.artist_tracks("123", &tuneweave_core::ArtistTrackListRequest::new(1, 0)));
    blocked!(scoped.artist_albums("123", &PageRequest::new(1, 0)));
    blocked!(scoped.artist_videos("123", &tuneweave_core::ArtistVideoListRequest::new(1, 0)));
    let detail = tuneweave_core::VideoDetailRequest::new(tuneweave_core::VideoResourceKind::Mv);
    blocked!(scoped.video("123", &detail));
    blocked!(scoped.video_stats("123", &detail));
    let stream =
        tuneweave_core::VideoStreamRequest::new(tuneweave_core::VideoResourceKind::Mv, 1080);
    blocked!(scoped.video_stream("123", &stream));
    blocked!(scoped.video_streams(&["123".into()], &stream));
    blocked!(scoped.lyrics_with_options("123", &LyricsRequest::default()));
    // Account media and catalogue ownership are covered by auth::media tests.
    assert_eq!(fixture.store.reads.load(Ordering::SeqCst), 0);
    fixture::requests(&mut fixture.network, 3).await;
}

#[tokio::test]
async fn invalid_session_removes_only_the_unchanged_selected_source() {
    let store = Arc::new(Store::default());
    seed(&store, "personal", "42", "invalid-session");
    let other = seed(&store, "other", "7", "other-session")
        .stored("other")
        .unwrap();
    let mut fixture = setup(
        replies(vec![
            json_response(&json!({"result":"fail","reason":"error_user_invalid"})),
            json_response(&json!({"result":"fail","reason":"error_user_invalid"})),
        ]),
        store,
    )
    .await;
    assert_eq!(
        fixture
            .provider
            .session_profile("personal")
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert!(stored(&fixture.store, "personal").is_none());
    assert_eq!(stored(&fixture.store, "other"), Some(other));
    let scoped = fixture
        .provider
        .caller_scope(&credential_fixture("43", "caller-session").caller().unwrap())
        .unwrap();
    assert_eq!(
        scoped.session_profile("default").await.unwrap_err().code,
        ErrorCode::AuthenticationRequired
    );
    assert!(
        scoped
            .caller_credential
            .as_ref()
            .unwrap()
            .lock()
            .unwrap()
            .is_none()
    );
    assert_eq!(
        scoped.session_profile("default").await.unwrap_err().code,
        ErrorCode::AuthenticationRequired
    );
    fixture::requests(&mut fixture.network, 2).await;
}

#[tokio::test]
async fn logout_honors_ownership_and_refuses_other_login_generations() {
    let store = Arc::new(Store::default());
    let original = seed(&store, "personal", "42", "session");
    let mut fixture = setup(vec![], store).await;
    let source = original.caller().unwrap();
    let foreign = credential_fixture("42", "session").caller().unwrap();
    assert_eq!(
        fixture
            .provider
            .logout_with_ownership("personal", Some(&foreign), CredentialMode::Both)
            .await
            .unwrap_err()
            .code,
        ErrorCode::Conflict
    );
    let client = fixture
        .provider
        .logout_with_ownership("default", Some(&source), CredentialMode::Client)
        .await
        .unwrap();
    assert!(client.removed && client.caller_credential_discard_required);
    assert!(stored(&fixture.store, "personal").is_some());
    let both = fixture
        .provider
        .logout_with_ownership("personal", Some(&source), CredentialMode::Both)
        .await
        .unwrap();
    assert!(both.removed && both.caller_credential_discard_required);
    assert!(stored(&fixture.store, "personal").is_none());
    assert!(!fixture.provider.logout("personal").await.unwrap());
    fixture::requests(&mut fixture.network, 0).await;
}

#[tokio::test]
async fn invalid_modes_inputs_missing_store_and_capacity_stop_before_network() {
    let store = Arc::new(Store::default());
    let mut fixture = setup(vec![], store).await;
    assert!(
        fixture
            .provider
            .password_login_with_mode(&request("personal"), CredentialMode::Client)
            .await
            .is_err()
    );
    let mut invalid = request("default");
    invalid.password_format = tuneweave_core::PasswordFormat::Md5;
    assert!(
        fixture
            .provider
            .password_login_with_mode(&invalid, CredentialMode::Both)
            .await
            .is_err()
    );
    let caller = credential_fixture("42", "session").caller().unwrap();
    let scoped = fixture.provider.caller_scope(&caller).unwrap();
    assert!(
        scoped
            .password_login_with_mode(&request("default"), CredentialMode::Client)
            .await
            .is_err()
    );
    assert!(
        fixture
            .provider
            .refresh_session_with_ownership("default", Some(&caller), CredentialMode::Server)
            .await
            .is_err()
    );
    assert!(
        fixture
            .provider
            .refresh_session_with_ownership("default", None, CredentialMode::Client)
            .await
            .is_err()
    );
    let mut no_store = fixture.provider.clone();
    no_store.credential_store = None;
    assert!(
        no_store
            .password_login_with_mode(&request("personal"), CredentialMode::Server)
            .await
            .is_err()
    );
    let leases: Vec<_> = (0..CAPACITY)
        .map(|_| {
            fixture
                .provider
                .reserve("default", CredentialMode::Client)
                .unwrap()
                .0
        })
        .collect();
    assert_eq!(
        fixture
            .provider
            .password_login_with_mode(&request("default"), CredentialMode::Client)
            .await
            .unwrap_err()
            .code,
        ErrorCode::RateLimited
    );
    drop(leases);
    assert!(
        fixture
            .provider
            .auth_registry
            .lock()
            .unwrap()
            .attempts
            .is_empty()
    );
    fixture::requests(&mut fixture.network, 0).await;
}

#[tokio::test]
async fn refresh_and_identity_reads_reject_late_success_or_error_after_source_changes() {
    for kind in ["refresh-first", "refresh-validate", "read"] {
        for action in ["replace", "logout"] {
            for late_error in [false, true] {
                let store = Arc::new(Store::default());
                let original = seed(&store, "personal", "42", "old-session");
                let boundary = usize::from(kind == "refresh-validate");
                let gate = Arc::new(Notify::new());
                let mut bodies = if kind == "read" {
                    vec![valid()]
                } else {
                    vec![login(42, "candidate-session"), valid()]
                };
                if late_error {
                    bodies[boundary] =
                        json_response(&json!({"result":"fail","reason":"error_user_invalid"}));
                }
                bodies.truncate(boundary + 1);
                let mut responses = replies(bodies);
                responses[boundary].1 = Some(gate.clone());
                let mut fixture = setup(responses, store).await;
                let provider = fixture.provider.clone();
                let source = original.caller().unwrap();
                let task = tokio::spawn(async move {
                    if kind == "read" {
                        provider.session_profile("personal").await.map(|_| ())
                    } else {
                        provider
                            .refresh_session_with_ownership(
                                "personal",
                                Some(&source),
                                CredentialMode::Both,
                            )
                            .await
                            .map(|_| ())
                    }
                });
                for _ in 0..=boundary {
                    received(&mut fixture).await;
                }
                if action == "replace" {
                    seed(&fixture.store, "personal", "43", "replacement-session");
                } else {
                    fixture.provider.logout("personal").await.unwrap();
                }
                let expected = stored(&fixture.store, "personal");
                gate.notify_one();
                let mut error = task.await.unwrap().unwrap_err();
                assert_eq!(error.code, ErrorCode::Conflict);
                assert!(error.take_caller_credential_update().is_none());
                assert_eq!(stored(&fixture.store, "personal"), expected);
                (&mut fixture.network.server).await.unwrap();
                assert!(fixture.network.seen.try_recv().is_err());
            }
        }
    }
}

#[tokio::test]
async fn cancellation_retires_login_and_refresh_leases_at_each_network_boundary() {
    for refresh in [false, true] {
        for boundary in 0..if refresh { 2 } else { 3 } {
            let store = Arc::new(Store::default());
            let original = seed(&store, "personal", "42", "old-session");
            let gate = Arc::new(Notify::new());
            let mut bodies = if refresh {
                vec![login(42, "candidate-session"), valid()]
            } else {
                vec![device(), login(42, "candidate-session"), valid()]
            };
            bodies.truncate(boundary + 1);
            let mut responses = replies(bodies);
            responses[boundary].1 = Some(gate.clone());
            let mut fixture = setup(responses, store).await;
            let provider = fixture.provider.clone();
            let source = original.caller().unwrap();
            let task = tokio::spawn(async move {
                if refresh {
                    provider
                        .refresh_session_with_ownership(
                            "personal",
                            Some(&source),
                            CredentialMode::Both,
                        )
                        .await
                } else {
                    provider
                        .password_login_with_mode(&request("personal"), CredentialMode::Both)
                        .await
                }
            });
            for _ in 0..=boundary {
                received(&mut fixture).await;
            }
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            gate.notify_one();
            assert!(
                fixture
                    .provider
                    .auth_registry
                    .lock()
                    .unwrap()
                    .attempts
                    .is_empty()
            );
            assert_eq!(
                stored(&fixture.store, "personal"),
                Some(original.stored("personal").unwrap())
            );
            (&mut fixture.network.server).await.unwrap();
            assert!(fixture.network.seen.try_recv().is_err());
        }
    }
}

#[tokio::test]
async fn logout_of_an_absent_alias_cancels_an_inflight_first_login_in_shared_provider() {
    let gate = Arc::new(Notify::new());
    let mut responses = replies(vec![device(), login(42, "candidate-session"), valid()]);
    responses[2].1 = Some(gate.clone());
    let mut fixture = setup(responses, Arc::new(Store::default())).await;
    let provider = fixture.provider.clone();
    let task = tokio::spawn(async move {
        provider
            .password_login_with_mode(&request("personal"), CredentialMode::Both)
            .await
    });
    for _ in 0..3 {
        received(&mut fixture).await;
    }
    assert!(!fixture.provider.logout("personal").await.unwrap());
    gate.notify_one();
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    assert!(stored(&fixture.store, "personal").is_none());
}

#[tokio::test]
async fn both_mode_can_use_an_older_sid_only_with_the_same_server_login_generation() {
    let store = Arc::new(Store::default());
    let original = credential_fixture("42", "old-session");
    let source = original.caller().unwrap();
    let mut value: serde_json::Value = serde_json::from_str(source.secret()).unwrap();
    value["session_id"] = json!("current-server-session");
    let current = NativeCredential::parse(
        &ProviderCredential::new(Platform::Kuwo, "kuwo_native_v1", value.to_string(), None)
            .unwrap(),
    )
    .unwrap();
    assert!(original.same_login(&current));
    store.put(&current.stored("personal").unwrap()).unwrap();
    let mut fixture = setup(replies(vec![login(42, "newest-session"), valid()]), store).await;
    let result = fixture
        .provider
        .refresh_session_with_ownership("personal", Some(&source), CredentialMode::Both)
        .await
        .unwrap();
    let returned = NativeCredential::parse(result.credential.as_ref().unwrap()).unwrap();
    assert!(original.same_login(&returned));
    assert_eq!(returned.input().unwrap().session_id(), "newest-session");
    assert_eq!(
        result.credential.unwrap().secret(),
        stored(&fixture.store, "personal").unwrap().secret()
    );
    fixture::requests(&mut fixture.network, 2).await;
}

#[tokio::test]
async fn corrupt_duplicate_or_wrong_platform_aliases_are_not_replaced_or_used() {
    for invalid in ["corrupt", "duplicate", "platform"] {
        let store = Arc::new(Store::default());
        let good = credential_fixture("42", "session")
            .stored("personal")
            .unwrap();
        match invalid {
            "corrupt" => {
                store
                    .put(
                        &StoredAccountCredential::new(
                            Platform::Kuwo,
                            "personal",
                            "kuwo_native_v1",
                            "bad-secret",
                        )
                        .unwrap(),
                    )
                    .unwrap();
            }
            "duplicate" => {
                store.put(&good).unwrap();
                store
                    .values
                    .lock()
                    .unwrap()
                    .insert("other-key".into(), good);
            }
            _ => {
                store
                    .put(
                        &StoredAccountCredential::new(
                            Platform::Migu,
                            "personal",
                            "kuwo_native_v1",
                            good.secret(),
                        )
                        .unwrap(),
                    )
                    .unwrap();
            }
        }
        let before = store.values.lock().unwrap().clone();
        let mut fixture = setup(vec![], store).await;
        assert_eq!(
            fixture
                .provider
                .password_login_with_mode(&request("personal"), CredentialMode::Both)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InternalError
        );
        assert_eq!(
            fixture
                .provider
                .session_profile("personal")
                .await
                .unwrap_err()
                .code,
            ErrorCode::InternalError
        );
        assert_eq!(*fixture.store.values.lock().unwrap(), before);
        fixture::requests(&mut fixture.network, 0).await;
    }
}
