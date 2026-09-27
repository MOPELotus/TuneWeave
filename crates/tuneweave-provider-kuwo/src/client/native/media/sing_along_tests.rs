use super::*;
use super::{content::sing_along::tests as audio, tests as data};
use crate::client::{
    catalog::tests::{json_response, response},
    native::tests as fixture,
};
use base64::engine::general_purpose::STANDARD as BASE64;
use serde_json::Value;

fn metadata(response: &[u8]) -> Value {
    let start = response.windows(4).position(|v| v == b"\r\n\r\n").unwrap() + 4;
    serde_json::from_slice(&response[start..]).unwrap()
}
pub(crate) fn flow() -> (Vec<Vec<u8>>, StreamRequest) {
    let sample = audio::sample();
    let mut rights = data::rights();
    rights["songs"][0]["audio"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "quality":"BCMS", "br":22000,"fmt":"BCMS", "policy":"vip",
            "st":0,"cost":0,"price":0,"avaliable":1
        }));
    rights["songs"][0]["token"]["BCMS"] = json!("ddddddddddddddddeeeeeeeeeeeeeeee");
    let mut media = data::media();
    media["data"]["bc_data"] = json!({
        "rid":67474,"format":"mgg","bitrate":22000,"quality":"BCMS",
        "surl":"https://er-sycdn.kuwo.cn/sing/file.mgg","url":"",
        "ekey":sample["ekey"],"type":0,"startPos":0,"endPos":0
    });
    let mut request = data::request(None);
    request.variant = StreamVariant::SingAlong;
    (
        vec![
            data::flow().remove(0),
            data::rights_reply(&rights),
            json_response(&media),
            response(
                200,
                "application/octet-stream",
                "",
                &BASE64
                    .decode(sample["cipher_base64"].as_str().unwrap())
                    .unwrap(),
            ),
        ],
        request,
    )
}
fn credential() -> ProviderCredential {
    fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap()
}
async fn deliver(
    client: &KuwoClient,
    request: &StreamRequest,
    action: Action,
) -> Result<tuneweave_core::AudioContent> {
    match action {
        Action::Play => {
            client
                .native_audio_content(&credential(), &data::track(), request)
                .await
        }
        Action::Download => {
            client
                .native_download_content(&credential(), &data::track(), request)
                .await
        }
    }
}

#[tokio::test]
async fn native_sing_along_uses_independent_bc_token_and_delivers_full_stereo_wave_for_each_action()
{
    for action in [Action::Play, Action::Download] {
        for main_quality in [Quality::Standard, Quality::Lossless, Quality::Auto] {
            let (mut replies, mut request) = flow();
            request.quality = main_quality;
            let lossless = main_quality != Quality::Standard;
            if lossless {
                let mut rights = metadata(&replies[1]);
                rights["songs"][0]["audio"][0] = json!({"quality":"F","br":2000,"fmt":"ALFLAC","policy":"vip","st":0,"cost":0,"avaliable":1});
                replies[1] = data::rights_reply(&rights);
                let mut media = metadata(&replies[2]);
                media["data"]["quality"] = json!("F");
                media["data"]["format"] = json!("flac");
                media["data"]["bitrate"] = json!(2000);
                media["data"]["url"] = json!("");
                media["data"]["surl"] = json!("https://er-sycdn.kuwo.cn/main/file.flac");
                replies[2] = json_response(&media);
            }
            let mut f = fixture::setup(replies).await;
            let delivered = deliver(&f.client, &request, action).await.unwrap();
            assert_eq!(delivered.content_type, "audio/wav");
            assert_eq!(delivered.filename, "kuwo-67474-sing-along.wav");
            assert_eq!(delivered.bytes.len(), 44 + 11_025 * 4);
            assert_eq!(&delivered.bytes[..4], b"RIFF");
            assert_eq!(
                u16::from_le_bytes(delivered.bytes[22..24].try_into().unwrap()),
                2
            );
            assert!(delivered.trial.is_none());
            let calls = fixture::requests(&mut f, 4).await;
            let rights = data::query(&calls[1]);
            assert_eq!(rights["quality"], "BCMS");
            assert_eq!(rights["action"], action.rights());
            let media = data::query(&calls[2]);
            assert_eq!(media["br"], if lossless { "2000kflac" } else { "128kmp3" });
            assert_eq!(
                media["format"],
                if lossless { "flac|mp3|aac" } else { "mp3|aac" }
            );
            assert_eq!(media["mode"], action.mode());
            assert_eq!(
                media["token"],
                if lossless {
                    "55555555555555556666666666666666"
                } else {
                    "11111111111111112222222222222222"
                }
            );
            assert_eq!(media["bc_token"], "ddddddddddddddddeeeeeeeeeeeeeeee");
            assert_eq!(media["loginSid"], "selected-session");
            assert!(calls[3].starts_with("GET /sing/file.mgg "));
            assert!(!calls[3].contains("selected-session"));
            assert!(!calls[3].to_ascii_lowercase().contains("cookie:"));
        }
    }
}

