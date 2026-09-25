use super::tests as data;
use super::*;
use crate::client::{catalog::tests::json_response, native::tests as fixture};

fn hires_media() -> serde_json::Value {
    let mut body = data::media();
    body["data"]["quality"] = json!("HR");
    body["data"]["bitrate"] = json!(4000);
    body["data"]["format"] = json!("flac");
    body["data"]["url"] = json!("");
    body["data"]["surl"] = json!("https://er-sycdn.kuwo.cn/hires/file.flac");
    body
}
fn rights(allow_hr: bool) -> serde_json::Value {
    let mut body = data::rights();
    body["songs"][0]["audio"] = json!([
        {"quality":"HR","br":4000,"fmt":"HIRFLAC","policy":"vip","st":if allow_hr {0} else {102},"cost":if allow_hr {0} else {1},"price":0,"avaliable":1},
        {"quality":"F","br":2000,"fmt":"ALFLAC","policy":"vip","st":0,"cost":0,"price":0,"avaliable":1}
    ]);
    body
}

#[tokio::test]
async fn native_hires_explicit_quality_never_downgrades_but_auto_selects_authorized_resources() {
    let credential = fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap();
    for action in [Action::Play, Action::Download] {
        for allow_hr in [false, true] {
            for automatic in [false, true] {
                let accepted = allow_hr || automatic;
                let mut media = hires_media();
                if !allow_hr {
                    media["data"]["quality"] = json!("F");
                    media["data"]["bitrate"] = json!(2000);
                }
                let mut replies = vec![
                    json_response(&json!({"result":"ok"})),
                    data::rights_reply(&rights(allow_hr)),
                ];
                if accepted {
                    replies.push(json_response(&media));
                }
                let mut f = fixture::setup(replies).await;
                let request = StreamRequest {
                    quality: if automatic {
                        Quality::Auto
                    } else {
                        Quality::Hires
                    },
                    ..data::request(None)
                };
                let quality = if allow_hr {
                    Quality::Hires
                } else {
                    Quality::Lossless
                };
                if action == Action::Play {
                    let result = f
                        .client
                        .native_stream(&credential, &data::track(), &request)
                        .await;
                    if accepted {
                        let result = result.unwrap();
                        assert_eq!(result.actual_quality, quality);
                        assert_eq!(result.bitrate, None);
                        assert!(result.trial.is_none());
                    } else {
                        assert_eq!(result.unwrap_err().code, ErrorCode::PermissionDenied);
                    }
                } else {
                    let result = f
                        .client
                        .native_download(&credential, &data::track(), &request)
                        .await
                        .unwrap();
                    assert_eq!(result.available, accepted);
                    assert_eq!(result.bitrate, None);
                    if accepted {
                        assert_eq!(result.actual_quality, quality);
                    } else {
                        assert!(result.url.is_none());
                    }
                }
                let calls = fixture::requests(&mut f, if accepted { 3 } else { 2 }).await;
                assert_eq!(
                    data::query(&calls[1])["quality"],
                    if automatic { "ZPLY" } else { "HR" }
                );
                assert_eq!(data::query(&calls[1])["action"], action.rights());
                if accepted {
                    assert_eq!(
                        data::query(&calls[2])["br"],
                        if allow_hr { "4000kflac" } else { "2000kflac" }
                    );
                }
            }
        }
    }
}

#[tokio::test]
async fn native_hires_requires_exact_rights_and_matching_media_without_invented_bitrate() {
    let credential = fixture::credential_fixture("42", "selected-session")
        .caller()
        .unwrap();
    let request = StreamRequest {
        quality: Quality::Hires,
        ..data::request(None)
    };
    for (pointer, value) in [
        ("/songs/0/audio/0/fmt", json!("ALFLAC")),
        ("/songs/0/audio/0/br", json!(2000)),
        ("/songs/0/audio/0/quality", json!("F")),
    ] {
        let mut rights = rights(true);
        *rights.pointer_mut(pointer).unwrap() = value;
        let mut f = fixture::setup(vec![
            json_response(&json!({"result":"ok"})),
            data::rights_reply(&rights),
        ])
        .await;
        assert_eq!(
            f.client
                .native_stream(&credential, &data::track(), &request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        fixture::requests(&mut f, 2).await;
    }
    for (pointer, value) in [
        ("/data/bitrate", json!(2000)),
        ("/data/quality", json!("F")),
        ("/data/format", json!("mflac")),
        (
            "/data/surl",
            json!("https://er-sycdn.kuwo.cn/hires/file.mflac"),
        ),
    ] {
        let mut media = hires_media();
        *media.pointer_mut(pointer).unwrap() = value;
        let mut f = fixture::setup(vec![
            json_response(&json!({"result":"ok"})),
            data::rights_reply(&rights(true)),
            json_response(&media),
        ])
        .await;
        assert_eq!(
            f.client
                .native_stream(&credential, &data::track(), &request)
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        fixture::requests(&mut f, 3).await;
    }
    for bitrate in [4000, 4_000_000, 128_000] {
        let mut f = fixture::setup(vec![]).await;
        assert_eq!(
            f.client
                .native_stream(
                    &credential,
                    &data::track(),
                    &StreamRequest {
                        bitrate: Some(bitrate),
                        ..request.clone()
                    }
                )
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
        fixture::requests(&mut f, 0).await;
    }
}
