//! HTTP contracts for Soda's authenticated artist detail and overview reads.
use super::*;

type Calls = Arc<Mutex<Vec<(String, Option<String>)>>>;

#[derive(Clone)]
struct Provider {
    caller: bool,
    failure: Option<ErrorCode>,
    calls: Calls,
    update: Arc<Mutex<Option<ProviderCredential>>>,
}

impl Provider {
    fn account(&self, account: Option<&str>) {
        assert!(matches!(account, Some("default" | "personal")));
        if self.caller {
            assert_eq!(account, Some("default"));
        }
    }

    fn artist(&self) -> Artist {
        let mut artist = sample_artist("123");
        artist.platform = Platform::Soda;
        artist.resource_ref = ResourceRef::new(Platform::Soda, "123").unwrap();
        artist.name = "Soda Artist".to_owned();
        artist.track_count = Some(3);
        artist.album_count = Some(7);
        artist.extensions = Extensions::from([
            ("backend".into(), json!("official_pc_account_artist_detail")),
            ("source_user_id".into(), json!("123456")),
            ("authenticated".into(), json!(true)),
            ("linked_user_id".into(), json!("456")),
            (
                "account_state".into(),
                json!({"is_collected": true, "blocked_by_me": false}),
            ),
        ]);
        artist
    }

    fn overview(&self) -> ArtistOverview {
        let mut track = sample_track("11");
        track.platform = Platform::Soda;
        track.resource_ref = ResourceRef::new(Platform::Soda, "11").unwrap();
        track.extensions = Extensions::from([
            (
                "backend".into(),
                json!("official_pc_account_artist_preview"),
            ),
            ("source_user_id".into(), json!("123456")),
            ("authenticated".into(), json!(true)),
        ]);
        ArtistOverview {
            artist: self.artist(),
            featured_tracks: vec![track],
            has_more_tracks: true,
            extensions: Extensions::from([
                ("backend".into(), json!("official_pc_account_artist_detail")),
                ("preview_scope".into(), json!("hot_tracks")),
            ]),
        }
    }

    fn complete(&self, kind: &str, account: Option<&str>) -> Result<()> {
        self.account(account);
        self.calls
            .lock()
            .unwrap()
            .push((kind.to_owned(), account.map(str::to_owned)));
        if let Some(code) = self.failure {
            return Err(
                TuneWeaveError::new(code, "Soda account artist detail failed")
                    .with_platform(Platform::Soda),
            );
        }
        if self.caller {
            *self.update.lock().unwrap() = Some(
                ProviderCredential::new(
                    Platform::Soda,
                    "fixture",
                    "artist-detail-rotated-secret",
                    None,
                )
                .unwrap(),
            );
        }
        Ok(())
    }
}

#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Soda
    }

    fn name(&self) -> &'static str {
        "Soda account artist detail contract"
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::ArtistDetail,
            Capability::ArtistOverview,
            Capability::CallerManagedCredentials,
        ])
    }

    fn with_caller_credential(
        &self,
        credential: &ProviderCredential,
    ) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(credential.secret(), "artist-detail-input-secret");
        Ok(Arc::new(Self {
            caller: true,
            ..self.clone()
        }))
    }

    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        Ok(self.update.lock().unwrap().take())
    }

    async fn artist(&self, id: &str, account: Option<&str>) -> Result<Artist> {
        assert_eq!(id, "123");
        self.complete("artist", account)?;
        Ok(self.artist())
    }

    async fn artist_overview(&self, id: &str, account: Option<&str>) -> Result<ArtistOverview> {
        assert_eq!(id, "123");
        self.complete("overview", account)?;
        Ok(self.overview())
    }
}

fn app(failure: Option<ErrorCode>) -> (Router, Calls) {
    let calls = Arc::default();
    let mut registry = ProviderRegistry::new();
    registry
        .register(Provider {
            caller: false,
            failure,
            calls: Arc::clone(&calls),
            update: Arc::default(),
        })
        .unwrap();
    (build_router(AppState::new(registry, Platform::Soda)), calls)
}

