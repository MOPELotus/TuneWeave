use super::*;

mod lyrics;
use crate::provider::session::tests::{Fixture, Frame};
use tuneweave_core::{Quality, ResourceRef, StreamVariant, TrackAvailabilityRequest};

const HASH: &str = "abcdef0123456789abcdef0123456789";
const TOKEN: &str = "rotated-web-media-token";

fn track() -> Track {
    let mut value = Track::new(
        ResourceRef::new(Platform::Kugou, "901").unwrap(),
        "Caller name",
    );
    value
        .extensions
        .insert("encode_album_audio_id".into(), json!("caller-forged-id"));
    value
}
fn request(caller: bool) -> StreamRequest {
    StreamRequest {
        account: (!caller).then(|| "A".into()),
        quality: Quality::Master,
        ..Default::default()
    }
}
fn song() -> Value {
    json!({"hash":HASH,"album_audio_id":901,"play_url":"https://webfs.yun.kugou.com/music.mp3",
        "play_backup_url":"https://webfs.cloud.kugou.com/music.mp3","timelength":123456,
        "filesize":1975000,"bitrate":128,"is_free_part":0,"has_privilege":true})
}
fn media(value: Value) -> String {
    raw(json!({"status":1,"err_code":0,"data":value}))
}
fn catalogue() -> Vec<Frame> {
    vec![reply(json!([{"__status":1,"base":{"album_audio_id":901,"audio_id":1901,"songname":"Trusted song","author_name":"Artist"}}])).into(),
        reply(json!([{"audio_id":1901,"audio_name":"Artist - Trusted song","hash":HASH,"filesize":1975000,"bitrate":128,"timelength":123456}])).into()]
}
fn frames(tail: Vec<Frame>) -> Vec<Frame> {
    let mut result = vec![web_reply("111", TOKEN).into()];
    result.extend(catalogue());
    result.extend(tail);
    result
}
async fn setup(frames: Vec<Frame>, caller: bool) -> (Fixture, KugouProvider, Arc<Store>) {
    let mut f = server(frames).await;
    f.provider.client.register_test_device();
    let store = Arc::new(Store::default());
    store
        .put(&web("111", "original-web-media").stored("A").unwrap())
        .unwrap();
    store
        .put(
            &credential("222", "unrelated-native-token")
                .stored("B")
                .unwrap(),
        )
        .unwrap();
    f.provider.credential_store = Some(store.clone());
    let provider = if caller {
        f.provider
            .caller_scope(&web("111", "caller-web-media").caller().unwrap())
            .unwrap()
    } else {
        f.provider.clone()
    };
    (f, provider, store)
}
fn query(request: &str) -> BTreeMap<String, String> {
    let target = request
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
    url.query_pairs().into_owned().collect()
}
fn check_rotation(provider: &KugouProvider, store: &Store, caller: bool) {
    let value = if caller {
        KugouCredential::parse_caller(&provider.take_response_credential().unwrap().unwrap())
            .unwrap()
    } else {
        read(store, "A")
    };
    let KugouCredential::Web(value) = value else {
        panic!("wrong credential kind");
    };
    assert_eq!(value.session.media_token().unwrap(), TOKEN);
    assert_eq!(
        read(store, "B").native().session.token,
        "unrelated-native-token"
    );
    if caller {
        let KugouCredential::Web(value) = read(store, "A") else {
            panic!();
        };
        assert_eq!(value.session.media_token().unwrap(), "original-web-media");
    }
}

