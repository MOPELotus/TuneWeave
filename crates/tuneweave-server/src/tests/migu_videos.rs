//! Routing contracts; real Migu protocol mapping has separate loopback and opt-in live tests.
use super::*;
struct Provider;
fn video(id: &str) -> Video {
    let mut v = sample_video(id);
    v.platform = Platform::Migu;
    v.resource_ref = ResourceRef::new(Platform::Migu, id).unwrap();
    v.duration_ms = Some(259000);
    v.subscribed = None;
    v.extensions = Extensions::from([("resource_type".into(), json!("D"))]);
    v
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Migu
    }
    fn name(&self) -> &'static str {
        "Migu MV HTTP contract"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::SearchMvs,
            Capability::ArtistVideos,
            Capability::VideoDetail,
            Capability::VideoStats,
        ])
    }
    async fn video(&self, id: &str, r: &VideoDetailRequest) -> Result<VideoDetail> {
        assert_eq!(r.kind, VideoResourceKind::Mv);
        assert!(r.account.is_none());
        Ok(VideoDetail {
            kind: r.kind,
            video: video(id),
            resolutions: vec![],
            extensions: Extensions::new(),
        })
    }
    async fn video_stats(&self, id: &str, r: &VideoDetailRequest) -> Result<VideoStats> {
        assert_eq!(r.kind, VideoResourceKind::Mv);
        assert!(r.account.is_none());
        Ok(VideoStats {
            video_ref: ResourceRef::new(Platform::Migu, id).unwrap(),
            kind: r.kind,
            liked: None,
            favorited: None,
            coins_contributed: None,
            view_count: Some(283133),
            danmaku_count: None,
            like_count: Some(68),
            coin_count: None,
            favorite_count: Some(228),
            comment_count: Some(192),
            share_count: Some(1850),
            extensions: Extensions::new(),
        })
    }
    async fn artist_videos(&self, id: &str, r: &ArtistVideoListRequest) -> Result<Page<Video>> {
        assert_eq!(id, "112");
        assert_eq!(r.kind, VideoKind::Mv);
        assert_eq!((r.limit, r.offset), (2, 8));
        assert_eq!(r.order.as_deref(), Some("platform_default"));
        assert!(r.account.is_none());
        Ok(Page {
            items: vec![video("9"), video("10")],
            pagination: PageMeta {
                limit: r.limit,
                offset: r.offset,
                total: None,
                has_more: true,
                next_offset: Some(10),
                extensions: Extensions::new(),
            },
        })
    }
    async fn search_catalog(&self, q: &SearchQuery) -> Result<Page<SearchItem>> {
        assert_eq!(q.kind, SearchKind::Mv);
        assert!(q.account.is_none());
        assert_eq!((q.limit, q.offset), (2, 18));
        assert_eq!(
            q.video_filters.as_ref().unwrap().order,
            tuneweave_core::VideoSearchOrder::Newest
        );
        Ok(Page {
            items: vec![
                SearchItem::Video(video("19")),
                SearchItem::Video(video("20")),
            ],
            pagination: PageMeta {
                limit: q.limit,
                offset: q.offset,
                total: None,
                has_more: true,
                next_offset: Some(20),
                extensions: Extensions::new(),
            },
        })
    }
}
fn app(real: bool) -> Router {
    let mut registry = ProviderRegistry::new();
    if real {
        registry
            .register(
                tuneweave_provider_migu::MiguProvider::new(tuneweave_provider_migu::MiguConfig {
                    proxy_url: Some("http://127.0.0.1:9".into()),
                    ..Default::default()
                })
                .unwrap(),
            )
            .unwrap();
    } else {
        registry.register(Provider).unwrap();
    }
    build_router(AppState::new(registry, Platform::Migu))
}
#[tokio::test]
async fn migu_mv_http_preserves_kinds_stats_search_filters_and_batch_order() {
    let (status, v) = json_response_from(app(false), "/v1/videos/migu:7?type=mv").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["video"]["ref"], "migu:7");
    assert_eq!(v["data"]["video"]["duration_ms"], 259000);
    assert_eq!(v["data"]["resolutions"], json!([]));
    let (status, v) = json_response_from(app(false), "/v1/videos/migu:7/stats?type=mv").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["data"]["view_count"], 283133);
    assert!(v["data"]["liked"].is_null());
    let (status, v) = json_response_from(
        app(false),
        "/v1/artists/migu:112/videos?type=mv&order=platform_default&limit=2&offset=8",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["data"][0]["ref"], "migu:9");
    assert_eq!(v["meta"]["pagination"]["next_offset"], 10);
    let (status, v) = json_response_from(
        app(false),
        "/v1/search?platform=migu&type=mv&q=test&order=newest&limit=2&offset=18",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["data"].as_array().unwrap().len(), 2);
    for (method, path, body) in [
        (
            Method::GET,
            "/v1/videos/details?refs=migu:7,migu:8,migu:7&type=mv",
            None,
        ),
        (
            Method::POST,
            "/v1/videos/details",
            Some(json!({"platform":"migu","ids":["7","8","7"],"type":"mv"})),
        ),
    ] {
        let (status, v) = json_request_from(app(false), method, path, body).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(
            v["data"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["video"]["ref"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["migu:7", "migu:8", "migu:7"]
        );
    }
}
#[tokio::test]
async fn migu_mv_http_real_provider_rejects_unsupported_scope_and_filters_before_network() {
    let (status, value) = json_response_from(app(true), "/v1/videos/migu:7?account=A").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(value["error"]["code"], "authentication_required");
    for platform in [
        Platform::Netease,
        Platform::Qq,
        Platform::Kugou,
        Platform::Kuwo,
    ] {
        assert!(
            parse_video_search_filters(platform, SearchKind::Mv, Some("newest"), None, None)
                .is_err()
        );
    }
    for path in [
        "/v1/videos/migu:7?type=video",
        "/v1/videos/migu:07/stats?type=mv",
        "/v1/artists/migu:112/videos?type=all",
        "/v1/artists/migu:112/videos?type=mv&order=hot",
        "/v1/artists/migu:112/videos?type=mv&account=A",
        "/v1/search?platform=migu&type=mv&q=test&account=A",
        "/v1/search?platform=migu&type=mv&q=test&limit=0",
    ] {
        let (status, v) = json_response_from(app(true), path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{path}: {v}");
        assert_eq!(v["error"]["code"], "invalid_request");
    }
}
