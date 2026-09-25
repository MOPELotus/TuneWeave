//! HTTP contract fixtures; real Provider account/membership/UID reads are tested in Kugou.
use super::*;

type Calls = Arc<Mutex<Vec<(Option<String>, String, bool)>>>;
#[derive(Clone)]
struct Provider {
    caller: bool,
    failure: Option<ErrorCode>,
    typed_update: bool,
    calls: Calls,
    update: Arc<Mutex<Option<ProviderCredential>>>,
}
impl Provider {
    fn read(
        &self,
        id: Option<&str>,
        account: Option<&str>,
        client: bool,
    ) -> tuneweave_core::Result<MembershipSummary> {
        self.calls.lock().unwrap().push((
            id.map(str::to_owned),
            account.unwrap().to_owned(),
            client,
        ));
        if self.caller
            && !matches!(
                self.failure,
                Some(ErrorCode::UpstreamError | ErrorCode::UpstreamTimeout)
            )
        {
            *self.update.lock().unwrap() = Some(
                ProviderCredential::new(Platform::Kugou, "fixture", "member-private-rotated", None)
                    .unwrap(),
            );
        }
        if let Some(code) = self.failure {
            let mut error =
                TuneWeaveError::new(code, "Membership read failed").with_platform(Platform::Kugou);
            if self.caller && self.typed_update {
                error = error.with_caller_credential_update(
                    ProviderCredential::new(
                        Platform::Kugou,
                        "fixture",
                        "independently-verified-update",
                        None,
                    )
                    .unwrap(),
                );
            }
            return Err(error);
        }
        Ok(MembershipSummary {
            user_ref: Some(ResourceRef::new(Platform::Kugou, "123456").unwrap()),
            active: Some(true),
            expires_at: Some("2027-01-15 08:00:00".into()),
            level: None,
            annual_count: None,
            icon_url: None,
            extensions: Extensions::from([
                ("backend".into(), json!("standard_vip_detail_v3")),
                ("source_user_id".into(), json!("123456")),
                ("date_timezone".into(), Value::Null),
                (
                    "membership_details".into(),
                    json!({"vip_type":6,"user_type":8}),
                ),
            ]),
        })
    }
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Kugou
    }
    fn name(&self) -> &'static str {
        "Kugou membership contract"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::UserMembership,
            Capability::UserMembershipClientInfo,
            Capability::CallerManagedCredentials,
        ])
    }
    fn with_caller_credential(
        &self,
        c: &ProviderCredential,
    ) -> tuneweave_core::Result<Arc<dyn MusicProvider>> {
        assert_eq!(c.secret(), "member-private-input");
        Ok(Arc::new(Self {
            caller: true,
            update: Arc::default(),
            ..self.clone()
        }))
    }
    fn take_response_credential(&self) -> tuneweave_core::Result<Option<ProviderCredential>> {
        Ok(self.update.lock().unwrap().take())
    }
    async fn user_membership(
        &self,
        id: Option<&str>,
        account: Option<&str>,
    ) -> tuneweave_core::Result<MembershipSummary> {
        self.read(id, account, false)
    }
    async fn user_membership_client_info(
        &self,
        id: Option<&str>,
        account: Option<&str>,
    ) -> tuneweave_core::Result<MembershipSummary> {
        self.read(id, account, true)
    }
}
fn app(failure: Option<ErrorCode>) -> (Router, Calls) {
    app_with_typed_update(failure, false)
}
fn app_with_typed_update(failure: Option<ErrorCode>, typed_update: bool) -> (Router, Calls) {
    let calls = Arc::default();
    let mut registry = ProviderRegistry::new();
    registry
        .register(Provider {
            caller: false,
            failure,
            typed_update,
            calls: Arc::clone(&calls),
            update: Arc::default(),
        })
        .unwrap();
    (
        build_router(AppState::new(registry, Platform::Kugou)),
        calls,
    )
}
fn request(path: &str, caller: bool, method: Method) -> Request<Body> {
    let mut r = Request::builder().uri(path).method(method);
    if caller {
        r = r.header(
            CALLER_CREDENTIAL_HEADER,
            CallerCredential::issue(
                &ProviderCredential::new(Platform::Kugou, "fixture", "member-private-input", None)
                    .unwrap(),
            )
            .unwrap()
            .value,
        );
    }
    r.body(Body::empty()).unwrap()
}
fn no_store(response: &Response) {
    assert!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .is_some_and(|v| v.to_str().unwrap().contains("no-store")),
        "missing no-store for {:?}",
        response.status()
    );
}

