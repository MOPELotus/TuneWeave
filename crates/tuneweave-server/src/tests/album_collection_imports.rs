//! HTTP import contracts. Each provider separately tests its official collection protocol.
use super::*;

#[derive(Clone)]
struct Provider {
    platform: Platform,
    source_type: &'static str,
    caller: bool,
    changed: bool,
    update: Arc<Mutex<Option<ProviderCredential>>>,
}
impl Provider {
    fn accept(&self, id: &str, kind: &str, account: Option<&str>) -> Result<()> {
        if id != "111" {
            return Err(TuneWeaveError::new(
                ErrorCode::PermissionDenied,
                "Collection source belongs to another account",
            )
            .with_platform(self.platform));
        }
        assert_eq!(kind, self.source_type);
        assert_eq!(
            account,
            Some(if self.caller { "default" } else { "personal" })
        );
        if self.caller {
            *self.update.lock().unwrap() = Some(
                ProviderCredential::new(self.platform, "test", "private-rotated", None).unwrap(),
            );
        }
        Ok(())
    }
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        self.platform
    }
    fn name(&self) -> &'static str {
        "Account collection import fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        let mut capabilities = BTreeSet::from([
            Capability::PlaylistRead,
            Capability::CallerManagedCredentials,
        ]);
        match self.source_type {
            "purchased_tracks" => {
                capabilities.insert(Capability::AccountPurchasedTracks);
            }
            "purchased_albums" => {
                capabilities.insert(Capability::AccountPurchasedAlbums);
            }
            _ => {}
        }
        capabilities
    }
    fn with_caller_credential(
        &self,
        credential: &ProviderCredential,
    ) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(credential.platform, self.platform);
        assert_eq!(credential.secret(), "private-input");
        Ok(Arc::new(Self {
            caller: true,
            update: Arc::default(),
            ..self.clone()
        }))
    }
    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        Ok(self.update.lock().unwrap().take())
    }
    async fn playlist_source(
        &self,
        id: &str,
        kind: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        self.accept(id, kind, account)?;
        let mut playlist = sample_playlist(id);
        playlist.platform = self.platform;
        playlist.resource_ref = ResourceRef::new(self.platform, id).unwrap();
        playlist.track_count = Some(3);
        playlist.extensions = Extensions::from([("source_snapshot_id".into(), json!("version-a"))]);
        Ok(playlist)
    }
    async fn playlist_source_items(
        &self,
        id: &str,
        kind: &str,
        request: &PageRequest,
    ) -> Result<Page<PlaylistPlayableItem>> {
        self.accept(id, kind, request.account.as_deref())?;
        assert_eq!(request.limit, 100);
        let ids = if request.offset == 0 {
            vec!["50", "50"]
        } else {
            assert_eq!(request.offset, 2);
            vec!["51"]
        };
        Ok(Page {
            items: ids
                .into_iter()
                .map(|id| {
                    PlaylistPlayableItem::Track(Track::new(
                        ResourceRef::new(self.platform, id).unwrap(),
                        "Synthetic song",
                    ))
                })
                .collect(),
            pagination: PageMeta {
                limit: 100,
                offset: request.offset,
                total: Some(3),
                has_more: request.offset == 0,
                next_offset: (request.offset == 0).then_some(2),
                extensions: Extensions::from([(
                    "source_snapshot_id".into(),
                    json!(if self.changed && request.offset == 2 {
                        "version-b"
                    } else {
                        "version-a"
                    }),
                )]),
            },
        })
    }
}

fn app(platform: Platform, source_type: &'static str, changed: bool) -> Router {
    let mut registry = ProviderRegistry::new();
    registry
        .register(Provider {
            platform,
            source_type,
            caller: false,
            changed,
            update: Arc::default(),
        })
        .unwrap();
    build_router(AppState::new(registry, platform))
}
fn request(platform: Platform, kind: &str, caller: bool, materialize: bool) -> Request<Body> {
    request_for_user(platform, kind, caller, materialize, "111")
}

fn request_for_user(
    platform: Platform,
    kind: &str,
    caller: bool,
    materialize: bool,
    uid: &str,
) -> Request<Body> {
    let mut source = json!({"platform":platform,"id":uid,"type":kind});
    if !caller {
        source["account"] = json!("personal");
    }
    let body = json!({"name":"Collection songs", "sources":[source]});
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(if materialize {
            "/v1/uni/materialize/imports"
        } else {
            "/v1/uni/playlists/imports"
        })
        .header(header::CONTENT_TYPE, "application/json");
    if caller {
        let credential = ProviderCredential::new(platform, "test", "private-input", None).unwrap();
        request = request.header(
            CALLER_CREDENTIAL_HEADER,
            CallerCredential::issue(&credential).unwrap().value,
        );
    }
    request.body(Body::from(body.to_string())).unwrap()
}

