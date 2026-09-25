use super::*;

type MembershipCall = (bool, Option<String>, String);

struct Provider {
    caller: bool,
    failure: Option<ErrorCode>,
    update: Mutex<Option<ProviderCredential>>,
    calls: Arc<Mutex<Vec<MembershipCall>>>,
}
impl Provider {
    fn read(
        &self,
        client: bool,
        id: Option<&str>,
        account: Option<&str>,
    ) -> Result<MembershipSummary> {
        let account = account.unwrap();
        assert_eq!(account, if self.caller { "default" } else { "A" });
        self.calls
            .lock()
            .unwrap()
            .push((client, id.map(str::to_owned), account.to_owned()));
        if self.caller {
            *self.update.lock().unwrap() = Some(
                ProviderCredential::new(Platform::Migu, "test", "updated-member-session", None)
                    .unwrap(),
            );
        }
        if let Some(code) = self.failure {
            return Err(TuneWeaveError::new(code, "membership fixture failure")
                .with_platform(Platform::Migu));
        }
        Ok(MembershipSummary {
            user_ref: Some(ResourceRef::new(Platform::Migu, "111").unwrap()),
            level: None,
            active: Some(true),
            annual_count: None,
            expires_at: Some("2026-10-01".into()),
            icon_url: None,
            extensions: Extensions::from([("backend".into(), json!("official_member_center_v3"))]),
        })
    }
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Migu
    }
    fn name(&self) -> &'static str {
        "Migu membership contract fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::UserMembership,
            Capability::UserMembershipClientInfo,
            Capability::CallerManagedCredentials,
        ])
    }
    fn with_caller_credential(&self, c: &ProviderCredential) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(c.platform, Platform::Migu);
        assert_eq!(c.secret(), "original-member-session");
        Ok(Arc::new(Self {
            caller: true,
            failure: self.failure,
            update: Mutex::new(None),
            calls: self.calls.clone(),
        }))
    }
    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        Ok(self.update.lock().unwrap().take())
    }
    async fn user_membership(
        &self,
        id: Option<&str>,
        account: Option<&str>,
    ) -> Result<MembershipSummary> {
        self.read(false, id, account)
    }
    async fn user_membership_client_info(
        &self,
        id: Option<&str>,
        account: Option<&str>,
    ) -> Result<MembershipSummary> {
        self.read(true, id, account)
    }
}

#[tokio::test]
async fn migu_membership_routes_preserve_selection_dispatch_expiry_and_error_credential_rules() {
    for caller in [false, true] {
        for client in [false, true] {
            for user_route in [false, true] {
                for (failure, status) in [
                    (None, StatusCode::OK),
                    (Some(ErrorCode::UpstreamError), StatusCode::BAD_GATEWAY),
                    (Some(ErrorCode::PermissionDenied), StatusCode::FORBIDDEN),
                    (
                        Some(ErrorCode::AuthenticationRequired),
                        StatusCode::UNAUTHORIZED,
                    ),
                    (Some(ErrorCode::Conflict), StatusCode::CONFLICT),
                ] {
                    let calls = Arc::new(Mutex::new(vec![]));
                    let mut registry = ProviderRegistry::new();
                    registry
                        .register(Provider {
                            caller: false,
                            failure,
                            update: Mutex::new(None),
                            calls: calls.clone(),
                        })
                        .unwrap();
                    let app = build_router(AppState::new(registry, Platform::Migu));
                    let base = if user_route {
                        "/v1/users/migu:111/membership?"
                    } else {
                        "/v1/account/membership?platform=migu&"
                    };
                    let uri = format!(
                        "{base}backend={}{}",
                        if client { "client" } else { "front" },
                        if caller { "" } else { "&account=A" }
                    );
                    let mut request = Request::builder().uri(uri);
                    if caller {
                        request = request.header(
                            CALLER_CREDENTIAL_HEADER,
                            CallerCredential::issue(
                                &ProviderCredential::new(
                                    Platform::Migu,
                                    "test",
                                    "original-member-session",
                                    None,
                                )
                                .unwrap(),
                            )
                            .unwrap()
                            .value,
                        );
                    }
                    let response = app
                        .oneshot(request.body(Body::empty()).unwrap())
                        .await
                        .unwrap();
                    assert_eq!(response.status(), status);
                    assert!(
                        response.headers()[header::CACHE_CONTROL]
                            .to_str()
                            .unwrap()
                            .contains("no-store")
                    );
                    let update = response
                        .headers()
                        .get("X-TuneWeave-Updated-Credential")
                        .map(|v| v.to_str().unwrap().to_owned());
                    let body: Value = serde_json::from_slice(
                        &to_bytes(response.into_body(), 65536).await.unwrap(),
                    )
                    .unwrap();
                    let should_update = caller
                        && !matches!(
                            failure,
                            Some(ErrorCode::AuthenticationRequired | ErrorCode::Conflict)
                        );
                    assert_eq!(update.is_some(), should_update);
                    assert_eq!(body["meta"]["caller_credential"].is_object(), should_update);
                    if should_update {
                        let header = update.unwrap();
                        let value = header.strip_prefix("migu=").unwrap();
                        assert_eq!(
                            CallerCredential::parse(value).unwrap().secret(),
                            "updated-member-session"
                        );
                        assert_eq!(body["meta"]["caller_credential"]["value"], value);
                    }
                    if failure.is_none() {
                        assert_eq!(body["data"]["user_ref"], "migu:111");
                        assert_eq!(body["data"]["active"], true);
                        assert_eq!(body["data"]["expires_at"], "2026-10-01");
                        assert!(body["data"]["level"].is_null());
                        if caller {
                            assert!(body["meta"].get("account").is_none());
                        } else {
                            assert_eq!(body["meta"]["account"], "A");
                        }
                    }
                    assert_eq!(
                        *calls.lock().unwrap(),
                        vec![(
                            client,
                            user_route.then(|| "111".to_owned()),
                            if caller {
                                "default".to_owned()
                            } else {
                                "A".to_owned()
                            }
                        )]
                    );
                }
            }
        }
    }
}
