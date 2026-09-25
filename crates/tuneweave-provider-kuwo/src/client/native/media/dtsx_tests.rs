use super::*;
use super::{content::tests as audio, tests as data};
use crate::client::{catalog::tests::json_response, native::tests as fixture};
use serde_json::Value;

const TOKEN: &str = "ddddddddddddddddeeeeeeeeeeeeeeee";

pub(crate) fn request() -> StreamRequest {
    StreamRequest {
        quality: Quality::Dtsx,
        ..data::request(None)
    }
}
pub(crate) fn rights() -> Value {
    let mut value = data::rights();
    value["uid"] = json!("42");
    value["songs"][0]["audio"][0] = json!({
        "quality":"DTSX","br":25000,"fmt":"DTSX",
        "policy":"vip","st":0,"cost":0,"price":0,"avaliable":1
    });
    value["songs"][0]["token"]["DTSX"] = json!(TOKEN);
    value
}
pub(crate) fn media() -> Value {
    let mut value = data::media();
    value["loginSid"] = json!("selected-session");
    value["data"]["format"] = json!("mmp4");
    value["data"]["quality"] = json!("DTSX");
    value["data"]["bitrate"] = json!(25000);
    value["data"]["url"] = json!("http://er-sycdn.kuwo.cn/dtsx/file.mmp4");
    value["data"]["surl"] = json!("https://er-sycdn.kuwo.cn/dtsx/file.mmp4");
    // Only reuse the independently generated device/key envelope. There are no
    // DTS audio bytes in this protocol fixture, and it proves no codec playback.
    value["data"]["ekey"] = audio::samples()[0]["ekey"].clone();
    value
}
pub(crate) fn flow() -> Vec<Vec<u8>> {
    vec![
        json_response(&json!({"result":"ok"})),
        data::rights_reply(&rights()),
        json_response(&media()),
    ]
}
fn input() -> KuwoNativeSessionInput {
    fixture::credential_fixture("42", "selected-session")
        .input()
        .unwrap()
}
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}
async fn resolve(client: &KuwoClient, request: &StreamRequest, action: Action) -> Result<Outcome> {
    client
        .fetch_native_media(&input(), "67474", request, action, BUDGET, || Ok(()))
        .await
}

#[tokio::test]
async fn native_dtsx_exact_selector_and_action_bound_token_resolve_encrypted_identity() {
    for action in [Action::Play, Action::Download] {
        let mut f = fixture::setup(flow()).await;
        let outcome = resolve(&f.client, &request(), action).await.unwrap();
        match outcome {
            Outcome::Allowed {
                url,
                backups,
                format,
                bitrate,
                quality,
                duration_ms,
                key,
                trial,
            } => {
                assert_eq!(url, "https://er-sycdn.kuwo.cn/dtsx/file.mmp4");
                assert!(backups.is_empty());
                assert_eq!(format, "mmp4");
                assert_eq!(quality, Quality::Dtsx);
                assert!(bitrate.is_none() && key.is_some() && trial.is_none());
                assert_eq!(duration_ms, 240_000);
            }
            _ => panic!("expected the exact encrypted resource"),
        }
        let calls = fixture::requests(&mut f, 3).await;
        let grant = data::query(&calls[1]);
        assert_eq!(grant["quality"], "DTSX");
        assert_eq!(grant["action"], action.rights());
        assert_eq!(grant["uid"], "42");
        assert_eq!(grant["sid"], "selected-session");
        let media = data::query(&calls[2]);
        assert_eq!(media["br"], "25000kmmp4");
        assert_eq!(media["format"], "flac|mp3|aac");
        assert_eq!(media["mode"], action.mode());
        assert_eq!(media["token"], TOKEN);
        assert_eq!(media["bc_token"], "");
        assert_eq!(media["rid"], "67474");
        assert_eq!(media["timestamp"], "1789540000");
        assert_eq!(media["playPay"], "000111111111");
        assert_eq!(media["downloadPay"], "000111111111");
        assert_eq!(media["loginUid"], "42");
        assert_eq!(media["loginSid"], "selected-session");
        assert!(
            calls
                .iter()
                .all(|r| !r.to_ascii_lowercase().contains("cookie:"))
        );
    }
}

