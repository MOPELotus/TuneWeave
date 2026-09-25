use super::*;
use tuneweave_core::PlaylistMutationAction;

#[tokio::test]
async fn kugou_library_routes_preserve_accounts_pagination_and_rotations_without_caching() {
    super::migu_library::check_library_routes(Platform::Kugou).await;
}

#[tokio::test]
async fn kugou_native_library_routes_require_credentials_and_reject_cross_account_references() {
    let mut registry = ProviderRegistry::new();
    registry
        .register(tuneweave_provider_kugou::KugouProvider::new(Default::default()).unwrap())
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Kugou));
    for (uri, status) in [
        (
            "/v1/account/playlists?platform=kugou&account=A",
            StatusCode::UNAUTHORIZED,
        ),
        (
            "/v1/users/kugou:111/playlists/created?account=A",
            StatusCode::UNAUTHORIZED,
        ),
        (
            "/v1/users/kugou:111/favorites/playlists?account=A",
            StatusCode::UNAUTHORIZED,
        ),
        (
            "/v1/playlists/kugou:cloudlist:111:0:1?account=A",
            StatusCode::UNAUTHORIZED,
        ),
        (
            "/v1/playlists/kugou:cloudlist:0111:0:1?account=A",
            StatusCode::BAD_REQUEST,
        ),
        (
            "/v1/playlists/kugou:cloudlist:111:0:1/tracks?account=A",
            StatusCode::UNAUTHORIZED,
        ),
        (
            "/v1/playlists/kugou:cloudlist:111:0:1/items?account=A",
            StatusCode::UNAUTHORIZED,
        ),
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), status, "{uri}");
        assert!(
            response.headers()[header::CACHE_CONTROL]
                .to_str()
                .unwrap()
                .contains("no-store")
        );
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
        assert!(body["meta"].get("caller_credential").is_none());
    }
}

