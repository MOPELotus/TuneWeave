use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use tuneweave_core::{AuthAccountChoice, AuthBrowserChallenge};

struct SmsProvider {
    mode: CredentialMode,
    allow_creation: bool,
    browser: bool,
    stage: Mutex<u8>,
    calls: Arc<AtomicUsize>,
}
fn browser_selection() -> AuthChallengeStatus {
    AuthChallengeStatus::BrowserVerificationRequired { verification: AuthBrowserChallenge {
        verification_id: "opaque-browser-id".into(),
        url: "https://h5.kugou.com/apps/verify-h5/dist/#/index/synthetic-event/1014/null/synthetic-mid/TuneWeaveVerify".into(),
        message_origin: "https://h5.kugou.com".into(), message_type: "kgVerifyCallbackData".into(),
        response_field: "dataJson".into(), remaining_attempts: 4,
    } }
}
fn selection() -> AuthChallengeStatus {
    AuthChallengeStatus::AccountSelectionRequired {
        accounts: vec![AuthAccountChoice {
            user_id: "222".into(),
            nickname: Some("Selected listener".into()),
            avatar_url: None,
        }],
    }
}
#[async_trait]
impl MusicProvider for SmsProvider {
    fn platform(&self) -> Platform {
        Platform::Kugou
    }
    fn name(&self) -> &'static str {
        "KuGou SMS HTTP contract fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::PhoneLogin, Capability::CallerManagedCredentials])
    }
    async fn begin_auth_challenge(
        &self,
        request: &AuthChallengeRequest,
        mode: CredentialMode,
    ) -> Result<ProviderAuthChallenge> {
        assert_eq!(mode, self.mode);
        assert_eq!(request.allow_account_creation, self.allow_creation);
        assert_eq!(request.principal, "13800000000");
        ProviderAuthChallenge::stateful(
            Platform::Kugou,
            request.clone(),
            mode,
            "private-sms-receipt".into(),
        )
    }
    async fn auth_challenge_status(
        &self,
        _: &ProviderAuthChallenge,
    ) -> Result<AuthChallengeStatus> {
        Ok(if *self.stage.lock().unwrap() == 0 {
            AuthChallengeStatus::Waiting
        } else if self.browser && *self.stage.lock().unwrap() == 1 {
            browser_selection()
        } else {
            selection()
        })
    }
    async fn advance_auth_challenge(
        &self,
        receipt: &ProviderAuthChallenge,
        action: &AuthChallengeAction,
    ) -> Result<AuthChallengeProgress> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        assert_eq!(receipt.credential_mode(), self.mode);
        assert_eq!(
            receipt.request().allow_account_creation,
            self.allow_creation
        );
        assert_eq!(
            receipt.provider_transaction_id(),
            Some("private-sms-receipt")
        );
        let mut stage = self.stage.lock().unwrap();
        match action {
            AuthChallengeAction::SubmitCode { code } if *stage == 0 && code == "123456" => {
                *stage = 1;
                Ok(AuthChallengeProgress::Pending(if self.browser {
                    browser_selection()
                } else {
                    selection()
                }))
            }
            AuthChallengeAction::SubmitBrowser {
                verification_id,
                code,
                response,
            } if self.browser
                && *stage == 1
                && verification_id == "opaque-browser-id"
                && code == "123456"
                && response == "synthetic-browser-callback" =>
            {
                *stage = 2;
                Ok(AuthChallengeProgress::Pending(selection()))
            }
            AuthChallengeAction::SubmitBrowser { .. } if self.browser && *stage == 1 => {
                Err(TuneWeaveError::invalid_request("Invalid browser callback"))
            }
            AuthChallengeAction::SelectAccount { user_id, code }
                if *stage == (if self.browser { 2 } else { 1 })
                    && user_id == "222"
                    && code == "123456" =>
            {
                *stage = 2;
                let mut profile =
                    AccountProfile::authenticated(Platform::Kugou, &receipt.request().account);
                profile.user_id = Some(user_id.clone());
                Ok(AuthChallengeProgress::Confirmed(ProviderAuthResult {
                    profile,
                    credential: self.mode.returns_to_caller().then(|| {
                        ProviderCredential::new(
                            Platform::Kugou,
                            "kugou_web_v1",
                            "synthetic-sms-verified-session",
                            None,
                        )
                        .unwrap()
                    }),
                }))
            }
            AuthChallengeAction::SelectAccount { code, .. } if code == "000000" => {
                Err(TuneWeaveError::new(
                    ErrorCode::AuthenticationRequired,
                    "SMS transaction consumed",
                )
                .with_consumed_auth_challenge())
            }
            _ => Err(TuneWeaveError::invalid_request("Invalid SMS action")),
        }
    }
}
fn fixture(
    mode: CredentialMode,
    allow_creation: bool,
) -> (axum::Router, AppState, Arc<AtomicUsize>) {
    fixture_with_browser(mode, allow_creation, false)
}
fn fixture_with_browser(
    mode: CredentialMode,
    allow_creation: bool,
    browser: bool,
) -> (axum::Router, AppState, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = ProviderRegistry::new();
    registry
        .register(SmsProvider {
            mode,
            allow_creation,
            browser,
            stage: Mutex::new(0),
            calls: calls.clone(),
        })
        .unwrap();
    let state = AppState::new(registry, Platform::Kugou);
    (build_router(state.clone()), state, calls)
}
async fn request(
    app: axum::Router,
    method: Method,
    path: &str,
    body: Option<Value>,
    expected: StatusCode,
) -> Value {
    let (status, headers, value) = json_request_with_headers(app, method, path, body).await;
    assert_eq!(status, expected, "{value}");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert!(
        headers
            .get(caller_scope::UPDATED_CREDENTIAL_HEADER)
            .is_none()
    );
    for secret in [
        "13800000000",
        "123456",
        "private-sms-receipt",
        "synthetic-sms-verified-session",
    ] {
        assert!(!value.to_string().contains(secret), "{value}");
    }
    value
}
async fn begin(app: axum::Router, mode: CredentialMode, allow_creation: bool) -> String {
    let mut body = json!({"platform":"kugou","principal":"13800000000","credential_mode":mode,"allow_account_creation":allow_creation});
    if mode != CredentialMode::Client {
        body["account"] = json!("personal");
    }
    let value = request(
        app,
        Method::POST,
        "/v1/auth/challenges",
        Some(body),
        StatusCode::OK,
    )
    .await;
    assert_eq!(value["data"]["state"], "waiting");
    assert!(value["data"].get("profile").is_none());
    assert!(value["data"].get("caller_credential").is_none());
    value["data"]["transaction_id"].as_str().unwrap().to_owned()
}

