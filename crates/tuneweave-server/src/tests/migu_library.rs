use super::*;

struct LibraryProvider {
    platform: Platform,
    caller: bool,
    failure: Option<ErrorCode>,
    update: Mutex<Option<ProviderCredential>>,
    calls: Arc<Mutex<Vec<String>>>,
}
impl LibraryProvider {
    fn read(
        &self,
        section: &str,
        uid: Option<&str>,
        request: &PageRequest,
    ) -> Result<Page<Playlist>> {
        assert_eq!(
            request.account.as_deref(),
            if self.caller {
                Some("default")
            } else {
                Some("A")
            }
        );
        assert_eq!((request.limit, request.offset), (10, 2));
        assert_eq!(uid, if section == "all" { None } else { Some("111") });
        self.calls.lock().unwrap().push(section.into());
        if self.caller {
            *self.update.lock().unwrap() = Some(
                ProviderCredential::new(self.platform, "test", "updated-library-session", None)
                    .unwrap(),
            );
        }
        if let Some(code) = self.failure {
            return Err(
                TuneWeaveError::new(code, "library fixture failure").with_platform(self.platform)
            );
        }
        Ok(Page {
            items: vec![],
            pagination: PageMeta {
                limit: 10,
                offset: 2,
                total: Some(2),
                has_more: false,
                next_offset: None,
                extensions: Extensions::new(),
            },
        })
    }
}
#[async_trait]
impl MusicProvider for LibraryProvider {
    fn platform(&self) -> Platform {
        self.platform
    }
    fn name(&self) -> &'static str {
        "Account library contract fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::AccountPlaylists,
            Capability::PlaylistRead,
            Capability::CallerManagedCredentials,
        ])
    }
    fn with_caller_credential(&self, c: &ProviderCredential) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(c.secret(), "original-library-session");
        Ok(Arc::new(Self {
            platform: self.platform,
            caller: true,
            failure: self.failure,
            update: Mutex::new(None),
            calls: self.calls.clone(),
        }))
    }
    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        Ok(self.update.lock().unwrap().take())
    }
    async fn account_playlists(&self, r: &PageRequest) -> Result<Page<Playlist>> {
        self.read("all", None, r)
    }
    async fn user_created_playlists(&self, uid: &str, r: &PageRequest) -> Result<Page<Playlist>> {
        self.read("created", Some(uid), r)
    }
    async fn user_favorite_playlists(&self, uid: &str, r: &PageRequest) -> Result<Page<Playlist>> {
        self.read("saved", Some(uid), r)
    }
}
#[tokio::test]
async fn migu_library_routes_preserve_account_pagination_and_rotations_on_success_and_later_error()
{
    check_library_routes(Platform::Migu).await;
}

pub(super) async fn check_library_routes(platform: Platform) {
    for caller in [false, true] {
        for (section, base) in [
            ("all", format!("/v1/account/playlists?platform={platform}&")),
            (
                "created",
                format!("/v1/users/{platform}:111/playlists/created?"),
            ),
            (
                "saved",
                format!("/v1/users/{platform}:111/favorites/playlists?"),
            ),
        ] {
            for (failure, status) in [
                (None, StatusCode::OK),
                (Some(ErrorCode::UpstreamError), StatusCode::BAD_GATEWAY),
                (Some(ErrorCode::PermissionDenied), StatusCode::FORBIDDEN),
                (
                    Some(ErrorCode::AuthenticationRequired),
                    StatusCode::UNAUTHORIZED,
                ),
                (Some(ErrorCode::Conflict), StatusCode::CONFLICT),
            ] {
                let calls = Arc::new(Mutex::new(vec![]));
                let mut registry = ProviderRegistry::new();
                registry
                    .register(LibraryProvider {
                        platform,
                        caller: false,
                        failure,
                        update: Mutex::new(None),
                        calls: calls.clone(),
                    })
                    .unwrap();
                let app = build_router(AppState::new(registry, platform));
                let mut request = Request::builder().uri(format!(
                    "{base}limit=10&offset=2{}",
                    if caller { "" } else { "&account=A" }
                ));
                if caller {
                    request = request.header(
                        CALLER_CREDENTIAL_HEADER,
                        CallerCredential::issue(
                            &ProviderCredential::new(
                                platform,
                                "test",
                                "original-library-session",
                                None,
                            )
                            .unwrap(),
                        )
                        .unwrap()
                        .value,
                    );
                }
                let response = app
                    .oneshot(request.body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status(), status);
                assert!(
                    response.headers()[header::CACHE_CONTROL]
                        .to_str()
                        .unwrap()
                        .contains("no-store")
                );
                let update = response
                    .headers()
                    .get("X-TuneWeave-Updated-Credential")
                    .map(|v| v.to_str().unwrap().to_owned());
                let body: Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                        .unwrap();
                let should_update = caller
                    && !matches!(
                        failure,
                        Some(ErrorCode::AuthenticationRequired | ErrorCode::Conflict)
                    );
                assert_eq!(update.is_some(), should_update);
                assert_eq!(body["meta"]["caller_credential"].is_object(), should_update);
                if let Some(update) = update {
                    let prefix = format!("{platform}=");
                    let value = update.strip_prefix(&prefix).unwrap();
                    assert_eq!(
                        CallerCredential::parse(value).unwrap().secret(),
                        "updated-library-session"
                    );
                    assert_eq!(body["meta"]["caller_credential"]["value"], value);
                }
                if failure.is_none() {
                    assert_eq!(body["meta"]["pagination"]["total"], 2);
                    assert_eq!(body["data"], json!([]));
                    if caller {
                        assert!(body["meta"].get("account").is_none());
                    } else {
                        assert_eq!(body["meta"]["account"], "A");
                    }
                }
                assert_eq!(*calls.lock().unwrap(), vec![section.to_owned()]);
            }
        }
    }
}
