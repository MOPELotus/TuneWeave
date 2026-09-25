use super::*;
use crate::KugouLoginClient;
use crate::provider::library::tests::store_account;
use crate::provider::session::tests::{Frame, exchange, paused, profile, read, reply};

fn account_profile(uid: &str, vip: Value) -> String {
    reply(json!({"userid":uid,"nickname":"Viewer","vip_type":vip}))
}
fn initial(uid: &str) -> Vec<Frame> {
    vec![
        exchange(uid, "rotated-video-token").into(),
        account_profile(uid, json!(6)).into(),
    ]
}
fn selected(caller_owned: bool, kind: VideoResourceKind) -> VideoStreamRequest {
    let mut request = VideoStreamRequest::new(kind, 480);
    request.account = (!caller_owned).then(|| "A".into());
    request
}
fn caller(client: KugouLoginClient) -> ProviderCredential {
    let mut c = credential("333", "caller-video-token").native().clone();
    c.session.client = client;
    c.caller().unwrap()
}

#[tokio::test]
async fn native_video_privileges_and_tracker_use_the_same_verified_account_and_client_signatures() {
    for client in [KugouLoginClient::Standard, KugouLoginClient::Concept] {
        for caller_owned in [false, true] {
            for kind in [VideoResourceKind::Mv, VideoResourceKind::Video] {
                let uid = if caller_owned { "333" } else { "111" };
                let mut all = initial(uid);
                all.extend([
                    raw(details(&[1])).into(),
                    raw(rights(vec![permission(1)])).into(),
                    raw(tracker(1)).into(),
                ]);
                let mut f = server(all).await;
                let store = store_account(&mut f.provider);
                if !caller_owned {
                    let mut c = read(&store, "A").native().clone();
                    c.session.client = client;
                    store.put(&c.stored("A").unwrap()).unwrap();
                }
                let other = read(&store, "B");
                let p = if caller_owned {
                    f.provider.caller_scope(&caller(client)).unwrap()
                } else {
                    f.provider.clone()
                };
                let prefix = if kind == VideoResourceKind::Mv {
                    "mv:"
                } else {
                    "video:"
                };
                let ids = vec![format!("{prefix}1"), "1".into(), format!("{prefix}1")];
                let result = p
                    .video_streams(&ids, &selected(caller_owned, kind))
                    .await
                    .unwrap();
                assert_eq!(result.len(), 3);
                for (stream, id) in result.iter().zip(&ids) {
                    assert_eq!(stream.video_ref.id(), id);
                    assert!(stream.available);
                    assert_eq!(stream.actual_resolution, Some(432));
                    assert_eq!(stream.size, Some(1000));
                    let json = serde_json::to_string(stream).unwrap();
                    assert!(
                        !json.contains("rotated-video-token")
                            && !json.contains("caller-video-token")
                    );
                }
                assert_eq!(read(&store, "B"), other);
                assert_eq!(
                    p.take_response_credential().unwrap().is_some(),
                    caller_owned
                );
                let calls = f.requests.await.unwrap();
                assert_eq!(calls.len(), 5);
                assert!(!calls[2].contains("rotated-video-token"));
                let catalogue_body: Value =
                    serde_json::from_str(calls[2].split("\r\n\r\n").nth(1).unwrap()).unwrap();
                assert_eq!(catalogue_body["token"], "");
                assert!(!params(&calls[2]).contains_key("userid"));
                for (index, router) in [(3, "media.store.kugou.com"), (4, "trackermv.kugou.com")] {
                    assert!(calls[index].to_ascii_lowercase().contains("kg-rc: 1\r\n"));
                    assert!(calls[index].to_ascii_lowercase().contains("kg-rec: 1\r\n"));
                    let mut q = params(&calls[index]);
                    let signature = q.remove("signature").unwrap();
                    let body = calls[index].split("\r\n\r\n").nth(1).unwrap_or("");
                    assert_eq!(q["userid"], uid);
                    assert_eq!(q["token"], "rotated-video-token");
                    assert_eq!(q["appid"], client.appid().to_string());
                    assert_eq!(q["clientver"], client.clientver().to_string());
                    assert_eq!(q["uuid"], "-");
                    let signing = q.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
                    let expected = if client == KugouLoginClient::Concept {
                        crate::signing::concept_signature(&signing, body.as_bytes())
                    } else {
                        crate::signing::android_signature(&signing, body.as_bytes())
                    };
                    assert_eq!(signature, expected);
                    assert!(
                        calls[index]
                            .to_ascii_lowercase()
                            .contains(&format!("x-router: {router}"))
                    );
                    if index == 3 {
                        let body: Value = serde_json::from_str(body).unwrap();
                        assert_eq!(body["vip"], 6);
                        assert_eq!(body["userid"].as_u64().unwrap().to_string(), uid);
                        assert_eq!(body["token"], "rotated-video-token");
                        assert_eq!(body["mid"], q["mid"]);
                    } else {
                        use md5::{Digest, Md5};
                        let salt = if client == KugouLoginClient::Concept {
                            "185672dd44712f60bb1736df5a377e82"
                        } else {
                            "57ae12eb6890223e355ccfcb74edf70d"
                        };
                        assert_eq!(
                            q["key"],
                            format!(
                                "{:x}",
                                Md5::digest(format!(
                                    "{}{salt}{}{}{uid}",
                                    hash(1),
                                    client.appid(),
                                    q["mid"]
                                ))
                            )
                        );
                        assert_eq!(q["ssl"], "1");
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn native_video_batch_deduplicates_only_inside_one_account_request_and_preserves_denials() {
    let mut all = initial("111");
    all.push(raw(details(&[2, 1])).into());
    let mut denial = permission(2);
    denial["status"] = json!(0);
    denial["info"] = Value::Null;
    all.extend([
        raw(rights(vec![permission(1)])).into(),
        raw(tracker(1)).into(),
        raw(rights(vec![denial])).into(),
    ]);
    let mut f = server(all).await;
    store_account(&mut f.provider);
    let ids = vec!["mv:1".into(), "2".into(), "1".into(), "mv:2".into()];
    let result = f
        .provider
        .video_streams(&ids, &selected(false, VideoResourceKind::Mv))
        .await
        .unwrap();
    assert_eq!(
        result.iter().map(|s| s.available).collect::<Vec<_>>(),
        [true, false, true, false]
    );
    assert!(result[1].url.is_none() && result[3].url.is_none());
    let calls = f.requests.await.unwrap();
    assert_eq!(calls.len(), 6);
    assert_eq!(
        calls
            .iter()
            .filter(|s| s.starts_with("GET /v2/interface/index"))
            .count(),
        1
    );
}

#[tokio::test]
async fn account_video_metadata_works_without_membership_but_media_does_not_guess_missing_vip() {
    for media in [false, true] {
        let mut all = vec![
            exchange("333", "rotated-video-token").into(),
            profile("333").into(),
        ];
        if !media {
            all.push(raw(details(&[1])).into());
        }
        let f = server(all).await;
        let p = f
            .provider
            .caller_scope(&caller(KugouLoginClient::Standard))
            .unwrap();
        if media {
            assert_eq!(
                p.video_stream("1", &selected(true, VideoResourceKind::Mv))
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::CapabilityNotSupported
            );
        } else {
            assert_eq!(
                p.video("mv:1", &VideoDetailRequest::new(VideoResourceKind::Mv))
                    .await
                    .unwrap()
                    .video
                    .resource_ref
                    .id(),
                "mv:1"
            );
        }
        assert!(p.take_response_credential().unwrap().is_some());
        assert_eq!(f.requests.await.unwrap().len(), if media { 2 } else { 3 });
    }
}

#[tokio::test]
async fn video_membership_codes_preserve_explicit_zero_and_reject_malformed_profile_fields() {
    for vip in [
        json!(0),
        json!("0"),
        json!(-1),
        json!(true),
        json!(4294967296u64),
        json!("6.5"),
        Value::Null,
    ] {
        let valid = vip == json!(0) || vip == json!("0");
        let mut all = vec![
            exchange("111", "rotated-video-token").into(),
            account_profile("111", vip.clone()).into(),
        ];
        if valid {
            all.extend([
                raw(details(&[1])).into(),
                raw(rights(vec![permission(1)])).into(),
                raw(tracker(1)).into(),
            ]);
        }
        let mut f = server(all).await;
        store_account(&mut f.provider);
        let result = f
            .provider
            .video_stream("1", &selected(false, VideoResourceKind::Mv))
            .await;
        if valid {
            assert!(result.unwrap().available);
        } else {
            assert_eq!(
                result.unwrap_err().code,
                if vip.is_null() {
                    ErrorCode::CapabilityNotSupported
                } else {
                    ErrorCode::UpstreamError
                }
            );
        }
        let calls = f.requests.await.unwrap();
        assert_eq!(calls.len(), if valid { 5 } else { 2 });
        if valid {
            let body: Value =
                serde_json::from_str(calls[3].split("\r\n\r\n").nth(1).unwrap()).unwrap();
            assert_eq!(body["vip"], 0);
        }
    }
}

#[tokio::test]
async fn video_authentication_and_transport_failures_preserve_only_valid_caller_rotations() {
    for tracker_stage in [false, true] {
        for (response, code, update) in [
            (
                raw(json!({"status":0,"error_code":20017})),
                ErrorCode::AuthenticationRequired,
                false,
            ),
            (
                raw(json!({"status":0,"errcode":20018})),
                ErrorCode::AuthenticationRequired,
                false,
            ),
            (
                raw(json!({"status":0,"error_code":99,"errmsg":"private-upstream-message"})),
                ErrorCode::UpstreamError,
                true,
            ),
            (
                "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    .into(),
                ErrorCode::RateLimited,
                true,
            ),
            (
                raw(json!({})).replace("Content-Type: application/json", "Content-Type: text/html"),
                ErrorCode::UpstreamError,
                true,
            ),
        ] {
            let mut all = initial("333");
            all.push(raw(details(&[1])).into());
            if tracker_stage {
                all.push(raw(rights(vec![permission(1)])).into());
            }
            all.push(response.into());
            let f = server(all).await;
            let p = f
                .provider
                .caller_scope(&caller(KugouLoginClient::Concept))
                .unwrap();
            let mut error = p
                .video_stream("1", &selected(true, VideoResourceKind::Mv))
                .await
                .unwrap_err();
            assert_eq!(error.code, code);
            assert!(!format!("{error:?}").contains("private-upstream-message"));
            assert_eq!(
                p.take_response_credential()
                    .unwrap()
                    .or_else(|| error.take_caller_credential_update())
                    .is_some(),
                update
            );
            assert_eq!(
                f.requests.await.unwrap().len(),
                if tracker_stage { 5 } else { 4 }
            );
        }
    }
}

#[tokio::test]
async fn video_logout_and_relogin_at_catalogue_privilege_and_tracker_boundaries_discard_late_results()
 {
    for boundary in 2..=4 {
        for caller_owned in [false, true] {
            let uid = if caller_owned { "333" } else { "111" };
            let mut all = initial(uid);
            all.extend([
                raw(details(&[1])).into(),
                raw(rights(vec![permission(1)])).into(),
                raw(tracker(1)).into(),
            ]);
            all.truncate(boundary);
            let body = match boundary {
                2 => details(&[1]),
                3 => rights(vec![permission(1)]),
                _ => tracker(1),
            };
            let (frame, release) = paused(raw(body));
            all.push(frame);
            let mut f = server(all).await;
            let store = store_account(&mut f.provider);
            let p = if caller_owned {
                f.provider
                    .caller_scope(&caller(KugouLoginClient::Standard))
                    .unwrap()
            } else {
                f.provider.clone()
            };
            let running_p = p.clone();
            let running = tokio::spawn(async move {
                running_p
                    .video_stream("1", &selected(caller_owned, VideoResourceKind::Mv))
                    .await
            });
            for _ in 0..=boundary {
                f.seen.recv().await.unwrap();
            }
            if caller_owned {
                *p.caller_credential.as_ref().unwrap().lock().unwrap() = None;
            } else {
                store
                    .put(&credential("111", "new-video-login").stored("A").unwrap())
                    .unwrap();
            }
            release.send(()).unwrap();
            assert_eq!(
                running.await.unwrap().unwrap_err().code,
                ErrorCode::Conflict
            );
            assert!(p.take_response_credential().unwrap().is_none());
            assert_eq!(f.requests.await.unwrap().len(), boundary + 1);
        }
    }
}

#[tokio::test]
async fn account_video_metadata_batches_check_session_between_catalogue_pages() {
    let ids = (1..=21).map(|n| n.to_string()).collect::<Vec<_>>();
    let mut all = initial("111");
    all.push(raw(details(&(1..=20).collect::<Vec<_>>())).into());
    let (frame, release) = paused(raw(details(&[21])));
    all.push(frame);
    let mut f = server(all).await;
    let store = store_account(&mut f.provider);
    let p = f.provider.clone();
    let running = tokio::spawn(async move {
        p.videos(
            &ids,
            &VideoDetailRequest {
                kind: VideoResourceKind::Mv,
                account: Some("A".into()),
            },
        )
        .await
    });
    for _ in 0..4 {
        f.seen.recv().await.unwrap();
    }
    store.remove(Platform::Kugou, "A").unwrap();
    release.send(()).unwrap();
    assert_eq!(
        running.await.unwrap().unwrap_err().code,
        ErrorCode::Conflict
    );
    assert_eq!(f.requests.await.unwrap().len(), 4);
}

#[tokio::test]
async fn account_video_never_exports_login_tokens_in_media_urls_or_partial_failed_batches() {
    for url in [
        "https://mvwebfs.tx.kugou.com/a?token=rotated-video-token",
        "https://mvwebfs.tx.kugou.com/a?token=rotated%2Dvideo%2Dtoken",
    ] {
        let mut all = initial("111");
        all.push(raw(details(&[1, 2])).into());
        all.extend([
            raw(rights(vec![permission(1)])).into(),
            raw(tracker(1)).into(),
        ]);
        let mut invalid = tracker(2);
        invalid["data"][hash(2).to_lowercase()]["downurl"] = json!(url);
        all.extend([raw(rights(vec![permission(2)])).into(), raw(invalid).into()]);
        let mut f = server(all).await;
        store_account(&mut f.provider);
        let error = f
            .provider
            .video_streams(
                &["1".into(), "2".into()],
                &selected(false, VideoResourceKind::Mv),
            )
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!format!("{error:?}").contains("rotated-video-token"));
        assert_eq!(f.requests.await.unwrap().len(), 7);
    }
}

#[tokio::test]
async fn account_video_input_validation_does_not_read_accounts_or_mix_caller_and_server_modes() {
    let mut f = server(vec![]).await;
    store_account(&mut f.provider);
    let p = f
        .provider
        .caller_scope(&caller(KugouLoginClient::Concept))
        .unwrap();
    assert_eq!(
        p.video_stream("1", &selected(false, VideoResourceKind::Mv))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        p.video_stream("video:1", &selected(true, VideoResourceKind::Mv))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        f.provider
            .video_stream(
                "1",
                &VideoStreamRequest {
                    account: Some("missing".into()),
                    ..request()
                }
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert!(f.requests.await.unwrap().is_empty());
}