#[tokio::test]
async fn kugou_sms_http_keeps_account_selection_in_the_original_transaction_and_ownership() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        for allow in [false, true] {
            let (app, state, calls) = fixture(mode, allow);
            let id = begin(app.clone(), mode, allow).await;
            let path = format!("/v1/auth/challenges/{id}/verify");
            let pending = request(
                app.clone(),
                Method::POST,
                &path,
                Some(json!({"code":"123456"})),
                StatusCode::OK,
            )
            .await;
            assert_eq!(pending["data"]["state"], "account_selection_required");
            assert_eq!(pending["data"]["accounts"][0]["user_id"], "222");
            assert!(pending["data"].get("profile").is_none());
            assert!(pending["data"].get("caller_credential").is_none());
            let stored = state.auth_transactions.get(&id).unwrap();
            let StoredAuthKind::Challenge {
                request: original,
                provider_challenge: Some(receipt),
                ..
            } = stored.kind
            else {
                panic!()
            };
            assert_eq!(original.allow_account_creation, allow);
            assert_eq!(&original, receipt.request());
            assert_eq!(receipt.credential_mode(), mode);
            let result = request(
                app.clone(),
                Method::POST,
                &path,
                Some(json!({"action":"select_account","user_id":"222","code":"123456"})),
                StatusCode::OK,
            )
            .await;
            assert_eq!(result["data"]["state"], "confirmed");
            assert_eq!(result["data"]["profile"]["user_id"], "222");
            assert_eq!(
                result["data"]["profile"]["account"],
                if mode == CredentialMode::Client {
                    "default"
                } else {
                    "personal"
                }
            );
            assert_eq!(
                result["data"].get("caller_credential").is_some(),
                mode.returns_to_caller()
            );
            if mode.returns_to_caller() {
                let caller = CallerCredential::parse(
                    result["data"]["caller_credential"]["value"]
                        .as_str()
                        .unwrap(),
                )
                .unwrap();
                assert_eq!(caller.platform, Platform::Kugou);
                assert_eq!(caller.kind, "kugou_web_v1");
            }
            assert!(state.auth_transactions.get(&id).is_err());
            assert_eq!(calls.load(Ordering::SeqCst), 2);
            request(
                app,
                Method::POST,
                &path,
                Some(json!({"code":"123456"})),
                StatusCode::NOT_FOUND,
            )
            .await;
            assert_eq!(calls.load(Ordering::SeqCst), 2);
        }
    }
}

