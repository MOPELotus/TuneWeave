use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct SnapshotProvider {
    caller: bool,
    mode: &'static str,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl MusicProvider for SnapshotProvider {
    fn platform(&self) -> Platform {
        Platform::Soda
    }
    fn name(&self) -> &'static str {
        "Snapshot playlist test provider"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::PlaylistRead,
            Capability::AlbumDetail,
            Capability::Favorites,
            Capability::TrackSubscriptionWrite,
            Capability::AccountAlbums,
            Capability::AlbumSubscriptionWrite,
            Capability::CallerManagedCredentials,
        ])
    }
    fn with_caller_credential(
        &self,
        credential: &ProviderCredential,
    ) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(credential.platform, Platform::Soda);
        assert_eq!(credential.secret(), "snapshot-original");
        Ok(Arc::new(Self {
            caller: true,
            mode: self.mode,
            calls: self.calls.clone(),
        }))
    }
    async fn playlist(&self, id: &str, account: Option<&str>) -> Result<Playlist> {
        assert_eq!(
            account,
            if self.caller {
                Some("default")
            } else {
                Some("personal")
            }
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Playlist {
            resource_ref: ResourceRef::new(Platform::Soda, id).unwrap(),
            platform: Platform::Soda,
            id: id.to_owned(),
            name: "Account playlist".to_owned(),
            description: "Order and duplicates".to_owned(),
            cover_url: None,
            creator: None,
            track_count: Some(3),
            tags: Vec::new(),
            subscribed: None,
            created_at: None,
            updated_at: None,
            extensions: Extensions::from([("source_snapshot_id".to_owned(), json!("revision-a"))]),
        })
    }
    async fn playlist_tracks(&self, _id: &str, request: &PageRequest) -> Result<Page<Track>> {
        assert_eq!(
            request.account.as_deref(),
            if self.caller {
                Some("default")
            } else {
                Some("personal")
            }
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        if request.offset == 2 && self.mode == "auth" {
            return Err(
                TuneWeaveError::new(ErrorCode::AuthenticationRequired, "session expired")
                    .with_platform(Platform::Soda),
            );
        }
        let (ids, next) = match request.offset {
            0 => (vec!["11", "22"], Some(2)),
            2 => (vec!["22"], None),
            _ => panic!("unexpected source offset"),
        };
        let mut extensions =
            Extensions::from([("source_snapshot_id".to_owned(), json!("revision-a"))]);
        if request.offset == 2 {
            match self.mode {
                "changed" => {
                    extensions.insert("source_snapshot_id".to_owned(), json!("revision-b"));
                }
                "missing" => {
                    extensions.remove("source_snapshot_id");
                }
                _ => (),
            }
        }
        Ok(Page {
            items: ids
                .into_iter()
                .map(|id| Track::new(ResourceRef::new(Platform::Soda, id).unwrap(), "Song"))
                .collect(),
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(3),
                has_more: next.is_some(),
                next_offset: next,
                extensions,
            },
        })
    }
    async fn favorite_playlist(&self, account: Option<&str>) -> Result<Playlist> {
        self.playlist("7200303561195061287", account).await
    }
    async fn album(&self, id: &str, account: Option<&str>) -> Result<Album> {
        assert_eq!(id, "900");
        let metadata = self.playlist(id, account).await?;
        Ok(Album {
            resource_ref: metadata.resource_ref,
            platform: Platform::Soda,
            id: metadata.id,
            name: "Account album".to_owned(),
            aliases: Vec::new(),
            artists: Vec::new(),
            description: metadata.description,
            cover_url: None,
            published_at: None,
            track_count: metadata.track_count,
            company: None,
            kind: None,
            extensions: metadata.extensions,
        })
    }
    async fn album_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        assert_eq!(id, "900");
        self.playlist_tracks(id, request).await
    }
    async fn favorite_tracks(&self, request: &PageRequest) -> Result<Page<Track>> {
        self.playlist_tracks("7200303561195061287", request).await
    }
    async fn user_favorite_playlist(
        &self,
        user_id: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        assert_eq!(user_id, "123456");
        self.favorite_playlist(account).await
    }
    async fn user_favorite_tracks(
        &self,
        user_id: &str,
        request: &PageRequest,
    ) -> Result<Page<Track>> {
        assert_eq!(user_id, "123456");
        self.favorite_tracks(request).await
    }
    async fn playlist_source(
        &self,
        id: &str,
        source_type: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        match source_type {
            "favorite_tracks" => self.user_favorite_playlist(id, account).await,
            "playlist" => self.playlist(id, account).await,
            "album" => {
                let album = self.album(id, account).await?;
                let mut extensions = album.extensions;
                extensions.insert("source_type".to_owned(), json!("album"));
                Ok(Playlist {
                    resource_ref: album.resource_ref,
                    platform: album.platform,
                    id: album.id,
                    name: album.name,
                    description: album.description,
                    cover_url: album.cover_url,
                    creator: None,
                    track_count: album.track_count,
                    tags: Vec::new(),
                    subscribed: None,
                    created_at: None,
                    updated_at: None,
                    extensions,
                })
            }
            _ => panic!("unexpected source type"),
        }
    }
    async fn playlist_source_items(
        &self,
        id: &str,
        source_type: &str,
        request: &PageRequest,
    ) -> Result<Page<PlaylistPlayableItem>> {
        let page = match source_type {
            "favorite_tracks" => self.user_favorite_tracks(id, request).await?,
            "album" => self.album_tracks(id, request).await?,
            "playlist" => self.playlist_tracks(id, request).await?,
            _ => panic!("unexpected source type"),
        };
        Ok(Page {
            items: page
                .items
                .into_iter()
                .map(PlaylistPlayableItem::Track)
                .collect(),
            pagination: page.pagination,
        })
    }
    async fn set_track_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        self.favorite_playlist(account).await?;
        Ok(SubscriptionResult {
            resource_ref: ResourceRef::new(Platform::Soda, id).unwrap(),
            subscribed,
            extensions: Extensions::new(),
        })
    }
    async fn account_albums(&self, request: &PageRequest) -> Result<Page<Album>> {
        assert_eq!(
            request.account.as_deref(),
            Some(if self.caller { "default" } else { "personal" })
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(Page {
            items: vec![Album {
                resource_ref: ResourceRef::new(Platform::Soda, "11").unwrap(),
                platform: Platform::Soda,
                id: "11".to_owned(),
                name: "Saved album".to_owned(),
                aliases: Vec::new(),
                artists: Vec::new(),
                description: String::new(),
                cover_url: None,
                published_at: None,
                track_count: None,
                company: None,
                kind: None,
                extensions: Extensions::from([("subscribed".to_owned(), json!(true))]),
            }],
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(1),
                has_more: false,
                next_offset: None,
                extensions: Extensions::from([("complete_snapshot".to_owned(), json!(true))]),
            },
        })
    }
    async fn user_favorite_albums(
        &self,
        user_id: &str,
        request: &PageRequest,
    ) -> Result<Page<Album>> {
        assert_eq!(user_id, "123456");
        self.account_albums(request).await
    }
    async fn set_album_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        assert_eq!(
            account,
            Some(if self.caller { "default" } else { "personal" })
        );
        self.calls.fetch_add(1, Ordering::SeqCst);
        Ok(SubscriptionResult {
            resource_ref: ResourceRef::new(Platform::Soda, id).unwrap(),
            subscribed,
            extensions: Extensions::new(),
        })
    }
    async fn set_album_subscriptions(
        &self,
        ids: &[String],
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<Vec<SubscriptionResult>> {
        let mut results = Vec::new();
        for id in ids {
            let result = self.set_album_subscription(id, subscribed, account).await?;
            if id == "22" && self.mode != "stable" {
                let code = match self.mode {
                    "auth" => ErrorCode::AuthenticationRequired,
                    "conflict" => ErrorCode::Conflict,
                    _ => ErrorCode::UpstreamError,
                };
                return Err(TuneWeaveError::new(code, "album batch was not confirmed").with_platform(Platform::Soda).with_details(json!({
                    "write_outcome":"unconfirmed", "atomic":false, "completed_refs":["soda:11"], "failed_ref":"soda:22", "remaining_refs":["soda:33"]
                })));
            }
            results.push(result);
        }
        Ok(results)
    }
    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        self.caller
            .then(|| {
                ProviderCredential::new(
                    Platform::Soda,
                    "test",
                    format!("snapshot-rotation-{}", self.calls.load(Ordering::SeqCst)),
                    None,
                )
            })
            .transpose()
    }
}

