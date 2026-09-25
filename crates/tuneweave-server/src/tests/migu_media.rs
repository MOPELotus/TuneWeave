//! HTTP/Uni contract fixtures. Native PC protocol and source races are tested in Migu's crate.
use super::*;

#[derive(Clone)]
struct Provider {
    account: &'static str,
    caller: bool,
    download_ready: bool,
    failure: Option<ErrorCode>,
    calls: Arc<Mutex<Vec<String>>>,
    update: Arc<Mutex<Option<ProviderCredential>>>,
}
impl Provider {
    fn check(&self, operation: &str, account: Option<&str>) -> Result<()> {
        assert!(
            account == Some(self.account)
                || (operation == "track" && account == Some("source-library"))
        );
        self.calls
            .lock()
            .unwrap()
            .push(format!("{operation}:{}", account.unwrap()));
        if self.caller {
            *self.update.lock().unwrap() = Some(
                ProviderCredential::new(Platform::Migu, "fixture", "verified-media-update", None)
                    .unwrap(),
            );
        }
        if operation != "track"
            && let Some(code) = self.failure
        {
            return Err(TuneWeaveError::new(code, "Account media unavailable")
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
        "Migu PC account media contract"
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
    fn with_caller_credential(
        &self,
        credential: &ProviderCredential,
    ) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(credential.secret(), "private-media-fixture");
        Ok(Arc::new(Self {
            account: "default",
            caller: true,
            update: Arc::default(),
            ..self.clone()
        }))
    }
    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        Ok(self.update.lock().unwrap().take())
    }
    fn requires_download_authorization(&self, _: Option<&str>) -> bool {
        true
    }
    async fn track(&self, id: &str, account: Option<&str>) -> Result<Track> {
        self.check("track", account)?;
        Ok(Track::new(
            ResourceRef::new(Platform::Migu, id).unwrap(),
            "Synthetic song",
        ))
    }
    async fn stream(&self, track: &Track, request: &StreamRequest) -> Result<MediaStream> {
        self.check("stream", request.account.as_deref())?;
        Ok(MediaStream {url:"https://freetyst.nf.migu.cn/public/product9th/product44/fixture.mp3?Tim=1&Key=synthetic&playSessionId=fixture".into(),backup_urls:vec![],headers:BTreeMap::new(),expires_at:None,format:Some("mp3".into()),codec:Some("mp3".into()),bitrate:Some(128000),size:None,duration_ms:Some(212000),requested_quality:request.quality,actual_quality:Quality::Standard,trial:None,origin_track:Some(track.resource_ref.clone()),resolved_track:track.resource_ref.clone(),resolved_platform:Platform::Migu,match_score:Some(1.0),attempts:vec![]})
    }
    async fn download(&self, track: &Track, request: &StreamRequest) -> Result<MediaDownload> {
        self.check("download", request.account.as_deref())?;
        if self.download_ready {
            assert_eq!(request.quality, Quality::High);
            return Ok(MediaDownload {
                track_ref: track.resource_ref.clone(),
                platform: Platform::Migu,
                available: true,
                url: Some("https://dlsdownfree.nf.migu.cn/wlansst/song?pars=synthetic".into()),
                headers: BTreeMap::new(),
                expires_at: None,
                format: Some("mp3".into()),
                codec: Some("mp3".into()),
                bitrate: Some(320000),
                size: Some(3450883),
                duration_ms: None,
                requested_quality: request.quality,
                actual_quality: Quality::High,
                platform_code: Some(0),
                fee: None,
                message: None,
                extensions: Extensions::new(),
            });
        }
        Err(TuneWeaveError::unsupported(
            Platform::Migu,
            Capability::AudioDownload,
        ))
    }
    async fn track_availability(
        &self,
        id: &str,
        request: &TrackAvailabilityRequest,
    ) -> Result<TrackAvailability> {
        self.check("availability", request.account.as_deref())?;
        Ok(TrackAvailability {
            track_ref: ResourceRef::new(Platform::Migu, id).unwrap(),
            playable: true,
            requested_bitrate: request.bitrate,
            actual_bitrate: Some(128000),
            platform_code: Some(0),
            message: "ok".into(),
            extensions: Extensions::new(),
        })
    }
}
fn app(scope: &str, failure: Option<ErrorCode>) -> (Router, Arc<Mutex<Vec<String>>>) {
    app_with_download(scope, failure, false)
}

fn app_with_download(
    scope: &str,
    failure: Option<ErrorCode>,
    download_ready: bool,
) -> (Router, Arc<Mutex<Vec<String>>>) {
    let calls = Arc::default();
    let mut registry = ProviderRegistry::new();
    registry
        .register(Provider {
            account: if scope == "named" {
                "personal"
            } else {
                "default"
            },
            caller: false,
            download_ready,
            failure,
            calls: Arc::clone(&calls),
            update: Arc::default(),
        })
        .unwrap();
    (build_router(AppState::new(registry, Platform::Migu)), calls)
}
fn caller() -> String {
    CallerCredential::issue(
        &ProviderCredential::new(Platform::Migu, "fixture", "private-media-fixture", None).unwrap(),
    )
    .unwrap()
    .value
}
fn request(scope: &str, path: &str) -> Request<Body> {
    let path = if scope == "caller" {
        path.into()
    } else {
        format!(
            "{path}{}account={}",
            if path.contains('?') { '&' } else { '?' },
            if scope == "named" {
                "personal"
            } else {
                "default"
            }
        )
    };
    let mut request = Request::builder().uri(path);
    if scope == "caller" {
        request = request.header(CALLER_CREDENTIAL_HEADER, caller());
    }
    request.body(Body::empty()).unwrap()
}
async fn inspect(response: Response, status: StatusCode, rotation: bool) -> Value {
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
        rotation
    );
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("private-media-fixture"));
    if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    }
}

