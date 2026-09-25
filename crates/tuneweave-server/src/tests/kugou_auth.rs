use super::*;
use tuneweave_core::{PasswordLoginBackend, ProviderQrPoll};

mod web_secondary;

struct NativePasswordVerificationProvider {
    mode: CredentialMode,
    browser: bool,
    secondary_sms: bool,
    receipt: Mutex<Option<ProviderPasswordChallenge>>,
}

#[async_trait]
impl MusicProvider for NativePasswordVerificationProvider {
    fn platform(&self) -> Platform {
        Platform::Kugou
    }
    fn name(&self) -> &'static str {
        "KuGou native password verification HTTP fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::PasswordLogin,
            Capability::CallerManagedCredentials,
        ])
    }
    async fn begin_password_login(
        &self,
        request: &PasswordLoginRequest,
        mode: CredentialMode,
    ) -> Result<PasswordLoginProgress> {
        assert_eq!(mode, self.mode);
        assert_eq!(request.backend, PasswordLoginBackend::Native);
        assert_eq!(request.password, "first-password");
        let receipt = ProviderPasswordChallenge::new(
            Platform::Kugou,
            PasswordLoginIdentity::from(request),
            mode,
            "private-provider-id".into(),
        )?;
        *self.receipt.lock().unwrap() = Some(receipt.clone());
        Ok(PasswordLoginProgress::Pending {
            challenge: receipt,
            verification: if self.secondary_sms {
                PasswordVerification::Sms {
                    masked_destination: "账号绑定手机".into(),
                    remaining_attempts: 5,
                    resend_after_secs: 31,
                }
            } else if self.browser {
                PasswordVerification::Browser {
                    protocol: tuneweave_core::PasswordBrowserProtocol::KugouNativeBridge,
                    verification_id: "http-browser-id".into(),
                    url: "https://h5.kugou.com/apps/h5Verify/verify.html?thisurl=KGCodeTX%7C12345"
                        .into(),
                    device_id: "synthetic-mid".into(),
                    client_version: 20809,
                    remaining_attempts: 5,
                }
            } else {
                PasswordVerification::Image {
                    image: tuneweave_core::AuthImageChallenge {
                        image_data_url: "data:image/png;base64,aW1hZ2U=".into(),
                        answer_kind: tuneweave_core::AuthImageAnswerKind::Alphanumeric,
                        remaining_attempts: 5,
                        refresh_after_secs: 2,
                    },
                }
            },
        })
    }
    async fn advance_password_login(
        &self,
        receipt: &ProviderPasswordChallenge,
        action: &PasswordChallengeAction,
    ) -> Result<PasswordLoginProgress> {
        assert_eq!(self.receipt.lock().unwrap().as_ref(), Some(receipt));
        assert_eq!(receipt.identity().backend, PasswordLoginBackend::Native);
        if self.secondary_sms {
            match action {
                PasswordChallengeAction::SubmitSms { code } if code == "123456" => {
                    return Ok(PasswordLoginProgress::Pending {
                        challenge: receipt.clone(),
                        verification: PasswordVerification::AccountSelection {
                            accounts: vec![tuneweave_core::AuthAccountChoice {
                                user_id: "111".into(),
                                nickname: Some("Test listener".into()),
                                avatar_url: None,
                            }],
                            remaining_attempts: 4,
                        },
                    });
                }
                PasswordChallengeAction::SelectAccount { user_id, code }
                    if user_id == "111" && code == "123456" => {}
                _ => {
                    return Err(TuneWeaveError::invalid_request(
                        "Unexpected native password continuation",
                    ));
                }
            }
        } else {
            assert_eq!(
                action,
                &if self.browser {
                    PasswordChallengeAction::SubmitBrowser {
                        verification_id: "http-browser-id".into(),
                        response: r#"{"close":0,"ticket":"private-ticket"}"#.into(),
                        password: "resubmitted-password".into(),
                    }
                } else {
                    PasswordChallengeAction::SubmitImage {
                        answer: "A7b9".into(),
                        password: "resubmitted-password".into(),
                    }
                }
            );
        }
        self.receipt.lock().unwrap().take();
        let mut profile =
            AccountProfile::authenticated(Platform::Kugou, &receipt.identity().account);
        profile.user_id = Some("111".into());
        Ok(PasswordLoginProgress::Confirmed(ProviderAuthResult {
            profile,
            credential: self.mode.returns_to_caller().then(|| {
                ProviderCredential::new(
                    Platform::Kugou,
                    "kugou_native_v1",
                    "synthetic-http-session",
                    None,
                )
                .unwrap()
            }),
        }))
    }
}

