use super::*;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use tokio::sync::Notify;

#[tokio::test]
async fn migu_sms_http_rejects_invalid_provider_inputs_and_releases_its_reservation() {
    let mut registry = ProviderRegistry::new();
    registry
        .register(
            tuneweave_provider_migu::MiguProvider::new(
                tuneweave_provider_migu::MiguConfig::default(),
            )
            .unwrap(),
        )
        .unwrap();
    let state = AppState::new(registry, Platform::Migu);
    let transactions = state.auth_transactions.clone();
    let app = build_router(state);
    for body in [
        json!({"platform":"migu","principal":"private-invalid-phone","credential_mode":"client"}),
        json!({"platform":"migu","principal":"13800138000","country_code":"1","credential_mode":"client"}),
        json!({"platform":"migu","principal":"13800138000","backend":"middle","credential_mode":"client"}),
        json!({"platform":"migu","principal":"13800138000","credential_mode":"server"}),
    ] {
        let (status, headers, response) =
            json_request_with_headers(app.clone(), Method::POST, "/v1/auth/challenges", Some(body))
                .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{response}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert!(
            headers
                .get(caller_scope::UPDATED_CREDENTIAL_HEADER)
                .is_none()
        );
        assert!(!response.to_string().contains("13800138000"));
        assert!(!response.to_string().contains("private-invalid-phone"));
        assert_eq!(transactions.counts().unwrap().total, 0);
    }
}

#[derive(Default)]
pub(super) struct Gate {
    enabled: AtomicBool,
    entered: Notify,
    release: Notify,
}
impl Gate {
    pub(super) fn enable(&self) {
        self.enabled.store(true, Ordering::SeqCst);
    }
    pub(super) async fn wait(&self) {
        if self.enabled.load(Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.notified().await;
        }
    }
    pub(super) async fn entered(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.entered.notified())
            .await
            .unwrap();
    }
    pub(super) fn open(&self) {
        self.enabled.store(false, Ordering::SeqCst);
        self.release.notify_one();
    }
}

#[derive(Clone, Default)]
struct ChallengeProvider {
    sent: Arc<AtomicUsize>,
    verified: Arc<AtomicUsize>,
    receipts: Arc<Mutex<Vec<ProviderAuthChallenge>>>,
    send_gate: Arc<Gate>,
    verify_gate: Arc<Gate>,
    fail_send: Arc<AtomicBool>,
    wrong_binding: Arc<AtomicBool>,
    wrong_result: Arc<AtomicUsize>,
    image_required: Arc<AtomicBool>,
    actions: Arc<AtomicUsize>,
}