fn request(uri: &str, caller: bool) -> Request<Body> {
    let mut request = Request::builder().uri(uri);
    if caller {
        request = request.header(
            CALLER_CREDENTIAL_HEADER,
            CallerCredential::issue(
                &ProviderCredential::new(
                    Platform::Soda,
                    "fixture",
                    "artist-detail-input-secret",
                    None,
                )
                .unwrap(),
            )
            .unwrap()
            .value,
        );
    }
    request.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn soda_account_artist_detail_http_three_owners_keep_scope_preview_and_rotations() {
    for owner in ["default", "personal", "caller"] {
        for endpoint in ["", "/overview"] {
            let (router, calls) = app(None);
            let mut path = format!("/v1/artists/soda:123{endpoint}");
            if owner != "caller" {
                path.push_str(&format!("?account={owner}"));
            }
            let response = router
                .oneshot(request(&path, owner == "caller"))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK, "{owner}{endpoint}");
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
                owner == "caller"
            );
            let rotated = response
                .headers()
                .get_all(caller_scope::UPDATED_CREDENTIAL_HEADER)
                .iter()
                .map(|value| value.to_str().unwrap().to_owned())
                .collect::<Vec<_>>();
            if owner == "caller" {
                assert_eq!(rotated.len(), 1);
                let value = rotated[0].strip_prefix("soda=").unwrap();
                assert_eq!(
                    CallerCredential::parse(value).unwrap().secret(),
                    "artist-detail-rotated-secret"
                );
            }
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            let raw = String::from_utf8_lossy(&bytes);
            for secret in ["artist-detail-input-secret", "artist-detail-rotated-secret"] {
                assert!(!raw.contains(secret));
            }
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            let artist = if endpoint.is_empty() {
                &value["data"]
            } else {
                &value["data"]["artist"]
            };
            assert_eq!(artist["ref"], "soda:123");
            assert_eq!(artist["id"], "123");
            assert_eq!(artist["extensions"]["source_user_id"], "123456");
            assert_eq!(artist["extensions"]["linked_user_id"], "456");
            assert_eq!(artist["extensions"]["account_state"]["is_collected"], true);
            if endpoint == "/overview" {
                assert_eq!(
                    value["data"]["featured_tracks"].as_array().unwrap().len(),
                    1
                );
                assert_eq!(value["data"]["featured_tracks"][0]["ref"], "soda:11");
                assert_eq!(value["data"]["has_more_tracks"], true);
                assert_eq!(value["data"]["extensions"]["preview_scope"], "hot_tracks");
                assert_eq!(
                    value["meta"]["account"],
                    if owner == "caller" {
                        Value::Null
                    } else {
                        json!(owner)
                    }
                );
            } else if owner != "caller" {
                assert_eq!(value["meta"]["account"], owner);
            } else {
                assert!(value["meta"].get("account").is_none());
            }
            assert_eq!(calls.lock().unwrap().len(), 1);
        }
    }
}

#[tokio::test]
async fn soda_account_artist_detail_http_errors_do_not_export_partial_data_or_credentials() {
    for endpoint in ["", "/overview"] {
        for (code, status) in [
            (ErrorCode::AuthenticationRequired, StatusCode::UNAUTHORIZED),
            (ErrorCode::Conflict, StatusCode::CONFLICT),
            (ErrorCode::UpstreamError, StatusCode::BAD_GATEWAY),
            (ErrorCode::UpstreamTimeout, StatusCode::GATEWAY_TIMEOUT),
        ] {
            let (router, _) = app(Some(code));
            let response = router
                .oneshot(request(
                    &format!("/v1/artists/soda:123{endpoint}?account=personal"),
                    false,
                ))
                .await
                .unwrap();
            assert_eq!(response.status(), status, "{endpoint}/{code:?}");
            assert!(
                response.headers()[header::CACHE_CONTROL]
                    .to_str()
                    .unwrap()
                    .contains("no-store")
            );
            assert!(
                !response
                    .headers()
                    .contains_key(caller_scope::UPDATED_CREDENTIAL_HEADER)
            );
            let value: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                    .unwrap();
            assert!(value.get("data").is_none_or(Value::is_null));
        }
    }
}

#[tokio::test]
async fn soda_account_artist_detail_http_rejects_mixed_caller_scope_and_invalid_ids_before_io() {
    for (path, caller, status) in [
        (
            "/v1/artists/soda:123?account=personal",
            true,
            StatusCode::BAD_REQUEST,
        ),
        (
            "/v1/artists/soda:0123?account=personal",
            false,
            StatusCode::BAD_REQUEST,
        ),
        (
            "/v1/artists/soda:0/overview?account=personal",
            false,
            StatusCode::BAD_REQUEST,
        ),
        (
            "/v1/artists/soda:123/overview?account=missing",
            false,
            StatusCode::UNAUTHORIZED,
        ),
    ] {
        let (router, calls) = if caller {
            let (router, calls) = app(None);
            (router, Some(calls))
        } else {
            let mut registry = ProviderRegistry::new();
            registry
                .register(
                    tuneweave_provider_soda::SodaProvider::new(
                        tuneweave_provider_soda::SodaConfig {
                            proxy_url: Some("http://127.0.0.1:9".into()),
                            ..Default::default()
                        },
                    )
                    .unwrap(),
                )
                .unwrap();
            (build_router(AppState::new(registry, Platform::Soda)), None)
        };
        let response = router.oneshot(request(path, caller)).await.unwrap();
        assert_eq!(response.status(), status, "{path}");
        if let Some(calls) = calls {
            assert!(calls.lock().unwrap().is_empty());
        }
        assert!(
            response.headers()[header::CACHE_CONTROL]
                .to_str()
                .unwrap()
                .contains("no-store")
        );
    }
}
