use super::*;
use tuneweave_core::{ProviderSessionRevocationResult, SessionRevocationState};

type Calls = Arc<Mutex<Vec<(String, CredentialMode, bool)>>>;
#[derive(Clone)]
struct Provider {
    calls: Calls,
    fail: bool,
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Soda
    }
    fn name(&self) -> &'static str {
        "Soda revocation contract"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::SessionRevocation,
            Capability::CallerManagedCredentials,
        ])
    }
    fn with_caller_credential(
        &self,
        source: &ProviderCredential,
    ) -> tuneweave_core::Result<Arc<dyn MusicProvider>> {
        assert_eq!(source.secret(), "revocation-private-input");
        Ok(Arc::new(self.clone()))
    }
    fn take_response_credential(&self) -> tuneweave_core::Result<Option<ProviderCredential>> {
        // A deliberately pending update must be suppressed by the HTTP scope too.
        Ok(Some(
            ProviderCredential::new(Platform::Soda, "fixture", "must-not-be-reissued", None)
                .unwrap(),
        ))
    }
    async fn revoke_session_with_ownership(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> tuneweave_core::Result<ProviderSessionRevocationResult> {
        if let Some(source) = source {
            assert_eq!(source.secret(), "revocation-private-input");
        }
        self.calls
            .lock()
            .unwrap()
            .push((account.into(), mode, source.is_some()));
        if self.fail {
            return Err(TuneWeaveError::new(ErrorCode::UpstreamError,"Session invalidation was not confirmed")
                .with_platform(Platform::Soda)
                .with_details(json!({"upstream_outcome":"unconfirmed","revocation_request_started":true,"removed":mode.persists_on_server(),"caller_credential_discard_required":source.is_some()}))
                .with_caller_credential_update(ProviderCredential::new(Platform::Soda,"fixture","must-not-be-reissued",None).unwrap()));
        }
        Ok(ProviderSessionRevocationResult {
            state: SessionRevocationState::Invalidated,
            removed: mode.persists_on_server(),
            caller_credential_discard_required: source.is_some(),
            revocation_request_started: true,
        })
    }
}
fn app(fail: bool) -> (Router, Calls) {
    let calls = Arc::default();
    let mut registry = ProviderRegistry::new();
    registry
        .register(Provider {
            calls: Arc::clone(&calls),
            fail,
        })
        .unwrap();
    (build_router(AppState::new(registry, Platform::Soda)), calls)
}
fn request(body: &str, caller: bool, method: Method, bad_id: bool) -> Request<Body> {
    let mut r = Request::builder()
        .uri("/v1/auth/session/revoke")
        .method(method)
        .header(header::CONTENT_TYPE, "application/json");
    if caller {
        r = r.header(
            CALLER_CREDENTIAL_HEADER,
            CallerCredential::issue(
                &ProviderCredential::new(
                    Platform::Soda,
                    "fixture",
                    "revocation-private-input",
                    None,
                )
                .unwrap(),
            )
            .unwrap()
            .value,
        );
    }
    if bad_id {
        r = r.header("x-request-id", "invalid request id");
    }
    r.body(Body::from(body.to_owned())).unwrap()
}
fn private(response: &Response) {
    assert!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .is_some_and(|v| v.to_str().unwrap().contains("no-store")),
        "{:?}",
        response.status()
    );
    assert!(
        !response
            .headers()
            .contains_key(caller_scope::UPDATED_CREDENTIAL_HEADER)
    );
}

#[tokio::test]
async fn soda_session_revocation_http_modes_return_evidence_without_credentials() {
    for (body, caller, mode, alias) in [
        (
            r#"{"platform":"soda"}"#,
            false,
            CredentialMode::Server,
            "default",
        ),
        (
            r#"{"platform":"soda","account":"personal"}"#,
            false,
            CredentialMode::Server,
            "personal",
        ),
        (
            r#"{"platform":"soda"}"#,
            true,
            CredentialMode::Client,
            "default",
        ),
        (
            r#"{"platform":"soda","account":"personal","credential_mode":"both"}"#,
            true,
            CredentialMode::Both,
            "personal",
        ),
    ] {
        for fail in [false, true] {
            let (router, calls) = app(fail);
            let response = router
                .oneshot(request(body, caller, Method::POST, false))
                .await
                .unwrap();
            private(&response);
            assert_eq!(
                response.status(),
                if fail {
                    StatusCode::BAD_GATEWAY
                } else {
                    StatusCode::OK
                }
            );
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            let value: Value = serde_json::from_slice(&bytes).unwrap();
            let raw = String::from_utf8_lossy(&bytes);
            assert!(!raw.contains("revocation-private-input"));
            assert!(!raw.contains("must-not-be-reissued"));
            assert!(!raw.contains("caller_credential\""));
            if fail {
                assert_eq!(value["error"]["details"]["upstream_outcome"], "unconfirmed");
                assert_eq!(value["error"]["retryable"], false);
            } else {
                assert_eq!(value["data"]["state"], "invalidated");
                assert_eq!(value["data"]["removed"], mode.persists_on_server());
                assert_eq!(value["data"]["caller_credential_discard_required"], caller);
            }
            assert_eq!(*calls.lock().unwrap(), vec![(alias.into(), mode, caller)]);
        }
    }
}

#[tokio::test]
async fn soda_session_revocation_http_early_errors_are_private_and_do_not_dispatch() {
    for (body, caller, method, bad_id) in [
        ("{}".to_owned(), false, Method::POST, false),
        ("{".to_owned(), false, Method::POST, false),
        (
            r#"{"platform":"soda","extra":true}"#.to_owned(),
            false,
            Method::POST,
            false,
        ),
        (
            r#"{"platform":"soda","credential_mode":"both"}"#.to_owned(),
            false,
            Method::POST,
            false,
        ),
        (
            r#"{"platform":"soda","credential_mode":"both"}"#.to_owned(),
            true,
            Method::POST,
            false,
        ),
        (
            r#"{"platform":"soda","credential_mode":"server"}"#.to_owned(),
            true,
            Method::POST,
            false,
        ),
        (
            r#"{"platform":"soda","account":"personal"}"#.to_owned(),
            true,
            Method::POST,
            false,
        ),
        (
            r#"{"platform":"soda"}"#.to_owned(),
            false,
            Method::GET,
            false,
        ),
        (
            r#"{"platform":"soda"}"#.to_owned(),
            false,
            Method::POST,
            true,
        ),
        (
            format!(r#"{{"platform":"soda","account":"{}"}}"#, "a".repeat(5000)),
            false,
            Method::POST,
            false,
        ),
    ] {
        let (router, calls) = app(false);
        let response = router
            .oneshot(request(&body, caller, method, bad_id))
            .await
            .unwrap();
        assert!(response.status().is_client_error());
        private(&response);
        assert!(calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn soda_session_revocation_http_real_provider_absent_alias_is_not_remote_success() {
    let mut registry = ProviderRegistry::new();
    registry
        .register(
            tuneweave_provider_soda::SodaProvider::new(tuneweave_provider_soda::SodaConfig {
                proxy_url: Some("http://127.0.0.1:9".into()),
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
    let router = build_router(AppState::new(registry, Platform::Soda));
    let response = router
        .oneshot(request(
            r#"{"platform":"soda"}"#,
            false,
            Method::POST,
            false,
        ))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    private(&response);
    let value: Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(value["data"]["state"], "no_stored_session");
    assert_eq!(value["data"]["revocation_request_started"], false);
    assert_eq!(value["data"]["removed"], false);
}