#[tokio::test]
async fn kugou_membership_http_preserves_expiry_sources_backends_and_caller_update() {
    for owner in ["default", "personal", "caller"] {
        for user in [false, true] {
            for backend in ["front", "client"] {
                let (router, calls) = app(None);
                let mut path = if user {
                    "/v1/users/kugou:123456/membership?".to_owned()
                } else {
                    "/v1/account/membership?platform=kugou&".to_owned()
                };
                path.push_str(&format!("backend={backend}"));
                if owner != "caller" {
                    path.push_str(&format!("&account={owner}"));
                }
                let response = router
                    .oneshot(request(&path, owner == "caller", Method::GET))
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::OK);
                no_store(&response);
                assert_eq!(
                    response
                        .headers()
                        .contains_key(caller_scope::UPDATED_CREDENTIAL_HEADER),
                    owner == "caller"
                );
                let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
                let v: Value = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(v["data"]["expires_at"], "2027-01-15 08:00:00");
                assert_eq!(v["data"]["extensions"]["date_timezone"], Value::Null);
                assert_eq!(v["data"]["extensions"]["source_user_id"], "123456");
                assert_eq!(v["data"]["active"], true);
                assert!(v["data"]["level"].is_null());
                assert!(!String::from_utf8_lossy(&bytes).contains("member-private-input"));
                assert_eq!(
                    *calls.lock().unwrap(),
                    vec![(
                        user.then(|| "123456".into()),
                        if owner == "caller" {
                            "default".into()
                        } else {
                            owner.into()
                        },
                        backend == "client"
                    )]
                );
            }
        }
    }
}