#[tokio::test]
async fn selected_web_account_signs_both_resource_steps_without_cookie_or_native_tokens() {
    for caller in [false, true] {
        for encoded in [false, true] {
            let mut first = song();
            let mut tail = vec![];
            if encoded {
                first["encode_album_audio_id"] = json!("verified_901");
            }
            tail.push(media(first).into());
            if encoded {
                tail.push(
                    response_with_cookies(
                        json!({"status":1,"err_code":0,"data":song()}),
                        &[cookie("999", "ignored-media-cookie")],
                    )
                    .into(),
                );
            }
            let (f, provider, store) = setup(frames(tail), caller).await;
            let stream = provider.stream(&track(), &request(caller)).await.unwrap();
            assert_eq!(stream.requested_quality, Quality::Master);
            assert_eq!(stream.actual_quality, Quality::Standard);
            assert_eq!(stream.bitrate, Some(128000));
            assert_eq!(stream.duration_ms, Some(123456));
            assert_eq!(stream.format, None);
            assert_eq!(stream.expires_at, None);
            assert_eq!(stream.trial, None);
            assert_eq!(stream.backup_urls.len(), 1);
            assert!(stream.headers.is_empty());
            let out = serde_json::to_string(&stream).unwrap();
            for secret in [
                TOKEN,
                "original-web-media",
                "caller-forged-id",
                "ignored-media-cookie",
            ] {
                assert!(!out.contains(secret));
            }
            check_rotation(&provider, &store, caller);
            let requests = f.requests.await.unwrap();
            assert_eq!(requests.len(), if encoded { 5 } else { 4 });
            let login = params(&requests[0]);
            for entry in &requests[1..3] {
                assert!(!entry.contains(TOKEN));
            }
            for (index, entry) in requests[3..].iter().enumerate() {
                assert!(entry.starts_with("GET /play/songinfo?"));
                assert!(!entry.to_lowercase().contains("\r\ncookie:"));
                assert!(!entry.to_lowercase().contains("x-router:"));
                assert!(entry.contains("referer: https://www.kugou.com/song/"));
                let mut q = query(entry);
                let signature = q.remove("signature").unwrap();
                assert_eq!(
                    signature,
                    crate::signing::web_signature(
                        &q.iter().map(|(k, v)| (k.as_str(), v.clone())).collect(),
                        b""
                    )
                );
                for (key, expected) in [
                    ("userid", "111"),
                    ("token", TOKEN),
                    ("appid", "1014"),
                    ("srcappid", "2919"),
                    ("clientver", "20000"),
                    ("platid", "4"),
                ] {
                    assert_eq!(q[key], expected);
                }
                assert_eq!(q["mid"], login["mid"]);
                assert_eq!(q["uuid"], login["uuid"]);
                assert!(q["clienttime"].parse::<u64>().unwrap() > 1_000_000_000_000);
                assert!(!q.contains_key("quality"));
                if index == 0 {
                    assert!(q["hash"].eq_ignore_ascii_case(HASH));
                    assert_eq!(q["album_audio_id"], "901");
                    assert!(!q.contains_key("encode_album_audio_id"));
                } else {
                    assert_eq!(q["encode_album_audio_id"], "verified_901");
                    assert!(!q.contains_key("hash"));
                    assert!(!q.contains_key("album_audio_id"));
                }
            }
        }
    }
}

