use super::*;

struct DigitalProvider {
    caller: bool,
    failure: Option<ErrorCode>,
}

#[async_trait]
impl MusicProvider for DigitalProvider {
    fn platform(&self) -> Platform {
        Platform::Netease
    }
    fn name(&self) -> &'static str {
        "Digital album route fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::SearchAlbums,
            Capability::DigitalAlbumTracks,
            Capability::CallerManagedCredentials,
        ])
    }
    fn with_caller_credential(
        &self,
        credential: &ProviderCredential,
    ) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(credential.platform, Platform::Netease);
        assert_eq!(credential.secret(), "original-album");
        Ok(Arc::new(Self {
            caller: true,
            failure: self.failure,
        }))
    }
    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        self.caller
            .then(|| ProviderCredential::new(Platform::Netease, "test", "rotated-album", None))
            .transpose()
    }
    async fn digital_album_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        assert_eq!(id, "700");
        assert_eq!(
            request.account.as_deref(),
            Some(if self.caller { "default" } else { "personal" })
        );
        assert_eq!(request.limit, 7);
        assert_eq!(request.offset, 2);
        if let Some(code) = self.failure {
            return Err(
                TuneWeaveError::new(code, "album read failed").with_platform(Platform::Netease)
            );
        }
        let mut track = sample_track("12");
        track.album = Some(AlbumSummary {
            resource_ref: Some(ResourceRef::new(Platform::Netease, "77").unwrap()),
            name: "Ordinary album".to_owned(),
            cover_url: None,
        });
        Ok(Page {
            items: vec![track],
            pagination: PageMeta {
                limit: 7,
                offset: 2,
                total: Some(3),
                has_more: false,
                next_offset: None,
                extensions: Extensions::from([
                    ("collection_id".to_owned(), json!(id)),
                    ("collection_type".to_owned(), json!("digital_album")),
                ]),
            },
        })
    }
    async fn search_catalog(&self, query: &SearchQuery) -> Result<Page<SearchItem>> {
        assert_eq!(query.kind, SearchKind::Album);
        Ok(Page {
            items: vec![
                SearchItem::Album(sample_album("77")),
                SearchItem::DigitalAlbum(sample_digital_album("700")),
            ],
            pagination: PageMeta {
                limit: query.limit,
                offset: query.offset,
                total: None,
                has_more: false,
                next_offset: None,
                extensions: Extensions::new(),
            },
        })
    }
}

fn app(failure: Option<ErrorCode>) -> Router {
    let mut registry = ProviderRegistry::new();
    registry
        .register(DigitalProvider {
            caller: false,
            failure,
        })
        .unwrap();
    build_router(AppState::new(registry, Platform::Netease))
}
fn credential() -> CallerCredential {
    CallerCredential::issue(
        &ProviderCredential::new(Platform::Netease, "test", "original-album", None).unwrap(),
    )
    .unwrap()
}

#[tokio::test]
async fn digital_album_tracks_route_keeps_source_pagination_and_latest_credentials() {
    for caller in [false, true] {
        let mut path = "/v1/digital-albums/netease:700/tracks?limit=7&offset=2".to_owned();
        if !caller {
            path.push_str("&account=personal");
        }
        let mut request = Request::builder().uri(path);
        if caller {
            request = request.header(CALLER_CREDENTIAL_HEADER, credential().value);
        }
        let response = app(None)
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response.headers()[header::CACHE_CONTROL]
                .to_str()
                .unwrap()
                .contains("no-store")
        );
        let updated = response
            .headers()
            .get(caller_scope::UPDATED_CREDENTIAL_HEADER);
        if caller {
            let updated = updated.unwrap();
            assert!(updated.is_sensitive());
            let credential = CallerCredential::parse(
                updated.to_str().unwrap().strip_prefix("netease=").unwrap(),
            )
            .unwrap();
            assert_eq!(credential.secret(), "rotated-album");
        } else {
            assert!(updated.is_none());
        }
        let value: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(value["data"][0]["album"]["ref"], "netease:77");
        assert_eq!(value["meta"]["pagination"]["offset"], 2);
        assert_eq!(value["meta"]["pagination"]["total"], 3);
        assert_eq!(
            value["meta"]["pagination"]["extensions"]["collection_id"],
            "700"
        );
        if caller {
            assert!(value["meta"].get("account").is_none());
        } else {
            assert_eq!(value["meta"]["account"], "personal");
        }
    }
}

#[tokio::test]
async fn digital_album_route_rejects_invalid_inputs_and_suppresses_invalidated_rotations() {
    for query in [
        "limit=0",
        "limit=101",
        "offset=-1",
        "offset=4294967296",
        "unknown=true",
        "limit=invalid",
    ] {
        let (status, value) = json_response_from(
            app(None),
            &format!("/v1/digital-albums/netease:700/tracks?{query}"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(value["error"]["code"], "invalid_request");
    }
    for (code, status) in [
        (ErrorCode::UpstreamError, StatusCode::BAD_GATEWAY),
        (ErrorCode::AuthenticationRequired, StatusCode::UNAUTHORIZED),
        (ErrorCode::Conflict, StatusCode::CONFLICT),
    ] {
        let response = app(Some(code))
            .oneshot(
                Request::builder()
                    .uri("/v1/digital-albums/netease:700/tracks?limit=7&offset=2")
                    .header(CALLER_CREDENTIAL_HEADER, credential().value)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), status);
        assert!(
            response.headers()[header::CACHE_CONTROL]
                .to_str()
                .unwrap()
                .contains("no-store")
        );
        assert_eq!(
            response
                .headers()
                .contains_key(caller_scope::UPDATED_CREDENTIAL_HEADER),
            code == ErrorCode::UpstreamError
        );
    }
    let (status, value) = caller_json_request(
        app(None),
        Method::GET,
        "/v1/digital-albums/netease:700/tracks?account=personal",
        None,
        &credential(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(value["error"]["code"], "invalid_request");
}

#[tokio::test]
async fn album_search_serializes_distinct_digital_results_and_round_trips_them() {
    let (status, value) =
        json_response_from(app(None), "/v1/search?platform=netease&type=album&q=albums").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["data"][0]["type"], "album");
    assert_eq!(value["data"][1]["type"], "digital_album");
    assert_eq!(value["data"][1]["data"]["ref"], "netease:700");
    let parsed: Vec<SearchItem> = serde_json::from_value(value["data"].clone()).unwrap();
    assert!(matches!(&parsed[1],SearchItem::DigitalAlbum(album) if album.id=="700"));
}
