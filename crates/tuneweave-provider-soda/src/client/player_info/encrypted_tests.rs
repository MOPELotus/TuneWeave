use super::*;
use sha1::{Digest, Sha1};

// Generated locally with ffmpeg from a one-second 440 Hz sine wave, AAC/128k,
// cenc-aes-ctr, synthetic key 00112233445566778899aabbccddeeff and KID 42*16.
// This file contains no platform recording, account or real authorization.
pub(crate) const AUDIO: &[u8] = include_bytes!("synthetic-aac-cenc.mp4");
pub(crate) const SPADE: &str = "nL8T+Vu+EvdZvhL1X7kR90G6DvVfpSbcXaUm3luiJdyloA==";
pub(crate) const KID: &str = "42424242424242424242424242424242";

pub(crate) fn fixture(preview: bool) -> (serde_json::Value, serde_json::Value) {
    let (mut body, mut info) = test_secondary_fixture(preview);
    if preview {
        body["track"]["preview"]["duration"] = json!(1000);
        body["track"]["audition_info"]["duration_ms"] = json!(1000);
    } else {
        body["track"]["duration"] = json!(1000);
    }
    info["Result"]["Data"]["Duration"] = json!(1.0);
    for row in info["Result"]["Data"]["PlayInfoList"]
        .as_array_mut()
        .unwrap()
    {
        row["Duration"] = json!(1.0);
        row["Size"] = json!(AUDIO.len());
        row["EncryptionMethod"] = json!("cenc-aes-ctr");
        row["PlayAuthID"] = json!(KID);
        row["PlayAuth"] = json!(SPADE);
    }
    (body, info)
}

pub(crate) fn audio_reply() -> Vec<u8> {
    let mut reply = format!("HTTP/1.1 200 OK\r\nContent-Type: audio/mp4\r\nContent-Length: {}\r\nSet-Cookie: sessionid_ss=cdn-must-not-rotate\r\nConnection: close\r\n\r\n", AUDIO.len()).into_bytes();
    reply.extend_from_slice(AUDIO);
    reply
}

pub(crate) fn assert_decrypted(bytes: &[u8]) {
    let mut cursor = 0;
    let mut payload = Vec::new();
    while cursor < bytes.len() {
        let size = u32::from_be_bytes(bytes[cursor..cursor + 4].try_into().unwrap()) as usize;
        assert!(size >= 8 && cursor + size <= bytes.len());
        if &bytes[cursor + 4..cursor + 8] == b"mdat" {
            payload.extend_from_slice(&bytes[cursor + 8..cursor + size]);
        }
        cursor += size;
    }
    // Independent oracle: ffmpeg -decryption_key <synthetic key> -i fixture.mp4
    // -map 0:a -c copy -f data -. Hash compares every decrypted AAC packet.
    assert_eq!(payload.len(), 14958);
    assert_eq!(
        hex::encode(Sha1::digest(&payload)),
        "ac001d65881454c4f6eebd3b43dd20440d50f441"
    );
}

fn authorized(body: &serde_json::Value, info: &serde_json::Value) -> Result<ValidatedSodaMedia> {
    let envelope = parse_track_envelope(&serde_json::to_vec(body).unwrap())?;
    let player = envelope.track_player.as_ref().unwrap();
    let model = parse_player_info(
        &serde_json::to_vec(info).unwrap(),
        player,
        envelope.status_info.now,
    )?;
    validate_decoded_player_model(
        envelope.track.as_ref().unwrap(),
        player,
        model,
        200000,
        envelope.status_info.now,
    )
}

#[test]
fn secondary_cenc_requires_complete_explicit_authorization_for_every_variant() {
    for preview in [false, true] {
        let (body, info) = fixture(preview);
        let media = authorized(&body, &info).unwrap();
        assert!(media.encrypted);
        assert_eq!(media.preview, preview);
        assert_eq!(media.selected.key_id.as_deref(), Some(KID));
        assert_eq!(media.selected.spade_a.as_deref(), Some(SPADE));
        let public = serde_json::to_string(&media.specs).unwrap();
        assert!(!public.contains(KID) && !public.contains(SPADE));
        for mutation in 0..13 {
            let mut bad = info.clone();
            // A bad unselected quality must not be silently discarded.
            let row = &mut bad["Result"]["Data"]["PlayInfoList"][0];
            match mutation {
                0 => row["EncryptionMethod"] = json!(""),
                1 => row["PlayAuthID"] = json!(""),
                2 => row["PlayAuth"] = json!(""),
                3 => row["EncryptionMethod"] = json!("aes-128-cbc"),
                4 => row["EncryptionMethod"] = json!("widevine"),
                5 => row["EncryptionMethod"] = json!("CENC-AES-CTR"),
                6 => row["PlayAuthID"] = json!("42"),
                7 => row["PlayAuthID"] = json!("g".repeat(32)),
                8 => row["PlayAuthID"] = json!(format!(" {KID}")),
                9 => row["PlayAuth"] = json!(format!("{SPADE}\n")),
                10 => row["PlayAuth"] = json!("A".repeat(16385)),
                11 => row["PlayAuth"] = json!("private-secret!"),
                _ => {
                    row.as_object_mut().unwrap().remove("PlayAuthID");
                }
            }
            let error = match authorized(&body, &bad) {
                Ok(_) => panic!("invalid authorization {mutation} accepted"),
                Err(e) => e,
            };
            assert_eq!(error.code, ErrorCode::UpstreamError);
            assert!(!format!("{error:?}").contains(SPADE));
        }
    }
}