#[tokio::test]
async fn web_media_identity_drift_and_forged_encoded_ids_fail_before_delivery() {
    for (key, bad) in [
        ("hash", json!("ffffffffffffffffffffffffffffffff")),
        ("album_audio_id", json!(902)),
        ("userid", json!(222)),
        ("encode_album_audio_id", json!("bad&identity=2")),
    ] {
        for second in [false, true] {
            let mut bad_song = song();
            bad_song[key] = bad.clone();
            let mut tail = vec![];
            if second {
                let mut first = song();
                first["encode_album_audio_id"] = json!("trusted901");
                tail.push(media(first).into());
            }
            tail.push(media(bad_song).into());
            let (f, provider, store) = setup(frames(tail), true).await;
            assert_eq!(
                provider
                    .stream(&track(), &request(true))
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::UpstreamError
            );
            check_rotation(&provider, &store, true);
            assert_eq!(f.requests.await.unwrap().len(), if second { 5 } else { 4 });
        }
    }
    let mut first = song();
    first["encode_album_audio_id"] = json!("one901");
    let mut second = song();
    second["encode_album_audio_id"] = json!("another901");
    let (f, provider, _) = setup(
        frames(vec![media(first).into(), media(second).into()]),
        true,
    )
    .await;
    assert_eq!(
        provider
            .stream(&track(), &request(true))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    assert_eq!(f.requests.await.unwrap().len(), 5);
}

#[tokio::test]
async fn web_trials_and_explicit_denials_are_not_reported_as_full_availability() {
    for caller in [false, true] {
        for state in ["full", "trial", "denied", "client"] {
            let mut body = song();
            if state == "trial" {
                body["is_free_part"] = json!(1);
                body["trans_param"] = json!({"hash_offset":{"start_ms":10000,"end_ms":40000}});
            }
            if state == "denied" {
                body["has_privilege"] = json!(false);
            }
            let wire = if state == "client" {
                raw(json!({"status":0,"err_code":30022,"data":null}))
            } else {
                media(body)
            };
            let (f, provider, store) = setup(frames(vec![wire.into()]), caller).await;
            let result = provider
                .track_availability(
                    "901",
                    &TrackAvailabilityRequest {
                        account: request(caller).account,
                        ..Default::default()
                    },
                )
                .await
                .unwrap();
            assert_eq!(result.playable, state == "full");
            assert_eq!(result.extensions.contains_key("trial"), state == "trial");
            assert!(!serde_json::to_string(&result).unwrap().contains("webfs"));
            check_rotation(&provider, &store, caller);
            assert_eq!(f.requests.await.unwrap().len(), 4);
        }
    }
    // A challenge is an unresolved request even if a status number also has a
    // denial meaning in the separate native protocol.
    for status in [0, 3] {
        let (f, provider, _) = setup(
            frames(vec![raw(json!({"status":status,"err_code":30020})).into()]),
            true,
        )
        .await;
        let error = provider
            .track_availability("901", &TrackAvailabilityRequest::default())
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::PermissionDenied);
        assert_eq!(error.details["additional_verification_required"], true);
        assert_eq!(f.requests.await.unwrap().len(), 4);
    }
}

#[tokio::test]
async fn web_media_rejects_ambiguous_trials_and_unproven_actual_metadata() {
    let mut cases = Vec::new();
    for (key, value) in [
        ("is_free_part", json!(1)),
        ("is_free_part", json!(2)),
        ("is_free_part", json!(null)),
        ("bitrate", json!(128000)),
        ("bitrate", json!(0)),
        ("bitrate", json!(true)),
        ("filesize", json!(0)),
        ("timelength", json!(123)),
        ("extname", json!("kgm")),
    ] {
        let mut data = song();
        data[key] = value;
        cases.push(data);
    }
    for (flag, start, end, store) in [
        (0, 10000, 30000, "audio"),
        (1, 40000, 30000, "audio"),
        (1, 0, 140000, "audio"),
        (1, 0, 30000, "album"),
    ] {
        let mut data = song();
        data["is_free_part"] = json!(flag);
        data["trans_param"] = json!({"hash_offset":{"start_ms":start,"end_ms":end}});
        data["store_type"] = json!(store);
        cases.push(data);
    }
    for data in cases {
        let (f, provider, store) = setup(frames(vec![media(data).into()]), true).await;
        assert!(provider.stream(&track(), &request(true)).await.is_err());
        check_rotation(&provider, &store, true);
        assert_eq!(f.requests.await.unwrap().len(), 4);
    }
}

#[tokio::test]
async fn web_media_never_exports_token_echoes_untrusted_urls_or_unknown_redirects() {
    let encoded = TOKEN
        .bytes()
        .map(|b| format!("%{b:02X}"))
        .collect::<String>();
    for url in [
        format!("https://webfs.yun.kugou.com/{TOKEN}.mp3"),
        format!("https://webfs.yun.kugou.com/a.mp3?t={encoded}"),
        "https://evil.example/audio.mp3".into(),
        "http://127.0.0.1/audio.mp3".into(),
        "https://user:pass@webfs.yun.kugou.com/a.mp3".into(),
    ] {
        for backup in [false, true] {
            let mut body = song();
            body[if backup {
                "play_backup_url"
            } else {
                "play_url"
            }] = json!(url);
            let (f, provider, _) = setup(frames(vec![media(body).into()]), true).await;
            let error = provider.stream(&track(), &request(true)).await.unwrap_err();
            assert!(!format!("{error:?}").contains(&url));
            assert_eq!(f.requests.await.unwrap().len(), 4);
        }
    }
}

