use super::*;

#[derive(Clone)]
struct RefreshFailure {
    mode: CredentialMode,
    code: ErrorCode,
    update: Option<ProviderCredential>,
    error_platform: Option<Platform>,
    prior_update: bool,
}

fn credential(secret: &str) -> ProviderCredential {
    ProviderCredential::new(Platform::Migu, "session", secret, None).unwrap()
}

#[async_trait]
impl MusicProvider for RefreshFailure {
    fn platform(&self) -> Platform {
        Platform::Migu
    }
    fn name(&self) -> &'static str {
        "Session rotation failure fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::SessionManagement,
            Capability::CallerManagedCredentials,
        ])
    }
    fn with_caller_credential(
        &self,
        source: &ProviderCredential,
    ) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(source.platform, Platform::Migu);
        assert_eq!(source.secret(), "input-secret");
        Ok(Arc::new(self.clone()))
    }
    async fn refresh_session_with_ownership(
        &self,
        account: &str,
        source: Option<&ProviderCredential>,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        assert_eq!(mode, self.mode);
        assert_eq!(
            account,
            if mode == CredentialMode::Client {
                "default"
            } else {
                "personal"
            }
        );
        assert_eq!(source.is_some(), mode != CredentialMode::Server);
        if let Some(source) = source {
            assert_eq!(source.secret(), "input-secret");
        }
        if self.prior_update {
            caller_scope::record_update(
                &CallerCredential::issue(&credential("earlier-secret")).unwrap(),
            );
        }
        let mut error =
            TuneWeaveError::new(ErrorCode::UpstreamError, "profile failed").retryable(true);
        error.platform = self.error_platform;
        if let Some(update) = &self.update {
            error = error.with_caller_credential_update(update.clone());
        }
        // Exercise invalidation even if a provider changes the code after attaching an update.
        error.code = self.code;
        Err(error)
    }
}

async fn refresh(failure: RefreshFailure) -> Response {
    let mode = failure.mode;
    let mut registry = ProviderRegistry::new();
    registry.register(failure).unwrap();
    let app = build_router(AppState::new(registry, Platform::Migu));
    let mut body = json!({"platform":"migu", "credential_mode":mode});
    if mode != CredentialMode::Client {
        body["account"] = json!("personal");
    }
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/v1/auth/session/refresh")
        .header(header::CONTENT_TYPE, "application/json");
    if mode != CredentialMode::Server {
        request = request.header(
            CALLER_CREDENTIAL_HEADER,
            CallerCredential::issue(&credential("input-secret"))
                .unwrap()
                .value,
        );
    }
    app.oneshot(request.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap()
}

async fn inspect(response: Response, status: StatusCode, update: bool) -> Value {
    assert_eq!(response.status(), status);
    assert!(
        response.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .split(',')
            .any(|value| value.trim() == "no-store")
    );
    assert_eq!(response.headers()[header::PRAGMA], "no-cache");
    let headers = response
        .headers()
        .get_all(caller_scope::UPDATED_CREDENTIAL_HEADER)
        .iter()
        .collect::<Vec<_>>();
    assert_eq!(headers.len(), usize::from(update));
    let header_value = if update {
        assert!(headers[0].is_sensitive());
        let value = headers[0].to_str().unwrap().strip_prefix("migu=").unwrap();
        assert_eq!(
            CallerCredential::parse(value).unwrap().secret(),
            "latest-secret"
        );
        Some(value.to_owned())
    } else {
        None
    };
    let body = to_bytes(response.into_body(), 65536).await.unwrap();
    for secret in [
        "input-secret",
        "earlier-secret",
        "latest-secret",
        "invalid-secret",
    ] {
        assert!(!String::from_utf8_lossy(&body).contains(secret));
    }
    let value: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(value["ok"], false);
    assert!(value.get("data").is_none());
    assert_eq!(value["meta"].get("caller_credential").is_some(), update);
    if let Some(header) = header_value {
        assert_eq!(value["meta"]["caller_credential"]["value"], header);
    }
    value
}

#[tokio::test]
async fn refresh_failure_preserves_original_error_and_delivers_the_latest_owned_update() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        for (code, status) in [
            (ErrorCode::UpstreamError, StatusCode::BAD_GATEWAY),
            (ErrorCode::UpstreamTimeout, StatusCode::GATEWAY_TIMEOUT),
        ] {
            let update = mode.returns_to_caller();
            let response = refresh(RefreshFailure {
                mode,
                code,
                update: update.then(|| credential("latest-secret")),
                error_platform: Some(Platform::Migu),
                prior_update: update,
            })
            .await;
            let value = inspect(response, status, update).await;
            assert_eq!(value["error"]["code"], code.as_str());
            assert_eq!(value["error"]["platform"], "migu");
            assert_eq!(value["error"]["retryable"], true);
            assert_eq!(value["error"]["details"], json!({}));
        }
    }
}

