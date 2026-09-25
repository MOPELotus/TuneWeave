//! HTTP contract fixtures; the Kuwo crate tests the actual protocol and account races.
use super::*;
use tuneweave_core::{PlaylistSubmission, PlaylistSubmissionStatus};

#[derive(Clone)]
struct Provider {
    account: &'static str,
    failure: Option<ErrorCode>,
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Kuwo
    }
    fn name(&self) -> &'static str {
        "Kuwo submission contract"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::AccountPlaylistSubmissions,
            Capability::PlaylistSubmissionWrite,
            Capability::PlaylistSubmissionRecordDelete,
            Capability::CallerManagedCredentials,
        ])
    }
    fn with_caller_credential(&self, c: &ProviderCredential) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(c.secret(), "private-submission-fixture");
        Ok(Arc::new(Self {
            account: "default",
            ..self.clone()
        }))
    }
    async fn account_playlist_submissions(
        &self,
        r: &PageRequest,
    ) -> Result<Page<PlaylistSubmission>> {
        assert_eq!(r.account.as_deref(), Some(self.account));
        assert_eq!((r.limit, r.offset), (2, 5));
        if let Some(code) = self.failure {
            return Err(
                TuneWeaveError::new(code, "Submission read failed").with_platform(Platform::Kuwo)
            );
        }
        Ok(Page {
            items: vec![PlaylistSubmission {
                playlist_ref: ResourceRef::new(Platform::Kuwo, "101").unwrap(),
                owner_id: "42".into(),
                name: Some("Pending publication".into()),
                cover_url: None,
                track_count: Some(12),
                play_count: None,
                review_status: PlaylistSubmissionStatus::Approved,
                published: None,
                extensions: Extensions::default(),
            }],
            pagination: PageMeta {
                limit: 2,
                offset: 5,
                total: Some(6),
                has_more: false,
                next_offset: None,
                extensions: Extensions::from([("complete_read".into(), json!(true))]),
            },
        })
    }

    async fn delete_playlist_submission_records(
        &self,
        id: &str,
        r: &tuneweave_core::PlaylistSubmissionRecordDeleteRequest,
    ) -> Result<tuneweave_core::PlaylistSubmissionRecordDeleteResult> {
        assert_eq!(id, "101");
        assert_eq!(r.account.as_deref(), Some(self.account));
        if let Some(code) = self.failure {
            return Err(TuneWeaveError::new(code,"Record deletion readback failed").with_platform(Platform::Kuwo)
                .retryable(false).with_details(json!({"record_delete_outcome":"confirmed","write_outcome":"partial","write_requests_dispatched":1,"automatic_retry":false})));
        }
        Ok(tuneweave_core::PlaylistSubmissionRecordDeleteResult {
            playlist_ref: ResourceRef::new(Platform::Kuwo, id).unwrap(),
            confirmed: true,
            changed: true,
            removed_records: 2,
            owned_playlist_present: false,
            playlist: None,
            published: None,
            extensions: Extensions::from([(
                "playlist_observation_scope".into(),
                json!("ordinary_created_directory"),
            )]),
        })
    }

    async fn submit_playlist(
        &self,
        id: &str,
        r: &tuneweave_core::PlaylistSubmissionRequest,
    ) -> Result<tuneweave_core::PlaylistSubmissionResult> {
        assert_eq!(id, "101");
        assert_eq!(r.account.as_deref(), Some(self.account));
        let edited = r.name.is_some() || r.recommendation.is_some();
        if r.name.is_some() {
            assert_eq!(r.name.as_deref(), Some("Revised playlist"));
            assert_eq!(r.description.as_deref(), Some("Updated description"));
            assert_eq!(r.tags, Some(vec!["calm".into()]));
        }
        if let Some(text) = &r.recommendation {
            assert_eq!(text, "好歌🎵");
        }
        if let Some(code) = self.failure {
            return Err(TuneWeaveError::new(code,"Submission readback failed").with_platform(Platform::Kuwo).retryable(false).with_details(json!({"write_requests_dispatched":if edited{2}else{1},"submission_outcome":"accepted","playlist_write_outcome":if edited{"confirmed"}else{"not_dispatched"},"write_outcome":"partial","automatic_retry":false})));
        }
        let reference = ResourceRef::new(Platform::Kuwo, id).unwrap();
        Ok(tuneweave_core::PlaylistSubmissionResult {
            playlist_ref: reference.clone(),
            accepted: true,
            metadata_updated: edited,
            published: Some(false),
            playlist: Playlist {
                resource_ref: reference,
                platform: Platform::Kuwo,
                id: id.into(),
                name: r.name.clone().unwrap_or_else(|| "Saved playlist".into()),
                description: "Description".into(),
                cover_url: None,
                creator: None,
                track_count: Some(10),
                tags: vec![],
                subscribed: None,
                created_at: None,
                updated_at: None,
                extensions: Extensions::default(),
            },
            records: vec![],
            extensions: Extensions::from([
                (
                    "write_requests_dispatched".into(),
                    json!(if edited { 2 } else { 1 }),
                ),
                ("records_correlated_to_request".into(), json!(false)),
                (
                    "recommendation_submitted".into(),
                    json!(r.recommendation.is_some()),
                ),
            ]),
        })
    }
}