#[tokio::test]
async fn kugou_membership_http_failed_reads_and_invalid_requests_never_export_credentials() {
    for user in [false, true] {
        let path = if user {
            "/v1/users/kugou:123456/membership"
        } else {
            "/v1/account/membership?platform=kugou"
        };
        for (code, status) in [
            (ErrorCode::AuthenticationRequired, StatusCode::UNAUTHORIZED),
            (ErrorCode::Conflict, StatusCode::CONFLICT),
            (ErrorCode::UpstreamError, StatusCode::BAD_GATEWAY),
            (ErrorCode::UpstreamTimeout, StatusCode::GATEWAY_TIMEOUT),
        ] {
            let (router, _) = app(Some(code));
            let response = router
                .oneshot(request(path, true, Method::GET))
                .await
                .unwrap();
            assert_eq!(response.status(), status);
            no_store(&response);
            assert!(
                !response
                    .headers()
                    .contains_key(caller_scope::UPDATED_CREDENTIAL_HEADER)
            );
            let v: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                    .unwrap();
            assert!(v.get("data").is_none_or(Value::is_null));
        }
    }
    for (method, path, status) in [
        (
            Method::GET,
            "/v1/account/membership?platform=kugou&unknown=1",
            StatusCode::BAD_REQUEST,
        ),
        (
            Method::GET,
            "/v1/account/membership?platform=kugou&backend=invalid",
            StatusCode::BAD_REQUEST,
        ),
        (
            Method::GET,
            "/v1/users/kugou:123456/membership?backend=invalid",
            StatusCode::BAD_REQUEST,
        ),
        (
            Method::POST,
            "/v1/account/membership?platform=kugou",
            StatusCode::METHOD_NOT_ALLOWED,
        ),
        (
            Method::POST,
            "/v1/users/kugou:123456/membership",
            StatusCode::METHOD_NOT_ALLOWED,
        ),
    ] {
        let (router, calls) = app(None);
        let response = router.oneshot(request(path, false, method)).await.unwrap();
        assert_eq!(response.status(), status);
        no_store(&response);
        assert!(calls.lock().unwrap().is_empty());
    }
    for path in [
        "/v1/account/membership?platform=kugou",
        "/v1/users/kugou:123456/membership",
    ] {
        let (router, calls) = app(None);
        let mut req = request(path, false, Method::GET);
        req.headers_mut()
            .insert("x-request-id", HeaderValue::from_static("bad request id"));
        let response = router.oneshot(req).await.unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        no_store(&response);
        assert!(calls.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn kugou_membership_http_ordinary_error_returns_only_verified_typed_caller_update() {
    for path in [
        "/v1/users/kugou:123456/membership",
        "/v1/account/membership?platform=kugou",
    ] {
        for caller in [false, true] {
            let (router, calls) = app_with_typed_update(Some(ErrorCode::UpstreamError), true);
            let response = router
                .oneshot(request(path, caller, Method::GET))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
            no_store(&response);
            let update = response
                .headers()
                .get(caller_scope::UPDATED_CREDENTIAL_HEADER);
            assert_eq!(update.is_some(), caller);
            if let Some(update) = update {
                let expected = CallerCredential::issue(
                    &ProviderCredential::new(
                        Platform::Kugou,
                        "fixture",
                        "independently-verified-update",
                        None,
                    )
                    .unwrap(),
                )
                .unwrap();
                assert_eq!(
                    update.to_str().unwrap(),
                    format!("kugou={}", expected.value)
                );
            }
            let body = to_bytes(response.into_body(), 65536).await.unwrap();
            let text = String::from_utf8_lossy(&body);
            assert!(!text.contains("independently-verified-update"));
            assert!(!text.contains("member-private"));
            let value: Value = serde_json::from_slice(&body).unwrap();
            assert!(value.get("data").is_none_or(Value::is_null));
            assert_eq!(calls.lock().unwrap().len(), 1);
        }
    }
}

#[tokio::test]
async fn kugou_membership_account_finisher_rejects_invalid_typed_update_and_prefers_latest_verified_one()
 {
    for case in [
        "valid",
        "server",
        "wrong-update-platform",
        "wrong-error-platform",
        "missing-error-platform",
        "expired",
        "auth",
        "conflict",
    ] {
        let response = caller_scope::scope(true, async {
            let mut error = TuneWeaveError::new(ErrorCode::UpstreamError, "fixture read failed");
            if case != "missing-error-platform" {
                error.platform = Some(if case == "wrong-error-platform" {
                    Platform::Migu
                } else {
                    Platform::Kugou
                });
            }
            error = error.with_caller_credential_update(
                ProviderCredential::new(
                    if case == "wrong-update-platform" {
                        Platform::Migu
                    } else {
                        Platform::Kugou
                    },
                    "fixture",
                    "latest-verified-private",
                    if case == "expired" { Some(1) } else { None },
                )
                .unwrap(),
            );
            if case == "auth" {
                error.code = ErrorCode::AuthenticationRequired;
            }
            if case == "conflict" {
                error.code = ErrorCode::Conflict;
            }
            let prior =
                ProviderCredential::new(Platform::Kugou, "fixture", "prior-verified-private", None)
                    .unwrap();
            caller_scope::record_update(&CallerCredential::issue(&prior).unwrap());
            let provider = Provider {
                caller: case != "server",
                failure: None,
                typed_update: false,
                calls: Arc::default(),
                update: Arc::new(Mutex::new(if case == "server" {
                    None
                } else {
                    Some(prior)
                })),
            };
            finish_account_operation::<MembershipSummary>(
                &provider,
                Platform::Kugou,
                case != "server",
                Err(error),
            )
            .unwrap_err()
            .into_response()
        })
        .await;
        assert_eq!(
            response.status(),
            match case {
                "valid" => StatusCode::BAD_GATEWAY,
                "auth" => StatusCode::UNAUTHORIZED,
                "conflict" => StatusCode::CONFLICT,
                _ => StatusCode::INTERNAL_SERVER_ERROR,
            }
        );
        no_store(&response);
        let header = response
            .headers()
            .get(caller_scope::UPDATED_CREDENTIAL_HEADER);
        assert_eq!(header.is_some(), case == "valid");
        if let Some(header) = header {
            let expected = CallerCredential::issue(
                &ProviderCredential::new(
                    Platform::Kugou,
                    "fixture",
                    "latest-verified-private",
                    None,
                )
                .unwrap(),
            )
            .unwrap();
            assert_eq!(
                header.to_str().unwrap(),
                format!("kugou={}", expected.value)
            );
        }
        let body = to_bytes(response.into_body(), 65536).await.unwrap();
        assert!(!String::from_utf8_lossy(&body).contains("verified-private"));
    }
}

#[tokio::test]
async fn kugou_membership_http_real_provider_missing_accounts_fail_without_network() {
    let mut registry = ProviderRegistry::new();
    registry
        .register(
            tuneweave_provider_kugou::KugouProvider::new(tuneweave_provider_kugou::KugouConfig {
                proxy_url: Some("http://127.0.0.1:9".into()),
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
    let router = build_router(AppState::new(registry, Platform::Kugou));
    for path in [
        "/v1/account/membership?platform=kugou",
        "/v1/users/kugou:123456/membership?account=missing&backend=client",
    ] {
        let response = router
            .clone()
            .oneshot(request(path, false, Method::GET))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        no_store(&response);
    }
}
