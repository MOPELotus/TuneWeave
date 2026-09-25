//! HTTP contract fixtures; native protocol behavior is tested in the provider crate.
use super::*;
use tuneweave_core::TrialWindow;

#[derive(Clone)]
struct Provider {
    account: &'static str,
    denied: bool,
    failure: Option<ErrorCode>,
    calls: Arc<Mutex<Vec<&'static str>>>,
}
impl Provider {
    fn accept(&self, operation: &'static str, account: Option<&str>) -> Result<()> {
        assert_eq!(account, Some(self.account));
        self.calls.lock().unwrap().push(operation);
        if operation != "track" {
            if let Some(code) = self.failure {
                return Err(TuneWeaveError::new(code, "Account media unavailable")
                    .with_platform(Platform::Kuwo));
            }
        }
        Ok(())
    }
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Kuwo
    }
    fn name(&self) -> &'static str {
        "Kuwo account media contract"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::CallerManagedCredentials,
            Capability::TrackDetail,
            Capability::AudioStream,
            Capability::AudioDownload,
            Capability::TrackAvailability,
        ])
    }
    fn with_caller_credential(&self, c: &ProviderCredential) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(c.secret(), "private-kuwo-media-fixture");
        Ok(Arc::new(Self {
            account: "default",
            ..self.clone()
        }))
    }
    fn requires_download_authorization(&self, _account: Option<&str>) -> bool {
        true
    }
    async fn track(&self, id: &str, account: Option<&str>) -> Result<Track> {
        self.accept("track", account)?;
        Ok(Track::new(
            ResourceRef::new(Platform::Kuwo, id).unwrap(),
            "Synthetic media",
        ))
    }
    async fn stream(&self, t: &Track, r: &StreamRequest) -> Result<MediaStream> {
        self.accept("stream", r.account.as_deref())?;
        if self.denied {
            return Err(
                TuneWeaveError::new(ErrorCode::PermissionDenied, "Full media denied")
                    .with_platform(Platform::Kuwo),
            );
        }
        Ok(MediaStream {
            url: "https://er-sycdn.kuwo.cn/account/song.mp3".into(),
            backup_urls: vec![],
            headers: BTreeMap::new(),
            expires_at: None,
            format: Some("mp3".into()),
            codec: Some("mp3".into()),
            bitrate: Some(128000),
            size: None,
            duration_ms: Some(240000),
            requested_quality: r.quality,
            actual_quality: Quality::Standard,
            trial: None,
            origin_track: Some(t.resource_ref.clone()),
            resolved_track: t.resource_ref.clone(),
            resolved_platform: Platform::Kuwo,
            match_score: Some(1.),
            attempts: vec![],
        })
    }
    async fn audio_content(&self, t: &Track, r: &StreamRequest) -> Result<AudioContent> {
        self.accept("audio_content", r.account.as_deref())?;
        Ok(AudioContent {
            track_ref: t.resource_ref.clone(),
            bytes: b"synthetic-play-bytes".to_vec(),
            content_type: if r.variant == StreamVariant::SingAlong {
                "audio/wav"
            } else if matches!(
                r.quality,
                Quality::Master | Quality::Spatial | Quality::Vinyl
            ) {
                "audio/flac"
            } else {
                "audio/mpeg"
            }
            .into(),
            filename: if r.variant == StreamVariant::SingAlong {
                "kuwo-67474-sing-along.wav"
            } else if r.quality == Quality::Vinyl {
                "kuwo-67474-vinyl.flac"
            } else if r.quality == Quality::Spatial {
                "kuwo-67474-spatial.flac"
            } else if r.quality == Quality::Master {
                "kuwo-67474-master.flac"
            } else {
                "kuwo-67474.mp3"
            }
            .into(),
            trial: content_trial(&t.id),
        })
    }
    async fn audio_download_content(&self, t: &Track, r: &StreamRequest) -> Result<AudioContent> {
        self.accept("download_content", r.account.as_deref())?;
        if self.denied {
            return Err(
                TuneWeaveError::new(ErrorCode::PermissionDenied, "Download rights denied")
                    .with_platform(Platform::Kuwo),
            );
        }
        Ok(AudioContent {
            track_ref: t.resource_ref.clone(),
            bytes: b"synthetic-download-bytes".to_vec(),
            content_type: if r.variant == StreamVariant::SingAlong {
                "audio/wav"
            } else {
                "audio/flac"
            }
            .into(),
            filename: if r.variant == StreamVariant::SingAlong {
                "kuwo-67474-sing-along.wav"
            } else if r.quality == Quality::Vinyl {
                "kuwo-67474-vinyl.flac"
            } else if r.quality == Quality::Spatial {
                "kuwo-67474-spatial.flac"
            } else if r.quality == Quality::Master {
                "kuwo-67474-master.flac"
            } else {
                "kuwo-67474.flac"
            }
            .into(),
            trial: content_trial(&t.id),
        })
    }
    async fn download(&self, t: &Track, r: &StreamRequest) -> Result<MediaDownload> {
        self.accept("download", r.account.as_deref())?;
        Ok(MediaDownload {
            track_ref: t.resource_ref.clone(),
            platform: Platform::Kuwo,
            available: !self.denied,
            url: (!self.denied).then(|| "https://er-sycdn.kuwo.cn/account/song.mp3".into()),
            headers: BTreeMap::new(),
            expires_at: None,
            format: Some("mp3".into()),
            codec: Some("mp3".into()),
            bitrate: Some(128000),
            size: None,
            duration_ms: Some(240000),
            requested_quality: r.quality,
            actual_quality: Quality::Standard,
            platform_code: Some(if self.denied { 4018 } else { 200 }),
            fee: None,
            message: None,
            extensions: Extensions::from([("scope".into(), json!("native_account"))]),
        })
    }
    async fn track_availability(
        &self,
        id: &str,
        r: &TrackAvailabilityRequest,
    ) -> Result<TrackAvailability> {
        self.accept("availability", r.account.as_deref())?;
        Ok(TrackAvailability {
            track_ref: ResourceRef::new(Platform::Kuwo, id).unwrap(),
            playable: !self.denied,
            requested_bitrate: r.bitrate,
            actual_bitrate: (!self.denied).then_some(128000),
            platform_code: Some(200),
            message: "Account media".into(),
            extensions: Extensions::new(),
        })
    }
}
fn content_trial(id: &str) -> Option<TrialWindow> {
    match id {
        "trial" => Some(TrialWindow {
            start_ms: 90_000,
            end_ms: 119_000,
        }),
        "invalid-trial" => Some(TrialWindow {
            start_ms: 30_000,
            end_ms: 30_000,
        }),
        _ => None,
    }
}
fn caller() -> CallerCredential {
    CallerCredential::issue(
        &ProviderCredential::new(
            Platform::Kuwo,
            "fixture",
            "private-kuwo-media-fixture",
            None,
        )
        .unwrap(),
    )
    .unwrap()
}
fn app(
    scope: &str,
    denied: bool,
    failure: Option<ErrorCode>,
) -> (Router, Arc<Mutex<Vec<&'static str>>>) {
    let calls = Arc::default();
    let mut registry = ProviderRegistry::new();
    registry
        .register(Provider {
            account: if scope == "named" {
                "personal"
            } else {
                "default"
            },
            denied,
            failure,
            calls: Arc::clone(&calls),
        })
        .unwrap();
    (build_router(AppState::new(registry, Platform::Kuwo)), calls)
}
fn request(scope: &str, path: &str) -> Request<Body> {
    let separator = if path.contains('?') { '&' } else { '?' };
    let path = if scope == "caller" {
        path.to_owned()
    } else {
        format!(
            "{path}{separator}account={}",
            if scope == "named" {
                "personal"
            } else {
                "default"
            }
        )
    };
    let mut request = Request::builder().uri(path);
    if scope == "caller" {
        request = request.header(CALLER_CREDENTIAL_HEADER, caller().value);
    }
    request.body(Body::empty()).unwrap()
}
fn private(response: &Response) {
    assert!(
        response.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("no-store")
    );
    assert!(
        response
            .headers()
            .get(caller_scope::UPDATED_CREDENTIAL_HEADER)
            .is_none()
    );
}

