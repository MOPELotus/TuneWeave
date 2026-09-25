use super::*;

mod video;

#[derive(Clone)]
struct MediaProvider {
    platform: Platform,
    caller: bool,
    denied: bool,
    download_unsupported: bool,
    failure: Option<ErrorCode>,
    calls: Arc<Mutex<Vec<String>>>,
    update: Arc<Mutex<Option<ProviderCredential>>>,
}
impl MediaProvider {
    fn accept(&self, operation: &str, account: Option<&str>) -> Result<()> {
        if self.platform == Platform::Kugou {
            assert_eq!(
                account,
                Some(if self.caller {
                    "default"
                } else if operation.starts_with("source") {
                    "library-account"
                } else {
                    "A"
                })
            );
            if self.caller {
                *self.update.lock().unwrap() = Some(
                    ProviderCredential::new(Platform::Kugou, "test", "rotated-media-http", None)
                        .unwrap(),
                );
            }
        }
        self.calls
            .lock()
            .unwrap()
            .push(format!("{}:{operation}", self.platform));
        if let Some(code) = self.failure {
            if !matches!(operation, "track" | "video") && !operation.starts_with("source") {
                return Err(TuneWeaveError::new(code, "media authorization failed")
                    .with_platform(self.platform));
            }
        }
        Ok(())
    }
    fn track_value(&self, id: &str) -> Track {
        let mut t = Track::new(
            ResourceRef::new(self.platform, id).unwrap(),
            "Verified Song",
        );
        t.artists = vec![ArtistSummary {
            name: "Artist".into(),
            resource_ref: None,
        }];
        t.duration_ms = Some(123000);
        t
    }
}
#[async_trait]
impl MusicProvider for MediaProvider {
    fn platform(&self) -> Platform {
        self.platform
    }
    fn name(&self) -> &'static str {
        "Account media HTTP fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::CallerManagedCredentials,
            Capability::TrackDetail,
            Capability::SearchTracks,
            Capability::AudioStream,
            Capability::AudioDownload,
            Capability::TrackAvailability,
            Capability::PlaylistRead,
            Capability::VideoDetail,
            Capability::VideoStream,
            Capability::Lyrics,
        ])
    }
    fn with_caller_credential(&self, c: &ProviderCredential) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(c.platform, self.platform);
        assert_eq!(c.secret(), "original-media-http");
        Ok(Arc::new(Self {
            caller: true,
            update: Arc::default(),
            ..self.clone()
        }))
    }
    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        Ok(self.update.lock().unwrap().take())
    }
    fn requires_download_authorization(&self, account: Option<&str>) -> bool {
        self.platform == Platform::Kugou && (account.is_some() || self.caller)
    }
    async fn track(&self, id: &str, account: Option<&str>) -> Result<Track> {
        self.accept("track", account)?;
        Ok(self.track_value(id))
    }
    async fn lyrics_with_options(&self, id: &str, wanted: &LyricsRequest) -> Result<Lyrics> {
        self.accept("lyrics", wanted.account.as_deref())?;
        assert!(wanted.word_synced && wanted.translated && wanted.romanized);
        assert!(!wanted.singing_annotations && wanted.song_type.is_none());
        let plain_only = id == "902";
        Ok(Lyrics {
            track_ref: ResourceRef::new(self.platform, id).unwrap(),
            plain: Some("[00:01.00]line".into()),
            word_synced: (!plain_only).then(|| "[1000,1000]<0,1000,0>word".into()),
            translated: (!plain_only).then(|| "translation".into()),
            romanized: (!plain_only).then(|| "romanized".into()),
            singing_annotations: None,
            singing_annotations_timestamp: None,
            format: if plain_only { "lrc" } else { "krc" }.into(),
            contributors: vec![],
            extensions: Extensions::new(),
        })
    }
    async fn video(&self, id: &str, request: &VideoDetailRequest) -> Result<VideoDetail> {
        self.accept("video", request.account.as_deref())?;
        let mut video = sample_video(id);
        video.platform = self.platform;
        video.resource_ref = ResourceRef::new(self.platform, id).unwrap();
        Ok(VideoDetail {
            kind: request.kind,
            video,
            resolutions: vec![],
            extensions: Extensions::new(),
        })
    }
    async fn video_stream(&self, id: &str, request: &VideoStreamRequest) -> Result<VideoStream> {
        self.accept("video_stream", request.account.as_deref())?;
        Ok(VideoStream {
            video_ref: ResourceRef::new(self.platform, id).unwrap(),
            platform: self.platform,
            available: !self.denied,
            url: (!self.denied).then(|| "https://mvwebfs.tx.kugou.com/account.mp4".into()),
            backup_urls: vec![],
            headers: BTreeMap::new(),
            expires_at: None,
            format: None,
            codec: None,
            width: Some(768),
            height: Some(432),
            size: Some(1000),
            duration_ms: Some(123456),
            source_range: None,
            requested_resolution: request.resolution,
            actual_resolution: Some(432),
            platform_code: Some(0),
            fee: None,
            message: None,
            extensions: Extensions::new(),
        })
    }
    async fn track_availability(
        &self,
        id: &str,
        request: &TrackAvailabilityRequest,
    ) -> Result<TrackAvailability> {
        self.accept("availability", request.account.as_deref())?;
        assert_eq!(request.bitrate, TrackAvailabilityRequest::DEFAULT_BITRATE);
        Ok(TrackAvailability {
            track_ref: ResourceRef::new(self.platform, id).unwrap(),
            playable: !self.denied,
            requested_bitrate: request.bitrate,
            actual_bitrate: if self.denied { None } else { Some(128000) },
            platform_code: Some(0),
            message: "Account availability".into(),
            extensions: Extensions::new(),
        })
    }
    async fn playlist_source(
        &self,
        id: &str,
        kind: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        self.accept("source", account)?;
        assert_eq!(kind, "playlist");
        Ok(Playlist {
            resource_ref: ResourceRef::new(self.platform, id).unwrap(),
            platform: self.platform,
            id: id.into(),
            name: "Source library".into(),
            description: String::new(),
            cover_url: None,
            creator: None,
            track_count: Some(2),
            tags: vec![],
            subscribed: None,
            created_at: None,
            updated_at: None,
            extensions: Extensions::from([("source_snapshot_id".into(), json!("stable-source"))]),
        })
    }
    async fn playlist_source_items(
        &self,
        _id: &str,
        kind: &str,
        request: &PageRequest,
    ) -> Result<Page<PlaylistPlayableItem>> {
        self.accept("source_items", request.account.as_deref())?;
        assert_eq!(kind, "playlist");
        assert_eq!(request.offset, 0);
        Ok(Page {
            items: vec![PlaylistPlayableItem::Track(self.track_value("901")); 2],
            pagination: PageMeta {
                limit: request.limit,
                offset: 0,
                total: Some(2),
                next_offset: None,
                has_more: false,
                extensions: Extensions::from([(
                    "source_snapshot_id".into(),
                    json!("stable-source"),
                )]),
            },
        })
    }
    async fn search(&self, q: &SearchQuery) -> Result<Page<Track>> {
        Ok(Page {
            items: vec![self.track_value("901")],
            pagination: PageMeta {
                limit: q.limit,
                offset: q.offset,
                total: Some(1),
                next_offset: None,
                has_more: false,
                extensions: Extensions::new(),
            },
        })
    }
    async fn stream(&self, track: &Track, request: &StreamRequest) -> Result<MediaStream> {
        self.accept("stream", request.account.as_deref())?;
        Ok(MediaStream {
            url: "https://fs.kugou.com/play-only.mp3".into(),
            backup_urls: vec![],
            headers: BTreeMap::new(),
            expires_at: None,
            format: Some("mp3".into()),
            codec: Some("mp3".into()),
            bitrate: Some(128000),
            size: Some(1975000),
            duration_ms: Some(123000),
            requested_quality: request.quality,
            actual_quality: Quality::Standard,
            trial: None,
            origin_track: Some(track.resource_ref.clone()),
            resolved_track: track.resource_ref.clone(),
            resolved_platform: self.platform,
            match_score: Some(1.0),
            attempts: vec![],
        })
    }
    async fn download(&self, track: &Track, request: &StreamRequest) -> Result<MediaDownload> {
        self.accept("download", request.account.as_deref())?;
        if self.download_unsupported && self.platform == Platform::Kugou {
            return Err(TuneWeaveError::unsupported(
                Platform::Kugou,
                Capability::AudioDownload,
            ));
        }
        Ok(MediaDownload {
            track_ref: track.resource_ref.clone(),
            platform: self.platform,
            available: !self.denied,
            url: (!self.denied).then(|| "https://fs.kugou.com/download-authorized.mp3".into()),
            headers: BTreeMap::new(),
            expires_at: None,
            format: Some("mp3".into()),
            codec: Some("mp3".into()),
            bitrate: Some(128000),
            size: Some(1975000),
            duration_ms: Some(123000),
            requested_quality: request.quality,
            actual_quality: Quality::Standard,
            platform_code: Some(if self.denied { 3 } else { 1 }),
            fee: None,
            message: None,
            extensions: Extensions::new(),
        })
    }
}

