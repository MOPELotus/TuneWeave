use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct SuggestionProvider {
    caller: bool,
    mode: &'static str,
    calls: Arc<AtomicUsize>,
}

#[async_trait]
impl MusicProvider for SuggestionProvider {
    fn platform(&self) -> Platform {
        Platform::Soda
    }
    fn name(&self) -> &'static str {
        "Account suggestion contract"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::SearchSuggestions,
            Capability::CallerManagedCredentials,
        ])
    }
    fn with_caller_credential(
        &self,
        credential: &ProviderCredential,
    ) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(credential.platform, Platform::Soda);
        assert_eq!(credential.secret(), "suggestion-original");
        Ok(Arc::new(Self {
            caller: true,
            mode: self.mode,
            calls: self.calls.clone(),
        }))
    }
    async fn search_suggestions(
        &self,
        request: &SearchSuggestionRequest,
    ) -> Result<SearchSuggestionList> {
        assert_eq!(
            request.account.as_deref(),
            Some(if self.caller { "default" } else { "personal" })
        );
        assert_eq!(request.query, "周");
        assert_eq!(request.client, SearchSuggestionClient::Pc);
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.mode != "stable" {
            return Err(TuneWeaveError::new(
                match self.mode {
                    "auth" => ErrorCode::AuthenticationRequired,
                    "conflict" => ErrorCode::Conflict,
                    _ => ErrorCode::UpstreamError,
                },
                "account suggestions failed",
            )
            .with_platform(Platform::Soda));
        }
        Ok(SearchSuggestionList {
            query: request.query.clone(),
            client: request.client,
            suggestions: vec![tuneweave_core::SearchSuggestion {
                keyword: "周杰伦".to_owned(),
                kind: Some(SearchKind::Artist),
                display_text: None,
                icon_url: None,
                resource: None,
                extensions: Extensions::new(),
            }],
            recommendations: Vec::new(),
            extensions: Extensions::from([
                ("authenticated".to_owned(), json!(true)),
                ("source_user_id".to_owned(), json!("123456")),
                ("backend".to_owned(), json!("official_pc_account_sug")),
            ]),
        })
    }
    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        if self.caller && self.mode == "stable" && self.calls.load(Ordering::SeqCst) == 1 {
            Ok(Some(ProviderCredential::new(
                Platform::Soda,
                "test",
                "suggestion-final",
                None,
            )?))
        } else {
            Ok(None)
        }
    }
}

fn app(mode: &'static str) -> (Router, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = ProviderRegistry::new();
    registry
        .register(SuggestionProvider {
            caller: false,
            mode,
            calls: calls.clone(),
        })
        .unwrap();
    (build_router(AppState::new(registry, Platform::Soda)), calls)
}

fn caller_header() -> String {
    CallerCredential::issue(
        &ProviderCredential::new(Platform::Soda, "test", "suggestion-original", None).unwrap(),
    )
    .unwrap()
    .value
}

#[tokio::test]
async fn account_suggestions_http_preserves_source_privacy_and_success_only_rotation() {
    for caller in [false, true] {
        for mode in ["stable", "auth", "conflict", "upstream"] {
            let (app, calls) = app(mode);
            let mut path =
                "/v1/search/suggestions?platform=soda&client=pc&q=%20%E5%91%A8%20".to_owned();
            if !caller {
                path.push_str("&account=personal");
            }
            let mut request = Request::builder().uri(path);
            if caller {
                request = request.header(CALLER_CREDENTIAL_HEADER, caller_header());
            }
            let response = app
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert!(
                response.headers()[header::CACHE_CONTROL]
                    .to_str()
                    .unwrap()
                    .contains("no-store")
            );
            let updated = response
                .headers()
                .get(caller_scope::UPDATED_CREDENTIAL_HEADER);
            if caller && mode == "stable" {
                let update = updated.unwrap();
                assert!(update.is_sensitive());
                let parsed = CallerCredential::parse(
                    update.to_str().unwrap().strip_prefix("soda=").unwrap(),
                )
                .unwrap();
                assert_eq!(parsed.secret(), "suggestion-final");
            } else {
                assert!(updated.is_none());
            }
            let status = response.status();
            let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
            let result: Value = serde_json::from_slice(&bytes).unwrap();
            for secret in ["suggestion-original", "suggestion-final", "twc1_"] {
                assert!(!String::from_utf8_lossy(&bytes).contains(secret));
            }
            if mode == "stable" {
                assert_eq!(status, StatusCode::OK);
                assert_eq!(result["data"]["query"], "周");
                assert_eq!(result["data"]["suggestions"][0]["keyword"], "周杰伦");
                assert_eq!(result["data"]["suggestions"][0]["kind"], "artist");
                assert_eq!(result["data"]["extensions"]["source_user_id"], "123456");
                assert_eq!(result["data"]["extensions"]["authenticated"], true);
                assert_eq!(result["meta"]["platform"], "soda");
                if !caller {
                    assert_eq!(result["meta"]["account"], "personal");
                }
            } else {
                assert_ne!(status, StatusCode::OK);
                assert_eq!(
                    result["error"]["code"],
                    match mode {
                        "auth" => "authentication_required",
                        "conflict" => "conflict",
                        _ => "upstream_error",
                    }
                );
                assert!(result.get("data").is_none_or(Value::is_null));
            }
        }
    }
}

#[tokio::test]
async fn account_suggestions_http_rejects_bad_input_or_mixed_sources_before_provider_call() {
    for caller in [false, true] {
        for query in [
            "platform=soda&client=pc",
            "platform=soda&client=unknown&q=x",
            "platform=soda&client=pc&q=x&unexpected=1",
        ] {
            let (app, calls) = app("stable");
            let path = format!(
                "/v1/search/suggestions?{query}{}",
                if caller { "" } else { "&account=personal" }
            );
            let mut request = Request::builder().uri(path);
            if caller {
                request = request.header(CALLER_CREDENTIAL_HEADER, caller_header());
            }
            let response = app
                .oneshot(request.body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert!(
                response.headers()[header::CACHE_CONTROL]
                    .to_str()
                    .unwrap()
                    .contains("no-store")
            );
            assert!(
                response
                    .headers()
                    .get(caller_scope::UPDATED_CREDENTIAL_HEADER)
                    .is_none()
            );
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        }
    }
    let (app, calls) = app("stable");
    let response = app
        .oneshot(
            Request::builder()
                .uri("/v1/search/suggestions?platform=soda&client=pc&q=x&account=personal")
                .header(CALLER_CREDENTIAL_HEADER, caller_header())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(
        response
            .headers()
            .get(caller_scope::UPDATED_CREDENTIAL_HEADER)
            .is_none()
    );
}
