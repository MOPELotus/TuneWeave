use super::*;
use super::{content::tests as audio, tests as data};
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests as fixture,
};
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::Value;

pub(crate) fn sample() -> Value {
    // This independently encrypted, self-generated FLAC exercises delivery.
    // It is not a platform vinyl recording or a real-account acceptance sample.
    audio::samples()
        .into_iter()
        .find(|s| s["format"] == "flac" && s["quality"].is_null())
        .unwrap()
}

fn metadata(response: &[u8]) -> Value {
    let start = response.windows(4).position(|v| v == b"\r\n\r\n").unwrap() + 4;
    serde_json::from_slice(&response[start..]).unwrap()
}

pub(crate) fn flow(encrypted: bool) -> (Vec<Vec<u8>>, StreamRequest) {
    let (mut replies, mut request) = audio::flow(&sample(), encrypted);
    let mut rights = metadata(&replies[1]);
    rights["songs"][0]["audio"][0] = json!({
        "quality":"VINYL","br":23000,"fmt":"VINYL",
        "policy":"vip","st":0,"cost":0,"price":0,"avaliable":1
    });
    rights["songs"][0]["token"]["VINYL"] = json!("ddddddddddddddddeeeeeeeeeeeeeeee");
    replies[1] = data::rights_reply(&rights);
    let mut media = metadata(&replies[2]);
    media["data"]["quality"] = json!("VINYL");
    media["data"]["bitrate"] = json!(23000);
    media["data"]["format"] = json!("flac");
    media["data"]["surl"] = json!("https://er-sycdn.kuwo.cn/vinyl/file.flac");
    replies[2] = json_response(&media);
    request.quality = Quality::Vinyl;
    (replies, request)
}

fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}

#[tokio::test]
async fn native_vinyl_content_uses_its_own_rights_token_and_exact_flac_selector() {
    for encrypted in [false, true] {
        for action in [Action::Play, Action::Download] {
            let (replies, request) = flow(encrypted);
            let mut f = fixture::setup(replies).await;
            let delivered = match action {
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
            }
            .unwrap();
            assert_eq!(delivered.content_type, "audio/flac");
            assert_eq!(delivered.filename, "kuwo-67474.flac");
            assert!(delivered.trial.is_none());
            assert_eq!(
                delivered.bytes,
                BASE64
                    .decode(sample()["plain_base64"].as_str().unwrap())
                    .unwrap()
            );
            let calls = fixture::requests(&mut f, 4).await;
            let rights = data::query(&calls[1]);
            assert_eq!(rights["quality"], "VINYL");
            assert_eq!(rights["action"], action.rights());
            let media = data::query(&calls[2]);
            assert_eq!(media["br"], "23000kflac");
            assert_eq!(media["format"], "mp3|aac");
            assert_eq!(media["mode"], action.mode());
            assert_eq!(media["token"], "ddddddddddddddddeeeeeeeeeeeeeeee");
            assert_eq!(media["bc_token"], "");
            assert_eq!(media["loginSid"], "selected-session");
            assert!(!calls[3].contains("selected-session"));
            assert!(!calls[3].to_ascii_lowercase().contains("cookie:"));
        }
    }
}

#[tokio::test]
async fn native_vinyl_rights_never_fall_back_and_auto_does_not_select_a_vinyl_recording() {
    for action in [Action::Play, Action::Download] {
        for (pointer, value) in [
            ("/songs/0/audio/0/st", json!(502)),
            ("/songs/0/audio/0/avaliable", json!(0)),
            ("/songs/0/audio/0/quality", json!("BCMS")),
            ("/songs/0/audio/0/quality", json!("ZPLY")),
            ("/songs/0/audio/0/quality", json!("ZPGA201")),
            ("/songs/0/audio/0/br", json!(20000)),
            ("/songs/0/audio/0/br", json!(20900)),
            ("/songs/0/audio/0/fmt", json!("ALFLAC")),
        ] {
            let (mut replies, request) = flow(true);
            let mut rights = metadata(&replies[1]);
            *rights.pointer_mut(pointer).unwrap() = value;
            // Even explicitly authorized ordinary lossless cannot satisfy Vinyl.
            rights["songs"][0]["audio"]
                .as_array_mut()
                .unwrap()
                .push(json!({
                    "quality":"F","br":2000,"fmt":"ALFLAC",
                    "policy":"vip","st":0,"cost":0,"avaliable":1
                }));
            replies[1] = data::rights_reply(&rights);
            replies.truncate(2);
            let mut f = fixture::setup(replies).await;
            let outcome = f
                .client
                .fetch_native_media(
                    &fixture::credential_fixture("42", "selected-session")
                        .input()
                        .unwrap(),
                    "67474",
                    &request,
                    action,
                    BUDGET,
                    || Ok(()),
                )
                .await
                .unwrap();
            assert!(matches!(outcome, Outcome::Denied { .. }), "{pointer}");
            fixture::requests(&mut f, 2).await;
        }
        let (mut replies, mut request) = flow(true);
        request.quality = Quality::Auto;
        replies.truncate(2);
        let mut f = fixture::setup(replies).await;
        let outcome = f
            .client
            .fetch_native_media(
                &fixture::credential_fixture("42", "selected-session")
                    .input()
                    .unwrap(),
                "67474",
                &request,
                action,
                BUDGET,
                || Ok(()),
            )
            .await
            .unwrap();
        assert!(matches!(outcome, Outcome::Denied { .. }));
        let calls = fixture::requests(&mut f, 2).await;
        assert_eq!(data::query(&calls[1])["quality"], "ZPLY");
    }
}

