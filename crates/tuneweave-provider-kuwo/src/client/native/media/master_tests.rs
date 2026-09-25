use super::*;
use super::{content::tests as audio, tests as data};
use crate::client::{catalog::tests::json_response, native::tests as fixture};
use serde_json::Value;

fn sample() -> Value {
    audio::samples()
        .into_iter()
        .find(|s| s["quality"] == "master")
        .unwrap()
}
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}
fn metadata(response: &[u8]) -> Value {
    let end = response.windows(4).position(|s| s == b"\r\n\r\n").unwrap() + 4;
    serde_json::from_slice(&response[end..]).unwrap()
}

#[tokio::test]
async fn native_master_selects_exact_authorized_tier_for_both_content_actions() {
    for action in [Action::Play, Action::Download] {
        for automatic in [false, true] {
            for allowed in [false, true] {
                let (mut responses, mut request) = audio::flow(&sample(), true);
                request.quality = if automatic {
                    Quality::Auto
                } else {
                    Quality::Master
                };
                let mut rights = metadata(&responses[1]);
                rights["songs"][0]["audio"][0]["policy"] = json!("vip");
                rights["songs"][0]["audio"][0]["st"] = json!(if allowed { 0 } else { 502 });
                rights["songs"][0]["audio"][0]["cost"] = json!(if allowed { 0 } else { 5 });
                rights["songs"][0]["audio"].as_array_mut().unwrap().push(json!({
                    "quality":"HR","br":4000,"fmt":"HIRFLAC","policy":"vip","st":0,"cost":0,"avaliable":1
                }));
                responses[1] = data::rights_reply(&rights);
                if !allowed && automatic {
                    let mut media = metadata(&responses[2]);
                    media["data"]["quality"] = json!("HR");
                    media["data"]["bitrate"] = json!(4000);
                    media["data"]["format"] = json!("flac");
                    media["data"]["surl"] = json!("https://er-sycdn.kuwo.cn/hires/file.flac");
                    responses[2] = json_response(&media);
                }
                if !allowed && !automatic {
                    responses.truncate(2);
                }
                let mut f = fixture::setup(responses).await;
                let result = match action {
                    Action::Play => {
                        f.client
                            .native_audio_content(&credential(), &data::track(), &request)
                            .await
                    }
                    Action::Download => {
                        f.client
                            .native_download_content(&credential(), &data::track(), &request)
                            .await
                    }
                };
                if allowed || automatic {
                    let delivered = result.unwrap();
                    assert_eq!(delivered.content_type, "audio/flac");
                    assert_eq!(delivered.filename, "kuwo-67474.flac");
                    assert!(delivered.trial.is_none());
                    assert_eq!(
                        delivered.bytes.len(),
                        sample()["bytes"].as_u64().unwrap() as usize
                    );
                    let calls = fixture::requests(&mut f, 4).await;
                    assert_eq!(data::query(&calls[1])["quality"], "ZPLY");
                    assert_eq!(data::query(&calls[1])["action"], action.rights());
                    let query = data::query(&calls[2]);
                    assert_eq!(
                        query["br"],
                        if allowed { "20900kmflac" } else { "4000kflac" }
                    );
                    assert_eq!(query["format"], if allowed { "mp3|aac" } else { "flac" });
                    assert_eq!(query["mode"], action.mode());
                    assert_eq!(
                        query["token"],
                        if allowed {
                            "bbbbbbbbbbbbbbbbcccccccccccccccc"
                        } else {
                            "9999999999999999aaaaaaaaaaaaaaaa"
                        }
                    );
                    assert!(!calls[3].contains("selected-session"));
                } else {
                    assert_eq!(result.unwrap_err().code, ErrorCode::PermissionDenied);
                    fixture::requests(&mut f, 2).await;
                }
            }
        }
    }
}

#[tokio::test]
async fn native_master_rejects_other_tiers_formats_missing_keys_and_invented_bitrates() {
    let (baseline, request) = audio::flow(&sample(), true);
    for (pointer, value) in [
        ("/data/quality", json!("ZP")),
        ("/data/quality", json!("HR")),
        ("/data/bitrate", json!(20000)),
        ("/data/bitrate", json!(4000)),
        ("/data/format", json!("flac")),
        ("/data/format", json!("mgg")),
        ("/data/ekey", json!("")),
        ("/data/ekey", json!(null)),
        ("/data/surl", json!("https://er-sycdn.kuwo.cn/file.flac")),
        ("/data/surl", json!("https://other.invalid/file.mflac")),
    ] {
        let mut responses = baseline[..3].to_vec();
        let mut body = metadata(&responses[2]);
        *body.pointer_mut(pointer).unwrap() = value;
        responses[2] = json_response(&body);
        let mut f = fixture::setup(responses).await;
        assert_eq!(
            f.client
                .native_audio_content(&credential(), &data::track(), &request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError,
            "{pointer}"
        );
        fixture::requests(&mut f, 3).await;
    }
    for (pointer, value) in [
        ("/songs/0/audio/0/quality", json!("ZP")),
        ("/songs/0/audio/0/br", json!(20000)),
        ("/songs/0/audio/0/fmt", json!("MFLAC")),
        ("/songs/0/audio/0/avaliable", json!(0)),
    ] {
        let mut responses = baseline[..2].to_vec();
        let mut body = metadata(&responses[1]);
        *body.pointer_mut(pointer).unwrap() = value;
        responses[1] = data::rights_reply(&body);
        let mut f = fixture::setup(responses).await;
        assert_eq!(
            f.client
                .native_audio_content(&credential(), &data::track(), &request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        fixture::requests(&mut f, 2).await;
    }
    for bitrate in [20_900, 20_900_000, 128_000] {
        let f = fixture::setup(vec![]).await;
        let mut request = request.clone();
        request.bitrate = Some(bitrate);
        assert_eq!(
            f.client
                .native_audio_content(&credential(), &data::track(), &request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
}

#[tokio::test]
async fn native_master_cipher_urls_never_become_plain_streams_or_downloads() {
    for operation in 0..3 {
        let (mut replies, request) = audio::flow(&sample(), true);
        replies.truncate(3);
        let mut f = fixture::setup(replies).await;
        match operation {
            0 => assert_eq!(
                f.client
                    .native_stream(&credential(), &data::track(), &request)
                    .await
                    .unwrap_err()
                    .code,
                ErrorCode::CapabilityNotSupported
            ),
            1 => {
                let result = f
                    .client
                    .native_download(&credential(), &data::track(), &request)
                    .await
                    .unwrap();
                assert!(!result.available && result.url.is_none());
                assert_eq!(result.extensions["content_delivery"], "download_content");
                assert!(!serde_json::to_string(&result).unwrap().contains("mflac"));
            }
            _ => {
                let result = f
                    .client
                    .native_track_availability(
                        &credential(),
                        "67474",
                        &TrackAvailabilityRequest::default(),
                    )
                    .await
                    .unwrap();
                assert!(result.playable);
                assert_eq!(result.actual_bitrate, None);
                assert_eq!(result.extensions["content_delivery"], "audio_content");
                assert!(!serde_json::to_string(&result).unwrap().contains("mflac"));
            }
        }
        fixture::requests(&mut f, 3).await;
    }
    // Encrypting bytes does not prove that they match the declared master format.
    let (mut responses, request) = audio::flow(&sample(), true);
    responses[3] = audio::flow(&audio::samples()[0], true).0.remove(3);
    let mut f = fixture::setup(responses).await;
    assert_eq!(
        f.client
            .native_audio_content(&credential(), &data::track(), &request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::UpstreamError
    );
    fixture::requests(&mut f, 4).await;
}
