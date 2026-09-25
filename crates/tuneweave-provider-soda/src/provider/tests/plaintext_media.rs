use super::*;
use crate::media::plaintext::tests::{AAC, ALAC, FLAC, reply};

fn replies(codec: &str, bytes: &[u8], secondary: bool, preview: bool) -> Vec<Vec<u8>> {
    let (mut body, mut info) = crate::client::player_info::test_secondary_fixture(preview);
    if preview {
        body["track"]["preview"]["duration"] = json!(1000);
        body["track"]["audition_info"]["duration_ms"] = json!(1000);
    } else {
        body["track"]["duration"] = json!(1000);
    }
    if secondary {
        info["Result"]["Data"]["Duration"] = json!(1.0);
        for row in info["Result"]["Data"]["PlayInfoList"]
            .as_array_mut()
            .unwrap()
        {
            row["Duration"] = json!(1.0);
            row["Size"] = json!(bytes.len());
            row["Codec"] = json!(codec);
            // All three encryption fields intentionally remain empty.
        }
    } else {
        let direct: serde_json::Value =
            serde_json::from_slice(&crate::client::test_account_track_fixture(preview)).unwrap();
        let mut model: serde_json::Value =
            serde_json::from_str(direct["track_player"]["video_model"].as_str().unwrap()).unwrap();
        model["video_duration"] = json!(1.0);
        for row in model["video_list"].as_array_mut().unwrap() {
            row["video_meta"]["size"] = json!(bytes.len());
            row["video_meta"]["codec_type"] = json!(codec);
            row["encrypt_info"] = json!({"encrypt":false});
        }
        body["track_player"]["video_model"] = json!(model.to_string());
    }
    let mut result = vec![
        account_reply("123456", Some("sessionid_ss=verified")).into_bytes(),
        crate::test_http::json(&body.to_string(), Some("sessionid_ss=media-current")).into_bytes(),
    ];
    if secondary {
        result.push(
            crate::test_http::json(&info.to_string(), Some("sessionid_ss=info-must-not-rotate"))
                .into_bytes(),
        );
    }
    result.push(reply(bytes));
    result
}

async fn deliver(
    owner: &str,
    codec: &str,
    bytes: &[u8],
    secondary: bool,
    preview: bool,
) -> Result<tuneweave_core::AudioContent> {
    let mut f = SessionFixture::new();
    let source = test_soda_credential().bind_user("123456").unwrap();
    for alias in ["default", "personal", "other"] {
        f.put(alias, &source);
    }
    let (origin, server) =
        crate::test_http::serve_bytes(replies(codec, bytes, secondary, preview)).await;
    f.provider.client = f.provider.client.clone().with_auth_test_origin(origin);
    let p = if owner == "caller" {
        f.provider
            .caller_credential_scope(&caller_from(&source))
            .unwrap()
    } else {
        f.provider.clone()
    };
    let track = Track::new(
        tuneweave_core::ResourceRef::new(Platform::Soda, "7304719759323564095").unwrap(),
        "synthetic",
    );
    let result = p
        .audio_content(
            &track,
            &StreamRequest {
                account: (owner != "caller").then(|| owner.to_owned()),
                ..StreamRequest::default()
            },
        )
        .await;
    // Content failure cannot undo an earlier independently verified account rotation.
    if owner == "caller" {
        let update = p.take_response_credential().unwrap().unwrap();
        assert_eq!(
            parse_soda_caller_credential(&update)
                .unwrap()
                .cookie_header()
                .unwrap(),
            "sessionid_ss=media-current"
        );
    } else {
        assert!(f.stored(owner).unwrap().secret().contains("media-current"));
    }
    for alias in ["default", "personal", "other"]
        .into_iter()
        .filter(|a| *a != owner)
    {
        assert_eq!(
            f.stored(alias).unwrap().secret(),
            source.serialize().unwrap()
        );
    }
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), if secondary { 4 } else { 3 });
    assert!(requests[0].contains("cookie: sessionid_ss=session-secret\r\n"));
    assert!(requests[1].contains("cookie: sessionid_ss=verified\r\n"));
    for request in &requests[2..] {
        assert!(!request.to_ascii_lowercase().contains("cookie:") && !request.contains("twc1_"));
    }
    result
}

#[tokio::test]
async fn plaintext_media_three_sources_deliver_all_codecs_with_original_bytes_and_trial() {
    for owner in ["default", "personal", "caller"] {
        for secondary in [false, true] {
            for preview in [false, true] {
                for (codec, bytes, mime, extension) in [
                    ("aac", AAC, "audio/mp4", "m4a"),
                    ("alac", ALAC, "audio/mp4", "m4a"),
                    ("flac", FLAC, "audio/flac", "flac"),
                ] {
                    let result = deliver(owner, codec, bytes, secondary, preview)
                        .await
                        .unwrap();
                    assert_eq!(result.bytes, bytes);
                    assert_eq!(result.content_type, mime);
                    assert_eq!(
                        result.filename,
                        format!("soda-7304719759323564095.{extension}")
                    );
                    assert_eq!(
                        result.trial,
                        preview.then_some(tuneweave_core::TrialWindow {
                            start_ms: 107904,
                            end_ms: 108904
                        })
                    );
                    if owner == "default"
                        && secondary
                        && !preview
                        && let Some(dir) = std::env::var_os("TUNEWEAVE_SODA_SYNTHETIC_OUTPUT_DIR")
                    {
                        std::fs::write(
                            std::path::Path::new(&dir)
                                .join(format!("delivered-{codec}.{extension}")),
                            &result.bytes,
                        )
                        .unwrap();
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn plaintext_media_three_sources_reject_disguised_encryption_truncation_and_wrong_codec() {
    let mut trailing = FLAC.to_vec();
    trailing.push(0);
    for owner in ["default", "personal", "caller"] {
        for secondary in [false, true] {
            for (codec, bytes) in [
                ("aac", crate::client::player_info::encrypted_tests::AUDIO),
                ("aac", &AAC[..AAC.len() - 1]),
                ("aac", ALAC),
                ("alac", AAC),
                ("flac", trailing.as_slice()),
                ("flac", b"fLaC".as_slice()),
            ] {
                let error = deliver(owner, codec, bytes, secondary, false)
                    .await
                    .unwrap_err();
                assert_eq!(error.code, ErrorCode::UpstreamError);
                for secret in [
                    "session-secret",
                    "verified",
                    "media-current",
                    "player-secret",
                    "cdn-must-not-rotate",
                    "info-must-not-rotate",
                ] {
                    assert!(!format!("{error:?}").contains(secret));
                }
            }
        }
    }
}
