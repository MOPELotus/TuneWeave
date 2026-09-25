use super::*;
use tuneweave_core::{
    CloudUploadPublishMetadata, CloudUploadStep, CloudUploadStepKind, CloudUploadStepResponse,
    CloudUploadStrategy, CloudUploadTransfer, CloudUploadTransferRequest, CloudUploadTransferState,
};

#[derive(Clone, Default)]
struct TransferProvider {
    calls: Arc<Mutex<Vec<String>>>,
    caller: bool,
}
impl TransferProvider {
    fn record(&self, operation: &str, id: &str, account: Option<&str>) {
        assert_eq!(id, "transfer-1");
        self.calls.lock().unwrap().push(format!(
            "{operation}:{}:{}",
            account.unwrap(),
            self.caller
        ));
    }
    fn view(&self) -> CloudUploadTransfer {
        CloudUploadTransfer {
            transfer_id: "transfer-1".into(),
            state: CloudUploadTransferState::Transferring,
            file_size: 8,
            offset: 0,
            expires_at: 1234567890,
            upload_required: true,
            step: Some(CloudUploadStep {
                step_id: "step-1".into(),
                kind: CloudUploadStepKind::Upload,
                offset: 0,
                length: 4,
                method: "POST".into(),
                url: "https://storage.example.test/private-object".into(),
                headers: BTreeMap::from([("x-token".into(), "synthetic-private-token".into())]),
                retry_delay_ms: 0,
                attempts_remaining: 3,
            }),
        }
    }
}
#[async_trait]
impl MusicProvider for TransferProvider {
    fn platform(&self) -> Platform {
        Platform::Netease
    }
    fn name(&self) -> &'static str {
        "cloud transfer HTTP fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::AccountCloudUploadTransfer,
            Capability::CallerManagedCredentials,
        ])
    }
    fn with_caller_credential(
        &self,
        credential: &ProviderCredential,
    ) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(credential.secret(), "synthetic-caller");
        Ok(Arc::new(Self {
            caller: true,
            ..self.clone()
        }))
    }
    async fn begin_cloud_upload_transfer(
        &self,
        request: &CloudUploadTransferRequest,
    ) -> Result<CloudUploadTransfer> {
        assert_eq!(request.file.filename, "original.flac");
        assert_eq!(request.file.md5, "0123456789abcdef0123456789abcdef");
        assert_eq!(request.file.file_size, 8);
        assert_eq!(request.file.bitrate, 999_000);
        assert_eq!(request.strategy, CloudUploadStrategy::Chunked);
        self.record("start", "transfer-1", request.file.account.as_deref());
        Ok(self.view())
    }
    async fn cloud_upload_transfer(
        &self,
        id: &str,
        account: Option<&str>,
    ) -> Result<CloudUploadTransfer> {
        self.record("read", id, account);
        Ok(self.view())
    }
    async fn advance_cloud_upload_transfer(
        &self,
        id: &str,
        response: &CloudUploadStepResponse,
        account: Option<&str>,
    ) -> Result<CloudUploadTransfer> {
        assert_eq!(response.step_id, "step-1");
        assert_eq!(response.status, Some(200));
        assert_eq!(response.body, "{\"offset\":4}");
        assert_eq!(response.headers["x-nos-context"], "private-context");
        self.record("advance", id, account);
        Ok(self.view())
    }
    async fn cancel_cloud_upload_transfer(&self, id: &str, account: Option<&str>) -> Result<bool> {
        self.record("cancel", id, account);
        Ok(true)
    }
    async fn publish_cloud_upload_transfer(
        &self,
        id: &str,
        metadata: &CloudUploadPublishMetadata,
        account: Option<&str>,
    ) -> Result<CloudUploadResult> {
        assert_eq!(metadata.song_name.as_deref(), Some("Synthetic Song"));
        self.record("publish", id, account);
        Ok(CloudUploadResult {
            track_ref: Some(ResourceRef::new(Platform::Netease, "123").unwrap()),
            upload_required: Some(true),
            uploaded: Some(true),
            published: true,
            extensions: Extensions::new(),
        })
    }
}
fn fixture() -> (Router, TransferProvider) {
    let p = TransferProvider::default();
    let mut registry = ProviderRegistry::new();
    registry.register(p.clone()).unwrap();
    (build_router(AppState::new(registry, Platform::Netease)), p)
}
fn start() -> Value {
    json!({"file":{"md5":"0123456789abcdef0123456789abcdef","file_size":8,"filename":"original.flac"},"strategy":"chunked"})
}
const ROOT: &str = "/v1/account/cloud/uploads/transfers";
async fn request(
    router: Router,
    method: Method,
    path: &str,
    body: Value,
    credential: Option<&CallerCredential>,
) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(c) = credential {
        builder = builder.header(CALLER_CREDENTIAL_HEADER, &c.value);
    }
    let response = router
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    assert_eq!(response.headers()[header::PRAGMA], "no-cache");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 256 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}
#[tokio::test]
async fn cloud_transfer_http_dispatches_all_operations_with_account_and_caller_scope() {
    for caller in [false, true] {
        let (router, p) = fixture();
        let credential = caller.then(|| {
            CallerCredential::issue(
                &ProviderCredential::new(Platform::Netease, "fixture", "synthetic-caller", None)
                    .unwrap(),
            )
            .unwrap()
        });
        let query = if caller {
            "?platform=netease"
        } else {
            "?platform=netease&account=locker"
        };
        for (method, suffix, body) in [
            (Method::POST, "", start()),
            (Method::GET, "/transfer-1", Value::Null),
            (
                Method::POST,
                "/transfer-1/advance",
                json!({"step_id":"step-1","status":200,"headers":{"x-nos-context":"private-context"},"body":"{\"offset\":4}"}),
            ),
            (
                Method::POST,
                "/transfer-1/complete",
                json!({"song_name":"Synthetic Song"}),
            ),
            (Method::DELETE, "/transfer-1", Value::Null),
        ] {
            let (status, json) = request(
                router.clone(),
                method,
                &format!("{ROOT}{suffix}{query}"),
                body,
                credential.as_ref(),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{suffix}: {json}");
            assert_eq!(json["meta"]["platform"], "netease");
            assert_eq!(
                json["meta"]["account"],
                if caller { Value::Null } else { json!("locker") }
            );
            if suffix.is_empty() {
                assert_eq!(
                    json["data"]["step"]["headers"]["x-token"],
                    "synthetic-private-token"
                );
            }
        }
        assert_eq!(
            *p.calls.lock().unwrap(),
            ["start", "read", "advance", "publish", "cancel"]
                .map(|s| format!("{s}:{}:{caller}", if caller { "default" } else { "locker" }))
        );
    }
}
#[tokio::test]
async fn cloud_transfer_http_rejects_unbound_fields_and_oversized_reports_before_provider() {
    let (router, p) = fixture();
    let mut zero = start();
    zero["file"]["file_size"] = json!(0);
    let mut extra = start();
    extra["file"]["account"] = json!("other");
    for (suffix, body) in [
        ("", zero),
        ("", extra),
        ("", json!({"file":{},"strategy":"arbitrary"})),
        ("/transfer-1/complete", json!({"resource_id":"replacement"})),
        (
            "/transfer-1/advance",
            json!({"step_id":"step-1","status":200,"offset":8}),
        ),
        (
            "/transfer-1/advance",
            json!({"step_id":"step-1","status":null,"body":"unconfirmed"}),
        ),
        (
            "/transfer-1/advance",
            json!({"step_id":"step-1","status":200,"body":"x".repeat(65537)}),
        ),
        (
            "/transfer-1/advance",
            json!({"step_id":"step-1","status":200,"body":"x".repeat(128*1024)}),
        ),
    ] {
        let (status, _) = request(
            router.clone(),
            Method::POST,
            &format!("{ROOT}{suffix}"),
            body,
            None,
        )
        .await;
        assert!(
            matches!(
                status,
                StatusCode::BAD_REQUEST | StatusCode::PAYLOAD_TOO_LARGE
            ),
            "{suffix}: {status}"
        );
    }
    assert!(p.calls.lock().unwrap().is_empty());
    let (status, json) = request(test_app_with_provider(), Method::POST, ROOT, start(), None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(json["error"]["code"], "capability_not_supported");
}
