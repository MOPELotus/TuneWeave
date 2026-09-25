use super::*;

const FLAC: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/client/mg3d/flac/tests/synthetic-stereo.flac"
));
const FLAC_24: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/client/mg3d/flac/tests/synthetic-stereo-24.flac"
));
const FLAC_24_96K: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/client/mg3d/flac/tests/synthetic-stereo-24-96k.flac"
));
const FLAC_24_88K2: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/client/mg3d/flac/tests/synthetic-stereo-24-88k2.flac"
));
const FLAC_24_176K4: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/client/mg3d/flac/tests/synthetic-stereo-24-176k4.flac"
));
const FLAC_24_192K: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/client/mg3d/flac/tests/synthetic-stereo-24-192k.flac"
));

fn encode(bytes: &[u8]) -> Vec<u8> {
    let key = b"CB4E917FFEB2B4A056445F4B3544495E";
    bytes
        .iter()
        .enumerate()
        .map(|(i, v)| (*v).wrapping_add(key[i % 32]))
        .collect()
}
fn first_flac_frame_offset(bytes: &[u8]) -> usize {
    let mut at = 4;
    loop {
        let header = &bytes[at..at + 4];
        let length =
            (usize::from(header[1]) << 16) | (usize::from(header[2]) << 8) | usize::from(header[3]);
        at += 4 + length;
        if header[0] & 128 != 0 {
            return at;
        }
    }
}
fn flac_responses(bytes: &[u8], tone: &str, format_id: &str) -> Vec<Vec<u8>> {
    let mut frames = responses();
    let response = String::from_utf8(frames[4].clone()).unwrap();
    let (_, body) = response.split_once("\r\n\r\n").unwrap();
    let mut detail: serde_json::Value = serde_json::from_str(body).unwrap();
    detail["resource"][0]["rateFormats"][0]["formatType"] = json!(tone);
    detail["resource"][0]["rateFormats"][0]["format"] = json!(format_id);
    frames[4] = reply(detail, None).into_bytes();
    let mut body = grant();
    body["data"]["formatId"] = json!(format_id);
    body["data"]["suffix"] = json!("flac");
    body["data"]["size"] = json!(bytes.len());
    frames[5] = reply(body, None).into_bytes();
    frames[MEDIA] = media_frame(&encode(bytes), "");
    frames
}
fn flac_request(alias: &str) -> StreamRequest {
    StreamRequest {
        quality: Quality::Lossless,
        ..request(alias)
    }
}

fn hires_request(alias: &str) -> StreamRequest {
    StreamRequest {
        quality: Quality::Hires,
        ..request(alias)
    }
}

#[tokio::test]
async fn mg3d_flac_selected_account_flow_returns_complete_flac_without_secrets_or_transcoding() {
    for (tone, format_id, quality, bytes) in [
        ("SQ", "020010", Quality::Lossless, FLAC),
        ("ZQ24", "011005", Quality::Hires, FLAC_24),
        ("ZQ24", "011005", Quality::Hires, FLAC_24_96K),
        ("ZQ24", "011005", Quality::Hires, FLAC_24_88K2),
        ("ZQ24", "011005", Quality::Hires, FLAC_24_176K4),
        ("ZQ24", "011005", Quality::Hires, FLAC_24_192K),
    ] {
        for mode in ["default", "named", "caller"] {
            let mut frames = flac_responses(bytes, tone, format_id);
            if mode == "named" {
                let mut response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: audio/flac\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            ).into_bytes();
                response.extend_from_slice(&encode(bytes));
                frames[MEDIA] = response;
            }
            let mut h = binary_server(frames, None).await;
            let (store, original, alias) = setup(&mut h.provider, mode);
            let request = StreamRequest {
                quality,
                ..request(alias)
            };
            let result = h
                .provider
                .audio_download_content(&track(), &request)
                .await
                .unwrap();
            assert_eq!(result.bytes, bytes);
            assert_eq!(result.content_type, "audio/flac");
            assert_eq!(result.filename, "123.flac");
            assert_eq!(result.track_ref, track().resource_ref);
            assert!(result.trial.is_none());
            assert_eq!(read(&store, "other").token(), "unrelated-pacm");
            for secret in [FILE_KEY, "native-token-fixture", "download-fixture", "pacm"] {
                assert!(!format!("{result:?}").contains(secret));
            }
            if mode == "caller" {
                assert_eq!(read(&store, alias), original);
                let credential = h.provider.take_response_credential().unwrap().unwrap();
                assert_eq!(
                    MiguCredential::parse_caller(&credential).unwrap().token(),
                    "final-pacm"
                );
                for secret in [FILE_KEY, "native-token-fixture", "download-fixture"] {
                    assert!(!credential.secret().contains(secret));
                }
                assert!(h.provider.take_response_credential().unwrap().is_none());
            } else {
                assert_eq!(read(&store, alias).token(), "final-pacm");
            }
            let wire = h.wire.await.unwrap();
            assert_eq!(wire.len(), 8);
            assert!(wire[5].contains(&format!("formatType={tone}")));
            for forbidden in [
                FILE_KEY,
                "cookie:",
                "pacmtoken",
                "token:",
                "sign:",
                "usessionid",
                "authorization:",
                "range:",
            ] {
                assert!(
                    !wire[MEDIA]
                        .to_lowercase()
                        .contains(&forbidden.to_lowercase())
                );
            }
            assert!(
                wire.iter()
                    .all(|r| !r.contains("/listen") && !r.contains("cloud/"))
            );
        }
    }
}

