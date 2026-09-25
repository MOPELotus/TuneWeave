//! HTTP and Uni fixtures; native ownership/protocol has separate provider tests.
use super::*;

#[derive(Clone)]
struct Provider {
    expected_account: Option<&'static str>,
    mode: &'static str,
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Kuwo
    }
    fn name(&self) -> &'static str {
        "Kuwo favorites contract fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::Favorites, Capability::CallerManagedCredentials])
    }
    fn with_caller_credential(&self, c: &ProviderCredential) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(c.secret(), "private-native-fixture");
        Ok(Arc::new(Self {
            expected_account: Some("default"),
            ..self.clone()
        }))
    }
    async fn favorite_playlist(&self, account: Option<&str>) -> Result<Playlist> {
        assert_eq!(account, self.expected_account);
        if self.mode == "denied" {
            return Err(denied());
        }
        let mut p = test_kuwo_playlist();
        p.id = "901".into();
        p.resource_ref = ResourceRef::new(Platform::Kuwo, "901").unwrap();
        p.name = "我喜欢听".into();
        p.track_count = Some(3);
        p.creator = None;
        p.extensions = Extensions::from([
            ("source_snapshot_id".into(), json!("revision-a")),
            ("is_favorite".into(), json!(true)),
        ]);
        Ok(p)
    }
    async fn favorite_tracks(&self, r: &PageRequest) -> Result<Page<Track>> {
        assert_eq!(r.account.as_deref(), self.expected_account);
        if self.mode == "denied" {
            return Err(denied());
        }
        if r.offset == 2 && self.mode == "auth" {
            return Err(
                TuneWeaveError::new(ErrorCode::AuthenticationRequired, "expired")
                    .with_platform(Platform::Kuwo),
            );
        }
        let (ids, next) = match r.offset {
            0 => (vec![11, 22], Some(2)),
            2 => (vec![22], None),
            _ => panic!("unexpected fixture window"),
        };
        let revision = if r.offset == 2 && self.mode == "changed" {
            "revision-b"
        } else {
            "revision-a"
        };
        Ok(Page {
            items: ids
                .into_iter()
                .map(|id| {
                    Track::new(
                        ResourceRef::new(Platform::Kuwo, id.to_string()).unwrap(),
                        "Song",
                    )
                })
                .collect(),
            pagination: PageMeta {
                limit: r.limit,
                offset: r.offset,
                total: Some(3),
                has_more: next.is_some(),
                next_offset: next,
                extensions: Extensions::from([("source_snapshot_id".into(), json!(revision))]),
            },
        })
    }
    async fn user_favorite_playlist(&self, uid: &str, account: Option<&str>) -> Result<Playlist> {
        if uid != "42" {
            return Err(denied());
        }
        self.favorite_playlist(account).await
    }
    async fn user_favorite_tracks(&self, uid: &str, r: &PageRequest) -> Result<Page<Track>> {
        if uid != "42" {
            return Err(denied());
        }
        self.favorite_tracks(r).await
    }
    async fn playlist_source(
        &self,
        uid: &str,
        kind: &str,
        account: Option<&str>,
    ) -> Result<Playlist> {
        assert_eq!(kind, "favorite_tracks");
        self.user_favorite_playlist(uid, account).await
    }
    async fn playlist_source_items(
        &self,
        uid: &str,
        kind: &str,
        r: &PageRequest,
    ) -> Result<Page<PlaylistPlayableItem>> {
        assert_eq!(kind, "favorite_tracks");
        let p = self.user_favorite_tracks(uid, r).await?;
        Ok(Page {
            items: p
                .items
                .into_iter()
                .map(PlaylistPlayableItem::Track)
                .collect(),
            pagination: p.pagination,
        })
    }
}
fn denied() -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::PermissionDenied, "not selected account")
        .with_platform(Platform::Kuwo)
}
fn app(mode: &'static str, expected_account: Option<&'static str>) -> Router {
    let mut registry = ProviderRegistry::new();
    registry
        .register(Provider {
            mode,
            expected_account,
        })
        .unwrap();
    build_router(AppState::new(registry, Platform::Kuwo))
}
fn credential() -> String {
    CallerCredential::issue(
        &ProviderCredential::new(Platform::Kuwo, "fixture", "private-native-fixture", None)
            .unwrap(),
    )
    .unwrap()
    .value
}

