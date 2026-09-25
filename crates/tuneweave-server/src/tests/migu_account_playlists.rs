use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tuneweave_core::PlaylistMutationAction;

struct PlaylistProvider {
    caller: bool,
    mode: &'static str,
    failure: Option<ErrorCode>,
    update: Mutex<Option<ProviderCredential>>,
    calls: Arc<AtomicUsize>,
}
impl PlaylistProvider {
    fn check(&self, account: Option<&str>) -> Result<()> {
        assert_eq!(account, Some(if self.caller { "default" } else { "A" }));
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.caller {
            *self.update.lock().unwrap() = Some(
                ProviderCredential::new(Platform::Migu, "test", "updated-playlist-session", None)
                    .unwrap(),
            );
        }
        if let Some(code) = self.failure {
            return Err(
                TuneWeaveError::new(code, "playlist fixture failure").with_platform(Platform::Migu)
            );
        }
        Ok(())
    }
    fn metadata(&self, account: Option<&str>) -> Result<Playlist> {
        self.check(account)?;
        Ok(Playlist {
            resource_ref: ResourceRef::new(Platform::Migu, "77").unwrap(),
            platform: Platform::Migu,
            id: "77".into(),
            name: "Actual playlist".into(),
            description: String::new(),
            cover_url: None,
            creator: None,
            track_count: Some(3),
            tags: vec![],
            subscribed: None,
            created_at: None,
            updated_at: None,
            extensions: Extensions::from([("source_snapshot_id".into(), json!("revision-a"))]),
        })
    }
    fn page(&self, r: &PageRequest) -> Result<Page<Track>> {
        self.check(r.account.as_deref())?;
        if r.offset == 2 && self.mode == "auth" {
            return Err(
                TuneWeaveError::new(ErrorCode::AuthenticationRequired, "expired")
                    .with_platform(Platform::Migu),
            );
        }
        let (ids, more) = if r.offset == 0 {
            (vec!["11", "22"], true)
        } else {
            (vec!["22"], false)
        };
        let revision = if r.offset == 2 && self.mode == "changed" {
            "revision-b"
        } else {
            "revision-a"
        };
        Ok(Page {
            items: ids
                .into_iter()
                .map(|id| Track::new(ResourceRef::new(Platform::Migu, id).unwrap(), "Song"))
                .collect(),
            pagination: PageMeta {
                limit: r.limit,
                offset: r.offset,
                total: Some(3),
                has_more: more,
                next_offset: more.then_some(2),
                extensions: Extensions::from([("source_snapshot_id".into(), json!(revision))]),
            },
        })
    }
}
#[async_trait]
impl MusicProvider for PlaylistProvider {
    async fn create_playlist(&self, r: &PlaylistCreateRequest) -> Result<PlaylistMutationResult> {
        assert_eq!(r.visibility, PlaylistVisibility::PlatformDefault);
        let playlist = self
            .metadata(r.account.as_deref())
            .map_err(mutation_failure)?;
        Ok(PlaylistMutationResult {
            playlist_ref: playlist.resource_ref.clone(),
            action: PlaylistMutationAction::Create,
            playlist: Some(playlist),
            extensions: Extensions::new(),
        })
    }
    async fn update_playlist(
        &self,
        id: &str,
        r: &PlaylistUpdateRequest,
    ) -> Result<PlaylistMutationResult> {
        assert_eq!(id, "77");
        assert_eq!(r.name.as_deref(), Some("New name"));
        let playlist = self
            .metadata(r.account.as_deref())
            .map_err(mutation_failure)?;
        Ok(PlaylistMutationResult {
            playlist_ref: playlist.resource_ref.clone(),
            action: PlaylistMutationAction::Update,
            playlist: Some(playlist),
            extensions: Extensions::new(),
        })
    }
    async fn delete_playlists(&self, r: &PlaylistDeleteRequest) -> Result<PlaylistDeleteResult> {
        assert_eq!(
            r.playlist_refs,
            vec![ResourceRef::new(Platform::Migu, "77").unwrap()]
        );
        self.check(r.account.as_deref()).map_err(mutation_failure)?;
        Ok(PlaylistDeleteResult {
            playlist_refs: r.playlist_refs.clone(),
            extensions: Extensions::new(),
        })
    }
    async fn mutate_playlist_items(
        &self,
        id: &str,
        action: PlaylistItemMutationAction,
        r: &PlaylistItemMutationRequest,
    ) -> Result<PlaylistItemMutationResult> {
        assert_eq!(id, "77");
        assert_eq!(r.kind, PlaylistItemKind::Track);
        assert_eq!(
            r.item_refs,
            vec![ResourceRef::new(Platform::Migu, "11").unwrap()]
        );
        self.check(r.account.as_deref()).map_err(mutation_failure)?;
        Ok(PlaylistItemMutationResult {
            playlist_ref: ResourceRef::new(Platform::Migu, id).unwrap(),
            item_refs: r.item_refs.clone(),
            kind: r.kind,
            action,
            snapshot_id: None,
            cloud_track_count: None,
            extensions: Extensions::new(),
        })
    }
    fn platform(&self) -> Platform {
        Platform::Migu
    }
    fn name(&self) -> &'static str {
        "Migu account playlist contract fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::PlaylistRead,
            Capability::Favorites,
            Capability::CallerManagedCredentials,
        ])
    }
    fn with_caller_credential(&self, c: &ProviderCredential) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(c.secret(), "original-playlist-session");
        Ok(Arc::new(Self {
            caller: true,
            mode: self.mode,
            failure: self.failure,
            update: Mutex::new(None),
            calls: self.calls.clone(),
        }))
    }
    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        Ok(self.update.lock().unwrap().take())
    }
    async fn set_track_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        assert_eq!(id, "11");
        self.check(account).map_err(|e| {
            e.retryable(false)
                .with_details(json!({"write_outcome":"unconfirmed"}))
        })?;
        Ok(SubscriptionResult {
            resource_ref: ResourceRef::new(Platform::Migu, id).unwrap(),
            subscribed,
            extensions: Extensions::new(),
        })
    }
    async fn set_playlist_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        assert_eq!(id, "77");
        self.check(account).map_err(|e| {
            e.retryable(false)
                .with_details(json!({"write_outcome":"unconfirmed"}))
        })?;
        Ok(SubscriptionResult {
            resource_ref: ResourceRef::new(Platform::Migu, id).unwrap(),
            subscribed,
            extensions: Extensions::new(),
        })
    }
    async fn playlist(&self, id: &str, account: Option<&str>) -> Result<Playlist> {
        assert_eq!(id, "77");
        self.metadata(account)
    }
    async fn playlist_tracks(&self, id: &str, r: &PageRequest) -> Result<Page<Track>> {
        assert_eq!(id, "77");
        self.page(r)
    }
    async fn favorite_playlist(&self, account: Option<&str>) -> Result<Playlist> {
        self.metadata(account)
    }
    async fn favorite_tracks(&self, r: &PageRequest) -> Result<Page<Track>> {
        self.page(r)
    }
    async fn user_favorite_playlist(&self, uid: &str, account: Option<&str>) -> Result<Playlist> {
        assert_eq!(uid, "111");
        self.metadata(account)
    }
    async fn user_favorite_tracks(&self, uid: &str, r: &PageRequest) -> Result<Page<Track>> {
        assert_eq!(uid, "111");
        self.page(r)
    }
    async fn playlist_source(
        &self,
        id: &str,
        kind: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        match kind {
            "playlist" => self.playlist(id, account).await,
            "favorite_tracks" => self.user_favorite_playlist(id, account).await,
            _ => panic!("unexpected source"),
        }
    }
    async fn playlist_source_items(
        &self,
        id: &str,
        kind: &str,
        r: &PageRequest,
    ) -> Result<Page<PlaylistPlayableItem>> {
        let p = match kind {
            "playlist" => self.playlist_tracks(id, r).await,
            "favorite_tracks" => self.user_favorite_tracks(id, r).await,
            _ => panic!("unexpected source"),
        }?;
        Ok(Page {
            items: p
                .items
                .into_iter()
                .map(PlaylistPlayableItem::Track)
                .collect(),
            pagination: p.pagination,
        })
    }
}
fn mutation_failure(e: TuneWeaveError) -> TuneWeaveError {
    e.retryable(false)
        .with_details(json!({"write_outcome":"unconfirmed"}))
}

