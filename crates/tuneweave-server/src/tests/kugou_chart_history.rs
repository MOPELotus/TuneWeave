use super::*;
use tuneweave_core::{ChartPeriod, ChartPeriodSummary};

struct PeriodProvider;
#[async_trait]
impl MusicProvider for PeriodProvider {
    fn platform(&self) -> Platform {
        Platform::Kugou
    }
    fn name(&self) -> &'static str {
        "period HTTP fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::ChartPeriods,
            Capability::ChartHistoricalTracks,
            Capability::ChartTracks,
        ])
    }
    async fn chart_periods(&self, id: &str, r: &PageRequest) -> Result<Page<ChartPeriodSummary>> {
        assert_eq!(id, "chart:42");
        assert_eq!((r.limit, r.offset), (2, 2));
        assert!(r.account.is_none());
        Ok(Page {
            items: vec![ChartPeriodSummary {
                period: ChartPeriod::Id { id: "6".into() },
                name: "259期".into(),
                year: Some(2026),
                is_current: Some(false),
                extensions: Extensions::new(),
            }],
            pagination: PageMeta {
                limit: 2,
                offset: 2,
                total: Some(3),
                has_more: false,
                next_offset: None,
                extensions: Extensions::new(),
            },
        })
    }
    async fn chart_tracks(&self, id: &str, r: &ChartTrackListRequest) -> Result<Page<Track>> {
        assert_eq!(id, "chart:42");
        assert_eq!(r.period, ChartPeriod::Id { id: "6".into() });
        assert_eq!((r.limit, r.offset), (2, 2));
        assert!(!r.include_tags);
        Ok(Page {
            items: vec![Track::new(
                ResourceRef::new(Platform::Kugou, "1000").unwrap(),
                "Historical song",
            )],
            pagination: PageMeta {
                limit: 2,
                offset: 2,
                total: Some(3),
                has_more: false,
                next_offset: None,
                extensions: Extensions::from([("requested_period".into(), json!(r.period))]),
            },
        })
    }
}
#[tokio::test]
async fn chart_history_period_discovery_and_selection_use_existing_pagination_and_strict_id_kind() {
    let mut registry = ProviderRegistry::new();
    registry.register(PeriodProvider).unwrap();
    let app = build_router(AppState::new(registry, Platform::Kugou));
    for query in ["limit=2&offset=2", "num=2&page=2"] {
        let (status, v) = json_response_from(
            app.clone(),
            &format!("/v1/charts/kugou:chart:42/periods?{query}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["data"][0]["period"], json!({"kind":"id","id":"6"}));
        assert_eq!(v["meta"]["pagination"]["total"], 3);
    }
    let (status, v) = json_response_from(
        app,
        "/v1/charts/kugou:chart:42/tracks?period_kind=id&period_id=6&num=2&page=2&tag=false",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(
        v["meta"]["pagination"]["extensions"]["requested_period"]["id"],
        "6"
    );
}

#[tokio::test]
async fn chart_history_real_kugou_provider_rejects_bad_period_queries_and_accounts_before_network()
{
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let p = tuneweave_provider_kugou::KugouProvider::new(tuneweave_provider_kugou::KugouConfig {
        proxy_url: Some(format!("http://{}", listener.local_addr().unwrap())),
        ..Default::default()
    })
    .unwrap();
    assert!(p.supports(Capability::ChartPeriods));
    assert!(p.supports(Capability::ChartHistoricalTracks));
    let mut registry = ProviderRegistry::new();
    registry.register(p).unwrap();
    let app = build_router(AppState::new(registry, Platform::Kugou));
    for (path, query) in [
        ("periods", "limit=0"),
        ("periods", "limit=101"),
        ("periods", "page=0"),
        ("periods", "offset=1&page=1"),
        ("periods", "period_id=6"),
        ("periods", "num=2&limit=2"),
        ("periods", "offset=4294967295"),
        ("periods", ""),
        ("tracks", "period_id=6"),
        ("tracks", "period_kind=id"),
        ("tracks", "period_kind=id&period_id="),
        (
            "tracks",
            "period_kind=id&period_id=6&period_date=2026-01-01",
        ),
        ("tracks", "period_kind=current&period_id=6"),
        (
            "tracks",
            "period_kind=day&period_id=6&period_date=2026-01-01",
        ),
        ("tracks", "period_kind=id&period_id=6&period_id=7"),
        ("tracks", "period_kind=id&period_id=06"),
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/charts/kugou:chart:42/{path}?{query}&account=default"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path} {query}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn chart_history_other_platforms_reject_id_and_period_discovery_before_io() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let proxy = Some(format!("http://{}", listener.local_addr().unwrap()));
    let mut registry = ProviderRegistry::new();
    registry
        .register(
            tuneweave_provider_migu::MiguProvider::new(tuneweave_provider_migu::MiguConfig {
                proxy_url: proxy.clone(),
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
    registry
        .register(
            tuneweave_provider_qq::QqProvider::new(tuneweave_provider_qq::QqConfig {
                proxy_url: proxy.clone(),
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
    registry
        .register(
            tuneweave_provider_kuwo::KuwoProvider::new(tuneweave_provider_kuwo::KuwoConfig {
                proxy_url: proxy,
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Migu));
    for platform in ["migu", "qq", "kuwo"] {
        for suffix in ["periods", "tracks?period_kind=id&period_id=6"] {
            let (status, v) = json_response_from(
                app.clone(),
                &format!("/v1/charts/{platform}:chart:42/{suffix}"),
            )
            .await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
            assert_eq!(v["error"]["code"], "capability_not_supported");
        }
    }
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