fn app(denied: bool, failure: Option<ErrorCode>) -> (Router, Arc<Mutex<Vec<String>>>) {
    media_app(denied, failure, false)
}
fn media_app(
    denied: bool,
    failure: Option<ErrorCode>,
    download_unsupported: bool,
) -> (Router, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::new(Mutex::new(vec![]));
    let mut registry = ProviderRegistry::new();
    for platform in [Platform::Kugou, Platform::Netease] {
        registry
            .register(MediaProvider {
                platform,
                caller: false,
                denied,
                download_unsupported,
                failure: if platform == Platform::Kugou {
                    failure
                } else {
                    None
                },
                calls: calls.clone(),
                update: Arc::default(),
            })
            .unwrap();
    }
    (
        build_router(AppState::new(registry, Platform::Kugou)),
        calls,
    )
}
fn request(uri: &str, caller: bool) -> Request<Body> {
    let mut b = Request::builder().uri(uri);
    if caller {
        b = b.header(
            CALLER_CREDENTIAL_HEADER,
            CallerCredential::issue(
                &ProviderCredential::new(Platform::Kugou, "test", "original-media-http", None)
                    .unwrap(),
            )
            .unwrap()
            .value,
        );
    }
    b.body(Body::empty()).unwrap()
}
async fn inspect(response: Response, status: StatusCode, rotated: bool) -> Value {
    assert_eq!(response.status(), status);
    assert!(
        response.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("no-store")
    );
    let headers = response
        .headers()
        .get_all(caller_scope::UPDATED_CREDENTIAL_HEADER)
        .iter()
        .collect::<Vec<_>>();
    assert_eq!(headers.len(), usize::from(rotated));
    if rotated {
        assert!(headers[0].is_sensitive());
        let c =
            CallerCredential::parse(headers[0].to_str().unwrap().strip_prefix("kugou=").unwrap())
                .unwrap();
        assert_eq!(c.secret(), "rotated-media-http");
    }
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    let text = String::from_utf8_lossy(&bytes);
    assert!(!text.contains("original-media-http") && !text.contains("rotated-media-http"));
    if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    }
}