fn app(scope: &str, failure: Option<ErrorCode>) -> Router {
    let mut registry = ProviderRegistry::new();
    registry
        .register(Provider {
            account: if scope == "named" {
                "personal"
            } else {
                "default"
            },
            failure,
        })
        .unwrap();
    build_router(AppState::new(registry, Platform::Kuwo))
}
fn request(scope: &str) -> Request<Body> {
    let path = format!(
        "/v1/account/playlist-submissions?platform=kuwo&limit=2&offset=5{}",
        if scope == "named" {
            "&account=personal"
        } else {
            ""
        }
    );
    let mut r = Request::builder().uri(path);
    if scope == "caller" {
        let c = CallerCredential::issue(
            &ProviderCredential::new(
                Platform::Kuwo,
                "fixture",
                "private-submission-fixture",
                None,
            )
            .unwrap(),
        )
        .unwrap();
        r = r.header(CALLER_CREDENTIAL_HEADER, c.value);
    }
    r.body(Body::empty()).unwrap()
}

fn submit_request(scope: &str, edit: bool) -> Request<Body> {
    let mut r = request(scope);
    *r.method_mut() = Method::POST;
    *r.uri_mut() = "/v1/playlists/kuwo:101/submission".parse().unwrap();
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    let mut body = if edit {
        json!({"name":"Revised playlist","description":"Updated description","tags":["calm"]})
    } else {
        json!({})
    };
    if scope == "named" {
        body["account"] = json!("personal");
    }
    *r.body_mut() = Body::from(body.to_string());
    r
}