#[tokio::test]
async fn secondary_cenc_content_matches_independent_ffmpeg_packets_and_preview_metadata() {
    for preview in [false, true] {
        let (body, mut info) = fixture(preview);
        // Each quality retains its own key ID; never take the first row's ID.
        info["Result"]["Data"]["PlayInfoList"][0]["PlayAuthID"] = json!("11".repeat(16));
        let (origin, server) = crate::test_http::serve_bytes(vec![
            crate::test_http::json(&info.to_string(), Some("sessionid_ss=ignored-info-cookie"))
                .into_bytes(),
            audio_reply(),
        ])
        .await;
        let client = SodaClient::test_client().with_auth_test_origin(origin);
        let identity = SodaTrackIdentity::parse("7304719759323564095").unwrap();
        let media = parse_authorized_media(
            &client,
            &serde_json::to_vec(&body).unwrap(),
            &identity,
            200000,
            "synthetic secondary CENC",
        )
        .await
        .unwrap();
        let content = client
            .deliver_audio_content(&identity, media)
            .await
            .unwrap();
        assert_decrypted(&content.bytes);
        assert_eq!(content.content_type, "audio/mp4");
        assert_eq!(
            content.trial,
            preview.then_some(tuneweave_core::TrialWindow {
                start_ms: 107904,
                end_ms: 108904
            })
        );
        if !preview && let Some(dir) = std::env::var_os("TUNEWEAVE_SODA_SYNTHETIC_OUTPUT_DIR") {
            std::fs::write(
                std::path::Path::new(&dir).join("synthetic-aac-decrypted.m4a"),
                &content.bytes,
            )
            .unwrap();
        }
        let requests = server.await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[1].starts_with("GET /media/higher?"));
        for request in requests {
            assert!(!request.to_ascii_lowercase().contains("cookie:"));
            assert!(!request.contains(SPADE) && !request.contains(KID));
        }
    }
}

#[tokio::test]
async fn secondary_cenc_rejects_unsupported_decrypted_codec_configuration() {
    let (body, info) = fixture(false);
    let media = authorized(&body, &info).unwrap();
    let mut malformed = AUDIO.to_vec();
    let esds = malformed
        .windows(4)
        .position(|window| window == b"esds")
        .expect("AAC decoder configuration");
    let box_start = esds - 4;
    let box_size = u32::from_be_bytes(malformed[box_start..esds].try_into().unwrap()) as usize;
    let box_end = box_start + box_size;
    let oti = malformed[esds + 4..box_end]
        .windows(2)
        .position(|window| window == [0x40, 0x15])
        .expect("MPEG-4 AAC object type indication")
        + esds
        + 4;
    // Keep the CENC tables, KID, and encrypted samples valid while changing AAC
    // to the MPEG Layer III OTI. The decryptor alone does not inspect esds.
    malformed[oti] = 0x69;
    assert_eq!(malformed.len(), AUDIO.len());
    assert_eq!(
        malformed.iter().zip(AUDIO).filter(|(a, b)| a != b).count(),
        1
    );
    let decrypted = crate::media::decrypt_cenc_audio(malformed.clone(), SPADE, KID)
        .expect("CENC structure and key remain valid after the esds-only mutation");
    assert_eq!(decrypted.format, SodaAudioFormat::Aac);
    assert!(crate::media::plaintext::validate(decrypted.bytes, decrypted.format).is_err());
    let mut reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: audio/mp4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        malformed.len()
    )
    .into_bytes();
    reply.extend_from_slice(&malformed);
    let (origin, server) = crate::test_http::serve_bytes(vec![reply]).await;
    let client = SodaClient::test_client().with_auth_test_origin(origin);
    let identity = SodaTrackIdentity::parse("7304719759323564095").unwrap();

    let error = client
        .deliver_audio_content(&identity, media)
        .await
        .expect_err("unsupported clear codec configuration must not be delivered");

    assert_eq!(error.code, ErrorCode::UpstreamError);
    assert_eq!(server.await.unwrap().len(), 1);
}

#[tokio::test]
async fn secondary_cenc_rejects_wrong_media_kid_bad_spade_and_declared_codec_before_output() {
    for mutation in 0..3 {
        let (body, mut info) = fixture(false);
        for row in info["Result"]["Data"]["PlayInfoList"]
            .as_array_mut()
            .unwrap()
        {
            match mutation {
                0 => row["PlayAuthID"] = json!("11".repeat(16)),
                1 => row["PlayAuth"] = json!("AA=="),
                _ => row["Codec"] = json!("flac"),
            }
        }
        let (origin, server) = crate::test_http::serve_bytes(vec![audio_reply()]).await;
        let client = SodaClient::test_client().with_auth_test_origin(origin);
        let media = authorized(&body, &info).unwrap();
        let identity = SodaTrackIdentity::parse("7304719759323564095").unwrap();
        let error = client
            .deliver_audio_content(&identity, media)
            .await
            .unwrap_err();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert!(!format!("{error:?}").contains(SPADE));
        assert_eq!(server.await.unwrap().len(), 1);
    }
}
