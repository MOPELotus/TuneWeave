use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct AlbumProvider {
    caller: bool,
    digital: bool,
    failure: Option<ErrorCode>,
    update: Mutex<Option<ProviderCredential>>,
    calls: Arc<AtomicUsize>,
}
impl AlbumProvider {
    fn check(&self, digital: bool, account: Option<&str>, write: bool, batch: bool) -> Result<()> {
        assert_eq!(
            digital, self.digital,
            "ordinary and digital routes must dispatch distinct methods"
        );
        assert_eq!(account, Some(if self.caller { "default" } else { "A" }));
        self.calls.fetch_add(1, Ordering::SeqCst);
        if self.caller {
            *self.update.lock().unwrap() = Some(
                ProviderCredential::new(Platform::Migu, "test", "updated-album-session", None)
                    .unwrap(),
            );
        }
        if let Some(code) = self.failure {
            let mut error = TuneWeaveError::new(code, "album collection fixture failure")
                .with_platform(Platform::Migu);
            if write {
                let mut details =
                    json!({"write_outcome":"unconfirmed", "resource_type":self.resource_type()});
                if batch {
                    details["atomic"] = json!(false);
                    details["completed_refs"] = json!(["migu:77"]);
                    details["failed_ref"] = json!("migu:78");
                    details["remaining_refs"] = json!([]);
                }
                error = error.retryable(false).with_details(details);
            }
            return Err(error);
        }
        Ok(())
    }
    fn resource_type(&self) -> &'static str {
        if self.digital { "5" } else { "2003" }
    }
    fn extensions(&self) -> Extensions {
        Extensions::from([("resource_type".into(), json!(self.resource_type()))])
    }
    fn page<T>(&self, item: T, r: &PageRequest) -> Page<T> {
        assert_eq!((r.limit, r.offset), (2, 3));
        Page {
            items: vec![item],
            pagination: PageMeta {
                limit: r.limit,
                offset: r.offset,
                total: Some(4),
                has_more: false,
                next_offset: None,
                extensions: self.extensions(),
            },
        }
    }
    fn subscription(&self, id: &str, subscribed: bool) -> SubscriptionResult {
        SubscriptionResult {
            resource_ref: ResourceRef::new(Platform::Migu, id).unwrap(),
            subscribed,
            extensions: self.extensions(),
        }
    }
    fn write(
        &self,
        digital: bool,
        ids: &[String],
        subscribed: bool,
        account: Option<&str>,
        batch: bool,
    ) -> Result<Vec<SubscriptionResult>> {
        assert_eq!(ids, if batch { vec!["77", "78"] } else { vec!["77"] });
        self.check(digital, account, true, batch)?;
        Ok(ids
            .iter()
            .map(|id| self.subscription(id, subscribed))
            .collect())
    }
}
#[async_trait]
impl MusicProvider for AlbumProvider {
    fn platform(&self) -> Platform {
        Platform::Migu
    }
    fn name(&self) -> &'static str {
        "Migu typed album collection fixture"
    }
    fn capabilities(&self) -> BTreeSet<Capability> {
        BTreeSet::from([
            Capability::CallerManagedCredentials,
            Capability::AccountAlbums,
            Capability::AccountDigitalAlbums,
            Capability::AlbumSubscriptionWrite,
            Capability::DigitalAlbumSubscriptionWrite,
        ])
    }
    fn with_caller_credential(&self, c: &ProviderCredential) -> Result<Arc<dyn MusicProvider>> {
        assert_eq!(c.secret(), "original-album-session");
        Ok(Arc::new(Self {
            caller: true,
            digital: self.digital,
            failure: self.failure,
            update: Mutex::new(None),
            calls: self.calls.clone(),
        }))
    }
    fn take_response_credential(&self) -> Result<Option<ProviderCredential>> {
        Ok(self.update.lock().unwrap().take())
    }
    async fn account_albums(&self, r: &PageRequest) -> Result<Page<Album>> {
        self.check(false, r.account.as_deref(), false, false)?;
        let mut album = sample_album("77");
        album.platform = Platform::Migu;
        album.resource_ref = ResourceRef::new(Platform::Migu, "77").unwrap();
        album.extensions = self.extensions();
        Ok(self.page(album, r))
    }
    async fn user_favorite_albums(&self, uid: &str, r: &PageRequest) -> Result<Page<Album>> {
        assert_eq!(uid, "111");
        self.account_albums(r).await
    }
    async fn account_digital_albums(&self, r: &PageRequest) -> Result<Page<DigitalAlbum>> {
        self.check(true, r.account.as_deref(), false, false)?;
        let mut album = sample_digital_album("77");
        album.platform = Platform::Migu;
        album.resource_ref = ResourceRef::new(Platform::Migu, "77").unwrap();
        album.extensions = self.extensions();
        album.purchased = None;
        album.price = None;
        Ok(self.page(album, r))
    }
    async fn user_favorite_digital_albums(
        &self,
        uid: &str,
        r: &PageRequest,
    ) -> Result<Page<DigitalAlbum>> {
        assert_eq!(uid, "111");
        self.account_digital_albums(r).await
    }
    async fn set_album_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        Ok(self
            .write(false, &[id.into()], subscribed, account, false)?
            .remove(0))
    }
    async fn set_digital_album_subscription(
        &self,
        id: &str,
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<SubscriptionResult> {
        Ok(self
            .write(true, &[id.into()], subscribed, account, false)?
            .remove(0))
    }
    async fn set_album_subscriptions(
        &self,
        ids: &[String],
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<Vec<SubscriptionResult>> {
        self.write(false, ids, subscribed, account, true)
    }
    async fn set_digital_album_subscriptions(
        &self,
        ids: &[String],
        subscribed: bool,
        account: Option<&str>,
    ) -> Result<Vec<SubscriptionResult>> {
        self.write(true, ids, subscribed, account, true)
    }
}
fn app(digital: bool, failure: Option<ErrorCode>) -> (Router, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = ProviderRegistry::new();
    registry
        .register(AlbumProvider {
            caller: false,
            digital,
            failure,
            update: Mutex::new(None),
            calls: calls.clone(),
        })
        .unwrap();
    (build_router(AppState::new(registry, Platform::Migu)), calls)
}
fn credential() -> String {
    CallerCredential::issue(
        &ProviderCredential::new(Platform::Migu, "test", "original-album-session", None).unwrap(),
    )
    .unwrap()
    .value
}

#[tokio::test]
async fn migu_album_routes_keep_resource_types_and_rotations_on_read_write_and_partial_failure() {
    for digital in [false, true] {
        for caller in [false, true] {
            for (failure, status) in [
                (None, StatusCode::OK),
                (Some(ErrorCode::UpstreamError), StatusCode::BAD_GATEWAY),
                (
                    Some(ErrorCode::AuthenticationRequired),
                    StatusCode::UNAUTHORIZED,
                ),
                (Some(ErrorCode::Conflict), StatusCode::CONFLICT),
            ] {
                for operation in 0..6 {
                    let kind = if digital { "digital-albums" } else { "albums" };
                    let resource_type = if digital { "5" } else { "2003" };
                    let write = operation >= 2;
                    let batch = operation >= 4;
                    let subscribed = operation % 2 == 0;
                    let method = if !write {
                        Method::GET
                    } else if subscribed {
                        Method::PUT
                    } else {
                        Method::DELETE
                    };
                    let mut uri = if operation == 1 {
                        format!("/v1/users/migu:111/favorites/{kind}?limit=2&offset=3")
                    } else if !write {
                        format!("/v1/account/library/{kind}?platform=migu&limit=2&offset=3")
                    } else if batch {
                        format!("/v1/account/library/{kind}")
                    } else {
                        format!("/v1/account/library/{kind}/migu:77?")
                    };
                    let mut body = if batch {
                        json!({"refs":["migu:77","migu:78"]})
                    } else {
                        Value::Null
                    };
                    if !caller {
                        if batch {
                            body["account"] = json!("A");
                        } else {
                            uri.push_str("&account=A");
                        }
                    }
                    let mut request = Request::builder().method(method).uri(&uri);
                    if caller {
                        request = request.header(CALLER_CREDENTIAL_HEADER, credential());
                    }
                    let request = if batch {
                        request
                            .header(header::CONTENT_TYPE, "application/json")
                            .body(Body::from(body.to_string()))
                    } else {
                        request.body(Body::empty())
                    }
                    .unwrap();
                    let (router, calls) = app(digital, failure);
                    let response = router.oneshot(request).await.unwrap();
                    assert_eq!(response.status(), status, "{uri}");
                    assert_eq!(calls.load(Ordering::SeqCst), 1);
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
                    let expected = caller
                        && !matches!(
                            failure,
                            Some(ErrorCode::AuthenticationRequired | ErrorCode::Conflict)
                        );
                    assert_eq!(update.is_some(), expected);
                    assert_eq!(body["meta"]["caller_credential"].is_object(), expected);
                    if let Some(update) = update {
                        assert_eq!(
                            body["meta"]["caller_credential"]["value"],
                            update.strip_prefix("migu=").unwrap()
                        );
                    }
                    if failure.is_none() {
                        if !write {
                            assert_eq!(body["data"][0]["ref"], "migu:77");
                            assert_eq!(
                                body["data"][0]["extensions"]["resource_type"],
                                resource_type
                            );
                            assert_eq!(body["meta"]["pagination"]["total"], 4);
                            assert_eq!(body["meta"]["pagination"]["offset"], 3);
                            if digital {
                                assert!(body["data"][0]["purchased"].is_null());
                            }
                        } else {
                            let items = if batch {
                                body["data"].as_array().unwrap().iter().collect::<Vec<_>>()
                            } else {
                                vec![&body["data"]]
                            };
                            assert_eq!(items.len(), if batch { 2 } else { 1 });
                            for (i, item) in items.iter().enumerate() {
                                assert_eq!(item["resource_ref"], format!("migu:{}", 77 + i));
                                assert_eq!(item["subscribed"], subscribed);
                                assert_eq!(item["extensions"]["resource_type"], resource_type);
                            }
                        }
                    } else if write {
                        assert_eq!(body["error"]["details"]["write_outcome"], "unconfirmed");
                        assert_eq!(body["error"]["retryable"], false);
                        if batch {
                            assert_eq!(
                                body["error"]["details"]["completed_refs"],
                                json!(["migu:77"])
                            );
                            assert_eq!(body["error"]["details"]["failed_ref"], "migu:78");
                            assert_eq!(body["error"]["details"]["remaining_refs"], json!([]));
                            assert_eq!(body["error"]["details"]["atomic"], false);
                        }
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn digital_album_routes_reject_invalid_inputs_before_provider_operations() {
    for user in [false, true] {
        for query in [
            "unknown=true",
            "limit=0",
            "limit=101",
            "limit=2&offset=4294967295",
            "offset=-1",
        ] {
            let base = if user {
                "/v1/users/migu:111/favorites/digital-albums"
            } else {
                "/v1/account/library/digital-albums"
            };
            let (router, calls) = app(true, None);
            let response = router
                .oneshot(
                    Request::builder()
                        .uri(format!("{base}?account=A&{query}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert_eq!(calls.load(Ordering::SeqCst), 0);
        }
    }
    for body in [
        json!({"refs":[]}),
        json!({"refs":["migu:77","netease:78"]}),
        json!({"refs":["migu:77"],"unknown":true}),
        json!({"refs":["migu:77"],"account":"A"}),
    ] {
        let (router, calls) = app(true, None);
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/v1/account/library/digital-albums")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(CALLER_CREDENTIAL_HEADER, credential())
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
}