#[tokio::test]
async fn kuwo_contribution_http_keeps_submission_receipt_and_partial_failures_separate_from_publication()
 {
    for scope in ["default", "named", "caller"] {
        for edit in [false, true] {
            for failure in [
                None,
                Some(ErrorCode::AuthenticationRequired),
                Some(ErrorCode::Conflict),
                Some(ErrorCode::UpstreamError),
                Some(ErrorCode::RateLimited),
            ] {
                let response = app(scope, failure)
                    .oneshot(submit_request(scope, edit))
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    match failure {
                        None => StatusCode::OK,
                        Some(ErrorCode::AuthenticationRequired) => StatusCode::UNAUTHORIZED,
                        Some(ErrorCode::Conflict) => StatusCode::CONFLICT,
                        Some(ErrorCode::RateLimited) => StatusCode::TOO_MANY_REQUESTS,
                        _ => StatusCode::BAD_GATEWAY,
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
                assert!(!String::from_utf8_lossy(&bytes).contains("private-submission-fixture"));
                let v: Value = serde_json::from_slice(&bytes).unwrap();
                if let Some(code) = failure {
                    assert_eq!(v["error"]["code"], code.as_str());
                    assert_eq!(v["error"]["details"]["submission_outcome"], "accepted");
                    assert_eq!(
                        v["error"]["details"]["write_requests_dispatched"],
                        if edit { 2 } else { 1 }
                    );
                } else {
                    assert_eq!(v["data"]["accepted"], true);
                    assert_eq!(v["data"]["metadata_updated"], edit);
                    assert_eq!(v["data"]["published"], false);
                    assert_eq!(v["data"]["records"], json!([]));
                }
            }
        }
    }
}

#[tokio::test]
async fn kuwo_contribution_http_rejects_ambiguous_payloads_and_marks_early_errors_private() {
    for (body, path) in [
        ("{", "/v1/playlists/kuwo:101/submission"),
        (
            r#"{"visibility":"public"}"#,
            "/v1/playlists/kuwo:101/submission",
        ),
        (r#"{"withdraw":true}"#, "/v1/playlists/kuwo:101/submission"),
        (r#"{"tags":"calm"}"#, "/v1/playlists/kuwo:101/submission"),
        (
            r#"{"name":"a","name":"b"}"#,
            "/v1/playlists/kuwo:101/submission",
        ),
        ("{}", "/v1/playlists/kuwo:101/submission?account=personal"),
    ] {
        let mut r = submit_request("default", false);
        *r.body_mut() = Body::from(body);
        *r.uri_mut() = path.parse().unwrap();
        let response = app("default", None).oneshot(r).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    for method in [Method::GET, Method::PUT, Method::DELETE] {
        let mut r = submit_request("default", false);
        *r.method_mut() = method;
        let response = app("default", None).oneshot(r).await.unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    let mut r = submit_request("default", false);
    r.headers_mut()
        .insert("x-request-id", HeaderValue::from_static("bad request id"));
    let response = app("default", None).oneshot(r).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let mut r = submit_request("caller", false);
    *r.body_mut() = Body::from(r#"{"account":"personal"}"#);
    let response = app("caller", None).oneshot(r).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn kuwo_contribution_http_real_provider_requires_credentials_and_other_providers_do_not_fall_back()
 {
    let guard = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    guard.set_nonblocking(true).unwrap();
    let p = tuneweave_provider_kuwo::KuwoProvider::new(tuneweave_provider_kuwo::KuwoConfig {
        proxy_url: Some(format!("http://{}", guard.local_addr().unwrap())),
        ..Default::default()
    })
    .unwrap();
    assert!(p.supports(Capability::PlaylistSubmissionWrite));
    let mut registry = ProviderRegistry::new();
    registry.register(p).unwrap();
    let response = build_router(AppState::new(registry, Platform::Kuwo))
        .oneshot(submit_request("default", false))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        guard.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    struct Unsupported;
    #[async_trait]
    impl MusicProvider for Unsupported {
        fn platform(&self) -> Platform {
            Platform::Soda
        }
        fn name(&self) -> &'static str {
            "No submission protocol"
        }
        fn capabilities(&self) -> BTreeSet<Capability> {
            BTreeSet::from([Capability::PlaylistWrite])
        }
        async fn update_playlist(
            &self,
            _: &str,
            _: &PlaylistUpdateRequest,
        ) -> Result<PlaylistMutationResult> {
            panic!("must not replace submission with ordinary metadata update")
        }
    }
    let mut registry = ProviderRegistry::new();
    registry.register(Unsupported).unwrap();
    let mut r = submit_request("default", true);
    *r.uri_mut() = "/v1/playlists/soda:101/submission".parse().unwrap();
    let response = build_router(AppState::new(registry, Platform::Soda))
        .oneshot(r)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn kuwo_submissions_http_preserves_selected_source_review_and_errors_privately() {
    for scope in ["default", "named", "caller"] {
        for failure in [
            None,
            Some(ErrorCode::AuthenticationRequired),
            Some(ErrorCode::Conflict),
            Some(ErrorCode::UpstreamError),
            Some(ErrorCode::RateLimited),
        ] {
            let response = app(scope, failure).oneshot(request(scope)).await.unwrap();
            assert_eq!(
                response.status(),
                match failure {
                    None => StatusCode::OK,
                    Some(ErrorCode::AuthenticationRequired) => StatusCode::UNAUTHORIZED,
                    Some(ErrorCode::Conflict) => StatusCode::CONFLICT,
                    Some(ErrorCode::RateLimited) => StatusCode::TOO_MANY_REQUESTS,
                    _ => StatusCode::BAD_GATEWAY,
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
            assert!(!String::from_utf8_lossy(&bytes).contains("private-submission-fixture"));
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            if let Some(code) = failure {
                assert_eq!(value["error"]["code"], code.as_str());
                assert!(value.get("data").is_none_or(Value::is_null));
            } else {
                assert_eq!(value["data"][0]["review_status"], "approved");
                assert!(value["data"][0]["published"].is_null());
                assert_eq!(value["data"][0]["playlist_ref"], "kuwo:101");
                assert_eq!(value["meta"]["pagination"]["total"], 6);
            }
        }
    }
}

#[tokio::test]
async fn kuwo_submissions_http_early_errors_and_wrong_methods_are_not_cached() {
    for query in [
        "?limit=0",
        "?limit=101",
        "?limit=no",
        "?offset=-1",
        "?limit=2&offset=4294967295",
        "?uid=42",
        "?account=A&account=B",
        "?limit=2&limit=2",
    ] {
        let r = Request::builder()
            .uri(format!("/v1/account/playlist-submissions{query}"))
            .body(Body::empty())
            .unwrap();
        let response = app("default", None).oneshot(r).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{query}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    // Empty account follows the shared selector's default-account semantics.
    let mut empty_account = request("default");
    *empty_account.uri_mut() = format!("{}&account=", empty_account.uri()).parse().unwrap();
    let response = app("default", None).oneshot(empty_account).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    for method in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
        let r = Request::builder()
            .method(method)
            .uri("/v1/account/playlist-submissions")
            .body(Body::empty())
            .unwrap();
        let response = app("default", None).oneshot(r).await.unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    let mut r = request("default");
    r.headers_mut()
        .insert("x-request-id", HeaderValue::from_static("bad request id"));
    let response = app("default", None).oneshot(r).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let mut r = request("caller");
    *r.uri_mut() = format!("{}&account=personal", r.uri()).parse().unwrap();
    let response = app("caller", None).oneshot(r).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
}

#[tokio::test]
async fn kuwo_submissions_real_provider_missing_credentials_and_other_platform_defaults_never_fallback()
 {
    let guard = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    guard.set_nonblocking(true).unwrap();
    let p = tuneweave_provider_kuwo::KuwoProvider::new(tuneweave_provider_kuwo::KuwoConfig {
        proxy_url: Some(format!("http://{}", guard.local_addr().unwrap())),
        ..Default::default()
    })
    .unwrap();
    assert!(p.supports(Capability::AccountPlaylistSubmissions));
    let mut registry = ProviderRegistry::new();
    registry.register(p).unwrap();
    let response = build_router(AppState::new(registry, Platform::Kuwo))
        .oneshot(request("default"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(
        guard.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    struct Unsupported;
    #[async_trait]
    impl MusicProvider for Unsupported {
        fn platform(&self) -> Platform {
            Platform::Soda
        }
        fn name(&self) -> &'static str {
            "ordinary playlist only"
        }
        fn capabilities(&self) -> BTreeSet<Capability> {
            BTreeSet::from([Capability::AccountPlaylists])
        }
        async fn account_playlists(&self, _: &PageRequest) -> Result<Page<Playlist>> {
            panic!("submissions must not fall back to ordinary playlists")
        }
    }
    let mut registry = ProviderRegistry::new();
    registry.register(Unsupported).unwrap();
    let response = build_router(AppState::new(registry, Platform::Soda))
        .oneshot(
            Request::builder()
                .uri("/v1/account/playlist-submissions?platform=soda")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    assert!(String::from_utf8_lossy(&bytes).contains("account_playlist_submissions"));
}

fn delete_records_request(scope: &str) -> Request<Body> {
    let mut r = request(scope);
    *r.method_mut() = Method::DELETE;
    *r.uri_mut() = format!(
        "/v1/account/playlist-submissions/kuwo:101{}",
        if scope == "named" {
            "?account=personal"
        } else {
            ""
        }
    )
    .parse()
    .unwrap();
    r
}

#[tokio::test]
async fn kuwo_submission_delete_http_preserves_source_counts_observations_and_partial_errors() {
    for scope in ["default", "named", "caller"] {
        for failure in [
            None,
            Some(ErrorCode::AuthenticationRequired),
            Some(ErrorCode::Conflict),
            Some(ErrorCode::UpstreamError),
            Some(ErrorCode::RateLimited),
        ] {
            let response = app(scope, failure)
                .oneshot(delete_records_request(scope))
                .await
                .unwrap();
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert!(
                !response
                    .headers()
                    .contains_key("x-tuneweave-updated-credential")
            );
            assert_eq!(
                response.status(),
                match failure {
                    None => StatusCode::OK,
                    Some(ErrorCode::AuthenticationRequired) => StatusCode::UNAUTHORIZED,
                    Some(ErrorCode::Conflict) => StatusCode::CONFLICT,
                    Some(ErrorCode::RateLimited) => StatusCode::TOO_MANY_REQUESTS,
                    _ => StatusCode::BAD_GATEWAY,
                }
            );
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            let v: Value = serde_json::from_slice(&bytes).unwrap();
            assert!(!String::from_utf8_lossy(&bytes).contains("private-submission-fixture"));
            if failure.is_none() {
                assert_eq!(v["data"]["confirmed"], true);
                assert_eq!(v["data"]["removed_records"], 2);
                assert_eq!(v["data"]["owned_playlist_present"], false);
                assert!(v["data"]["published"].is_null());
                assert!(v["data"].get("withdrawn").is_none());
            } else {
                assert_eq!(v["error"]["details"]["record_delete_outcome"], "confirmed");
                assert_eq!(v["error"]["details"]["write_outcome"], "partial");
                assert_eq!(v["error"]["retryable"], false);
            }
        }
    }
}

#[tokio::test]
async fn kuwo_submission_delete_http_marks_early_errors_private_and_rejects_ambiguous_operations() {
    for query in [
        "?withdraw=true",
        "?account=A&account=B",
        "?platform=kuwo",
        "?ids=101,102",
    ] {
        let mut r = delete_records_request("default");
        *r.uri_mut() = format!("/v1/account/playlist-submissions/kuwo:101{query}")
            .parse()
            .unwrap();
        let response = app("default", None).oneshot(r).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    for method in [Method::GET, Method::POST, Method::PUT] {
        let mut r = delete_records_request("default");
        *r.method_mut() = method;
        let response = app("default", None).oneshot(r).await.unwrap();
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
    for change in ["request-id", "caller-alias", "body", "ref"] {
        let mut r = delete_records_request(if change == "caller-alias" {
            "caller"
        } else {
            "default"
        });
        match change {
            "request-id" => {
                r.headers_mut()
                    .insert("x-request-id", HeaderValue::from_static("bad request id"));
            }
            "caller-alias" => {
                *r.uri_mut() = "/v1/account/playlist-submissions/kuwo:101?account=personal"
                    .parse()
                    .unwrap();
            }
            "ref" => {
                *r.uri_mut() = "/v1/account/playlist-submissions/invalid".parse().unwrap();
            }
            _ => {
                *r.body_mut() = Body::from(r#"{"account":"personal"}"#);
            }
        }
        let response = app("default", None).oneshot(r).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{change}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }
}

#[tokio::test]
async fn kuwo_submission_delete_http_real_provider_requires_credentials_and_never_deletes_a_playlist_as_fallback()
 {
    let guard = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    guard.set_nonblocking(true).unwrap();
    let p = tuneweave_provider_kuwo::KuwoProvider::new(tuneweave_provider_kuwo::KuwoConfig {
        proxy_url: Some(format!("http://{}", guard.local_addr().unwrap())),
        ..Default::default()
    })
    .unwrap();
    assert!(p.supports(Capability::PlaylistSubmissionRecordDelete));
    let mut registry = ProviderRegistry::new();
    registry.register(p).unwrap();
    let response = build_router(AppState::new(registry, Platform::Kuwo))
        .oneshot(delete_records_request("default"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        guard.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
    struct Unsupported;
    #[async_trait]
    impl MusicProvider for Unsupported {
        fn platform(&self) -> Platform {
            Platform::Soda
        }
        fn name(&self) -> &'static str {
            "No history deletion"
        }
        fn capabilities(&self) -> BTreeSet<Capability> {
            BTreeSet::from([Capability::PlaylistWrite])
        }
        async fn delete_playlists(
            &self,
            _: &PlaylistDeleteRequest,
        ) -> Result<PlaylistDeleteResult> {
            panic!("must not delete original playlist as a record-deletion fallback")
        }
    }
    let mut registry = ProviderRegistry::new();
    registry.register(Unsupported).unwrap();
    let mut r = delete_records_request("default");
    *r.uri_mut() = "/v1/account/playlist-submissions/soda:101".parse().unwrap();
    let response = build_router(AppState::new(registry, Platform::Soda))
        .oneshot(r)
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
}

async fn recommendation_request(scope: &str, edit: bool) -> Request<Body> {
    let mut r = submit_request(scope, edit);
    let body = to_bytes(std::mem::replace(r.body_mut(), Body::empty()), 65536)
        .await
        .unwrap();
    let mut value: Value = serde_json::from_slice(&body).unwrap();
    value["recommendation"] = json!("好歌🎵");
    *r.body_mut() = Body::from(value.to_string());
    r
}

#[tokio::test]
async fn kuwo_recommendation_http_preserves_two_step_results_for_all_account_sources() {
    for scope in ["default", "named", "caller"] {
        for edit in [false, true] {
            for failure in [
                None,
                Some(ErrorCode::AuthenticationRequired),
                Some(ErrorCode::Conflict),
                Some(ErrorCode::PermissionDenied),
                Some(ErrorCode::UpstreamError),
                Some(ErrorCode::RateLimited),
            ] {
                let response = app(scope, failure)
                    .oneshot(recommendation_request(scope, edit).await)
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    match failure {
                        None => StatusCode::OK,
                        Some(ErrorCode::AuthenticationRequired) => StatusCode::UNAUTHORIZED,
                        Some(ErrorCode::Conflict) => StatusCode::CONFLICT,
                        Some(ErrorCode::PermissionDenied) => StatusCode::FORBIDDEN,
                        Some(ErrorCode::RateLimited) => StatusCode::TOO_MANY_REQUESTS,
                        _ => StatusCode::BAD_GATEWAY,
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
                assert!(!String::from_utf8_lossy(&bytes).contains("private-submission-fixture"));
                let value: Value = serde_json::from_slice(&bytes).unwrap();
                if let Some(code) = failure {
                    assert_eq!(value["error"]["code"], code.as_str());
                    let d = &value["error"]["details"];
                    assert_eq!(d["write_requests_dispatched"], 2);
                    assert_eq!(d["playlist_write_outcome"], "confirmed");
                    assert_eq!(d["submission_outcome"], "accepted");
                    assert_eq!(d["automatic_retry"], false);
                } else {
                    let d = &value["data"];
                    assert_eq!(d["accepted"], true);
                    assert_eq!(d["metadata_updated"], true);
                    assert_eq!(d["published"], false);
                    assert_eq!(d["extensions"]["recommendation_submitted"], true);
                    assert_eq!(d["extensions"]["write_requests_dispatched"], 2);
                }
            }
        }
    }
}

#[tokio::test]
async fn kuwo_recommendation_http_real_provider_validates_input_before_account_or_network_access() {
    let guard = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    guard.set_nonblocking(true).unwrap();
    let p = tuneweave_provider_kuwo::KuwoProvider::new(tuneweave_provider_kuwo::KuwoConfig {
        proxy_url: Some(format!("http://{}", guard.local_addr().unwrap())),
        ..Default::default()
    })
    .unwrap();
    let mut registry = ProviderRegistry::new();
    registry.register(p).unwrap();
    let app = build_router(AppState::new(registry, Platform::Kuwo));
    for (body, expected) in [
        (r#"{"recommendation":""}"#, StatusCode::BAD_REQUEST),
        (r#"{"recommendation":"hello!"}"#, StatusCode::BAD_REQUEST),
        (r#"{"recommendation":" a"}"#, StatusCode::BAD_REQUEST),
        (r#"{"recommendation":"😀😀😀"}"#, StatusCode::BAD_REQUEST),
        (r#"{"recommendation":7}"#, StatusCode::BAD_REQUEST),
        (
            r#"{"recommendation":"a","recommendation":"b"}"#,
            StatusCode::BAD_REQUEST,
        ),
        (r#"{"recommendation":"好歌🎵"}"#, StatusCode::UNAUTHORIZED),
        (r#"{"recommendation":null}"#, StatusCode::UNAUTHORIZED),
    ] {
        let mut r = submit_request("default", false);
        *r.body_mut() = Body::from(body);
        let response = app.clone().oneshot(r).await.unwrap();
        assert_eq!(response.status(), expected, "{body}");
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        let value: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            value["error"]["details"]
                .get("write_requests_dispatched")
                .is_none()
        );
    }
    assert_eq!(
        guard.accept().unwrap_err().kind(),
        std::io::ErrorKind::WouldBlock
    );
}
