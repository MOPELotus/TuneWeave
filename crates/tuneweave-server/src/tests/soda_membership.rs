//! HTTP contract fixtures; real Provider account/commerce/UID reads are tested in Soda.
use super::*;

type Calls = Arc<Mutex<Vec<(Option<String>, String, bool)>>>;
#[derive(Clone)]
struct Provider {
    caller: bool,
    failure: Option<ErrorCode>,
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
                ProviderCredential::new(Platform::Soda, "fixture", "member-private-rotated", None)
                    .unwrap(),
            );
        }
        if let Some(code) = self.failure {
            return Err(
                TuneWeaveError::new(code, "Membership read failed").with_platform(Platform::Soda)
            );
        }
        Ok(MembershipSummary {
            user_ref: Some(ResourceRef::new(Platform::Soda, "123456").unwrap()),
            active: Some(true),
            expires_at: Some("2027-01-15T08:00:00Z".into()),
            level: None,
            annual_count: None,
            icon_url: None,
            extensions: Extensions::from([
                (
                    "backend".into(),
                    json!("official_pc_commerce_membership_v2"),
                ),
                ("source_user_id".into(), json!("123456")),
                ("expires_at_epoch_seconds".into(), json!(1800000000)),
                ("membership_type".into(), json!("vip")),
            ]),
        })
    }
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Soda
    }
    fn name(&self) -> &'static str {
        "Soda membership contract"
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
fn request(path: &str, caller: bool, method: Method) -> Request<Body> {
    let mut r = Request::builder().uri(path).method(method);
    if caller {
        r = r.header(
            CALLER_CREDENTIAL_HEADER,
            CallerCredential::issue(
                &ProviderCredential::new(Platform::Soda, "fixture", "member-private-input", None)
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
async fn soda_membership_http_preserves_expiry_sources_backends_and_caller_update() {
    for owner in ["default", "personal", "caller"] {
        for user in [false, true] {
            for backend in ["front", "client"] {
                let (router, calls) = app(None);
                let mut path = if user {
                    "/v1/users/soda:123456/membership?".to_owned()
                } else {
                    "/v1/account/membership?platform=soda&".to_owned()
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
                assert_eq!(v["data"]["expires_at"], "2027-01-15T08:00:00Z");
                assert_eq!(
                    v["data"]["extensions"]["expires_at_epoch_seconds"],
                    1800000000
                );
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
async fn soda_membership_http_failed_reads_and_invalid_requests_never_export_credentials() {
    for user in [false, true] {
        let path = if user {
            "/v1/users/soda:123456/membership"
        } else {
            "/v1/account/membership?platform=soda"
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
            "/v1/account/membership?platform=soda&unknown=1",
            StatusCode::BAD_REQUEST,
        ),
        (
            Method::GET,
            "/v1/account/membership?platform=soda&backend=invalid",
            StatusCode::BAD_REQUEST,
        ),
        (
            Method::GET,
            "/v1/users/soda:123456/membership?backend=invalid",
            StatusCode::BAD_REQUEST,
        ),
        (
            Method::POST,
            "/v1/account/membership?platform=soda",
            StatusCode::METHOD_NOT_ALLOWED,
        ),
        (
            Method::POST,
            "/v1/users/soda:123456/membership",
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
        "/v1/account/membership?platform=soda",
        "/v1/users/soda:123456/membership",
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
async fn soda_membership_http_real_provider_missing_accounts_fail_without_network() {
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
    for path in [
        "/v1/account/membership?platform=soda",
        "/v1/users/soda:123456/membership?account=missing&backend=client",
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
