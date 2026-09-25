use super::super::{content::tests as audio, tests as data};
use super::*;
use crate::client::{catalog::tests::response, native::tests as fixture};
use serde_json::Value;

pub(crate) fn rights() -> Value {
    let mut body = data::rights();
    body["songs"][0]["audio"][0]["policy"] = json!("song");
    body["songs"][0]["audio"][0]["st"] = json!(103);
    body["songs"][0]["audio"][0]["price"] = json!(2);
    body
}
pub(crate) fn metadata() -> Value {
    json!({"code":200,"result":"ok","timestamp":1789552602,"songs":[{
        "id":67474,"duration":240,"start":90,"end":91,"format":"mp3","br":128,
        "url":"http://ga-sycdn.kuwo.cn/preview/song.mp3",
        "https":"https://ga-sycdn.kuwo.cn/preview/song.mp3",
        "car_url_https":"https://vehiclecdn.kuwo.cn/not-used.mp3?secret=not-exported"
    }]})
}
pub(crate) fn flow(content: bool) -> Vec<Vec<u8>> {
    let mut bodies = vec![
        data::flow().remove(0),
        data::rights_reply(&rights()),
        data::rights_reply(&metadata()),
    ];
    if content {
        bodies.push(audio::flow(&audio::samples()[0], false).0.remove(3));
    }
    bodies
}
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected&+%session")
        .caller()
        .unwrap()
}

#[tokio::test]
async fn native_trial_stream_content_and_availability_preserve_window_and_identity() {
    for quality in [Quality::Auto, Quality::Standard] {
        for operation in 0..3 {
            let mut f = fixture::setup(flow(operation == 1)).await;
            let request = StreamRequest {
                quality,
                ..data::request(None)
            };
            let exposed = match operation {
                0 => {
                    let media = f
                        .client
                        .native_stream(&credential(), &data::track(), &request)
                        .await
                        .unwrap();
                    assert_eq!(
                        media.trial,
                        Some(TrialWindow {
                            start_ms: 90_000,
                            end_ms: 91_000
                        })
                    );
                    assert_eq!(media.duration_ms, Some(1000));
                    assert_eq!(media.actual_quality, Quality::Standard);
                    assert_eq!(media.url, "https://ga-sycdn.kuwo.cn/preview/song.mp3");
                    serde_json::to_string(&media).unwrap()
                }
                1 => {
                    let media = f
                        .client
                        .native_audio_content(&credential(), &data::track(), &request)
                        .await
                        .unwrap();
                    assert_eq!(
                        media.trial,
                        Some(TrialWindow {
                            start_ms: 90_000,
                            end_ms: 91_000
                        })
                    );
                    assert_eq!(media.filename, "kuwo-67474-preview.mp3");
                    assert_eq!(
                        media.bytes.len(),
                        audio::samples()[0]["bytes"].as_u64().unwrap() as usize
                    );
                    format!("{media:?}")
                }
                _ => {
                    let media = f
                        .client
                        .native_track_availability(
                            &credential(),
                            "67474",
                            &TrackAvailabilityRequest::default(),
                        )
                        .await
                        .unwrap();
                    assert!(!media.playable);
                    assert_eq!(media.actual_bitrate, None);
                    assert_eq!(media.extensions["preview_available"], true);
                    assert_eq!(media.extensions["full_track"], false);
                    assert_eq!(media.extensions["preview_start_ms"], 90000);
                    assert_eq!(media.extensions["preview_duration_ms"], 1000);
                    serde_json::to_string(&media).unwrap()
                }
            };
            for secret in [
                "selected&+%session",
                "vehiclecdn",
                "not-exported",
                "11111111111111112222222222222222",
            ] {
                assert!(!exposed.contains(secret));
            }
            let calls = fixture::requests(&mut f, if operation == 1 { 4 } else { 3 }).await;
            assert!(calls[2].starts_with("GET /audi.tion?"));
            let query = data::query(&calls[2]);
            assert_eq!(query["loginUid"], "42");
            assert_eq!(query["loginSid"], "selected&+%session");
            assert_eq!(query["ids"], "67474");
            assert_eq!(query["op"], "query");
            assert_eq!(query["appuid"], "1234567890");
            assert!(!query.contains_key("token"));
            if operation == 1 {
                assert!(!calls[3].contains("selected"));
                assert!(!calls[3].to_ascii_lowercase().contains("cookie:"));
            }
        }
    }
}