#[tokio::test]
async fn native_password_image_http_preserves_binding_and_ownership_until_confirmation() {
    native_password_verification_http(false, false).await;
}

#[tokio::test]
async fn native_password_browser_http_preserves_binding_and_ownership_until_confirmation() {
    native_password_verification_http(true, false).await;
}

#[tokio::test]
async fn native_password_secondary_sms_http_keeps_account_selection_in_the_original_transaction() {
    native_password_verification_http(false, true).await;
}

async fn native_password_verification_http(browser: bool, secondary_sms: bool) {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let mut registry = ProviderRegistry::new();
        registry
            .register(NativePasswordVerificationProvider {
                mode,
                browser,
                secondary_sms,
                receipt: Mutex::new(None),
            })
            .unwrap();
        let app = build_router(AppState::new(registry, Platform::Kugou));
        let account = if mode == CredentialMode::Client {
            "default"
        } else {
            "personal"
        };
        let mut body = json!({"platform":"kugou","backend":"native","principal_type":"phone",
            "principal":"13800138000","password":"first-password","credential_mode":mode});
        if mode.persists_on_server() {
            body["account"] = json!(account);
        }
        let value = inspect(
            call(
                app.clone(),
                Method::POST,
                "/v1/auth/password",
                Some(body),
                None,
            )
            .await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(value["data"]["state"], "verification_required");
        let verification = &value["data"]["verification"];
        if secondary_sms {
            assert_eq!(verification["method"], "sms");
            assert_eq!(verification["masked_destination"], "账号绑定手机");
            assert_eq!(verification["resend_after_secs"], 31);
        } else if browser {
            assert_eq!(verification["method"], "browser");
            assert_eq!(verification["protocol"], "kugou_native_bridge");
            assert_eq!(verification["verification_id"], "http-browser-id");
            assert_eq!(
                verification["url"],
                "https://h5.kugou.com/apps/h5Verify/verify.html?thisurl=KGCodeTX%7C12345"
            );
            assert_eq!(verification["device_id"], "synthetic-mid");
            assert_eq!(verification["client_version"], 20809);
        } else {
            assert_eq!(verification["method"], "image");
            assert_eq!(verification["image"]["answer_kind"], "alphanumeric");
        }
        assert!(value["data"].get("caller_credential").is_none());
        assert!(!value.to_string().contains("13800138000"));
        assert!(!value.to_string().contains("first-password"));
        let id = value["data"]["transaction_id"].as_str().unwrap();
        let path = format!("/v1/auth/password/challenges/{id}/verify");
        let action = if secondary_sms {
            let pending = inspect(
                call(
                    app.clone(),
                    Method::POST,
                    &path,
                    Some(json!({"action":"submit_sms","code":"123456"})),
                    None,
                )
                .await,
                StatusCode::OK,
            )
            .await;
            assert_eq!(pending["data"]["transaction_id"], id);
            assert_eq!(pending["data"]["state"], "verification_required");
            assert_eq!(
                pending["data"]["verification"]["method"],
                "account_selection"
            );
            assert_eq!(
                pending["data"]["verification"]["accounts"][0]["user_id"],
                "111"
            );
            assert!(pending["data"].get("caller_credential").is_none());
            assert!(!pending.to_string().contains("123456"));
            for invalid in [
                json!({"action":"select_account","user_id":"111"}),
                json!({"action":"select_account","user_id":"111","code":"123456","password":"extra"}),
                json!({"action":"select_account","user_id":"222","code":"123456"}),
            ] {
                inspect(
                    call(app.clone(), Method::POST, &path, Some(invalid), None).await,
                    StatusCode::BAD_REQUEST,
                )
                .await;
            }
            json!({"action":"select_account","user_id":"111","code":"123456"})
        } else if browser {
            json!({"action":"submit_browser","verification_id":"http-browser-id","response":r#"{"close":0,"ticket":"private-ticket"}"#,"password":"resubmitted-password"})
        } else {
            json!({"action":"submit_image","answer":"A7b9","password":"resubmitted-password"})
        };
        let mut forged = action.clone();
        forged["backend"] = json!("web");
        inspect(
            call(app.clone(), Method::POST, &path, Some(forged), None).await,
            StatusCode::BAD_REQUEST,
        )
        .await;
        let confirmed = inspect(
            call(app.clone(), Method::POST, &path, Some(action.clone()), None).await,
            StatusCode::OK,
        )
        .await;
        assert_eq!(confirmed["data"]["account"], account);
        assert_eq!(
            confirmed["data"].get("caller_credential").is_some(),
            mode.returns_to_caller()
        );
        assert!(!confirmed.to_string().contains("resubmitted-password"));
        if mode.returns_to_caller() {
            assert_eq!(
                CallerCredential::parse(
                    confirmed["data"]["caller_credential"]["value"]
                        .as_str()
                        .unwrap()
                )
                .unwrap()
                .kind,
                "kugou_native_v1"
            );
        }
        inspect(
            call(app, Method::POST, &path, Some(action), None).await,
            StatusCode::NOT_FOUND,
        )
        .await;
    }
}

struct PasswordProvider {
    backend: PasswordLoginBackend,
    mode: CredentialMode,
    verification: bool,
}
#[async_trait]
impl MusicProvider for PasswordProvider {
    fn platform(&self) -> Platform {
        Platform::Kugou
    }
    fn name(&self) -> &'static str {
        "KuGou password HTTP contract fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::PasswordLogin,
            Capability::CallerManagedCredentials,
        ])
    }
    async fn password_login_with_mode(
        &self,
        request: &PasswordLoginRequest,
        mode: CredentialMode,
    ) -> Result<ProviderAuthResult> {
        assert_eq!(mode, self.mode);
        assert_eq!(request.backend, self.backend);
        assert_eq!(request.principal_type, PrincipalType::Phone);
        assert_eq!(request.principal, "13800000000");
        assert_eq!(request.password, "synthetic-password");
        assert_eq!(request.password_format, PasswordFormat::Plain);
        if self.verification {
            return Err(TuneWeaveError::new(ErrorCode::PermissionDenied,"KuGou Web password login was rejected").with_platform(Platform::Kugou).with_details(json!({"platform_code":30767,"additional_verification_required":true,"verification_kind":"phone"})));
        }
        let mut profile = AccountProfile::authenticated(Platform::Kugou, &request.account);
        profile.user_id = Some("111".into());
        Ok(ProviderAuthResult {
            profile,
            credential: mode.returns_to_caller().then(|| {
                ProviderCredential::new(
                    Platform::Kugou,
                    if self.backend == PasswordLoginBackend::Native {
                        "kugou_native_v1"
                    } else {
                        "kugou_web_v1"
                    },
                    "synthetic-http-session",
                    None,
                )
                .unwrap()
            }),
        })
    }
}

