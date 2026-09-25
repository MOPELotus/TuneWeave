use super::*;
use tuneweave_core::ProviderQrPoll;

type Calls = Arc<Mutex<Vec<(String, CredentialMode)>>>;
struct Provider(Calls, bool);
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Soda
    }
    fn name(&self) -> &'static str {
        "Soda authentication generation contract"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::QrLogin, Capability::CallerManagedCredentials])
    }
    async fn start_qr_login_with_mode(
        &self,
        _: Option<&str>,
        _: CredentialMode,
    ) -> tuneweave_core::Result<ProviderQrStart> {
        panic!("HTTP QR creation must pass the destination account")
    }
    async fn start_qr_login_for_account(
        &self,
        _: Option<&str>,
        account: &str,
        mode: CredentialMode,
    ) -> tuneweave_core::Result<ProviderQrStart> {
        self.0.lock().unwrap().push((account.into(), mode));
        Ok(ProviderQrStart {
            provider_transaction_id: "generation-fixture".into(),
            url: "https://example.com/qr".into(),
            image_data_url: None,
            expires_at: None,
        })
    }
    async fn poll_qr_login_with_mode(
        &self,
        id: &str,
        account: &str,
        mode: CredentialMode,
    ) -> tuneweave_core::Result<ProviderQrPoll> {
        assert_eq!(id, "generation-fixture");
        assert_eq!(
            self.0.lock().unwrap().last(),
            Some(&(account.to_owned(), mode))
        );
        let error = TuneWeaveError::new(
            ErrorCode::Conflict,
            "Soda authentication transaction was cancelled",
        )
        .with_platform(Platform::Soda);
        Err(if self.1 {
            error.with_consumed_auth_challenge()
        } else {
            error
        })
    }
    async fn verify_qr_login(
        &self,
        id: &str,
        account: &str,
        mode: CredentialMode,
        _: &tuneweave_core::QrVerificationAction,
    ) -> tuneweave_core::Result<ProviderQrPoll> {
        self.poll_qr_login_with_mode(id, account, mode).await
    }
}
fn app() -> (Router, Calls) {
    app_with_consumed(true)
}
fn app_with_consumed(consumed: bool) -> (Router, Calls) {
    let calls = Arc::default();
    let mut registry = ProviderRegistry::new();
    registry
        .register(Provider(Arc::clone(&calls), consumed))
        .unwrap();
    (build_router(AppState::new(registry, Platform::Soda)), calls)
}
fn private(response: &Response) {
    assert!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .is_some_and(|v| v.to_str().unwrap().contains("no-store"))
    );
    assert!(
        !response
            .headers()
            .contains_key(caller_scope::UPDATED_CREDENTIAL_HEADER)
    );
}

#[tokio::test]
async fn soda_auth_generation_http_binds_account_at_creation_and_preserves_conflicts() {
    for (account, mode) in [
        ("personal", CredentialMode::Server),
        ("personal", CredentialMode::Both),
        ("default", CredentialMode::Client),
    ] {
        let (router, calls) = app();
        let mut body = json!({"platform":"soda", "account":account, "credential_mode":mode});
        if mode == CredentialMode::Client {
            body.as_object_mut().unwrap().remove("account");
        }
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/auth/qr")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        private(&response);
        let bytes = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let id = body["data"]["transaction_id"].as_str().unwrap();
        assert_eq!(calls.lock().unwrap().as_slice(), &[(account.into(), mode)]);
        let response = router
            .oneshot(
                Request::builder()
                    .uri(format!("/v1/auth/qr/{id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        private(&response);
        let bytes = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("caller_credential"));
    }
}

#[tokio::test]
async fn soda_auth_generation_auth_errors_before_handler_always_disable_caching() {
    let paths = [
        "/v1/auth/qr",
        "/v1/auth/qr/unknown",
        "/v1/auth/qr/unknown/verification",
        "/v1/auth/import",
        "/v1/auth/password",
        "/v1/auth/session",
        "/v1/auth/session/refresh",
        "/v1/auth/challenges",
        "/v1/auth/challenges/unknown/verify",
        "/v1/auth/security-challenges",
    ];
    for path in paths {
        for bad_id in [false, true] {
            let (router, calls) = app();
            let mut request = Request::builder()
                .method(if bad_id { Method::POST } else { Method::PUT })
                .uri(path)
                .header(header::CONTENT_TYPE, "application/json");
            if bad_id {
                request = request.header("x-request-id", "invalid request id");
            }
            let response = router
                .oneshot(request.body(Body::from("{")).unwrap())
                .await
                .unwrap();
            assert!(response.status().is_client_error(), "{path}");
            private(&response);
            assert!(calls.lock().unwrap().is_empty());
        }
    }
    for body in [
        "{",
        r#"{"platform":"soda","account":"personal","credential_mode":"client"}"#,
    ] {
        let (router, calls) = app();
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/v1/auth/qr")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.status().is_client_error());
        private(&response);
        assert!(calls.lock().unwrap().is_empty());
    }

    for (method, uri, body) in [
        (Method::GET, "/v1/auth/session?unknown=1", None),
        (Method::POST, "/v1/auth/qr?unknown=1", Some("{}")),
    ] {
        let (router, calls) = app();
        let response = router
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(uri)
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(body.map_or_else(Body::empty, Body::from))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{uri}");
        private(&response);
        assert!(calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn soda_auth_generation_http_consumes_only_explicitly_consumed_qr_errors() {
    for consumed in [false, true] {
        for verification in [false, true] {
            let (router, _) = app_with_consumed(consumed);
            let response = router
                .clone()
                .oneshot(
                    Request::builder()
                        .method(Method::POST)
                        .uri("/v1/auth/qr")
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(r#"{"platform":"soda","account":"personal"}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), 65536)
                .await
                .unwrap();
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            let id = body["data"]["transaction_id"].as_str().unwrap();
            let path = format!(
                "/v1/auth/qr/{id}{}",
                if verification { "/verification" } else { "" }
            );
            for attempt in 0..2 {
                let request = Request::builder()
                    .uri(&path)
                    .method(if verification {
                        Method::POST
                    } else {
                        Method::GET
                    })
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(if verification {
                        Body::from(r#"{"action":"send_sms"}"#)
                    } else {
                        Body::empty()
                    })
                    .unwrap();
                let response = router.clone().oneshot(request).await.unwrap();
                assert_eq!(
                    response.status(),
                    if consumed && attempt == 1 {
                        StatusCode::NOT_FOUND
                    } else {
                        StatusCode::CONFLICT
                    }
                );
                private(&response);
            }
        }
    }
}