async fn exercise_import(mode: &'static str, caller: bool, materialize: bool, source_type: &str) {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = ProviderRegistry::new();
    registry
        .register(SnapshotProvider {
            caller: false,
            mode,
            calls: calls.clone(),
        })
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Soda));
    let path = if materialize {
        "/v1/uni/materialize/imports"
    } else {
        "/v1/uni/playlists/imports"
    };
    let reference = match source_type {
        "playlist" => "soda:7200303561195061287",
        "favorite_tracks" => "soda:123456",
        "album" => "soda:900",
        _ => panic!("unexpected source type"),
    };
    let mut source = json!({"ref":reference,"type":source_type});
    if !caller {
        source["account"] = json!("personal");
    }
    let body = json!({"sources":[source]});
    let mut request = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if caller {
        let credential = CallerCredential::issue(
            &ProviderCredential::new(Platform::Soda, "test", "snapshot-original", None).unwrap(),
        )
        .unwrap();
        request = request.header(CALLER_CREDENTIAL_HEADER, credential.value);
    }
    let response = app
        .clone()
        .oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert!(
        response.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("no-store")
    );
    let updated = response
        .headers()
        .get(caller_scope::UPDATED_CREDENTIAL_HEADER);
    if caller && mode != "auth" {
        let value = updated.unwrap();
        assert!(value.is_sensitive());
        let credential =
            CallerCredential::parse(value.to_str().unwrap().strip_prefix("soda=").unwrap())
                .unwrap();
        assert_eq!(credential.secret(), "snapshot-rotation-3");
    } else {
        assert!(updated.is_none());
    }
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("snapshot-rotation"));
    let result: Value = serde_json::from_slice(&bytes).unwrap();
    let (_, directory) = json_response_from(app.clone(), "/v1/uni/playlists").await;
    if mode != "stable" {
        assert_ne!(status, StatusCode::OK, "{result}");
        assert_eq!(
            result["error"]["code"],
            if mode == "auth" {
                "authentication_required"
            } else {
                "upstream_error"
            }
        );
        assert_eq!(directory["data"], json!([]));
        return;
    }
    assert_eq!(status, StatusCode::OK, "{result}");
    let items = if materialize {
        assert_eq!(directory["data"], json!([]));
        assert_eq!(result["data"]["item_count"], 3);
        result["data"]["items"].clone()
    } else {
        assert_eq!(directory["data"].as_array().unwrap().len(), 1);
        assert_eq!(result["data"]["playlist"]["item_count"], 3);
        let reference = result["data"]["playlist"]["ref"].as_str().unwrap();
        let (_, items) =
            json_response_from(app, &format!("/v1/uni/playlists/{reference}/items")).await;
        items["data"].clone()
    };
    assert_eq!(items[0]["source_ref"], "soda:11");
    assert_eq!(items[1]["source_ref"], "soda:22");
    assert_eq!(items[2]["source_ref"], "soda:22");
    assert_ne!(items[1]["id"], items[2]["id"]);
    assert_eq!(items[2]["position"], 2);
    assert!(!items.to_string().contains("twc1_"));
    assert!(!items.to_string().contains("personal"));
}