#[tokio::test]
async fn native_trial_never_replaces_download_quality_or_restricted_rights() {
    let mut restricted = vec![rights(); 10];
    restricted[0]["songs"][0]["payInfo"]["cannotOnlinePlay"] = json!(1);
    restricted[1]["songs"][0]["payInfo"]
        .as_object_mut()
        .unwrap()
        .remove("cannotOnlinePlay");
    restricted[2]["songs"][0]["audio"][0]["avaliable"] = json!(0);
    restricted[3]["songs"][0]["audio"][0]["st"] = json!(107);
    restricted[4]["songs"][0]["audio"][0]["st"] = json!(201);
    restricted[5]["songs"][0]["audio"][0]["st"] = json!(502);
    restricted[6]["songs"][0]["audio"][0]["st"] = json!(1000);
    restricted[7]["songs"][0]["audio"][0]["st"] = json!(0);
    restricted[8]["songs"][0]["audio"][0]["fmt"] = json!("MP3H");
    restricted[9]["songs"][0]["audio"][0]["policy"] = json!("vip");
    // st=0 with a purchased song policy authorizes full media; use the free
    // policy with a nonzero price to exercise an inconclusive permission.
    restricted[7]["songs"][0]["audio"][0]["policy"] = json!("");
    for body in restricted {
        let mut f = fixture::setup(vec![data::flow().remove(0), data::rights_reply(&body)]).await;
        let e = f
            .client
            .native_stream(&credential(), &data::track(), &data::request(None))
            .await
            .unwrap_err();
        assert_eq!(e.code, ErrorCode::PermissionDenied);
        fixture::requests(&mut f, 2).await;
    }
    for quality in [
        Quality::Low,
        Quality::High,
        Quality::Lossless,
        Quality::Hires,
    ] {
        let mut f = fixture::setup(flow(false)[..2].to_vec()).await;
        let request = StreamRequest {
            quality,
            ..data::request(None)
        };
        assert_eq!(
            f.client
                .native_stream(&credential(), &data::track(), &request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        fixture::requests(&mut f, 2).await;
    }
    for content in [false, true] {
        let mut f = fixture::setup(flow(false)[..2].to_vec()).await;
        if content {
            assert_eq!(
                f.client
                    .native_download_content(&credential(), &data::track(), &data::request(None))
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::PermissionDenied
            );
        } else {
            assert!(
                !f.client
                    .native_download(&credential(), &data::track(), &data::request(None))
                    .await
                    .unwrap()
                    .available
            );
        }
        let calls = fixture::requests(&mut f, 2).await;
        assert_eq!(data::query(&calls[1])["action"], "download");
    }
}

#[test]
fn native_trial_metadata_rejects_identity_ranges_unsafe_locations_and_partial_responses() {
    let input = fixture::credential_fixture("42", "selected-session")
        .input()
        .unwrap();
    for (key, value) in [
        ("id", json!(67475)),
        ("duration", json!(0)),
        ("duration", json!(86401)),
        ("duration", json!(90)),
        ("start", json!(91)),
        ("end", json!(90)),
        ("end", json!(121)),
        ("end", json!(-1)),
        ("start", json!("090")),
        ("br", json!(320)),
        ("format", json!("aac")),
        ("ekey", json!("encrypted")),
        ("https", json!("http://ga-sycdn.kuwo.cn/file.mp3")),
        ("https", json!("https://example.com/song.mp3")),
        (
            "https",
            json!("https://ga-sycdn.kuwo.cn/selected-session.mp3"),
        ),
        ("https", json!("https://ga-sycdn.kuwo.cn/song.mp3?sid=x")),
        ("https", json!("https://ga-sycdn.kuwo.cn/song.mp3#x")),
        ("https", json!("https://ga-sycdn.kuwo.cn/song.flac")),
        ("url", json!("http://example.com/song.mp3")),
    ] {
        let mut body = metadata();
        body["songs"][0][key] = value;
        assert!(
            parse(&serde_json::to_vec(&body).unwrap(), &input, "67474").is_err(),
            "{key}"
        );
    }
    for key in ["id", "duration", "start", "end", "format", "br", "https"] {
        let mut body = metadata();
        body["songs"][0].as_object_mut().unwrap().remove(key);
        assert!(
            parse(&serde_json::to_vec(&body).unwrap(), &input, "67474").is_err(),
            "{key}"
        );
    }
    let mut body = metadata();
    body["songs"]
        .as_array_mut()
        .unwrap()
        .push(metadata()["songs"][0].clone());
    assert!(parse(&serde_json::to_vec(&body).unwrap(), &input, "67474").is_err());
    for body in [
        json!({"code":200,"result":"ok"}),
        json!({"code":200,"result":"fail","songs":[]}),
        json!({"code":999,"result":"ok","songs":[]}),
    ] {
        assert!(parse(&serde_json::to_vec(&body).unwrap(), &input, "67474").is_err());
    }
    for body in [
        json!({"code":200,"result":"ok","songs":[]}),
        json!({"code":407}),
    ] {
        assert!(matches!(
            parse(&serde_json::to_vec(&body).unwrap(), &input, "67474").unwrap(),
            Outcome::Denied { .. }
        ));
    }
}

#[tokio::test]
async fn native_trial_content_checks_frame_duration_and_refuses_missing_authorization() {
    for long_body in [false, true] {
        let mut bodies = flow(true);
        if long_body {
            let mut media = metadata();
            media["songs"][0]["end"] = json!(119);
            bodies[2] = data::rights_reply(&media); // 1 second bytes cannot satisfy a 29 second preview.
        } else {
            bodies[2] = data::rights_reply(&json!({"code":200,"result":"ok","songs":[]}));
            bodies.truncate(3);
        }
        let mut f = fixture::setup(bodies).await;
        let error = f
            .client
            .native_audio_content(&credential(), &data::track(), &data::request(None))
            .await
            .unwrap_err();
        assert_eq!(
            error.code,
            if long_body {
                ErrorCode::UpstreamError
            } else {
                ErrorCode::PermissionDenied
            }
        );
        fixture::requests(&mut f, if long_body { 4 } else { 3 }).await;
    }
    // Receiving a bounded HTTP body is not enough: a wrong container is rejected.
    let mut bodies = flow(true);
    bodies[3] = response(200, "audio/mpeg", "", b"not-an-mp3");
    let mut f = fixture::setup(bodies).await;
    assert_eq!(
        f.client
            .native_audio_content(&credential(), &data::track(), &data::request(None))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    fixture::requests(&mut f, 4).await;
}

#[tokio::test]
async fn native_trial_content_rejects_complete_audio_longer_than_authorized_clip() {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let sample = &audio::samples()[0];
    let bytes = STANDARD
        .decode(sample["plain_base64"].as_str().unwrap())
        .unwrap();
    let frames = if bytes.starts_with(b"ID3") {
        let size = bytes[6..10]
            .iter()
            .fold(0_usize, |n, b| (n << 7) | usize::from(*b));
        &bytes[10 + size..]
    } else {
        &bytes[..]
    };
    let mut bodies = flow(true);
    bodies[3] = response(200, "audio/mpeg", "", &frames.repeat(5));
    let mut f = fixture::setup(bodies).await;
    assert_eq!(
        f.client
            .native_audio_content(&credential(), &data::track(), &data::request(None))
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    fixture::requests(&mut f, 4).await;
}
