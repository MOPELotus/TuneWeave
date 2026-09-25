use super::*;

const HIGH: &str = "11111111111111111111111111111111";
const FLAC: &str = "22222222222222222222222222222222";

fn multi_catalogue(uid: &str) -> Vec<Frame> {
    let mut all = vec![
        exchange(uid, "rotated-media-token").into(),
        profile(uid).into(),
    ];
    all.extend(catalogue().into_iter().take(1));
    all.push(
        reply(
            json!([{"audio_id":1901,"audio_name":"Artist - Trusted song", "hash":HASH,
        "filesize":1975000,"bitrate":128,"timelength":123456,
        "hash_320":HIGH,"filesize_320":4938000,"timelength_320":123456,
        "hash_flac":FLAC,"filesize_flac":13888800,"bitrate_flac":900,"timelength_flac":123456}]),
        )
        .into(),
    );
    all
}
fn authorized(hash: &str, bitrate: u64, trial: bool) -> Value {
    let mut body = tracker();
    body["hash"] = json!(hash);
    body["bitRate"] = json!(bitrate);
    body["extName"] = json!(if hash == FLAC { "flac" } else { "mp3" });
    if trial {
        body["timeLength"] = json!(60);
        body["hash_offset"] = json!({"start_ms":0,"end_ms":60000});
    }
    body
}
fn attempt(hash: &str, body: Value) -> Vec<Frame> {
    vec![reply(json!({"auth":"private-user-authorization"})).into(),
        reply(json!({"auth":"private-song-authorization","open_time":1700000000,"hash":hash,"album_audio_id":901})).into(),
        raw(body).into()]
}
fn denied() -> Value {
    json!({"status":3,"errcode":0})
}
fn wanted(account: Option<&str>) -> StreamRequest {
    StreamRequest {
        quality: Quality::Master,
        ..request(account)
    }
}

