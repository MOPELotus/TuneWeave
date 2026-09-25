use super::super::tests::{Store, received, replies, seed, setup, stored};
use super::*;
use crate::client::{
    catalog::tests::{json_response, response},
    native::{media::tests as data, tests as fixture},
};
use std::sync::atomic::Ordering;
use tokio::sync::Notify;

#[derive(Clone, Copy)]
enum Method {
    Stream,
    Download,
    Availability,
}
async fn invoke(
    p: &KuwoProvider,
    method: Method,
    account: Option<&str>,
) -> Result<serde_json::Value> {
    Ok(match method {
        Method::Stream => {
            serde_json::to_value(p.stream(&data::track(), &data::request(account)).await?).unwrap()
        }
        Method::Download => {
            serde_json::to_value(p.download(&data::track(), &data::request(account)).await?)
                .unwrap()
        }
        Method::Availability => serde_json::to_value(
            p.track_availability(
                "67474",
                &TrackAvailabilityRequest {
                    bitrate: 128_000,
                    account: account.map(str::to_owned),
                },
            )
            .await?,
        )
        .unwrap(),
    })
}

#[tokio::test]
async fn native_media_provider_all_operations_preserve_all_three_owners() {
    for scope in 0..3 {
        for method in [Method::Stream, Method::Download, Method::Availability] {
            let store = Arc::new(Store::default());
            let alias = if scope == 0 { "default" } else { "personal" };
            let selected = seed(&store, alias, "42", "selected-session");
            seed(&store, "other", "43", "other-session");
            let original = store.values.lock().unwrap().clone();
            store.forbid_reads.store(scope == 2, Ordering::SeqCst);
            let mut f = setup(replies(data::flow()), store).await;
            let p = if scope == 2 {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let result = invoke(&p, method, (scope != 2).then_some(alias))
                .await
                .unwrap();
            assert!(result.get("url").is_some() || result["playable"] == true);
            assert_eq!(*f.store.values.lock().unwrap(), original);
            assert!(p.take_response_credential().unwrap().is_none());
            if scope == 2 {
                assert_eq!(f.store.reads.load(Ordering::SeqCst), 0);
            }
            let calls = fixture::requests(&mut f.network, 3).await;
            assert_eq!(data::query(&calls[1])["uid"], "42");
            assert_eq!(data::query(&calls[2])["loginSid"], "selected-session");
        }
    }
}

#[tokio::test]
async fn native_media_late_success_failure_and_401_never_cross_generation_at_any_boundary() {
    for scope in 0..3 {
        for boundary in 0..3 {
            for replacement in 0..3 {
                for failure in [false, true] {
                    let store = Arc::new(Store::default());
                    let alias = if scope == 0 { "default" } else { "personal" };
                    let selected = seed(&store, alias, "42", "selected-session");
                    let other = seed(&store, "other", "43", "other-session")
                        .stored("other")
                        .unwrap();
                    let gate = Arc::new(Notify::new());
                    let bodies = data::flow()
                        .into_iter()
                        .enumerate()
                        .map(|(i, b)| {
                            (
                                if failure && i == boundary {
                                    response(401, "application/json", "", b"private")
                                } else {
                                    b
                                },
                                (i == boundary).then(|| gate.clone()),
                            )
                        })
                        .collect();
                    let mut f = setup(bodies, store).await;
                    let p = if scope == 2 {
                        f.provider
                            .caller_scope(&selected.caller().unwrap())
                            .unwrap()
                    } else {
                        f.provider.clone()
                    };
                    let background = p.clone();
                    let task = tokio::spawn(async move {
                        invoke(&background, Method::Stream, (scope != 2).then_some(alias)).await
                    });
                    for _ in 0..=boundary {
                        received(&mut f).await;
                    }
                    if scope == 2 {
                        let value = if replacement == 0 {
                            None
                        } else {
                            Some(fixture::credential_fixture(
                                if replacement == 1 { "42" } else { "44" },
                                if replacement == 1 {
                                    "selected-session"
                                } else {
                                    "new-session"
                                },
                            ))
                        };
                        *p.caller_credential.as_ref().unwrap().lock().unwrap() = value;
                    } else if replacement == 0 {
                        f.provider.logout(alias).await.unwrap();
                    } else {
                        seed(
                            &f.store,
                            alias,
                            if replacement == 1 { "42" } else { "44" },
                            if replacement == 1 {
                                "selected-session"
                            } else {
                                "new-session"
                            },
                        );
                    }
                    let after = stored(&f.store, alias);
                    let caller_after = p
                        .caller_credential
                        .as_ref()
                        .map(|c| c.lock().unwrap().clone());
                    gate.notify_one();
                    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
                    assert_eq!(stored(&f.store, alias), after);
                    assert_eq!(stored(&f.store, "other"), Some(other));
                    assert_eq!(
                        p.caller_credential
                            .as_ref()
                            .map(|c| c.lock().unwrap().clone()),
                        caller_after
                    );
                    assert!(f.network.seen.try_recv().is_err());
                }
            }
        }
    }
}

#[tokio::test]
async fn native_media_authentication_errors_clear_only_the_exact_owner_but_token_errors_do_not() {
    for scope in 0..3 {
        for boundary in 0..3 {
            let store = Arc::new(Store::default());
            let alias = if scope == 0 { "default" } else { "personal" };
            let selected = seed(&store, alias, "42", "selected-session");
            seed(&store, "other", "43", "other-session");
            let original = store.values.lock().unwrap().clone();
            let mut bodies = data::flow();
            bodies.truncate(boundary + 1);
            bodies[boundary] = if boundary == 2 {
                json_response(&json!({"code":4017,"loginSid":"selected-session"}))
            } else {
                response(401, "application/json", "", b"private")
            };
            let mut f = setup(replies(bodies), store).await;
            let p = if scope == 2 {
                f.provider
                    .caller_scope(&selected.caller().unwrap())
                    .unwrap()
            } else {
                f.provider.clone()
            };
            assert_eq!(
                invoke(&p, Method::Download, (scope != 2).then_some(alias))
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::AuthenticationRequired
            );
            if scope == 2 {
                assert!(
                    p.caller_credential
                        .as_ref()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .is_none()
                );
                assert_eq!(*f.store.values.lock().unwrap(), original);
            } else {
                assert!(stored(&f.store, alias).is_none());
                assert_eq!(stored(&f.store, "other"), original.get("other").cloned());
            }
            fixture::requests(&mut f.network, boundary + 1).await;
        }
        let store = Arc::new(Store::default());
        let selected = seed(&store, "personal", "42", "selected-session");
        let original = store.values.lock().unwrap().clone();
        let mut bodies = data::flow();
        bodies[2] = json_response(&json!({"code":4018,"loginSid":"selected-session"}));
        let mut f = setup(replies(bodies), store).await;
        let p = if scope == 2 {
            f.provider
                .caller_scope(&selected.caller().unwrap())
                .unwrap()
        } else {
            f.provider.clone()
        };
        assert_eq!(
            invoke(&p, Method::Stream, (scope != 2).then_some("personal"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        assert_eq!(*f.store.values.lock().unwrap(), original);
        if scope == 2 {
            assert!(
                p.caller_credential
                    .as_ref()
                    .unwrap()
                    .lock()
                    .unwrap()
                    .is_some()
            );
        }
        fixture::requests(&mut f.network, 3).await;
    }
}

#[tokio::test]
async fn native_media_cancellation_and_budget_expiry_do_not_mutate_credentials_or_continue_requests()
 {
    for scope in 0..3 {
        for boundary in 0..3 {
            for cancel in [false, true] {
                let store = Arc::new(Store::default());
                let alias = if scope == 0 { "default" } else { "personal" };
                let selected = seed(&store, alias, "42", "selected-session");
                let original = store.values.lock().unwrap().clone();
                let gate = Arc::new(Notify::new());
                let mut f = setup(
                    data::flow()
                        .into_iter()
                        .enumerate()
                        .map(|(i, b)| (b, (i == boundary).then(|| gate.clone())))
                        .collect(),
                    store,
                )
                .await;
                let p = if scope == 2 {
                    f.provider
                        .caller_scope(&selected.caller().unwrap())
                        .unwrap()
                } else {
                    f.provider.clone()
                };
                let background = p.clone();
                let task = tokio::spawn(async move {
                    background
                        .read_native_media(
                            "67474",
                            &data::request((scope != 2).then_some(alias)),
                            Action::Play,
                            if cancel {
                                Duration::from_secs(60)
                            } else {
                                Duration::from_millis(300)
                            },
                        )
                        .await
                        .map(|_| ())
                });
                for _ in 0..=boundary {
                    received(&mut f).await;
                }
                if cancel {
                    task.abort();
                    assert!(task.await.unwrap_err().is_cancelled());
                } else {
                    assert_eq!(
                        task.await.unwrap().unwrap_err().code,
                        ErrorCode::UpstreamTimeout
                    );
                }
                gate.notify_one();
                assert_eq!(*f.store.values.lock().unwrap(), original);
                assert!(f.network.seen.try_recv().is_err());
                if scope == 2 {
                    assert_eq!(
                        *p.caller_credential.as_ref().unwrap().lock().unwrap(),
                        Some(selected)
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn native_media_two_accounts_can_finish_in_reverse_order_without_crossing_tokens() {
    let store = Arc::new(Store::default());
    seed(&store, "a", "42", "a-session");
    seed(&store, "b", "43", "b-session");
    let original = store.values.lock().unwrap().clone();
    let gate = Arc::new(Notify::new());
    let mut a = setup(
        data::flow()
            .into_iter()
            .enumerate()
            .map(|(i, b)| (b, (i == 2).then(|| gate.clone())))
            .collect(),
        store.clone(),
    )
    .await;
    let mut b = setup(replies(data::flow()), store).await;
    let pa = a.provider.clone();
    let task = tokio::spawn(async move { invoke(&pa, Method::Stream, Some("a")).await });
    for _ in 0..3 {
        received(&mut a).await;
    }
    assert!(
        invoke(&b.provider, Method::Download, Some("b"))
            .await
            .unwrap()["available"]
            == true
    );
    let calls = fixture::requests(&mut b.network, 3).await;
    assert_eq!(data::query(&calls[2])["loginUid"], "43");
    assert_eq!(data::query(&calls[2])["loginSid"], "b-session");
    gate.notify_one();
    assert!(task.await.unwrap().is_ok());
    assert_eq!(*a.store.values.lock().unwrap(), original);
}

#[tokio::test]
async fn native_media_invalid_scope_request_and_missing_credentials_stop_before_io() {
    let store = Arc::new(Store::default());
    let selected = seed(&store, "personal", "42", "selected-session");
    seed(&store, "bad", "42", "injected,loginSid=other");
    let mut f = setup(vec![], store).await;
    for method in [Method::Stream, Method::Download, Method::Availability] {
        assert_eq!(
            invoke(&f.provider, method, Some("missing"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
        assert_eq!(
            invoke(&f.provider, method, Some("bad"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        let caller = f
            .provider
            .caller_scope(&selected.caller().unwrap())
            .unwrap();
        assert_eq!(
            invoke(&caller, method, Some("personal"))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    fixture::requests(&mut f.network, 0).await;
}

fn catalogue_flow() -> Vec<Vec<u8>> {
    vec![
        data::flow().remove(0),
        crate::client::catalog::tests::home(),
        json_response(&json!({
            "code":200,"data":{"musicrid":"MUSIC_67474","rid":67474,"name":"Catalogue title",
            "artist":"Catalogue artist","duration":240,"hasLossless":true,"online":0,"payInfo":{"nplay":"0000"}}
        })),
    ]
}

#[tokio::test]
async fn native_media_catalogue_validates_owner_and_never_exports_catalogue_rights_as_account_rights()
 {
    for scope in 0..3 {
        let store = Arc::new(Store::default());
        let alias = if scope == 0 { "default" } else { "personal" };
        let selected = seed(&store, alias, "42", "selected-session");
        let original = store.values.lock().unwrap().clone();
        store.forbid_reads.store(scope == 2, Ordering::SeqCst);
        let mut f = setup(replies(catalogue_flow()), store).await;
        let p = if scope == 2 {
            f.provider
                .caller_scope(&selected.caller().unwrap())
                .unwrap()
        } else {
            f.provider.clone()
        };
        let t = p
            .track("67474", (scope != 2).then_some(alias))
            .await
            .unwrap();
        assert_eq!(t.name, "Catalogue title");
        assert_eq!(t.playable, None);
        assert!(t.available_qualities.is_empty());
        assert_eq!(t.extensions["catalogue_scope"], "public");
        assert_eq!(*f.store.values.lock().unwrap(), original);
        assert!(p.requires_download_authorization((scope != 2).then_some(alias)));
        assert!(!f.provider.requires_download_authorization(None));
        let calls = fixture::requests(&mut f.network, 3).await;
        for call in &calls[1..] {
            assert!(!call.contains("selected-session"));
            assert!(!call.contains("loginUid"));
        }
    }
}

#[tokio::test]
async fn native_media_catalogue_discards_late_success_errors_and_logout_results() {
    for scope in 0..3 {
        for boundary in 0..3 {
            for fail in [false, true] {
                let store = Arc::new(Store::default());
                let alias = if scope == 0 { "default" } else { "personal" };
                let selected = seed(&store, alias, "42", "selected-session");
                let gate = Arc::new(Notify::new());
                let bodies = catalogue_flow()
                    .into_iter()
                    .enumerate()
                    .map(|(i, b)| {
                        (
                            if fail && i == boundary {
                                response(401, "application/json", "", b"private")
                            } else {
                                b
                            },
                            (i == boundary).then(|| gate.clone()),
                        )
                    })
                    .collect();
                let mut f = setup(bodies, store).await;
                let p = if scope == 2 {
                    f.provider
                        .caller_scope(&selected.caller().unwrap())
                        .unwrap()
                } else {
                    f.provider.clone()
                };
                let background = p.clone();
                let task = tokio::spawn(async move {
                    background
                        .track("67474", (scope != 2).then_some(alias))
                        .await
                });
                for _ in 0..=boundary {
                    received(&mut f).await;
                }
                if scope == 2 {
                    *p.caller_credential.as_ref().unwrap().lock().unwrap() = None;
                } else {
                    f.provider.logout(alias).await.unwrap();
                }
                let after = f.store.values.lock().unwrap().clone();
                gate.notify_one();
                assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
                assert_eq!(*f.store.values.lock().unwrap(), after);
                assert!(f.network.seen.try_recv().is_err());
            }
        }
    }
}
