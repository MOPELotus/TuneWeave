//! HTTP/Uni contracts; protocol, credentials and manifest checks live in the provider tests.
use super::*;

const URL: &str = "https://freevod.nf.migu.cn/fixture/index.m3u8?playSessionId=fixture&resourceId=7&resourceType=D";
#[derive(Clone)]
struct Provider {
    caller: bool,
    failure: Option<ErrorCode>,
    calls: Arc<Mutex<Vec<String>>>,
    update: Arc<Mutex<Option<ProviderCredential>>>,
}
impl Provider {
    fn check(&self, operation: &str, account: Option<&str>) -> Result<()> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("{operation}:{}", account.unwrap_or("anonymous")));
        if self.caller {
            assert_eq!(account, Some("default"));
            *self.update.lock().unwrap() = Some(
                ProviderCredential::new(Platform::Migu, "fixture", "verified-mv-update", None)
                    .unwrap(),
            );
        }
        if operation == "stream"
            && let Some(code) = self.failure
        {
            return Err(
                TuneWeaveError::new(code, "MV authorization failed").with_platform(Platform::Migu)
            );
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
        "Migu MV playback contract"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::CallerManagedCredentials,
            Capability::VideoDetail,
            Capability::VideoStream,
        ])
    }
    fn with_caller_credential(&self, c: &ProviderCredential) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(c.secret(), "private-mv-fixture");
        Ok(Arc::new(Self {
            caller: true,
            update: Arc::default(),
            ..self.clone()
        }))
    }
    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        Ok(self.update.lock().unwrap().take())
    }
    async fn video(&self, id: &str, r: &VideoDetailRequest) -> Result<VideoDetail> {
        self.check("detail", r.account.as_deref())?;
        assert_eq!(r.kind, VideoResourceKind::Mv);
        let mut video = sample_video(id);
        video.platform = Platform::Migu;
        video.resource_ref = ResourceRef::new(Platform::Migu, id).unwrap();
        Ok(VideoDetail {
            kind: r.kind,
            video,
            resolutions: vec![],
            extensions: Extensions::new(),
        })
    }
    async fn video_stream(&self, id: &str, r: &VideoStreamRequest) -> Result<VideoStream> {
        self.check("stream", r.account.as_deref())?;
        assert_eq!(r.kind, VideoResourceKind::Mv);
        assert!(matches!(r.resolution, 0 | 1080));
        Ok(VideoStream {
            video_ref: ResourceRef::new(Platform::Migu, id).unwrap(),
            platform: Platform::Migu,
            available: true,
            url: Some(URL.into()),
            backup_urls: vec![],
            headers: BTreeMap::new(),
            expires_at: None,
            format: Some("hls".into()),
            codec: None,
            width: None,
            height: None,
            size: None,
            duration_ms: Some(215000),
            source_range: None,
            requested_resolution: r.resolution,
            actual_resolution: None,
            platform_code: Some(0),
            fee: None,
            message: None,
            extensions: Extensions::from([(
                "format_type".into(),
                json!(if r.resolution == 0 { "PQ" } else { "SQ" }),
            )]),
        })
    }
    async fn migu_native_mv_stream(
        &self,
        id: &str,
        r: &tuneweave_core::MiguNativeMvStreamRequest,
    ) -> Result<VideoStream> {
        self.check("native", r.account.as_deref())?;
        Ok(VideoStream {
            video_ref: ResourceRef::new(Platform::Migu, id).unwrap(),
            platform: Platform::Migu,
            available: true,
            url: Some(URL.into()),
            backup_urls: vec![],
            headers: BTreeMap::new(),
            expires_at: None,
            format: Some("hls".into()),
            codec: None,
            width: None,
            height: None,
            size: None,
            duration_ms: Some(212000),
            source_range: Some(tuneweave_core::VideoSourceRange {
                start_ms: 3000,
                end_ms: 215000,
            }),
            requested_resolution: 0,
            actual_resolution: None,
            platform_code: Some(0),
            fee: None,
            message: None,
            extensions: Extensions::from([("actual_format".into(), json!("PQ"))]),
        })
    }
}
fn app(failure: Option<ErrorCode>) -> (Router, Arc<Mutex<Vec<String>>>) {
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
    (build_router(AppState::new(registry, Platform::Migu)), calls)
}
fn request(path: &str, body: Option<Value>, account: Option<&str>, caller: bool) -> Request<Body> {
    let mut path = path.to_owned();
    let body = body.map(|mut v| {
        if let Some(account) = account {
            v["account"] = json!(account);
        }
        v
    });
    if body.is_none()
        && let Some(account) = account
    {
        path.push_str(&format!(
            "{}account={account}",
            if path.contains('?') { '&' } else { '?' }
        ));
    }
    let mut r = Request::builder().uri(path).method(if body.is_some() {
        Method::POST
    } else {
        Method::GET
    });
    if caller {
        let c =
            ProviderCredential::new(Platform::Migu, "fixture", "private-mv-fixture", None).unwrap();
        r = r.header(
            CALLER_CREDENTIAL_HEADER,
            CallerCredential::issue(&c).unwrap().value,
        );
    }
    if let Some(v) = body {
        r.header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(v.to_string()))
            .unwrap()
    } else {
        r.body(Body::empty()).unwrap()
    }
}
async fn inspect(response: Response, status: StatusCode, private: bool, rotation: bool) -> Value {
    assert_eq!(response.status(), status);
    if private {
        assert!(
            response.headers()[header::CACHE_CONTROL]
                .to_str()
                .unwrap()
                .contains("no-store")
        );
    }
    assert_eq!(
        response
            .headers()
            .contains_key(caller_scope::UPDATED_CREDENTIAL_HEADER),
        rotation
    );
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("private-mv-fixture"));
    if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    }
}

