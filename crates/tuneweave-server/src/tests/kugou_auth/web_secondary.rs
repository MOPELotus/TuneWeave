use super::*;

struct WebSecondaryProvider {
    mode: CredentialMode,
    backend: PasswordLoginBackend,
    receipt: Mutex<Option<ProviderPasswordChallenge>>,
    step: Mutex<u8>,
}

#[async_trait]
impl MusicProvider for WebSecondaryProvider {
    fn platform(&self) -> Platform {
        Platform::Kugou
    }
    fn name(&self) -> &'static str {
        "KuGou Web secondary SMS HTTP contract fixture"
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
        assert_eq!(request.backend, self.backend);
        assert_eq!(mode, self.mode);
        assert_eq!(request.principal, "original-user");
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
            verification: PasswordVerification::Sms {
                masked_destination: "138****8000".into(),
                remaining_attempts: 5,
                resend_after_secs: 60,
            },
        })
    }
    async fn advance_password_login(
        &self,
        receipt: &ProviderPasswordChallenge,
        action: &PasswordChallengeAction,
    ) -> Result<PasswordLoginProgress> {
        assert_eq!(self.receipt.lock().unwrap().as_ref(), Some(receipt));
        assert_eq!(receipt.identity().backend, self.backend);
        assert_eq!(receipt.credential_mode(), self.mode);
        let mut step = self.step.lock().unwrap();
        let verification = match (*step, action) {
            (0, PasswordChallengeAction::SubmitSms { code }) if code == "123456" => {
                PasswordVerification::SmsBrowser {
                    verification: tuneweave_core::AuthBrowserChallenge {
                        verification_id: "web-sms-browser-id".into(),
                        url: "https://h5.kugou.com/apps/verify-h5/dist/#/index/synthetic-event/1014/null/synthetic-mid/TuneWeaveVerify".into(),
                        message_origin: "https://h5.kugou.com".into(),
                        message_type: "kgVerifyCallbackData".into(),
                        response_field: "dataJson".into(),
                        remaining_attempts: 4,
                    },
                }
            }
            (1, PasswordChallengeAction::SubmitSmsBrowser { verification_id, code, response })
                if verification_id == "web-sms-browser-id"
                    && code == "123456"
                    && response == "synthetic-callback" => {
                PasswordVerification::AccountSelection {
                    accounts: vec![tuneweave_core::AuthAccountChoice {
                        user_id: "111".into(),
                        nickname: Some("Test listener".into()),
                        avatar_url: None,
                    }],
                    remaining_attempts: 3,
                }
            }
            (2, PasswordChallengeAction::SelectAccount { user_id, code })
                if user_id == "111" && code == "123456" => {
                self.receipt.lock().unwrap().take();
                let mut profile = AccountProfile::authenticated(Platform::Kugou, &receipt.identity().account);
                profile.user_id = Some("111".into());
                return Ok(PasswordLoginProgress::Confirmed(ProviderAuthResult {
                    profile,
                    credential: self.mode.returns_to_caller().then(|| {
                        ProviderCredential::new(Platform::Kugou, "kugou_web_v1", "synthetic-http-session", None).unwrap()
                    }),
                }));
            }
            _ => return Err(TuneWeaveError::invalid_request("Unexpected Web password continuation")),
        };
        *step += 1;
        Ok(PasswordLoginProgress::Pending {
            challenge: receipt.clone(),
            verification,
        })
    }
}