struct TrackSnapshotProvider {
    caller: bool,
    failure: Option<ErrorCode>,
    changed: bool,
    update: Mutex<Option<ProviderCredential>>,
}
impl TrackSnapshotProvider {
    fn accept(&self, account: Option<&str>) {
        assert_eq!(account, Some(if self.caller { "default" } else { "A" }));
        if self.caller {
            *self.update.lock().unwrap() = Some(
                ProviderCredential::new(Platform::Kugou, "test", "rotated-tracks", None).unwrap(),
            );
        }
    }
    fn fail(&self, write: bool) -> Result<()> {
        if let Some(code) = self.failure {
            let mut error = TuneWeaveError::new(code, "native playlist operation failed")
                .with_platform(Platform::Kugou);
            if write {
                error = error.retryable(false).with_details(json!({
                    "write_outcome":"unconfirmed", "write_requests_dispatched":1
                }));
            }
            return Err(error);
        }
        Ok(())
    }
}
#[async_trait]
impl MusicProvider for TrackSnapshotProvider {
    fn platform(&self) -> Platform {
        Platform::Kugou
    }
    fn name(&self) -> &'static str {
        "Native playlist HTTP snapshot fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::AccountPurchasedTracks,
            Capability::AccountPurchasedAlbums,
            Capability::PlaylistRead,
            Capability::PlaylistOccurrenceRead,
            Capability::PlaylistOccurrenceWrite,
            Capability::PlaylistWrite,
            Capability::Favorites,
            Capability::TrackSubscriptionWrite,
            Capability::PlaylistSubscriptionWrite,
            Capability::CallerManagedCredentials,
        ])
    }
    fn with_caller_credential(&self, c: &ProviderCredential) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(c.platform, Platform::Kugou);
        assert_eq!(c.secret(), "original-tracks");
        Ok(Arc::new(Self {
            caller: true,
            failure: self.failure,
            changed: self.changed,
            update: Mutex::new(None),
        }))
    }
    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        Ok(self.update.lock().unwrap().take())
    }
    async fn account_purchased_tracks(
        &self,
        request: &PageRequest,
    ) -> Result<Page<PurchasedTrack>> {
        self.accept(request.account.as_deref());
        self.fail(false)?;
        assert_eq!((request.limit, request.offset), (10, 5));
        Ok(Page {
            items: vec![PurchasedTrack {
                track: None,
                name: Some("Unresolved purchased song".into()),
                artists: vec![],
                cover_url: None,
                extensions: Extensions::from([
                    ("goods_id".into(), json!("88")),
                    ("good_scid".into(), json!("111")),
                ]),
            }],
            pagination: purchase_page(request),
        })
    }
    async fn account_purchased_albums(
        &self,
        request: &PageRequest,
    ) -> Result<Page<PurchasedAlbum>> {
        self.accept(request.account.as_deref());
        self.fail(false)?;
        assert_eq!((request.limit, request.offset), (10, 5));
        Ok(Page {
            items: vec![PurchasedAlbum {
                album: None,
                digital_album: None,
                name: Some("Unresolved purchased album".into()),
                artists: vec![],
                cover_url: None,
                extensions: Extensions::from([("goods_id".into(), json!("99"))]),
            }],
            pagination: purchase_page(request),
        })
    }
    async fn playlist_track_occurrences(
        &self,
        id: &str,
        request: &PageRequest,
    ) -> Result<Page<PlaylistTrackOccurrence>> {
        self.accept(request.account.as_deref());
        self.fail(false)?;
        assert_eq!(id, "cloudlist:111:1:7");
        Ok(Page {
            items: vec![PlaylistTrackOccurrence {
                id: "entry:111:1:7:81".into(),
                position: 0,
                track: None,
                extensions: Extensions::new(),
            }],
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(1),
                has_more: false,
                next_offset: None,
                extensions: Extensions::from([("source_snapshot_id".into(), json!("raw-before"))]),
            },
        })
    }
    async fn reorder_playlist_occurrences(
        &self,
        id: &str,
        request: &PlaylistOccurrenceOrderRequest,
    ) -> Result<PlaylistOccurrenceOrderResult> {
        self.accept(request.account.as_deref());
        self.fail(true)?;
        assert_eq!(id, "cloudlist:111:1:7");
        assert_eq!(request.snapshot_id, "raw-before");
        Ok(PlaylistOccurrenceOrderResult {
            playlist_ref: ResourceRef::new(Platform::Kugou, id).unwrap(),
            occurrence_ids: request.occurrence_ids.clone(),
            snapshot_id: "raw-after".into(),
            extensions: Extensions::from([("confirmed".into(), json!(true))]),
        })
    }
    async fn reorder_playlist_tracks(
        &self,
        id: &str,
        request: &PlaylistTrackOrderRequest,
    ) -> Result<PlaylistTrackOrderResult> {
        self.accept(request.account.as_deref());
        self.fail(true)?;
        Ok(PlaylistTrackOrderResult {
            playlist_ref: ResourceRef::new(Platform::Kugou, id).unwrap(),
            track_refs: request.track_refs.clone(),
            snapshot_id: Some("raw-after".into()),
            extensions: Extensions::new(),
        })
    }
    async fn reorder_account_playlists(
        &self,
        request: &PlaylistOrderRequest,
    ) -> Result<PlaylistOrderResult> {
        self.accept(request.account.as_deref());
        self.fail(true)?;
        Ok(PlaylistOrderResult {
            playlist_refs: request.playlist_refs.clone(),
            extensions: Extensions::new(),
        })
    }
    async fn playlist(&self, id: &str, account: Option<&str>) -> Result<Playlist> {
        self.accept(account);
        assert_eq!(id, "cloudlist:111:1:7");
        Ok(Playlist {
            resource_ref: ResourceRef::new(Platform::Kugou, id).unwrap(),
            platform: Platform::Kugou,
            id: id.into(),
            name: "Account playlist".into(),
            description: String::new(),
            cover_url: None,
            creator: None,
            track_count: Some(3),
            tags: vec![],
            subscribed: Some(true),
            created_at: None,
            updated_at: None,
            extensions: Extensions::from([("source_snapshot_id".into(), json!("v3-revision-a"))]),
        })
    }
    async fn playlist_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        self.accept(request.account.as_deref());
        assert_eq!(id, "cloudlist:111:1:7");
        if let Some(code) = self.failure {
            return Err(TuneWeaveError::new(code, "native track page failed")
                .with_platform(Platform::Kugou));
        }
        let (entries, next) = match request.offset {
            0 => (vec![(11, 81), (22, 95)], Some(2)),
            2 => (vec![(22, 99)], None),
            _ => panic!("unexpected window"),
        };
        Ok(Page {
            items: entries
                .into_iter()
                .enumerate()
                .map(|(i, (id, fileid))| {
                    let mut t = Track::new(
                        ResourceRef::new(Platform::Kugou, id.to_string()).unwrap(),
                        "Song",
                    );
                    t.extensions.insert("file_id".into(), json!(fileid));
                    t.extensions.insert(
                        "playlist_position".into(),
                        json!(request.offset as usize + i),
                    );
                    t
                })
                .collect(),
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(3),
                next_offset: next,
                has_more: next.is_some(),
                extensions: Extensions::from([(
                    "source_snapshot_id".into(),
                    json!(if self.changed && request.offset == 2 {
                        "v3-revision-b"
                    } else {
                        "v3-revision-a"
                    }),
                )]),
            },
        })
    }
    async fn favorite_playlist(&self, account: Option<&str>) -> Result<Playlist> {
        let mut playlist = self.playlist("cloudlist:111:1:7", account).await?;
        self.fail(false)?;
        playlist.id = "cloudlist:111:0:37".into();
        playlist.resource_ref = ResourceRef::new(Platform::Kugou, &playlist.id).unwrap();
        playlist.name = "Renamed favorite playlist".into();
        playlist
            .extensions
            .insert("source_type".into(), json!("favorite_tracks"));
        Ok(playlist)
    }
    async fn favorite_tracks(&self, request: &PageRequest) -> Result<Page<Track>> {
        self.playlist_tracks("cloudlist:111:1:7", request).await
    }
    async fn user_favorite_playlist(&self, uid: &str, account: Option<&str>) -> Result<Playlist> {
        assert_eq!(uid, "111");
        self.favorite_playlist(account).await
    }
    async fn user_favorite_tracks(&self, uid: &str, request: &PageRequest) -> Result<Page<Track>> {
        assert_eq!(uid, "111");
        self.favorite_tracks(request).await
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
        request: &PageRequest,
    ) -> Result<Page<PlaylistPlayableItem>> {
        let tracks = match kind {
            "playlist" => self.playlist_tracks(id, request).await?,
            "favorite_tracks" => self.user_favorite_tracks(id, request).await?,
            _ => panic!("unexpected source"),
        };
        Ok(Page {
            items: tracks
                .items
                .into_iter()
                .map(PlaylistPlayableItem::Track)
                .collect(),
            pagination: tracks.pagination,
        })
    }
    async fn set_track_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        assert_eq!(id, "11");
        self.accept(account);
        self.fail(true)?;
        Ok(SubscriptionResult {
            resource_ref: ResourceRef::new(Platform::Kugou, id).unwrap(),
            subscribed,
            extensions: Extensions::from([("changed".into(), json!(true))]),
        })
    }
    async fn mutate_playlist_items(
        &self,
        id: &str,
        action: PlaylistItemMutationAction,
        request: &PlaylistItemMutationRequest,
    ) -> Result<PlaylistItemMutationResult> {
        assert_eq!(id, "cloudlist:111:0:37");
        assert_eq!(request.kind, PlaylistItemKind::Track);
        assert_eq!(
            request.item_refs,
            vec![ResourceRef::new(Platform::Kugou, "11").unwrap()]
        );
        self.accept(request.account.as_deref());
        self.fail(true)?;
        Ok(PlaylistItemMutationResult {
            playlist_ref: ResourceRef::new(Platform::Kugou, id).unwrap(),
            item_refs: request.item_refs.clone(),
            kind: request.kind,
            action,
            snapshot_id: Some("confirmed-v3".into()),
            cloud_track_count: Some(3),
            extensions: Extensions::from([("atomic".into(), json!(false))]),
        })
    }
    async fn create_playlist(&self, r: &PlaylistCreateRequest) -> Result<PlaylistMutationResult> {
        self.accept(r.account.as_deref());
        assert_eq!(r.name, "HTTP created");
        assert_eq!(r.visibility, PlaylistVisibility::Private);
        self.fail(true)?;
        let mut p = self.favorite_playlist(r.account.as_deref()).await?;
        p.name = r.name.clone();
        Ok(PlaylistMutationResult {
            playlist_ref: p.resource_ref.clone(),
            playlist: Some(p),
            action: PlaylistMutationAction::Create,
            extensions: Extensions::new(),
        })
    }
    async fn update_playlist(
        &self,
        id: &str,
        r: &PlaylistUpdateRequest,
    ) -> Result<PlaylistMutationResult> {
        assert_eq!(id, "cloudlist:111:0:37");
        self.accept(r.account.as_deref());
        assert_eq!(r.name.as_deref(), Some("HTTP renamed"));
        assert_eq!(r.description.as_deref(), Some("Description"));
        assert_eq!(r.tags, Some(vec!["Tag".into()]));
        self.fail(true)?;
        let mut p = self.favorite_playlist(r.account.as_deref()).await?;
        p.name = r.name.clone().unwrap();
        Ok(PlaylistMutationResult {
            playlist_ref: p.resource_ref.clone(),
            playlist: Some(p),
            action: PlaylistMutationAction::Update,
            extensions: Extensions::new(),
        })
    }
    async fn delete_playlists(&self, r: &PlaylistDeleteRequest) -> Result<PlaylistDeleteResult> {
        self.accept(r.account.as_deref());
        assert_eq!(r.playlist_refs[0].id(), "cloudlist:111:0:37");
        self.fail(true).map_err(|mut e| {
            if r.playlist_refs.len() == 3 {
                e.details["confirmed_refs"] = json!([r.playlist_refs[0]]);
                e.details["unconfirmed_refs"] = json!([r.playlist_refs[1]]);
                e.details["not_attempted_refs"] = json!([r.playlist_refs[2]]);
                e.details["write_requests_dispatched"] = json!(2);
            }
            e
        })?;
        Ok(PlaylistDeleteResult {
            playlist_refs: r.playlist_refs.clone(),
            extensions: Extensions::from([("atomic".into(), json!(false))]),
        })
    }
    async fn set_playlist_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        assert_eq!(id, "collection_1_222_88_0");
        self.accept(account);
        self.fail(true)?;
        Ok(SubscriptionResult {
            resource_ref: ResourceRef::new(Platform::Kugou, id).unwrap(),
            subscribed,
            extensions: Extensions::from([(
                "account_playlist_ref".into(),
                json!("kugou:cloudlist:111:1:37"),
            )]),
        })
    }
}
fn tracks_app(failure: Option<ErrorCode>, changed: bool) -> Router {
    let mut registry = ProviderRegistry::new();
    registry
        .register(TrackSnapshotProvider {
            caller: false,
            failure,
            changed,
            update: Mutex::new(None),
        })
        .unwrap();
    build_router(AppState::new(registry, Platform::Kugou))
}
fn tracks_caller() -> String {
    CallerCredential::issue(
        &ProviderCredential::new(Platform::Kugou, "test", "original-tracks", None).unwrap(),
    )
    .unwrap()
    .value
}
fn assert_track_rotation(response: &axum::response::Response, expected: bool) {
    assert!(
        response.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("no-store")
    );
    let update = response
        .headers()
        .get(caller_scope::UPDATED_CREDENTIAL_HEADER);
    assert_eq!(update.is_some(), expected);
    if let Some(value) = update {
        assert!(value.is_sensitive());
        let c = CallerCredential::parse(value.to_str().unwrap().strip_prefix("kugou=").unwrap())
            .unwrap();
        assert_eq!(c.secret(), "rotated-tracks");
    }
}