#[tokio::test]
async fn kuwo_media_http_default_named_caller_single_batch_and_redirects_preserve_scope() {
    for scope in ["default", "named", "caller"] {
        for (path, operation) in [
            (
                "/v1/tracks/kuwo:67474/stream?quality=standard&fallback=false",
                "stream",
            ),
            (
                "/v1/tracks/kuwo:67474/stream/redirect?quality=standard&fallback=false",
                "stream",
            ),
            (
                "/v1/tracks/kuwo:67474/download?quality=standard",
                "download",
            ),
            (
                "/v1/tracks/kuwo:67474/download/redirect?quality=standard",
                "download",
            ),
            (
                "/v1/tracks/kuwo:67474/availability?br=128000",
                "availability",
            ),
            (
                "/v1/tracks/streams?refs=kuwo:67474,kuwo:67474&quality=standard&fallback=false",
                "stream",
            ),
        ] {
            let (app, calls) = app(scope, false, None);
            let response = app.oneshot(request(scope, path)).await.unwrap();
            private(&response);
            let redirect = path.contains("/redirect");
            assert_eq!(
                response.status(),
                if redirect {
                    StatusCode::FOUND
                } else {
                    StatusCode::OK
                },
                "{path}"
            );
            if redirect {
                assert_eq!(
                    response.headers()[header::LOCATION],
                    "https://er-sycdn.kuwo.cn/account/song.mp3"
                );
            }
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("private-kuwo-media-fixture"));
            let calls = calls.lock().unwrap();
            assert!(calls.contains(&operation));
            assert!(calls.iter().all(|c| *c == "track" || *c == operation));
            if operation == "availability" {
                assert_eq!(*calls, ["availability"]);
            }
        }
    }
}

