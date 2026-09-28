//! HTTP contract fixtures; real Migu protocol and account races live in its provider tests.
use super::*;

#[derive(Clone)]
struct Provider {
    caller: bool,
    account: &'static str,
    failure: Option<ErrorCode>,
    update: Arc<Mutex<Option<ProviderCredential>>>,
    calls: Arc<Mutex<Vec<String>>>,
}
impl Provider {
    fn accept(&self, request: &PageRequest) -> Result<()> {
        assert_eq!(request.account.as_deref(), Some(self.account));
        assert_eq!((request.limit, request.offset), (2, 1));
        self.calls.lock().unwrap().push(self.account.into());
        if self.caller {
            *self.update.lock().unwrap() = Some(
                ProviderCredential::new(
                    Platform::Migu,
                    "fixture",
                    "verified-purchase-update",
                    None,
                )
                .unwrap(),
            );
        }
        if let Some(code) = self.failure {
            return Err(TuneWeaveError::new(code, "Purchase read fixture failed")
                .with_platform(Platform::Migu));
        }
        Ok(())
    }
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Migu
    }
    fn name(&self) -> &'static str {
        "Migu purchased tracks contract"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::AccountPurchasedTracks,
            Capability::AccountPurchasedAlbums,
            Capability::CallerManagedCredentials,
        ])
    }
    fn with_caller_credential(
        &self,
        credential: &ProviderCredential,
    ) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(credential.secret(), "purchase-fixture-secret");
        Ok(Arc::new(Self {
            caller: true,
            account: "default",
            update: Arc::default(),
            ..self.clone()
        }))
    }
    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        Ok(self.update.lock().unwrap().take())
    }
    async fn account_purchased_tracks(
        &self,
        request: &PageRequest,
    ) -> Result<Page<PurchasedTrack>> {
        self.accept(request)?;
        let track = Track::new(
            ResourceRef::new(Platform::Migu, "123").unwrap(),
            "Synthetic song",
        );
        Ok(Page {
            items: vec![
                PurchasedTrack {
                    track: Some(track),
                    name: Some("Synthetic song".into()),
                    artists: vec![],
                    cover_url: None,
                    extensions: Extensions::from([("resource_ref".into(), json!("migu:123"))]),
                },
                PurchasedTrack {
                    track: None,
                    name: None,
                    artists: vec![],
                    cover_url: None,
                    extensions: Extensions::from([
                        ("resource_ref".into(), json!("migu:124")),
                        ("catalogue_resolved".into(), json!(false)),
                    ]),
                },
            ],
            pagination: PageMeta {
                limit: 2,
                offset: 1,
                total: Some(3),
                has_more: false,
                next_offset: None,
                extensions: Extensions::from([
                    ("complete_read".into(), json!(true)),
                    ("consistency".into(), json!("two_complete_reads")),
                    ("source_user_id".into(), json!("111")),
                ]),
            },
        })
    }
    async fn account_purchased_albums(
        &self,
        request: &PageRequest,
    ) -> Result<Page<PurchasedAlbum>> {
        self.accept(request)?;
        let mut ordinary = sample_album("77");
        ordinary.platform = Platform::Migu;
        ordinary.resource_ref = ResourceRef::new(Platform::Migu, "77").unwrap();
        let mut digital = sample_digital_album("77");
        digital.platform = Platform::Migu;
        digital.resource_ref = ResourceRef::new(Platform::Migu, "77").unwrap();
        digital.purchased = None;
        digital.price = None;
        Ok(Page {
            items: vec![
                PurchasedAlbum {
                    name: Some(ordinary.name.clone()),
                    album: Some(ordinary),
                    digital_album: None,
                    artists: vec![],
                    cover_url: None,
                    extensions: Extensions::from([("resource_type".into(), json!("2003"))]),
                },
                PurchasedAlbum {
                    name: Some(digital.name.clone()),
                    album: None,
                    digital_album: Some(Box::new(digital)),
                    artists: vec![],
                    cover_url: None,
                    extensions: Extensions::from([("resource_type".into(), json!("5"))]),
                },
            ],
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(3),
                has_more: false,
                next_offset: None,
                extensions: Extensions::from([
                    ("complete_read".into(), json!(true)),
                    ("source_user_id".into(), json!("111")),
                    ("consistency".into(), json!("two_complete_reads")),
                ]),
            },
        })
    }
}
fn app(scope: &str, failure: Option<ErrorCode>) -> (Router, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::default();
    let mut registry = ProviderRegistry::new();
    registry
        .register(Provider {
            caller: false,
            account: if scope == "named" {
                "personal"
            } else {
                "default"
            },
            failure,
            update: Arc::default(),
            calls: Arc::clone(&calls),
        })
        .unwrap();
    (build_router(AppState::new(registry, Platform::Migu)), calls)
}
fn request(scope: &str, extra: &str) -> Request<Body> {
    let path = format!(
        "/v1/account/purchases/tracks?platform=migu&limit=2&offset=1{extra}{}",
        match scope {
            "named" => "&account=personal",
            "default" => "&account=default",
            _ => "",
        }
    );
    let mut request = Request::builder().uri(path);
    if scope == "caller" {
        let credential =
            ProviderCredential::new(Platform::Migu, "fixture", "purchase-fixture-secret", None)
                .unwrap();
        request = request.header(
            CALLER_CREDENTIAL_HEADER,
            CallerCredential::issue(&credential).unwrap().value,
        );
    }
    request.body(Body::empty()).unwrap()
}
fn album_request(scope: &str, extra: &str) -> Request<Body> {
    let mut request = request(scope, extra);
    *request.uri_mut() = request
        .uri()
        .to_string()
        .replace("/purchases/tracks", "/purchases/albums")
        .parse()
        .unwrap();
    request
}
async fn inspect(response: Response, status: StatusCode, rotated: bool) -> Value {
    assert_eq!(response.status(), status);
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
        rotated
    );
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("purchase-fixture-secret"));
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn migu_purchases_http_account_selection_records_and_credential_updates_follow_each_outcome()
{
    for scope in ["default", "named", "caller", "implicit-default"] {
        for (failure, status) in [
            (None, StatusCode::OK),
            (Some(ErrorCode::UpstreamError), StatusCode::BAD_GATEWAY),
            (
                Some(ErrorCode::AuthenticationRequired),
                StatusCode::UNAUTHORIZED,
            ),
            (Some(ErrorCode::Conflict), StatusCode::CONFLICT),
            (
                Some(ErrorCode::UpstreamTimeout),
                StatusCode::GATEWAY_TIMEOUT,
            ),
        ] {
            let (app, calls) = app(scope, failure);
            let body = inspect(
                app.oneshot(request(scope, "")).await.unwrap(),
                status,
                scope == "caller"
                    && !matches!(
                        failure,
                        Some(ErrorCode::AuthenticationRequired | ErrorCode::Conflict)
                    ),
            )
            .await;
            assert_eq!(
                calls.lock().unwrap().as_slice(),
                [if scope == "named" {
                    "personal"
                } else {
                    "default"
                }]
            );
            if failure.is_none() {
                assert_eq!(body["data"][0]["track"]["ref"], "migu:123");
                assert!(body["data"][1]["track"].is_null());
                assert_eq!(body["data"][1]["extensions"]["resource_ref"], "migu:124");
                assert_eq!(body["meta"]["pagination"]["total"], 3);
                assert_eq!(
                    body["meta"]["pagination"]["extensions"]["consistency"],
                    "two_complete_reads"
                );
            }
        }
    }
}