#[tokio::test]
async fn migu_media_http_default_named_caller_scope_rotation_and_download_separation() {
    for scope in ["default", "named", "caller"] {
        for path in [
            "/v1/tracks/migu:123/stream?quality=standard&fallback=false",
            "/v1/tracks/migu:123/stream/redirect?fallback=false",
            "/v1/tracks/migu:123/availability?br=128000",
            "/v1/tracks/streams?refs=migu:123,migu:123&quality=standard&fallback=false",
        ] {
            let (app, calls) = app(scope, None);
            let response = app.oneshot(request(scope, path)).await.unwrap();
            let redirect = path.contains("/redirect");
            if redirect {
                assert!(
                    response.headers()[header::LOCATION]
                        .to_str()
                        .unwrap()
                        .contains("freetyst.nf.migu.cn")
                );
            }
            let body = inspect(
                response,
                if redirect {
                    StatusCode::FOUND
                } else {
                    StatusCode::OK
                },
                scope == "caller",
            )
            .await;
            if path.contains("availability") {
                assert_eq!(body["data"]["actual_bitrate"], 128000);
            }
            assert!(
                !calls
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|v| v.starts_with("download"))
            );
        }
        for redirect in [false, true] {
            let (app, calls) = app(scope, None);
            let path = format!(
                "/v1/tracks/migu:123/download{}?fallback=true",
                if redirect { "/redirect" } else { "" }
            );
            let response = app.oneshot(request(scope, &path)).await.unwrap();
            assert!(!response.headers().contains_key(header::LOCATION));
            inspect(
                response,
                StatusCode::UNPROCESSABLE_ENTITY,
                scope == "caller",
            )
            .await;
            let calls = calls.lock().unwrap();
            assert!(calls.iter().any(|v| v.starts_with("download:")));
            assert!(!calls.iter().any(|v| v.starts_with("stream:")));
        }
    }
}

