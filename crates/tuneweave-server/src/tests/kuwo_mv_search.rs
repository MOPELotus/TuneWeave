//! Search routing contracts; Kuwo has real protocol loopback and opt-in tests.
use super::*;
struct Provider {
    failure: bool,
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Kuwo
    }
    fn name(&self) -> &'static str {
        "Kuwo MV search contract"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::SearchMvs])
    }
    async fn search_catalog(&self, q: &SearchQuery) -> Result<Page<SearchItem>> {
        assert_eq!(q.kind, SearchKind::Mv);
        assert_eq!(q.variant, SearchVariant::Default);
        assert!(q.account.is_none() && q.video_filters.is_none());
        assert_eq!(q.query, "test");
        assert_eq!((q.limit, q.offset), (2, 19));
        if self.failure {
            return Err(
                TuneWeaveError::new(ErrorCode::UpstreamError, "MV catalogue changed")
                    .with_platform(Platform::Kuwo),
            );
        }
        let mut video = sample_video("20");
        video.platform = Platform::Kuwo;
        video.resource_ref = ResourceRef::new(Platform::Kuwo, "20").unwrap();
        video.subscribed = None;
        video.extensions = Extensions::from([
            ("kind".into(), json!("mv")),
            ("source_track_id".into(), json!("20")),
            ("online".into(), json!(0)),
        ]);
        Ok(Page {
            items: vec![SearchItem::Video(video)],
            pagination: PageMeta {
                limit: 2,
                offset: 19,
                total: Some(20),
                has_more: false,
                next_offset: None,
                extensions: Extensions::from([(
                    "pagination_scope".into(),
                    json!("upstream_catalogue_positions"),
                )]),
            },
        })
    }
}
#[tokio::test]
async fn kuwo_mv_search_http_maps_typed_video_metadata_pagination_and_terminal_errors() {
    for failure in [false, true] {
        for selector in ["type=mv", "kind=mv", "type=1004"] {
            let mut registry = ProviderRegistry::new();
            registry.register(Provider { failure }).unwrap();
            let app = build_router(AppState::new(registry, Platform::Kuwo));
            let (status, value) = json_response_from(
                app,
                &format!("/v1/search?platform=kuwo&{selector}&q=test&limit=2&offset=19"),
            )
            .await;
            if failure {
                assert_eq!(status, StatusCode::BAD_GATEWAY);
                assert_eq!(value["error"]["code"], "upstream_error");
                assert!(value.get("data").is_none_or(Value::is_null));
            } else {
                assert_eq!(status, StatusCode::OK, "{value}");
                assert_eq!(value["data"][0]["type"], "video");
                let v = &value["data"][0]["data"];
                assert_eq!(v["ref"], "kuwo:20");
                assert_eq!(v["extensions"]["kind"], "mv");
                assert_eq!(v["extensions"]["online"], 0);
                assert!(v["subscribed"].is_null());
                let p = &value["meta"]["pagination"];
                assert_eq!(p["total"], 20);
                assert_eq!(p["offset"], 19);
                assert_eq!(p["has_more"], false);
                assert_eq!(
                    p["extensions"]["pagination_scope"],
                    "upstream_catalogue_positions"
                );
            }
        }
    }
}
#[tokio::test]
async fn kuwo_mv_search_http_real_provider_rejects_unproven_filters_and_credentials_before_io() {
    let guard = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    guard.set_nonblocking(true).unwrap();
    let provider =
        tuneweave_provider_kuwo::KuwoProvider::new(tuneweave_provider_kuwo::KuwoConfig {
            proxy_url: Some(format!("http://{}", guard.local_addr().unwrap())),
            ..Default::default()
        })
        .unwrap();
    assert!(provider.supports(Capability::SearchMvs));
    let mut registry = ProviderRegistry::new();
    registry.register(provider).unwrap();
    let app = build_router(AppState::new(registry, Platform::Kuwo));
    for tail in [
        "&account=default",
        "&variant=cloud",
        "&order=newest",
        "&order=relevance",
        "&duration=any",
        "&category_id=1",
        "&highlight=true",
        "&search_id=foreign",
        "&limit=0",
        "&limit=101",
        "&offset=4294967295",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/search?platform=kuwo&type=mv&q=test{tail}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{tail}");
        if tail.contains("account") {
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        }
    }
    let credential = CallerCredential::issue(
        &ProviderCredential::new(
            Platform::Kuwo,
            "unsupported",
            "private-search-fixture",
            None,
        )
        .unwrap(),
    )
    .unwrap();
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/search?platform=kuwo&type=mv&q=test")
                .header(CALLER_CREDENTIAL_HEADER, credential.value)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("private-search-fixture"));
    assert_eq!(
        guard.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