#[tokio::test]
async fn migu_purchases_http_validation_and_real_provider_rejects_disabled_capabilities() {
    for scope in ["default", "named", "caller"] {
        for extra in [
            "&limit=0",
            "&offset=4294967295",
            "&unknown=1",
            "&limit=bad",
            "&account=duplicate",
        ] {
            let (app, calls) = app(scope, None);
            inspect(
                app.oneshot(request(scope, extra)).await.unwrap(),
                StatusCode::BAD_REQUEST,
                false,
            )
            .await;
            assert!(calls.lock().unwrap().is_empty());
        }
    }
    let mut registry = ProviderRegistry::new();
    registry
        .register(tuneweave_provider_migu::MiguProvider::new(Default::default()).unwrap())
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Migu));
    for account in ["", "&account=absent"] {
        let request = Request::builder()
            .uri(format!(
                "/v1/account/purchases/tracks?platform=migu{account}"
            ))
            .body(Body::empty())
            .unwrap();
        inspect(
            app.clone().oneshot(request).await.unwrap(),
            StatusCode::UNPROCESSABLE_ENTITY,
            false,
        )
        .await;
    }
}

#[tokio::test]
async fn migu_purchases_album_http_preserves_digital_identity_and_account_error_contracts() {
    for scope in ["default", "named", "caller", "implicit-default"] {
        for (failure, status) in [
            (None, StatusCode::OK),
            (Some(ErrorCode::UpstreamError), StatusCode::BAD_GATEWAY),
            (
                Some(ErrorCode::AuthenticationRequired),
                StatusCode::UNAUTHORIZED,
            ),
            (Some(ErrorCode::Conflict), StatusCode::CONFLICT),
            (
                Some(ErrorCode::UpstreamTimeout),
                StatusCode::GATEWAY_TIMEOUT,
            ),
        ] {
            let (app, calls) = app(scope, failure);
            let body = inspect(
                app.oneshot(album_request(scope, "")).await.unwrap(),
                status,
                scope == "caller"
                    && !matches!(
                        failure,
                        Some(ErrorCode::AuthenticationRequired | ErrorCode::Conflict)
                    ),
            )
            .await;
            assert_eq!(calls.lock().unwrap().len(), 1);
            if failure.is_none() {
                assert_eq!(body["data"][0]["album"]["ref"], "migu:77");
                assert!(body["data"][0].get("digital_album").is_none());
                assert!(body["data"][1]["album"].is_null());
                assert_eq!(body["data"][1]["digital_album"]["ref"], "migu:77");
                assert!(body["data"][1]["digital_album"]["purchased"].is_null());
                assert!(body["data"][1]["digital_album"]["price"].is_null());
                assert_eq!(body["data"][0]["extensions"]["resource_type"], "2003");
                assert_eq!(body["data"][1]["extensions"]["resource_type"], "5");
                assert_eq!(body["meta"]["pagination"]["total"], 3);
            }
        }
        for extra in [
            "&limit=0",
            "&offset=4294967295",
            "&unknown=1",
            "&account=duplicate",
        ] {
            if scope == "implicit-default" && extra == "&account=duplicate" {
                continue;
            }
            let (app, calls) = app(scope, None);
            inspect(
                app.oneshot(album_request(scope, extra)).await.unwrap(),
                StatusCode::BAD_REQUEST,
                false,
            )
            .await;
            assert!(calls.lock().unwrap().is_empty());
        }
    }
    let mut registry = ProviderRegistry::new();
    registry
        .register(tuneweave_provider_migu::MiguProvider::new(Default::default()).unwrap())
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Migu));
    for account in ["", "&account=absent"] {
        let request = Request::builder()
            .uri(format!(
                "/v1/account/purchases/albums?platform=migu{account}"
            ))
            .body(Body::empty())
            .unwrap();
        inspect(
            app.clone().oneshot(request).await.unwrap(),
            StatusCode::UNPROCESSABLE_ENTITY,
            false,
        )
        .await;
    }
}

