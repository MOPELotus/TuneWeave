//! HTTP ownership/error contract; native protocol success has separate loopback tests.
use super::*;
use tuneweave_core::PlaylistMutationAction;

#[derive(Clone)]
struct Provider {
    account: &'static str,
    fail: bool,
    reject_before_write: bool,
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Kuwo
    }
    fn name(&self) -> &'static str {
        "Kuwo management contract fixture"
    }
    async fn update_playlist_cover(
        &self,
        id: &str,
        r: &ImageUploadRequest,
    ) -> Result<PlaylistCoverUpdateResult> {
        assert_eq!(id, "101");
        assert_eq!(r.account.as_deref(), Some(self.account));
        assert_eq!(r.filename, "cover.png");
        assert_eq!(r.content_type, "image/png");
        assert_eq!(r.data, cover_bytes());
        assert_eq!(
            (r.image_size, r.crop_x, r.crop_y),
            (Some(1), Some(0), Some(0))
        );
        if self.fail {
            return Err(
                unconfirmed().with_details(json!({"write_outcome":"unconfirmed",
                "write_requests_dispatched":2,"upload_requests_dispatched":1,
                "playlist_write_requests_dispatched":1,"upload_outcome":"confirmed",
                "playlist_write_outcome":"unconfirmed","automatic_retry":false})),
            );
        }
        Ok(PlaylistCoverUpdateResult {
            playlist_ref: ResourceRef::new(Platform::Kuwo, id).unwrap(),
            image: ImageUploadResult {
                url: Some("https://img4.kuwo.cn/cover.jpg".into()),
                image_id: None,
                extensions: Extensions::from([
                    ("width".into(), json!(700)),
                    ("height".into(), json!(700)),
                ]),
            },
            extensions: Extensions::from([
                ("confirmed".into(), json!(true)),
                ("write_requests_dispatched".into(), json!(2)),
            ]),
        })
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::PlaylistWrite,
            Capability::TrackSubscriptionWrite,
            Capability::PlaylistSubscriptionWrite,
            Capability::PlaylistCollectionOrderWrite,
            Capability::PlaylistVisibilityWrite,
            Capability::CallerManagedCredentials,
        ])
    }
    fn with_caller_credential(&self, c: &ProviderCredential) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(c.secret(), "private-management-fixture");
        Ok(Arc::new(Self {
            account: "default",
            ..self.clone()
        }))
    }
    async fn create_playlist(&self, r: &PlaylistCreateRequest) -> Result<PlaylistMutationResult> {
        assert_eq!(r.account.as_deref(), Some(self.account));
        assert_eq!(r.name, "New playlist");
        assert_eq!(r.visibility, PlaylistVisibility::Public);
        assert_eq!(r.kind, PlaylistKind::Normal);
        if self.fail {
            return Err(unconfirmed());
        }
        Ok(PlaylistMutationResult {
            playlist_ref: ResourceRef::new(Platform::Kuwo, "201").unwrap(),
            action: PlaylistMutationAction::Create,
            playlist: None,
            extensions: Extensions::from([("confirmed".into(), json!(true))]),
        })
    }
    async fn delete_playlists(&self, r: &PlaylistDeleteRequest) -> Result<PlaylistDeleteResult> {
        assert_eq!(r.account.as_deref(), Some(self.account));
        assert!(
            [vec!["101"], vec!["101", "102"]].contains(
                &r.playlist_refs
                    .iter()
                    .map(ResourceRef::id)
                    .collect::<Vec<_>>()
            )
        );
        if self.fail {
            return Err(unconfirmed());
        }
        Ok(PlaylistDeleteResult {
            playlist_refs: r.playlist_refs.clone(),
            extensions: Extensions::from([("confirmed".into(), json!(true))]),
        })
    }
    async fn update_playlist(
        &self,
        id: &str,
        r: &PlaylistUpdateRequest,
    ) -> Result<PlaylistMutationResult> {
        assert_eq!(id, "101");
        assert_eq!(r.account.as_deref(), Some(self.account));
        assert_eq!(r.name.as_deref(), Some("New playlist"));
        assert_eq!(r.description.as_deref(), Some(""));
        assert_eq!(r.tags, Some(vec![]));
        assert_eq!(r.variant, PlaylistMetadataUpdateVariant::Default);
        if self.reject_before_write {
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "This published playlist change requires explicit contribution management",
            )
            .with_platform(Platform::Kuwo));
        }
        if self.fail {
            return Err(unconfirmed());
        }
        Ok(PlaylistMutationResult {
            playlist_ref: ResourceRef::new(Platform::Kuwo, "101").unwrap(),
            action: PlaylistMutationAction::Update,
            playlist: None,
            extensions: Extensions::from([("confirmed".into(), json!(true))]),
        })
    }
    async fn update_playlist_visibility(
        &self,
        id: &str,
        r: &PlaylistVisibilityUpdateRequest,
    ) -> Result<PlaylistMutationResult> {
        assert_eq!(id, "101");
        assert_eq!(r.account.as_deref(), Some(self.account));
        assert_eq!(r.visibility, PlaylistVisibility::Private);
        if self.reject_before_write {
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "This published playlist change requires explicit contribution management",
            )
            .with_platform(Platform::Kuwo));
        }
        if self.fail {
            return Err(unconfirmed());
        }
        Ok(PlaylistMutationResult {
            playlist_ref: ResourceRef::new(Platform::Kuwo, id).unwrap(),
            action: PlaylistMutationAction::Update,
            playlist: None,
            extensions: Extensions::from([("confirmed".into(), json!(true))]),
        })
    }
    async fn set_playlist_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        assert_eq!(account, Some(self.account));
        assert_eq!(id, "999");
        if self.fail {
            return Err(unconfirmed());
        }
        Ok(SubscriptionResult {
            resource_ref: ResourceRef::new(Platform::Kuwo, id).unwrap(),
            subscribed,
            extensions: Extensions::from([
                ("confirmed".into(), json!(true)),
                ("library_section".into(), json!("collected")),
            ]),
        })
    }
    async fn set_track_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        assert_eq!(account, Some(self.account));
        assert_eq!(id, if subscribed { "44" } else { "22" });
        if self.fail {
            return Err(unconfirmed());
        }
        Ok(SubscriptionResult {
            resource_ref: ResourceRef::new(Platform::Kuwo, id).unwrap(),
            subscribed,
            extensions: Extensions::from([
                ("confirmed".into(), json!(true)),
                ("favorite_playlist_ref".into(), json!("kuwo:901")),
            ]),
        })
    }
    async fn reorder_collected_playlists(
        &self,
        r: &PlaylistOrderRequest,
    ) -> Result<PlaylistOrderResult> {
        assert_eq!(r.account.as_deref(), Some(self.account));
        assert_eq!(
            r.playlist_refs
                .iter()
                .map(ResourceRef::id)
                .collect::<Vec<_>>(),
            ["102", "101"]
        );
        if self.fail {
            return Err(unconfirmed());
        }
        Ok(PlaylistOrderResult {
            playlist_refs: r.playlist_refs.clone(),
            extensions: Extensions::from([
                ("confirmed".into(), json!(true)),
                ("library_section".into(), json!("collected")),
            ]),
        })
    }
    async fn reorder_account_playlists(
        &self,
        r: &PlaylistOrderRequest,
    ) -> Result<PlaylistOrderResult> {
        assert_eq!(r.account.as_deref(), Some(self.account));
        assert_eq!(
            r.playlist_refs
                .iter()
                .map(ResourceRef::id)
                .collect::<Vec<_>>(),
            ["102", "101"]
        );
        if self.fail {
            return Err(unconfirmed());
        }
        Ok(PlaylistOrderResult {
            playlist_refs: r.playlist_refs.clone(),
            extensions: Extensions::from([
                ("confirmed".into(), json!(true)),
                ("write_requests_dispatched".into(), json!(1)),
            ]),
        })
    }
    async fn reorder_playlist_tracks(
        &self,
        id: &str,
        r: &PlaylistTrackOrderRequest,
    ) -> Result<PlaylistTrackOrderResult> {
        assert!(["101", "901"].contains(&id));
        assert_eq!(r.account.as_deref(), Some(self.account));
        assert_eq!(
            r.track_refs.iter().map(ResourceRef::id).collect::<Vec<_>>(),
            ["22", "33", "11", "22"]
        );
        if self.fail {
            return Err(unconfirmed());
        }
        Ok(PlaylistTrackOrderResult {
            playlist_ref: ResourceRef::new(Platform::Kuwo, id).unwrap(),
            track_refs: r.track_refs.clone(),
            snapshot_id: Some("confirmed-order".into()),
            extensions: Extensions::from([
                ("confirmed".into(), json!(true)),
                ("write_requests_dispatched".into(), json!(1)),
            ]),
        })
    }
    async fn mutate_playlist_items(
        &self,
        id: &str,
        action: PlaylistItemMutationAction,
        r: &PlaylistItemMutationRequest,
    ) -> Result<PlaylistItemMutationResult> {
        assert_eq!(id, "101");
        assert_eq!(r.account.as_deref(), Some(self.account));
        assert_eq!(r.kind, PlaylistItemKind::Track);
        assert_eq!(
            r.item_refs,
            vec![
                ResourceRef::new(
                    Platform::Kuwo,
                    if action == PlaylistItemMutationAction::Add {
                        "44"
                    } else {
                        "22"
                    }
                )
                .unwrap()
            ]
        );
        if self.reject_before_write {
            return Err(TuneWeaveError::new(
                ErrorCode::CapabilityNotSupported,
                "This published playlist change requires explicit contribution management",
            )
            .with_platform(Platform::Kuwo));
        }
        if self.fail {
            return Err(unconfirmed());
        }
        Ok(PlaylistItemMutationResult {
            playlist_ref: ResourceRef::new(Platform::Kuwo, id).unwrap(),
            item_refs: r.item_refs.clone(),
            kind: r.kind,
            action,
            snapshot_id: Some("verified-snapshot".into()),
            cloud_track_count: Some(2),
            extensions: Extensions::from([
                ("confirmed".into(), json!(true)),
                ("write_requests_dispatched".into(), json!(1)),
            ]),
        })
    }
}
fn unconfirmed() -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::Conflict,"write readback could not be confirmed").with_platform(Platform::Kuwo)
        .retryable(false).with_details(json!({"write_outcome":"unconfirmed","write_requests_dispatched":1,"automatic_retry":false}))
}
fn requests() -> [(Method, &'static str, Option<Value>); 17] {
    [
        (
            Method::PUT,
            "/v1/account/favorites/playlists/order",
            Some(json!({"refs":["kuwo:102","kuwo:101"]})),
        ),
        (
            Method::PUT,
            "/v1/account/favorites/playlists/kuwo:999",
            None,
        ),
        (
            Method::DELETE,
            "/v1/account/favorites/playlists/kuwo:999",
            None,
        ),
        (
            Method::PUT,
            "/v1/account/playlists/order",
            Some(json!({"refs":["kuwo:102","kuwo:101"]})),
        ),
        (
            Method::PUT,
            "/v1/playlists/kuwo:101/tracks/order",
            Some(json!({"refs":["kuwo:22","kuwo:33","kuwo:11","kuwo:22"]})),
        ),
        (
            Method::PUT,
            "/v1/playlists/kuwo:901/tracks/order",
            Some(json!({"ids":[22,33,11,22]})),
        ),
        (Method::PUT, "/v1/account/favorites/tracks/kuwo:44", None),
        (Method::DELETE, "/v1/account/favorites/tracks/kuwo:22", None),
        (
            Method::POST,
            "/v1/playlists/kuwo:101/tracks",
            Some(json!({"refs":["kuwo:44"]})),
        ),
        (
            Method::DELETE,
            "/v1/playlists/kuwo:101/tracks",
            Some(json!({"refs":["kuwo:22"]})),
        ),
        (
            Method::POST,
            "/v1/playlists/kuwo:101/items",
            Some(json!({"refs":["kuwo:44"],"kind":"track"})),
        ),
        (
            Method::DELETE,
            "/v1/playlists/kuwo:101/items",
            Some(json!({"refs":["kuwo:22"],"kind":"track"})),
        ),
        (
            Method::PUT,
            "/v1/playlists/kuwo:101/visibility",
            Some(json!({"visibility":"private"})),
        ),
        (
            Method::PATCH,
            "/v1/playlists/kuwo:101",
            Some(json!({"name":"New playlist","description":"","tags":[]})),
        ),
        (
            Method::POST,
            "/v1/playlists",
            Some(
                json!({"platform":"kuwo","name":"New playlist","visibility":"public","kind":"normal"}),
            ),
        ),
        (Method::DELETE, "/v1/playlists/kuwo:101", None),
        (
            Method::DELETE,
            "/v1/playlists",
            Some(json!({"refs":["kuwo:101","kuwo:102"]})),
        ),
    ]
}

#[tokio::test]
async fn kuwo_visibility_http_rejects_missing_implicit_numeric_and_unknown_values_before_provider()
{
    // This provider asserts exactly `private`; invalid bodies must never reach it.
    let mut registry = ProviderRegistry::new();
    registry
        .register(Provider {
            account: "default",
            fail: false,
            reject_before_write: false,
        })
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Kuwo));
    for body in [
        json!({}),
        json!({"visibility":null}),
        json!({"visibility":"platform_default"}),
        json!({"visibility":"PUBLIC"}),
        json!({"visibility":"10"}),
        json!({"visibility":10}),
        json!({"visibility":false}),
        json!({"visibility":"private","name":"ignored"}),
        json!({"visibility":"private","privacy":10}),
        json!({"visibility":"private","url":"https://evil.test/"}),
    ] {
        let (status, headers, value) = json_request_with_headers(
            app.clone(),
            Method::PUT,
            "/v1/playlists/kuwo:101/visibility",
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    }
    let response = app
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri("/v1/playlists/kuwo:101/visibility")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"visibility":"public","visibility":"private"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn kuwo_playlist_mutation_preflight_errors_are_private_and_do_not_reach_provider() {
    let mut registry = ProviderRegistry::new();
    registry
        .register(Provider {
            account: "default",
            fail: false,
            reject_before_write: false,
        })
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Kuwo));
    let cases = [
        (Method::POST, "/v1/playlists", Some(r#"{"unknown":true}"#)),
        (
            Method::PATCH,
            "/v1/playlists/kuwo:101",
            Some(r#"{"unknown":true}"#),
        ),
        (Method::PATCH, "/v1/playlists/invalid", Some(r#"{}"#)),
        (Method::DELETE, "/v1/playlists/kuwo:101?unknown=yes", None),
        (Method::DELETE, "/v1/playlists", Some(r#"{"unknown":true}"#)),
        (
            Method::POST,
            "/v1/playlists/kuwo:101/items",
            Some(r#"{"unknown":true}"#),
        ),
        (
            Method::PUT,
            "/v1/playlists/kuwo:101/tracks/order",
            Some(r#"{"unknown":true}"#),
        ),
    ];
    for (method, uri, body) in cases {
        let request = Request::builder()
            .method(&method)
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(body.map_or_else(Body::empty, Body::from))
            .unwrap();
        let response = app.clone().oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{method} {uri}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
}

struct UnsupportedVisibility;
#[async_trait]
impl MusicProvider for UnsupportedVisibility {
    fn platform(&self) -> Platform {
        Platform::Soda
    }
    fn name(&self) -> &'static str {
        "metadata-only fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::PlaylistWrite])
    }
    async fn update_playlist(
        &self,
        _: &str,
        _: &PlaylistUpdateRequest,
    ) -> Result<PlaylistMutationResult> {
        panic!("visibility must never silently fall back to metadata update");
    }
}

struct UnsupportedCollectedOrder;
#[async_trait]
impl MusicProvider for UnsupportedCollectedOrder {
    fn platform(&self) -> Platform {
        Platform::Soda
    }
    fn name(&self) -> &'static str {
        "ordinary ordering and collection fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::PlaylistWrite,
            Capability::PlaylistSubscriptionWrite,
        ])
    }
    async fn reorder_account_playlists(
        &self,
        _: &PlaylistOrderRequest,
    ) -> Result<PlaylistOrderResult> {
        panic!("collected ordering must not fall back to ordinary ordering");
    }
    async fn set_playlist_subscription(
        &self,
        _: &str,
        _: bool,
        _: Option<&str>,
    ) -> Result<SubscriptionResult> {
        panic!("the static order route must not reach the dynamic collection route");
    }
}

#[tokio::test]
async fn kuwo_collected_order_new_contract_has_no_default_fallback_or_route_ambiguity() {
    let mut registry = ProviderRegistry::new();
    registry.register(UnsupportedCollectedOrder).unwrap();
    let app = build_router(AppState::new(registry, Platform::Soda));
    let (status, headers, value) = json_request_with_headers(
        app.clone(),
        Method::PUT,
        "/v1/account/favorites/playlists/order",
        Some(json!({"refs":["soda:101","soda:102"]})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{value}");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(value["error"]["code"], "capability_not_supported");
    assert!(
        value
            .to_string()
            .contains("playlist_collection_order_write")
    );
    for method in [Method::GET, Method::POST, Method::DELETE] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri("/v1/account/favorites/playlists/order")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
}

#[tokio::test]
async fn kuwo_collected_order_http_preserves_reference_aliases_and_rejects_invalid_bodies() {
    let mut registry = ProviderRegistry::new();
    registry
        .register(Provider {
            account: "default",
            fail: false,
            reject_before_write: false,
        })
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Kuwo));
    for body in [
        json!({}),
        json!({"refs":[]}),
        json!({"refs":null}),
        json!({"refs":["kuwo:102","soda:101"]}),
        json!({"refs":["kuwo:102","kuwo:101"],"section":"created"}),
        json!({"refs":["kuwo:102","kuwo:101"],"account":3}),
    ] {
        let (status, headers, value) = json_request_with_headers(
            app.clone(),
            Method::PUT,
            "/v1/account/favorites/playlists/order",
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{value}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    }
    for body in [
        json!({"playlist_refs":["kuwo:102","kuwo:101"]}),
        json!({"platform":"kuwo","ids":[102,101]}),
    ] {
        let (status, headers, value) = json_request_with_headers(
            app.clone(),
            Method::PUT,
            "/v1/account/favorites/playlists/order",
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(
            value["data"]["playlist_refs"],
            json!(["kuwo:102", "kuwo:101"])
        );
        assert_eq!(value["data"]["extensions"]["library_section"], "collected");
    }
}

#[tokio::test]
async fn kuwo_visibility_new_contract_is_explicitly_unsupported_by_other_provider_defaults() {
    let mut registry = ProviderRegistry::new();
    registry.register(UnsupportedVisibility).unwrap();
    let app = build_router(AppState::new(registry, Platform::Soda));
    for target in ["public", "private"] {
        let (status, headers, value) = json_request_with_headers(
            app.clone(),
            Method::PUT,
            "/v1/playlists/soda:101/visibility",
            Some(json!({"visibility":target})),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{value}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(value["error"]["code"], "capability_not_supported");
        assert!(value.to_string().contains("playlist_visibility_write"));
    }
}

#[tokio::test]
async fn kuwo_management_http_default_named_caller_success_and_uncertainty_are_private() {
    for scope in ["default", "named", "caller"] {
        for fail in [false, true] {
            for (method, path, mut body) in requests() {
                let mut registry = ProviderRegistry::new();
                registry
                    .register(Provider {
                        account: if scope == "named" {
                            "personal"
                        } else {
                            "default"
                        },
                        fail,
                        reject_before_write: false,
                    })
                    .unwrap();
                let app = build_router(AppState::new(registry, Platform::Kuwo));
                let path = if scope == "named" && body.is_none() {
                    format!("{path}?account=personal")
                } else {
                    path.into()
                };
                if scope == "named" {
                    if let Some(body) = &mut body {
                        body["account"] = json!("personal");
                    }
                }
                let mut request = Request::builder()
                    .method(method.clone())
                    .uri(&path)
                    .header(header::CONTENT_TYPE, "application/json");
                if scope == "caller" {
                    let credential = CallerCredential::issue(
                        &ProviderCredential::new(
                            Platform::Kuwo,
                            "fixture",
                            "private-management-fixture",
                            None,
                        )
                        .unwrap(),
                    )
                    .unwrap();
                    request = request.header(CALLER_CREDENTIAL_HEADER, credential.value);
                }
                let response = app
                    .oneshot(
                        request
                            .body(body.map_or_else(Body::empty, |v| Body::from(v.to_string())))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    if fail {
                        StatusCode::CONFLICT
                    } else {
                        StatusCode::OK
                    }
                );
                assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
                assert!(
                    response
                        .headers()
                        .get(caller_scope::UPDATED_CREDENTIAL_HEADER)
                        .is_none()
                );
                let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
                assert!(!String::from_utf8_lossy(&bytes).contains("private-management-fixture"));
                let value: Value = serde_json::from_slice(&bytes).unwrap();
                if fail {
                    assert_eq!(value["error"]["details"]["write_outcome"], "unconfirmed");
                    assert_eq!(value["error"]["retryable"], false);
                } else {
                    assert_eq!(value["data"]["extensions"]["confirmed"], true);
                    if path.starts_with("/v1/account/favorites/playlists/order") {
                        assert_eq!(
                            value["data"]["playlist_refs"],
                            json!(["kuwo:102", "kuwo:101"])
                        );
                        assert_eq!(value["data"]["extensions"]["library_section"], "collected");
                    }
                    if path.starts_with("/v1/account/playlists/order") {
                        assert_eq!(
                            value["data"]["playlist_refs"],
                            json!(["kuwo:102", "kuwo:101"])
                        );
                    }
                    if path.contains("/tracks/order") {
                        assert_eq!(
                            value["data"]["track_refs"],
                            json!(["kuwo:22", "kuwo:33", "kuwo:11", "kuwo:22"])
                        );
                        assert_eq!(value["data"]["snapshot_id"], "confirmed-order");
                    }
                    if path.contains("/account/favorites/playlists/kuwo:") {
                        assert_eq!(value["data"]["subscribed"], method == Method::PUT);
                        assert_eq!(value["data"]["resource_ref"], "kuwo:999");
                        assert_eq!(value["data"]["extensions"]["library_section"], "collected");
                    }
                    if path.contains("/account/favorites/tracks/") {
                        assert_eq!(value["data"]["subscribed"], method == Method::PUT);
                        assert_eq!(
                            value["data"]["extensions"]["favorite_playlist_ref"],
                            "kuwo:901"
                        );
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn kuwo_management_real_provider_missing_credentials_and_unsupported_creation_never_send() {
    let guard = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    guard.set_nonblocking(true).unwrap();
    let p = tuneweave_provider_kuwo::KuwoProvider::new(tuneweave_provider_kuwo::KuwoConfig {
        proxy_url: Some(format!("http://{}", guard.local_addr().unwrap())),
        ..Default::default()
    })
    .unwrap();
    assert!(p.capabilities().contains(&Capability::PlaylistWrite));
    assert!(
        p.capabilities()
            .contains(&Capability::PlaylistVisibilityWrite)
    );
    let mut registry = ProviderRegistry::new();
    registry.register(p).unwrap();
    let app = build_router(AppState::new(registry, Platform::Kuwo));
    for (method, path, body) in requests() {
        let (status, headers, value) =
            json_request_with_headers(app.clone(), method, path, body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{value}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(value["error"]["code"], "authentication_required");
    }
    let (status, headers, value) = json_request_with_headers(
        app.clone(),
        Method::POST,
        "/v1/playlists",
        Some(json!({"platform":"kuwo","name":"New playlist","visibility":"private"})),
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{value}");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    for (path, body, expected) in [
        (
            "/v1/playlists/kuwo:101/tracks",
            json!({"refs":["kuwo:44","kuwo:44"]}),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/v1/playlists/kuwo:101/videos",
            json!({"refs":["kuwo:44"]}),
            StatusCode::UNPROCESSABLE_ENTITY,
        ),
    ] {
        let (status, headers, value) =
            json_request_with_headers(app.clone(), Method::POST, path, Some(body)).await;
        assert_eq!(status, expected, "{value}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    }
    assert_eq!(
        guard.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

fn cover_bytes() -> Vec<u8> {
    vec![
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 6,
        0, 0, 0, 31, 21, 196, 137, 0, 0, 0, 13, 73, 68, 65, 84, 120, 156, 99, 248, 207, 192, 240,
        31, 0, 5, 0, 1, 255, 137, 153, 61, 29, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ]
}

#[tokio::test]
async fn kuwo_cover_http_binary_body_scopes_and_two_write_uncertainty_are_private() {
    for scope in ["default", "named", "caller"] {
        for fail in [false, true] {
            let mut registry = ProviderRegistry::new();
            registry
                .register(Provider {
                    account: if scope == "named" {
                        "personal"
                    } else {
                        "default"
                    },
                    fail,
                    reject_before_write: false,
                })
                .unwrap();
            let app = build_router(AppState::new(registry, Platform::Kuwo));
            let mut path =
                "/v1/playlists/kuwo:101/cover?filename=cover.png&image_size=1&crop_x=0&crop_y=0"
                    .to_owned();
            if scope == "named" {
                path.push_str("&account=personal");
            }
            let mut r = Request::builder()
                .method(Method::PUT)
                .uri(path)
                .header(header::CONTENT_TYPE, "image/png");
            if scope == "caller" {
                let c = CallerCredential::issue(
                    &ProviderCredential::new(
                        Platform::Kuwo,
                        "fixture",
                        "private-management-fixture",
                        None,
                    )
                    .unwrap(),
                )
                .unwrap();
                r = r.header(CALLER_CREDENTIAL_HEADER, c.value);
            }
            let response = app
                .oneshot(r.body(Body::from(cover_bytes())).unwrap())
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                if fail {
                    StatusCode::CONFLICT
                } else {
                    StatusCode::OK
                }
            );
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert!(
                response
                    .headers()
                    .get(caller_scope::UPDATED_CREDENTIAL_HEADER)
                    .is_none()
            );
            let b = to_bytes(response.into_body(), 65536).await.unwrap();
            assert!(!String::from_utf8_lossy(&b).contains("private-management-fixture"));
            let v: Value = serde_json::from_slice(&b).unwrap();
            if fail {
                assert_eq!(v["error"]["details"]["write_requests_dispatched"], 2);
                assert_eq!(v["error"]["details"]["upload_outcome"], "confirmed");
                assert_eq!(v["error"]["retryable"], false);
            } else {
                assert_eq!(v["data"]["playlist_ref"], "kuwo:101");
                assert_eq!(v["data"]["image"]["extensions"]["width"], 700);
                assert_eq!(v["data"]["extensions"]["write_requests_dispatched"], 2);
            }
        }
    }
}

#[tokio::test]
async fn kuwo_cover_http_preselection_errors_and_missing_real_credentials_never_send() {
    let guard = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    guard.set_nonblocking(true).unwrap();
    let p = tuneweave_provider_kuwo::KuwoProvider::new(tuneweave_provider_kuwo::KuwoConfig {
        proxy_url: Some(format!("http://{}", guard.local_addr().unwrap())),
        ..Default::default()
    })
    .unwrap();
    let mut registry = ProviderRegistry::new();
    registry.register(p).unwrap();
    let app = build_router(AppState::new(registry, Platform::Kuwo));
    for (method, path, mime, status) in [
        (
            Method::PUT,
            "/v1/playlists/kuwo:101/cover?filename=cover.png",
            "image/png",
            StatusCode::UNAUTHORIZED,
        ),
        (
            Method::PUT,
            "/v1/playlists/kuwo:101/cover?unknown=yes",
            "image/png",
            StatusCode::BAD_REQUEST,
        ),
        (
            Method::PUT,
            "/v1/playlists/invalid/cover",
            "image/png",
            StatusCode::BAD_REQUEST,
        ),
        (
            Method::PUT,
            "/v1/playlists/kuwo:101/cover?image_size=bad",
            "image/png",
            StatusCode::BAD_REQUEST,
        ),
        (
            Method::GET,
            "/v1/playlists/kuwo:101/cover",
            "image/png",
            StatusCode::METHOD_NOT_ALLOWED,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header(header::CONTENT_TYPE, mime)
                    .body(Body::from(cover_bytes()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{path}");
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "no-store",
            "{path}"
        );
    }
    assert_eq!(
        guard.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn kuwo_submission_preflight_rejection_is_private_and_not_an_uncertain_write() {
    // Protocol tests cover the actual published-state and retained-song rules.
    // This contract fixture checks that every HTTP adapter preserves their error.
    for scope in ["default", "named", "caller"] {
        for (method, path, mut body) in requests().into_iter().skip(8).take(6) {
            let mut registry = ProviderRegistry::new();
            registry
                .register(Provider {
                    account: if scope == "named" {
                        "personal"
                    } else {
                        "default"
                    },
                    fail: false,
                    reject_before_write: true,
                })
                .unwrap();
            let app = build_router(AppState::new(registry, Platform::Kuwo));
            if scope == "named" {
                body.as_mut().unwrap()["account"] = json!("personal");
            }
            let mut request = Request::builder()
                .method(method)
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json");
            if scope == "caller" {
                let credential = CallerCredential::issue(
                    &ProviderCredential::new(
                        Platform::Kuwo,
                        "fixture",
                        "private-management-fixture",
                        None,
                    )
                    .unwrap(),
                )
                .unwrap();
                request = request.header(CALLER_CREDENTIAL_HEADER, credential.value);
            }
            let response = app
                .oneshot(request.body(Body::from(body.unwrap().to_string())).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert!(
                response
                    .headers()
                    .get(caller_scope::UPDATED_CREDENTIAL_HEADER)
                    .is_none()
            );
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("private-management-fixture"));
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(value["error"]["code"], "capability_not_supported");
            assert_eq!(value["error"]["retryable"], false);
            assert!(value.get("data").is_none_or(Value::is_null));
            assert!(value["error"]["details"].get("write_outcome").is_none());
            assert!(
                value["error"]["details"]
                    .get("write_requests_dispatched")
                    .is_none()
            );
        }
    }
}
