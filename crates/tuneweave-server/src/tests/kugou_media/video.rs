use super::*;

#[tokio::test]
async fn kugou_account_video_http_details_stream_batches_and_redirects_preserve_scoped_credentials()
{
    for caller in [false, true] {
        for (uri, body) in [
            ("/v1/videos/kugou:mv:17", None),
            (
                "/v1/videos/details?refs=kugou:mv:17,kugou:17,kugou:mv:17",
                None,
            ),
            ("/v1/videos/kugou:video:17/stream?resolution=480", None),
            (
                "/v1/videos/streams?refs=kugou:mv:17,kugou:17,kugou:mv:17&resolution=480",
                None,
            ),
            (
                "/v1/videos/details",
                Some(json!({"refs":["kugou:video:17","kugou:video:17"]})),
            ),
            (
                "/v1/videos/streams",
                Some(json!({"refs":["kugou:video:17","kugou:video:17"],"resolution":480})),
            ),
            (
                "/v1/videos/kugou:mv:17/stream/redirect?resolution=480",
                None,
            ),
        ] {
            let (router, _) = app(false, None);
            let request = if let Some(mut body) = body {
                if !caller {
                    body["account"] = json!("A");
                }
                post(uri, body, caller)
            } else {
                let uri = if caller {
                    uri.into()
                } else {
                    format!(
                        "{uri}{}account=A",
                        if uri.contains('?') { '&' } else { '?' }
                    )
                };
                request(&uri, caller)
            };
            let response = router.oneshot(request).await.unwrap();
            let redirect = uri.contains("/redirect");
            if redirect {
                assert_eq!(
                    response.headers()[header::LOCATION],
                    "https://mvwebfs.tx.kugou.com/account.mp4"
                );
            }
            let body = inspect(
                response,
                if redirect {
                    StatusCode::FOUND
                } else {
                    StatusCode::OK
                },
                caller,
            )
            .await;
            if uri.contains("streams?") {
                assert_eq!(body["data"][0]["video_ref"], "kugou:mv:17");
                assert_eq!(body["data"][2]["video_ref"], "kugou:mv:17");
            }
        }
    }
}

#[tokio::test]
async fn kugou_account_video_denials_have_no_redirect_or_anonymous_audio_fallback() {
    for caller in [false, true] {
        for redirect in [false, true] {
            let (router, calls) = app(true, None);
            let mut uri = format!(
                "/v1/videos/kugou:mv:17/stream{}?resolution=480",
                if redirect { "/redirect" } else { "" }
            );
            if !caller {
                uri.push_str("&account=A");
            }
            let response = router.oneshot(request(&uri, caller)).await.unwrap();
            assert!(!response.headers().contains_key(header::LOCATION));
            let body = inspect(
                response,
                if redirect {
                    StatusCode::NOT_FOUND
                } else {
                    StatusCode::OK
                },
                caller,
            )
            .await;
            if !redirect {
                assert_eq!(body["data"]["available"], false);
                assert!(body["data"]["url"].is_null());
            }
            assert_eq!(*calls.lock().unwrap(), ["kugou:video_stream"]);
        }
    }
}

#[tokio::test]
async fn kugou_account_video_http_auth_and_conflict_errors_suppress_pending_caller_updates() {
    for uri in [
        "/v1/videos/kugou:mv:17/stream",
        "/v1/videos/streams?refs=kugou:mv:17,kugou:mv:17",
        "/v1/videos/kugou:video:17/stream/redirect",
    ] {
        for (code, status, rotation) in [
            (
                ErrorCode::AuthenticationRequired,
                StatusCode::UNAUTHORIZED,
                false,
            ),
            (ErrorCode::Conflict, StatusCode::CONFLICT, false),
            (ErrorCode::UpstreamError, StatusCode::BAD_GATEWAY, true),
        ] {
            let (router, _) = app(false, Some(code));
            let response = router.oneshot(request(uri, true)).await.unwrap();
            assert!(!response.headers().contains_key(header::LOCATION));
            inspect(response, status, rotation).await;
        }
    }
}