#[tokio::test]
async fn mg3d_flac_bad_integrity_or_transfer_type_never_returns_content_or_falls_back() {
    for (tone, format_id, quality, valid) in [
        ("SQ", "020010", Quality::Lossless, FLAC),
        ("ZQ24", "011005", Quality::Hires, FLAC_24),
    ] {
        let mut invalid_md5 = valid.to_vec();
        invalid_md5[26] ^= 1;
        for (bytes, mime) in [
            (invalid_md5.as_slice(), "application/octet-stream"),
            (valid, "audio/mpeg"),
        ] {
            let mut frames = flac_responses(bytes, tone, format_id);
            frames[MEDIA] = media_frame(&encode(bytes), "");
            if mime == "audio/mpeg" {
                let response = &frames[MEDIA];
                let split = response.windows(4).position(|v| v == b"\r\n\r\n").unwrap() + 4;
                let mut changed = String::from_utf8(response[..split].to_vec())
                    .unwrap()
                    .replace("application/octet-stream", mime)
                    .into_bytes();
                changed.extend_from_slice(&response[split..]);
                frames[MEDIA] = changed;
            }
            frames.truncate(MEDIA + 1);
            let mut h = binary_server(frames, None).await;
            let (store, original, alias) = setup(&mut h.provider, "caller");
            let failure = h
                .provider
                .audio_download_content(
                    &track(),
                    &StreamRequest {
                        quality,
                        ..request(alias)
                    },
                )
                .await
                .unwrap_err();
            assert_eq!(failure.code, ErrorCode::UpstreamError);
            assert_eq!(read(&store, alias), original);
            assert_eq!(h.wire.await.unwrap().len(), 7);
        }
    }
}

#[tokio::test]
async fn mg3d_zq24_grant_does_not_override_bad_frame_signature_crc_or_actual_bit_depth() {
    let frame_start = first_flac_frame_offset(FLAC_24);
    let mut bad_signature = FLAC_24.to_vec();
    bad_signature[frame_start] ^= 1;
    let mut bad_crc = FLAC_24.to_vec();
    let last = bad_crc.len() - 1;
    bad_crc[last] ^= 1;
    let bad_bit_depth = FLAC.to_vec();

    for bytes in [&bad_signature, &bad_crc, &bad_bit_depth] {
        let mut frames = flac_responses(bytes, "ZQ24", "011005");
        frames.truncate(MEDIA + 1);
        let mut h = binary_server(frames, None).await;
        let (_, _, alias) = setup(&mut h.provider, "named");
        let failure = h
            .provider
            .audio_download_content(
                &track(),
                &StreamRequest {
                    quality: Quality::Auto,
                    ..request(alias)
                },
            )
            .await
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::UpstreamError);
        assert_eq!(h.wire.await.unwrap().len(), 7);
    }
}

