//! HTTP contracts for Kuwo MV metadata and playback; real upstream media stays opt-in.
use super::*;

const URL: &str = "https://kw-bj.kuwo.cn/0123456789abcdef0123456789abcdef/1234abcd/ll/resource/m3/24/54/1234567890.mp4";

struct Provider;

fn video(id: &str) -> Video {
    let mut video = sample_video(id);
    video.platform = Platform::Kuwo;
    video.resource_ref = ResourceRef::new(Platform::Kuwo, id).unwrap();
    video.duration_ms = Some(214_000);
    video.subscribed = None;
    video.extensions = Extensions::from([
        ("kind".into(), json!("mv")),
        ("source_track_id".into(), json!(id)),
    ]);
    video
}

#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Kuwo
    }

    fn name(&self) -> &'static str {
        "Kuwo MV playback HTTP contract"
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::VideoDetail, Capability::VideoStream])
    }

    async fn video(&self, id: &str, request: &VideoDetailRequest) -> Result<VideoDetail> {
        if request.kind != VideoResourceKind::Mv
            || request.account.is_some()
            || !id
                .parse::<u64>()
                .is_ok_and(|value| value > 0 && value.to_string() == id)
        {
            return Err(TuneWeaveError::invalid_request(
                "Kuwo MV fixture received an invalid reference or account",
            ));
        }
        Ok(VideoDetail {
            kind: request.kind,
            video: video(id),
            resolutions: vec![],
            extensions: Extensions::new(),
        })
    }

    async fn video_stream(&self, id: &str, request: &VideoStreamRequest) -> Result<VideoStream> {
        let detail = self
            .video(
                id,
                &VideoDetailRequest {
                    kind: request.kind,
                    account: request.account.clone(),
                },
            )
            .await?;
        if !(1..=4320).contains(&request.resolution) {
            return Err(TuneWeaveError::invalid_request(
                "Kuwo MV fixture received an invalid resolution",
            ));
        }
        Ok(VideoStream {
            video_ref: detail.video.resource_ref,
            platform: Platform::Kuwo,
            available: true,
            url: Some(URL.into()),
            backup_urls: vec![],
            headers: BTreeMap::new(),
            expires_at: None,
            format: Some("mp4".into()),
            codec: None,
            width: None,
            height: None,
            size: None,
            duration_ms: Some(214_000),
            source_range: None,
            requested_resolution: request.resolution,
            actual_resolution: None,
            platform_code: Some(200),
            fee: None,
            message: None,
            extensions: Extensions::from([("backend".into(), json!("current_web_mv_play_url"))]),
        })
    }
}

fn app() -> Router {
    let mut registry = ProviderRegistry::new();
    registry.register(Provider).unwrap();
    build_router(AppState::new(registry, Platform::Kuwo))
}

#[tokio::test]
async fn kuwo_mv_http_preserves_metadata_identity_and_stream_batch_order() {
    let (status, value) = json_response_from(app(), "/v1/videos/kuwo:215252").await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["data"]["kind"], "mv");
    assert_eq!(value["data"]["video"]["ref"], "kuwo:215252");
    assert_eq!(value["data"]["video"]["duration_ms"], 214_000);

    let (status, value) = json_response_from(
        app(),
        "/v1/videos/streams?refs=kuwo:215252,kuwo:550531865,kuwo:215252&resolution=4320",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    let rows = value["data"].as_array().unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(
        rows.iter()
            .map(|row| row["video_ref"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["kuwo:215252", "kuwo:550531865", "kuwo:215252"]
    );
    assert!(rows.iter().all(|row| {
        row["requested_resolution"] == 4320
            && row["actual_resolution"].is_null()
            && row["format"] == "mp4"
    }));

    let (status, value) = json_request_from(
        app(),
        Method::POST,
        "/v1/videos/streams",
        Some(json!({
            "platform": "kuwo",
            "ids": ["215252", "550531865"],
            "kind": "mv",
            "resolution": 1080
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{value}");
    assert_eq!(value["data"].as_array().unwrap().len(), 2);
    assert!(
        value["data"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["requested_resolution"] == 1080)
    );
}

#[tokio::test]
async fn kuwo_mv_http_rejects_non_public_inputs_before_provider_and_keeps_account_errors_private() {
    for path in [
        "/v1/videos/kuwo:215252?kind=video",
        "/v1/videos/kuwo:215252/stream?resolution=auto",
        "/v1/videos/streams?refs=kuwo:215252,kuwo:550531865&platform=migu",
        "/v1/videos/streams?refs=kuwo:215252&unknown=true",
    ] {
        let (status, value) = json_response_from(app(), path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {value}");
        assert_eq!(value["error"]["code"], "invalid_request");
    }

    let response = app()
        .oneshot(
            Request::builder()
                .uri("/v1/videos/kuwo:215252/stream?resolution=1080&account=named")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("named"));
}

#[tokio::test]
async fn kuwo_mv_http_redirect_preserves_the_exact_authorized_url_and_private_cache_policy() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/v1/videos/kuwo:215252/stream/redirect?resolution=480")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(response.headers()[header::LOCATION], URL);
    assert_eq!(
        response.headers()[header::CACHE_CONTROL],
        "private, no-store"
    );
}