#[tokio::test]
async fn kugou_password_http_preserves_ownership_and_reports_verification_without_credentials() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        for backend in [
            PasswordLoginBackend::Default,
            PasswordLoginBackend::Web,
            PasswordLoginBackend::Native,
        ] {
            for verification in [false, true] {
                let mut registry = ProviderRegistry::new();
                registry
                    .register(PasswordProvider {
                        backend,
                        mode,
                        verification,
                    })
                    .unwrap();
                let app = build_router(AppState::new(registry, Platform::Kugou));
                let mut body = json!({"platform":"kugou","principal_type":"phone","principal":"13800000000","password":"synthetic-password","credential_mode":mode});
                if backend != PasswordLoginBackend::Default {
                    body["backend"] = json!(backend);
                }
                if mode != CredentialMode::Client {
                    body["account"] = json!("personal");
                }
                let value = inspect(
                    call(app, Method::POST, "/v1/auth/password", Some(body), None).await,
                    if verification {
                        StatusCode::FORBIDDEN
                    } else {
                        StatusCode::OK
                    },
                )
                .await;
                assert!(!value.to_string().contains("synthetic-password"));
                assert!(!value.to_string().contains("13800000000"));
                if verification {
                    assert!(value["data"].get("caller_credential").is_none());
                } else {
                    assert_eq!(
                        value["data"]["account"],
                        if mode == CredentialMode::Client {
                            "default"
                        } else {
                            "personal"
                        }
                    );
                    let returned = value["data"].get("caller_credential");
                    assert_eq!(returned.is_some(), mode.returns_to_caller());
                    if let Some(caller) = returned {
                        let decoded =
                            CallerCredential::parse(caller["value"].as_str().unwrap()).unwrap();
                        assert_eq!(decoded.platform, Platform::Kugou);
                        assert_eq!(
                            decoded.kind,
                            if backend == PasswordLoginBackend::Native {
                                "kugou_native_v1"
                            } else {
                                "kugou_web_v1"
                            }
                        );
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn kugou_password_http_rejects_unsupported_inputs_without_upstream_requests() {
    let mut registry = ProviderRegistry::new();
    registry
        .register(tuneweave_provider_kugou::KugouProvider::new(Default::default()).unwrap())
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Kugou));
    let base = json!({"platform":"kugou","principal_type":"phone","principal":"13800000000","password":"synthetic-password","credential_mode":"client"});
    for (key, value) in [
        ("backend", json!("middle")),
        ("backend", Value::Null),
        ("password_format", json!("md5")),
        ("country_code", json!("+1")),
        ("secure_captcha", json!("ticket")),
        ("account", json!("named")),
        ("password", json!(" padded")),
        ("principal", json!("invalid-phone")),
    ] {
        let mut body = base.clone();
        body[key] = value;
        inspect(
            call(
                app.clone(),
                Method::POST,
                "/v1/auth/password",
                Some(body),
                None,
            )
            .await,
            StatusCode::BAD_REQUEST,
        )
        .await;
    }
}

#[tokio::test]
async fn kugou_web_http_rejects_mixed_sources_and_discards_expired_caller_sessions() {
    let mut registry = ProviderRegistry::new();
    registry
        .register(tuneweave_provider_kugou::KugouProvider::new(Default::default()).unwrap())
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Kugou));
    let secret = json!({"version":1,"generation":"b".repeat(64),"session":{"device":{"guid":"12345678-1234-4234-8234-123456789abc","mid":"8b24a211ab448ed0b361923045ebe493","dfid":null},"user_id":"111","cookie":{"value":"KugooID=111&t=synthetic-web-token&a_id=1014","domain":"kugou.com","path":"/","expires":0}}}).to_string();
    let caller = CallerCredential::issue(
        &ProviderCredential::new(Platform::Kugou, "kugou_web_v1", secret, None).unwrap(),
    )
    .unwrap();
    inspect(
        call(
            app.clone(),
            Method::GET,
            "/v1/auth/session?platform=kugou&account=other",
            None,
            Some(&caller.value),
        )
        .await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    inspect(
        call(
            app.clone(),
            Method::GET,
            "/v1/playlists/kugou:123",
            None,
            Some(&caller.value),
        )
        .await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    let expired = inspect(
        call(
            app.clone(),
            Method::GET,
            "/v1/auth/session?platform=kugou",
            None,
            Some(&caller.value),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(expired["data"]["authenticated"], false);
    assert!(!expired.to_string().contains("synthetic-web-token"));
    let logout = inspect(
        call(
            app,
            Method::DELETE,
            "/v1/auth/session?platform=kugou&credential_mode=client",
            None,
            Some(&caller.value),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(logout["data"]["caller_credential_discard_required"], true);
}

#[tokio::test]
async fn password_http_rejects_unsupported_backends_before_provider_io() {
    let mut registry = ProviderRegistry::new();
    registry
        .register(tuneweave_provider_migu::MiguProvider::new(Default::default()).unwrap())
        .unwrap();
    registry
        .register(tuneweave_provider_kuwo::KuwoProvider::new(Default::default()).unwrap())
        .unwrap();
    registry
        .register(tuneweave_provider_netease::NeteaseProvider::new(Default::default()).unwrap())
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Migu));
    for (platform, backend) in [("migu", "native"), ("netease", "native")] {
        let response = call(app.clone(), Method::POST, "/v1/auth/password", Some(json!({
            "platform":platform, "backend":backend, "principal_type":"username",
            "principal":"synthetic-user", "password":"synthetic-password", "credential_mode":"client"
        })), None).await;
        let value = inspect(response, StatusCode::BAD_REQUEST).await;
        assert!(
            value
                .to_string()
                .contains("Unsupported password login backend")
        );
    }
}

struct QrProvider {
    kind: &'static str,
    mode: CredentialMode,
    step: Mutex<u8>,
}
#[async_trait]
impl MusicProvider for QrProvider {
    fn platform(&self) -> Platform {
        Platform::Kugou
    }
    fn name(&self) -> &'static str {
        "KuGou HTTP ownership fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::QrLogin, Capability::CallerManagedCredentials])
    }
    async fn start_qr_login_with_mode(
        &self,
        kind: Option<&str>,
        mode: CredentialMode,
    ) -> Result<ProviderQrStart> {
        assert_eq!(kind, Some(self.kind));
        assert_eq!(mode, self.mode);
        Ok(ProviderQrStart {
            provider_transaction_id: "private-provider-id".to_owned(),
            url: "https://h5.kugou.com/apps/loginQRCode/html/index.html?synthetic=1".to_owned(),
            image_data_url: None,
            expires_at: None,
        })
    }
    async fn poll_qr_login_with_mode(
        &self,
        id: &str,
        account: &str,
        mode: CredentialMode,
    ) -> Result<ProviderQrPoll> {
        assert_eq!(id, "private-provider-id");
        assert_eq!(mode, self.mode);
        assert_eq!(
            account,
            if mode == CredentialMode::Client {
                "default"
            } else {
                "personal"
            }
        );
        let mut step = self.step.lock().unwrap();
        *step += 1;
        if *step == 1 {
            return Ok(ProviderQrPoll {
                verification: None,
                state: AuthState::Waiting,
                message: None,
                profile: None,
                credential: None,
            });
        }
        let mut profile = AccountProfile::authenticated(Platform::Kugou, account);
        profile.user_id = Some("111".to_owned());
        Ok(ProviderQrPoll {
            verification: None,
            state: AuthState::Confirmed,
            message: None,
            profile: Some(profile),
            credential: mode.returns_to_caller().then(|| {
                ProviderCredential::new(
                    Platform::Kugou,
                    if self.kind == "web" {
                        "kugou_web_v1"
                    } else {
                        "kugou_native_v1"
                    },
                    "synthetic-http-session",
                    None,
                )
                .unwrap()
            }),
        })
    }
}

async fn call(
    app: axum::Router,
    method: Method,
    path: &str,
    body: Option<Value>,
    credential: Option<&str>,
) -> Response {
    let mut request = Request::builder().method(method).uri(path);
    if let Some(value) = credential {
        request = request.header(CALLER_CREDENTIAL_HEADER, value);
    }
    let body = if let Some(value) = body {
        request = request.header(header::CONTENT_TYPE, "application/json");
        Body::from(value.to_string())
    } else {
        Body::empty()
    };
    app.oneshot(request.body(body).unwrap()).await.unwrap()
}
async fn inspect(response: Response, status: StatusCode) -> Value {
    assert_eq!(response.status(), status);
    assert!(
        response.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("no-store")
    );
    let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
    assert!(!String::from_utf8_lossy(&bytes).contains("synthetic-http-session"));
    assert!(!String::from_utf8_lossy(&bytes).contains("private-provider-id"));
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn kugou_qr_http_pins_account_and_ownership_and_delivers_a_credential_only_once() {
    for kind in ["concept", "web"] {
        for mode in [
            CredentialMode::Server,
            CredentialMode::Client,
            CredentialMode::Both,
        ] {
            let mut registry = ProviderRegistry::new();
            registry
                .register(QrProvider {
                    kind,
                    mode,
                    step: Mutex::new(0),
                })
                .unwrap();
            let app = build_router(AppState::new(registry, Platform::Kugou));
            let mut body = json!({"platform":"kugou","credential_mode":mode,"login_type":kind});
            if mode != CredentialMode::Client {
                body["account"] = json!("personal");
            }
            let start = inspect(
                call(app.clone(), Method::POST, "/v1/auth/qr", Some(body), None).await,
                StatusCode::OK,
            )
            .await;
            let id = start["data"]["transaction_id"].as_str().unwrap();
            assert_ne!(id, "private-provider-id");
            let path = format!("/v1/auth/qr/{id}");
            let waiting = inspect(
                call(app.clone(), Method::GET, &path, None, None).await,
                StatusCode::OK,
            )
            .await;
            assert_eq!(waiting["data"]["state"], "waiting");
            assert!(waiting["data"].get("caller_credential").is_none());
            let done = inspect(
                call(
                    app.clone(),
                    Method::GET,
                    &format!("{path}?account=other&credential_mode=server"),
                    None,
                    None,
                )
                .await,
                StatusCode::OK,
            )
            .await;
            assert_eq!(done["data"]["state"], "confirmed");
            assert_eq!(
                done["data"]["profile"]["account"],
                if mode == CredentialMode::Client {
                    "default"
                } else {
                    "personal"
                }
            );
            let returned = done["data"].get("caller_credential");
            assert_eq!(returned.is_some(), mode.returns_to_caller());
            if let Some(returned) = returned {
                let decoded = CallerCredential::parse(returned["value"].as_str().unwrap()).unwrap();
                assert_eq!(decoded.platform, Platform::Kugou);
                assert_eq!(decoded.secret(), "synthetic-http-session");
            }
            assert_eq!(
                call(app, Method::GET, &path, None, None).await.status(),
                StatusCode::NOT_FOUND
            );
        }
    }
}

#[tokio::test]
async fn kugou_real_provider_http_rejects_mixed_sources_and_unsupported_login_without_network() {
    use tuneweave_provider_kugou::{KugouConfig, KugouProvider};
    let mut registry = ProviderRegistry::new();
    registry
        .register(KugouProvider::new(KugouConfig::default()).unwrap())
        .unwrap();
    let app = build_router(AppState::new(registry, Platform::Kugou));
    let none = inspect(
        call(
            app.clone(),
            Method::GET,
            "/v1/auth/session?platform=kugou",
            None,
            None,
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(none["data"]["authenticated"], false);
    let bad = CallerCredential::issue(
        &ProviderCredential::new(Platform::Kugou, "cookie", "synthetic-cookie", None).unwrap(),
    )
    .unwrap();
    inspect(
        call(
            app.clone(),
            Method::GET,
            "/v1/auth/session?platform=kugou",
            None,
            Some(&bad.value),
        )
        .await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    let secret=json!({"version":1,"generation":"a".repeat(64),"session":{"client":"standard","device":{"guid":"12345678-1234-4234-8234-123456789abc","mid":"184952901251249932502431607290359178387","dfid":null},"user_id":"111","token":"synthetic-native-token","vip_token":null,"t1":null}}).to_string();
    let caller = CallerCredential::issue(
        &ProviderCredential::new(Platform::Kugou, "kugou_native_v1", secret, None).unwrap(),
    )
    .unwrap();
    inspect(
        call(
            app.clone(),
            Method::GET,
            "/v1/auth/session?platform=kugou&account=other",
            None,
            Some(&caller.value),
        )
        .await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    inspect(
        call(
            app.clone(),
            Method::GET,
            "/v1/playlists/kugou:123",
            None,
            Some(&caller.value),
        )
        .await,
        StatusCode::BAD_REQUEST,
    )
    .await;
    let logout = inspect(
        call(
            app.clone(),
            Method::DELETE,
            "/v1/auth/session?platform=kugou&credential_mode=client",
            None,
            Some(&caller.value),
        )
        .await,
        StatusCode::OK,
    )
    .await;
    assert_eq!(logout["data"]["removed"], false);
    assert_eq!(logout["data"]["caller_credential_discard_required"], true);
    let unsupported = call(
        app,
        Method::POST,
        "/v1/auth/qr",
        Some(json!({"platform":"kugou","credential_mode":"client","login_type":"password"})),
        None,
    )
    .await;
    assert!(!unsupported.status().is_success());
}