#[tokio::test]
async fn migu_mv_playback_http_single_batches_redirect_and_all_account_scopes() {
    for (account, caller) in [
        (None, false),
        (Some("default"), false),
        (Some("personal"), false),
        (None, true),
    ] {
        for (path, body, resolution) in [
            ("/v1/videos/migu:7/stream?resolution=auto", None, 0),
            ("/v1/videos/migu:7/stream?resolution=1080", None, 1080),
            (
                "/v1/videos/streams?refs=migu:7,migu:8,migu:7&resolution=auto",
                None,
                0,
            ),
            (
                "/v1/videos/streams",
                Some(json!({"refs":["migu:7","migu:8","migu:7"],"resolution":"auto"})),
                0,
            ),
            (
                "/v1/videos/streams",
                Some(json!({"ids":["7","8","7"],"platform":"migu","resolution":0})),
                0,
            ),
            ("/v1/videos/migu:7/stream/redirect?resolution=auto", None, 0),
        ] {
            let (router, calls) = app(None);
            let response = router
                .oneshot(request(path, body, account, caller))
                .await
                .unwrap();
            let redirect = path.contains("/redirect");
            if redirect {
                assert_eq!(response.headers()[header::LOCATION], URL);
            }
            let v = inspect(
                response,
                if redirect {
                    StatusCode::FOUND
                } else {
                    StatusCode::OK
                },
                account.is_some() || caller,
                caller,
            )
            .await;
            if !redirect {
                let items = v["data"]
                    .as_array()
                    .cloned()
                    .unwrap_or_else(|| vec![v["data"].clone()]);
                for item in &items {
                    assert_eq!(item["requested_resolution"], resolution);
                    assert!(item["actual_resolution"].is_null());
                }
                if items.len() == 3 {
                    assert_eq!(
                        items
                            .iter()
                            .map(|v| v["video_ref"].as_str().unwrap())
                            .collect::<Vec<_>>(),
                        ["migu:7", "migu:8", "migu:7"]
                    );
                }
            }
            let expected = format!(
                "stream:{}",
                if caller {
                    "default"
                } else {
                    account.unwrap_or("anonymous")
                }
            );
            assert!(calls.lock().unwrap().iter().all(|c| c == &expected));
        }
    }
}

#[tokio::test]
async fn migu_mv_playback_http_errors_suppress_invalid_updates_and_never_redirect() {
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
            "/v1/videos/migu:7/stream?resolution=auto",
            "/v1/videos/streams?refs=migu:7,migu:8&resolution=auto",
            "/v1/videos/migu:7/stream/redirect?resolution=auto",
        ] {
            let (router, calls) = app(Some(code));
            let response = router
                .oneshot(request(path, None, None, true))
                .await
                .unwrap();
            assert!(!response.headers().contains_key(header::LOCATION));
            inspect(response, status, true, rotation).await;
            assert_eq!(*calls.lock().unwrap(), ["stream:default"]);
        }
    }
}