#[tokio::test]
async fn denied_qualities_use_fresh_grants_within_the_selected_account_for_play_and_download() {
    for caller_owned in [false, true] {
        for behavior in [Behavior::Play, Behavior::Download] {
            let uid = if caller_owned { "333" } else { "111" };
            let mut all = multi_catalogue(uid);
            all.extend(attempt(FLAC, denied()));
            // The middle asset is rejected at song authorization, before tracker.
            all.push(reply(json!({"auth":"second-user-authorization"})).into());
            all.push(raw(json!({"status":0,"error_code":35002})).into());
            all.extend(attempt(HASH, tracker()));
            let mut f = server(all).await;
            f.provider.client.register_test_device();
            let store = store_account(&mut f.provider);
            let other = read(&store, "B");
            let p = if caller_owned {
                f.provider
                    .caller_scope(&caller(KugouLoginClient::Concept))
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let result = p
                .account_media_stream(
                    &track(),
                    &wanted(if caller_owned { None } else { Some("A") }),
                    behavior,
                )
                .await
                .unwrap();
            assert_eq!(result.requested_quality, Quality::Master);
            assert_eq!(result.actual_quality, Quality::Standard);
            assert!(result.trial.is_none());
            assert_eq!(read(&store, "B"), other);
            assert_eq!(
                p.take_response_credential().unwrap().is_some(),
                caller_owned
            );
            let calls = f.requests.await.unwrap();
            assert_eq!(calls.len(), 12);
            assert_eq!(
                calls
                    .iter()
                    .filter(|s| s.starts_with("GET /v1/user_verify?"))
                    .count(),
                3
            );
            for call in &calls[4..] {
                assert!(
                    call.contains(&format!("userid={uid}"))
                        && call.contains("token=rotated-media-token")
                );
            }
            for index in [6, 11] {
                assert!(calls[index].contains(&format!("behavior={}", behavior.name())));
            }
            assert!(calls[6].contains("quality=flac") && calls[11].contains("quality=128"));
        }
    }
}

#[tokio::test]
async fn full_lower_quality_is_preferred_to_trial_but_trial_never_becomes_a_download() {
    for lower_full in [false, true] {
        for behavior in [Behavior::Play, Behavior::Download] {
            let mut all = multi_catalogue("111");
            all.extend(attempt(FLAC, authorized(FLAC, 900000, true)));
            all.extend(attempt(HIGH, denied()));
            all.extend(attempt(HASH, authorized(HASH, 128000, !lower_full)));
            let mut f = server(all).await;
            f.provider.client.register_test_device();
            store_account(&mut f.provider);
            let result = f
                .provider
                .account_media_stream(&track(), &wanted(Some("A")), behavior)
                .await;
            if lower_full {
                let stream = result.unwrap();
                assert_eq!(stream.actual_quality, Quality::Standard);
                assert!(stream.trial.is_none());
            } else if behavior == Behavior::Play {
                let stream = result.unwrap();
                assert_eq!(stream.actual_quality, Quality::Lossless);
                assert!(stream.trial.is_some());
            } else {
                assert_eq!(result.unwrap_err().code, ErrorCode::PermissionDenied);
            }
            assert_eq!(f.requests.await.unwrap().len(), 13);
        }
    }
}

#[tokio::test]
async fn invalid_or_encrypted_tracker_responses_do_not_trigger_quality_fallback() {
    for (body, expected, update) in [
        (
            json!({"status":0,"error_code":20017}),
            ErrorCode::AuthenticationRequired,
            false,
        ),
        (
            json!({"status":0,"error_code":99999}),
            ErrorCode::UpstreamError,
            true,
        ),
        (json!({"status":1}), ErrorCode::UpstreamError, true),
        (
            json!({"status":3,"error_code":0,"errcode":20017}),
            ErrorCode::UpstreamError,
            true,
        ),
        (
            {
                let mut v = authorized(FLAC, 900000, false);
                v["extName"] = json!("kgm");
                v
            },
            ErrorCode::PermissionDenied,
            true,
        ),
        (
            {
                let mut v = authorized(FLAC, 900000, false);
                v["hash"] = json!(HASH);
                v
            },
            ErrorCode::UpstreamError,
            true,
        ),
    ] {
        let mut all = multi_catalogue("333");
        all.extend(attempt(FLAC, body));
        let f = server(all).await;
        f.provider.client.register_test_device();
        let p = f
            .provider
            .caller_scope(&caller(KugouLoginClient::Standard))
            .unwrap();
        let mut error = p.stream(&track(), &wanted(None)).await.unwrap_err();
        assert_eq!(error.code, expected);
        assert_eq!(
            p.take_response_credential()
                .unwrap()
                .or_else(|| error.take_caller_credential_update())
                .is_some(),
            update
        );
        assert_eq!(f.requests.await.unwrap().len(), 7);
    }
}

#[tokio::test]
async fn user_denial_and_http_rate_limiting_do_not_retry_lower_assets() {
    for user_denial in [false, true] {
        let mut all = multi_catalogue("111");
        if user_denial {
            all.push(raw(denied()).into());
        } else {
            all.extend(attempt(FLAC, tracker()));
            all.pop();
            all.push(
                "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .to_owned()
                    .into(),
            );
        }
        let mut f = server(all).await;
        f.provider.client.register_test_device();
        store_account(&mut f.provider);
        let error = f
            .provider
            .stream(&track(), &wanted(Some("A")))
            .await
            .unwrap_err();
        assert_eq!(
            error.code,
            if user_denial {
                ErrorCode::PermissionDenied
            } else {
                ErrorCode::RateLimited
            }
        );
        assert_eq!(
            f.requests.await.unwrap().len(),
            if user_denial { 5 } else { 7 }
        );
    }
}

#[tokio::test]
async fn relogin_at_a_failed_or_lower_quality_boundary_prevents_retry_and_late_delivery() {
    for boundary in [6, 7, 8, 9] {
        let mut all = multi_catalogue("111");
        all.extend(attempt(FLAC, denied()));
        all.extend(attempt(HIGH, authorized(HIGH, 320000, false)));
        all.truncate(boundary);
        let body = match boundary {
            6 => raw(denied()),
            7 => reply(json!({"auth":"lower-user"})),
            8 => reply(json!({"auth":"lower-song","open_time":1700000000,"hash":HIGH})),
            _ => raw(authorized(HIGH, 320000, false)),
        };
        let (frame, release) = paused(body);
        all.push(frame);
        let mut f = server(all).await;
        f.provider.client.register_test_device();
        let store = store_account(&mut f.provider);
        let p = f.provider.clone();
        let running = tokio::spawn(async move { p.stream(&track(), &wanted(Some("A"))).await });
        for _ in 0..=boundary {
            f.seen.recv().await.unwrap();
        }
        store
            .put(&credential("111", "new-generation").stored("A").unwrap())
            .unwrap();
        release.send(()).unwrap();
        assert_eq!(
            running.await.unwrap().unwrap_err().code,
            ErrorCode::Conflict
        );
        assert_eq!(read(&store, "A").native().session.token, "new-generation");
        assert_eq!(f.requests.await.unwrap().len(), boundary + 1);
    }
}

#[tokio::test]
async fn availability_reports_real_full_or_trial_grants_without_urls_or_download_claims() {
    for caller_owned in [false, true] {
        for trial in [false, true] {
            let uid = if caller_owned { "333" } else { "111" };
            let mut body = tracker();
            if trial {
                body["timeLength"] = json!(60);
                body["hash_offset"] = json!({"start_ms":0,"end_ms":60000});
            }
            let mut f = server(frames(uid, body)).await;
            f.provider.client.register_test_device();
            store_account(&mut f.provider);
            let p = if caller_owned {
                f.provider
                    .caller_scope(&caller(KugouLoginClient::Concept))
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let r = TrackAvailabilityRequest {
                account: if caller_owned { None } else { Some("A".into()) },
                ..Default::default()
            };
            let result = p.track_availability("901", &r).await.unwrap();
            assert_eq!(result.playable, !trial);
            assert_eq!(result.requested_bitrate, 999000);
            assert_eq!(result.actual_bitrate, Some(128000));
            assert_eq!(result.extensions.contains_key("trial"), trial);
            let json = serde_json::to_string(&result).unwrap();
            assert!(
                !json.contains("https:") && !json.contains("token") && !json.contains("auth\"")
            );
            assert_eq!(
                p.take_response_credential().unwrap().is_some(),
                caller_owned
            );
            let calls = f.requests.await.unwrap();
            assert!(calls[6].contains("behavior=play"));
        }
    }
}

#[tokio::test]
async fn availability_highest_sentinel_and_explicit_bitrate_select_and_report_actual_quality() {
    for (bitrate, hash, actual, quality) in [
        (999000, FLAC, 900000, "flac"),
        (320000, HIGH, 320000, "320"),
        (128000, HASH, 128000, "128"),
    ] {
        let mut all = multi_catalogue("111");
        all.extend(attempt(hash, authorized(hash, actual, false)));
        let mut f = server(all).await;
        f.provider.client.register_test_device();
        store_account(&mut f.provider);
        let result = f
            .provider
            .track_availability(
                "901",
                &TrackAvailabilityRequest {
                    bitrate,
                    account: Some("A".into()),
                },
            )
            .await
            .unwrap();
        assert!(result.playable);
        assert_eq!(result.actual_bitrate, Some(actual));
        let calls = f.requests.await.unwrap();
        assert_eq!(calls.len(), 7);
        assert!(calls[6].contains(&format!("quality={quality}")));
    }
}

#[tokio::test]
async fn availability_preserves_failure_categories_and_only_explicit_denials_become_unplayable() {
    for (body, expected) in [
        (denied(), None),
        (
            json!({"status":0,"error_code":20017}),
            Some(ErrorCode::AuthenticationRequired),
        ),
        (json!({"status":1}), Some(ErrorCode::UpstreamError)),
    ] {
        let f = server(frames("333", body)).await;
        f.provider.client.register_test_device();
        let p = f
            .provider
            .caller_scope(&caller(KugouLoginClient::Standard))
            .unwrap();
        let result = p
            .track_availability("901", &TrackAvailabilityRequest::default())
            .await;
        if let Some(code) = expected {
            assert_eq!(result.unwrap_err().code, code);
        } else {
            let v = result.unwrap();
            assert!(!v.playable);
            assert_eq!(v.platform_code, Some(0));
            assert!(v.actual_bitrate.is_none());
        }
        assert_eq!(f.requests.await.unwrap().len(), 7);
    }
}

#[tokio::test]
async fn availability_rejects_invalid_inputs_missing_accounts_and_caller_alias_mixing_before_network()
 {
    let mut f = server(vec![]).await;
    store_account(&mut f.provider);
    for bitrate in [0, 320001, 999001, u64::MAX] {
        assert_eq!(
            f.provider
                .track_availability(
                    "901",
                    &TrackAvailabilityRequest {
                        bitrate,
                        account: Some("A".into())
                    }
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        f.provider
            .track_availability("901", &TrackAvailabilityRequest::default())
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    let p = f
        .provider
        .caller_scope(&caller(KugouLoginClient::Standard))
        .unwrap();
    assert_eq!(
        p.track_availability(
            "901",
            &TrackAvailabilityRequest {
                account: Some("A".into()),
                ..Default::default()
            }
        )
        .await
        .unwrap_err()
        .code,
        ErrorCode::InvalidRequest
    );
    assert!(f.requests.await.unwrap().is_empty());
}
