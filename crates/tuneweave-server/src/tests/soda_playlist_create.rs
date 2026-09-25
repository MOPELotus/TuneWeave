use super::*;
use tuneweave_core::PlaylistMutationAction;

type PlaylistCreateSeen = Arc<Mutex<Vec<(String, PlaylistVisibility, Option<String>)>>>;

#[derive(Clone)]
struct PlaylistCreateProvider {
    seen: PlaylistCreateSeen,
}

#[async_trait]
impl MusicProvider for PlaylistCreateProvider {
    fn platform(&self) -> Platform {
        Platform::Soda
    }

    fn name(&self) -> &'static str {
        "Soda playlist creation contract"
    }

    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::PlaylistWrite,
            Capability::CallerManagedCredentials,
        ])
    }

    fn with_caller_credential(
        &self,
        credential: &ProviderCredential,
    ) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(credential.platform, Platform::Soda);
        assert_eq!(credential.secret(), "caller-session");
        Ok(Arc::new(self.clone()))
    }

    async fn create_playlist(
        &self,
        request: &PlaylistCreateRequest,
    ) -> Result<PlaylistMutationResult> {
        assert_eq!(request.kind, PlaylistKind::Normal);
        assert_eq!(request.account.as_deref(), Some("default"));
        self.seen.lock().unwrap().push((
            request.name.clone(),
            request.visibility,
            request.account.clone(),
        ));
        let playlist_ref = ResourceRef::new(Platform::Soda, "42").expect("valid Soda reference");
        Ok(PlaylistMutationResult {
            playlist_ref: playlist_ref.clone(),
            action: PlaylistMutationAction::Create,
            playlist: Some(Playlist {
                resource_ref: playlist_ref,
                platform: Platform::Soda,
                id: "42".into(),
                name: request.name.clone(),
                description: String::new(),
                cover_url: None,
                creator: None,
                track_count: Some(0),
                tags: Vec::new(),
                subscribed: None,
                created_at: None,
                updated_at: None,
                extensions: Extensions::from([("source_user_id".into(), json!("123456"))]),
            }),
            extensions: Extensions::from([
                (
                    "requested_visibility".into(),
                    json!(match request.visibility {
                        PlaylistVisibility::Public => "public",
                        PlaylistVisibility::Private => "private",
                        PlaylistVisibility::PlatformDefault => "platform_default",
                    }),
                ),
                ("visibility_verified".into(), json!(false)),
            ]),
        })
    }
}

#[tokio::test]
async fn soda_playlist_create_route_preserves_private_request_and_returns_no_store() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry
        .register(PlaylistCreateProvider {
            seen: Arc::clone(&seen),
        })
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Soda));
    let caller_credential = CallerCredential::issue(
        &ProviderCredential::new(Platform::Soda, "fixture", "caller-session", None).unwrap(),
    )
    .unwrap();

    let response = app
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/v1/playlists")
                .header(header::CONTENT_TYPE, "application/json")
                .header(CALLER_CREDENTIAL_HEADER, caller_credential.value)
                .body(Body::from(
                    json!({
                        "platform":"soda",
                        "name":"night mix",
                        "visibility":"private",
                        "kind":"normal"
                    })
                    .to_string(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("no-store")
    );
    let body: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65_536).await.unwrap()).unwrap();
    assert_eq!(body["data"]["action"], "create");
    assert_eq!(body["data"]["playlist_ref"], "soda:42");
    assert_eq!(
        body["data"]["extensions"]["requested_visibility"],
        "private"
    );
    assert_eq!(body["data"]["extensions"]["visibility_verified"], false);
    assert_eq!(body["meta"]["caller_credential"], Value::Null);
    assert_eq!(
        *seen.lock().unwrap(),
        vec![(
            "night mix".into(),
            PlaylistVisibility::Private,
            Some("default".into()),
        )]
    );
}