#[tokio::test]
async fn web_password_sms_browser_and_account_choice_preserve_one_http_transaction() {
    for backend in [PasswordLoginBackend::Default, PasswordLoginBackend::Web] {
        for mode in [
            CredentialMode::Server,
            CredentialMode::Client,
            CredentialMode::Both,
        ] {
            let mut registry = ProviderRegistry::new();
            registry
                .register(WebSecondaryProvider {
                    mode,
                    backend,
                    receipt: Mutex::new(None),
                    step: Mutex::new(0),
                })
                .unwrap();
            let app = build_router(AppState::new(registry, Platform::Kugou));
            let account = if mode == CredentialMode::Client {
                "default"
            } else {
                "personal"
            };
            let mut body = json!({"platform":"kugou", "backend":backend,
                "principal_type":"username", "principal":"original-user", "password":"first-password", "credential_mode":mode});
            if mode.persists_on_server() {
                body["account"] = json!(account);
            }
            let initial = inspect(
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
            assert_eq!(initial["data"]["verification"]["method"], "sms");
            let id = initial["data"]["transaction_id"].as_str().unwrap();
            let path = format!("/v1/auth/password/challenges/{id}/verify");
            let browser = inspect(
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
            assert_eq!(browser["data"]["verification"]["method"], "sms_browser");
            let challenge = &browser["data"]["verification"]["verification"];
            assert_eq!(challenge["message_origin"], "https://h5.kugou.com");
            assert_eq!(challenge["message_type"], "kgVerifyCallbackData");
            assert_eq!(challenge["response_field"], "dataJson");
            assert_eq!(challenge["remaining_attempts"], 4);
            for invalid in [
                json!({"action":"submit_sms_browser","verification_id":"web-sms-browser-id","response":"synthetic-callback"}),
                json!({"action":"submit_sms_browser","verification_id":"web-sms-browser-id","code":"123456","response":"synthetic-callback","password":"forbidden"}),
                json!({"action":"submit_sms_browser","verification_id":"web-sms-browser-id","code":"123456","response":"synthetic-callback","backend":"native"}),
                json!({"action":"submit_sms_browser","verification_id":"other-transaction","code":"123456","response":"synthetic-callback"}),
                json!({"action":"submit_browser","verification_id":"web-sms-browser-id","response":"synthetic-callback","password":"forbidden"}),
            ] {
                inspect(
                    call(app.clone(), Method::POST, &path, Some(invalid), None).await,
                    StatusCode::BAD_REQUEST,
                )
                .await;
            }
            let action = json!({"action":"submit_sms_browser","verification_id":"web-sms-browser-id","code":"123456","response":"synthetic-callback"});
            let choices = inspect(
                call(app.clone(), Method::POST, &path, Some(action.clone()), None).await,
                StatusCode::OK,
            )
            .await;
            assert_eq!(
                choices["data"]["verification"]["method"],
                "account_selection"
            );
            assert_eq!(
                choices["data"]["verification"]["accounts"][0]["user_id"],
                "111"
            );
            for pending in [&initial, &browser, &choices] {
                assert_eq!(pending["data"]["state"], "verification_required");
                assert_eq!(pending["data"]["transaction_id"], id);
                assert!(pending["data"].get("caller_credential").is_none());
                for secret in [
                    "original-user",
                    "first-password",
                    "123456",
                    "synthetic-callback",
                ] {
                    assert!(!pending.to_string().contains(secret));
                }
            }
            let confirmed = inspect(
                call(
                    app.clone(),
                    Method::POST,
                    &path,
                    Some(json!({"action":"select_account","user_id":"111","code":"123456"})),
                    None,
                )
                .await,
                StatusCode::OK,
            )
            .await;
            assert_eq!(confirmed["data"]["account"], account);
            assert_eq!(confirmed["data"]["user_id"], "111");
            assert_eq!(
                confirmed["data"].get("caller_credential").is_some(),
                mode.returns_to_caller()
            );
            if mode.returns_to_caller() {
                assert_eq!(
                    CallerCredential::parse(
                        confirmed["data"]["caller_credential"]["value"]
                            .as_str()
                            .unwrap()
                    )
                    .unwrap()
                    .kind,
                    "kugou_web_v1"
                );
            }
            inspect(
                call(app, Method::POST, &path, Some(action), None).await,
                StatusCode::NOT_FOUND,
            )
            .await;
        }
    }
}