#[tokio::test]
async fn account_playlist_imports_keep_complete_snapshot_order_and_latest_caller_rotation() {
    for caller in [false, true] {
        for materialize in [false, true] {
            for source_type in ["playlist", "favorite_tracks", "album"] {
                exercise_import("stable", caller, materialize, source_type).await;
            }
        }
    }
}

#[tokio::test]
async fn account_playlist_imports_fail_atomically_when_source_revision_changes_or_session_expires()
{
    for mode in ["changed", "missing", "auth"] {
        for caller in [false, true] {
            for materialize in [false, true] {
                for source_type in ["playlist", "favorite_tracks", "album"] {
                    exercise_import(mode, caller, materialize, source_type).await;
                }
            }
        }
    }
}

#[tokio::test]
async fn account_album_http_details_and_tracks_preserve_scope_snapshot_and_caller_rotation() {
    for caller in [false, true] {
        for tracks in [false, true] {
            let calls = Arc::new(AtomicUsize::new(0));
            let mut registry = ProviderRegistry::new();
            registry
                .register(SnapshotProvider {
                    caller: false,
                    mode: "stable",
                    calls: calls.clone(),
                })
                .unwrap();
            let app = build_router(AppState::new(registry, Platform::Soda));
            let mut path = if tracks {
                "/v1/albums/soda:900/tracks?limit=2&offset=0".to_owned()
            } else {
                "/v1/albums/soda:900?".to_owned()
            };
            if !caller {
                path.push_str("&account=personal");
            }
            let mut request = Request::builder().uri(path);
            if caller {
                let credential = CallerCredential::issue(
                    &ProviderCredential::new(Platform::Soda, "test", "snapshot-original", None)
                        .unwrap(),
                )
                .unwrap();
                request = request.header(CALLER_CREDENTIAL_HEADER, credential.value);
            }
            let response = app
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert!(
                response.headers()[header::CACHE_CONTROL]
                    .to_str()
                    .unwrap()
                    .contains("no-store")
            );
            let update = response
                .headers()
                .get(caller_scope::UPDATED_CREDENTIAL_HEADER);
            if caller {
                let update = update.unwrap();
                assert!(update.is_sensitive());
                let parsed = CallerCredential::parse(
                    update.to_str().unwrap().strip_prefix("soda=").unwrap(),
                )
                .unwrap();
                assert_eq!(parsed.secret(), "snapshot-rotation-1");
            } else {
                assert!(update.is_none());
            }
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            let result: Value = serde_json::from_slice(&bytes).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("snapshot-rotation"));
            assert_eq!(result["meta"]["platform"], "soda");
            if tracks {
                assert_eq!(result["data"][0]["ref"], "soda:11");
                assert_eq!(result["data"][1]["ref"], "soda:22");
                let page = &result["meta"]["pagination"];
                assert_eq!(page["limit"], 2);
                assert_eq!(page["offset"], 0);
                assert_eq!(page["total"], 3);
                assert_eq!(page["next_offset"], 2);
                assert_eq!(page["extensions"]["source_snapshot_id"], "revision-a");
            } else {
                assert_eq!(result["data"]["ref"], "soda:900");
                assert_eq!(result["data"]["name"], "Account album");
                assert_eq!(result["data"]["track_count"], 3);
                assert_eq!(
                    result["data"]["extensions"]["source_snapshot_id"],
                    "revision-a"
                );
            }
        }
    }
}