#[tokio::test]
async fn native_sing_along_requires_both_full_rights_and_never_uses_ordinary_preview() {
    for action in [Action::Play, Action::Download] {
        for (pointer, value) in [
            ("/songs/0/audio/1/st", json!(502)),
            ("/songs/0/audio/1/avaliable", json!(0)),
            ("/songs/0/audio/1/quality", json!("ZPGA714")),
            ("/songs/0/audio/1/br", json!(24000)),
            ("/songs/0/audio/1/fmt", json!("MP3128")),
            ("/songs/0/audio/0/st", json!(502)),
            ("/songs/0/audio/0/avaliable", json!(0)),
        ] {
            let (mut replies, request) = flow();
            let mut rights = metadata(&replies[1]);
            *rights.pointer_mut(pointer).unwrap() = value;
            replies[1] = data::rights_reply(&rights);
            replies.truncate(2);
            let mut f = fixture::setup(replies).await;
            assert_eq!(
                deliver(&f.client, &request, action).await.unwrap_err().code,
                ErrorCode::PermissionDenied,
                "{pointer}"
            );
            fixture::requests(&mut f, 2).await;
        }
        for which in ["H", "BCMS"] {
            let (mut replies, request) = flow();
            let mut rights = metadata(&replies[1]);
            rights["songs"][0]["token"]
                .as_object_mut()
                .unwrap()
                .remove(which);
            replies[1] = data::rights_reply(&rights);
            replies.truncate(2);
            let mut f = fixture::setup(replies).await;
            assert_eq!(
                deliver(&f.client, &request, action).await.unwrap_err().code,
                ErrorCode::UpstreamError
            );
            fixture::requests(&mut f, 2).await;
        }
        let (mut replies, request) = flow();
        let mut rights = metadata(&replies[1]);
        rights["songs"][0]["audio"][0]["policy"] = json!("vip");
        rights["songs"][0]["audio"][0]["st"] = json!(102);
        replies[1] = data::rights_reply(&rights);
        replies.truncate(2);
        let mut f = fixture::setup(replies).await;
        assert_eq!(
            deliver(&f.client, &request, action).await.unwrap_err().code,
            ErrorCode::PermissionDenied
        );
        fixture::requests(&mut f, 2).await;
    }
}

#[tokio::test]
async fn native_sing_along_download_cannot_reuse_play_authorization() {
    for action in [Action::Play, Action::Download] {
        let (mut replies, request) = flow();
        let mut rights = metadata(&replies[1]);
        rights["songs"][0]["payInfo"]["cannotDownload"] = json!(1);
        replies[1] = data::rights_reply(&rights);
        if action == Action::Download {
            replies.truncate(2);
        }
        let mut f = fixture::setup(replies).await;
        let result = deliver(&f.client, &request, action).await;
        if action == Action::Play {
            assert!(result.is_ok());
        } else {
            assert_eq!(result.unwrap_err().code, ErrorCode::PermissionDenied);
        }
        let calls = fixture::requests(&mut f, if action == Action::Play { 4 } else { 2 }).await;
        assert_eq!(data::query(&calls[1])["action"], action.rights());
    }
}

#[tokio::test]
async fn native_sing_along_rejects_partial_main_or_sidecar_for_both_actions() {
    for action in [Action::Play, Action::Download] {
        for parent in ["/data", "/data/bc_data"] {
            for field in ["type", "startPos", "endPos"] {
                let (mut replies, request) = flow();
                let mut media = metadata(&replies[2]);
                *media.pointer_mut(&format!("{parent}/{field}")).unwrap() = json!(1);
                if field == "startPos" {
                    *media.pointer_mut(&format!("{parent}/endPos")).unwrap() = json!(30);
                }
                replies[2] = json_response(&media);
                replies.truncate(3);
                let mut f = fixture::setup(replies).await;
                assert_eq!(
                    deliver(&f.client, &request, action).await.unwrap_err().code,
                    ErrorCode::PermissionDenied
                );
                fixture::requests(&mut f, 3).await;
            }
        }
    }
}

