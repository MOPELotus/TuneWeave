use super::*;

type ProfileCall = (String, UserProfileBackend, Option<String>);

struct Provider {
    caller: bool,
    failure: Option<ErrorCode>,
    update: Mutex<Option<ProviderCredential>>,
    calls: Arc<Mutex<Vec<ProfileCall>>>,
    sessions: Arc<Mutex<Vec<String>>>,
}

impl Provider {
    fn profile(
        &self,
        id: &str,
        backend: UserProfileBackend,
        account: Option<&str>,
    ) -> Result<UserProfile> {
        self.calls
            .lock()
            .unwrap()
            .push((id.to_owned(), backend, account.map(str::to_owned)));
        if let Some(code) = self.failure {
            return Err(TuneWeaveError::new(code, "user profile fixture failure")
                .with_platform(Platform::Migu));
        }
        if backend != UserProfileBackend::Modern {
            return Err(TuneWeaveError::unsupported(
                Platform::Migu,
                Capability::UserProfileLegacy,
            ));
        }
        if self.caller {
            *self.update.lock().unwrap() = Some(
                ProviderCredential::new(Platform::Migu, "test", "updated-profile-session", None)
                    .unwrap(),
            );
        }
        let mut extensions = Extensions::new();
        extensions.insert("backend".to_owned(), json!(backend));
        extensions.insert("account".to_owned(), json!(account));
        Ok(UserProfile {
            user: User {
                resource_ref: ResourceRef::new(Platform::Migu, id).unwrap(),
                platform: Platform::Migu,
                id: id.to_owned(),
                name: "Migu profile".to_owned(),
                avatar_url: Some("https://example.test/migu-avatar.jpg".to_owned()),
                signature: Some("profile fixture".to_owned()),
                followed: Some(false),
                mutual: Some(false),
                extensions: Extensions::new(),
            },
            level: Some(7),
            listened_track_count: Some(321),
            playlist_count: Some(4),
            playlist_subscriber_count: Some(2),
            following_count: Some(8),
            follower_count: Some(13),
            event_count: Some(5),
            birthday: None,
            created_at: Some("2020-01-01T00:00:00Z".to_owned()),
            background_url: None,
            description: Some("Migu profile fixture".to_owned()),
            public_listening_history: Some(false),
            extensions,
        })
    }
}

#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Migu
    }

    fn name(&self) -> &'static str {
        "Migu user profile contract fixture"
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::UserProfileModern,
            Capability::AccountProfile,
            Capability::CallerManagedCredentials,
        ])
    }

    fn with_caller_credential(
        &self,
        credential: &ProviderCredential,
    ) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(credential.platform, Platform::Migu);
        assert_eq!(credential.secret(), "original-profile-session");
        Ok(Arc::new(Self {
            caller: true,
            failure: self.failure,
            update: Mutex::new(None),
            calls: self.calls.clone(),
            sessions: self.sessions.clone(),
        }))
    }

    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        Ok(self.update.lock().unwrap().take())
    }

    async fn user_profile(
        &self,
        id: &str,
        backend: UserProfileBackend,
        account: Option<&str>,
    ) -> Result<UserProfile> {
        self.profile(id, backend, account)
    }

    async fn session_profile(&self, account: &str) -> Result<AccountProfile> {
        self.sessions.lock().unwrap().push(account.to_owned());
        let mut profile = AccountProfile::authenticated(Platform::Migu, account);
        profile.user_id = Some("111".to_owned());
        profile.nickname = Some("Migu account".to_owned());
        Ok(profile)
    }
}

fn app(provider: Provider) -> Router {
    let mut registry = ProviderRegistry::new();
    registry.register(provider).unwrap();
    build_router(AppState::new(registry, Platform::Migu))
}