#[tokio::test]
async fn native_vinyl_requires_matching_tier_format_and_valid_flac() {
    for (pointer, value) in [
        ("/data/quality", json!("ZP")),
        ("/data/quality", json!("ZPLY")),
        ("/data/quality", json!("ZPGA714")),
        ("/data/bitrate", json!(20900)),
        ("/data/bitrate", json!(24000)),
        ("/data/format", json!("mflac")),
        ("/data/format", json!("mgg")),
        ("/data/format", json!("mmp4")),
        ("/data/ekey", json!("malformed-key")),
        ("/data/surl", json!("https://er-sycdn.kuwo.cn/file.mflac")),
    ] {
        let (mut replies, request) = flow(true);
        let mut media = metadata(&replies[2]);
        *media.pointer_mut(pointer).unwrap() = value;
        replies[2] = json_response(&media);
        replies.truncate(3);
        let mut f = fixture::setup(replies).await;
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
    for encrypted in [false, true] {
        for malformed in 0..3 {
            let (mut replies, request) = flow(encrypted);
            let field = if encrypted {
                "cipher_base64"
            } else {
                "plain_base64"
            };
            let mut bytes = BASE64.decode(sample()[field].as_str().unwrap()).unwrap();
            if malformed == 0 {
                bytes.pop();
            } else if malformed == 1 {
                bytes[0] ^= 1;
            } else {
                let mp3 = audio::samples()
                    .into_iter()
                    .find(|s| s["format"] == "mp3")
                    .unwrap();
                bytes = BASE64.decode(mp3[field].as_str().unwrap()).unwrap();
            }
            replies[3] = response(200, "application/octet-stream", "", &bytes);
            let mut f = fixture::setup(replies).await;
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
    }
    // A FLAC-labelled response without its ekey cannot make cipher bytes valid.
    let (mut replies, request) = flow(true);
    let mut media = metadata(&replies[2]);
    media["data"]["ekey"] = json!("");
    replies[2] = json_response(&media);
    let mut f = fixture::setup(replies).await;
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

#[tokio::test]
async fn native_vinyl_keeps_plain_flac_metadata_and_requires_content_for_ciphertext() {
    let (plain, request) = flow(false);
    let mut f = fixture::setup(plain[..3].to_vec()).await;
    let stream = f
        .client
        .native_stream(&credential(), &data::track(), &request)
        .await
        .unwrap();
    assert_eq!(stream.requested_quality, Quality::Vinyl);
    assert_eq!(stream.actual_quality, Quality::Vinyl);
    assert_eq!(stream.bitrate, None);
    assert_eq!(stream.format.as_deref(), Some("flac"));
    assert_eq!(stream.codec.as_deref(), Some("flac"));
    assert!(stream.url.ends_with("/vinyl/file.flac"));
    assert!(stream.trial.is_none());
    fixture::requests(&mut f, 3).await;
    let mut f = fixture::setup(plain[..3].to_vec()).await;
    let download = f
        .client
        .native_download(&credential(), &data::track(), &request)
        .await
        .unwrap();
    assert!(download.available);
    assert_eq!(download.requested_quality, Quality::Vinyl);
    assert_eq!(download.actual_quality, Quality::Vinyl);
    assert_eq!(download.bitrate, None);
    assert_eq!(download.format.as_deref(), Some("flac"));
    assert!(download.url.unwrap().ends_with("/vinyl/file.flac"));
    fixture::requests(&mut f, 3).await;
    let (replies, request) = flow(true);
    let mut f = fixture::setup(replies[..3].to_vec()).await;
    assert_eq!(
        f.client
            .native_stream(&credential(), &data::track(), &request)
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    fixture::requests(&mut f, 3).await;
    let mut f = fixture::setup(replies[..3].to_vec()).await;
    let download = f
        .client
        .native_download(&credential(), &data::track(), &request)
        .await
        .unwrap();
    assert!(!download.available);
    assert!(download.url.is_none());
    assert_eq!(download.extensions["content_delivery"], "download_content");
    fixture::requests(&mut f, 3).await;
}

#[tokio::test]
async fn native_vinyl_rejects_unproven_layouts_and_selector_as_bitrate_before_network() {
    let credential = credential();
    for (bitrate, immersive_type, variant) in [
        (Some(23000), None, StreamVariant::Default),
        (Some(23_000_000), None, StreamVariant::Default),
        (
            None,
            Some(tuneweave_core::ImmersiveAudioType::C51),
            StreamVariant::Default,
        ),
        (None, None, StreamVariant::Modern),
    ] {
        let mut f = fixture::setup(vec![]).await;
        let request = StreamRequest {
            quality: Quality::Vinyl,
            bitrate,
            immersive_type,
            variant,
            ..data::request(None)
        };
        let error = f
            .client
            .native_audio_content(&credential, &data::track(), &request)
            .await
            .unwrap_err();
        assert_eq!(
            error.code,
            if bitrate.is_some() {
                ErrorCode::InvalidRequest
            } else {
                ErrorCode::CapabilityNotSupported
            }
        );
        fixture::requests(&mut f, 0).await;
    }
}