#[tokio::test]
async fn migu_mv_playback_uni_client_and_persisted_items_use_current_playback_account() {
    for persisted in [false, true] {
        for caller in [false, true] {
            let (router, calls) = app(None);
            let source = json!({"items":[{"ref":"migu:7","kind":"mv"}],"accounts":{"migu":"source-library"}});
            let materialized = router
                .clone()
                .oneshot(request(
                    "/v1/uni/materialize/items",
                    Some(source.clone()),
                    None,
                    false,
                ))
                .await
                .unwrap();
            let item = inspect(materialized, StatusCode::OK, false, false).await["data"]["items"]
                [0]
            .clone();
            assert!(!item.to_string().contains("private-mv-fixture"));
            let r = if persisted {
                let (_, created) = json_request_from(
                    router.clone(),
                    Method::POST,
                    "/v1/uni/playlists",
                    Some(json!({"name":"MV"})),
                )
                .await;
                let reference = created["data"]["ref"].as_str().unwrap();
                let response = router
                    .clone()
                    .oneshot(request(
                        &format!("/v1/uni/playlists/{reference}/items"),
                        Some(source),
                        None,
                        false,
                    ))
                    .await
                    .unwrap();
                inspect(response, StatusCode::OK, false, false).await;
                let (_, items) = json_response_from(
                    router.clone(),
                    &format!("/v1/uni/playlists/{reference}/items"),
                )
                .await;
                let id = items["data"][0]["id"].as_str().unwrap();
                request(
                    &format!(
                        "/v1/playlists/{reference}/items/{id}/stream?resolution=auto&fallback=false&unblock=false"
                    ),
                    None,
                    (!caller).then_some("playback"),
                    caller,
                )
            } else {
                let mut body =
                    json!({"item":item,"resolution":"auto","fallback":false,"unblock":false});
                if !caller {
                    body["accounts"] = json!({"migu":"playback"});
                }
                request("/v1/uni/items/stream", Some(body), None, caller)
            };
            assert!(
                calls
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|c| c == "detail:source-library")
            );
            calls.lock().unwrap().clear();
            let result = inspect(
                router.oneshot(r).await.unwrap(),
                StatusCode::OK,
                true,
                caller,
            )
            .await;
            assert_eq!(result["data"]["extensions"]["transport"], "native_video");
            assert_eq!(result["data"]["stream"]["url"], URL);
            assert_eq!(
                *calls.lock().unwrap(),
                [if caller {
                    "stream:default"
                } else {
                    "stream:playback"
                }]
            );
        }
    }
}

#[tokio::test]
async fn migu_native_mv_http_exposes_the_source_range_without_dropping_it_into_a_redirect() {
    let (router, calls) = app(None);
    let response = router
        .oneshot(request(
            "/v1/videos/migu:7/native-stream?format=auto",
            None,
            None,
            false,
        ))
        .await
        .unwrap();
    let value = inspect(response, StatusCode::OK, true, false).await;
    assert_eq!(value["data"]["source_range"]["start_ms"], 3000);
    assert_eq!(value["data"]["source_range"]["end_ms"], 215000);
    assert_eq!(value["data"]["duration_ms"], 212000);
    assert_eq!(value["data"]["extensions"]["actual_format"], "PQ");
    assert_eq!(*calls.lock().unwrap(), ["native:anonymous"]);

    let (router, calls) = app(None);
    let response = router
        .oneshot(request(
            "/v1/videos/migu:7/native-stream?format=bogus",
            None,
            None,
            false,
        ))
        .await
        .unwrap();
    let _ = inspect(response, StatusCode::BAD_REQUEST, false, false).await;
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn migu_native_mv_http_supports_selected_and_caller_credentials_without_caching() {
    for (account, caller, rotation) in [(Some("default"), false, false), (None, true, true)] {
        let (router, calls) = app(None);
        let response = router
            .oneshot(request(
                "/v1/videos/migu:7/native-stream?format=pq",
                None,
                account,
                caller,
            ))
            .await
            .unwrap();
        let value = inspect(response, StatusCode::OK, true, rotation).await;
        assert_eq!(value["data"]["source_range"]["start_ms"], 3000);
        assert_eq!(*calls.lock().unwrap(), ["native:default"]);
    }
}

#[test]
fn migu_mv_playback_auto_resolution_is_platform_specific() {
    for platform in [
        Platform::Qq,
        Platform::Kugou,
        Platform::Kuwo,
        Platform::Bilibili,
    ] {
        for value in ["auto", "0"] {
            assert!(parse_video_stream_resolution(platform, Some(value)).is_err());
        }
        assert_eq!(parse_video_stream_resolution(platform, None).unwrap(), 1080);
    }
    for value in ["auto", "0"] {
        assert_eq!(
            parse_video_stream_resolution(Platform::Migu, Some(value)).unwrap(),
            0
        );
    }
}