#[tokio::test]
async fn kugou_sms_http_rejects_bad_actions_without_losing_choices_and_consumed_errors_cannot_replay()
 {
    let (app, state, calls) = fixture(CredentialMode::Client, false);
    let id = begin(app.clone(), CredentialMode::Client, false).await;
    let path = format!("/v1/auth/challenges/{id}/verify");
    request(
        app.clone(),
        Method::POST,
        &path,
        Some(json!({"action":"submit_code","code":"123456"})),
        StatusCode::OK,
    )
    .await;
    for body in [
        json!({"action":"select_account","user_id":"222","code":123456}),
        json!({"action":"select_account","user_id":222,"code":"123456"}),
        json!({"action":"select_account","user_id":"222"}),
        json!({"action":"select_account","user_id":"222","code":"123456","allow_account_creation":true}),
        json!({"action":"select_account","user_id":"222","code":"123456","account":"other"}),
    ] {
        request(
            app.clone(),
            Method::POST,
            &path,
            Some(body),
            StatusCode::BAD_REQUEST,
        )
        .await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(state.auth_transactions.get(&id).is_ok());
    }
    for body in [
        json!({"action":"select_account","user_id":"999","code":"123456"}),
        json!({"action":"submit_code","code":"123456"}),
    ] {
        request(
            app.clone(),
            Method::POST,
            &path,
            Some(body),
            StatusCode::BAD_REQUEST,
        )
        .await;
        assert!(state.auth_transactions.get(&id).is_ok());
    }
    let consumed = request(
        app.clone(),
        Method::POST,
        &path,
        Some(json!({"action":"select_account","user_id":"222","code":"000000"})),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    assert_eq!(consumed["error"]["details"]["challenge_consumed"], true);
    assert!(state.auth_transactions.get(&id).is_err());
    let count = calls.load(Ordering::SeqCst);
    request(
        app,
        Method::POST,
        &path,
        Some(json!({"action":"select_account","user_id":"222","code":"123456"})),
        StatusCode::NOT_FOUND,
    )
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), count);
}

#[tokio::test]
async fn kugou_sms_http_rejects_invalid_real_provider_inputs_before_network_and_releases_capacity()
{
    let guard = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    guard.set_nonblocking(true).unwrap();
    let mut registry = ProviderRegistry::new();
    registry
        .register(
            tuneweave_provider_kugou::KugouProvider::new(tuneweave_provider_kugou::KugouConfig {
                proxy_url: Some(format!("http://{}", guard.local_addr().unwrap())),
                ..Default::default()
            })
            .unwrap(),
        )
        .unwrap();
    let state = AppState::new(registry, Platform::Kugou);
    let app = build_router(state.clone());
    for (field, value) in [
        ("principal", json!("bad-phone")),
        ("country_code", json!("1")),
        ("backend", json!("middle")),
        ("account", json!("other")),
        ("method", json!("email")),
        ("allow_account_creation", json!("true")),
        ("allow_account_creation", Value::Null),
    ] {
        let mut body =
            json!({"platform":"kugou","principal":"13800000000","credential_mode":"client"});
        body[field] = value;
        request(
            app.clone(),
            Method::POST,
            "/v1/auth/challenges",
            Some(body),
            StatusCode::BAD_REQUEST,
        )
        .await;
        assert_eq!(state.auth_transactions.counts().unwrap().total, 0);
    }
    assert!(matches!(guard.accept(), Err(e) if e.kind() == std::io::ErrorKind::WouldBlock));
}

#[tokio::test]
async fn kugou_sms_creation_option_does_not_enable_registration_for_other_platforms() {
    struct Unsupported(Platform);
    #[async_trait]
    impl MusicProvider for Unsupported {
        fn platform(&self) -> Platform {
            self.0
        }
        fn name(&self) -> &'static str {
            "Unsupported registration fixture"
        }
        fn capabilities(&self) -> BTreeSet<Capability> {
            BTreeSet::from([Capability::PhoneLogin])
        }
        async fn begin_auth_challenge(
            &self,
            _: &AuthChallengeRequest,
            _: CredentialMode,
        ) -> Result<ProviderAuthChallenge> {
            panic!("registration option must be rejected before calling this provider")
        }
    }
    for platform in [
        Platform::Migu,
        Platform::Soda,
        Platform::Netease,
        Platform::Qq,
        Platform::Bilibili,
    ] {
        let mut registry = ProviderRegistry::new();
        registry.register(Unsupported(platform)).unwrap();
        let state = AppState::new(registry, platform);
        request(build_router(state.clone()), Method::POST, "/v1/auth/challenges",
            Some(json!({"platform":platform,"principal":"13800000000","credential_mode":"client","allow_account_creation":true})),
            StatusCode::BAD_REQUEST).await;
        assert_eq!(state.auth_transactions.counts().unwrap().total, 0);
    }
}