#[tokio::test]
async fn migu_native_download_http_preserves_ownership_and_never_substitutes_play_authorization() {
    for scope in ["default", "named", "caller"] {
        for redirect in [false, true] {
            for denied in [false, true] {
                let (router, calls) =
                    app_with_download(scope, denied.then_some(ErrorCode::PermissionDenied), true);
                let path = format!(
                    "/v1/tracks/migu:123/download{}?quality=high&fallback=true",
                    if redirect { "/redirect" } else { "" }
                );
                let response = router.oneshot(request(scope, &path)).await.unwrap();
                let status = if denied {
                    StatusCode::FORBIDDEN
                } else if redirect {
                    StatusCode::FOUND
                } else {
                    StatusCode::OK
                };
                if redirect && !denied {
                    assert_eq!(
                        response.headers()[header::LOCATION],
                        "https://dlsdownfree.nf.migu.cn/wlansst/song?pars=synthetic"
                    );
                    assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
                } else {
                    assert!(!response.headers().contains_key(header::LOCATION));
                }
                let body = inspect(response, status, scope == "caller").await;
                if !denied && !redirect {
                    assert_eq!(body["data"]["available"], true);
                    assert_eq!(body["data"]["actual_quality"], "high");
                    assert_eq!(body["data"]["bitrate"], 320000);
                    assert_eq!(body["data"]["size"], 3450883);
                    assert!(body["data"]["expires_at"].is_null());
                }
                let calls = calls.lock().unwrap();
                assert_eq!(
                    calls
                        .iter()
                        .filter(|call| call.starts_with("download:"))
                        .count(),
                    1
                );
                assert!(!calls.iter().any(|call| call.starts_with("stream:")));
            }
        }
    }
}

#[tokio::test]
async fn migu_media_http_preserves_errors_and_suppresses_invalidated_caller_updates() {
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
        for path in [
            "/v1/tracks/migu:123/stream?fallback=false",
            "/v1/tracks/migu:123/availability?br=128000",
        ] {
            let (app, _) = app("caller", Some(code));
            let response = app.oneshot(request("caller", path)).await.unwrap();
            assert!(!response.headers().contains_key(header::LOCATION));
            inspect(response, status, rotation).await;
        }
    }
}

#[tokio::test]
async fn migu_media_http_real_provider_missing_account_stops_before_any_public_request() {
    let guard = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    guard.set_nonblocking(true).unwrap();
    let mut registry = ProviderRegistry::new();
    registry
        .register(
            tuneweave_provider_migu::MiguProvider::new(tuneweave_provider_migu::MiguConfig {
                proxy_url: Some(format!("http://{}", guard.local_addr().unwrap())),
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Migu));
    for scope in ["default", "named"] {
        for path in [
            "/v1/tracks/migu:123",
            "/v1/tracks/migu:123/stream?fallback=false",
            "/v1/tracks/migu:123/download?fallback=true",
            "/v1/tracks/migu:123/availability",
            "/v1/search?platform=migu&kind=track&q=Song",
        ] {
            inspect(
                app.clone().oneshot(request(scope, path)).await.unwrap(),
                StatusCode::UNAUTHORIZED,
                false,
            )
            .await;
            assert_eq!(
                guard.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
    }
}

#[tokio::test]
async fn migu_media_uni_materialization_does_not_reuse_source_accounts_for_playback() {
    for playback_caller in [false, true] {
        let (router, calls) = app("named", None);
        let (status,body)=json_request_from(router.clone(),Method::POST,"/v1/uni/materialize/items",Some(json!({"items":[{"ref":"migu:123","kind":"track"}],"accounts":{"migu":"source-library"}}))).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let item = body["data"]["items"][0].clone();
        assert!(!item.to_string().contains("source-library"));
        assert!(!item.to_string().contains("private-media-fixture"));
        calls.lock().unwrap().clear();
        let mut body = json!({"item":item,"quality":"standard","fallback":false});
        if !playback_caller {
            body["accounts"] = json!({"migu":"personal"});
        }
        let mut request = Request::builder()
            .method(Method::POST)
            .uri("/v1/uni/items/stream")
            .header(header::CONTENT_TYPE, "application/json");
        if playback_caller {
            request = request.header(CALLER_CREDENTIAL_HEADER, caller());
        }
        let response = router
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let body = inspect(response, StatusCode::OK, playback_caller).await;
        assert_eq!(body["data"]["source_ref"], "migu:123");
        assert_eq!(body["data"]["stream"]["resolved_platform"], "migu");
        assert_eq!(
            *calls.lock().unwrap(),
            vec![format!(
                "stream:{}",
                if playback_caller {
                    "default"
                } else {
                    "personal"
                }
            )]
        );
    }
}