#[tokio::test]
async fn refresh_rejects_invalid_update_contracts_without_leaking_prior_updates() {
    let mut wrong_platform = credential("invalid-secret");
    wrong_platform.platform = Platform::Soda;
    let mut expired = credential("invalid-secret");
    expired.expires_at = Some(1);
    let mut invalid_kind = credential("invalid-secret");
    invalid_kind.kind = "bad kind".into();
    for (mode, error_platform, update) in [
        (
            CredentialMode::Server,
            Some(Platform::Migu),
            credential("invalid-secret"),
        ),
        (
            CredentialMode::Client,
            Some(Platform::Soda),
            credential("invalid-secret"),
        ),
        (CredentialMode::Both, None, credential("invalid-secret")),
        (CredentialMode::Client, Some(Platform::Migu), wrong_platform),
        (CredentialMode::Both, Some(Platform::Migu), expired),
        (CredentialMode::Client, Some(Platform::Migu), invalid_kind),
        (
            CredentialMode::Both,
            Some(Platform::Migu),
            credential(&"x".repeat(65_536)),
        ),
    ] {
        let response = refresh(RefreshFailure {
            mode,
            code: ErrorCode::UpstreamError,
            update: Some(update),
            error_platform,
            prior_update: true,
        })
        .await;
        let value = inspect(response, StatusCode::INTERNAL_SERVER_ERROR, false).await;
        assert_eq!(value["error"]["code"], "internal_error");
    }
}

#[tokio::test]
async fn invalidated_refresh_suppresses_both_typed_and_previously_queued_credentials() {
    for code in [ErrorCode::AuthenticationRequired, ErrorCode::Conflict] {
        for mode in [CredentialMode::Client, CredentialMode::Both] {
            let response = refresh(RefreshFailure {
                mode,
                code,
                update: Some(credential("latest-secret")),
                error_platform: Some(Platform::Migu),
                prior_update: true,
            })
            .await;
            let status = if code == ErrorCode::Conflict {
                StatusCode::CONFLICT
            } else {
                StatusCode::UNAUTHORIZED
            };
            let value = inspect(response, status, false).await;
            assert_eq!(value["error"]["code"], code.as_str());
        }
    }
}

#[tokio::test]
async fn ordinary_api_error_conversion_never_exports_a_typed_refresh_update() {
    let error = TuneWeaveError::new(ErrorCode::UpstreamError, "operation failed")
        .with_platform(Platform::Migu)
        .with_caller_credential_update(credential("latest-secret"));
    let response = caller_scope::scope(true, async { ApiError::from(error).into_response() }).await;
    let value = inspect(response, StatusCode::BAD_GATEWAY, false).await;
    assert_eq!(value["error"]["details"], json!({}));
}

#[tokio::test]
async fn refresh_failure_logs_keep_status_and_source_without_credential_material() {
    let logs = CapturedTestLogs::default();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .without_time()
        .with_ansi(false)
        .with_writer(logs.clone())
        .finish();
    let response = refresh(RefreshFailure {
        mode: CredentialMode::Both,
        code: ErrorCode::UpstreamTimeout,
        update: Some(credential("latest-secret")),
        error_platform: Some(Platform::Migu),
        prior_update: true,
    })
    .with_subscriber(subscriber)
    .await;
    inspect(response, StatusCode::GATEWAY_TIMEOUT, true).await;
    let fields = logs.request_completion_fields();
    assert_eq!(fields["status"], 504);
    assert_eq!(fields["platform"], "migu");
    assert_eq!(fields["credential_source"], "caller");
    let logs = logs.text();
    for secret in ["input-secret", "earlier-secret", "latest-secret"] {
        assert!(!logs.contains(secret));
        assert!(!logs.contains(&CallerCredential::issue(&credential(secret)).unwrap().value));
    }
}
