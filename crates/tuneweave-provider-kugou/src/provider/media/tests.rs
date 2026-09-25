use super::*;
use crate::KugouLoginClient;
use crate::provider::library::tests::store_account;
use crate::provider::session::tests::{
    Frame, credential, exchange, paused, profile, raw, read, reply, server,
};
use serde_json::Value;
use tuneweave_core::{Quality, ResourceRef};

mod availability;

const HASH: &str = "abcdef0123456789abcdef0123456789";
fn track() -> Track {
    let mut value = Track::new(
        ResourceRef::new(Platform::Kugou, "901").unwrap(),
        "Untrusted caller name",
    );
    value.extensions.insert(
        "qualities".into(),
        json!({"standard":{"hash":"ffffffffffffffffffffffffffffffff"}}),
    );
    value
}
fn request(account: Option<&str>) -> StreamRequest {
    StreamRequest {
        account: account.map(str::to_owned),
        ..Default::default()
    }
}
fn catalogue() -> Vec<Frame> {
    vec![reply(json!([{"__status":1,"base":{"album_audio_id":901,"audio_id":1901,"songname":"Trusted song","author_name":"Artist"}}])).into(),
        reply(json!([{"audio_id":1901,"audio_name":"Artist - Trusted song","hash":HASH,"filesize":1975000,"bitrate":128,"timelength":123456}])).into()]
}
fn tracker() -> Value {
    json!({"status":1,"hash":HASH,"url":["https://fs.kugou.com/media.mp3"],"fileSize":1975000,"bitRate":128000,"timeLength":123,"extName":"mp3"})
}
fn frames(uid: &str, body: Value) -> Vec<Frame> {
    let mut frames = vec![
        exchange(uid, "rotated-media-token").into(),
        profile(uid).into(),
    ];
    frames.extend(catalogue());
    frames.push(reply(json!({"auth":"private-user-authorization","userid":uid})).into());
    frames.push(reply(json!({"auth":"private-song-authorization","open_time":1700000000,"album_audio_id":901,"hash":HASH})).into());
    frames.push(raw(body).into());
    frames
}
fn caller(kind: KugouLoginClient) -> ProviderCredential {
    let mut value = credential("333", "caller-media-token").native().clone();
    value.session.client = kind;
    value.caller().unwrap()
}

#[tokio::test]
async fn account_media_uses_verified_catalogue_and_exact_server_alias_without_exporting_grants() {
    let mut f = server(frames("111", tracker())).await;
    f.provider.client.register_test_device();
    let store = store_account(&mut f.provider);
    let other = read(&store, "B");
    let mut wanted = request(Some("A"));
    wanted.quality = Quality::Master;
    let stream = f.provider.stream(&track(), &wanted).await.unwrap();
    assert_eq!(stream.actual_quality, Quality::Standard);
    assert_eq!(stream.requested_quality, Quality::Master);
    assert_eq!(stream.resolved_track, track().resource_ref);
    assert_eq!(stream.url, "https://fs.kugou.com/media.mp3");
    assert_eq!(
        read(&store, "A").native().session.token,
        "rotated-media-token"
    );
    assert_eq!(read(&store, "B"), other);
    assert!(f.provider.take_response_credential().unwrap().is_none());
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 7);
    for raw in &requests[2..4] {
        assert!(!raw.contains("rotated-media-token"));
        assert!(!raw.contains("original"));
    }
    for raw in &requests[4..] {
        assert!(raw.contains("userid=111"));
        assert!(raw.contains("token=rotated-media-token"));
        assert!(!raw.contains("ffffffffffffffffffffffffffffffff"));
    }
    let output = serde_json::to_string(&stream).unwrap();
    for secret in [
        "rotated-media-token",
        "private-user-authorization",
        "private-song-authorization",
    ] {
        assert!(!output.contains(secret));
    }
}

#[tokio::test]
async fn caller_media_returns_only_the_latest_login_credential_for_both_native_clients() {
    for kind in [KugouLoginClient::Standard, KugouLoginClient::Concept] {
        let mut f = server(frames("333", tracker())).await;
        f.provider.client.register_test_device();
        let store = store_account(&mut f.provider);
        let before = read(&store, "A");
        let provider = f.provider.caller_scope(&caller(kind)).unwrap();
        let download = provider.download(&track(), &request(None)).await.unwrap();
        assert!(download.available);
        assert_eq!(download.extensions["authorization_behavior"], "download");
        let updated = provider.take_response_credential().unwrap().unwrap();
        let updated = KugouCredential::parse_caller(&updated).unwrap();
        assert_eq!(updated.native().session.token, "rotated-media-token");
        assert_eq!(updated.native().session.client, kind);
        assert_eq!(read(&store, "A"), before);
        let requests = f.requests.await.unwrap();
        assert_eq!(requests.len(), 7);
        assert!(requests[6].contains("behavior=download"));
        assert!(requests[6].contains("userid=333"));
        assert!(
            !updated
                .native()
                .caller()
                .unwrap()
                .secret()
                .contains("private-song-authorization")
        );
    }
}

