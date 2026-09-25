use super::auth_challenges::Gate;
use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Clone, Default)]
struct Provider {
    starts: Arc<AtomicUsize>,
    actions: Arc<AtomicUsize>,
    receipts: Arc<Mutex<Vec<ProviderPasswordChallenge>>>,
    start_gate: Arc<Gate>,
    action_gate: Arc<Gate>,
    direct: Arc<AtomicBool>,
    bad_start: Arc<AtomicBool>,
    bad_pending: Arc<AtomicBool>,
    bad_final: Arc<AtomicBool>,
}
fn image() -> PasswordVerification {
    PasswordVerification::Image {
        image: tuneweave_core::AuthImageChallenge {
            image_data_url: "data:image/jpeg;base64,fixture".into(),
            answer_kind: tuneweave_core::AuthImageAnswerKind::Arithmetic,
            remaining_attempts: 5,
            refresh_after_secs: 2,
        },
    }
}
fn sms() -> PasswordVerification {
    PasswordVerification::Sms {
        masked_destination: "138****8000".into(),
        remaining_attempts: 5,
        resend_after_secs: 60,
    }
}
impl Provider {
    fn confirmed(&self, account: &str, mode: CredentialMode) -> PasswordLoginProgress {
        PasswordLoginProgress::Confirmed(ProviderAuthResult {
            profile: AccountProfile::authenticated(
                Platform::Migu,
                if self.bad_final.load(Ordering::SeqCst) {
                    "wrong-account"
                } else {
                    account
                },
            ),
            credential: mode.returns_to_caller().then(|| {
                ProviderCredential::new(Platform::Migu, "test", "private-credential", None).unwrap()
            }),
        })
    }
}
#[async_trait]
impl MusicProvider for Provider {
    fn platform(&self) -> Platform {
        Platform::Migu
    }
    fn name(&self) -> &'static str {
        "Password challenge fixture"
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
        let n = self.starts.fetch_add(1, Ordering::SeqCst);
        self.start_gate.wait().await;
        if self.direct.load(Ordering::SeqCst) {
            return Ok(self.confirmed(&request.account, mode));
        }
        let mut identity = PasswordLoginIdentity::from(request);
        if self.bad_start.load(Ordering::SeqCst) {
            identity.principal = "wrong-principal".into();
        }
        let receipt = ProviderPasswordChallenge::new(
            Platform::Migu,
            identity,
            mode,
            format!("private-password-handle-{n}"),
        )
        .unwrap();
        self.receipts.lock().unwrap().push(receipt.clone());
        Ok(PasswordLoginProgress::Pending {
            challenge: receipt,
            verification: image(),
        })
    }
    async fn advance_password_login(
        &self,
        receipt: &ProviderPasswordChallenge,
        action: &PasswordChallengeAction,
    ) -> Result<PasswordLoginProgress> {
        assert!(self.receipts.lock().unwrap().contains(receipt));
        self.actions.fetch_add(1, Ordering::SeqCst);
        self.action_gate.wait().await;
        if let PasswordChallengeAction::SubmitSms { code } = action {
            if code == "0000" {
                return Err(TuneWeaveError::new(
                    ErrorCode::AuthenticationRequired,
                    "Wrong code",
                ));
            }
            if code == "9999" {
                return Err(TuneWeaveError::new(
                    ErrorCode::UpstreamTimeout,
                    "Uncertain verification",
                )
                .with_consumed_auth_challenge());
            }
            assert_eq!(code, "1234");
            return Ok(self.confirmed(&receipt.identity().account, receipt.credential_mode()));
        }
        let challenge = if self.bad_pending.load(Ordering::SeqCst) {
            ProviderPasswordChallenge::new(
                receipt.platform(),
                receipt.identity().clone(),
                receipt.credential_mode(),
                "different-handle".into(),
            )
            .unwrap()
        } else {
            receipt.clone()
        };
        Ok(PasswordLoginProgress::Pending {
            challenge,
            verification: if matches!(action, PasswordChallengeAction::RefreshImage) {
                image()
            } else {
                sms()
            },
        })
    }
}
fn app(provider: &Provider) -> (Router, AuthTransactions) {
    let mut registry = ProviderRegistry::new();
    registry.register(provider.clone()).unwrap();
    let state = AppState::new(registry, Platform::Migu);
    let tx = state.auth_transactions.clone();
    (build_router(state), tx)
}
fn body(mode: CredentialMode) -> Value {
    let mut b = json!({"platform":"migu","principal_type":"username","principal":"private-principal","password":"private-password","credential_mode":mode});
    if mode.persists_on_server() {
        b["account"] = json!("A");
    }
    b
}
async fn start(app: Router, mode: CredentialMode) -> String {
    let (status, headers, response) =
        json_request_with_headers(app, Method::POST, "/v1/auth/password", Some(body(mode))).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(response["data"]["state"], "verification_required");
    assert_eq!(response["data"]["verification"]["method"], "image");
    for secret in [
        "private-principal",
        "private-password",
        "private-password-handle",
        "private-credential",
    ] {
        assert!(!response.to_string().contains(secret));
    }
    assert!(response["data"].get("caller_credential").is_none());
    response["data"]["transaction_id"].as_str().unwrap().into()
}
async fn action(app: Router, id: &str, body: Value) -> (StatusCode, HeaderMap, Value) {
    let result = json_request_with_headers(
        app,
        Method::POST,
        &format!("/v1/auth/password/challenges/{id}/verify"),
        Some(body),
    )
    .await;
    assert_eq!(result.1[header::CACHE_CONTROL], "no-store");
    result
}