#[tokio::test]
async fn native_dtsx_denied_or_mismatched_grant_never_selects_lower_or_preview_media() {
    for action in [Action::Play, Action::Download] {
        for (pointer, replacement) in [
            ("/songs/0/audio/0/st", json!(502)),
            ("/songs/0/audio/0/avaliable", json!(0)),
            ("/songs/0/audio/0/br", json!(24000)),
            ("/songs/0/audio/0/br", json!(2000)),
            ("/songs/0/audio/0/fmt", json!("MMP4")),
            ("/songs/0/audio/0/quality", json!("ZPGA714")),
            ("/songs/0/audio/0/quality", json!("ZPGA201")),
            ("/songs/0/audio/0/quality", json!("BCMS")),
        ] {
            let mut grant = rights();
            *grant.pointer_mut(pointer).unwrap() = replacement;
            grant["songs"][0]["audio"].as_array_mut().unwrap().extend([
                json!({"quality":"F","br":2000,"fmt":"ALFLAC","policy":"vip","st":0,"cost":0,"avaliable":1}),
                json!({"quality":"H","br":128,"fmt":"MP3128","policy":"song","st":103,"price":2,"avaliable":1}),
            ]);
            let mut f = fixture::setup(vec![flow()[0].clone(), data::rights_reply(&grant)]).await;
            assert!(
                matches!(
                    resolve(&f.client, &request(), action).await.unwrap(),
                    Outcome::Denied { .. }
                ),
                "{pointer}"
            );
            fixture::requests(&mut f, 2).await;
        }
        let mut f = fixture::setup(flow()[..2].to_vec()).await;
        let mut automatic = request();
        automatic.quality = Quality::Auto;
        assert!(matches!(
            resolve(&f.client, &automatic, action).await.unwrap(),
            Outcome::Denied { .. }
        ));
        assert_eq!(
            data::query(&fixture::requests(&mut f, 2).await[1])["quality"],
            "ZPLY"
        );
    }
}

#[tokio::test]
async fn native_dtsx_grant_identity_and_own_token_are_required_before_media_resolution() {
    for action in [Action::Play, Action::Download] {
        for mode in 0..7 {
            let mut grant = rights();
            match mode {
                0 => grant["uid"] = json!("43"),
                1 => grant["songs"][0]["id"] = json!(67475),
                2 => grant["timestamp"] = json!(0),
                3 => grant["songs"][0]["token"]["DTSX"] = json!(""),
                4 => {
                    grant["songs"][0]["token"]
                        .as_object_mut()
                        .unwrap()
                        .remove("DTSX");
                }
                5 => grant["songs"][0]["token"]["DTSX"] = json!("selected-session"),
                _ => grant["songs"][0]["token"]["DTSX"] = json!("bad token"),
            }
            let mut f = fixture::setup(vec![flow()[0].clone(), data::rights_reply(&grant)]).await;
            assert!(
                matches!(resolve(&f.client, &request(), action).await, Err(e) if e.code == ErrorCode::UpstreamError),
                "mode={mode}"
            );
            fixture::requests(&mut f, 2).await;
        }
    }
}

#[tokio::test]
async fn native_dtsx_play_grant_does_not_authorize_a_later_download() {
    let mut download = rights();
    download["songs"][0]["payInfo"]["cannotDownload"] = json!(1);
    let mut replies = flow();
    replies.extend([flow()[0].clone(), data::rights_reply(&download)]);
    let mut f = fixture::setup(replies).await;
    assert!(matches!(
        resolve(&f.client, &request(), Action::Play).await.unwrap(),
        Outcome::Allowed { .. }
    ));
    assert!(matches!(
        resolve(&f.client, &request(), Action::Download)
            .await
            .unwrap(),
        Outcome::Denied { .. }
    ));
    let calls = fixture::requests(&mut f, 5).await;
    assert_eq!(data::query(&calls[1])["action"], "play");
    assert_eq!(data::query(&calls[4])["action"], "download");
}