#[tokio::test]
async fn account_track_metadata_accepts_account_scope_without_claiming_playback_entitlements() {
    let mut frames = vec![
        exchange("111", "rotated-media-token").into(),
        profile("111").into(),
    ];
    frames.extend(catalogue());
    let mut f = server(frames).await;
    f.provider.client.register_test_device();
    store_account(&mut f.provider);
    let track = f.provider.track("901", Some("A")).await.unwrap();
    assert_eq!(track.name, "Trusted song");
    assert!(!track.extensions.contains_key("authorization_behavior"));
    assert_eq!(f.requests.await.unwrap().len(), 4);
}

#[tokio::test]
async fn auth_failure_suppresses_rotations_but_permission_and_transport_failures_preserve_them() {
    for (status, code, expected, update) in [
        (0, 20017, ErrorCode::AuthenticationRequired, false),
        (3, 0, ErrorCode::PermissionDenied, true),
        (0, 12345, ErrorCode::UpstreamError, true),
    ] {
        let f=server(frames("333",json!({"status":status,"errcode":code,"url":["https://fs.kugou.com/should-not-leak.mp3"]}))).await;
        f.provider.client.register_test_device();
        let provider = f
            .provider
            .caller_scope(&caller(KugouLoginClient::Standard))
            .unwrap();
        let mut failure = provider.stream(&track(), &request(None)).await.unwrap_err();
        assert_eq!(failure.code, expected);
        let returned = provider
            .take_response_credential()
            .unwrap()
            .or_else(|| failure.take_caller_credential_update());
        assert_eq!(returned.is_some(), update);
        assert!(!format!("{failure:?}").contains("should-not-leak"));
        assert_eq!(f.requests.await.unwrap().len(), 7);
    }
}