#[tokio::test]
async fn kugou_account_stream_download_and_batch_preserve_scoped_rotations_and_no_store() {
    for caller in [false, true] {
        for endpoint in [
            "/v1/tracks/kugou:901/stream?fallback=false",
            "/v1/tracks/kugou:901/download?fallback=false",
            "/v1/tracks/streams?refs=kugou:901,kugou:901&fallback=false",
        ] {
            let (app, _) = app(false, None);
            let uri = if caller {
                endpoint.into()
            } else {
                format!("{endpoint}&account=A")
            };
            let value = inspect(
                app.oneshot(request(&uri, caller)).await.unwrap(),
                StatusCode::OK,
                caller,
            )
            .await;
            assert_eq!(value["ok"], true);
        }
    }
}

#[tokio::test]
async fn denied_account_download_cannot_fall_back_to_an_available_full_stream() {
    for caller in [false, true] {
        for redirect in [false, true] {
            let (app, calls) = app(true, None);
            let mut uri = format!(
                "/v1/tracks/kugou:901/download{}?fallback=false",
                if redirect { "/redirect" } else { "" }
            );
            if !caller {
                uri.push_str("&account=A");
            }
            let response = app.oneshot(request(&uri, caller)).await.unwrap();
            assert!(response.headers().get(header::LOCATION).is_none());
            let body = inspect(
                response,
                if redirect {
                    StatusCode::FORBIDDEN
                } else {
                    StatusCode::OK
                },
                caller,
            )
            .await;
            if !redirect {
                assert_eq!(body["data"]["available"], false);
                assert!(body["data"]["url"].is_null());
            }
            assert_eq!(*calls.lock().unwrap(), ["kugou:track", "kugou:download"]);
        }
    }
}