#[test]
fn native_dtsx_response_requires_exact_identity_valid_key_and_trusted_mmp4_url() {
    for (pointer, replacement) in [
        ("/loginSid", json!("other-session")),
        ("/duration", json!(0)),
        ("/data/rid", json!(67475)),
        ("/data/quality", json!(null)),
        ("/data/quality", json!("")),
        ("/data/quality", json!("ZPGA714")),
        ("/data/quality", json!("VINYL")),
        ("/data/bitrate", json!(24000)),
        ("/data/bitrate", json!(25000000)),
        ("/data/format", json!("mp4")),
        ("/data/format", json!("flac")),
        ("/data/format", json!("mflac")),
        ("/data/format", json!("mgg")),
        ("/data/ekey", json!(null)),
        ("/data/ekey", json!("")),
        ("/data/ekey", json!("malformed-key")),
        (
            "/data/surl",
            json!("https://er-sycdn.kuwo.cn/dtsx/file.mp4"),
        ),
        ("/data/surl", json!("https://other.example/file.mmp4")),
        (
            "/data/surl",
            json!("https://er-sycdn.kuwo.cn/dtsx/file.mmp4?sid=selected-session"),
        ),
        (
            "/data/surl",
            json!("http://er-sycdn.kuwo.cn/dtsx/file.mmp4"),
        ),
        ("/data/url", json!("http://other.example/file.mmp4")),
    ] {
        let mut reply = media();
        *reply.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            matches!(response::parse(&serde_json::to_vec(&reply).unwrap(), &input(), "67474", DTSX), Err(e) if e.code == ErrorCode::UpstreamError),
            "{pointer}"
        );
    }
    for absent in ["quality", "ekey", "rid", "type", "startPos", "endPos"] {
        let mut reply = media();
        reply["data"].as_object_mut().unwrap().remove(absent);
        assert!(
            response::parse(
                &serde_json::to_vec(&reply).unwrap(),
                &input(),
                "67474",
                DTSX
            )
            .is_err(),
            "{absent}"
        );
    }
}

#[tokio::test]
async fn native_dtsx_partial_media_is_rejected_by_both_content_operations_before_cdn_io() {
    for action in [Action::Play, Action::Download] {
        for field in ["type", "startPos", "endPos"] {
            let mut value = media();
            value["data"][field] = json!(1);
            let mut replies = flow();
            replies[2] = json_response(&value);
            let mut f = fixture::setup(replies).await;
            let result = match action {
                Action::Play => {
                    f.client
                        .native_audio_content(&credential(), &data::track(), &request())
                        .await
                }
                Action::Download => {
                    f.client
                        .native_download_content(&credential(), &data::track(), &request())
                        .await
                }
            };
            assert_eq!(result.unwrap_err().code, ErrorCode::PermissionDenied);
            fixture::requests(&mut f, 3).await;
        }
    }
}

#[tokio::test]
async fn native_dtsx_json_resolution_never_exports_an_encrypted_url_or_selector_bitrate() {
    let mut f = fixture::setup(flow()).await;
    assert_eq!(
        f.client
            .native_stream(&credential(), &data::track(), &request())
            .await
            .unwrap_err()
            .code,
        ErrorCode::CapabilityNotSupported
    );
    fixture::requests(&mut f, 3).await;
    let mut f = fixture::setup(flow()).await;
    let download = f
        .client
        .native_download(&credential(), &data::track(), &request())
        .await
        .unwrap();
    assert!(
        !download.available
            && download.url.is_none()
            && download.bitrate.is_none()
            && download.size.is_none()
    );
    assert_eq!(download.requested_quality, Quality::Dtsx);
    assert_eq!(download.actual_quality, Quality::Auto);
    assert_eq!(download.extensions["content_delivery"], "download_content");
    let public = serde_json::to_string(&download).unwrap();
    for secret in [
        "selected-session",
        TOKEN,
        "file.mmp4",
        "25000",
        "do-not-export",
    ] {
        assert!(!public.contains(secret));
    }
    fixture::requests(&mut f, 3).await;
}

#[tokio::test]
async fn native_dtsx_invalid_bitrate_or_variant_fails_before_io() {
    for (bitrate, variant, immersive_type) in [
        (Some(25000), StreamVariant::Default, None),
        (Some(25000000), StreamVariant::Default, None),
        (None, StreamVariant::SingAlong, None),
        (None, StreamVariant::Modern, None),
        (
            None,
            StreamVariant::Default,
            Some(tuneweave_core::ImmersiveAudioType::C51),
        ),
    ] {
        let mut f = fixture::setup(vec![]).await;
        let request = StreamRequest {
            bitrate,
            variant,
            immersive_type,
            ..request()
        };
        let error = f
            .client
            .native_audio_content(&credential(), &data::track(), &request)
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