#[async_trait]
impl MusicProvider for ChallengeProvider {
    fn platform(&self) -> Platform {
        Platform::Migu
    }
    fn name(&self) -> &'static str {
        "Stateful challenge fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([Capability::PhoneLogin, Capability::CallerManagedCredentials])
    }
    async fn begin_auth_challenge(
        &self,
        request: &AuthChallengeRequest,
        mode: CredentialMode,
    ) -> Result<ProviderAuthChallenge> {
        let id = self.sent.fetch_add(1, Ordering::SeqCst);
        self.send_gate.wait().await;
        if self.fail_send.load(Ordering::SeqCst) {
            return Err(TuneWeaveError::new(
                ErrorCode::UpstreamError,
                "Delivery not confirmed",
            ));
        }
        let mut request = request.clone();
        if self.wrong_binding.load(Ordering::SeqCst) {
            request.account = "wrong-owner".into();
        }
        let receipt = ProviderAuthChallenge::stateful(
            Platform::Migu,
            request,
            mode,
            format!("private-handle-{id}"),
        )
        .unwrap();
        self.receipts.lock().unwrap().push(receipt.clone());
        Ok(receipt)
    }
    async fn auth_challenge_status(
        &self,
        _: &ProviderAuthChallenge,
    ) -> Result<AuthChallengeStatus> {
        Ok(if self.image_required.load(Ordering::SeqCst) {
            AuthChallengeStatus::VerificationRequired {
                verification: tuneweave_core::AuthImageChallenge {
                    image_data_url: "data:image/jpeg;base64,fixture".into(),
                    answer_kind: tuneweave_core::AuthImageAnswerKind::Arithmetic,
                    remaining_attempts: 5,
                    refresh_after_secs: 2,
                },
            }
        } else {
            AuthChallengeStatus::Waiting
        })
    }
    async fn advance_auth_challenge(
        &self,
        receipt: &ProviderAuthChallenge,
        action: &AuthChallengeAction,
    ) -> Result<AuthChallengeProgress> {
        if let AuthChallengeAction::SubmitCode { code } = action {
            return self
                .complete_auth_challenge(receipt, code)
                .await
                .map(AuthChallengeProgress::Confirmed);
        }
        assert!(self.receipts.lock().unwrap().contains(receipt));
        self.actions.fetch_add(1, Ordering::SeqCst);
        self.verify_gate.wait().await;
        if let AuthChallengeAction::SubmitImage { answer } = action {
            assert_eq!(answer, "42");
            self.image_required.store(false, Ordering::SeqCst);
        }
        Ok(AuthChallengeProgress::Pending(
            self.auth_challenge_status(receipt).await?,
        ))
    }
    async fn complete_auth_challenge(
        &self,
        challenge: &ProviderAuthChallenge,
        code: &str,
    ) -> Result<ProviderAuthResult> {
        self.verified.fetch_add(1, Ordering::SeqCst);
        assert!(self.receipts.lock().unwrap().contains(challenge));
        self.verify_gate.wait().await;
        if code == "bad" {
            return Err(TuneWeaveError::new(
                ErrorCode::AuthenticationRequired,
                "Code rejected",
            ));
        }
        if code == "uncertain" {
            return Err(TuneWeaveError::new(
                ErrorCode::UpstreamTimeout,
                "Verification outcome is uncertain",
            )
            .with_consumed_auth_challenge());
        }
        if matches!(code, "expired" | "replaced") {
            return Err(TuneWeaveError::new(
                if code == "expired" {
                    ErrorCode::ResourceNotFound
                } else {
                    ErrorCode::Conflict
                },
                "Challenge is no longer available",
            ));
        }
        assert_eq!(code, "123456");
        let wrong_result = self.wrong_result.load(Ordering::SeqCst);
        let mut profile = AccountProfile::authenticated(
            if wrong_result == 1 {
                Platform::Qq
            } else {
                Platform::Migu
            },
            if wrong_result == 2 {
                "wrong-owner"
            } else {
                &challenge.request().account
            },
        );
        if wrong_result == 3 {
            profile.authenticated = false;
        }
        Ok(ProviderAuthResult {
            profile,
            credential: challenge.credential_mode().returns_to_caller().then(|| {
                ProviderCredential::new(Platform::Migu, "test", "verified-session", None).unwrap()
            }),
        })
    }
}

fn app(provider: &ChallengeProvider) -> (Router, AuthTransactions) {
    let mut registry = ProviderRegistry::new();
    registry.register(provider.clone()).unwrap();
    let state = AppState::new(registry, Platform::Migu);
    let transactions = state.auth_transactions.clone();
    (build_router(state), transactions)
}
fn challenge_request(account: &str) -> AuthChallengeRequest {
    AuthChallengeRequest {
        allow_account_creation: false,
        accept_platform_policies: false,
        account: account.into(),
        method: ChallengeMethod::Sms,
        backend: AuthChallengeBackend::Standard,
        principal: "13800138000".into(),
        country_code: Some("86".into()),
    }
}
fn body(mode: CredentialMode) -> Value {
    let mut body = json!({"platform":"migu", "principal":"13800138000", "credential_mode":mode});
    if mode.persists_on_server() {
        body["account"] = json!("personal");
    }
    body
}
async fn start(app: Router, mode: CredentialMode) -> String {
    let (status, headers, response) =
        json_request_with_headers(app, Method::POST, "/v1/auth/challenges", Some(body(mode))).await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    for secret in ["13800138000", "private-handle"] {
        assert!(!response.to_string().contains(secret));
    }
    response["data"]["transaction_id"].as_str().unwrap().into()
}
async fn verify(app: Router, id: &str, code: &str) -> (StatusCode, HeaderMap, Value) {
    let result = json_request_with_headers(
        app,
        Method::POST,
        &format!("/v1/auth/challenges/{id}/verify"),
        Some(json!({"code":code})),
    )
    .await;
    assert_eq!(result.1[header::CACHE_CONTROL], "no-store");
    result
}
fn expire(transactions: &AuthTransactions, id: &str) {
    transactions
        .entries
        .write()
        .unwrap()
        .get_mut(id)
        .unwrap()
        .expires_at = Instant::now();
}