#[tokio::test]
async fn native_sing_along_checks_sidecar_identity_codec_and_encrypted_container() {
    for (pointer, value) in [
        ("/data/rid", json!(67475)),
        ("/data/bc_data", Value::Null),
        ("/data/bc_data/rid", json!(67475)),
        ("/data/bc_data/format", json!("mmp4")),
        ("/data/bc_data/quality", json!("ZPGA714")),
        ("/data/bc_data/quality", Value::Null),
        ("/data/bc_data/bitrate", json!(128)),
        ("/data/bc_data/ekey", json!("")),
        (
            "/data/bc_data/surl",
            json!("https://er-sycdn.kuwo.cn/sing/file.mp3"),
        ),
    ] {
        let (mut replies, request) = flow();
        let mut media = metadata(&replies[2]);
        *media.pointer_mut(pointer).unwrap() = value;
        replies[2] = json_response(&media);
        replies.truncate(3);
        let mut f = fixture::setup(replies).await;
        assert_eq!(
            deliver(&f.client, &request, Action::Play)
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError,
            "{pointer}"
        );
        fixture::requests(&mut f, 3).await;
    }
    for truncate in [false, true] {
        let (mut replies, request) = flow();
        let mut body = BASE64
            .decode(audio::sample()["cipher_base64"].as_str().unwrap())
            .unwrap();
        if truncate {
            body.pop();
        } else {
            body[0] ^= 1;
        }
        replies[3] = response(200, "application/octet-stream", "", &body);
        let mut f = fixture::setup(replies).await;
        assert_eq!(
            deliver(&f.client, &request, Action::Play)
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        fixture::requests(&mut f, 4).await;
    }
}

#[tokio::test]
async fn native_sing_along_does_not_expose_stem_url_or_borrow_main_output_metadata() {
    let (replies, request) = flow();
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
    assert!(download.url.is_none() && download.bitrate.is_none() && download.size.is_none());
    assert_eq!(download.actual_quality, Quality::Auto);
    assert_eq!(download.requested_quality, Quality::Standard);
    assert_eq!(download.extensions["content_delivery"], "download_content");
    assert_eq!(download.extensions["requested_variant"], "sing_along");
    assert!(
        !serde_json::to_string(&download)
            .unwrap()
            .contains("file.mgg")
    );
    fixture::requests(&mut f, 3).await;
}

#[tokio::test]
async fn native_sing_along_selects_only_ordinary_main_tiers_and_default_does_not_select_stems() {
    for (quality, spec) in [
        (Quality::Low, LOW),
        (Quality::Standard, STANDARD),
        (Quality::Higher, HIGH),
        (Quality::High, HIGH),
        (Quality::Lossless, LOSSLESS),
        (Quality::Hires, HI_RES),
        (Quality::Auto, HI_RES),
    ] {
        let (_, mut request) = flow();
        request.quality = quality;
        assert_eq!(specs(&request).unwrap()[0], spec);
    }
    for quality in [Quality::Master, Quality::Spatial, Quality::Vinyl] {
        let (_, mut request) = flow();
        request.quality = quality;
        let mut f = fixture::setup(Vec::new()).await;
        assert_eq!(
            deliver(&f.client, &request, Action::Play)
                .await
                .unwrap_err()
                .code,
            ErrorCode::CapabilityNotSupported
        );
        fixture::requests(&mut f, 0).await;
    }
    let (replies, mut request) = flow();
    request.bitrate = Some(128_000);
    assert_eq!(
        validate_request(&request).unwrap_err().code,
        ErrorCode::InvalidRequest
    );
    request.bitrate = None;
    request.variant = StreamVariant::Default;
    let mut f = fixture::setup(replies[..3].to_vec()).await;
    let stream = f
        .client
        .native_stream(&credential(), &data::track(), &request)
        .await
        .unwrap();
    assert!(stream.url.ends_with("file.mp3"));
    assert_eq!(stream.actual_quality, Quality::Standard);
    let calls = fixture::requests(&mut f, 3).await;
    assert_eq!(data::query(&calls[1])["quality"], "H");
    assert_eq!(data::query(&calls[2])["bc_token"], "");
}
