use super::*;

struct Provider {
    failure: bool,
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Migu
    }
    fn name(&self) -> &'static str {
        "Migu chart contract"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::ChartCatalog, Capability::ChartTracks])
    }
    async fn chart_catalog(&self, r: &ChartCatalogRequest) -> Result<ChartCatalog> {
        assert!(r.account.is_none());
        Ok(ChartCatalog {
            platform: Platform::Migu,
            view: r.view,
            groups: vec![ChartGroup {
                code: None,
                name: "Official".into(),
                display_type: None,
                target_url: None,
                extensions: Extensions::new(),
                charts: vec![Chart {
                    resource_ref: Some(ResourceRef::new(Platform::Migu, "chart:16").unwrap()),
                    platform: Platform::Migu,
                    id: Some("16".into()),
                    name: "Hot".into(),
                    description: String::new(),
                    cover_url: None,
                    update_frequency: None,
                    updated_at_ms: None,
                    track_count: None,
                    play_count: None,
                    subscribed: None,
                    playable: None,
                    target_kind: Some("chart".into()),
                    target_url: None,
                    previews: vec![],
                    extensions: Extensions::new(),
                }],
            }],
            extensions: Extensions::from([
                ("source_data_version".into(), json!("1789614335187")),
                ("period_scope".into(), json!("current")),
            ]),
        })
    }
    async fn chart_tracks(&self, id: &str, r: &ChartTrackListRequest) -> Result<Page<Track>> {
        assert!(matches!(id, "16" | "chart:16"));
        assert!(r.account.is_none());
        assert_eq!((r.limit, r.offset), (2, 18));
        if self.failure {
            return Err(
                TuneWeaveError::new(ErrorCode::UpstreamError, "Migu chart count disagrees with its complete response")
                    .with_platform(Platform::Migu)
                    .with_details(json!({"reason":"chart_count_mismatch","declared_track_count":100,"received_track_count":99})),
            );
        }
        let mut t = sample_track("19");
        t.platform = Platform::Migu;
        t.resource_ref = ResourceRef::new(Platform::Migu, "19").unwrap();
        t.playable = None;
        t.extensions = Extensions::from([
            ("chart_id".into(), json!("16")),
            ("chart_rank".into(), json!(19)),
        ]);
        if r.include_tags {
            t.extensions.insert("chart_rank_change".into(), json!(-2));
        }
        Ok(Page {
            items: vec![t],
            pagination: PageMeta {
                limit: 2,
                offset: 18,
                total: Some(19),
                has_more: false,
                next_offset: None,
                extensions: Extensions::from([
                    ("update_label".into(), json!("2026-09-16")),
                    (
                        "consistency_scope".into(),
                        json!("single_complete_response"),
                    ),
                ]),
            },
        })
    }
}

#[tokio::test]
async fn migu_charts_http_preserves_source_reference_views_tags_pagination_and_errors() {
    for failure in [false, true] {
        let mut registry = ProviderRegistry::new();
        registry.register(Provider { failure }).unwrap();
        let app = build_router(AppState::new(registry, Platform::Migu));
        for view in ["overview", "summary", "modern"] {
            let (status, v) = json_response_from(
                app.clone(),
                &format!("/v1/charts?platform=migu&view={view}"),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(v["data"]["view"], view);
            let c = &v["data"]["groups"][0]["charts"][0];
            assert_eq!(c["ref"], "migu:chart:16");
            assert_eq!(
                v["data"]["extensions"]["source_data_version"],
                "1789614335187"
            );
            assert_eq!(v["data"]["extensions"]["period_scope"], "current");
            assert!(c["updated_at_ms"].is_null() && c["playable"].is_null());
        }
        for id in ["16", "chart:16"] {
            for pagination in ["offset=18", "page=10"] {
                for (tag, expected) in [
                    ("", true),
                    ("&tag=false", false),
                    ("&include_tags=true", true),
                ] {
                    let (status, v) = json_response_from(
                        app.clone(),
                        &format!("/v1/charts/migu:{id}/tracks?limit=2&{pagination}{tag}"),
                    )
                    .await;
                    if failure {
                        assert_eq!(status, StatusCode::BAD_GATEWAY);
                        assert_eq!(v["error"]["code"], "upstream_error");
                        assert_eq!(v["error"]["details"]["reason"], "chart_count_mismatch");
                        assert_eq!(v["error"]["details"]["declared_track_count"], 100);
                        assert_eq!(v["error"]["details"]["received_track_count"], 99);
                        assert!(v.get("data").is_none_or(Value::is_null));
                    } else {
                        assert_eq!(status, StatusCode::OK, "{v}");
                        assert_eq!(v["data"][0]["ref"], "migu:19");
                        assert_eq!(v["data"][0]["extensions"]["chart_rank"], 19);
                        assert_eq!(
                            v["data"][0]["extensions"]
                                .get("chart_rank_change")
                                .is_some(),
                            expected
                        );
                        assert!(v["data"][0]["playable"].is_null());
                        assert_eq!(v["meta"]["pagination"]["total"], 19);
                        assert_eq!(
                            v["meta"]["pagination"]["extensions"]["update_label"],
                            "2026-09-16"
                        );
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn migu_charts_http_real_provider_rejects_credentials_and_bad_windows_before_io() {
    let guard = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    guard.set_nonblocking(true).unwrap();
    let provider =
        tuneweave_provider_migu::MiguProvider::new(tuneweave_provider_migu::MiguConfig {
            proxy_url: Some(format!("http://{}", guard.local_addr().unwrap())),
            ..Default::default()
        })
        .unwrap();
    assert!(
        provider.supports(Capability::ChartCatalog) && provider.supports(Capability::ChartTracks)
    );
    let mut registry = ProviderRegistry::new();
    registry.register(provider).unwrap();
    let app = build_router(AppState::new(registry, Platform::Migu));
    for uri in [
        "/v1/charts?platform=migu&view=future",
        "/v1/charts?platform=migu&account=default",
        "/v1/charts/migu:chart:01/tracks",
        "/v1/charts/migu:16/tracks?limit=0",
        "/v1/charts/migu:16/tracks?limit=101",
        "/v1/charts/migu:16/tracks?offset=4294967295",
        "/v1/charts/migu:16/tracks?page=0",
        "/v1/charts/migu:16/tracks?page=1&offset=0",
        "/v1/charts/migu:16/tracks?tag=maybe",
        "/v1/charts/migu:16/tracks?account=default",
    ] {
        let response = app
            .clone()
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        if uri.contains("account") {
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
    }
    let credential = CallerCredential::issue(
        &ProviderCredential::new(Platform::Migu, "unsupported", "private-chart-fixture", None)
            .unwrap(),
    )
    .unwrap();
    for uri in [
        "/v1/charts?platform=migu",
        "/v1/charts/migu:chart:16/tracks",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header(CALLER_CREDENTIAL_HEADER, &credential.value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("private-chart-fixture"));
    }
    assert_eq!(
        guard.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