#[tokio::test]
async fn kuwo_media_http_download_denial_never_reuses_play_authorization() {
    for scope in ["default", "named", "caller"] {
        for redirect in [false, true] {
            let (app, calls) = app(scope, true, None);
            let path = if redirect {
                "/v1/tracks/kuwo:67474/download/redirect?fallback=true"
            } else {
                "/v1/tracks/kuwo:67474/download?fallback=true"
            };
            let response = app.oneshot(request(scope, path)).await.unwrap();
            private(&response);
            assert_eq!(
                response.status(),
                if redirect {
                    StatusCode::FORBIDDEN
                } else {
                    StatusCode::OK
                }
            );
            assert!(response.headers().get(header::LOCATION).is_none());
            let value: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                    .unwrap();
            if !redirect {
                assert_eq!(value["data"]["available"], false);
                assert!(value["data"]["url"].is_null());
            }
            assert_eq!(*calls.lock().unwrap(), ["track", "download"]);
        }
    }
}

#[tokio::test]
async fn kuwo_media_http_account_failures_and_early_input_errors_are_private() {
    for scope in ["default", "named", "caller"] {
        for failure in [
            ErrorCode::AuthenticationRequired,
            ErrorCode::Conflict,
            ErrorCode::UpstreamError,
            ErrorCode::CapabilityNotSupported,
        ] {
            for path in [
                "/v1/tracks/kuwo:67474/download",
                "/v1/tracks/kuwo:67474/availability",
            ] {
                let (app, _) = app(scope, false, Some(failure));
                let response = app.oneshot(request(scope, path)).await.unwrap();
                private(&response);
                assert!(!response.status().is_success());
                let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
                assert!(!String::from_utf8_lossy(&bytes).contains("private-kuwo-media-fixture"));
            }
        }
        for path in [
            "/v1/tracks/kuwo:67474/stream?quality=invalid",
            "/v1/tracks/kuwo:67474/download?unknown=true",
            "/v1/tracks/kuwo:67474/availability?br=x",
        ] {
            let (app, calls) = app(scope, false, None);
            let response = app.oneshot(request(scope, path)).await.unwrap();
            private(&response);
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert!(calls.lock().unwrap().is_empty());
        }
    }
}