#[tokio::test]
async fn mg3d_flac_final_uid_or_generation_change_discards_fully_decoded_content() {
    let mut frames = flac_responses(FLAC_24, "ZQ24", "011005");
    frames[7] = profile("222", "").into_bytes();
    let mut h = binary_server(frames, None).await;
    let (_, _, alias) = setup(&mut h.provider, "caller");
    assert!(
        h.provider
            .audio_download_content(&track(), &hires_request(alias))
            .await
            .is_err()
    );
    assert!(h.provider.take_response_credential().unwrap().is_none());
    assert_eq!(h.wire.await.unwrap().len(), 8);
    let mut h = binary_server(flac_responses(FLAC_24, "ZQ24", "011005"), Some(7)).await;
    let (store, _, alias) = setup(&mut h.provider, "named");
    let provider = h.provider.clone();
    let task = tokio::spawn(async move {
        provider
            .audio_download_content(&track(), &hires_request(alias))
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), h.seen)
        .await
        .unwrap()
        .unwrap();
    let next = MiguCredential::verified("111".into(), "new-login-pacm".into()).unwrap();
    store.put(&stored(alias, &next)).unwrap();
    h.release.send(()).unwrap();
    assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
    assert_eq!(read(&store, alias), next);
    assert_eq!(h.wire.await.unwrap().len(), 8);
}

#[tokio::test]
async fn mg3d_flac_cancel_at_final_verification_does_not_publish_caller_rotation() {
    let mut h = binary_server(flac_responses(FLAC_24, "ZQ24", "011005"), Some(7)).await;
    let (store, original, alias) = setup(&mut h.provider, "caller");
    let provider = h.provider.clone();
    let task = tokio::spawn(async move {
        provider
            .audio_download_content(&track(), &hires_request(alias))
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), h.seen)
        .await
        .unwrap()
        .unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(read(&store, alias), original);
    assert!(h.provider.take_response_credential().unwrap().is_none());
    h.wire.abort();
}

#[tokio::test]
async fn mg3d_flac_auto_only_falls_back_when_sq_authorization_is_denied() {
    let mut frames = frames()
        .into_iter()
        .map(String::into_bytes)
        .collect::<Vec<_>>();
    frames[4]=reply(json!({"code":"000000","resource":[{"resourceType":"2","contentId":"123","copyrightId":"6005971HBUU","songId":"456","songName":"Song","length":"00:01","rateFormats":[{"formatType":"ZQ24","format":"011005"},{"formatType":"SQ","format":"020010"},{"formatType":"PQ","format":"020007"}]}]}),None).into_bytes();
    frames.insert(5, reply(json!({"code":"200010"}), None).into_bytes());
    let mut sq_grant = grant();
    sq_grant["data"]["formatId"] = json!("020010");
    sq_grant["data"]["suffix"] = json!("flac");
    sq_grant["data"]["size"] = json!(FLAC.len());
    frames.insert(6, reply(sq_grant, None).into_bytes());
    frames[7] = media_frame(&encode(FLAC), "");
    let mut h = binary_server(frames, None).await;
    let (_, _, alias) = setup(&mut h.provider, "named");
    let result = h
        .provider
        .audio_download_content(
            &track(),
            &StreamRequest {
                quality: Quality::Auto,
                ..request(alias)
            },
        )
        .await
        .unwrap();
    assert_eq!(result.bytes, FLAC);
    assert_eq!(result.content_type, "audio/flac");
    let wire = h.wire.await.unwrap();
    assert_eq!(wire.len(), 9);
    assert!(wire[5].contains("formatType=ZQ24"));
    assert!(wire[6].contains("formatType=SQ"));
    let mut frames = flac_responses(FLAC, "SQ", "020010");
    frames[5] = reply(json!({"code":"200010"}), None).into_bytes();
    frames.truncate(6);
    let mut h = binary_server(frames, None).await;
    let (_, _, alias) = setup(&mut h.provider, "named");
    assert_eq!(
        h.provider
            .audio_download_content(&track(), &flac_request(alias))
            .await
            .unwrap_err()
            .code,
        ErrorCode::PermissionDenied
    );
    assert_eq!(h.wire.await.unwrap().len(), 6);
}