#[tokio::test]
async fn cross_platform_download_requires_the_resolved_account_download_grant() {
    for denied in [false, true] {
        for caller in [false, true] {
            let (app, calls) = app(denied, None);
            let mut uri =
                "/v1/tracks/netease:10/download?playback_platform=kugou&fallback=false".to_owned();
            if !caller {
                uri.push_str("&account=A");
            }
            let body = inspect(
                app.oneshot(request(&uri, caller)).await.unwrap(),
                StatusCode::OK,
                caller,
            )
            .await;
            assert_eq!(body["data"]["available"], !denied);
            assert_eq!(body["data"]["ref"], "netease:10");
            if !denied {
                assert_eq!(
                    body["data"]["url"],
                    "https://fs.kugou.com/download-authorized.mp3"
                );
            }
            assert_eq!(
                calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|v| *v == "kugou:download")
                    .count(),
                1
            );
        }
    }
}

#[tokio::test]
async fn media_auth_and_conflict_errors_suppress_caller_updates_including_redirects() {
    for (failure, status, update) in [
        (
            ErrorCode::AuthenticationRequired,
            StatusCode::UNAUTHORIZED,
            false,
        ),
        (ErrorCode::Conflict, StatusCode::CONFLICT, false),
        (ErrorCode::UpstreamError, StatusCode::BAD_GATEWAY, true),
    ] {
        let (app, _) = app(false, Some(failure));
        inspect(
            app.oneshot(request(
                "/v1/tracks/kugou:901/download/redirect?fallback=false",
                true,
            ))
            .await
            .unwrap(),
            status,
            update,
        )
        .await;
    }
    let (app, _) = app(false, None);
    let response = app
        .oneshot(request(
            "/v1/tracks/kugou:901/stream/redirect?fallback=false",
            true,
        ))
        .await
        .unwrap();
    assert_eq!(
        response.headers()[header::LOCATION],
        "https://fs.kugou.com/play-only.mp3"
    );
    inspect(response, StatusCode::FOUND, true).await;
}

fn post(uri: &str, body: Value, caller: bool) -> Request<Body> {
    let mut request = request(uri, caller);
    *request.method_mut() = Method::POST;
    request.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    *request.body_mut() = Body::from(body.to_string());
    request
}

