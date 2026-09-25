//! HTTP scope contracts; real SDK/account protocol boundaries use Soda loopback tests.
use super::*;

type Calls = Arc<Mutex<Vec<(String, Option<String>)>>>;
#[derive(Clone)]
struct Provider {
    caller: bool,
    failure: Option<ErrorCode>,
    calls: Calls,
    update: Arc<Mutex<Option<ProviderCredential>>>,
}
impl Provider {
    fn page<T>(
        &self,
        kind: &str,
        id: &str,
        limit: u32,
        offset: u32,
        account: Option<&str>,
        item: T,
    ) -> Result<Page<T>> {
        assert_eq!(id, "123");
        assert_eq!((limit, offset), (2, 49));
        assert!(matches!(account, Some("default" | "personal")));
        if self.caller {
            assert_eq!(account, Some("default"));
        }
        self.calls
            .lock()
            .unwrap()
            .push((kind.into(), account.map(str::to_owned)));
        if let Some(code) = self.failure {
            return Err(
                TuneWeaveError::new(code, "Artist catalogue failed").with_platform(Platform::Soda)
            );
        }
        if self.caller {
            *self.update.lock().unwrap() = Some(
                ProviderCredential::new(Platform::Soda, "fixture", "artist-rotated-secret", None)
                    .unwrap(),
            );
        }
        Ok(Page {
            items: vec![item],
            pagination: PageMeta {
                limit,
                offset,
                total: Some(52),
                has_more: true,
                next_offset: Some(50),
                extensions: Extensions::from([
                    ("source_user_id".into(), json!("123456")),
                    ("authenticated".into(), json!(true)),
                    ("complete_read".into(), json!(true)),
                ]),
            },
        })
    }
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Soda
    }
    fn name(&self) -> &'static str {
        "Soda account artist contract"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::ArtistTracks,
            Capability::ArtistAlbums,
            Capability::CallerManagedCredentials,
        ])
    }
    fn with_caller_credential(&self, c: &ProviderCredential) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(c.secret(), "artist-input-secret");
        Ok(Arc::new(Self {
            caller: true,
            update: Arc::default(),
            ..self.clone()
        }))
    }
    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        Ok(self.update.lock().unwrap().take())
    }
    async fn artist_tracks(&self, id: &str, r: &ArtistTrackListRequest) -> Result<Page<Track>> {
        assert_eq!(r.order, ArtistTrackOrder::PlatformDefault);
        self.page(
            "tracks",
            id,
            r.limit,
            r.offset,
            r.account.as_deref(),
            Track::new(ResourceRef::new(Platform::Soda, "50").unwrap(), "Song"),
        )
    }
    async fn artist_albums(&self, id: &str, r: &PageRequest) -> Result<Page<Album>> {
        let mut a = sample_album("50");
        a.platform = Platform::Soda;
        a.resource_ref = ResourceRef::new(Platform::Soda, "50").unwrap();
        self.page("albums", id, r.limit, r.offset, r.account.as_deref(), a)
    }
}
fn app(failure: Option<ErrorCode>) -> (Router, Calls) {
    let calls = Arc::default();
    let mut registry = ProviderRegistry::new();
    registry
        .register(Provider {
            caller: false,
            failure,
            calls: Arc::clone(&calls),
            update: Arc::default(),
        })
        .unwrap();
    (build_router(AppState::new(registry, Platform::Soda)), calls)
}
fn request(uri: &str, caller: bool) -> Request<Body> {
    let mut r = Request::builder().uri(uri);
    if caller {
        r = r.header(
            CALLER_CREDENTIAL_HEADER,
            CallerCredential::issue(
                &ProviderCredential::new(Platform::Soda, "fixture", "artist-input-secret", None)
                    .unwrap(),
            )
            .unwrap()
            .value,
        );
    }
    r.body(Body::empty()).unwrap()
}
#[tokio::test]
async fn soda_account_artist_http_three_owners_keep_windows_scope_and_rotated_credentials() {
    for owner in ["default", "personal", "caller"] {
        for kind in ["tracks", "albums"] {
            let (router, calls) = app(None);
            let mut path = format!("/v1/artists/soda:123/{kind}?limit=2&offset=49");
            if kind == "tracks" {
                path.push_str("&order=platform_default");
            }
            if owner != "caller" {
                path.push_str(&format!("&account={owner}"));
            }
            let response = router
                .oneshot(request(&path, owner == "caller"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert!(
                response.headers()[header::CACHE_CONTROL]
                    .to_str()
                    .unwrap()
                    .contains("no-store")
            );
            assert_eq!(
                response
                    .headers()
                    .contains_key(caller_scope::UPDATED_CREDENTIAL_HEADER),
                owner == "caller"
            );
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            let raw = String::from_utf8_lossy(&bytes);
            assert!(!raw.contains("artist-input-secret"));
            assert!(!raw.contains("artist-rotated-secret"));
            let v: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(v["data"][0]["ref"], "soda:50");
            assert_eq!(v["meta"]["pagination"]["offset"], 49);
            assert_eq!(
                v["meta"]["pagination"]["extensions"]["source_user_id"],
                "123456"
            );
            assert_eq!(calls.lock().unwrap().len(), 1);
        }
    }
}
#[tokio::test]
async fn soda_account_artist_http_failures_do_not_export_partial_data_or_credentials() {
    for kind in ["tracks", "albums"] {
        for (code, status) in [
            (ErrorCode::AuthenticationRequired, StatusCode::UNAUTHORIZED),
            (ErrorCode::Conflict, StatusCode::CONFLICT),
            (ErrorCode::UpstreamError, StatusCode::BAD_GATEWAY),
            (ErrorCode::UpstreamTimeout, StatusCode::GATEWAY_TIMEOUT),
        ] {
            let (router, _) = app(Some(code));
            let path = format!(
                "/v1/artists/soda:123/{kind}?limit=2&offset=49{}",
                if kind == "tracks" {
                    "&order=platform_default"
                } else {
                    ""
                }
            );
            let response = router.oneshot(request(&path, true)).await.unwrap();
            assert_eq!(response.status(), status);
            assert!(
                response.headers()[header::CACHE_CONTROL]
                    .to_str()
                    .unwrap()
                    .contains("no-store")
            );
            assert!(
                !response
                    .headers()
                    .contains_key(caller_scope::UPDATED_CREDENTIAL_HEADER)
            );
            let v: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                    .unwrap();
            assert!(v.get("data").is_none_or(Value::is_null));
        }
    }
    for path in [
        "/v1/artists/soda:123/tracks?limit=0",
        "/v1/artists/soda:123/tracks?order=unknown",
        "/v1/artists/soda:123/albums?limit=101",
        "/v1/artists/soda:123/albums?account=personal",
    ] {
        let (router, calls) = app(None);
        let response = router.oneshot(request(path, true)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(calls.lock().unwrap().is_empty());
        assert!(
            response.headers()[header::CACHE_CONTROL]
                .to_str()
                .unwrap()
                .contains("no-store")
        );
    }
}
#[tokio::test]
async fn soda_account_artist_http_real_provider_validates_identity_window_and_missing_sources_before_network()
 {
    for (suffix, status) in [
        (
            "0123/tracks?order=platform_default&account=personal",
            StatusCode::BAD_REQUEST,
        ),
        (
            "123/albums?limit=100&offset=4294967295&account=personal",
            StatusCode::BAD_REQUEST,
        ),
        (
            "123/tracks?order=hot&account=personal",
            StatusCode::BAD_REQUEST,
        ),
        (
            "123/tracks?order=platform_default&account=personal",
            StatusCode::UNAUTHORIZED,
        ),
        ("123/albums?account=personal", StatusCode::UNAUTHORIZED),
    ] {
        let mut registry = ProviderRegistry::new();
        registry
            .register(
                tuneweave_provider_soda::SodaProvider::new(
                    tuneweave_provider_soda::SodaConfig::default(),
                )
                .unwrap(),
            )
            .unwrap();
        let response = build_router(AppState::new(registry, Platform::Soda))
            .oneshot(request(&format!("/v1/artists/soda:{suffix}"), false))
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{suffix}");
        assert!(
            response.headers()[header::CACHE_CONTROL]
                .to_str()
                .unwrap()
                .contains("no-store")
        );
    }
}