#[tokio::test]
async fn password_http_retains_original_binding_and_only_confirmed_result_returns_credentials() {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let provider = Provider::default();
        let (app, tx) = app(&provider);
        let id = start(app.clone(), mode).await;
        let original = tx.get(&id).unwrap();
        for body in [
            json!({"action":"refresh_image"}),
            json!({"action":"submit_image","answer":"42","password":"private-resubmitted"}),
            json!({"action":"resend_sms"}),
        ] {
            let (status, headers, response) = action(app.clone(), &id, body).await;
            assert_eq!(status, StatusCode::OK, "{response}");
            assert_eq!(response["data"]["transaction_id"], id);
            assert_eq!(response["data"]["state"], "verification_required");
            assert!(response["data"].get("authenticated").is_none());
            assert!(response["data"].get("caller_credential").is_none());
            assert!(
                headers
                    .get(caller_scope::UPDATED_CREDENTIAL_HEADER)
                    .is_none()
            );
            for secret in [
                "private-principal",
                "private-password",
                "private-password-handle",
                "private-resubmitted",
                "private-credential",
            ] {
                assert!(!response.to_string().contains(secret));
            }
            let current = tx.get(&id).unwrap();
            assert_eq!(current.created_at, original.created_at);
            assert_eq!(current.expires_at, original.expires_at);
            assert!(matches!(
                current.kind,
                StoredAuthKind::Password {
                    verifying: false,
                    ..
                }
            ));
        }
        assert_eq!(provider.starts.load(Ordering::SeqCst), 1);
        assert_eq!(
            action(
                app.clone(),
                &id,
                json!({"action":"submit_sms","code":"0000"})
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
        let (status, _, response) = action(
            app.clone(),
            &id,
            json!({"action":"submit_sms","code":"1234"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{response}");
        assert_eq!(response["data"]["authenticated"], true);
        assert_eq!(
            response["data"].get("caller_credential").is_some(),
            mode.returns_to_caller()
        );
        assert_eq!(tx.counts().unwrap().total, 0);
        assert_eq!(
            action(app, &id, json!({"action":"resend_sms"})).await.0,
            StatusCode::NOT_FOUND
        );
    }
}

#[tokio::test]
async fn ordinary_password_http_result_stays_compatible_and_rejects_unconfirmed_account() {
    for bad in [false, true] {
        let provider = Provider::default();
        provider.direct.store(true, Ordering::SeqCst);
        provider.bad_final.store(bad, Ordering::SeqCst);
        let (app, tx) = app(&provider);
        let (status, headers, response) = json_request_with_headers(
            app,
            Method::POST,
            "/v1/auth/password",
            Some(body(CredentialMode::Client)),
        )
        .await;
        assert_eq!(
            status,
            if bad {
                StatusCode::INTERNAL_SERVER_ERROR
            } else {
                StatusCode::OK
            },
            "{response}"
        );
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        if !bad {
            assert_eq!(response["data"]["authenticated"], true);
            assert!(response["data"].get("transaction_id").is_none());
        }
        assert_eq!(tx.counts().unwrap().total, 0);
    }
}

#[tokio::test]
async fn password_receipt_substitution_and_uncertain_errors_consume_the_http_transaction() {
    for kind in ["start", "pending", "final", "uncertain"] {
        let provider = Provider::default();
        provider.bad_start.store(kind == "start", Ordering::SeqCst);
        let (app, tx) = app(&provider);
        if kind == "start" {
            assert_eq!(
                json_request_with_headers(
                    app,
                    Method::POST,
                    "/v1/auth/password",
                    Some(body(CredentialMode::Client))
                )
                .await
                .0,
                StatusCode::INTERNAL_SERVER_ERROR
            );
        } else {
            let id = start(app.clone(), CredentialMode::Both).await;
            provider
                .bad_pending
                .store(kind == "pending", Ordering::SeqCst);
            provider.bad_final.store(kind == "final", Ordering::SeqCst);
            let body = if kind == "pending" {
                json!({"action":"refresh_image"})
            } else {
                json!({"action":"submit_sms","code":if kind=="uncertain"{"9999"}else{"1234"}})
            };
            assert_eq!(
                action(app.clone(), &id, body).await.0,
                if kind == "uncertain" {
                    StatusCode::GATEWAY_TIMEOUT
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                }
            );
            assert_eq!(
                action(app, &id, json!({"action":"resend_sms"})).await.0,
                StatusCode::NOT_FOUND
            );
        }
        assert_eq!(tx.counts().unwrap().total, 0);
    }
}

#[tokio::test]
async fn password_start_reserves_capacity_before_provider_work_and_cancel_releases_it() {
    let provider = Provider::default();
    let (app, tx) = app(&provider);
    for _ in 0..AUTH_TRANSACTION_CAPACITY {
        tx.insert(StoredAuthKind::Qr {
            platform: Platform::Migu,
            account: "A".into(),
            credential_mode: CredentialMode::Server,
            provider_transaction_id: "qr-fixture".into(),
        })
        .unwrap();
    }
    assert_eq!(
        json_request_with_headers(
            app.clone(),
            Method::POST,
            "/v1/auth/password",
            Some(body(CredentialMode::Client))
        )
        .await
        .0,
        StatusCode::TOO_MANY_REQUESTS
    );
    assert_eq!(provider.starts.load(Ordering::SeqCst), 0);
    tx.entries.write().unwrap().clear();
    provider.start_gate.enable();
    let worker = app.clone();
    let task = tokio::spawn(async move {
        json_request_with_headers(
            worker,
            Method::POST,
            "/v1/auth/password",
            Some(body(CredentialMode::Client)),
        )
        .await
    });
    provider.start_gate.entered().await;
    assert_eq!(tx.counts().unwrap().password, 1);
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(tx.counts().unwrap().total, 0);
}

#[tokio::test]
async fn password_actions_are_exclusive_and_late_progress_cannot_revive_expired_or_cancelled_transactions()
 {
    for cancel in [false, true] {
        let provider = Provider::default();
        let (app, tx) = app(&provider);
        let id = start(app.clone(), CredentialMode::Client).await;
        provider.action_gate.enable();
        let worker = app.clone();
        let selected = id.clone();
        let task = tokio::spawn(async move {
            action(worker, &selected, json!({"action":"refresh_image"})).await
        });
        provider.action_gate.entered().await;
        assert_eq!(
            action(app.clone(), &id, json!({"action":"resend_sms"}))
                .await
                .0,
            StatusCode::CONFLICT
        );
        if cancel {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            tx.entries.write().unwrap().get_mut(&id).unwrap().expires_at = Instant::now();
            provider.action_gate.open();
            assert_eq!(task.await.unwrap().0, StatusCode::NOT_FOUND);
        }
        assert_eq!(tx.counts().unwrap().total, 0);
        assert_eq!(
            action(app, &id, json!({"action":"resend_sms"})).await.0,
            StatusCode::NOT_FOUND
        );
    }
}

#[tokio::test]
async fn password_action_body_and_route_cannot_switch_identity_mode_or_login_flow() {
    let provider = Provider::default();
    let (app, tx) = app(&provider);
    let id = start(app.clone(), CredentialMode::Client).await;
    for body in [
        json!({"action":"submit_image","answer":"42"}),
        json!({"action":"submit_image","answer":"42","password":"secret","account":"A"}),
        json!({"action":"resend_sms","principal":"other"}),
        json!({"action":"submit_sms","code":"1234","credential_mode":"both"}),
        json!({"code":"1234"}),
    ] {
        assert_eq!(
            action(app.clone(), &id, body).await.0,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(tx.counts().unwrap().password, 1);
    }
    assert_eq!(provider.actions.load(Ordering::SeqCst), 0);
    let (status, _, _) = json_request_with_headers(
        app.clone(),
        Method::POST,
        &format!("/v1/auth/challenges/{id}/verify"),
        Some(json!({"code":"1234"})),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(tx.counts().unwrap().password, 1);
    assert_eq!(
        action(app, &id, json!({"action":"submit_sms","code":"1234"}))
            .await
            .0,
        StatusCode::OK
    );
}

#[tokio::test]
async fn late_password_start_cannot_publish_pending_or_confirmed_results_after_expiry() {
    for direct in [false, true] {
        let provider = Provider::default();
        provider.direct.store(direct, Ordering::SeqCst);
        provider.start_gate.enable();
        let (app, tx) = app(&provider);
        let worker = app.clone();
        let task = tokio::spawn(async move {
            json_request_with_headers(
                worker,
                Method::POST,
                "/v1/auth/password",
                Some(body(CredentialMode::Client)),
            )
            .await
        });
        provider.start_gate.entered().await;
        for transaction in tx.entries.write().unwrap().values_mut() {
            transaction.expires_at = Instant::now();
        }
        provider.start_gate.open();
        let (status, headers, response) = task.await.unwrap();
        assert_eq!(status, StatusCode::NOT_FOUND, "{response}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert!(!response.to_string().contains("private-credential"));
        assert_eq!(tx.counts().unwrap().total, 0);
    }
}
