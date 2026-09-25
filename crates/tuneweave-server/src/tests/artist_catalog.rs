use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
struct ArtistProvider(Arc<AtomicUsize>);
impl ArtistProvider {
    fn check(&self, id: &str, limit: u32, offset: u32, account: Option<&str>) {
        assert_eq!(id, "112");
        assert_eq!((limit, offset), (2, 3));
        assert_eq!(account, None);
        self.0.fetch_add(1, Ordering::SeqCst);
    }
    fn page<T>(&self, items: Vec<T>) -> Page<T> {
        Page {
            items,
            pagination: PageMeta {
                limit: 2,
                offset: 3,
                total: Some(4),
                has_more: false,
                next_offset: None,
                extensions: Extensions::new(),
            },
        }
    }
}
#[async_trait]
impl MusicProvider for ArtistProvider {
    fn platform(&self) -> Platform {
        Platform::Migu
    }
    fn name(&self) -> &'static str {
        "Artist HTTP fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::ArtistTracks,
            Capability::ArtistAlbums,
            Capability::ArtistDigitalAlbums,
        ])
    }
    async fn artist_tracks(&self, id: &str, r: &ArtistTrackListRequest) -> Result<Page<Track>> {
        self.check(id, r.limit, r.offset, r.account.as_deref());
        assert_eq!(r.order, ArtistTrackOrder::PlatformDefault);
        Ok(self.page(vec![Track::new(
            ResourceRef::new(Platform::Migu, "77").unwrap(),
            "Song",
        )]))
    }
    async fn artist_albums(&self, id: &str, r: &PageRequest) -> Result<Page<Album>> {
        self.check(id, r.limit, r.offset, r.account.as_deref());
        let mut album = sample_album("77");
        album.resource_ref = ResourceRef::new(Platform::Migu, "77").unwrap();
        album.platform = Platform::Migu;
        album
            .extensions
            .insert("resource_type".into(), json!("2003"));
        Ok(self.page(vec![album]))
    }
    async fn artist_digital_albums(&self, id: &str, r: &PageRequest) -> Result<Page<DigitalAlbum>> {
        self.check(id, r.limit, r.offset, r.account.as_deref());
        let mut album = sample_digital_album("77");
        album.resource_ref = ResourceRef::new(Platform::Migu, "77").unwrap();
        album.platform = Platform::Migu;
        album.purchased = None;
        album.extensions.insert("resource_type".into(), json!("5"));
        Ok(self.page(vec![album]))
    }
}
fn app() -> (Router, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = ProviderRegistry::new();
    registry.register(ArtistProvider(calls.clone())).unwrap();
    (build_router(AppState::new(registry, Platform::Migu)), calls)
}

#[tokio::test]
async fn artist_http_separates_digital_albums_and_preserves_explicit_platform_order() {
    for kind in ["tracks", "albums", "digital-albums"] {
        let (router, calls) = app();
        let uri = format!(
            "/v1/artists/migu:112/{kind}?limit=2&offset=3{}",
            if kind == "tracks" {
                "&order=platform_default"
            } else {
                ""
            }
        );
        let response = router
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(body["data"][0]["ref"], "migu:77");
        assert_eq!(body["meta"]["pagination"]["total"], 4);
        assert_eq!(body["meta"]["pagination"]["offset"], 3);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        if kind != "tracks" {
            assert_eq!(
                body["data"][0]["extensions"]["resource_type"],
                if kind == "albums" { "2003" } else { "5" }
            );
        }
        if kind == "digital-albums" {
            assert!(body["data"][0]["purchased"].is_null());
        }
    }
    assert_eq!(ArtistTrackOrder::default(), ArtistTrackOrder::Hot);
    assert_eq!(
        serde_json::to_value(ArtistTrackOrder::PlatformDefault).unwrap(),
        "platform_default"
    );
}

#[tokio::test]
async fn artist_digital_album_http_rejects_invalid_pages_unknown_fields_and_references() {
    for path in [
        "/v1/artists/migu:112/digital-albums?limit=0",
        "/v1/artists/migu:112/digital-albums?limit=101",
        "/v1/artists/migu:112/digital-albums?offset=-1",
        "/v1/artists/migu:112/digital-albums?limit=2&offset=4294967295",
        "/v1/artists/migu:112/digital-albums?unknown=1",
        "/v1/artists/invalid/digital-albums",
    ] {
        let (router, calls) = app();
        let response = router
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
