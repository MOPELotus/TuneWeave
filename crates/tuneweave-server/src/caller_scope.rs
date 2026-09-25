use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    future::Future,
    sync::Arc,
};

use axum::{
    http::{HeaderValue, header},
    response::{IntoResponse, Response},
};
use tuneweave_core::{
    CallerCredential, ErrorCode, MusicProvider, Platform, ProviderCredential, Result,
    TuneWeaveError,
};

pub const UPDATED_CREDENTIAL_HEADER: &str = "x-tuneweave-updated-credential";

struct ProviderScope {
    input: ProviderCredential,
    provider: Arc<dyn MusicProvider>,
}

#[derive(Default)]
struct RequestScopes {
    providers: BTreeMap<Platform, ProviderScope>,
    updates: BTreeMap<Platform, CallerCredential>,
    invalidated: BTreeSet<Platform>,
    invalidate_all: bool,
    sensitive: bool,
}

tokio::task_local! {
    static SCOPES: RefCell<RequestScopes>;
}

pub(crate) fn mark_sensitive() {
    let _ = SCOPES.try_with(|scopes| scopes.borrow_mut().sensitive = true);
}

pub(crate) fn record_update(credential: &CallerCredential) {
    let _ = SCOPES.try_with(|scopes| {
        let mut scopes = scopes.borrow_mut();
        scopes.sensitive = true;
        scopes
            .updates
            .insert(credential.platform, credential.clone());
    });
}

pub(crate) fn invalidate(platform: Option<Platform>) {
    let _ = SCOPES.try_with(|scopes| {
        let mut scopes = scopes.borrow_mut();
        scopes.sensitive = true;
        if let Some(platform) = platform {
            scopes.invalidated.insert(platform);
        } else {
            scopes.invalidate_all = true;
        }
    });
}

pub(crate) fn provider_scope(
    base: Arc<dyn MusicProvider>,
    credential: &ProviderCredential,
    now: u64,
) -> Result<Arc<dyn MusicProvider>> {
    let cached = SCOPES
        .try_with(|scopes| {
            let scopes = scopes.borrow();
            scopes.providers.get(&credential.platform).map(|scope| {
                if scope.input != *credential {
                    return Err(TuneWeaveError::invalid_request(
                        "caller credential changed within one request",
                    )
                    .with_platform(credential.platform));
                }
                Ok(scope.provider.clone())
            })
        })
        .ok()
        .flatten();
    if let Some(cached) = cached {
        return cached;
    }
    if credential.is_expired_at(now) {
        return Err(TuneWeaveError::new(
            ErrorCode::AuthenticationRequired,
            "caller credential has expired",
        )
        .with_platform(credential.platform));
    }
    let provider = base.with_caller_credential(credential)?;
    if provider.platform() != credential.platform {
        return Err(contract_error(
            "caller provider scope belongs to a different platform",
        ));
    }
    let _ = SCOPES.try_with(|scopes| {
        let mut scopes = scopes.borrow_mut();
        scopes.sensitive = true;
        scopes.providers.insert(
            credential.platform,
            ProviderScope {
                input: credential.clone(),
                provider: provider.clone(),
            },
        );
    });
    Ok(provider)
}

fn contract_error(message: &str) -> TuneWeaveError {
    TuneWeaveError::new(ErrorCode::InternalError, message)
}

fn collect_updates() -> Result<Vec<CallerCredential>> {
    // Do not hold a RefCell borrow while calling a provider.
    let providers = SCOPES.with(|scopes| {
        let scopes = scopes.borrow();
        scopes
            .providers
            .iter()
            .filter(|(platform, _)| {
                !scopes.invalidate_all && !scopes.invalidated.contains(*platform)
            })
            .map(|(platform, scope)| (*platform, scope.provider.clone()))
            .collect::<Vec<_>>()
    });
    for (platform, provider) in providers {
        if let Some(credential) = provider.take_response_credential()? {
            if credential.platform != platform {
                return Err(contract_error(
                    "rotated credential platform does not match the request scope",
                ));
            }
            let issued = CallerCredential::issue(&credential)
                .map_err(|_| contract_error("provider returned an invalid rotated credential"))?;
            record_update(&issued);
        }
    }
    Ok(SCOPES.with(|scopes| {
        let mut scopes = scopes.borrow_mut();
        std::mem::take(&mut scopes.updates)
            .into_iter()
            .filter(|(platform, _)| {
                !scopes.invalidate_all && !scopes.invalidated.contains(platform)
            })
            .map(|(_, credential)| credential)
            .collect()
    }))
}