async fn materialized(router: &Router, kind: &str, caller: bool) -> Value {
    let mut body = json!({"items":[{"ref":format!("kugou:{kind}:17"),"kind":kind}]});
    if !caller {
        body["accounts"] = json!({"kugou":"A"});
    }
    let response = router
        .clone()
        .oneshot(post("/v1/uni/materialize/items", body, caller))
        .await
        .unwrap();
    inspect(response, StatusCode::OK, caller).await["data"]["items"][0].clone()
}

#[tokio::test]
async fn kugou_account_video_uni_persisted_and_client_items_use_the_current_playback_credentials() {
    for kind in ["mv", "video"] {
        for persisted in [false, true] {
            for caller in [false, true] {
                let (router, calls) = app(false, None);
                let item = materialized(&router, kind, !caller).await;
                assert!(!item.to_string().contains("media-http"));
                let request = if persisted {
                    let (_, created) = json_request_from(
                        router.clone(),
                        Method::POST,
                        "/v1/uni/playlists",
                        Some(json!({"name":"Videos"})),
                    )
                    .await;
                    let reference = created["data"]["ref"].as_str().unwrap();
                    let mut body =
                        json!({"items":[{"ref":format!("kugou:{kind}:17"),"kind":kind}]});
                    if caller {
                        body["accounts"] = json!({"kugou":"A"});
                    }
                    let response = router
                        .clone()
                        .oneshot(post(
                            &format!("/v1/uni/playlists/{reference}/items"),
                            body,
                            !caller,
                        ))
                        .await
                        .unwrap();
                    inspect(response, StatusCode::OK, !caller).await;
                    let (_, items) = json_response_from(
                        router.clone(),
                        &format!("/v1/uni/playlists/{reference}/items"),
                    )
                    .await;
                    let id = items["data"][0]["id"].as_str().unwrap();
                    let mut uri = format!(
                        "/v1/playlists/{reference}/items/{id}/stream?fallback=false&unblock=false&resolution=480"
                    );
                    if !caller {
                        uri.push_str("&account=A");
                    }
                    request(&uri, caller)
                } else {
                    let mut body =
                        json!({"item":item,"fallback":false,"unblock":false,"resolution":480});
                    if !caller {
                        body["accounts"] = json!({"kugou":"A"});
                    }
                    post("/v1/uni/items/stream", body, caller)
                };
                calls.lock().unwrap().clear();
                let body = inspect(
                    router.oneshot(request).await.unwrap(),
                    StatusCode::OK,
                    caller,
                )
                .await;
                assert_eq!(body["data"]["extensions"]["transport"], "native_video");
                assert_eq!(
                    body["data"]["stream"]["url"],
                    "https://mvwebfs.tx.kugou.com/account.mp4"
                );
                assert_eq!(*calls.lock().unwrap(), ["kugou:video_stream"]);
            }
        }
    }
}

#[tokio::test]
async fn uni_video_fallback_does_not_export_a_credential_invalidated_by_the_origin_error() {
    for (code, rotation) in [
        (ErrorCode::AuthenticationRequired, false),
        (ErrorCode::Conflict, false),
        (ErrorCode::UpstreamError, true),
    ] {
        let (router, calls) = app(false, Some(code));
        let mut item = materialized(&router, "mv", false).await;
        item["snapshot"]["title"] = json!("Verified Song");
        item["snapshot"]["artists"] = json!(["Artist"]);
        item["snapshot"]["duration_ms"] = json!(123000);
        calls.lock().unwrap().clear();
        let response = router
            .oneshot(post(
                "/v1/uni/items/stream",
                json!({"item":item,"fallback":true,"unblock":false,"fallback_platforms":"netease"}),
                true,
            ))
            .await
            .unwrap();
        let body = inspect(response, StatusCode::OK, rotation).await;
        assert_eq!(body["data"]["stream"]["resolved_platform"], "netease");
        assert_eq!(
            *calls.lock().unwrap(),
            ["kugou:video_stream", "netease:stream"]
        );
    }
}
