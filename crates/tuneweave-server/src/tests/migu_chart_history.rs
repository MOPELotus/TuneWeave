use super::*;
use tuneweave_core::ChartPeriod;

struct HistoryProvider {
    expected: ChartPeriod,
}
#[async_trait]
impl MusicProvider for HistoryProvider {
    fn platform(&self) -> Platform {
        Platform::Migu
    }
    fn name(&self) -> &'static str {
        "Migu historical chart HTTP contract"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::ChartTracks, Capability::ChartHistoricalTracks])
    }
    async fn chart_tracks(&self, id: &str, r: &ChartTrackListRequest) -> Result<Page<Track>> {
        assert_eq!(id, "chart:20");
        assert_eq!((r.limit, r.offset), (2, 47));
        assert!(r.account.is_none());
        assert_eq!(r.period, self.expected);
        let items = (48..50)
            .map(|id| {
                let mut t = Track::new(
                    ResourceRef::new(Platform::Migu, id.to_string()).unwrap(),
                    "Chart track",
                );
                t.extensions
                    .insert("chart_requested_period".into(), json!(r.period));
                t
            })
            .collect();
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: 2,
                offset: 47,
                total: Some(49),
                has_more: false,
                next_offset: None,
                extensions: Extensions::from([
                    ("requested_period".into(), json!(r.period)),
                    (
                        "period_column_id".into(),
                        json!(if r.period == ChartPeriod::Current {
                            "20"
                        } else {
                            "2001"
                        }),
                    ),
                    ("update_label".into(), json!("2026-09-17")),
                    (
                        "period_binding_scope".into(),
                        json!("request_and_returned_column"),
                    ),
                ]),
            },
        })
    }
}

#[tokio::test]
async fn chart_history_http_forwards_explicit_periods_and_preserves_current_compatibility() {
    for (query, expected) in [
        ("", ChartPeriod::Current),
        ("&period_kind=current", ChartPeriod::Current),
        (
            "&period_kind=day&period_date=2026-01-01",
            ChartPeriod::Day {
                date: "2026-01-01".into(),
            },
        ),
        (
            "&period_kind=week&period_date=2026-09-14",
            ChartPeriod::Week {
                date: "2026-09-14".into(),
            },
        ),
    ] {
        let mut registry = ProviderRegistry::new();
        registry
            .register(HistoryProvider {
                expected: expected.clone(),
            })
            .unwrap();
        let app = build_router(AppState::new(registry, Platform::Migu));
        let (status, v) = json_response_from(
            app,
            &format!("/v1/charts/migu:chart:20/tracks?limit=2&offset=47{query}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(
            v["data"][0]["extensions"]["chart_requested_period"],
            json!(expected)
        );
        assert_eq!(
            v["meta"]["pagination"]["extensions"]["requested_period"],
            json!(expected)
        );
        assert_eq!(
            v["meta"]["pagination"]["extensions"]["update_label"],
            "2026-09-17"
        );
        assert_eq!(v["meta"]["pagination"]["total"], 49);
    }
}

#[tokio::test]
async fn chart_history_http_invalid_or_ambiguous_periods_are_rejected_before_io_with_private_errors()
 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let provider =
        tuneweave_provider_migu::MiguProvider::new(tuneweave_provider_migu::MiguConfig {
            proxy_url: Some(format!("http://{}", listener.local_addr().unwrap())),
            ..Default::default()
        })
        .unwrap();
    assert!(provider.supports(Capability::ChartHistoricalTracks));
    let mut registry = ProviderRegistry::new();
    registry.register(provider).unwrap();
    let app = build_router(AppState::new(registry, Platform::Migu));
    for query in [
        "period_kind=day",
        "period_date=2026-01-01",
        "period_kind=current&period_date=2026-01-01",
        "period_kind=month&period_date=2026-01-01",
        "period_kind=week&period_date=2026-02-29",
        "period_kind=day&period_date=2026-01-01&period_date=2026-02-01",
        "period_kind=current&period_kind=day&period_date=2026-01-01",
        "period_kind=week&period_date=20260911",
        "period_kind=week&period_date=2026-09-11%20",
        "period_kind=&period_date=2026-01-01",
        "period=20260101",
        "period_kind=week&period_date=2026-09-11&date=2026-09-11",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/v1/charts/migu:chart:20/tracks?{query}&account=default"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}

#[tokio::test]
async fn chart_history_unsupported_qq_kugou_and_kuwo_reject_in_sdk_and_http_without_network() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let proxy = Some(format!("http://{}", listener.local_addr().unwrap()));
    let providers: Vec<Box<dyn MusicProvider>> = vec![
        Box::new(
            tuneweave_provider_qq::QqProvider::new(tuneweave_provider_qq::QqConfig {
                proxy_url: proxy.clone(),
                ..Default::default()
            })
            .unwrap(),
        ),
        Box::new(
            tuneweave_provider_kugou::KugouProvider::new(tuneweave_provider_kugou::KugouConfig {
                proxy_url: proxy.clone(),
                ..Default::default()
            })
            .unwrap(),
        ),
        Box::new(
            tuneweave_provider_kuwo::KuwoProvider::new(tuneweave_provider_kuwo::KuwoConfig {
                proxy_url: proxy.clone(),
                ..Default::default()
            })
            .unwrap(),
        ),
    ];
    for provider in &providers {
        for period in [
            ChartPeriod::Day {
                date: "2026-01-01".into(),
            },
            ChartPeriod::Week {
                date: "2026-09-11".into(),
            },
        ] {
            let mut r = ChartTrackListRequest::new(1, 0);
            r.period = period;
            assert_eq!(
                provider.chart_tracks("20", &r).await.unwrap_err().code,
                ErrorCode::CapabilityNotSupported
            );
        }
        assert_eq!(
            provider.supports(Capability::ChartHistoricalTracks),
            provider.platform() == Platform::Kugou
        );
    }
    let mut registry = ProviderRegistry::new();
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
            tuneweave_provider_kugou::KugouProvider::new(tuneweave_provider_kugou::KugouConfig {
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
    let app = build_router(AppState::new(registry, Platform::Qq));
    for platform in ["qq", "kugou", "kuwo"] {
        let (status, v) = json_response_from(
            app.clone(),
            &format!(
                "/v1/charts/{platform}:chart:20/tracks?period_kind=day&period_date=2026-01-01"
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
        assert_eq!(v["error"]["code"], "capability_not_supported");
    }
    assert_eq!(
        listener.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