#[tokio::test]
async fn account_download_does_not_retry_as_play_or_anonymous_when_only_trial_is_authorized() {
    let mut body = tracker();
    body["hash_offset"] = json!({"start_ms":0,"end_ms":60000});
    body["timeLength"] = json!(60);
    let mut f = server(frames("111", body)).await;
    f.provider.client.register_test_device();
    store_account(&mut f.provider);
    assert_eq!(
        f.provider
            .download(&track(), &request(Some("A")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 7);
    assert!(requests[6].contains("behavior=download"));
    assert!(!requests[6].contains("IsFreePart"));
}

#[tokio::test]
async fn logout_or_relogin_during_each_authorization_boundary_prevents_late_media_delivery() {
    for boundary in 4..=6 {
        for relogin in [false, true] {
            let mut all = frames("111", tracker());
            all.truncate(boundary);
            let last = match boundary {
                4 => reply(json!({"auth":"private-user-authorization"})),
                5 => reply(json!({"auth":"private-song-authorization","open_time":1700000000})),
                _ => raw(tracker()),
            };
            let (frame, release) = paused(last);
            all.push(frame);
            let mut f = server(all).await;
            f.provider.client.register_test_device();
            let store = store_account(&mut f.provider);
            let provider = f.provider.clone();
            let running =
                tokio::spawn(async move { provider.stream(&track(), &request(Some("A"))).await });
            for _ in 0..=boundary {
                f.seen.recv().await.unwrap();
            }
            store.remove(Platform::Kugou, "A").unwrap();
            if relogin {
                store
                    .put(
                        &credential("111", "new-login-generation")
                            .stored("A")
                            .unwrap(),
                    )
                    .unwrap();
            }
            release.send(()).unwrap();
            assert_eq!(
                running.await.unwrap().unwrap_err().code,
                ErrorCode::Conflict
            );
            assert!(f.provider.take_response_credential().unwrap().is_none());
            if relogin {
                assert_eq!(
                    read(&store, "A").native().session.token,
                    "new-login-generation"
                );
            }
            assert_eq!(f.requests.await.unwrap().len(), boundary + 1);
        }
    }
}

#[tokio::test]
async fn invalid_account_media_selection_fails_before_network_and_cannot_use_another_alias() {
    let mut f = server(vec![]).await;
    store_account(&mut f.provider);
    assert_eq!(
        f.provider
            .stream(&track(), &request(Some("missing")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    let mut invalid = track();
    invalid.id = "902".into();
    assert_eq!(
        f.provider
            .stream(&invalid, &request(Some("A")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let mut invalid_request = request(Some("A"));
    invalid_request.bitrate = Some(0);
    assert_eq!(
        f.provider
            .stream(&track(), &invalid_request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    let scoped = f
        .provider
        .caller_scope(&caller(KugouLoginClient::Standard))
        .unwrap();
    assert_eq!(
        scoped
            .stream(&track(), &request(Some("A")))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(!f.provider.requires_download_authorization(None));
    assert!(
        f.provider.requires_download_authorization(Some("A"))
            && scoped.requires_download_authorization(None)
    );
    assert!(f.requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn cross_platform_matching_uses_public_candidates_then_the_selected_native_account_grant() {
    use tuneweave_core::{ArtistSummary, ProviderRegistry, ResolveRequest, StreamResolver};
    for caller_owned in [false, true] {
        for matches in [false, true] {
            let uid = if caller_owned { "333" } else { "111" };
            let mut all=vec![exchange(uid,"matching-token").into(),profile(uid).into(),
                reply(json!({"page":1,"pagesize":100,"total":1,"lists":[{"MixSongID":901,"SongName":if matches {"Trusted song"} else {"Entirely unrelated title"},
                    "SingerName":if matches {"Artist"} else {"Another artist"},"Duration":123,"FileHash":HASH}]})).into()];
            if matches {
                all.extend(frames(uid, tracker()));
            }
            let mut f = server(all).await;
            f.provider.client.register_test_device();
            let store = store_account(&mut f.provider);
            let mut registry = ProviderRegistry::new();
            let provider = if caller_owned {
                f.provider
                    .caller_scope(&caller(KugouLoginClient::Standard))
                    .unwrap()
            } else {
                f.provider.clone()
            };
            registry.register(provider.clone()).unwrap();
            let resolver = StreamResolver::new(registry, vec![]);
            let mut origin = Track::new(
                ResourceRef::new(Platform::Netease, "10").unwrap(),
                "Trusted song",
            );
            origin.artists = vec![ArtistSummary {
                name: "Artist".into(),
                resource_ref: None,
            }];
            origin.duration_ms = Some(123456);
            let request = ResolveRequest {
                playback_platforms: vec![Platform::Kugou],
                fallback: false,
                accounts: [(
                    Platform::Kugou,
                    if caller_owned {
                        "default".into()
                    } else {
                        "A".into()
                    },
                )]
                .into(),
                ..Default::default()
            };
            let result = resolver.resolve(&origin, &request).await;
            if matches {
                assert_eq!(result.unwrap().resolved_track, track().resource_ref);
            } else {
                assert_eq!(result.unwrap_err().code, ErrorCode::MatchRejected);
            }
            let requests = f.requests.await.unwrap();
            assert_eq!(requests.len(), if matches { 10 } else { 3 });
            assert!(requests[2].starts_with("GET /song_search_v2?"));
            assert!(requests[2].contains("userid=-1"));
            assert!(
                !requests[2].contains("matching-token")
                    && !requests[2].contains("rotated-media-token")
            );
            if caller_owned {
                assert!(provider.take_response_credential().unwrap().is_some());
            } else {
                assert_eq!(
                    read(&store, "A").native().session.token,
                    if matches {
                        "rotated-media-token"
                    } else {
                        "matching-token"
                    }
                );
            }
        }
    }
}

#[tokio::test]
async fn caller_scope_invalidation_while_tracker_is_pending_suppresses_the_late_stream_and_rotation()
 {
    let mut all = frames("333", tracker());
    all.pop();
    let (frame, release) = paused(raw(tracker()));
    all.push(frame);
    let mut f = server(all).await;
    f.provider.client.register_test_device();
    let provider = f
        .provider
        .caller_scope(&caller(KugouLoginClient::Standard))
        .unwrap();
    let running_provider = provider.clone();
    let running =
        tokio::spawn(async move { running_provider.stream(&track(), &request(None)).await });
    for _ in 0..7 {
        f.seen.recv().await.unwrap();
    }
    *provider.caller_credential.as_ref().unwrap().lock().unwrap() = None;
    release.send(()).unwrap();
    assert_eq!(
        running.await.unwrap().unwrap_err().code,
        ErrorCode::Conflict
    );
    assert!(provider.take_response_credential().unwrap().is_none());
    assert_eq!(f.requests.await.unwrap().len(), 7);
}