async fn import_media_items(
    router: &Router,
    platform: Platform,
    materialize: bool,
    caller: bool,
) -> (Vec<Value>, Option<String>) {
    let mut source = json!({"ref":format!("{platform}:source-library"),"type":"playlist"});
    if platform == Platform::Kugou && !caller {
        source["account"] = json!("library-account");
    }
    let response = router
        .clone()
        .oneshot(post(
            if materialize {
                "/v1/uni/materialize/imports"
            } else {
                "/v1/uni/playlists/imports"
            },
            json!({"sources":[source]}),
            caller,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .contains_key(caller_scope::UPDATED_CREDENTIAL_HEADER),
        caller && platform == Platform::Kugou
    );
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    let (items, reference) = if materialize {
        (body["data"]["items"].clone(), None)
    } else {
        let reference = body["data"]["playlist"]["ref"].as_str().unwrap().to_owned();
        let (_, body) = json_response_from(
            router.clone(),
            &format!("/v1/uni/playlists/{reference}/items"),
        )
        .await;
        (body["data"].clone(), Some(reference))
    };
    let items = items.as_array().unwrap().clone();
    assert_eq!(items.len(), 2);
    assert_ne!(items[0]["id"], items[1]["id"]);
    for item in &items {
        let text = item.to_string();
        assert!(
            !text.contains("library-account")
                && !text.contains("media-http")
                && !text.contains("twc1_")
        );
    }
    (items, reference)
}

#[tokio::test]
async fn kugou_availability_http_preserves_denials_error_codes_and_caller_rotation() {
    for caller in [false, true] {
        for denied in [false, true] {
            let (router, calls) = app(denied, None);
            let body = inspect(
                router
                    .oneshot(request(
                        if caller {
                            "/v1/tracks/kugou:901/availability"
                        } else {
                            "/v1/tracks/kugou:901/availability?account=A"
                        },
                        caller,
                    ))
                    .await
                    .unwrap(),
                StatusCode::OK,
                caller,
            )
            .await;
            assert_eq!(body["data"]["playable"], !denied);
            assert_eq!(*calls.lock().unwrap(), ["kugou:availability"]);
        }
    }
    for (code, status, rotation) in [
        (
            ErrorCode::AuthenticationRequired,
            StatusCode::UNAUTHORIZED,
            false,
        ),
        (ErrorCode::Conflict, StatusCode::CONFLICT, false),
        (ErrorCode::UpstreamError, StatusCode::BAD_GATEWAY, true),
    ] {
        let (router, _) = app(false, Some(code));
        inspect(
            router
                .oneshot(request("/v1/tracks/kugou:901/availability", true))
                .await
                .unwrap(),
            status,
            rotation,
        )
        .await;
    }
}

#[tokio::test]
async fn kugou_uni_import_and_materialization_separate_source_accounts_from_playback_accounts() {
    for platform in [Platform::Kugou, Platform::Netease] {
        for materialize in [false, true] {
            for source_caller in [false, true] {
                for playback_caller in [false, true] {
                    let (router, calls) = app(false, None);
                    let (items, reference) =
                        import_media_items(&router, platform, materialize, source_caller).await;
                    calls.lock().unwrap().clear();
                    let request = if let Some(reference) = reference {
                        let mut uri = format!(
                            "/v1/playlists/{reference}/items/{}/stream?playback_platform=kugou&fallback=false",
                            items[1]["id"].as_str().unwrap()
                        );
                        if !playback_caller {
                            uri.push_str("&account=A");
                        }
                        request(&uri, playback_caller)
                    } else {
                        let mut body =
                            json!({"item":items[1],"playback_platform":"kugou","fallback":false});
                        if !playback_caller {
                            body["accounts"] = json!({"kugou":"A"});
                        }
                        post("/v1/uni/items/stream", body, playback_caller)
                    };
                    let body = inspect(
                        router.oneshot(request).await.unwrap(),
                        StatusCode::OK,
                        playback_caller,
                    )
                    .await;
                    assert_eq!(body["data"]["stream"]["resolved_track"], "kugou:901");
                    assert_eq!(body["data"]["source_ref"], format!("{platform}:901"));
                    assert_eq!(*calls.lock().unwrap(), ["kugou:stream"]);
                }
            }
        }
    }
}

#[tokio::test]
async fn kugou_uni_native_stream_errors_do_not_reuse_source_identity_or_fall_back_when_disabled() {
    for materialize in [false, true] {
        for (code, status, rotation) in [
            (
                ErrorCode::AuthenticationRequired,
                StatusCode::UNAUTHORIZED,
                false,
            ),
            (ErrorCode::Conflict, StatusCode::CONFLICT, false),
            (ErrorCode::PermissionDenied, StatusCode::FORBIDDEN, true),
            (ErrorCode::UpstreamError, StatusCode::BAD_GATEWAY, true),
        ] {
            let (router, calls) = app(false, Some(code));
            let (items, reference) =
                import_media_items(&router, Platform::Kugou, materialize, false).await;
            calls.lock().unwrap().clear();
            let request = if let Some(reference) = reference {
                request(
                    &format!(
                        "/v1/playlists/{reference}/items/{}/stream/redirect?fallback=false",
                        items[0]["id"].as_str().unwrap()
                    ),
                    true,
                )
            } else {
                post(
                    "/v1/uni/items/stream",
                    json!({"item":items[0],"fallback":false}),
                    true,
                )
            };
            let response = router.oneshot(request).await.unwrap();
            assert!(!response.headers().contains_key(header::LOCATION));
            inspect(response, status, rotation).await;
            assert_eq!(*calls.lock().unwrap(), ["kugou:stream"]);
        }
    }
}

#[tokio::test]
async fn kugou_uni_unmatched_candidates_obey_explicit_and_automatic_fallback_controls() {
    for unblock in [false, true] {
        let (router, calls) = app(false, None);
        let (mut items, _) = import_media_items(&router, Platform::Netease, true, false).await;
        items[0]["snapshot"]["title"] = json!("Completely unrelated composition");
        items[0]["snapshot"]["artists"] = json!(["Other singer"]);
        calls.lock().unwrap().clear();
        let response = router.oneshot(post(
            "/v1/uni/items/stream",
            json!({"item":items[0],"playback_platform":"kugou","fallback":false,"unblock":unblock}),
            true,
        )).await.unwrap();
        assert!(!response.headers().contains_key(header::LOCATION));
        let body = inspect(
            response,
            if unblock {
                StatusCode::OK
            } else {
                StatusCode::UNPROCESSABLE_ENTITY
            },
            false,
        )
        .await;
        if unblock {
            assert_eq!(body["data"]["stream"]["resolved_platform"], "netease");
            assert_eq!(body["data"]["stream"]["attempts"][0]["status"], "no_match");
            assert_eq!(*calls.lock().unwrap(), ["netease:stream"]);
        } else {
            assert_eq!(body["error"]["code"], "match_rejected");
            assert!(calls.lock().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn kugou_web_playback_only_scope_rejects_direct_and_resolved_downloads() {
    for caller in [false, true] {
        for source in ["kugou:901", "netease:10"] {
            for redirect in [false, true] {
                let (router, calls) = media_app(false, None, true);
                let mut uri = format!(
                    "/v1/tracks/{source}/download{}?playback_platform=kugou&fallback=false&unblock=false",
                    if redirect { "/redirect" } else { "" }
                );
                if !caller {
                    uri.push_str("&account=A");
                }
                let response = router.oneshot(request(&uri, caller)).await.unwrap();
                assert!(response.headers().get(header::LOCATION).is_none());
                let body = inspect(response, StatusCode::UNPROCESSABLE_ENTITY, caller).await;
                assert_eq!(body["error"]["code"], "capability_not_supported");
                assert!(!body.to_string().contains("play-only.mp3"));
                assert_eq!(
                    calls
                        .lock()
                        .unwrap()
                        .iter()
                        .filter(|v| *v == "kugou:download")
                        .count(),
                    1
                );
                if source.starts_with("netease") {
                    assert!(calls.lock().unwrap().contains(&"kugou:stream".into()));
                } else {
                    assert!(!calls.lock().unwrap().contains(&"kugou:stream".into()));
                }
            }
        }
    }
}

#[tokio::test]
async fn kugou_lyrics_http_preserves_display_flags_rotation_and_account_errors() {
    for caller in [false, true] {
        for (failure, status) in [
            (None, StatusCode::OK),
            (
                Some(ErrorCode::AuthenticationRequired),
                StatusCode::UNAUTHORIZED,
            ),
            (Some(ErrorCode::Conflict), StatusCode::CONFLICT),
            (Some(ErrorCode::UpstreamError), StatusCode::BAD_GATEWAY),
            (Some(ErrorCode::RateLimited), StatusCode::TOO_MANY_REQUESTS),
        ] {
            let (router, calls) = app(false, failure);
            let uri = format!(
                "/v1/tracks/kugou:901/lyrics?qrc=true&trans=true&roma=true{}",
                if caller { "" } else { "&account=A" }
            );
            let update = caller
                && !matches!(
                    failure,
                    Some(ErrorCode::AuthenticationRequired | ErrorCode::Conflict)
                );
            let body = inspect(
                router.oneshot(request(&uri, caller)).await.unwrap(),
                status,
                update,
            )
            .await;
            assert_eq!(*calls.lock().unwrap(), vec!["kugou:lyrics"]);
            if failure.is_none() {
                assert_eq!(body["data"]["format"], "krc");
                assert_eq!(body["data"]["translated"], "translation");
                assert_eq!(body["data"]["romanized"], "romanized");
            }
        }
    }
}

#[tokio::test]
async fn kugou_web_style_lyrics_keep_missing_optional_tracks_and_current_credentials() {
    for caller in [false, true] {
        let (router, calls) = app(false, None);
        let uri = format!(
            "/v1/tracks/kugou:902/lyrics?word_synced=true&translated=true&romanized=true{}",
            if caller { "" } else { "&account=A" }
        );
        let body = inspect(
            router.oneshot(request(&uri, caller)).await.unwrap(),
            StatusCode::OK,
            caller,
        )
        .await;
        assert_eq!(body["data"]["format"], "lrc");
        assert_eq!(body["data"]["plain"], "[00:01.00]line");
        for field in [
            "word_synced",
            "translated",
            "romanized",
            "singing_annotations",
        ] {
            assert!(body["data"][field].is_null());
        }
        assert_eq!(*calls.lock().unwrap(), vec!["kugou:lyrics"]);
    }
}