#[tokio::test]
async fn challenge_capacity_is_reserved_before_any_delivery_and_expired_slots_are_reused() {
    let provider = ChallengeProvider::default();
    let (app, transactions) = app(&provider);
    let mut reservations = Vec::new();
    for _ in 0..AUTH_TRANSACTION_CAPACITY {
        reservations.push(
            transactions
                .reserve_challenge(
                    Platform::Migu,
                    CredentialMode::Server,
                    challenge_request("personal"),
                )
                .unwrap(),
        );
    }
    let (status, headers, _) = json_request_with_headers(
        app.clone(),
        Method::POST,
        "/v1/auth/challenges",
        Some(body(CredentialMode::Server)),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(provider.sent.load(Ordering::SeqCst), 0);
    let id = transactions
        .entries
        .read()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone();
    expire(&transactions, &id);
    let new_id = start(app, CredentialMode::Server).await;
    assert_ne!(new_id, id);
    assert_eq!(provider.sent.load(Ordering::SeqCst), 1);
    assert_eq!(
        transactions.counts().unwrap().total,
        AUTH_TRANSACTION_CAPACITY
    );
    drop(reservations);
    assert_eq!(transactions.counts().unwrap().total, 1);
}

#[tokio::test]
async fn pending_delivery_occupies_capacity_and_cancellation_releases_its_reservation() {
    let provider = ChallengeProvider::default();
    provider.send_gate.enable();
    let (app, transactions) = app(&provider);
    let reservations = (0..AUTH_TRANSACTION_CAPACITY - 1)
        .map(|_| {
            transactions
                .reserve_challenge(
                    Platform::Migu,
                    CredentialMode::Server,
                    challenge_request("personal"),
                )
                .unwrap()
        })
        .collect::<Vec<_>>();
    let pending = tokio::spawn(json_request_with_headers(
        app.clone(),
        Method::POST,
        "/v1/auth/challenges",
        Some(body(CredentialMode::Server)),
    ));
    provider.send_gate.entered().await;
    let (status, _, _) = json_request_with_headers(
        app.clone(),
        Method::POST,
        "/v1/auth/challenges",
        Some(body(CredentialMode::Server)),
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(provider.sent.load(Ordering::SeqCst), 1);
    pending.abort();
    assert!(pending.await.unwrap_err().is_cancelled());
    assert_eq!(
        transactions.counts().unwrap().total,
        AUTH_TRANSACTION_CAPACITY - 1
    );
    provider.send_gate.open();
    start(app, CredentialMode::Server).await;
    assert_eq!(provider.sent.load(Ordering::SeqCst), 2);
    drop(reservations);
}

#[tokio::test]
async fn challenge_verification_is_exclusive_retries_a_rejected_code_and_consumes_success() {
    let provider = ChallengeProvider::default();
    provider.verify_gate.enable();
    let (app, transactions) = app(&provider);
    let id = start(app.clone(), CredentialMode::Both).await;
    let pending = {
        let app = app.clone();
        let id = id.clone();
        tokio::spawn(async move { verify(app, &id, "bad").await })
    };
    provider.verify_gate.entered().await;
    assert_eq!(
        verify(app.clone(), &id, "123456").await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(provider.verified.load(Ordering::SeqCst), 1);
    provider.verify_gate.open();
    assert_eq!(pending.await.unwrap().0, StatusCode::UNAUTHORIZED);
    assert_eq!(transactions.counts().unwrap().total, 1);
    let (status, _, response) = verify(app.clone(), &id, "123456").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(response["data"]["state"], "confirmed");
    assert!(response["data"]["caller_credential"]["value"].is_string());
    assert_eq!(transactions.counts().unwrap().total, 0);
    assert_eq!(verify(app, &id, "123456").await.0, StatusCode::NOT_FOUND);
    assert_eq!(provider.verified.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn same_principal_challenges_keep_distinct_handles_and_original_ownership() {
    let provider = ChallengeProvider::default();
    let (app, _) = app(&provider);
    let modes = [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ];
    let mut ids = Vec::new();
    for mode in modes {
        ids.push(start(app.clone(), mode).await);
    }
    let receipts = provider.receipts.lock().unwrap().clone();
    assert_eq!(receipts.len(), 3);
    assert_ne!(
        receipts[0].provider_transaction_id(),
        receipts[1].provider_transaction_id()
    );
    assert_ne!(ids[0], ids[1]);
    for (id, mode) in ids.iter().zip(modes) {
        let (status, _, response) = verify(app.clone(), id, "123456").await;
        assert_eq!(status, StatusCode::OK, "{response}");
        assert_eq!(
            response["data"]["profile"]["account"],
            if mode == CredentialMode::Client {
                "default"
            } else {
                "personal"
            }
        );
        assert_eq!(
            response["data"]["caller_credential"].is_object(),
            mode.returns_to_caller()
        );
    }
}

#[tokio::test]
async fn failed_delivery_or_wrong_receipt_cannot_leave_a_public_transaction() {
    for bad_binding in [false, true] {
        let provider = ChallengeProvider::default();
        provider.fail_send.store(!bad_binding, Ordering::SeqCst);
        provider.wrong_binding.store(bad_binding, Ordering::SeqCst);
        let (app, transactions) = app(&provider);
        let (status, headers, response) = json_request_with_headers(
            app,
            Method::POST,
            "/v1/auth/challenges",
            Some(body(CredentialMode::Server)),
        )
        .await;
        assert_eq!(
            status,
            if bad_binding {
                StatusCode::INTERNAL_SERVER_ERROR
            } else {
                StatusCode::BAD_GATEWAY
            }
        );
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert!(!response.to_string().contains("private-handle"));
        assert_eq!(transactions.counts().unwrap().total, 0);
        assert_eq!(provider.sent.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn late_delivery_and_late_verification_cannot_revive_expired_challenges() {
    let provider = ChallengeProvider::default();
    provider.send_gate.enable();
    let (app, transactions) = app(&provider);
    let pending = tokio::spawn(json_request_with_headers(
        app.clone(),
        Method::POST,
        "/v1/auth/challenges",
        Some(body(CredentialMode::Client)),
    ));
    provider.send_gate.entered().await;
    let id = transactions
        .entries
        .read()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone();
    assert_eq!(
        verify(app.clone(), &id, "123456").await.0,
        StatusCode::NOT_FOUND
    );
    expire(&transactions, &id);
    provider.send_gate.open();
    let (status, headers, _) = pending.await.unwrap();
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    assert_eq!(transactions.counts().unwrap().total, 0);
    assert_eq!(provider.verified.load(Ordering::SeqCst), 0);

    let expired_id = start(app.clone(), CredentialMode::Client).await;
    expire(&transactions, &expired_id);
    assert_eq!(
        verify(app.clone(), &expired_id, "123456").await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(provider.verified.load(Ordering::SeqCst), 0);

    let id = start(app.clone(), CredentialMode::Client).await;
    provider.verify_gate.enable();
    let pending = {
        let app = app.clone();
        let id = id.clone();
        tokio::spawn(async move { verify(app, &id, "123456").await })
    };
    provider.verify_gate.entered().await;
    expire(&transactions, &id);
    provider.verify_gate.open();
    let (status, _, response) = pending.await.unwrap();
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(response.get("data").is_none());
    assert_eq!(transactions.counts().unwrap().total, 0);
    assert_eq!(verify(app, &id, "123456").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cancelled_verification_and_invalid_success_are_not_replayable() {
    for wrong_result in 0..=3 {
        let cancel = wrong_result == 0;
        let provider = ChallengeProvider::default();
        provider.wrong_result.store(wrong_result, Ordering::SeqCst);
        let (app, transactions) = app(&provider);
        let id = start(app.clone(), CredentialMode::Server).await;
        if cancel {
            provider.verify_gate.enable();
            let pending = {
                let app = app.clone();
                let id = id.clone();
                tokio::spawn(async move { verify(app, &id, "123456").await })
            };
            provider.verify_gate.entered().await;
            pending.abort();
            assert!(pending.await.unwrap_err().is_cancelled());
        } else {
            assert_eq!(
                verify(app.clone(), &id, "123456").await.0,
                StatusCode::INTERNAL_SERVER_ERROR
            );
        }
        assert_eq!(transactions.counts().unwrap().total, 0);
        assert_eq!(verify(app, &id, "123456").await.0, StatusCode::NOT_FOUND);
        assert_eq!(provider.verified.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn provider_expiration_or_replacement_consumes_the_public_transaction() {
    for code in ["expired", "replaced", "uncertain"] {
        let provider = ChallengeProvider::default();
        let (app, transactions) = app(&provider);
        let id = start(app.clone(), CredentialMode::Client).await;
        assert_eq!(
            verify(app.clone(), &id, code).await.0,
            if code == "uncertain" {
                StatusCode::GATEWAY_TIMEOUT
            } else if code == "expired" {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::CONFLICT
            }
        );
        assert_eq!(transactions.counts().unwrap().total, 0);
        assert_eq!(verify(app, &id, "123456").await.0, StatusCode::NOT_FOUND);
        assert_eq!(provider.verified.load(Ordering::SeqCst), 1);
    }
}

#[tokio::test]
async fn malformed_challenge_inputs_are_not_cached_or_forwarded_and_cannot_change_ownership() {
    let provider = ChallengeProvider::default();
    let (app, transactions) = app(&provider);
    for bad in [
        json!({}),
        json!({"platform":"unknown", "principal":"13800138000"}),
        json!({"platform":"migu", "principal":null}),
    ] {
        let (status, headers, _) =
            json_request_with_headers(app.clone(), Method::POST, "/v1/auth/challenges", Some(bad))
                .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    }
    assert_eq!(provider.sent.load(Ordering::SeqCst), 0);
    assert_eq!(transactions.counts().unwrap().total, 0);
    let id = start(app.clone(), CredentialMode::Client).await;
    for bad in [
        json!({"code":null}),
        json!({"code":"123456", "account":"other"}),
        json!({"code":"123456", "credential_mode":"both"}),
        json!({"code":"123456", "provider_transaction_id":"other"}),
    ] {
        let (status, headers, _) = json_request_with_headers(
            app.clone(),
            Method::POST,
            &format!("/v1/auth/challenges/{id}/verify"),
            Some(bad),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
    }
    assert_eq!(provider.verified.load(Ordering::SeqCst), 0);
    assert_eq!(verify(app, &id, "123456").await.0, StatusCode::OK);
}

#[tokio::test]
async fn legacy_adapter_preserves_modes_and_refuses_stateful_or_other_platform_receipts() {
    let request = challenge_request("personal");
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let receipt = TestProvider
            .begin_auth_challenge(&request, mode)
            .await
            .unwrap();
        assert!(receipt.provider_transaction_id().is_none());
        assert_eq!(receipt.request(), &request);
        assert_eq!(receipt.credential_mode(), mode);
        let result = TestProvider
            .complete_auth_challenge(&receipt, "123456")
            .await
            .unwrap();
        assert_eq!(result.profile.account, "personal");
        assert_eq!(result.credential.is_some(), mode.returns_to_caller());
    }
    for receipt in [
        ProviderAuthChallenge::stateless(Platform::Migu, request.clone(), CredentialMode::Server),
        ProviderAuthChallenge::stateful(
            Platform::Netease,
            request,
            CredentialMode::Server,
            "private-handle".into(),
        )
        .unwrap(),
    ] {
        assert_eq!(
            TestProvider
                .complete_auth_challenge(&receipt, "123456")
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
}

#[tokio::test]
async fn image_progress_uses_the_original_http_transaction_and_only_final_confirmation_exports_credentials()
 {
    for mode in [
        CredentialMode::Server,
        CredentialMode::Client,
        CredentialMode::Both,
    ] {
        let provider = ChallengeProvider::default();
        provider.image_required.store(true, Ordering::SeqCst);
        let (app, transactions) = app(&provider);
        let (status, headers, response) = json_request_with_headers(
            app.clone(),
            Method::POST,
            "/v1/auth/challenges",
            Some(body(mode)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{response}");
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(response["data"]["state"], "verification_required");
        assert!(response["data"].get("profile").is_none());
        let id = response["data"]["transaction_id"].as_str().unwrap();
        let before = transactions.get(id).unwrap();
        for action in [
            json!({"action":"refresh_image"}),
            json!({"action":"submit_image","answer":"42"}),
        ] {
            let expected = if action["action"] == "refresh_image" {
                "verification_required"
            } else {
                "waiting"
            };
            let (status, headers, response) = json_request_with_headers(
                app.clone(),
                Method::POST,
                &format!("/v1/auth/challenges/{id}/verify"),
                Some(action),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{response}");
            assert_eq!(headers[header::CACHE_CONTROL], "no-store");
            assert_eq!(response["data"]["state"], expected);
            assert!(
                response["data"].get("profile").is_none()
                    && response["data"].get("caller_credential").is_none()
            );
            assert!(
                headers
                    .get(caller_scope::UPDATED_CREDENTIAL_HEADER)
                    .is_none()
            );
            for secret in ["13800138000", "private-handle", "verified-session"] {
                assert!(!response.to_string().contains(secret));
            }
            let after = transactions.get(id).unwrap();
            assert_eq!(after.created_at, before.created_at);
            assert_eq!(after.expires_at, before.expires_at);
            assert!(matches!(
                after.kind,
                StoredAuthKind::Challenge {
                    verifying: false,
                    ..
                }
            ));
        }
        assert_eq!(provider.sent.load(Ordering::SeqCst), 1);
        assert_eq!(provider.verified.load(Ordering::SeqCst), 0);
        let (status, _, response) = json_request_with_headers(
            app.clone(),
            Method::POST,
            &format!("/v1/auth/challenges/{id}/verify"),
            Some(json!({"action":"submit_code","code":"123456"})),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{response}");
        assert_eq!(response["data"]["state"], "confirmed");
        assert_eq!(
            response["data"].get("caller_credential").is_some(),
            mode.returns_to_caller()
        );
        assert_eq!(transactions.counts().unwrap().total, 0);
        assert_eq!(verify(app, id, "123456").await.0, StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn intermediate_http_progress_is_exclusive_and_cannot_revive_expired_or_cancelled_receipts() {
    for cancel in [false, true] {
        let provider = ChallengeProvider::default();
        provider.image_required.store(true, Ordering::SeqCst);
        let (app, transactions) = app(&provider);
        let id = start(app.clone(), CredentialMode::Client).await;
        provider.verify_gate.enable();
        let worker = app.clone();
        let selected = id.clone();
        let task = tokio::spawn(async move {
            json_request_with_headers(
                worker,
                Method::POST,
                &format!("/v1/auth/challenges/{selected}/verify"),
                Some(json!({"action":"submit_image","answer":"42"})),
            )
            .await
        });
        provider.verify_gate.entered().await;
        assert_eq!(
            verify(app.clone(), &id, "123456").await.0,
            StatusCode::CONFLICT
        );
        if cancel {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
        } else {
            expire(&transactions, &id);
            provider.verify_gate.open();
            let (status, headers, response) = task.await.unwrap();
            assert_eq!(status, StatusCode::NOT_FOUND, "{response}");
            assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        }
        assert_eq!(transactions.counts().unwrap().total, 0);
        assert_eq!(verify(app, &id, "123456").await.0, StatusCode::NOT_FOUND);
    }
}

#[tokio::test]
async fn malformed_actions_cannot_override_challenge_ownership_or_fall_back_to_legacy_fields() {
    let provider = ChallengeProvider::default();
    let (app, transactions) = app(&provider);
    let id = start(app.clone(), CredentialMode::Client).await;
    for action in [
        json!({"action":"refresh_image","code":"123456"}),
        json!({"action":"refresh_image","account":"A"}),
        json!({"action":"submit_image","answer":"42","credential_mode":"both"}),
        json!({"action":"submit_code","code":"123456","captcha":"123456"}),
        json!({"action":"submit_image","code":"123456"}),
        json!({"action":"unknown","code":"123456"}),
    ] {
        let (status, headers, _) = json_request_with_headers(
            app.clone(),
            Method::POST,
            &format!("/v1/auth/challenges/{id}/verify"),
            Some(action),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(headers[header::CACHE_CONTROL], "no-store");
        assert_eq!(transactions.counts().unwrap().total, 1);
    }
    assert_eq!(provider.actions.load(Ordering::SeqCst), 0);
    assert_eq!(provider.verified.load(Ordering::SeqCst), 0);
    assert_eq!(verify(app, &id, "123456").await.0, StatusCode::OK);
}
