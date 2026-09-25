use super::*;
use crate::client::video_playback::tests::{MANIFEST, grant, resource};
use crate::credential::MiguCredential;
use crate::provider::account_media::tests::{reply, server, setup};
use crate::provider::session::tests::{gated, profile, read, stored};
use std::time::Duration;
use tuneweave_core::{AccountCredentialStore, VideoDetailRequest, VideoResourceKind};

mod native_account;
fn manifest() -> String {
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: application/x-mpegurl\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{MANIFEST}",
        MANIFEST.len()
    )
}
fn frames() -> Vec<String> {
    vec![
        profile("111", "pacmtoken: verified-1\r\n"),
        reply(resource(), None),
        reply(grant(), Some("candidate-2")),
        profile("111", "pacmtoken: verified-2\r\n"),
        manifest(),
    ]
}
fn request(account: Option<&str>) -> VideoStreamRequest {
    VideoStreamRequest {
        account: account.map(str::to_owned),
        ..VideoStreamRequest::new(VideoResourceKind::Mv, 0)
    }
}

#[tokio::test]
async fn native_mv_stream_keeps_the_official_source_range_and_uses_only_anonymous_v005() {
    let grant = json!({
        "code":"000000",
        "info":"操作成功",
        "data":{
            "playUrl":"http://freevod.nf.migu.cn:8080/hls/v2/opaque-grant/index.m3u8?playSessionId=session&resourceId=7&resourceType=D&userId=opaque",
            "formatType":"HQ",
            "offset":3000,
            "backgroundDuration":0,
            "vipResolution":false,
            "uhdVip":false
        }
    });
    let (p, wire) = server(vec![
        reply(resource(), None),
        reply(grant, None),
        manifest(),
    ])
    .await;
    let stream = p
        .migu_native_mv_stream(
            "7",
            &MiguNativeMvStreamRequest {
                format: tuneweave_core::MiguNativeMvFormat::Auto,
                account: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(stream.duration_ms, Some(5000));
    assert_eq!(stream.source_range.unwrap().start_ms, 3000);
    assert_eq!(stream.source_range.unwrap().end_ms, 8000);
    assert_eq!(stream.extensions["actual_format"], "HQ");
    assert_eq!(stream.extensions["format_fallback_allowed"], true);
    assert_eq!(stream.extensions["authorization_scope"], "anonymous_device");
    assert!(
        stream
            .url
            .as_deref()
            .unwrap()
            .starts_with("http://freevod.nf.migu.cn:8080/")
    );

    let wire = wire.await.unwrap();
    assert_eq!(wire.len(), 3);
    assert!(wire[1].starts_with("GET /strategy/mvplayinfo/by-priority/v1.1?"));
    assert!(wire[1].contains("canFallback=true"));
    assert!(wire[1].contains("formatType=SQ"));
    assert!(
        wire[1]
            .to_ascii_lowercase()
            .contains("signversion: v005\r\n")
    );
    assert!(wire[1].to_ascii_lowercase().contains("ce: "));
    assert!(!wire[1].to_ascii_lowercase().contains("token:"));
    assert!(wire[2].starts_with("GET /hls/v2/opaque-grant/index.m3u8?"));
}

#[tokio::test]
async fn native_mv_stream_requires_the_explicit_account_before_network_access() {
    let (p, wire) = server(Vec::new()).await;
    let error = p
        .migu_native_mv_stream(
            "7",
            &MiguNativeMvStreamRequest {
                format: tuneweave_core::MiguNativeMvFormat::Pq,
                account: Some("default".into()),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::AuthenticationRequired);
    assert!(wire.await.unwrap().is_empty());
}
#[tokio::test]
async fn mv_playback_public_and_three_credential_owners_keep_metadata_and_cdn_credentials_separate()
{
    for mode in ["anonymous", "default", "named", "caller"] {
        let responses = if mode == "anonymous" {
            vec![
                reply(resource(), Some("ignored-public-token")),
                reply(grant(), Some("ignored-play-token")),
                manifest(),
            ]
        } else {
            frames()
        };
        let (mut p, wire) = server(responses).await;
        let (store, original, alias) =
            setup(&mut p, if mode == "anonymous" { "default" } else { mode });
        let stream = p
            .video_stream("7", &request((mode != "anonymous").then_some(alias)))
            .await
            .unwrap();
        assert_eq!(stream.duration_ms, Some(8000));
        assert_eq!(stream.format.as_deref(), Some("hls"));
        assert_eq!(stream.actual_resolution, None);
        assert_eq!(stream.extensions["format_type"], "PQ");
        assert!(stream.headers.is_empty());
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        if mode == "caller" {
            assert_eq!(read(&store, alias), original);
            assert_eq!(
                MiguCredential::parse_caller(&p.take_response_credential().unwrap().unwrap())
                    .unwrap()
                    .token(),
                "verified-2"
            );
        } else if mode == "anonymous" {
            assert_eq!(read(&store, alias), original);
            assert!(p.take_response_credential().unwrap().is_none());
        } else {
            assert_eq!(read(&store, alias).token(), "verified-2");
            assert!(p.take_response_credential().unwrap().is_none());
        }
        let wire = wire.await.unwrap();
        let (source, grant, cdn) = if mode == "anonymous" {
            (0, 1, 2)
        } else {
            (1, 2, 4)
        };
        for index in [source, cdn] {
            assert!(
                !wire[index].to_lowercase().contains("pacmtoken:")
                    && !wire[index].to_lowercase().contains("cookie:")
            );
        }
        assert!(wire[grant].starts_with("GET /MIGUM2.0/v1.0/content/mvplayinfo.do?"));
        assert!(wire[grant].contains("needHttps=true"));
        assert!(wire[grant].contains("format=050019"));
        assert!(wire[grant].contains("url=%2Fopaque%252Bvalue%2Ffile.mp4%3FF%3D050019"));
        if mode == "anonymous" {
            assert!(!wire[grant].contains("pacmtoken:") && !wire[grant].contains("deviceId:"));
        } else {
            assert!(wire[grant].to_lowercase().contains("pacmtoken: verified-1"));
            assert!(wire[3].contains("pacmtoken: candidate-2"));
        }
    }
}
#[tokio::test]
async fn mv_playback_explicit_sq_denials_never_downgrade_and_cdn_failures_do_not_revoke_accounts() {
    for case in 0..6 {
        let mut frames = frames();
        let expected = match case {
            0 => {
                frames[2] = reply(
                    json!({"code":"000001","playUrl":"https://invalid"}),
                    Some("candidate-2"),
                );
                frames.truncate(4);
                ErrorCode::PermissionDenied
            }
            1 => {
                frames[2] =
                    "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .into();
                frames.truncate(3);
                ErrorCode::AuthenticationRequired
            }
            2 => {
                frames[4] =
                    "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                        .into();
                ErrorCode::UpstreamError
            }
            3 => {
                frames[3] = profile("222", "");
                frames.truncate(4);
                ErrorCode::AuthenticationRequired
            }
            4 => {
                frames[2] = reply(
                    json!({"code":"000000","playUrl":"https://freevod.nf.migu.cn/opaque/index.m3u8?playSessionId=initial-pacm&resourceId=7&resourceType=D"}),
                    Some("candidate-2"),
                );
                frames.truncate(4);
                ErrorCode::UpstreamError
            }
            _ => {
                let s = MANIFEST.replace("#EXTINF:4,", "#EXTINF:1,");
                frames[4] = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/x-mpegurl\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{s}",
                    s.len()
                );
                ErrorCode::PermissionDenied
            }
        };
        let expected_requests = frames.len();
        let (mut p, wire) = server(frames).await;
        let (store, _, alias) = setup(&mut p, "named");
        let mut r = request(Some(alias));
        r.resolution = 1080;
        let err = p.video_stream("7", &r).await.unwrap_err();
        assert_eq!(err.code, expected, "case{case}");
        assert_eq!(read(&store, "other").token(), "unrelated-pacm");
        if matches!(case, 1 | 3) {
            assert!(
                store
                    .load_platform(Platform::Migu)
                    .unwrap()
                    .iter()
                    .all(|v| v.account != alias)
            );
        } else {
            assert_eq!(read(&store, alias).token(), "verified-2");
        }
        let wire = wire.await.unwrap();
        assert_eq!(wire.len(), expected_requests);
        assert!(wire[2].contains("format=050015"));
    }
}
#[tokio::test]
async fn mv_playback_each_network_boundary_rejects_late_results_after_logout_relogin_or_account_switch()
 {
    for mode in ["default", "named", "caller"] {
        for boundary in 0..5 {
            for action in ["logout", "relogin", "switch"] {
                if mode == "caller" && action == "logout" {
                    continue;
                }
                for late_error in [false, true] {
                    let mut replies = frames();
                    replies.truncate(boundary + 1);
                    if late_error {
                        replies[boundary]="HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into();
                    }
                    let (mut p, seen, release, wire) = gated(replies).await;
                    let (store, _, alias) = setup(&mut p, mode);
                    let p = Arc::new(p);
                    let operation = p.clone();
                    let task = tokio::spawn(async move {
                        operation.video_stream("7", &request(Some(alias))).await
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
                    let next = MiguCredential::verified(
                        if action == "switch" { "333" } else { "111" }.into(),
                        token,
                    )
                    .unwrap();
                    if mode == "caller" {
                        *p.caller_credential.as_ref().unwrap().lock().unwrap() = next.clone();
                    } else if action == "logout" {
                        store.remove(Platform::Migu, alias).unwrap();
                    } else {
                        store.put(&stored(alias, &next)).unwrap();
                    }
                    release.send(()).unwrap();
                    let mut err = tokio::time::timeout(Duration::from_secs(5), task)
                        .await
                        .unwrap()
                        .unwrap()
                        .unwrap_err();
                    assert_eq!(
                        err.code,
                        ErrorCode::Conflict,
                        "{mode}/{boundary}/{action}/{late_error}"
                    );
                    assert!(err.take_caller_credential_update().is_none());
                    assert!(p.take_response_credential().unwrap().is_none());
                    assert_eq!(read(&store, "other").token(), "unrelated-pacm");
                    wire.await.unwrap();
                }
            }
        }
    }
}
#[tokio::test]
async fn mv_playback_cancellation_and_total_deadline_preserve_only_verified_credential_updates() {
    for cancel in [false, true] {
        for boundary in 0..5 {
            let mut replies = frames();
            replies.truncate(boundary + 1);
            let (mut p, seen, _release, wire) = gated(replies).await;
            let (store, original, alias) = setup(&mut p, "caller");
            let p = Arc::new(p);
            let operation = p.clone();
            let task = tokio::spawn(async move {
                operation
                    .read_mv_stream_bounded("7", &request(Some(alias)), Duration::from_millis(300))
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
                    assert_eq!(
                        MiguCredential::parse_caller(&update.unwrap())
                            .unwrap()
                            .token(),
                        if boundary < 4 {
                            "verified-1"
                        } else {
                            "verified-2"
                        }
                    );
                }
            }
            assert_eq!(read(&store, alias), original);
            wire.abort();
        }
    }
}
#[tokio::test]
async fn mv_playback_two_accounts_finishing_in_reverse_order_keep_grants_and_sources_separate() {
    let (mut a, seen, release, wire_a) = gated(frames()).await;
    let (store, _, alias) = setup(&mut a, "named");
    let a = Arc::new(a);
    let operation = a.clone();
    let task =
        tokio::spawn(async move { operation.video_stream("7", &request(Some(alias))).await });
    seen.await.unwrap();
    let frames_b = frames()
        .into_iter()
        .map(|s| {
            s.replace("111", "222")
                .replace("verified-", "other-verified-")
                .replace("candidate-", "other-candidate-")
        })
        .collect();
    let (mut b, wire_b) = server(frames_b).await;
    b.credential_store = Some(store.clone());
    let r = b.video_stream("7", &request(Some("other"))).await.unwrap();
    assert_eq!(r.extensions["source_user_id"], "222");
    release.send(()).unwrap();
    let r = task.await.unwrap().unwrap();
    assert_eq!(r.extensions["source_user_id"], "111");
    assert_eq!(read(&store, "other").token(), "other-verified-2");
    assert_eq!(read(&store, alias).token(), "verified-2");
    wire_a.await.unwrap();
    wire_b.await.unwrap();
}
#[tokio::test]
async fn mv_selected_details_and_statistics_retain_public_catalogue_semantics_and_verified_source()
{
    for mode in ["default", "named", "caller"] {
        for stats in [false, true] {
            let (mut p, wire) = server(vec![
                profile("111", "pacmtoken: verified-1\r\n"),
                reply(resource(), None),
            ])
            .await;
            let (_, _, alias) = setup(&mut p, mode);
            let mut r = VideoDetailRequest::new(VideoResourceKind::Mv);
            r.account = Some(alias.into());
            let value = if stats {
                serde_json::to_value(p.video_stats("7", &r).await.unwrap()).unwrap()
            } else {
                serde_json::to_value(p.video("7", &r).await.unwrap()).unwrap()["video"].clone()
            };
            assert_eq!(value["extensions"]["source_user_id"], "111");
            assert_eq!(value["extensions"]["catalogue_scope"], "public");
            let wire = wire.await.unwrap();
            assert!(!wire[1].contains("pacmtoken:"));
        }
    }
}
#[tokio::test]
async fn mv_selected_metadata_rejects_encoded_secrets_and_late_account_changes() {
    for encoded in [false, true] {
        let mut data = resource();
        data["resource"][0]["summary"] = json!(if encoded {
            "%69nitial-pacm"
        } else {
            "initial-pacm"
        });
        let (mut p, wire) = server(vec![
            profile("111", "pacmtoken: verified-1\r\n"),
            reply(data, None),
        ])
        .await;
        let (_, _, alias) = setup(&mut p, "caller");
        let mut r = VideoDetailRequest::new(VideoResourceKind::Mv);
        r.account = Some(alias.into());
        assert_eq!(
            p.video("7", &r).await.unwrap_err().code,
            ErrorCode::UpstreamError
        );
        wire.await.unwrap();
    }
    for mode in ["named", "caller"] {
        for boundary in 0..2 {
            for late_error in [false, true] {
                let mut frames = vec![
                    profile("111", "pacmtoken: verified-1\r\n"),
                    reply(resource(), None),
                ];
                frames.truncate(boundary + 1);
                if late_error {
                    frames[boundary] = "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into();
                }
                let (mut p, seen, release, wire) = gated(frames).await;
                let (store, _, alias) = setup(&mut p, mode);
                let p = Arc::new(p);
                let operation = p.clone();
                let task = tokio::spawn(async move {
                    let mut r = VideoDetailRequest::new(VideoResourceKind::Mv);
                    r.account = Some(alias.into());
                    operation.video_stats("7", &r).await
                });
                tokio::time::timeout(Duration::from_secs(5), seen)
                    .await
                    .unwrap()
                    .unwrap();
                let next = MiguCredential::verified("333".into(), "next-login".into()).unwrap();
                if mode == "caller" {
                    *p.caller_credential.as_ref().unwrap().lock().unwrap() = next;
                } else {
                    store.put(&stored(alias, &next)).unwrap();
                }
                release.send(()).unwrap();
                assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
                assert!(p.take_response_credential().unwrap().is_none());
                wire.await.unwrap();
            }
        }
    }
}
#[tokio::test]
async fn mv_playback_batch_binds_one_login_deduplicates_and_preserves_order() {
    for mode in ["anonymous", "named", "caller"] {
        let mut source8 = resource();
        source8["resource"][0]["contentId"] = json!("8");
        let mut grant8 = grant();
        grant8["playUrl"] = json!(
            grant8["playUrl"]
                .as_str()
                .unwrap()
                .replace("resourceId=7", "resourceId=8")
        );
        let responses = if mode == "anonymous" {
            vec![
                reply(resource(), None),
                reply(grant(), None),
                manifest(),
                reply(source8, None),
                reply(grant8, None),
                manifest(),
            ]
        } else {
            let mut v = frames();
            v.extend([
                reply(source8, None),
                reply(grant8, Some("candidate-3")),
                profile("111", "pacmtoken: verified-3\r\n"),
                manifest(),
            ]);
            v
        };
        let count = responses.len();
        let (mut p, wire) = server(responses).await;
        let (store, original, alias) = setup(&mut p, mode);
        let ids = ["7".into(), "8".into(), "7".into()];
        let streams = p
            .video_streams(&ids, &request((mode != "anonymous").then_some(alias)))
            .await
            .unwrap();
        assert_eq!(
            streams.iter().map(|s| s.video_ref.id()).collect::<Vec<_>>(),
            ["7", "8", "7"]
        );
        assert_eq!(streams[0], streams[2]);
        let wire = wire.await.unwrap();
        assert_eq!(wire.len(), count);
        if mode == "named" {
            assert_eq!(read(&store, alias).token(), "verified-3");
        } else {
            assert_eq!(read(&store, alias), original);
        }
        if mode == "caller" {
            assert_eq!(
                MiguCredential::parse_caller(&p.take_response_credential().unwrap().unwrap())
                    .unwrap()
                    .token(),
                "verified-3"
            );
        }
    }
}
#[tokio::test]
async fn mv_playback_batch_preflights_every_id_and_rejects_mid_batch_relogin() {
    let (p, wire) = server(vec![]).await;
    for ids in [vec![], vec!["7".into(), "08".into()], vec!["7".into(); 101]] {
        assert_eq!(
            p.video_streams(&ids, &request(None))
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(wire.await.unwrap().is_empty());
    for mode in ["named", "caller"] {
        let mut source8 = resource();
        source8["resource"][0]["contentId"] = json!("8");
        let mut replies = frames();
        replies.push(reply(source8, None));
        let (mut p, seen, release, wire) = gated(replies).await;
        let (store, _, alias) = setup(&mut p, mode);
        let p = Arc::new(p);
        let operation = p.clone();
        let task = tokio::spawn(async move {
            operation
                .video_streams(&["7".into(), "8".into()], &request(Some(alias)))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), seen)
            .await
            .unwrap()
            .unwrap();
        let next = MiguCredential::verified("111".into(), "verified-2".into()).unwrap();
        if mode == "caller" {
            *p.caller_credential.as_ref().unwrap().lock().unwrap() = next;
        } else {
            store.put(&stored(alias, &next)).unwrap();
        }
        release.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
        assert!(p.take_response_credential().unwrap().is_none());
        wire.await.unwrap();
    }
}
#[tokio::test]
async fn mv_playback_batch_checks_earlier_urls_against_later_rotated_secrets() {
    let mut source8 = resource();
    source8["resource"][0]["contentId"] = json!("8");
    let mut first = grant();
    first["playUrl"] = json!(
        first["playUrl"]
            .as_str()
            .unwrap()
            .replace("playSessionId=fixture", "playSessionId=candidate-3")
    );
    let mut second = grant();
    second["playUrl"] = json!(
        second["playUrl"]
            .as_str()
            .unwrap()
            .replace("resourceId=7", "resourceId=8")
    );
    let mut replies = frames();
    replies[2] = reply(first, Some("candidate-2"));
    replies.extend([
        reply(source8, None),
        reply(second, Some("candidate-3")),
        profile("111", "pacmtoken: verified-3\r\n"),
        manifest(),
    ]);
    let (mut p, wire) = server(replies).await;
    let (store, _, alias) = setup(&mut p, "named");
    let err = p
        .video_streams(&["7".into(), "8".into()], &request(Some(alias)))
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::UpstreamError);
    assert_eq!(read(&store, alias).token(), "verified-3");
    assert_eq!(wire.await.unwrap().len(), 9);
}
#[tokio::test]
async fn mv_playback_preflight_and_response_size_checks_prevent_invalid_network_paths() {
    let (p, wire) = server(vec![]).await;
    for resolution in [360, 480, 720, 2160] {
        let mut r = request(None);
        r.resolution = resolution;
        assert!(p.video_stream("7", &r).await.is_err());
    }
    assert_eq!(
        p.video_stream("7", &request(Some("missing")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert!(wire.await.unwrap().is_empty());
    let (p,wire)=server(vec![reply(resource(),None),reply(grant(),None),"HTTP/1.1 200 OK\r\nContent-Type: application/x-mpegurl\r\nContent-Length: 262145\r\nConnection: close\r\n\r\n".into()]).await;
    assert!(p.video_stream("7", &request(None)).await.is_err());
    assert_eq!(wire.await.unwrap().len(), 3);
}
#[tokio::test]
#[ignore = "official anonymous Migu PC authorization and HLS text only; no media segments, keys or account"]
async fn live_migu_mv_pc_playback_default_format_and_https_manifest_are_bound() {
    let p = MiguProvider::new(MiguConfig::default()).unwrap();
    let stream = p
        .video_stream("600906000000317007", &request(None))
        .await
        .unwrap();
    assert!(
        stream
            .url
            .unwrap()
            .starts_with("https://freevod.nf.migu.cn/")
    );
    assert_eq!(stream.extensions["format_type"], "PQ");
    assert_eq!(stream.extensions["authorization_scope"], "anonymous");
    assert!(stream.actual_resolution.is_none());
    assert!(stream.duration_ms.unwrap() > 0);
    assert!(stream.headers.is_empty());
}
