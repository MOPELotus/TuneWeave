use super::*;

struct VideoProvider;
#[async_trait]
impl MusicProvider for VideoProvider {
    fn platform(&self) -> Platform {
        Platform::Kugou
    }
    fn name(&self) -> &'static str {
        "KuGou video HTTP fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::VideoDetail, Capability::VideoStream])
    }
    async fn video(&self, id: &str, request: &VideoDetailRequest) -> Result<VideoDetail> {
        let prefix = if request.kind == VideoResourceKind::Mv {
            "mv:"
        } else {
            "video:"
        };
        let numeric = id.strip_prefix(prefix).unwrap_or(id);
        if !numeric
            .parse::<u64>()
            .is_ok_and(|n| n > 0 && n.to_string() == numeric)
            || request.account.is_some()
        {
            return Err(TuneWeaveError::invalid_request(
                "Invalid fixture video reference or account",
            ));
        }
        let mut video = sample_video(id);
        video.platform = Platform::Kugou;
        video.resource_ref = ResourceRef::new(Platform::Kugou, id).unwrap();
        Ok(VideoDetail {
            kind: request.kind,
            video,
            resolutions: vec![],
            extensions: Extensions::new(),
        })
    }

    async fn video_stream(&self, id: &str, request: &VideoStreamRequest) -> Result<VideoStream> {
        let d = self
            .video(
                id,
                &VideoDetailRequest {
                    kind: request.kind,
                    account: request.account.clone(),
                },
            )
            .await?;
        Ok(VideoStream {
            video_ref: d.video.resource_ref,
            platform: Platform::Kugou,
            available: true,
            url: Some("https://mvwebfs.tx.kugou.com/fixture?auth=test%2Bvalue".into()),
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
}
fn app() -> Router {
    let mut registry = ProviderRegistry::new();
    registry.register(VideoProvider).unwrap();
    build_router(AppState::new(registry, Platform::Kugou))
}

#[tokio::test]
async fn kugou_video_http_infers_namespaced_kinds_and_keeps_explicit_kind_validation() {
    for (id, kind) in [("mv:17", "mv"), ("video:17", "video"), ("17", "mv")] {
        let (status, value) = json_response_from(app(), &format!("/v1/videos/kugou:{id}")).await;
        assert_eq!(status, StatusCode::OK, "{id} {value}");
        assert_eq!(value["data"]["kind"], kind);
        assert_eq!(value["data"]["video"]["ref"], format!("kugou:{id}"));
    }
    for path in [
        "/v1/videos/kugou:mv:17?kind=video",
        "/v1/videos/kugou:video:17?kind=mv",
        "/v1/videos/kugou:mv:17?account=named",
    ] {
        let (status, value) = json_response_from(app(), path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
        assert_eq!(value["error"]["code"], "invalid_request");
    }
}

#[tokio::test]
async fn kugou_video_http_batches_preserve_prefixed_and_bare_references_and_duplicate_positions() {
    for (method, path, body, refs) in [
        (
            Method::GET,
            "/v1/videos/details?refs=kugou:mv:17,kugou:17,kugou:mv:17",
            None,
            vec!["kugou:mv:17", "kugou:17", "kugou:mv:17"],
        ),
        (
            Method::POST,
            "/v1/videos/details",
            Some(json!({"platform":"kugou","ids":["video:17","17","video:17"],"kind":"video"})),
            vec!["kugou:video:17", "kugou:17", "kugou:video:17"],
        ),
    ] {
        let (status, value) = json_request_from(app(), method, path, body).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        let rows = value["data"].as_array().unwrap();
        assert_eq!(rows.len(), refs.len());
        for (row, reference) in rows.iter().zip(refs) {
            assert_eq!(row["video"]["ref"], reference);
        }
    }
    let (status, _) =
        json_response_from(app(), "/v1/videos/details?refs=kugou:mv:17,kugou:video:17").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn kugou_video_stream_http_preserves_resolution_refs_and_validates_kind_and_account() {
    for (method, path, body, expected) in [
        (
            Method::GET,
            "/v1/videos/streams?refs=kugou:mv:17,kugou:17,kugou:mv:17&resolution=480",
            None,
            vec!["kugou:mv:17", "kugou:17", "kugou:mv:17"],
        ),
        (
            Method::POST,
            "/v1/videos/streams",
            Some(
                json!({"platform":"kugou","ids":["video:17","17","video:17"],"kind":"video","resolution":480}),
            ),
            vec!["kugou:video:17", "kugou:17", "kugou:video:17"],
        ),
    ] {
        let (status, value) = json_request_from(app(), method, path, body).await;
        assert_eq!(status, StatusCode::OK, "{value}");
        let rows = value["data"].as_array().unwrap();
        assert_eq!(rows.len(), expected.len());
        for (row, reference) in rows.iter().zip(expected) {
            assert_eq!(row["video_ref"], reference);
            assert_eq!(row["requested_resolution"], 480);
            assert_eq!(row["actual_resolution"], 432);
        }
    }
    for id in ["mv:17", "video:17", "17"] {
        let (status, value) = json_response_from(
            app(),
            &format!("/v1/videos/kugou:{id}/stream?resolution=480"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{value}");
        assert_eq!(value["data"]["video_ref"], format!("kugou:{id}"));
    }
    for path in [
        "/v1/videos/kugou:mv:17/stream?kind=video",
        "/v1/videos/kugou:mv:17/stream?account=named",
        "/v1/videos/kugou:17/stream?resolution=0",
        "/v1/videos/streams?refs=kugou:mv:17,kugou:video:17",
    ] {
        let (status, _) = json_response_from(app(), path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}");
    }
}

#[tokio::test]
async fn kugou_video_stream_redirect_uses_the_same_exact_authorized_https_url() {
    let response = app()
        .oneshot(
            Request::builder()
                .uri("/v1/videos/kugou:mv:17/stream/redirect?resolution=480")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FOUND);
    assert_eq!(
        response.headers()[header::LOCATION],
        "https://mvwebfs.tx.kugou.com/fixture?auth=test%2Bvalue"
    );
}