fn finish(mut response: Response) -> Response {
    let updates = match collect_updates() {
        Ok(updates) => updates,
        Err(error) => {
            response = crate::response::ApiError::from(error).into_response();
            Vec::new()
        }
    };
    // Build all values before publishing any credential header.
    let values = updates
        .into_iter()
        .map(|credential| {
            let mut value =
                HeaderValue::from_str(&format!("{}={}", credential.platform, credential.value))
                    .map_err(|_| {
                        contract_error(
                            "rotated credential cannot be represented as a response header",
                        )
                    })?;
            value.set_sensitive(true);
            Ok(value)
        })
        .collect::<Result<Vec<_>>>();
    match values {
        Ok(values) => {
            if !values.is_empty() {
                for value in values {
                    response
                        .headers_mut()
                        .append(UPDATED_CREDENTIAL_HEADER, value);
                }
                response.headers_mut().append(
                    header::ACCESS_CONTROL_EXPOSE_HEADERS,
                    HeaderValue::from_static(UPDATED_CREDENTIAL_HEADER),
                );
            }
        }
        Err(error) => response = crate::response::ApiError::from(error).into_response(),
    }
    if SCOPES.with(|scopes| scopes.borrow().sensitive) {
        let already_no_store = response
            .headers()
            .get_all(header::CACHE_CONTROL)
            .iter()
            .any(|value| {
                value.to_str().is_ok_and(|value| {
                    value
                        .split(',')
                        .any(|directive| directive.trim().eq_ignore_ascii_case("no-store"))
                })
            });
        if !already_no_store {
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        }
        response
            .headers_mut()
            .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    }
    response
}