#[tokio::test]
async fn web_media_http_and_challenge_errors_preserve_only_valid_login_rotation() {
    let mut cases = vec![];
    for (status, code) in [
        ("401 Unauthorized", ErrorCode::AuthenticationRequired),
        ("429 Too Many Requests", ErrorCode::RateLimited),
        ("302 Found", ErrorCode::UpstreamError),
    ] {
        cases.push((media(song()).replacen("200 OK", status, 1), code));
    }
    cases.push((
        media(song()).replace("application/json", "text/html"),
        ErrorCode::UpstreamError,
    ));
    cases.push((raw(json!({"status":0,"err_code":30020,"data":{"SSA-CODE":"private-event","SSA-HMID":"private-mid"}})),ErrorCode::PermissionDenied));
    cases.push((
        raw(json!({"status":1,"err_code":0,"error_code":30022,"data":song()})),
        ErrorCode::UpstreamError,
    ));
    cases.push((
        media(song()).replacen(
            "Content-Length:",
            "Content-Length: 1048577\r\nIgnored-Length:",
            1,
        ),
        ErrorCode::UpstreamError,
    ));
    for (wire, expected) in cases {
        for second in [false, true] {
            let mut tail = vec![];
            if second {
                let mut first = song();
                first["encode_album_audio_id"] = json!("verified901");
                tail.push(media(first).into());
            }
            tail.push(wire.clone().into());
            let (f, provider, store) = setup(frames(tail), true).await;
            let mut error = provider.stream(&track(), &request(true)).await.unwrap_err();
            assert_eq!(error.code, expected);
            assert!(!format!("{error:?}").contains("private-event"));
            let update = provider
                .take_response_credential()
                .unwrap()
                .or_else(|| error.take_caller_credential_update());
            assert_eq!(
                update.is_some(),
                expected != ErrorCode::AuthenticationRequired
            );
            assert_eq!(read(&store, "B").native().session.user_id, "222");
            assert_eq!(f.requests.await.unwrap().len(), if second { 5 } else { 4 });
        }
    }
}

#[tokio::test]
async fn web_media_late_catalogue_or_playback_cannot_survive_logout_or_relogin() {
    for caller in [false, true] {
        for boundary in [2, 3, 4] {
            let mut first = song();
            first["encode_album_audio_id"] = json!("verified901");
            let mut all = frames(vec![media(first).into(), media(song()).into()]);
            let payload = if boundary == 2 {
                reply(
                    json!([{"audio_id":1901,"audio_name":"Artist - Trusted song","hash":HASH,"filesize":1975000,"bitrate":128,"timelength":123456}]),
                )
            } else {
                media(song())
            };
            let (frame, release) = paused(payload);
            all.truncate(boundary);
            all.push(frame);
            let (mut f, provider, store) = setup(all, caller).await;
            let active = provider.clone();
            let task = tokio::spawn(async move { active.stream(&track(), &request(caller)).await });
            for _ in 0..=boundary {
                f.seen.recv().await.unwrap();
            }
            let replacement = web("111", "new-login-token");
            if caller {
                *provider.caller_credential.as_ref().unwrap().lock().unwrap() = None;
            } else {
                store.put(&replacement.stored("A").unwrap()).unwrap();
            }
            release.send(()).unwrap();
            assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
            assert!(provider.take_response_credential().unwrap().is_none());
            if !caller {
                assert_eq!(read(&store, "A"), replacement);
            }
            assert_eq!(f.requests.await.unwrap().len(), boundary + 1);
        }
    }
}