#[tokio::test]
async fn kugou_tracks_and_playable_routes_preserve_occurrences_rotation_and_failure_ownership() {
    for caller in [false, true] {
        for suffix in ["tracks", "items"] {
            for failure in [
                None,
                Some(ErrorCode::UpstreamError),
                Some(ErrorCode::AuthenticationRequired),
                Some(ErrorCode::Conflict),
            ] {
                let mut req = Request::builder().uri(format!(
                    "/v1/playlists/kugou:cloudlist:111:1:7/{suffix}?offset=0&limit=100{}",
                    if caller { "" } else { "&account=A" }
                ));
                if caller {
                    req = req.header(CALLER_CREDENTIAL_HEADER, tracks_caller());
                }
                let response = tracks_app(failure, false)
                    .oneshot(req.body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_track_rotation(
                    &response,
                    caller
                        && !matches!(
                            failure,
                            Some(ErrorCode::AuthenticationRequired | ErrorCode::Conflict)
                        ),
                );
                assert_eq!(
                    response.status(),
                    match failure {
                        None => StatusCode::OK,
                        Some(ErrorCode::AuthenticationRequired) => StatusCode::UNAUTHORIZED,
                        Some(ErrorCode::Conflict) => StatusCode::CONFLICT,
                        _ => StatusCode::BAD_GATEWAY,
                    }
                );
                let body: Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                        .unwrap();
                if failure.is_none() {
                    assert_eq!(body["data"].as_array().unwrap().len(), 2);
                    assert_eq!(
                        body["meta"]["pagination"]["extensions"]["source_snapshot_id"],
                        "v3-revision-a"
                    );
                    if suffix == "tracks" {
                        assert_eq!(body["data"][0]["extensions"]["file_id"], 81);
                    } else {
                        assert_eq!(body["data"][0]["position"], 0);
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn kugou_native_playlist_uni_imports_keep_duplicates_and_reject_changed_source_revisions() {
    for caller in [false, true] {
        for materialize in [false, true] {
            for changed in [false, true] {
                let app = tracks_app(None, changed);
                let mut source = json!({"ref":"kugou:cloudlist:111:1:7","type":"playlist"});
                if !caller {
                    source["account"] = json!("A");
                }
                let mut req = Request::builder()
                    .method(Method::POST)
                    .uri(if materialize {
                        "/v1/uni/materialize/imports"
                    } else {
                        "/v1/uni/playlists/imports"
                    })
                    .header(header::CONTENT_TYPE, "application/json");
                if caller {
                    req = req.header(CALLER_CREDENTIAL_HEADER, tracks_caller());
                }
                let response = app
                    .clone()
                    .oneshot(
                        req.body(Body::from(json!({"sources":[source]}).to_string()))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_track_rotation(&response, caller);
                assert_eq!(
                    response.status(),
                    if changed {
                        StatusCode::BAD_GATEWAY
                    } else {
                        StatusCode::OK
                    }
                );
                let body: Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                        .unwrap();
                let (_, directory) = json_response_from(app.clone(), "/v1/uni/playlists").await;
                if changed {
                    assert_eq!(directory["data"], json!([]));
                    continue;
                }
                let items = if materialize {
                    assert_eq!(directory["data"], json!([]));
                    body["data"]["items"].clone()
                } else {
                    let reference = body["data"]["playlist"]["ref"].as_str().unwrap();
                    json_response_from(app, &format!("/v1/uni/playlists/{reference}/items"))
                        .await
                        .1["data"]
                        .clone()
                };
                assert_eq!(items.as_array().unwrap().len(), 3);
                assert_eq!(items[1]["source_ref"], "kugou:22");
                assert_eq!(items[2]["source_ref"], "kugou:22");
                assert_ne!(items[1]["id"], items[2]["id"]);
                assert_eq!(items[2]["position"], 2);
                assert!(!items.to_string().contains("twc1_"));
            }
        }
    }
}

#[tokio::test]
async fn kugou_favorite_read_routes_preserve_identity_updates_and_errors() {
    for caller in [false, true] {
        for (base, tracks) in [
            ("/v1/account/favorites/playlist?platform=kugou&", false),
            ("/v1/account/favorites/tracks?platform=kugou&", true),
            ("/v1/users/kugou:111/favorites/playlist?", false),
            ("/v1/users/kugou:111/favorites/tracks?", true),
        ] {
            for (failure, status) in [
                (None, StatusCode::OK),
                (Some(ErrorCode::UpstreamError), StatusCode::BAD_GATEWAY),
                (
                    Some(ErrorCode::AuthenticationRequired),
                    StatusCode::UNAUTHORIZED,
                ),
                (Some(ErrorCode::Conflict), StatusCode::CONFLICT),
            ] {
                let mut request = Request::builder().uri(format!(
                    "{base}{}{}",
                    if tracks { "limit=2&offset=0&" } else { "" },
                    if caller { "" } else { "account=A" }
                ));
                if caller {
                    request = request.header(CALLER_CREDENTIAL_HEADER, tracks_caller());
                }
                let response = tracks_app(failure, false)
                    .oneshot(request.body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status(), status, "{base}");
                let rotated = caller
                    && !matches!(
                        failure,
                        Some(ErrorCode::AuthenticationRequired | ErrorCode::Conflict)
                    );
                assert_track_rotation(&response, rotated);
                let body: Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                        .unwrap();
                assert_eq!(body["meta"]["caller_credential"].is_object(), rotated);
                if failure.is_none() {
                    if tracks {
                        assert_eq!(body["data"].as_array().unwrap().len(), 2);
                    } else {
                        assert_eq!(body["data"]["ref"], "kugou:cloudlist:111:0:37");
                        assert_eq!(body["data"]["name"], "Renamed favorite playlist");
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn kugou_favorite_uni_imports_preserve_duplicate_songs_and_abort_changed_snapshots() {
    for caller in [false, true] {
        for materialize in [false, true] {
            for changed in [false, true] {
                let app = tracks_app(None, changed);
                let mut source = json!({"ref":"kugou:111","type":"favorite_tracks"});
                if !caller {
                    source["account"] = json!("A");
                }
                let mut request = Request::builder()
                    .method(Method::POST)
                    .uri(if materialize {
                        "/v1/uni/materialize/imports"
                    } else {
                        "/v1/uni/playlists/imports"
                    })
                    .header(header::CONTENT_TYPE, "application/json");
                if caller {
                    request = request.header(CALLER_CREDENTIAL_HEADER, tracks_caller());
                }
                let response = app
                    .clone()
                    .oneshot(
                        request
                            .body(Body::from(json!({"sources":[source]}).to_string()))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    if changed {
                        StatusCode::BAD_GATEWAY
                    } else {
                        StatusCode::OK
                    }
                );
                assert_track_rotation(&response, caller);
                let body: Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                        .unwrap();
                let (_, directory) = json_response_from(app.clone(), "/v1/uni/playlists").await;
                if changed {
                    assert_eq!(directory["data"], json!([]));
                    continue;
                }
                let items = if materialize {
                    assert_eq!(directory["data"], json!([]));
                    body["data"]["items"].clone()
                } else {
                    let reference = body["data"]["playlist"]["ref"].as_str().unwrap();
                    json_response_from(app, &format!("/v1/uni/playlists/{reference}/items"))
                        .await
                        .1["data"]
                        .clone()
                };
                assert_eq!(items.as_array().unwrap().len(), 3);
                assert_eq!(items[1]["source_ref"], "kugou:22");
                assert_eq!(items[2]["source_ref"], "kugou:22");
                assert_ne!(items[1]["id"], items[2]["id"]);
                assert_eq!(items[2]["position"], 2);
                assert!(!items.to_string().contains("twc1_"));
            }
        }
    }
}

#[tokio::test]
async fn kugou_favorite_and_playlist_write_routes_preserve_confirmation_and_uncertainty() {
    for caller in [false, true] {
        for add in [false, true] {
            for kind in ["favorite", "tracks", "items"] {
                for (failure, status) in [
                    (None, StatusCode::OK),
                    (Some(ErrorCode::UpstreamError), StatusCode::BAD_GATEWAY),
                    (
                        Some(ErrorCode::AuthenticationRequired),
                        StatusCode::UNAUTHORIZED,
                    ),
                    (Some(ErrorCode::Conflict), StatusCode::CONFLICT),
                ] {
                    let mut uri = if kind == "favorite" {
                        "/v1/account/favorites/tracks/kugou:11".into()
                    } else {
                        format!("/v1/playlists/kugou:cloudlist:111:0:37/{kind}")
                    };
                    let mut payload = if kind == "favorite" {
                        Value::Null
                    } else {
                        json!({"refs":["kugou:11"],"kind":"track"})
                    };
                    if !caller {
                        if kind == "favorite" {
                            uri.push_str("?account=A");
                        } else {
                            payload["account"] = json!("A");
                        }
                    }
                    let mut request = Request::builder()
                        .uri(uri)
                        .method(if !add {
                            Method::DELETE
                        } else if kind == "favorite" {
                            Method::PUT
                        } else {
                            Method::POST
                        })
                        .header(header::CONTENT_TYPE, "application/json");
                    if caller {
                        request = request.header(CALLER_CREDENTIAL_HEADER, tracks_caller());
                    }
                    let response = tracks_app(failure, false)
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
                    assert_eq!(response.status(), status, "{kind}");
                    let rotated = caller
                        && !matches!(
                            failure,
                            Some(ErrorCode::AuthenticationRequired | ErrorCode::Conflict)
                        );
                    assert_track_rotation(&response, rotated);
                    let body: Value = serde_json::from_slice(
                        &to_bytes(response.into_body(), 65536).await.unwrap(),
                    )
                    .unwrap();
                    assert_eq!(body["meta"]["caller_credential"].is_object(), rotated);
                    if failure.is_some() {
                        assert_eq!(body["error"]["details"]["write_outcome"], "unconfirmed");
                        assert_eq!(body["error"]["details"]["write_requests_dispatched"], 1);
                        assert_eq!(body["error"]["retryable"], false);
                    } else if kind == "favorite" {
                        assert_eq!(body["data"]["resource_ref"], "kugou:11");
                        assert_eq!(body["data"]["subscribed"], add);
                    } else {
                        assert_eq!(body["data"]["playlist_ref"], "kugou:cloudlist:111:0:37");
                        assert_eq!(body["data"]["item_refs"], json!(["kugou:11"]));
                        assert_eq!(body["data"]["action"], if add { "add" } else { "remove" });
                        assert_eq!(body["data"]["snapshot_id"], "confirmed-v3");
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn kugou_management_routes_preserve_inputs_confirmation_and_partial_failure_ownership() {
    for caller in [false, true] {
        for kind in ["create", "update", "delete", "batch_delete"] {
            for (failure, status) in [
                (None, StatusCode::OK),
                (Some(ErrorCode::UpstreamError), StatusCode::BAD_GATEWAY),
                (
                    Some(ErrorCode::AuthenticationRequired),
                    StatusCode::UNAUTHORIZED,
                ),
                (Some(ErrorCode::Conflict), StatusCode::CONFLICT),
            ] {
                let (method, path, mut body) = match kind {
                    "create" => (
                        Method::POST,
                        "/v1/playlists",
                        json!({"platform":"kugou","name":"HTTP created","visibility":"private"}),
                    ),
                    "update" => (
                        Method::PATCH,
                        "/v1/playlists/kugou:cloudlist:111:0:37",
                        json!({"name":"HTTP renamed","description":"Description","tags":["Tag"]}),
                    ),
                    "delete" => (
                        Method::DELETE,
                        "/v1/playlists/kugou:cloudlist:111:0:37",
                        Value::Null,
                    ),
                    _ => (
                        Method::DELETE,
                        "/v1/playlists",
                        json!({"refs":["kugou:cloudlist:111:0:37","kugou:cloudlist:111:0:38","kugou:cloudlist:111:0:39"]}),
                    ),
                };
                let mut uri = path.to_owned();
                if !caller {
                    if body.is_null() {
                        uri.push_str("?account=A");
                    } else {
                        body["account"] = json!("A");
                    }
                }
                let mut request = Request::builder()
                    .uri(uri)
                    .method(method)
                    .header(header::CONTENT_TYPE, "application/json");
                if caller {
                    request = request.header(CALLER_CREDENTIAL_HEADER, tracks_caller());
                }
                let response = tracks_app(failure, false)
                    .oneshot(
                        request
                            .body(if body.is_null() {
                                Body::empty()
                            } else {
                                Body::from(body.to_string())
                            })
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), status, "{kind}");
                let rotated = caller
                    && !matches!(
                        failure,
                        Some(ErrorCode::AuthenticationRequired | ErrorCode::Conflict)
                    );
                assert_track_rotation(&response, rotated);
                let body: Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                        .unwrap();
                assert_eq!(body["meta"]["caller_credential"].is_object(), rotated);
                if failure.is_some() {
                    assert_eq!(body["error"]["details"]["write_outcome"], "unconfirmed");
                    assert_eq!(body["error"]["retryable"], false);
                    if kind == "batch_delete" {
                        assert_eq!(
                            body["error"]["details"]["confirmed_refs"],
                            json!(["kugou:cloudlist:111:0:37"])
                        );
                        assert_eq!(
                            body["error"]["details"]["unconfirmed_refs"],
                            json!(["kugou:cloudlist:111:0:38"])
                        );
                        assert_eq!(
                            body["error"]["details"]["not_attempted_refs"],
                            json!(["kugou:cloudlist:111:0:39"])
                        );
                    }
                } else if kind.contains("delete") {
                    assert_eq!(body["data"]["playlist_refs"][0], "kugou:cloudlist:111:0:37");
                } else {
                    assert_eq!(body["data"]["playlist_ref"], "kugou:cloudlist:111:0:37");
                    assert_eq!(body["data"]["action"], kind);
                }
            }
        }
    }
}

#[tokio::test]
async fn kugou_collection_write_routes_keep_source_reference_and_account_local_reference_distinct()
{
    for caller in [false, true] {
        for add in [false, true] {
            for (failure, status) in [
                (None, StatusCode::OK),
                (Some(ErrorCode::UpstreamError), StatusCode::BAD_GATEWAY),
                (
                    Some(ErrorCode::AuthenticationRequired),
                    StatusCode::UNAUTHORIZED,
                ),
                (Some(ErrorCode::Conflict), StatusCode::CONFLICT),
            ] {
                let mut request = Request::builder()
                    .method(if add { Method::PUT } else { Method::DELETE })
                    .uri(format!(
                        "/v1/account/favorites/playlists/kugou:collection_1_222_88_0{}",
                        if caller { "" } else { "?account=A" }
                    ));
                if caller {
                    request = request.header(CALLER_CREDENTIAL_HEADER, tracks_caller());
                }
                let response = tracks_app(failure, false)
                    .oneshot(request.body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status(), status);
                let rotated = caller
                    && !matches!(
                        failure,
                        Some(ErrorCode::AuthenticationRequired | ErrorCode::Conflict)
                    );
                assert_track_rotation(&response, rotated);
                let body: Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                        .unwrap();
                assert_eq!(body["meta"]["caller_credential"].is_object(), rotated);
                if failure.is_some() {
                    assert_eq!(body["error"]["details"]["write_outcome"], "unconfirmed");
                    assert_eq!(body["error"]["retryable"], false);
                } else {
                    assert_eq!(body["data"]["resource_ref"], "kugou:collection_1_222_88_0");
                    assert_eq!(body["data"]["subscribed"], add);
                    assert_eq!(
                        body["data"]["extensions"]["account_playlist_ref"],
                        "kugou:cloudlist:111:1:37"
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn kugou_ordering_routes_preserve_occurrence_payloads_accounts_and_rotation_on_every_outcome()
{
    for caller in [false, true] {
        for kind in ["read", "raw_order", "track_order", "library_order"] {
            for (failure, status) in [
                (None, StatusCode::OK),
                (Some(ErrorCode::UpstreamError), StatusCode::BAD_GATEWAY),
                (
                    Some(ErrorCode::AuthenticationRequired),
                    StatusCode::UNAUTHORIZED,
                ),
                (Some(ErrorCode::Conflict), StatusCode::CONFLICT),
            ] {
                let (method, path, mut payload) = match kind {
                    "read" => (
                        Method::GET,
                        "/v1/playlists/kugou:cloudlist:111:1:7/track-occurrences",
                        Value::Null,
                    ),
                    "raw_order" => (
                        Method::PUT,
                        "/v1/playlists/kugou:cloudlist:111:1:7/track-occurrences/order",
                        json!({"occurrence_ids":["entry:111:1:7:81"],"snapshot_id":"raw-before"}),
                    ),
                    "track_order" => (
                        Method::PUT,
                        "/v1/playlists/kugou:cloudlist:111:1:7/tracks/order",
                        json!({"refs":["kugou:901","kugou:901"]}),
                    ),
                    _ => (
                        Method::PUT,
                        "/v1/account/playlists/order",
                        json!({"refs":["kugou:cloudlist:111:0:7","kugou:cloudlist:111:0:1"]}),
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
                    .method(method)
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "application/json");
                if caller {
                    request = request.header(CALLER_CREDENTIAL_HEADER, tracks_caller());
                }
                let response = tracks_app(failure, false)
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
                assert_eq!(response.status(), status, "{kind}");
                assert_track_rotation(
                    &response,
                    caller
                        && !matches!(
                            failure,
                            Some(ErrorCode::AuthenticationRequired | ErrorCode::Conflict)
                        ),
                );
                let body: Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                        .unwrap();
                if failure.is_none() {
                    match kind {
                        "read" => {
                            assert_eq!(body["data"][0]["id"], "entry:111:1:7:81");
                            assert!(body["data"][0]["track"].is_null());
                            assert_eq!(
                                body["meta"]["pagination"]["extensions"]["source_snapshot_id"],
                                "raw-before"
                            );
                        }
                        "raw_order" => {
                            assert_eq!(body["data"]["snapshot_id"], "raw-after");
                            assert_eq!(body["data"]["occurrence_ids"], json!(["entry:111:1:7:81"]));
                        }
                        "track_order" => assert_eq!(
                            body["data"]["track_refs"],
                            json!(["kugou:901", "kugou:901"])
                        ),
                        _ => assert_eq!(body["data"]["playlist_refs"].as_array().unwrap().len(), 2),
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn occurrence_order_http_accepts_full_sized_lists_and_rejects_malformed_or_excessive_inputs()
{
    let path = "/v1/playlists/kugou:cloudlist:111:1:7/track-occurrences/order";
    for payload in [
        json!({"occurrence_ids":[]}),
        json!({"occurrence_ids":[],"snapshot_id":""}),
        json!({"occurrence_ids":[1],"snapshot_id":"raw-before"}),
        json!({"occurrence_ids":[],"snapshot_id":"raw-before","extra":1}),
        json!({"occurrence_ids":vec!["a";40001],"snapshot_id":"raw-before"}),
        json!({"occurrence_ids":["a".repeat(161)],"snapshot_id":"raw-before"}),
    ] {
        let response = tracks_app(None, false)
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri(path)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let payload=json!({"occurrence_ids":(0..38400).map(|n|format!("entry:{}:{n}","1".repeat(65))).collect::<Vec<_>>(),"snapshot_id":"raw-before","account":"A"}).to_string();
    assert!(payload.len() > 2 * 1024 * 1024);
    let response = tracks_app(None, false)
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(payload))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_track_rotation(&response, false);
    let response = tracks_app(None, false)
        .oneshot(
            Request::builder()
                .method(Method::PUT)
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(" ".repeat(8 * 1024 * 1024 + 1)))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

fn purchase_page(request: &PageRequest) -> PageMeta {
    PageMeta {
        limit: request.limit,
        offset: request.offset,
        total: Some(6),
        has_more: false,
        next_offset: None,
        extensions: Extensions::from([
            ("source_snapshot_id".into(), json!("purchase-snapshot")),
            ("unresolved_entries".into(), json!(1)),
            ("complete_read".into(), json!(true)),
        ]),
    }
}
#[tokio::test]
async fn kugou_purchase_routes_preserve_unresolved_records_and_deliver_safe_rotations_for_each_outcome()
 {
    for caller in [false, true] {
        for kind in ["tracks", "albums"] {
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
                    "/v1/account/purchases/{kind}?platform=kugou&limit=10&offset=5{}",
                    if caller { "" } else { "&account=A" }
                );
                let mut request = Request::builder().uri(uri);
                if caller {
                    request = request.header(CALLER_CREDENTIAL_HEADER, tracks_caller());
                }
                let response = tracks_app(failure, false)
                    .oneshot(request.body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status(), status);
                assert_track_rotation(
                    &response,
                    caller
                        && !matches!(
                            failure,
                            Some(ErrorCode::AuthenticationRequired | ErrorCode::Conflict)
                        ),
                );
                let body: Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                        .unwrap();
                if failure.is_none() {
                    assert_eq!(body["data"].as_array().unwrap().len(), 1);
                    assert!(
                        body["data"][0][if kind == "tracks" { "track" } else { "album" }].is_null()
                    );
                    assert!(body["data"][0].get("ref").is_none());
                    assert_eq!(body["meta"]["pagination"]["total"], 6);
                    assert_eq!(
                        body["meta"]["pagination"]["extensions"]["source_snapshot_id"],
                        "purchase-snapshot"
                    );
                }
            }
        }
    }
}
#[tokio::test]
async fn purchase_routes_reject_invalid_pagination_unknown_parameters_and_mixed_credentials() {
    for kind in ["tracks", "albums"] {
        for extra in [
            "&limit=0",
            "&offset=4294967295&limit=10",
            "&unknown=1",
            "&limit=wat",
        ] {
            let response = tracks_app(None, false)
                .oneshot(
                    Request::builder()
                        .uri(format!(
                            "/v1/account/purchases/{kind}?platform=kugou&account=A{extra}"
                        ))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }
        let response = tracks_app(None, false)
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/account/purchases/{kind}?platform=kugou&account=A&limit=10&offset=5"
                    ))
                    .header(CALLER_CREDENTIAL_HEADER, tracks_caller())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }
    let mut registry = ProviderRegistry::new();
    registry
        .register(tuneweave_provider_kugou::KugouProvider::new(Default::default()).unwrap())
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Kugou));
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/account/purchases/tracks?account=absent")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

const LEGACY_WEB_PLAYLIST_REF: &str = "kugou:legacy_web_collection:111:YWJj";
const LEGACY_WEB_PLAYLIST_ID: &str = "legacy_web_collection:111:YWJj";
const LEGACY_WEB_PLAYLIST_SNAPSHOT: &str = "kugou-legacy-web-raw-raw-fixture-a";

struct LegacyWebPlaylistImportProvider {
    changed_second_page: bool,
    unresolved_second_page: bool,
}

#[async_trait]
impl MusicProvider for LegacyWebPlaylistImportProvider {
    fn platform(&self) -> Platform {
        Platform::Kugou
    }

    fn name(&self) -> &'static str {
        "Kugou legacy Web playlist import fixture"
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::PlaylistRead])
    }

    async fn playlist(&self, id: &str, account: Option<&str>) -> Result<Playlist> {
        assert_eq!(id, LEGACY_WEB_PLAYLIST_ID);
        assert_eq!(account, Some("A"));
        Ok(Playlist {
            resource_ref: ResourceRef::new(Platform::Kugou, id).unwrap(),
            platform: Platform::Kugou,
            id: id.to_owned(),
            name: "Legacy Web playlist".to_owned(),
            description: String::new(),
            cover_url: None,
            creator: None,
            track_count: Some(3),
            tags: Vec::new(),
            subscribed: None,
            created_at: None,
            updated_at: None,
            extensions: Extensions::from([(
                "source_snapshot_id".to_owned(),
                json!(LEGACY_WEB_PLAYLIST_SNAPSHOT),
            )]),
        })
    }

    async fn playlist_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        assert_eq!(id, LEGACY_WEB_PLAYLIST_ID);
        assert_eq!(request.account.as_deref(), Some("A"));
        let (ids, next_offset, snapshot) = match request.offset {
            0 => (vec!["701", "702"], Some(2), LEGACY_WEB_PLAYLIST_SNAPSHOT),
            2 if self.unresolved_second_page => {
                return Err(TuneWeaveError::new(
                    ErrorCode::UpstreamError,
                    "legacy Web occurrence has no complete canonical track identity",
                )
                .with_platform(Platform::Kugou));
            }
            2 => (
                vec!["702"],
                None,
                if self.changed_second_page {
                    "kugou-legacy-web-raw-raw-fixture-b"
                } else {
                    LEGACY_WEB_PLAYLIST_SNAPSHOT
                },
            ),
            offset => panic!("unexpected legacy Web playlist offset {offset}"),
        };
        Ok(Page {
            items: ids
                .into_iter()
                .map(|id| {
                    Track::new(
                        ResourceRef::new(Platform::Kugou, id).unwrap(),
                        "Legacy song",
                    )
                })
                .collect(),
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(3),
                has_more: next_offset.is_some(),
                next_offset,
                extensions: Extensions::from([("source_snapshot_id".to_owned(), json!(snapshot))]),
            },
        })
    }

    async fn playlist_source(
        &self,
        id: &str,
        source_type: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        assert_eq!(source_type, "playlist");
        self.playlist(id, account).await
    }

    async fn playlist_source_items(
        &self,
        id: &str,
        source_type: &str,
        request: &PageRequest,
    ) -> Result<Page<PlaylistPlayableItem>> {
        assert_eq!(source_type, "playlist");
        let tracks = self.playlist_tracks(id, request).await?;
        Ok(Page {
            items: tracks
                .items
                .into_iter()
                .map(PlaylistPlayableItem::Track)
                .collect(),
            pagination: tracks.pagination,
        })
    }
}

fn legacy_web_playlist_app(changed_second_page: bool, unresolved_second_page: bool) -> Router {
    let mut registry = ProviderRegistry::new();
    registry
        .register(LegacyWebPlaylistImportProvider {
            changed_second_page,
            unresolved_second_page,
        })
        .unwrap();
    build_router(AppState::new(registry, Platform::Kugou))
}

#[tokio::test]
async fn kugou_legacy_web_playlist_reads_expose_the_consistent_snapshot_and_keep_duplicates() {
    let app = legacy_web_playlist_app(false, false);
    let metadata = json_response_from(
        app.clone(),
        &format!("/v1/playlists/{LEGACY_WEB_PLAYLIST_REF}?account=A"),
    )
    .await;
    assert_eq!(metadata.0, StatusCode::OK);
    assert_eq!(metadata.1["data"]["ref"], LEGACY_WEB_PLAYLIST_REF);
    assert_eq!(
        metadata.1["data"]["extensions"]["source_snapshot_id"],
        LEGACY_WEB_PLAYLIST_SNAPSHOT
    );

    for (offset, expected_refs) in [(0, vec!["kugou:701", "kugou:702"]), (2, vec!["kugou:702"])] {
        let response = json_response_from(
            app.clone(),
            &format!(
                "/v1/playlists/{LEGACY_WEB_PLAYLIST_REF}/tracks?account=A&limit=2&offset={offset}"
            ),
        )
        .await;
        assert_eq!(response.0, StatusCode::OK);
        let actual_refs = response.1["data"]
            .as_array()
            .unwrap()
            .iter()
            .map(|track| track["ref"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(actual_refs, expected_refs);
        assert_eq!(
            response.1["meta"]["pagination"]["extensions"]["source_snapshot_id"],
            LEGACY_WEB_PLAYLIST_SNAPSHOT
        );
    }
}

#[tokio::test]
async fn kugou_legacy_web_playlist_uni_imports_preserve_order_and_duplicates_or_fail_whole() {
    for materialize in [false, true] {
        for (changed, unresolved) in [(false, false), (true, false), (false, true)] {
            let app = legacy_web_playlist_app(changed, unresolved);
            let uri = if materialize {
                "/v1/uni/materialize/imports"
            } else {
                "/v1/uni/playlists/imports"
            };
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri(uri)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(
                            json!({"sources":[{"ref":LEGACY_WEB_PLAYLIST_REF,"type":"playlist","account":"A"}]})
                                .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            let expected_failure = changed || unresolved;
            assert_eq!(
                response.status(),
                if expected_failure {
                    StatusCode::BAD_GATEWAY
                } else {
                    StatusCode::OK
                },
                "materialize={materialize}, changed={changed}, unresolved={unresolved}"
            );
            let body: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                    .unwrap();
            let directory = json_response_from(app.clone(), "/v1/uni/playlists").await.1;
            if expected_failure {
                assert_eq!(directory["data"], json!([]));
                continue;
            }

            let items = if materialize {
                assert_eq!(directory["data"], json!([]));
                body["data"]["items"].clone()
            } else {
                let imported_ref = body["data"]["playlist"]["ref"].as_str().unwrap();
                json_response_from(app, &format!("/v1/uni/playlists/{imported_ref}/items"))
                    .await
                    .1["data"]
                    .clone()
            };
            let imported_refs = items
                .as_array()
                .unwrap()
                .iter()
                .map(|item| item["source_ref"].as_str().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(imported_refs, vec!["kugou:701", "kugou:702", "kugou:702"]);
            assert_ne!(items[1]["id"], items[2]["id"]);
            assert_eq!(items[0]["position"], 0);
            assert_eq!(items[1]["position"], 1);
            assert_eq!(items[2]["position"], 2);
        }
    }
}