#[tokio::test]
async fn mg3d_zq24_96k_must_match_the_grant_and_cannot_fallback_after_corruption() {
    let mut corrupt = FLAC_24_96K.to_vec();
    corrupt[26] ^= 1;
    for (tone, format_id, bytes, quality) in [
        ("SQ", "020010", FLAC_24_96K, Quality::Lossless),
        ("ZQ24", "011005", corrupt.as_slice(), Quality::Auto),
    ] {
        let mut frames = flac_responses(bytes, tone, format_id);
        frames.truncate(MEDIA + 1);
        let mut h = binary_server(frames, None).await;
        let (store, original, alias) = setup(&mut h.provider, "caller");
        let failure = h
            .provider
            .audio_download_content(
                &track(),
                &StreamRequest {
                    quality,
                    ..request(alias)
                },
            )
            .await
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::UpstreamError);
        assert!(!format!("{failure:?}").contains(FILE_KEY));
        assert_eq!(read(&store, alias), original);
        assert_eq!(h.wire.await.unwrap().len(), 7);
    }
}

#[tokio::test]
async fn mg3d_zq24_96k_generation_change_and_cancellation_withhold_decoded_content() {
    for mode in ["named", "caller"] {
        let mut h = binary_server(flac_responses(FLAC_24_96K, "ZQ24", "011005"), Some(7)).await;
        let (store, original, alias) = setup(&mut h.provider, mode);
        let provider = h.provider.clone();
        let task = tokio::spawn(async move {
            provider
                .audio_download_content(&track(), &hires_request(alias))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), h.seen)
            .await
            .unwrap()
            .unwrap();
        if mode == "named" {
            let next = MiguCredential::verified("111".into(), "new-login-pacm".into()).unwrap();
            store.put(&stored(alias, &next)).unwrap();
            h.release.send(()).unwrap();
            assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
            assert_eq!(read(&store, alias), next);
            assert_eq!(h.wire.await.unwrap().len(), 8);
        } else {
            task.abort();
            assert!(task.await.unwrap_err().is_cancelled());
            assert_eq!(read(&store, alias), original);
            assert!(h.provider.take_response_credential().unwrap().is_none());
            h.wire.abort();
        }
    }
}

#[tokio::test]
async fn mg3d_zq24_high_rates_require_matching_sq_bits_and_never_fallback_after_corruption() {
    for valid in [FLAC_24_88K2, FLAC_24_176K4, FLAC_24_192K] {
        let mut corrupt = valid.to_vec();
        corrupt[26] ^= 1;
        for (tone, format_id, bytes, quality) in [
            ("SQ", "020010", valid, Quality::Lossless),
            ("ZQ24", "011005", corrupt.as_slice(), Quality::Auto),
        ] {
            let mut frames = flac_responses(bytes, tone, format_id);
            frames.truncate(MEDIA + 1);
            let mut h = binary_server(frames, None).await;
            let (store, original, alias) = setup(&mut h.provider, "caller");
            let failure = h
                .provider
                .audio_download_content(
                    &track(),
                    &StreamRequest {
                        quality,
                        ..request(alias)
                    },
                )
                .await
                .unwrap_err();
            assert_eq!(failure.code, ErrorCode::UpstreamError);
            assert_eq!(read(&store, alias), original);
            assert_eq!(read(&store, "other").token(), "unrelated-pacm");
            assert!(!format!("{failure:?}").contains(FILE_KEY));
            let wire = h.wire.await.unwrap();
            assert_eq!(wire.len(), 7);
            assert!(wire[5].contains(&format!("formatType={tone}")));
            assert!(wire.iter().all(|request| !request.contains("/listen")));
        }
    }
}

#[tokio::test]
async fn mg3d_zq24_high_rates_need_the_selected_resource_format_before_any_cdn_request() {
    for bytes in [FLAC_24_88K2, FLAC_24_176K4, FLAC_24_192K] {
        let mut frames = flac_responses(bytes, "ZQ24", "011005");
        let mut wrong_grant = grant();
        wrong_grant["data"]["formatId"] = json!("020010");
        wrong_grant["data"]["suffix"] = json!("flac");
        wrong_grant["data"]["size"] = json!(bytes.len());
        frames[5] = reply(wrong_grant, None).into_bytes();
        frames.truncate(MEDIA);
        let mut h = binary_server(frames, None).await;
        let (store, original, alias) = setup(&mut h.provider, "caller");
        let failure = h
            .provider
            .audio_download_content(&track(), &hires_request(alias))
            .await
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::UpstreamError);
        assert_eq!(read(&store, alias), original);
        let wire = h.wire.await.unwrap();
        assert_eq!(wire.len(), MEDIA);
        assert!(wire[5].contains("formatType=ZQ24"));
        assert!(wire.iter().all(|request| !request.contains("/wlansst")));
    }
}