#[tokio::test]
async fn web_playback_permission_cannot_be_reused_as_download_authorization() {
    for caller in [false, true] {
        let (f, provider, store) = setup(vec![web_reply("111", TOKEN).into()], caller).await;
        assert!(provider.requires_download_authorization(request(caller).account.as_deref()));
        assert_eq!(
            provider
                .download(&track(), &request(caller))
                .await
                .unwrap_err()
                .code,
            ErrorCode::CapabilityNotSupported
        );
        check_rotation(&provider, &store, caller);
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn web_media_input_errors_and_cookie_scope_fail_before_the_media_endpoint() {
    let (f, provider, _) = setup(vec![], true).await;
    let mut bad = request(true);
    bad.account = Some("A".into());
    assert_eq!(
        provider.stream(&track(), &bad).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    bad = request(true);
    bad.variant = StreamVariant::Modern;
    assert_eq!(
        provider.stream(&track(), &bad).await.unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    let mut bad_track = track();
    bad_track.id = "902".into();
    assert_eq!(
        provider
            .stream(&bad_track, &request(true))
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidRequest
    );
    assert!(f.requests.await.unwrap().is_empty());
    let mut wire = frames(vec![]);
    wire[0] = web_reply("111", TOKEN)
        .replace(
            "Domain=.kugou.com; Path=/;",
            "Domain=loginservice.kugou.com; Path=/;",
        )
        .into();
    let (f, provider, _) = setup(wire, true).await;
    assert_eq!(
        provider
            .stream(&track(), &request(true))
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    assert_eq!(f.requests.await.unwrap().len(), 3);
}

#[tokio::test]
async fn web_account_metadata_is_public_catalogue_with_verified_account_lifetime() {
    for caller in [false, true] {
        let (f, provider, store) = setup(frames(vec![]), caller).await;
        let result = provider
            .track("901", request(caller).account.as_deref())
            .await
            .unwrap();
        assert_eq!(result.name, "Trusted song");
        assert!(!result.extensions.contains_key("authorization_behavior"));
        check_rotation(&provider, &store, caller);
        assert_eq!(f.requests.await.unwrap().len(), 3);
    }
}

#[tokio::test]
async fn cross_platform_resolver_uses_web_account_only_after_a_strict_catalogue_match() {
    use tuneweave_core::{ArtistSummary, ProviderRegistry, ResolveRequest, StreamResolver};
    for caller in [false, true] {
        for matches in [false, true] {
            let mut all = vec![web_reply("111", "matching-web-token").into(),
                reply(json!({"page":1,"pagesize":100,"total":1,"lists":[{"MixSongID":901,
                    "SongName":if matches {"Trusted song"}else{"Unrelated title"},
                    "SingerName":if matches {"Artist"}else{"Another artist"},"Duration":123,"FileHash":HASH}]})).into()];
            if matches {
                all.extend(frames(vec![media(song()).into()]));
            }
            let (f, provider, store) = setup(all, caller).await;
            let mut registry = ProviderRegistry::new();
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
            let resolved = resolver
                .resolve(
                    &origin,
                    &ResolveRequest {
                        playback_platforms: vec![Platform::Kugou],
                        fallback: false,
                        accounts: [(
                            Platform::Kugou,
                            if caller { "default".into() } else { "A".into() },
                        )]
                        .into(),
                        ..Default::default()
                    },
                )
                .await;
            if matches {
                assert_eq!(resolved.unwrap().resolved_track, track().resource_ref);
                check_rotation(&provider, &store, caller);
            } else {
                assert_eq!(resolved.unwrap_err().code, ErrorCode::MatchRejected);
                assert_eq!(
                    provider.take_response_credential().unwrap().is_some(),
                    caller
                );
            }
            let requests = f.requests.await.unwrap();
            assert_eq!(requests.len(), if matches { 6 } else { 2 });
            assert!(requests[1].starts_with("GET /song_search_v2?"));
            assert!(requests[1].contains("userid=-1"));
            assert!(!requests[1].contains("matching-web-token"));
            assert!(!requests[1].contains(TOKEN));
        }
    }
}