#[tokio::test]
async fn favorite_http_reads_and_writes_use_selected_scope_and_return_updated_credentials() {
    for caller in [false, true] {
        for (method, path) in [
            (Method::GET, "/v1/account/favorites/playlist?platform=soda"),
            (Method::GET, "/v1/account/favorites/tracks?platform=soda"),
            (Method::GET, "/v1/users/soda:123456/favorites/playlist?"),
            (Method::GET, "/v1/users/soda:123456/favorites/tracks?"),
            (Method::PUT, "/v1/account/favorites/tracks/soda:11?"),
            (Method::DELETE, "/v1/account/favorites/tracks/soda:11?"),
        ] {
            let calls = Arc::new(AtomicUsize::new(0));
            let mut registry = ProviderRegistry::new();
            registry
                .register(SnapshotProvider {
                    caller: false,
                    mode: "stable",
                    calls: calls.clone(),
                })
                .unwrap();
            let app = build_router(AppState::new(registry, Platform::Soda));
            let uri = if caller {
                path.to_owned()
            } else {
                format!("{path}&account=personal")
            };
            let mut request = Request::builder().method(method.clone()).uri(uri);
            if caller {
                let credential = CallerCredential::issue(
                    &ProviderCredential::new(Platform::Soda, "test", "snapshot-original", None)
                        .unwrap(),
                )
                .unwrap();
                request = request.header(CALLER_CREDENTIAL_HEADER, credential.value);
            }
            let response = app
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert!(
                response.headers()[header::CACHE_CONTROL]
                    .to_str()
                    .unwrap()
                    .contains("no-store")
            );
            let updated = response
                .headers()
                .get(caller_scope::UPDATED_CREDENTIAL_HEADER);
            if caller {
                let credential = CallerCredential::parse(
                    updated
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .strip_prefix("soda=")
                        .unwrap(),
                )
                .unwrap();
                assert_eq!(credential.secret(), "snapshot-rotation-1");
            } else {
                assert!(updated.is_none());
            }
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            let result: Value = serde_json::from_slice(&bytes).unwrap();
            if method != Method::GET {
                assert_eq!(result["data"]["subscribed"], method == Method::PUT);
            } else if path.contains("/playlist") {
                assert_eq!(result["data"]["id"], "7200303561195061287");
            } else {
                assert_eq!(result["data"].as_array().unwrap().len(), 2);
            }
        }
    }
}