#[tokio::test]
async fn migu_playlist_mutation_routes_deliver_verified_updates_on_success_and_confirmation_failure()
 {
    for caller in [false, true] {
        for (failure, status) in [
            (None, StatusCode::OK),
            (Some(ErrorCode::UpstreamError), StatusCode::BAD_GATEWAY),
            (
                Some(ErrorCode::AuthenticationRequired),
                StatusCode::UNAUTHORIZED,
            ),
            (Some(ErrorCode::Conflict), StatusCode::CONFLICT),
        ] {
            for operation in 0..8 {
                let (method, path, mut payload) = match operation {
                    0 => (
                        Method::POST,
                        "/v1/playlists",
                        json!({"platform":"migu","name":"New name","visibility":"platform_default"}),
                    ),
                    1 => (
                        Method::PATCH,
                        "/v1/playlists/migu:77",
                        json!({"name":"New name"}),
                    ),
                    2 => (Method::DELETE, "/v1/playlists/migu:77", Value::Null),
                    3 => (Method::DELETE, "/v1/playlists", json!({"refs":["migu:77"]})),
                    4 => (
                        Method::POST,
                        "/v1/playlists/migu:77/tracks",
                        json!({"refs":["migu:11"]}),
                    ),
                    5 => (
                        Method::DELETE,
                        "/v1/playlists/migu:77/tracks",
                        json!({"refs":["migu:11"]}),
                    ),
                    6 => (
                        Method::POST,
                        "/v1/playlists/migu:77/items",
                        json!({"refs":["migu:11"],"kind":"track"}),
                    ),
                    _ => (
                        Method::DELETE,
                        "/v1/playlists/migu:77/items",
                        json!({"refs":["migu:11"],"kind":"track"}),
                    ),
                };
                let mut uri = path.to_owned();
                if !caller {
                    if payload.is_null() {
                        uri.push_str("?account=A");
                    } else {
                        payload["account"] = json!("A");
                    }
                }
                let mut request = Request::builder()
                    .uri(uri)
                    .method(method)
                    .header(header::CONTENT_TYPE, "application/json");
                if caller {
                    request = request.header(CALLER_CREDENTIAL_HEADER, credential());
                }
                let response = app("stable", failure)
                    .oneshot(
                        request
                            .body(if payload.is_null() {
                                Body::empty()
                            } else {
                                Body::from(payload.to_string())
                            })
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    status,
                    "operation {operation} caller {caller}"
                );
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
                let expected = caller
                    && !matches!(
                        failure,
                        Some(ErrorCode::AuthenticationRequired | ErrorCode::Conflict)
                    );
                assert_eq!(update.is_some(), expected);
                assert_eq!(body["meta"]["caller_credential"].is_object(), expected);
                if let Some(update) = update {
                    assert_eq!(
                        body["meta"]["caller_credential"]["value"],
                        update.strip_prefix("migu=").unwrap()
                    );
                }
                if failure.is_some() {
                    assert_eq!(body["error"]["details"]["write_outcome"], "unconfirmed");
                    assert_eq!(body["error"]["retryable"], false);
                }
            }
        }
    }
}

