//! HTTP contracts; account protocol, pagination and resolver integration are tested in Soda.
use super::*;
#[derive(Clone)]
struct Provider {
    caller: bool,
    failure: Option<ErrorCode>,
    calls: Arc<Mutex<Vec<SearchQuery>>>,
    update: Arc<Mutex<Option<ProviderCredential>>>,
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Soda
    }
    fn name(&self) -> &'static str {
        "Soda account search contract"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::SearchTracks,
            Capability::SearchAlbums,
            Capability::SearchArtists,
            Capability::SearchPlaylists,
            Capability::CallerManagedCredentials,
        ])
    }
    fn with_caller_credential(&self, c: &ProviderCredential) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(c.secret(), "search-private-input");
        Ok(Arc::new(Self {
            caller: true,
            update: Arc::default(),
            ..self.clone()
        }))
    }
    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        Ok(self.update.lock().unwrap().take())
    }
    async fn search_catalog(&self, q: &SearchQuery) -> Result<Page<SearchItem>> {
        self.calls.lock().unwrap().push(q.clone());
        assert_eq!(q.query, "周");
        assert_eq!((q.limit, q.offset), (5, 18));
        assert!(matches!(q.account.as_deref(), Some("personal" | "default")));
        if self.caller {
            assert_eq!(q.account.as_deref(), Some("default"));
        }
        if let Some(code) = self.failure {
            return Err(
                TuneWeaveError::new(code, "Account search failed").with_platform(Platform::Soda)
            );
        }
        let r = ResourceRef::new(Platform::Soda, "1018").unwrap();
        let ext = Extensions::from([
            ("backend".into(), json!("official_pc_account_search")),
            ("source_user_id".into(), json!("123456")),
            ("authenticated".into(), json!(true)),
        ]);
        let item = match q.kind {
            SearchKind::Track => {
                let mut t = Track::new(r, "Song");
                t.extensions = ext.clone();
                SearchItem::Track(t)
            }
            SearchKind::Album => {
                let mut v = sample_album("1018");
                v.platform = Platform::Soda;
                v.resource_ref = r;
                v.extensions = ext.clone();
                SearchItem::Album(v)
            }
            SearchKind::Artist => {
                let mut v = sample_artist("1018");
                v.platform = Platform::Soda;
                v.resource_ref = r;
                v.extensions = ext.clone();
                SearchItem::Artist(v)
            }
            SearchKind::Playlist => {
                let mut v = sample_playlist("1018");
                v.platform = Platform::Soda;
                v.resource_ref = r;
                v.extensions = ext.clone();
                SearchItem::Playlist(v)
            }
            _ => panic!("unexpected fixture kind"),
        };
        if self.caller {
            *self.update.lock().unwrap() = Some(
                ProviderCredential::new(Platform::Soda, "fixture", "search-rotated-private", None)
                    .unwrap(),
            );
        }
        Ok(Page {
            items: vec![item],
            pagination: PageMeta {
                limit: q.limit,
                offset: q.offset,
                total: None,
                has_more: true,
                next_offset: Some(19),
                extensions: ext,
            },
        })
    }
}
fn app(failure: Option<ErrorCode>) -> (Router, Arc<Mutex<Vec<SearchQuery>>>) {
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
                &ProviderCredential::new(Platform::Soda, "fixture", "search-private-input", None)
                    .unwrap(),
            )
            .unwrap()
            .value,
        );
    }
    r.body(Body::empty()).unwrap()
}
#[tokio::test]
async fn soda_account_search_http_four_types_and_three_sources_preserve_metadata_and_updates() {
    for owner in ["default", "personal", "caller"] {
        for kind in ["track", "album", "artist", "playlist"] {
            let (router, calls) = app(None);
            let mut path =
                format!("/v1/search?platform=soda&q=%20%E5%91%A8%20&type={kind}&limit=5&offset=18");
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
            assert!(!String::from_utf8_lossy(&bytes).contains("search-private-input"));
            let v: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(v["data"][0]["type"], kind);
            assert_eq!(v["data"][0]["data"]["ref"], "soda:1018");
            assert_eq!(v["meta"]["pagination"]["offset"], 18);
            assert_eq!(
                v["meta"]["pagination"]["extensions"]["source_user_id"],
                "123456"
            );
            assert_eq!(calls.lock().unwrap().len(), 1);
        }
    }
}
#[tokio::test]
async fn soda_account_search_http_errors_and_bad_inputs_have_no_partial_data_or_credentials() {
    for (code, status) in [
        (ErrorCode::AuthenticationRequired, StatusCode::UNAUTHORIZED),
        (ErrorCode::Conflict, StatusCode::CONFLICT),
        (ErrorCode::UpstreamError, StatusCode::BAD_GATEWAY),
    ] {
        let (router, _) = app(Some(code));
        let response = router
            .oneshot(request(
                "/v1/search?platform=soda&q=%E5%91%A8&type=track&limit=5&offset=18",
                true,
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert!(
            !response
                .headers()
                .contains_key(caller_scope::UPDATED_CREDENTIAL_HEADER)
        );
        assert!(
            response.headers()[header::CACHE_CONTROL]
                .to_str()
                .unwrap()
                .contains("no-store")
        );
        let v: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
        assert!(v.get("data").is_none_or(Value::is_null));
    }
    for path in [
        "/v1/search?platform=soda&q=x&limit=0",
        "/v1/search?platform=soda&q=x&type=unknown",
        "/v1/search?platform=soda&q=x&account=personal",
        "/v1/search?platform=soda&q=x&unknown=1",
    ] {
        let (router, calls) = app(None);
        let response = router.oneshot(request(path, true)).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
        assert!(calls.lock().unwrap().is_empty());
    }
}
#[tokio::test]
async fn soda_account_search_http_real_provider_missing_accounts_fail_without_network() {
    for kind in ["track", "album", "artist", "playlist"] {
        let mut registry = ProviderRegistry::new();
        registry
            .register(
                tuneweave_provider_soda::SodaProvider::new(tuneweave_provider_soda::SodaConfig {
                    proxy_url: Some("http://127.0.0.1:9".into()),
                    ..Default::default()
                })
                .unwrap(),
            )
            .unwrap();
        let (status, v) = json_response_from(
            build_router(AppState::new(registry, Platform::Soda)),
            &format!("/v1/search?platform=soda&q=x&type={kind}&account=missing"),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(v["error"]["code"], "authentication_required");
    }
}