#[tokio::test]
async fn purchased_track_imports_reject_foreign_owner_without_creating_or_exporting_credentials() {
    for caller in [false, true] {
        for materialize in [false, true] {
            let router = app(Platform::Kugou, "purchased_tracks", false);
            let response = router
                .clone()
                .oneshot(request_for_user(
                    Platform::Kugou,
                    "purchased_tracks",
                    caller,
                    materialize,
                    "222",
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN);
            assert!(
                !response
                    .headers()
                    .contains_key(caller_scope::UPDATED_CREDENTIAL_HEADER)
            );
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("private-"));
            let (status, playlists) = json_response_from(router, "/v1/uni/playlists").await;
            assert_eq!(status, StatusCode::OK);
            assert!(playlists["data"].as_array().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn migu_purchase_import_sources_are_gated_by_their_capabilities() {
    let mut registry = ProviderRegistry::new();
    registry
        .register(tuneweave_provider_migu::MiguProvider::new(Default::default()).unwrap())
        .unwrap();
    let router = build_router(AppState::new(registry, Platform::Migu));

    for (kind, capability) in [
        ("purchased_tracks", "account_purchased_tracks"),
        ("purchased_albums", "account_purchased_albums"),
    ] {
        for materialize in [false, true] {
            let request = Request::builder()
                .method(Method::POST)
                .uri(if materialize {
                    "/v1/uni/materialize/imports"
                } else {
                    "/v1/uni/playlists/imports"
                })
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    json!({
                        "name": "Not created",
                        "sources": [{"platform": "migu", "id": "111", "type": kind}]
                    })
                    .to_string(),
                ))
                .unwrap();
            let response = router.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
            assert!(
                !response
                    .headers()
                    .contains_key(caller_scope::UPDATED_CREDENTIAL_HEADER)
            );
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            let error: Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(error["error"]["code"], "capability_not_supported");
            assert_eq!(error["error"]["details"]["capability"], capability);
        }
    }
}

#[tokio::test]
async fn album_collection_imports_preserve_source_type_order_and_duplicates_in_both_modes() {
    for (platform, kind) in [
        (Platform::Soda, "collected_albums"),
        (Platform::Migu, "favorite_albums"),
        (Platform::Migu, "purchased_albums"),
        (Platform::Kugou, "purchased_tracks"),
        (Platform::Kugou, "purchased_albums"),
    ] {
        for caller in [false, true] {
            for materialize in [false, true] {
                let router = app(platform, kind, false);
                let response = router
                    .clone()
                    .oneshot(request(platform, kind, caller, materialize))
                    .await
                    .unwrap();
                let status = response.status();
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
                    caller
                );
                let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
                let body: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(status, StatusCode::OK, "{body}");
                assert_eq!(body["data"]["sources"][0]["type"], kind);
                for secret in ["private-input", "private-rotated"] {
                    assert!(!body.to_string().contains(secret));
                }
                let items = if materialize {
                    // The existing wire contract keeps the field as null when redacted.
                    assert!(body["data"]["sources"][0]["account"].is_null());
                    assert!(!body.to_string().contains("personal"));
                    body["data"]["items"].clone()
                } else {
                    let reference = body["data"]["playlist"]["ref"].as_str().unwrap();
                    let (status, entries) =
                        json_response_from(router, &format!("/v1/uni/playlists/{reference}/items"))
                            .await;
                    assert_eq!(status, StatusCode::OK);
                    entries["data"].clone()
                };
                let items = items.as_array().unwrap();
                assert_eq!(items.len(), 3);
                for (index, item) in items.iter().enumerate() {
                    assert_eq!(item["extensions"]["import_source_type"], kind);
                    assert_eq!(
                        item["source_ref"],
                        format!("{platform}:{}", if index < 2 { "50" } else { "51" })
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn album_collection_imports_reject_changed_later_pages_without_creating_a_playlist() {
    for (platform, kind) in [
        (Platform::Soda, "collected_albums"),
        (Platform::Migu, "purchased_albums"),
        (Platform::Kugou, "purchased_tracks"),
        (Platform::Kugou, "purchased_albums"),
    ] {
        for materialize in [false, true] {
            let router = app(platform, kind, true);
            let response = router
                .clone()
                .oneshot(request(platform, kind, false, materialize))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
            let (status, playlists) = json_response_from(router, "/v1/uni/playlists").await;
            assert_eq!(status, StatusCode::OK);
            assert!(playlists["data"].as_array().unwrap().is_empty());
        }
    }
}