#[tokio::test]
async fn kuwo_media_http_real_provider_missing_account_never_uses_public_catalogue_or_playback() {
    let guard = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    guard.set_nonblocking(true).unwrap();
    let mut registry = ProviderRegistry::new();
    registry
        .register(
            tuneweave_provider_kuwo::KuwoProvider::new(tuneweave_provider_kuwo::KuwoConfig {
                proxy_url: Some(format!("http://{}", guard.local_addr().unwrap())),
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Kuwo));
    for scope in ["default", "named"] {
        for suffix in [
            "stream?fallback=true",
            "stream/content",
            "download/content",
            "download?fallback=true",
            "availability",
        ] {
            let response = app
                .clone()
                .oneshot(request(scope, &format!("/v1/tracks/kuwo:67474/{suffix}")))
                .await
                .unwrap();
            private(&response);
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
            assert_eq!(
                guard.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
    }
}

#[tokio::test]
async fn kuwo_media_http_account_query_is_private_before_routing_and_request_id_validation() {
    for query in [
        "account=default",
        "%61ccount=personal",
        "account=",
        "account=default&account=personal",
    ] {
        for method in [Method::GET, Method::DELETE] {
            let (app, calls) = app("default", false, None);
            let response = app
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(format!("/v1/tracks/kuwo:67474/stream?quality=bad&{query}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            private(&response);
            assert!(!response.status().is_success());
            assert!(calls.lock().unwrap().is_empty());
        }
        let (app, calls) = app("default", false, None);
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/tracks/kuwo:67474/download?{query}"))
                    .header("x-request-id", "invalid id")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        private(&response);
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn kuwo_content_http_preserves_scope_bytes_mime_and_separate_download_rights() {
    for scope in ["default", "named", "caller"] {
        for (route, operation, expected, mime, disposition) in [
            (
                "stream/content",
                "audio_content",
                b"synthetic-play-bytes".as_slice(),
                "audio/mpeg",
                "inline; filename=\"kuwo-67474.mp3\"",
            ),
            (
                "download/content",
                "download_content",
                b"synthetic-download-bytes".as_slice(),
                "audio/flac",
                "attachment; filename=\"kuwo-67474.flac\"",
            ),
        ] {
            let (router, calls) = app(scope, false, None);
            let response = router
                .oneshot(request(
                    scope,
                    &format!("/v1/tracks/kuwo:67474/{route}?quality=standard"),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            private(&response);
            assert_eq!(response.headers()[header::CONTENT_TYPE], mime);
            assert_eq!(response.headers()["x-tuneweave-audio-kind"], "full");
            assert!(
                response
                    .headers()
                    .get("x-tuneweave-trial-start-ms")
                    .is_none()
            );
            assert_eq!(response.headers()[header::CONTENT_DISPOSITION], disposition);
            assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
            assert_eq!(
                response.headers()[header::X_CONTENT_TYPE_OPTIONS],
                "nosniff"
            );
            assert_eq!(
                response.headers()[header::CONTENT_LENGTH],
                expected.len().to_string()
            );
            assert_eq!(
                to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
                expected
            );
            assert_eq!(*calls.lock().unwrap(), vec!["track", operation]);
        }
        let (router, calls) = app(scope, true, None);
        let response = router
            .oneshot(request(
                scope,
                "/v1/tracks/kuwo:67474/download/content?quality=standard",
            ))
            .await
            .unwrap();
        private(&response);
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(*calls.lock().unwrap(), vec!["track", "download_content"]);
    }
}

#[tokio::test]
async fn kuwo_content_http_rejects_routing_and_bad_inputs_before_provider_calls() {
    for scope in ["default", "named", "caller"] {
        for route in ["stream/content", "download/content"] {
            for query in [
                "fallback=false",
                "unblock=false",
                "playback_platform=kuwo",
                "fallback_platforms=kuwo",
                "source=kuwo",
                "quality=invalid",
                "quality=standard&bitrate=-1",
            ] {
                let (router, calls) = app(scope, false, None);
                let response = router
                    .oneshot(request(
                        scope,
                        &format!("/v1/tracks/kuwo:67474/{route}?{query}"),
                    ))
                    .await
                    .unwrap();
                private(&response);
                assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
                assert!(calls.lock().unwrap().is_empty());
            }
            for code in [
                ErrorCode::AuthenticationRequired,
                ErrorCode::Conflict,
                ErrorCode::UpstreamError,
                ErrorCode::UpstreamTimeout,
            ] {
                let (router, calls) = app(scope, false, Some(code));
                let response = router
                    .oneshot(request(scope, &format!("/v1/tracks/kuwo:67474/{route}")))
                    .await
                    .unwrap();
                private(&response);
                assert!(!response.status().is_success());
                assert_eq!(calls.lock().unwrap().len(), 2);
            }
        }
    }
}

#[tokio::test]
async fn kuwo_download_content_default_implementation_cannot_fall_back_to_playback() {
    struct PlaybackOnly;
    #[async_trait]
    impl MusicProvider for PlaybackOnly {
        fn platform(&self) -> Platform {
            Platform::Kuwo
        }
        fn name(&self) -> &'static str {
            "playback-only fixture"
        }
        fn capabilities(&self) -> BTreeSet<Capability> {
            BTreeSet::from([
                Capability::TrackDetail,
                Capability::AudioStream,
                Capability::AudioDownload,
            ])
        }
        async fn track(&self, id: &str, _account: Option<&str>) -> Result<Track> {
            Ok(Track::new(
                ResourceRef::new(Platform::Kuwo, id).unwrap(),
                "fixture",
            ))
        }
        async fn audio_content(&self, _t: &Track, _r: &StreamRequest) -> Result<AudioContent> {
            panic!("download must not reuse playback")
        }
        async fn download(&self, _t: &Track, _r: &StreamRequest) -> Result<MediaDownload> {
            panic!("content must not invent a URL fallback")
        }
    }
    let mut registry = ProviderRegistry::new();
    registry.register(PlaybackOnly).unwrap();
    let router = build_router(AppState::new(registry, Platform::Kuwo));
    let response = router
        .oneshot(
            Request::builder()
                .uri("/v1/tracks/kuwo:67474/download/content?account=default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    private(&response);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert_eq!(body["error"]["code"], "capability_not_supported");
}

#[tokio::test]
async fn kuwo_content_http_marks_previews_and_rejects_preview_downloads() {
    for scope in ["default", "named", "caller"] {
        for (route, id, status) in [
            ("stream/content", "trial", StatusCode::OK),
            ("download/content", "trial", StatusCode::FORBIDDEN),
            ("stream/content", "invalid-trial", StatusCode::BAD_GATEWAY),
            ("download/content", "invalid-trial", StatusCode::BAD_GATEWAY),
        ] {
            let (router, calls) = app(scope, false, None);
            let response = router
                .oneshot(request(scope, &format!("/v1/tracks/kuwo:{id}/{route}")))
                .await
                .unwrap();
            private(&response);
            assert_eq!(response.status(), status);
            if status == StatusCode::OK {
                assert_eq!(response.headers()["x-tuneweave-audio-kind"], "trial");
                assert_eq!(response.headers()["x-tuneweave-trial-start-ms"], "90000");
                assert_eq!(response.headers()["x-tuneweave-trial-end-ms"], "119000");
                assert_eq!(
                    to_bytes(response.into_body(), 1024).await.unwrap().as_ref(),
                    b"synthetic-play-bytes"
                );
            } else {
                let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
                assert!(!String::from_utf8_lossy(&body).contains("synthetic-"));
            }
            assert_eq!(calls.lock().unwrap().len(), 2);
        }
    }
}

#[tokio::test]
async fn kuwo_master_http_preserves_explicit_quality_for_play_and_download_content() {
    special_quality_http("master").await;
}

#[tokio::test]
async fn kuwo_vinyl_http_preserves_explicit_quality_for_play_and_download_content() {
    special_quality_http("vinyl").await;
}

#[tokio::test]
async fn kuwo_sing_along_http_preserves_variant_scope_and_independent_download_errors() {
    for scope in ["default", "named", "caller"] {
        for (route, operation, disposition, body) in [
            (
                "stream/content",
                "audio_content",
                "inline",
                "synthetic-play-bytes",
            ),
            (
                "download/content",
                "download_content",
                "attachment",
                "synthetic-download-bytes",
            ),
        ] {
            for denied in [false, true] {
                let (router, calls) =
                    app(scope, false, denied.then_some(ErrorCode::PermissionDenied));
                let response = router
                    .oneshot(request(
                        scope,
                        &format!("/v1/tracks/kuwo:67474/{route}?variant=sing_along&quality=high"),
                    ))
                    .await
                    .unwrap();
                private(&response);
                assert_eq!(*calls.lock().unwrap(), vec!["track", operation]);
                if denied {
                    assert_eq!(response.status(), StatusCode::FORBIDDEN);
                    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
                    let text = String::from_utf8(bytes.to_vec()).unwrap();
                    assert!(!text.contains(body));
                    assert!(!text.contains("private-kuwo-media-fixture"));
                } else {
                    assert_eq!(response.status(), StatusCode::OK);
                    assert_eq!(response.headers()[header::CONTENT_TYPE], "audio/wav");
                    assert_eq!(response.headers()["x-tuneweave-audio-kind"], "full");
                    assert_eq!(
                        response.headers()[header::CONTENT_DISPOSITION],
                        format!("{disposition}; filename=\"kuwo-67474-sing-along.wav\"")
                    );
                    assert_eq!(to_bytes(response.into_body(), 65536).await.unwrap(), body);
                }
            }
        }
    }
}

#[tokio::test]
async fn kuwo_spatial_http_preserves_explicit_quality_for_play_and_download_content() {
    special_quality_http("spatial").await;
}

async fn special_quality_http(quality: &str) {
    for scope in ["default", "named", "caller"] {
        for (route, operation, disposition) in [
            ("stream/content", "audio_content", "inline"),
            ("download/content", "download_content", "attachment"),
        ] {
            let (router, calls) = app(scope, false, None);
            let response = router
                .oneshot(request(
                    scope,
                    &format!("/v1/tracks/kuwo:67474/{route}?quality={quality}"),
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            private(&response);
            assert_eq!(response.headers()[header::CONTENT_TYPE], "audio/flac");
            assert_eq!(response.headers()["x-tuneweave-audio-kind"], "full");
            assert_eq!(
                response.headers()[header::CONTENT_DISPOSITION],
                format!("{disposition}; filename=\"kuwo-67474-{quality}.flac\"")
            );
            assert_eq!(*calls.lock().unwrap(), vec!["track", operation]);
        }
    }
}