#[tokio::test]
async fn kugou_sms_browser_http_stays_in_original_transaction_and_ownership_until_uid_confirmation()
{
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let (app, state, _) = fixture_with_browser(mode, false, true);
        let id = begin(app.clone(), mode, false).await;
        let path = format!("/v1/auth/challenges/{id}/verify");
        let before = state.auth_transactions.get(&id).unwrap();
        let pending = request(
            app.clone(),
            Method::POST,
            &path,
            Some(json!({"code":"123456"})),
            StatusCode::OK,
        )
        .await;
        assert_eq!(pending["data"]["state"], "browser_verification_required");
        assert_eq!(
            pending["data"]["verification"]["message_origin"],
            "https://h5.kugou.com"
        );
        assert_eq!(
            pending["data"]["verification"]["response_field"],
            "dataJson"
        );
        assert!(pending["data"].get("caller_credential").is_none());
        assert!(pending["data"].get("profile").is_none());
        for body in [
            json!({"action":"submit_browser","verification_id":"other-id","code":"123456","response":"synthetic-browser-callback"}),
            json!({"action":"submit_browser","verification_id":"opaque-browser-id","code":"123456","response":"bad"}),
            json!({"action":"submit_browser","verification_id":"opaque-browser-id","code":"123456","response":"synthetic-browser-callback","allow_account_creation":true}),
            json!({"action":"submit_browser","verification_id":"opaque-browser-id","code":"123456","response":{}}),
            json!({"action":"submit_browser","verification_id":"opaque-browser-id","code":"123456"}),
        ] {
            request(
                app.clone(),
                Method::POST,
                &path,
                Some(body),
                StatusCode::BAD_REQUEST,
            )
            .await;
            let after = state.auth_transactions.get(&id).unwrap();
            assert_eq!(after.created_at, before.created_at);
            assert_eq!(after.expires_at, before.expires_at);
        }
        let selection = request(app.clone(),Method::POST,&path,Some(json!({"action":"submit_browser","verification_id":"opaque-browser-id","code":"123456","response":"synthetic-browser-callback"})),StatusCode::OK).await;
        assert_eq!(selection["data"]["state"], "account_selection_required");
        assert!(selection["data"].get("caller_credential").is_none());
        let result = request(
            app.clone(),
            Method::POST,
            &path,
            Some(json!({"action":"select_account","user_id":"222","code":"123456"})),
            StatusCode::OK,
        )
        .await;
        assert_eq!(result["data"]["state"], "confirmed");
        assert_eq!(
            result["data"].get("caller_credential").is_some(),
            mode.returns_to_caller()
        );
        assert!(state.auth_transactions.get(&id).is_err());
        request(app,Method::POST,&path,Some(json!({"action":"submit_browser","verification_id":"opaque-browser-id","code":"123456","response":"synthetic-browser-callback"})),StatusCode::NOT_FOUND).await;
    }
}

#[tokio::test]
async fn kugou_sms_browser_http_early_rejections_and_provider_consumption_remain_no_store() {
    let (app, state, _) = fixture_with_browser(CredentialMode::Client, false, true);
    let id = begin(app.clone(), CredentialMode::Client, false).await;
    let path = format!("/v1/auth/challenges/{id}/verify");
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::GET)
                .uri(&path)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    request(
        app.clone(),
        Method::POST,
        &path,
        Some(json!({"code":"123456"})),
        StatusCode::OK,
    )
    .await;
    request(app.clone(),Method::POST,&path,Some(json!({"action":"submit_browser","verification_id":"opaque-browser-id","code":"123456","response":"synthetic-browser-callback"})),StatusCode::OK).await;
    request(
        app.clone(),
        Method::POST,
        &path,
        Some(json!({"action":"select_account","user_id":"222","code":"000000"})),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    assert!(state.auth_transactions.get(&id).is_err());
    request(
        app,
        Method::POST,
        &path,
        Some(json!({"code":"123456"})),
        StatusCode::NOT_FOUND,
    )
    .await;
}