#[tokio::test]
async fn album_collection_http_routes_preserve_selected_source_paging_and_final_credentials() {
    for caller in [false, true] {
        for (method, path, batch) in [
            (
                Method::GET,
                "/v1/account/library/albums?platform=soda",
                false,
            ),
            (
                Method::GET,
                "/v1/users/soda:123456/favorites/albums?",
                false,
            ),
            (Method::PUT, "/v1/account/library/albums/soda:11?", false),
            (Method::DELETE, "/v1/account/library/albums/soda:11?", false),
            (Method::PUT, "/v1/account/library/albums", true),
            (Method::DELETE, "/v1/account/library/albums", true),
        ] {
            let calls = Arc::new(AtomicUsize::new(0));
            let mut registry = ProviderRegistry::new();
            registry
                .register(SnapshotProvider {
                    caller: false,
                    mode: "stable",
                    calls: calls.clone(),
                })
                .unwrap();
            let uri = if !caller && !batch {
                format!("{path}&account=personal")
            } else {
                path.to_owned()
            };
            let mut request = Request::builder().method(method.clone()).uri(uri);
            if caller {
                let credential = CallerCredential::issue(
                    &ProviderCredential::new(Platform::Soda, "test", "snapshot-original", None)
                        .unwrap(),
                )
                .unwrap();
                request = request.header(CALLER_CREDENTIAL_HEADER, credential.value);
            }
            let body = if batch {
                let mut body = json!({"refs":["soda:11","soda:22"]});
                if !caller {
                    body["account"] = json!("personal");
                }
                request = request.header(header::CONTENT_TYPE, "application/json");
                Body::from(body.to_string())
            } else {
                Body::empty()
            };
            let response = build_router(AppState::new(registry, Platform::Soda))
                .oneshot(request.body(body).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{method} {path}");
            let expected_calls = if batch { 2 } else { 1 };
            assert_eq!(calls.load(Ordering::SeqCst), expected_calls);
            assert!(
                response.headers()[header::CACHE_CONTROL]
                    .to_str()
                    .unwrap()
                    .contains("no-store")
            );
            let updated = response
                .headers()
                .get(caller_scope::UPDATED_CREDENTIAL_HEADER);
            if caller {
                let value = updated.unwrap();
                assert!(value.is_sensitive());
                let credential =
                    CallerCredential::parse(value.to_str().unwrap().strip_prefix("soda=").unwrap())
                        .unwrap();
                assert_eq!(
                    credential.secret(),
                    format!("snapshot-rotation-{expected_calls}")
                );
            } else {
                assert!(updated.is_none());
            }
            let value: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                    .unwrap();
            if method == Method::GET {
                assert_eq!(value["data"][0]["ref"], "soda:11");
                assert!(value["data"][0]["track_count"].is_null());
                assert_eq!(value["data"][0]["extensions"]["subscribed"], true);
                assert_eq!(value["meta"]["pagination"]["total"], 1);
                assert_eq!(
                    value["meta"]["pagination"]["extensions"]["complete_snapshot"],
                    true
                );
            } else if batch {
                assert_eq!(value["data"].as_array().unwrap().len(), 2);
                assert_eq!(value["data"][1]["resource_ref"], "soda:22");
                assert_eq!(value["data"][1]["subscribed"], method == Method::PUT);
            } else {
                assert_eq!(value["data"]["subscribed"], method == Method::PUT);
            }
        }
    }
}

#[tokio::test]
async fn album_batch_http_errors_keep_partial_results_and_suppress_invalidated_credentials() {
    for caller in [false, true] {
        for (mode, status) in [
            ("partial", StatusCode::BAD_GATEWAY),
            ("auth", StatusCode::UNAUTHORIZED),
            ("conflict", StatusCode::CONFLICT),
        ] {
            let calls = Arc::new(AtomicUsize::new(0));
            let mut registry = ProviderRegistry::new();
            registry
                .register(SnapshotProvider {
                    caller: false,
                    mode,
                    calls: calls.clone(),
                })
                .unwrap();
            let mut request = Request::builder()
                .method(Method::PUT)
                .uri("/v1/account/library/albums")
                .header(header::CONTENT_TYPE, "application/json");
            let mut body = json!({"refs":["soda:11","soda:22","soda:33"]});
            if caller {
                let credential = CallerCredential::issue(
                    &ProviderCredential::new(Platform::Soda, "test", "snapshot-original", None)
                        .unwrap(),
                )
                .unwrap();
                request = request.header(CALLER_CREDENTIAL_HEADER, credential.value);
            } else {
                body["account"] = json!("personal");
            }
            let response = build_router(AppState::new(registry, Platform::Soda))
                .oneshot(request.body(Body::from(body.to_string())).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), status);
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            assert!(
                response.headers()[header::CACHE_CONTROL]
                    .to_str()
                    .unwrap()
                    .contains("no-store")
            );
            let updated = response
                .headers()
                .get(caller_scope::UPDATED_CREDENTIAL_HEADER);
            if caller && mode == "partial" {
                let credential = CallerCredential::parse(
                    updated
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .strip_prefix("soda=")
                        .unwrap(),
                )
                .unwrap();
                assert_eq!(credential.secret(), "snapshot-rotation-2");
            } else {
                assert!(updated.is_none());
            }
            let value: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                    .unwrap();
            assert_eq!(
                value["error"]["details"]["completed_refs"],
                json!(["soda:11"])
            );
            assert_eq!(value["error"]["details"]["failed_ref"], "soda:22");
            assert_eq!(
                value["error"]["details"]["remaining_refs"],
                json!(["soda:33"])
            );
            assert_eq!(value["error"]["details"]["write_outcome"], "unconfirmed");
            assert_eq!(value["error"]["retryable"], false);
        }
    }
}
