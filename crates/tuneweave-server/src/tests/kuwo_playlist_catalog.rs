use super::*;
use tuneweave_core::{
    PlaylistCatalogKind, PlaylistCatalogRequest, PlaylistCatalogTag, PlaylistCatalogTagGroup,
    PlaylistCatalogTaxonomy, PlaylistCatalogTaxonomyRequest,
};

struct Provider {
    fail: bool,
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Kuwo
    }
    fn name(&self) -> &'static str {
        "Kuwo catalogue contract"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::PlaylistCatalog, Capability::PlaylistRead])
    }
    async fn playlist_catalog(&self, request: &PlaylistCatalogRequest) -> Result<Page<Playlist>> {
        assert!(request.account.is_none());
        if request.catalog == PlaylistCatalogKind::Tag {
            assert_eq!(request.tag_id.as_deref(), Some("2189"));
        } else {
            assert!(request.tag_id.is_none());
        }
        if self.fail {
            return Err(
                TuneWeaveError::new(ErrorCode::UpstreamError, "Catalogue total changed")
                    .with_platform(Platform::Kuwo),
            );
        }
        let all = vec![test_kuwo_playlist()];
        let items = all
            .into_iter()
            .skip(request.offset as usize)
            .take(request.limit as usize)
            .collect::<Vec<_>>();
        let end = request.offset + items.len() as u32;
        Ok(Page {
            items,
            pagination: PageMeta {
                limit: request.limit,
                offset: request.offset,
                total: Some(1),
                has_more: end < 1,
                next_offset: (end < 1).then_some(end),
                extensions: Extensions::from([
                    ("catalog".into(), json!(request.catalog)),
                    (
                        "catalog_scope".into(),
                        json!("official_web_curated_playlists"),
                    ),
                ]),
            },
        })
    }
    async fn playlist_catalog_taxonomy(
        &self,
        request: &PlaylistCatalogTaxonomyRequest,
    ) -> Result<PlaylistCatalogTaxonomy> {
        assert!(request.account.is_none());
        if self.fail {
            return Err(
                TuneWeaveError::new(ErrorCode::UpstreamError, "Catalogue taxonomy failed")
                    .with_platform(Platform::Kuwo),
            );
        }
        Ok(PlaylistCatalogTaxonomy {
            platform: Platform::Kuwo,
            groups: vec![PlaylistCatalogTagGroup {
                id: "5".into(),
                name: "主题".into(),
                tags: vec![PlaylistCatalogTag {
                    id: "2189".into(),
                    name: "短视频".into(),
                    extensions: Extensions::new(),
                }],
                extensions: Extensions::new(),
            }],
            extensions: Extensions::new(),
        })
    }
    async fn playlist(&self, id: &str, account: Option<&str>) -> Result<Playlist> {
        TestKuwoProvider.playlist(id, account).await
    }
    async fn playlist_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        TestKuwoProvider.playlist_tracks(id, request).await
    }
}
fn app(fail: bool) -> Router {
    let mut registry = ProviderRegistry::new();
    registry.register(Provider { fail }).unwrap();
    build_router(AppState::new(registry, Platform::Kuwo))
}