pub(crate) async fn scope(sensitive: bool, future: impl Future<Output = Response>) -> Response {
    SCOPES
        .scope(
            RefCell::new(RequestScopes {
                sensitive,
                ..RequestScopes::default()
            }),
            async {
                let response = future.await;
                finish(response)
            },
        )
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use serde_json::Value;
    use std::sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    };
    use tower::ServiceExt;
    use tuneweave_core::{
        ArtistSummary, AudioContent, Capability, Extensions, MediaDownload, MediaStream, Page,
        PageMeta, ProviderRegistry, Quality, ResourceRef, SearchQuery, StreamRequest, Track,
    };

    struct RotatingMedia {
        platform: Platform,
        created: Arc<AtomicUsize>,
        sequence: AtomicUsize,
        pending: Mutex<Option<ProviderCredential>>,
        failure: Option<ErrorCode>,
        foreign_update: bool,
    }
    impl RotatingMedia {
        fn new(platform: Platform, failure: Option<ErrorCode>) -> Self {
            Self {
                platform,
                created: Arc::new(AtomicUsize::new(0)),
                sequence: AtomicUsize::new(0),
                pending: Mutex::new(None),
                failure,
                foreign_update: false,
            }
        }
        fn rotate(&self) {
            let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
            *self.pending.lock().unwrap() = Some(
                ProviderCredential::new(
                    self.platform,
                    "test",
                    format!("rotated-{}-{sequence}", self.platform),
                    None,
                )
                .unwrap(),
            );
        }
        fn metadata(&self, id: &str) -> Track {
            let mut track = Track::new(ResourceRef::new(self.platform, id).unwrap(), "scope song");
            track.artists.push(ArtistSummary {
                resource_ref: None,
                name: "scope artist".to_owned(),
            });
            track.duration_ms = Some(180000);
            track
        }
    }
    #[async_trait]
    impl MusicProvider for RotatingMedia {
        fn platform(&self) -> Platform {
            self.platform
        }
        fn name(&self) -> &'static str {
            "Rotating media test"
        }
        fn capabilities(&self) -> BTreeSet<Capability> {
            BTreeSet::from([
                Capability::CallerManagedCredentials,
                Capability::TrackDetail,
                Capability::SearchTracks,
                Capability::AudioStream,
                Capability::AudioDownload,
            ])
        }
        fn with_caller_credential(
            &self,
            credential: &ProviderCredential,
        ) -> Result<Arc<dyn MusicProvider>> {
            assert_eq!(credential.secret(), "original-scope-test-secret");
            self.created.fetch_add(1, Ordering::SeqCst);
            Ok(Arc::new(Self {
                platform: self.platform,
                created: self.created.clone(),
                sequence: AtomicUsize::new(0),
                pending: Mutex::new(None),
                failure: self.failure,
                foreign_update: self.foreign_update,
            }))
        }
        fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
            let pending = self.pending.lock().unwrap().take();
            if self.foreign_update {
                return pending
                    .map(|credential| {
                        ProviderCredential::new(Platform::Migu, "test", credential.secret(), None)
                    })
                    .transpose();
            }
            Ok(pending)
        }
        async fn track(&self, id: &str, _account: Option<&str>) -> Result<Track> {
            self.rotate();
            Ok(self.metadata(id))
        }
        async fn search(&self, query: &SearchQuery) -> Result<Page<Track>> {
            self.rotate();
            Ok(Page {
                items: vec![self.metadata("2")],
                pagination: PageMeta {
                    limit: query.limit,
                    offset: query.offset,
                    total: Some(1),
                    next_offset: None,
                    has_more: false,
                    extensions: Extensions::new(),
                },
            })
        }
        async fn stream(&self, track: &Track, _request: &StreamRequest) -> Result<MediaStream> {
            self.rotate();
            if let Some(code) = self.failure {
                return Err(TuneWeaveError::new(code, "simulated media failure")
                    .with_platform(self.platform));
            }
            Ok(MediaStream {
                url: format!("/v1/tracks/{}/stream/content", track.resource_ref),
                backup_urls: Vec::new(),
                headers: BTreeMap::new(),
                expires_at: None,
                format: Some("mp3".to_owned()),
                codec: Some("mp3".to_owned()),
                bitrate: Some(128000),
                size: Some(3),
                duration_ms: Some(180000),
                requested_quality: Quality::Auto,
                actual_quality: Quality::Standard,
                trial: None,
                origin_track: Some(track.resource_ref.clone()),
                resolved_track: track.resource_ref.clone(),
                resolved_platform: self.platform,
                match_score: Some(1.0),
                attempts: Vec::new(),
            })
        }
        async fn download(&self, track: &Track, _request: &StreamRequest) -> Result<MediaDownload> {
            self.rotate();
            Ok(MediaDownload {
                track_ref: track.resource_ref.clone(),
                platform: self.platform,
                available: false,
                url: None,
                headers: BTreeMap::new(),
                expires_at: None,
                format: None,
                codec: None,
                bitrate: None,
                size: None,
                duration_ms: Some(180000),
                requested_quality: Quality::Auto,
                actual_quality: Quality::Standard,
                platform_code: None,
                fee: None,
                message: None,
                extensions: Extensions::new(),
            })
        }
        async fn audio_content(
            &self,
            track: &Track,
            _request: &StreamRequest,
        ) -> Result<AudioContent> {
            self.rotate();
            if let Some(code) = self.failure {
                return Err(TuneWeaveError::new(code, "simulated content failure")
                    .with_platform(self.platform));
            }
            Ok(AudioContent {
                track_ref: track.resource_ref.clone(),
                bytes: vec![1, 2, 3],
                content_type: "audio/mpeg".to_owned(),
                filename: "test.mp3".to_owned(),
                trial: None,
            })
        }
    }

    fn caller(platform: Platform) -> CallerCredential {
        CallerCredential::issue(
            &ProviderCredential::new(platform, "test", "original-scope-test-secret", None).unwrap(),
        )
        .unwrap()
    }
    fn updates(response: &Response) -> BTreeMap<Platform, String> {
        response
            .headers()
            .get_all(UPDATED_CREDENTIAL_HEADER)
            .iter()
            .map(|value| {
                assert!(value.is_sensitive());
                let (platform, value) = value.to_str().unwrap().split_once('=').unwrap();
                let credential = CallerCredential::parse(value).unwrap();
                assert_eq!(platform, credential.platform.to_string());
                (credential.platform, credential.secret().to_owned())
            })
            .collect()
    }
    fn app(provider: RotatingMedia) -> axum::Router {
        let mut registry = ProviderRegistry::new();
        registry.register(provider).unwrap();
        crate::build_router(crate::AppState::new(registry, Platform::Soda))
    }
    async fn request(app: axum::Router, path: &str, platforms: &[Platform]) -> Response {
        let mut request = Request::builder().uri(path);
        for platform in platforms {
            request = request.header(crate::CALLER_CREDENTIAL_HEADER, caller(*platform).value);
        }
        app.oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn metadata_binary_redirect_and_batch_return_the_latest_updates() {
        for (path, expected_sequence, status) in [
            ("/v1/tracks/soda:1", 1, StatusCode::OK),
            (
                "/v1/tracks/soda:1/stream?fallback=false&unblock=false",
                2,
                StatusCode::OK,
            ),
            ("/v1/tracks/soda:1/stream/content", 2, StatusCode::OK),
            (
                "/v1/tracks/soda:1/stream/redirect?fallback=false&unblock=false",
                2,
                StatusCode::FOUND,
            ),
            (
                "/v1/tracks/streams?refs=soda:1,soda:2&fallback=false&unblock=false",
                2,
                StatusCode::OK,
            ),
        ] {
            let response = request(
                app(RotatingMedia::new(Platform::Soda, None)),
                path,
                &[Platform::Soda],
            )
            .await;
            assert_eq!(response.status(), status, "{path}");
            assert!(
                response.headers()[header::CACHE_CONTROL]
                    .to_str()
                    .unwrap()
                    .contains("no-store")
            );
            assert_eq!(
                response.headers()[header::ACCESS_CONTROL_EXPOSE_HEADERS],
                UPDATED_CREDENTIAL_HEADER
            );
            assert_eq!(
                updates(&response).get(&Platform::Soda),
                Some(&format!("rotated-soda-{expected_sequence}")),
                "{path}"
            );
            if path.ends_with("/content") {
                assert_eq!(
                    &to_bytes(response.into_body(), 1024).await.unwrap()[..],
                    &[1, 2, 3]
                );
            }
        }
    }

    #[tokio::test]
    async fn client_hosted_uni_stream_returns_updates_without_storing_credentials_in_the_item() {
        let app = app(RotatingMedia::new(Platform::Soda, None));
        let materialized = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/uni/materialize/items")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(
                        crate::CALLER_CREDENTIAL_HEADER,
                        caller(Platform::Soda).value,
                    )
                    .body(Body::from(r#"{"items":[{"ref":"soda:1","kind":"track"}]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(materialized.status(), StatusCode::OK);
        assert_eq!(updates(&materialized)[&Platform::Soda], "rotated-soda-1");
        let body: Value =
            serde_json::from_slice(&to_bytes(materialized.into_body(), 65536).await.unwrap())
                .unwrap();
        let item = body["data"]["items"][0].clone();
        assert!(item.is_object());
        assert!(!item.to_string().contains("twc1_") && !item.to_string().contains("secret"));
        let stream = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/uni/items/stream")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(
                        crate::CALLER_CREDENTIAL_HEADER,
                        caller(Platform::Soda).value,
                    )
                    .body(Body::from(
                        serde_json::json!({"item":item,"fallback":false,"unblock":false})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stream.status(), StatusCode::OK);
        assert_eq!(stream.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(updates(&stream)[&Platform::Soda], "rotated-soda-1");
        let body: Value =
            serde_json::from_slice(&to_bytes(stream.into_body(), 65536).await.unwrap()).unwrap();
        assert_eq!(body["data"]["stream"]["resolved_track"], "soda:1");
    }

    #[tokio::test]
    async fn uni_append_uses_caller_metadata_and_both_item_endpoints_reject_mixed_aliases() {
        let app = app(RotatingMedia::new(Platform::Soda, None));
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/uni/playlists")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"name":"scope test"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
        let reference = body["data"]["ref"].as_str().unwrap();
        let path = format!("/v1/uni/playlists/{reference}/items");
        for (path, accounts, expected) in [
            (path.as_str(), serde_json::json!({}), StatusCode::OK),
            (
                path.as_str(),
                serde_json::json!({"soda":"personal"}),
                StatusCode::BAD_REQUEST,
            ),
            (
                "/v1/uni/materialize/items",
                serde_json::json!({"soda":"personal"}),
                StatusCode::BAD_REQUEST,
            ),
        ] {
            let response = app.clone().oneshot(Request::builder()
                .method("POST").uri(path)
                .header(header::CONTENT_TYPE, "application/json")
                .header(crate::CALLER_CREDENTIAL_HEADER, caller(Platform::Soda).value)
                .body(Body::from(serde_json::json!({"items":[{"ref":"soda:1","kind":"track"}],"accounts":accounts}).to_string())).unwrap()).await.unwrap();
            assert_eq!(response.status(), expected);
            let updates = updates(&response);
            if expected == StatusCode::OK {
                assert_eq!(updates[&Platform::Soda], "rotated-soda-1");
            } else {
                assert!(updates.is_empty());
            }
        }
        let stored = request(app, &format!("/v1/uni/playlists/{reference}/export"), &[]).await;
        assert_eq!(stored.status(), StatusCode::OK);
        let body = to_bytes(stored.into_body(), 65536).await.unwrap();
        let text = std::str::from_utf8(&body).unwrap();
        assert!(!text.contains("twc1_") && !text.contains("secret") && !text.contains("personal"));
    }

    #[tokio::test]
    async fn download_fallback_reuses_the_same_caller_scope_and_concurrent_requests_stay_separate()
    {
        let provider = RotatingMedia::new(Platform::Soda, None);
        let created = provider.created.clone();
        let app = app(provider);
        let path = "/v1/tracks/soda:1/download?fallback=false&unblock=false";
        let (first, second) = tokio::join!(
            request(app.clone(), path, &[Platform::Soda]),
            request(app, path, &[Platform::Soda])
        );
        for response in [first, second] {
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(updates(&response)[&Platform::Soda], "rotated-soda-4");
        }
        assert_eq!(created.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn fallback_returns_each_valid_platform_update_and_discards_invalidated_sessions() {
        for code in [
            ErrorCode::UpstreamError,
            ErrorCode::AuthenticationRequired,
            ErrorCode::Conflict,
        ] {
            let mut registry = ProviderRegistry::new();
            registry
                .register(RotatingMedia::new(Platform::Soda, Some(code)))
                .unwrap();
            registry
                .register(RotatingMedia::new(Platform::Qq, None))
                .unwrap();
            let app = crate::build_router(crate::AppState::new(registry, Platform::Soda));
            let response = request(
                app,
                "/v1/tracks/soda:1/stream?fallback=true&fallback_platforms=qq&unblock=false",
                &[Platform::Soda, Platform::Qq],
            )
            .await;
            assert_eq!(response.status(), StatusCode::OK);
            let updates = updates(&response);
            assert_eq!(updates[&Platform::Qq], "rotated-qq-2");
            assert_eq!(
                updates.contains_key(&Platform::Soda),
                code == ErrorCode::UpstreamError
            );
        }
    }

    #[tokio::test]
    async fn binary_failures_keep_prior_updates_except_for_invalidated_sessions() {
        for (code, status, updated) in [
            (ErrorCode::UpstreamError, StatusCode::BAD_GATEWAY, true),
            (
                ErrorCode::AuthenticationRequired,
                StatusCode::UNAUTHORIZED,
                false,
            ),
            (ErrorCode::Conflict, StatusCode::CONFLICT, false),
        ] {
            let response = request(
                app(RotatingMedia::new(Platform::Soda, Some(code))),
                "/v1/tracks/soda:1/stream/content",
                &[Platform::Soda],
            )
            .await;
            assert_eq!(response.status(), status);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            assert_eq!(updates(&response).contains_key(&Platform::Soda), updated);
            let body: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                    .unwrap();
            assert_eq!(body["error"]["code"], code.as_str());
        }
    }

    #[tokio::test]
    async fn foreign_provider_updates_fail_without_delivering_the_credential() {
        let mut provider = RotatingMedia::new(Platform::Soda, None);
        provider.foreign_update = true;
        let response = request(
            app(provider),
            "/v1/tracks/soda:1/stream?fallback=false&unblock=false",
            &[Platform::Soda],
        )
        .await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert!(updates(&response).is_empty());
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        assert!(!String::from_utf8_lossy(&body).contains("rotated-soda"));
    }
}