fn caller_credential() -> CallerCredential {
    CallerCredential::issue(
        &ProviderCredential::new(Platform::Migu, "test", "original-profile-session", None).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn migu_user_profile_routes_dispatch_backend_identity_and_account_scope() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sessions = Arc::new(Mutex::new(Vec::new()));
    let application = app(Provider {
        caller: false,
        failure: None,
        update: Mutex::new(None),
        calls: calls.clone(),
        sessions: sessions.clone(),
    });

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/users/migu:111?backend=modern&account=A")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(body["data"]["user"]["ref"], "migu:111");
    assert_eq!(body["data"]["level"], 7);
    assert_eq!(body["data"]["extensions"]["backend"], "modern");
    assert_eq!(body["data"]["extensions"]["account"], "A");
    assert_eq!(body["meta"]["platform"], "migu");
    assert_eq!(body["meta"]["account"], "A");

    let response = application
        .oneshot(
            Request::builder()
                .uri("/v1/users/migu:111?backend=eapi")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.headers().get(header::CACHE_CONTROL).is_none());
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(body["data"]["extensions"]["backend"], "modern");
    assert!(body["meta"].get("account").is_none());
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            (
                "111".to_owned(),
                UserProfileBackend::Modern,
                Some("A".to_owned()),
            ),
            ("111".to_owned(), UserProfileBackend::Modern, None),
        ]
    );
    assert!(sessions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn migu_account_profile_resolves_session_user_and_rotates_caller_credential() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let sessions = Arc::new(Mutex::new(Vec::new()));
    let application = app(Provider {
        caller: false,
        failure: None,
        update: Mutex::new(None),
        calls: calls.clone(),
        sessions: sessions.clone(),
    });
    let credential = caller_credential();
    let expected = CallerCredential::issue(
        &ProviderCredential::new(Platform::Migu, "test", "updated-profile-session", None).unwrap(),
    )
    .unwrap();
    let response = application
        .oneshot(
            Request::builder()
                .uri("/v1/account/profile?platform=migu&backend=v2")
                .header(CALLER_CREDENTIAL_HEADER, credential.value.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let updated = response
        .headers()
        .get("X-TuneWeave-Updated-Credential")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(updated.strip_prefix("migu=").unwrap(), expected.value);
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(body["data"]["user"]["ref"], "migu:111");
    assert_eq!(body["data"]["extensions"]["backend"], "modern");
    assert_eq!(body["data"]["extensions"]["account"], "default");
    assert_eq!(body["meta"]["platform"], "migu");
    assert_eq!(body["meta"]["caller_credential"]["value"], expected.value);
    assert!(body["meta"].get("account").is_none());
    assert_eq!(*sessions.lock().unwrap(), vec!["default".to_owned()]);
    assert_eq!(
        *calls.lock().unwrap(),
        vec![(
            "111".to_owned(),
            UserProfileBackend::Modern,
            Some("default".to_owned()),
        )]
    );
}

#[tokio::test]
async fn migu_user_profile_errors_keep_private_account_responses_and_status_mapping() {
    for (failure, status) in [
        (ErrorCode::UpstreamError, StatusCode::BAD_GATEWAY),
        (ErrorCode::PermissionDenied, StatusCode::FORBIDDEN),
        (ErrorCode::AuthenticationRequired, StatusCode::UNAUTHORIZED),
        (ErrorCode::Conflict, StatusCode::CONFLICT),
    ] {
        let application = app(Provider {
            caller: false,
            failure: Some(failure),
            update: Mutex::new(None),
            calls: Arc::new(Mutex::new(Vec::new())),
            sessions: Arc::new(Mutex::new(Vec::new())),
        });
        let response = application
            .oneshot(
                Request::builder()
                    .uri("/v1/users/migu:111?backend=modern&account=A")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(body["ok"], false);
        assert_eq!(body["error"]["code"], failure.as_str());
        assert!(body["meta"].get("caller_credential").is_none());
    }
}

#[tokio::test]
async fn migu_user_profile_backend_boundaries_preserve_private_error_handling() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let application = app(Provider {
        caller: false,
        failure: None,
        update: Mutex::new(None),
        calls: calls.clone(),
        sessions: Arc::new(Mutex::new(Vec::new())),
    });

    let response = application
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/users/migu:111?backend=legacy&account=A")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(body["error"]["code"], "capability_not_supported");
    assert_eq!(
        *calls.lock().unwrap(),
        vec![(
            "111".to_owned(),
            UserProfileBackend::Legacy,
            Some("A".to_owned()),
        )]
    );

    let response = application
        .oneshot(
            Request::builder()
                .uri("/v1/users/migu:111?backend=experimental&account=A")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(body["error"]["code"], "invalid_request");
    assert_eq!(calls.lock().unwrap().len(), 1);
}
