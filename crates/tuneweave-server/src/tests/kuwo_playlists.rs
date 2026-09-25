//! HTTP/Uni contract fixtures. Native account protocol is tested in the provider.
use super::*;

#[derive(Clone)]
struct Provider {
    caller: bool,
    fail: &'static str,
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Kuwo
    }
    fn name(&self) -> &'static str {
        "Kuwo playlist contract fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::PlaylistRead,
            Capability::CallerManagedCredentials,
        ])
    }
    fn with_caller_credential(
        &self,
        credential: &ProviderCredential,
    ) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(credential.platform, Platform::Kuwo);
        assert_eq!(credential.secret(), "private-native-fixture");
        Ok(Arc::new(Self {
            caller: true,
            ..self.clone()
        }))
    }
    async fn playlist(&self, id: &str, account: Option<&str>) -> Result<Playlist> {
        assert!(matches!(id, "101" | "202"));
        assert_eq!(
            account,
            Some(if self.caller { "default" } else { "personal" })
        );
        let mut playlist = test_kuwo_playlist();
        playlist.id = id.into();
        playlist.resource_ref = ResourceRef::new(Platform::Kuwo, id).unwrap();
        playlist.name = if id == "202" {
            "收藏歌单"
        } else {
            "私人自建歌单"
        }
        .into();
        playlist.subscribed = (id == "202").then_some(true);
        playlist.track_count = Some(3);
        playlist.creator = None;
        playlist.extensions =
            Extensions::from([("source_snapshot_id".into(), json!("revision-a"))]);
        playlist.tags.clear();
        if id == "101" {
            if self.fail != "empty_tags" {
                playlist.tags = vec!["流行".into(), "安静".into()];
            }
            playlist
                .extensions
                .insert("editable_metadata_verified".into(), json!(true));
        }
        // This must never be copied wholesale into Uni source metadata.
        playlist.extensions.insert(
            "private_token".into(),
            json!("never-copy-provider-extension"),
        );
        Ok(playlist)
    }
    async fn playlist_tracks(&self, id: &str, request: &PageRequest) -> Result<Page<Track>> {
        assert!(matches!(id, "101" | "202"));
        assert_eq!(
            request.account.as_deref(),
            Some(if self.caller { "default" } else { "personal" })
        );
        if request.offset == 2 && self.fail == "auth" {
            return Err(
                TuneWeaveError::new(ErrorCode::AuthenticationRequired, "expired")
                    .with_platform(Platform::Kuwo),
            );
        }
        let (ids, next) = match request.offset {
            0 => (vec![11, 22], Some(2)),
            2 => (vec![22], None),
            _ => panic!("unexpected window"),
        };
        let version = if request.offset == 2 && matches!(self.fail, "changed" | "tags_changed") {
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
                limit: request.limit,
                offset: request.offset,
                total: Some(3),
                has_more: next.is_some(),
                next_offset: next,
                extensions: Extensions::from([("source_snapshot_id".into(), json!(version))]),
            },
        })
    }
}
fn app(fail: &'static str) -> Router {
    let mut registry = ProviderRegistry::new();
    registry
        .register(Provider {
            caller: false,
            fail,
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
async fn kuwo_account_playlist_http_preserves_account_scope_pagination_and_private_responses() {
    for id in ["101", "202"] {
        for caller in [false, true] {
            for tracks in [false, true] {
                let path = format!(
                    "/v1/playlists/kuwo:{id}{}{}",
                    if tracks { "/tracks" } else { "" },
                    if caller { "" } else { "?account=personal" }
                );
                let mut request = Request::builder().uri(path);
                if caller {
                    request = request.header(CALLER_CREDENTIAL_HEADER, credential());
                }
                let response = app("stable")
                    .oneshot(request.body(Body::empty()).unwrap())
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
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
                if tracks {
                    assert_eq!(value["data"][0]["ref"], "kuwo:11");
                    assert_eq!(value["meta"]["pagination"]["total"], 3);
                } else {
                    assert_eq!(
                        value["data"]["name"],
                        if id == "202" {
                            "收藏歌单"
                        } else {
                            "私人自建歌单"
                        }
                    );
                    if id == "202" {
                        assert_eq!(value["data"]["subscribed"], true);
                    } else {
                        assert_eq!(value["data"]["tags"], json!(["流行", "安静"]));
                        assert_eq!(
                            value["data"]["extensions"]["editable_metadata_verified"],
                            true
                        );
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn kuwo_account_playlist_uni_import_keeps_duplicates_and_rejects_changed_or_expired_sources()
{
    for id in ["101", "202"] {
        for caller in [false, true] {
            for materialize in [false, true] {
                for mode in ["stable", "empty_tags", "changed", "tags_changed", "auth"] {
                    let app = app(mode);
                    let mut source = json!({"ref":format!("kuwo:{id}"),"type":"playlist"});
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
                    let response = app
                        .clone()
                        .oneshot(
                            request
                                .body(Body::from(json!({"sources":[source]}).to_string()))
                                .unwrap(),
                        )
                        .await
                        .unwrap();
                    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
                    assert!(
                        response
                            .headers()
                            .get(caller_scope::UPDATED_CREDENTIAL_HEADER)
                            .is_none()
                    );
                    let status = response.status();
                    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
                    let result: Value = serde_json::from_slice(&bytes).unwrap();
                    let (_, directory) = json_response_from(app.clone(), "/v1/uni/playlists").await;
                    if !matches!(mode, "stable" | "empty_tags") {
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
                    let tags = if mode == "empty_tags" {
                        json!([])
                    } else {
                        json!(["流行", "安静"])
                    };
                    let source = &result["data"]["sources"][0]["extensions"];
                    if id == "101" {
                        assert_eq!(source["source_tags"], tags);
                        assert_eq!(source["source_metadata_verified"], true);
                    } else {
                        assert!(source.get("source_tags").is_none());
                        assert!(source.get("source_metadata_verified").is_none());
                    }
                    assert!(!result.to_string().contains("never-copy-provider-extension"));
                    if !materialize && id == "101" {
                        assert_eq!(
                            result["data"]["playlist"]["extensions"]["import_sources"][0]["extensions"]
                                ["source_tags"],
                            tags
                        );
                    }
                    if materialize {
                        assert!(!result["data"].to_string().contains("personal"));
                    }
                    let items = if materialize {
                        assert_eq!(directory["data"], json!([]));
                        assert_eq!(result["data"]["item_count"], 3);
                        result["data"]["items"].clone()
                    } else {
                        let reference = result["data"]["playlist"]["ref"].as_str().unwrap();
                        let (_, items) = json_response_from(
                            app.clone(),
                            &format!("/v1/uni/playlists/{reference}/items"),
                        )
                        .await;
                        items["data"].clone()
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
}