#[tokio::test]
async fn kuwo_playlist_catalogue_http_preserves_explicit_kind_and_empty_page_total() {
    for kind in ["latest", "hot"] {
        for offset in [0, 19] {
            let (status, v) = json_response_from(
                app(false),
                &format!("/v1/playlists?platform=kuwo&catalog={kind}&limit=1&offset={offset}"),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{v}");
            assert_eq!(v["meta"]["pagination"]["limit"], 1);
            assert_eq!(v["meta"]["pagination"]["offset"], offset);
            assert_eq!(v["meta"]["pagination"]["total"], 1);
            assert_eq!(v["meta"]["pagination"]["has_more"], false);
            assert_eq!(v["meta"]["pagination"]["extensions"]["catalog"], kind);
            assert_eq!(
                v["data"].as_array().unwrap().len(),
                usize::from(offset == 0)
            );
            if offset == 0 {
                assert_eq!(v["data"][0]["ref"], "kuwo:2952464073");
            }
        }
    }
    let (status, v) = json_response_from(app(false), "/v1/playlists?catalog=latest").await;
    assert_eq!(status, StatusCode::OK, "{v}");
    assert_eq!(v["meta"]["pagination"]["limit"], 20);
    let (status, v) = json_response_from(app(true), "/v1/playlists?catalog=hot").await;
    assert_eq!(status, StatusCode::BAD_GATEWAY, "{v}");
    assert_eq!(v["error"]["code"], "upstream_error");
    assert!(v.get("data").is_none_or(Value::is_null));
}

#[tokio::test]
async fn kuwo_playlist_catalogue_http_requires_catalog_and_rejects_unimplemented_filters() {
    for query in [
        "",
        "catalog=",
        "catalog=new",
        "catalog=future",
        "catalog=latest&catalog=hot",
        "catalog=latest&tag=123",
        "catalog=latest&tag_id=2189",
        "catalog=tag",
        "catalog=tag&tag_id=",
        "catalog=tag&tag_id=%20%20",
        "catalog=hot&order=new",
        "catalog=hot&limit=0",
        "catalog=hot&limit=101",
        "catalog=hot&offset=4294967295",
        "catalog=hot&offset=-1",
        "catalog=hot&limit=abc",
        "catalog=hot&page=1",
    ] {
        let (status, v) = json_response_from(app(false), &format!("/v1/playlists?{query}")).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{query}: {v}");
    }
}

#[tokio::test]
async fn kuwo_playlist_tag_taxonomy_and_tag_catalogue_are_explicit() {
    let (status, taxonomy) =
        json_response_from(app(false), "/v1/playlists/tags?platform=kuwo").await;
    assert_eq!(status, StatusCode::OK, "{taxonomy}");
    assert_eq!(taxonomy["data"]["platform"], "kuwo");
    assert_eq!(taxonomy["data"]["groups"][0]["id"], "5");
    assert_eq!(taxonomy["data"]["groups"][0]["tags"][0]["id"], "2189");

    let (status, playlists) = json_response_from(
        app(false),
        "/v1/playlists?platform=kuwo&catalog=tag&tag_id=2189&limit=20&offset=0",
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{playlists}");
    assert_eq!(playlists["meta"]["pagination"]["total"], 1);
    assert_eq!(playlists["data"][0]["ref"], "kuwo:2952464073");
}

#[tokio::test]
async fn kuwo_playlist_catalogue_http_default_trait_stays_unsupported() {
    // The new public method does not silently turn existing recommendations into a catalogue.
    let request = PlaylistCatalogRequest::new(PlaylistCatalogKind::Latest, 20, 0);
    assert_eq!(
        TestKuwoProvider
            .playlist_catalog(&request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert_eq!(
        TestKuwoProvider
            .playlist_catalog_taxonomy(&PlaylistCatalogTaxonomyRequest { account: None })
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    let (status, v) = json_response_from(
        test_app_with_kuwo(),
        "/v1/playlists?platform=kuwo&catalog=latest",
    )
    .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{v}");
    assert_eq!(v["error"]["details"]["capability"], "playlist_catalog");
}

#[tokio::test]
async fn kuwo_playlist_catalogue_http_real_provider_rejects_account_context_without_io() {
    let guard = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    guard.set_nonblocking(true).unwrap();
    let provider =
        tuneweave_provider_kuwo::KuwoProvider::new(tuneweave_provider_kuwo::KuwoConfig {
            proxy_url: Some(format!("http://{}", guard.local_addr().unwrap())),
            ..Default::default()
        })
        .unwrap();
    assert!(provider.supports(Capability::PlaylistCatalog));
    let mut registry = ProviderRegistry::new();
    registry.register(provider).unwrap();
    let app = build_router(AppState::new(registry, Platform::Kuwo));
    let credential = CallerCredential::issue(
        &ProviderCredential::new(
            Platform::Kuwo,
            "unsupported",
            "private-playlist-catalogue",
            None,
        )
        .unwrap(),
    )
    .unwrap();
    for (query, caller) in [
        ("catalog=latest&account=default", false),
        ("catalog=hot&account=named", false),
        ("catalog=latest", true),
        ("catalog=tag&tag_id=2189&account=default", false),
        ("catalog=tag&tag_id=2189", true),
    ] {
        let mut request = Request::builder().uri(format!("/v1/playlists?platform=kuwo&{query}"));
        if caller {
            request = request.header(CALLER_CREDENTIAL_HEADER, &credential.value);
        }
        let response = app
            .clone()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("private-playlist-catalogue"));
    }
    assert_eq!(
        guard.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/v1/playlists/tags?platform=kuwo&account=default")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn kuwo_playlist_catalogue_http_reference_supports_detail_server_and_client_uni() {
    let app = app(false);
    let (status, catalogue) = json_response_from(
        app.clone(),
        "/v1/playlists?platform=kuwo&catalog=hot&limit=1",
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let reference = catalogue["data"][0]["ref"].as_str().unwrap();
    let (status, detail) =
        json_response_from(app.clone(), &format!("/v1/playlists/{reference}")).await;
    assert_eq!(status, StatusCode::OK, "{detail}");
    assert_eq!(detail["data"]["ref"], reference);
    assert_eq!(detail["data"]["track_count"], 3);
    let source = json!({"ref":reference,"type":"playlist"});
    let (status, imported) = json_request_from(
        app.clone(),
        Method::POST,
        "/v1/uni/playlists/imports",
        Some(json!({"sources":[source.clone()]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{imported}");
    assert_eq!(imported["data"]["playlist"]["item_count"], 3);
    assert_eq!(imported["data"]["sources"][0]["ref"], reference);
    let uni_ref = imported["data"]["playlist"]["ref"].as_str().unwrap();
    let (status, items) =
        json_response_from(app.clone(), &format!("/v1/uni/playlists/{uni_ref}/items")).await;
    assert_eq!(status, StatusCode::OK, "{items}");
    assert_eq!(items["data"][0]["source_ref"], "kuwo:215257");
    assert_eq!(items["data"][1]["source_ref"], "kuwo:6871885");
    assert_eq!(items["data"][2]["source_ref"], "kuwo:215257");
    assert_ne!(items["data"][0]["id"], items["data"][2]["id"]);
    let (status, materialized) = json_request_from(
        app,
        Method::POST,
        "/v1/uni/materialize/imports?limit=2&offset=1",
        Some(json!({"sources":[source]})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{materialized}");
    assert_eq!(materialized["data"]["item_count"], 3);
    assert_eq!(
        materialized["data"]["items"][0]["source_ref"],
        "kuwo:6871885"
    );
    assert_eq!(
        materialized["data"]["items"][1]["source_ref"],
        "kuwo:215257"
    );
    assert_eq!(materialized["data"]["items"][1]["position"], 2);
}