#[tokio::test]
async fn migu_following_artist_reads_are_disabled_and_not_advertised() {
    let mut registry = ProviderRegistry::new();
    registry
        .register(tuneweave_provider_migu::MiguProvider::new(Default::default()).unwrap())
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Migu));

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/capabilities?platform=migu")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = to_bytes(response.into_body(), 65536).await.unwrap();
    let capabilities: Value = serde_json::from_slice(&body).unwrap();
    let declared = capabilities["data"][0]["capabilities"].as_array().unwrap();
    for capability in [
        "account_purchased_tracks",
        "account_purchased_albums",
        "account_following_artists",
        "artist_subscription_write",
    ] {
        assert!(!declared.iter().any(|value| value == capability));
    }

    for uri in [
        "/v1/account/following/artists?platform=migu",
        "/v1/users/migu:123/following/artists",
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            !response
                .headers()
                .contains_key(caller_scope::UPDATED_CREDENTIAL_HEADER)
        );
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        let error: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(error["error"]["code"], "capability_not_supported");
        assert_eq!(
            error["error"]["details"]["capability"],
            "account_following_artists"
        );
    }

    for method in [Method::PUT, Method::DELETE] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri("/v1/account/following/artists/migu:123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            !response
                .headers()
                .contains_key(caller_scope::UPDATED_CREDENTIAL_HEADER)
        );
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        let error: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(error["error"]["code"], "capability_not_supported");
        assert_eq!(
            error["error"]["details"]["capability"],
            "artist_subscription_write"
        );
    }
}

#[tokio::test]
async fn migu_purchases_implicit_account_early_rejections_always_disable_caching() {
    for kind in ["tracks", "albums"] {
        for failure in ["query", "method", "request-id"] {
            let (app, calls) = app("implicit-default", None);
            let uri = format!(
                "/v1/account/purchases/{kind}?platform=migu&limit={}",
                if failure == "query" { "bad" } else { "2" }
            );
            let mut request = Request::builder().uri(uri);
            let status = if failure == "method" {
                request = request.method("POST");
                StatusCode::METHOD_NOT_ALLOWED
            } else {
                StatusCode::BAD_REQUEST
            };
            if failure == "request-id" {
                request = request.header("x-request-id", "invalid request id");
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
            assert!(
                !response
                    .headers()
                    .contains_key(caller_scope::UPDATED_CREDENTIAL_HEADER)
            );
            assert!(calls.lock().unwrap().is_empty());
        }
    }
}
