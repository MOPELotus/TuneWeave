use super::*;
use serde_json::Value;
use tuneweave_core::MiguNativeMvFormat;

const NATIVE_TOKEN: &str = "native-mv-token-fixture";

fn native_grant() -> Value {
    json!({"code":"000000","data":{
        "playUrl":"http://freevod.nf.migu.cn:8080/hls/v2/opaque-grant/index.m3u8?playSessionId=grant&resourceId=7&resourceType=D&userId=opaque-account-value",
        "formatType":"SQ","offset":3000,"backgroundDuration":0,
        "vipResolution":false,"uhdVip":false
    }})
}

fn validation(uid: &str) -> String {
    let data = json!({"code":"000000","data":{"userInfoItem":{"userId":uid}}});
    let body = crate::client::native_http::encode(data.to_string().as_bytes());
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn account_frames() -> Vec<String> {
    vec![
        profile("111", "pacmtoken: profile-pacm\r\n"),
        reply(
            json!({"code":"000000","data":NATIVE_TOKEN}),
            Some("exchange-pacm"),
        ),
        profile("111", "pacmtoken: verified-pacm\r\n"),
        validation("111"),
        reply(resource(), None),
        reply(native_grant(), None),
        manifest(),
        profile("111", "pacmtoken: final-pacm\r\n"),
    ]
}

fn native_request(account: Option<&str>) -> MiguNativeMvStreamRequest {
    MiguNativeMvStreamRequest {
        account: account.map(str::to_owned),
        format: MiguNativeMvFormat::Sq,
    }
}

async fn wire_result(wire: tokio::task::JoinHandle<Vec<String>>) -> Vec<String> {
    tokio::time::timeout(Duration::from_secs(5), wire)
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn native_account_mv_selected_owners_bind_verified_token_and_keep_cdn_anonymous() {
    for mode in ["default", "named", "caller"] {
        let (mut p, wire) = server(account_frames()).await;
        let (store, original, alias) = setup(&mut p, mode);
        // A caller credential selects its account without requiring a stored alias.
        let request = native_request((mode != "caller").then_some(alias));
        let stream = p.migu_native_mv_stream("7", &request).await.unwrap();
        assert_eq!(stream.extensions["authorization_scope"], "selected_account");
        assert_eq!(stream.extensions["source_user_id"], "111");
        assert_eq!(stream.extensions["actual_format"], "SQ");
        assert_eq!(stream.extensions["format_fallback_allowed"], false);
        assert_eq!(stream.source_range.unwrap().start_ms, 3000);
        assert_eq!(stream.source_range.unwrap().end_ms, 8000);
        assert_eq!(stream.duration_ms, Some(5000));
        assert!(stream.headers.is_empty());
        let output = serde_json::to_string(&stream).unwrap();
        for secret in [
            NATIVE_TOKEN,
            "initial-pacm",
            "profile-pacm",
            "exchange-pacm",
            "verified-pacm",
            "final-pacm",
            "do-not-retain-session",
        ] {
            assert!(!output.contains(secret));
        }
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        if mode == "caller" {
            assert_eq!(read(&store, alias), original);
            let update = p.take_response_credential().unwrap().unwrap();
            assert_eq!(
                MiguCredential::parse_caller(&update).unwrap().token(),
                "final-pacm"
            );
            assert!(!update.secret().contains(NATIVE_TOKEN));
            assert!(!update.secret().contains("do-not-retain-session"));
            assert!(p.take_response_credential().unwrap().is_none());
        } else {
            assert_eq!(read(&store, alias).token(), "final-pacm");
            assert!(p.take_response_credential().unwrap().is_none());
        }
        let wire = wire_result(wire).await;
        assert_eq!(wire.len(), 8);
        assert!(
            wire[1].starts_with("GET /user/h5/token/v1.0?uSessionId=do-not-retain-session&_t=")
        );
        assert!(wire[3].starts_with("GET /user/token-validate/v2.0?"));
        assert!(
            wire[5].starts_with(
                "GET /strategy/mvplayinfo/by-priority/v1.1?contentId=7&formatType=SQ "
            )
        );
        assert!(wire[5].contains(&format!("token: {NATIVE_TOKEN}\r\n")));
        assert!(wire[5].contains("signversion: V005\r\n"));
        for header in [
            "version: 8.9.1\r\n",
            "appid: music\r\n",
            "os: Android\r\n",
            "pkgname: cmccwm.mobilemusic\r\n",
            "recommendstatus: 0\r\n",
            "randomsessionkey: 000000\r\n",
        ] {
            assert!(wire[5].contains(header));
        }
        let ce = wire[5]
            .lines()
            .find_map(|line| line.strip_prefix("ce: "))
            .unwrap();
        // CE is hex encoding with the official additive key. Decode independently
        // here to assert the native grant binds UID, rather than a PACM header.
        let key = b"ccTWaprX2aWmTIgA";
        let decoded = (0..ce.len())
            .step_by(2)
            .enumerate()
            .map(|(i, at)| {
                u8::from_str_radix(&ce[at..at + 2], 16)
                    .unwrap()
                    .wrapping_sub(key[i % key.len()])
            })
            .collect::<Vec<_>>();
        let ce = String::from_utf8(decoded).unwrap();
        assert!(ce.starts_with("deviceId="));
        assert!(ce.ends_with("&uid=111"));
        for at in [4, 5, 6] {
            assert!(!wire[at].to_lowercase().contains("pacmtoken:"));
            assert!(!wire[at].to_lowercase().contains("cookie:"));
        }
        for at in [4, 6] {
            assert!(!wire[at].contains(NATIVE_TOKEN));
            assert!(!wire[at].to_lowercase().contains("\r\nce:"));
        }
        assert!(!wire[5].contains("canFallback"));
        assert!(wire[7].contains("pacmtoken: verified-pacm\r\n"));
    }
}

#[tokio::test]
async fn native_account_mv_requires_string_bridge_and_matching_native_uid_before_grant() {
    for (at, replacement, count, expected) in [
        (
            1,
            reply(json!({"code":"000000","data":{"token":NATIVE_TOKEN}}), None),
            3,
            ErrorCode::UpstreamError,
        ),
        (3, validation("222"), 4, ErrorCode::PermissionDenied),
        (3, validation(""), 4, ErrorCode::UpstreamError),
    ] {
        let mut frames = account_frames();
        frames[at] = replacement;
        frames.truncate(count);
        let (mut p, wire) = server(frames).await;
        let (store, _, alias) = setup(&mut p, "named");
        let error = p
            .migu_native_mv_stream("7", &native_request(Some(alias)))
            .await
            .unwrap_err();
        assert_eq!(error.code, expected);
        assert!(!error.message.contains(NATIVE_TOKEN));
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        let wire = wire_result(wire).await;
        assert_eq!(wire.len(), count);
        assert!(
            wire.iter()
                .all(|request| !request.contains("/strategy/mvplayinfo"))
        );
    }
}

#[tokio::test]
async fn native_account_mv_auto_allows_explicit_fallback_and_zero_offset_stays_a_source_range() {
    let mut frames = account_frames();
    let mut grant = native_grant();
    grant["data"]["formatType"] = json!("HQ");
    grant["data"]["offset"] = json!(0);
    frames[5] = reply(grant, None);
    let (mut p, wire) = server(frames).await;
    let (_, _, alias) = setup(&mut p, "named");
    let mut request = native_request(Some(alias));
    request.format = MiguNativeMvFormat::Auto;
    let stream = p.migu_native_mv_stream("7", &request).await.unwrap();
    assert_eq!(stream.extensions["actual_format"], "HQ");
    assert_eq!(stream.extensions["format_fallback_allowed"], true);
    assert_eq!(stream.source_range.unwrap().start_ms, 0);
    assert_eq!(stream.duration_ms, Some(8000));
    assert_eq!(stream.actual_resolution, None);
    let wire = wire_result(wire).await;
    assert!(wire[5].starts_with(
        "GET /strategy/mvplayinfo/by-priority/v1.1?canFallback=true&contentId=7&formatType=SQ "
    ));
}

#[tokio::test]
async fn native_account_mv_manifest_and_final_uid_failures_never_return_an_authorized_stream() {
    for changed_uid in [false, true] {
        let mut frames = account_frames();
        if changed_uid {
            frames[7] = profile("222", "");
        } else {
            frames[6] =
                "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .into();
            frames.truncate(7);
        }
        let (mut p, wire) = server(frames).await;
        let (store, _, alias) = setup(&mut p, "named");
        let error = p
            .migu_native_mv_stream("7", &native_request(Some(alias)))
            .await
            .unwrap_err();
        assert_eq!(
            error.code,
            if changed_uid {
                ErrorCode::AuthenticationRequired
            } else {
                ErrorCode::UpstreamError
            }
        );
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        if changed_uid {
            assert!(
                store
                    .load_platform(Platform::Migu)
                    .unwrap()
                    .iter()
                    .all(|value| value.account != alias)
            );
        } else {
            assert_eq!(read(&store, alias).token(), "verified-pacm");
        }
        assert_eq!(
            wire_result(wire).await.len(),
            if changed_uid { 8 } else { 7 }
        );
    }
}

#[tokio::test]
async fn native_account_mv_denials_formats_and_background_limits_never_fallback_or_revoke_pacm() {
    for case in 0..7 {
        let mut frames = account_frames();
        let mut grant = native_grant();
        let expected = match case {
            0 => {
                grant["data"]["cannotType"] = json!("needLogin");
                ErrorCode::PermissionDenied
            }
            1 => {
                grant["data"]["formatType"] = json!("HQ");
                ErrorCode::UpstreamError
            }
            2 => {
                grant["data"]["backgroundDuration"] = json!(60);
                ErrorCode::CapabilityNotSupported
            }
            3 => {
                grant["data"]["offset"] = json!(8000);
                ErrorCode::PermissionDenied
            }
            4 => {
                grant["data"]["playUrl"] = json!(
                    "http://freevod.nf.migu.cn:8080/hls/v2/opaque/index.m3u8?playSessionId=x&resourceId=8&resourceType=D"
                );
                ErrorCode::UpstreamError
            }
            _ => ErrorCode::PermissionDenied,
        };
        frames[5] = if case >= 5 {
            format!(
                "HTTP/1.1 {} Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                if case == 5 { 401 } else { 403 }
            )
        } else {
            reply(grant, None)
        };
        frames.truncate(6);
        let (mut p, wire) = server(frames).await;
        let (store, original, alias) = setup(&mut p, "caller");
        let mut error = p
            .migu_native_mv_stream("7", &native_request(Some(alias)))
            .await
            .unwrap_err();
        assert_eq!(error.code, expected, "case {case}");
        assert_eq!(read(&store, alias), original);
        let update = error.take_caller_credential_update().unwrap();
        assert_eq!(
            MiguCredential::parse_caller(&update).unwrap().token(),
            "verified-pacm"
        );
        assert!(!update.secret().contains(NATIVE_TOKEN));
        assert_eq!(wire_result(wire).await.len(), 6);
    }
}

#[tokio::test]
async fn native_account_mv_blocks_secret_reflection_before_manifest_and_after_final_rotation() {
    for secret in [
        NATIVE_TOKEN,
        "do-not-retain-session",
        "initial-pacm",
        "profile-pacm",
        "exchange-pacm",
        "verified-pacm",
        "final-pacm",
    ] {
        let mut grant = native_grant();
        grant["data"]["playUrl"] = json!(format!(
            "http://freevod.nf.migu.cn:8080/hls/v2/opaque/index.m3u8?playSessionId=grant&resourceId=7&resourceType=D&echo={secret}"
        ));
        let mut frames = account_frames();
        frames[5] = reply(grant, None);
        let count = if secret == "final-pacm" { 8 } else { 6 };
        frames.truncate(count);
        let (mut p, wire) = server(frames).await;
        let (_, _, alias) = setup(&mut p, "named");
        let error = p
            .migu_native_mv_stream("7", &native_request(Some(alias)))
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!error.message.contains(secret));
        assert!(
            !serde_json::to_string(&error.details)
                .unwrap()
                .contains(secret)
        );
        assert_eq!(wire_result(wire).await.len(), count);
    }
}

#[tokio::test]
async fn native_account_mv_every_network_boundary_rejects_relogin_and_identity_switch() {
    for mode in ["named", "caller"] {
        for boundary in 0..8 {
            for uid in ["111", "333"] {
                let mut frames = account_frames();
                frames.truncate(boundary + 1);
                let (mut p, seen, release, wire) = gated(frames).await;
                let (store, _, alias) = setup(&mut p, mode);
                let p = Arc::new(p);
                let operation = p.clone();
                let task = tokio::spawn(async move {
                    operation
                        .migu_native_mv_stream("7", &native_request(Some(alias)))
                        .await
                });
                tokio::time::timeout(Duration::from_secs(5), seen)
                    .await
                    .unwrap()
                    .unwrap();
                let token = if mode == "caller" {
                    p.caller_credential
                        .as_ref()
                        .unwrap()
                        .lock()
                        .unwrap()
                        .token()
                        .to_owned()
                } else {
                    read(&store, alias).token().to_owned()
                };
                let replacement = MiguCredential::verified(uid.into(), token).unwrap();
                if mode == "caller" {
                    *p.caller_credential.as_ref().unwrap().lock().unwrap() = replacement;
                } else {
                    store.put(&stored(alias, &replacement)).unwrap();
                }
                release.send(()).unwrap();
                let mut error = tokio::time::timeout(Duration::from_secs(5), task)
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap_err();
                assert_eq!(error.code, ErrorCode::Conflict, "{mode}/{boundary}/{uid}");
                assert!(error.take_caller_credential_update().is_none());
                assert!(p.take_response_credential().unwrap().is_none());
                assert_eq!(read(&store, "other").token(), "unrelated-pacm");
                tokio::time::timeout(Duration::from_secs(5), wire)
                    .await
                    .unwrap()
                    .unwrap();
            }
        }
    }
}

#[tokio::test]
async fn native_account_mv_cancellation_and_deadline_do_not_persist_temporary_tokens() {
    for cancel in [false, true] {
        for boundary in 0..8 {
            let mut frames = account_frames();
            frames.truncate(boundary + 1);
            let (mut p, seen, _release, wire) = gated(frames).await;
            let (store, original, alias) = setup(&mut p, "caller");
            let p = Arc::new(p);
            let operation = p.clone();
            let task = tokio::spawn(async move {
                operation
                    .read_native_mv_stream_bounded(
                        "7",
                        &native_request(None),
                        Duration::from_millis(300),
                    )
                    .await
            });
            tokio::time::timeout(Duration::from_secs(5), seen)
                .await
                .unwrap()
                .unwrap();
            if cancel {
                task.abort();
                assert!(task.await.unwrap_err().is_cancelled());
                assert!(p.take_response_credential().unwrap().is_none());
            } else {
                let mut error = task.await.unwrap().unwrap_err();
                assert_eq!(error.code, ErrorCode::UpstreamTimeout);
                let update = error.take_caller_credential_update();
                if boundary == 0 {
                    assert!(update.is_none());
                } else {
                    let update = update.unwrap();
                    assert_eq!(
                        MiguCredential::parse_caller(&update).unwrap().token(),
                        if boundary <= 2 {
                            "profile-pacm"
                        } else {
                            "verified-pacm"
                        }
                    );
                    assert!(!update.secret().contains(NATIVE_TOKEN));
                    assert!(!update.secret().contains("do-not-retain-session"));
                }
            }
            assert_eq!(read(&store, alias), original);
            wire.abort();
        }
    }
}