#[tokio::test]
async fn kuwo_favorite_http_four_routes_preserve_default_named_and_caller_scope_and_no_store() {
    for path in [
        "/v1/account/favorites/playlist",
        "/v1/account/favorites/tracks",
        "/v1/users/kuwo:42/favorites/playlist",
        "/v1/users/kuwo:42/favorites/tracks",
    ] {
        for scope in ["default", "named", "caller"] {
            for mode in ["stable", "denied"] {
                let mut url = path.to_owned();
                if scope == "named" {
                    url.push_str("?account=personal");
                }
                let expected = if scope == "named" {
                    Some("personal")
                } else if scope == "caller" || path.contains("/account/") {
                    Some("default")
                } else {
                    None
                };
                let mut r = Request::builder().uri(url);
                if scope == "caller" {
                    r = r.header(CALLER_CREDENTIAL_HEADER, credential());
                }
                let response = app(mode, expected)
                    .oneshot(r.body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    if mode == "stable" {
                        StatusCode::OK
                    } else {
                        StatusCode::FORBIDDEN
                    }
                );
                assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
                assert!(
                    response
                        .headers()
                        .get(caller_scope::UPDATED_CREDENTIAL_HEADER)
                        .is_none()
                );
                let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
                let value: Value = serde_json::from_slice(&bytes).unwrap();
                assert!(!String::from_utf8_lossy(&bytes).contains("private-native-fixture"));
                if mode == "stable" {
                    if path.ends_with("playlist") {
                        assert_eq!(value["data"]["ref"], "kuwo:901");
                        assert_eq!(value["data"]["name"], "我喜欢听");
                    } else {
                        assert_eq!(value["data"][0]["ref"], "kuwo:11");
                        assert_eq!(value["meta"]["pagination"]["total"], 3);
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn kuwo_favorite_uni_uses_uid_source_real_playlist_and_atomic_consistent_items() {
    for caller in [false, true] {
        for materialize in [false, true] {
            for mode in ["stable", "changed", "auth"] {
                let router = app(mode, Some(if caller { "default" } else { "personal" }));
                let mut source = json!({"ref":"kuwo:42","type":"favorite_tracks"});
                if !caller {
                    source["account"] = json!("personal");
                }
                let mut request = Request::builder()
                    .method(Method::POST)
                    .uri(if materialize {
                        "/v1/uni/materialize/imports"
                    } else {
                        "/v1/uni/playlists/imports"
                    })
                    .header(header::CONTENT_TYPE, "application/json");
                if caller {
                    request = request.header(CALLER_CREDENTIAL_HEADER, credential());
                }
                let response = router
                    .clone()
                    .oneshot(
                        request
                            .body(Body::from(json!({"sources":[source]}).to_string()))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                let status = response.status();
                assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
                assert!(
                    response
                        .headers()
                        .get(caller_scope::UPDATED_CREDENTIAL_HEADER)
                        .is_none()
                );
                let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
                let result: Value = serde_json::from_slice(&bytes).unwrap();
                let (_, directory) = json_response_from(router.clone(), "/v1/uni/playlists").await;
                if mode != "stable" {
                    assert_ne!(status, StatusCode::OK, "{result}");
                    assert_eq!(directory["data"], json!([]));
                    assert_eq!(
                        result["error"]["code"],
                        if mode == "auth" {
                            "authentication_required"
                        } else {
                            "upstream_error"
                        }
                    );
                    continue;
                }
                assert_eq!(status, StatusCode::OK, "{result}");
                let items = if materialize {
                    assert_eq!(directory["data"], json!([]));
                    result["data"]["items"].clone()
                } else {
                    let reference = result["data"]["playlist"]["ref"].as_str().unwrap();
                    let (_, p) = json_response_from(
                        router.clone(),
                        &format!("/v1/uni/playlists/{reference}/items"),
                    )
                    .await;
                    p["data"].clone()
                };
                assert_eq!(items[0]["source_ref"], "kuwo:11");
                assert_eq!(items[1]["source_ref"], "kuwo:22");
                assert_eq!(items[2]["source_ref"], "kuwo:22");
                assert_ne!(items[1]["id"], items[2]["id"]);
                for secret in ["private-native-fixture", "twc1_", "personal"] {
                    assert!(!items.to_string().contains(secret));
                }
            }
        }
    }
}

#[tokio::test]
async fn kuwo_favorite_real_provider_no_credential_never_reaches_network_and_errors_are_private() {
    let guard = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    guard.set_nonblocking(true).unwrap();
    let provider =
        tuneweave_provider_kuwo::KuwoProvider::new(tuneweave_provider_kuwo::KuwoConfig {
            proxy_url: Some(format!("http://{}", guard.local_addr().unwrap())),
            ..Default::default()
        })
        .unwrap();
    let mut registry = ProviderRegistry::new();
    registry.register(provider).unwrap();
    let router = build_router(AppState::new(registry, Platform::Kuwo));
    for path in [
        "/v1/account/favorites/playlist",
        "/v1/account/favorites/tracks",
        "/v1/users/kuwo:42/favorites/playlist",
        "/v1/users/kuwo:42/favorites/tracks",
    ] {
        let (status, headers, value) =
            json_request_with_headers(router.clone(), Method::GET, path, None).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{value}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    }
    assert_eq!(
        guard.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