#[test]
fn playlist_visibility_default_remains_public_and_platform_default_is_explicit() {
    assert_eq!(
        parse_playlist_visibility(None, None).unwrap(),
        PlaylistVisibility::Public
    );
    assert_eq!(
        parse_playlist_visibility(Some(&json!("private")), None).unwrap(),
        PlaylistVisibility::Private
    );
    assert_eq!(
        parse_playlist_visibility(Some(&json!("platform_default")), None).unwrap(),
        PlaylistVisibility::PlatformDefault
    );
    assert!(parse_playlist_visibility(None, Some(&json!("platform_default"))).is_err());
    assert!(parse_playlist_visibility(Some(&json!("platform_default")), Some(&json!(0))).is_err());
    assert_eq!(
        serde_json::to_value(PlaylistVisibility::PlatformDefault).unwrap(),
        json!("platform_default")
    );
}
fn app(mode: &'static str, failure: Option<ErrorCode>) -> Router {
    let mut registry = ProviderRegistry::new();
    registry
        .register(PlaylistProvider {
            caller: false,
            mode,
            failure,
            update: Mutex::new(None),
            calls: Arc::new(AtomicUsize::new(0)),
        })
        .unwrap();
    build_router(AppState::new(registry, Platform::Migu))
}
fn credential() -> String {
    CallerCredential::issue(
        &ProviderCredential::new(Platform::Migu, "test", "original-playlist-session", None)
            .unwrap(),
    )
    .unwrap()
    .value
}
#[tokio::test]
async fn migu_account_playlist_and_favorite_routes_return_updates_and_no_store_on_success_and_failure()
 {
    for caller in [false, true] {
        for (base, tracks) in [
            ("/v1/playlists/migu:77?", false),
            ("/v1/playlists/migu:77/tracks?", true),
            ("/v1/account/favorites/playlist?platform=migu&", false),
            ("/v1/account/favorites/tracks?platform=migu&", true),
            ("/v1/users/migu:111/favorites/playlist?", false),
            ("/v1/users/migu:111/favorites/tracks?", true),
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
                let uri = format!(
                    "{base}{}{}",
                    if tracks { "limit=2&offset=0&" } else { "" },
                    if caller { "" } else { "account=A" }
                );
                let mut request = Request::builder().uri(uri);
                if caller {
                    request = request.header(CALLER_CREDENTIAL_HEADER, credential());
                }
                let response = app("stable", failure)
                    .oneshot(request.body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status(), status, "{base}");
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
                let expected = caller
                    && !matches!(
                        failure,
                        Some(ErrorCode::AuthenticationRequired | ErrorCode::Conflict)
                    );
                assert_eq!(update.is_some(), expected, "{base}");
                assert_eq!(
                    body["meta"]["caller_credential"].is_object(),
                    expected,
                    "{base}"
                );
                if let Some(header) = update {
                    let value = header.strip_prefix("migu=").unwrap();
                    assert_eq!(body["meta"]["caller_credential"]["value"], value);
                }
                if failure.is_none() {
                    if tracks {
                        assert_eq!(body["data"][0]["ref"], "migu:11");
                        assert_eq!(body["meta"]["pagination"]["total"], 3);
                    } else {
                        assert_eq!(body["data"]["ref"], "migu:77");
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn migu_uni_account_sources_keep_order_duplicates_and_reject_changed_complete_reads() {
    for caller in [false, true] {
        for materialize in [false, true] {
            for favorites in [false, true] {
                for mode in ["stable", "changed", "auth"] {
                    let mut source = json!({"ref":if favorites{"migu:111"}else{"migu:77"},"type":if favorites{"favorite_tracks"}else{"playlist"}});
                    if !caller {
                        source["account"] = json!("A");
                    }
                    let body = json!({"sources":[source]});
                    let path = if materialize {
                        "/v1/uni/materialize/imports"
                    } else {
                        "/v1/uni/playlists/imports"
                    };
                    let mut request = Request::builder()
                        .method(Method::POST)
                        .uri(path)
                        .header(header::CONTENT_TYPE, "application/json");
                    if caller {
                        request = request.header(CALLER_CREDENTIAL_HEADER, credential());
                    }
                    let router = app(mode, None);
                    let response = router
                        .clone()
                        .oneshot(request.body(Body::from(body.to_string())).unwrap())
                        .await
                        .unwrap();
                    let status = response.status();
                    let update = response
                        .headers()
                        .get("X-TuneWeave-Updated-Credential")
                        .is_some();
                    let body: Value = serde_json::from_slice(
                        &to_bytes(response.into_body(), 1024 * 1024).await.unwrap(),
                    )
                    .unwrap();
                    let (_, directory) =
                        json_response_from(router.clone(), "/v1/uni/playlists").await;
                    if mode == "stable" {
                        assert!(status.is_success(), "{body}");
                        let items = if materialize {
                            assert_eq!(directory["data"], json!([]));
                            assert_eq!(body["data"]["item_count"], 3);
                            body["data"]["items"].clone()
                        } else {
                            assert_eq!(directory["data"].as_array().unwrap().len(), 1);
                            assert_eq!(body["data"]["playlist"]["item_count"], 3);
                            let reference = body["data"]["playlist"]["ref"].as_str().unwrap();
                            let (_, response) = json_response_from(
                                router,
                                &format!("/v1/uni/playlists/{reference}/items"),
                            )
                            .await;
                            response["data"].clone()
                        };
                        assert_eq!(items[0]["source_ref"], "migu:11");
                        assert_eq!(items[1]["source_ref"], "migu:22");
                        assert_eq!(items[2]["source_ref"], "migu:22");
                        assert_ne!(items[1]["id"], items[2]["id"]);
                        assert_eq!(items[2]["position"], 2);
                        let serialized = items.to_string();
                        assert!(!serialized.contains("original-playlist-session"));
                        assert!(!serialized.contains("updated-playlist-session"));
                        assert!(!serialized.contains("twc1_"));
                    } else {
                        assert!(!status.is_success(), "{body}");
                        assert!(body.get("data").is_none());
                        assert_eq!(directory["data"], json!([]));
                        assert_eq!(
                            body["error"]["code"],
                            if mode == "auth" {
                                "authentication_required"
                            } else {
                                "upstream_error"
                            }
                        );
                    }
                    assert_eq!(update, caller && mode != "auth", "{path} {mode}");
                }
            }
        }
    }
}

#[tokio::test]
async fn migu_favorite_write_routes_preserve_confirmed_results_failure_updates_and_no_store() {
    for caller in [false, true] {
        for (method, subscribed) in [(Method::PUT, true), (Method::DELETE, false)] {
            for (failure, status) in [
                (None, StatusCode::OK),
                (Some(ErrorCode::UpstreamError), StatusCode::BAD_GATEWAY),
                (
                    Some(ErrorCode::AuthenticationRequired),
                    StatusCode::UNAUTHORIZED,
                ),
                (Some(ErrorCode::Conflict), StatusCode::CONFLICT),
            ] {
                let uri = format!(
                    "/v1/account/favorites/tracks/migu:11{}",
                    if caller { "" } else { "?account=A" }
                );
                let mut request = Request::builder().uri(uri).method(method.clone());
                if caller {
                    request = request.header(CALLER_CREDENTIAL_HEADER, credential());
                }
                let response = app("stable", failure)
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
                let expected = caller
                    && !matches!(
                        failure,
                        Some(ErrorCode::AuthenticationRequired | ErrorCode::Conflict)
                    );
                assert_eq!(update.is_some(), expected);
                assert_eq!(body["meta"]["caller_credential"].is_object(), expected);
                if let Some(update) = update {
                    assert_eq!(
                        body["meta"]["caller_credential"]["value"],
                        update.strip_prefix("migu=").unwrap()
                    );
                }
                if failure.is_none() {
                    assert_eq!(body["data"]["resource_ref"], "migu:11");
                    assert_eq!(body["data"]["subscribed"], subscribed);
                } else {
                    assert_eq!(body["error"]["details"]["write_outcome"], "unconfirmed");
                    assert_eq!(body["error"]["retryable"], false);
                }
            }
        }
    }
}

#[tokio::test]
async fn migu_playlist_collection_routes_preserve_confirmation_failure_updates_and_no_store() {
    for caller in [false, true] {
        for (method, subscribed) in [(Method::PUT, true), (Method::DELETE, false)] {
            for (failure, status) in [
                (None, StatusCode::OK),
                (Some(ErrorCode::UpstreamError), StatusCode::BAD_GATEWAY),
                (
                    Some(ErrorCode::AuthenticationRequired),
                    StatusCode::UNAUTHORIZED,
                ),
                (Some(ErrorCode::Conflict), StatusCode::CONFLICT),
            ] {
                let uri = format!(
                    "/v1/account/favorites/playlists/migu:77{}",
                    if caller { "" } else { "?account=A" }
                );
                let mut request = Request::builder().uri(uri).method(method.clone());
                if caller {
                    request = request.header(CALLER_CREDENTIAL_HEADER, credential());
                }
                let response = app("stable", failure)
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
                let expected = caller
                    && !matches!(
                        failure,
                        Some(ErrorCode::AuthenticationRequired | ErrorCode::Conflict)
                    );
                assert_eq!(update.is_some(), expected);
                assert_eq!(body["meta"]["caller_credential"].is_object(), expected);
                if let Some(update) = update {
                    assert_eq!(
                        body["meta"]["caller_credential"]["value"],
                        update.strip_prefix("migu=").unwrap()
                    );
                }
                if failure.is_none() {
                    assert_eq!(body["data"]["resource_ref"], "migu:77");
                    assert_eq!(body["data"]["subscribed"], subscribed);
                } else {
                    assert_eq!(body["error"]["details"]["write_outcome"], "unconfirmed");
                    assert_eq!(body["error"]["retryable"], false);
                }
            }
        }
    }
}
