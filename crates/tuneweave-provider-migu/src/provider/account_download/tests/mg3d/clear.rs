use super::*;

const FLAC: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/client/mg3d/flac/tests/synthetic-stereo.flac"
));
const FLAC_24: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/client/mg3d/flac/tests/synthetic-stereo-24.flac"
));
const FLAC_96K: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/client/mg3d/flac/tests/synthetic-stereo-24-96k.flac"
));

// These are the existing locally generated fixtures; no platform media is used.
// Their generation and source hashes live beside the FLAC validator tests.

fn clear_responses(
    bytes: &[u8],
    tone: &str,
    format: &str,
    key: Option<serde_json::Value>,
) -> Vec<Vec<u8>> {
    let mut frames = responses();
    let response = String::from_utf8(frames[4].clone()).unwrap();
    let mut detail: serde_json::Value =
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    detail["resource"][0]["rateFormats"][0]["formatType"] = json!(tone);
    detail["resource"][0]["rateFormats"][0]["format"] = json!(format);
    frames[4] = reply(detail, None).into_bytes();
    let mut body = grant();
    body["data"].as_object_mut().unwrap().remove("fileKey");
    if let Some(key) = key {
        body["data"]["fileKey"] = key;
    }
    body["data"]["formatId"] = json!(format);
    body["data"]["suffix"] = json!(if matches!(tone, "PQ" | "HQ") {
        "mp3"
    } else {
        "flac"
    });
    body["data"]["size"] = json!(bytes.len());
    frames[5] = reply(body, None).into_bytes();
    frames[MEDIA] = media_frame(bytes, "");
    frames
}

#[tokio::test]
async fn native_clear_download_content_supports_proven_formats_and_all_selected_account_modes() {
    let standard = plain();
    let mut high = vec![0_u8; 1044];
    high[..4].copy_from_slice(&[0xff, 0xfb, 0xe0, 0]);
    let high = high.repeat(40);
    for (tone, format, quality, bytes) in [
        ("PQ", "020007", Quality::Standard, standard.as_slice()),
        ("HQ", "020008", Quality::High, high.as_slice()),
        ("SQ", "020010", Quality::Lossless, FLAC),
        ("ZQ24", "011005", Quality::Hires, FLAC_24),
        ("ZQ24", "011005", Quality::Hires, FLAC_96K),
    ] {
        for (mode, key) in [
            ("default", None),
            ("named", Some(json!(""))),
            ("caller", Some(json!(null))),
        ] {
            let mut frames = clear_responses(bytes, tone, format, key);
            let mime = if matches!(tone, "PQ" | "HQ") {
                "audio/mpeg"
            } else {
                "audio/flac"
            };
            if mode == "named" {
                let mut response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    bytes.len()
                )
                .into_bytes();
                response.extend_from_slice(bytes);
                frames[MEDIA] = response;
            }
            let mut h = binary_server(frames, None).await;
            let (store, original, alias) = setup(&mut h.provider, mode);
            let req = StreamRequest {
                quality,
                ..request(alias)
            };
            let result = h
                .provider
                .audio_download_content(&track(), &req)
                .await
                .unwrap();
            assert_eq!(result.bytes, bytes);
            assert_eq!(result.content_type, mime);
            assert_eq!(
                result.filename,
                if matches!(tone, "PQ" | "HQ") {
                    "123.mp3"
                } else {
                    "123.flac"
                }
            );
            assert!(result.trial.is_none());
            assert_eq!(result.track_ref, track().resource_ref);
            assert_eq!(read(&store, "other").token(), "unrelated-pacm");
            if mode == "caller" {
                assert_eq!(read(&store, alias), original);
                let update = h.provider.take_response_credential().unwrap().unwrap();
                assert_eq!(
                    MiguCredential::parse_caller(&update).unwrap().token(),
                    "final-pacm"
                );
                assert!(!update.secret().contains("native-token-fixture"));
                assert!(h.provider.take_response_credential().unwrap().is_none());
            } else {
                assert_eq!(read(&store, alias).token(), "final-pacm");
            }
            for secret in [
                "native-token-fixture",
                "do-not-retain-session",
                "download-fixture",
                FILE_KEY,
            ] {
                assert!(!format!("{result:?}").contains(secret));
            }
            let wire = h.wire.await.unwrap();
            assert_eq!(wire.len(), 8);
            assert!(wire[5].starts_with(&format!("GET /MIGUM2.0/strategy/download-url/by-songid/v1.0?contentId=123&formatType={tone}&songId=456 ")));
            for forbidden in [
                "pacmtoken",
                "cookie:",
                "token:",
                "ce:",
                "sign:",
                "authorization:",
                "range:",
                FILE_KEY,
            ] {
                assert!(
                    !wire[MEDIA]
                        .to_lowercase()
                        .contains(&forbidden.to_lowercase())
                );
            }
            assert!(
                wire.iter()
                    .all(|request| !request.contains("/listen") && !request.contains("can-listen"))
            );
        }
    }
}

#[tokio::test]
async fn native_clear_download_corruption_or_unexpected_transfer_never_returns_bytes_or_falls_back()
{
    for (bytes, tone, format, quality) in [
        (plain(), "PQ", "020007", Quality::Standard),
        (FLAC.to_vec(), "SQ", "020010", Quality::Lossless),
    ] {
        let mut corrupt = bytes.clone();
        corrupt[0] ^= 1;
        let mut wrong_mime = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: audio/aac\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            bytes.len()
        )
        .into_bytes();
        wrong_mime.extend_from_slice(&bytes);
        for response in [
            b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            b"HTTP/1.1 302 Found\r\nLocation: https://evil.example/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            b"HTTP/1.1 206 Partial Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
            media_frame(&bytes, "Content-Range: bytes 0-0/1\r\n"),
            media_frame(&bytes, "Content-Encoding: gzip\r\n"),
            media_frame(&bytes[..bytes.len()-1], ""),
            media_frame(&corrupt, ""),
            wrong_mime,
        ] {
            let mut frames = clear_responses(&bytes, tone, format, None);
            frames[MEDIA] = response;
            frames.truncate(MEDIA+1);
            let mut h = binary_server(frames, None).await;
            let (store, original, alias) = setup(&mut h.provider, "caller");
            let req = StreamRequest {quality, ..request(alias)};
            let failure = h.provider.audio_download_content(&track(), &req).await.unwrap_err();
            assert_eq!(failure.code, ErrorCode::UpstreamError);
            assert!(!format!("{failure:?}").contains("download-fixture"));
            assert_eq!(read(&store, alias), original);
            assert_eq!(h.wire.await.unwrap().len(), 7);
        }
    }
    // A missing key must not turn encrypted media into ordinary downloadable bytes.
    let mut frames = clear_responses(&plain(), "PQ", "020007", None);
    frames[MEDIA] = media_frame(&encrypted(), "");
    frames.truncate(7);
    let mut h = binary_server(frames, None).await;
    let (_, _, alias) = setup(&mut h.provider, "named");
    let req = StreamRequest {
        quality: Quality::Auto,
        ..request(alias)
    };
    assert!(
        h.provider
            .audio_download_content(&track(), &req)
            .await
            .is_err()
    );
    assert_eq!(h.wire.await.unwrap().len(), 7);
}

#[tokio::test]
async fn native_clear_download_flac_integrity_and_authorized_bit_depth_remain_mandatory() {
    for (tone, format, quality, mut bytes, corrupt_md5) in [
        ("SQ", "020010", Quality::Lossless, FLAC.to_vec(), true),
        ("ZQ24", "011005", Quality::Hires, FLAC_24.to_vec(), true),
        ("ZQ24", "011005", Quality::Hires, FLAC.to_vec(), false),
    ] {
        if corrupt_md5 {
            // STREAMINFO PCM MD5, not framing.
            bytes[26] ^= 1;
        }
        let mut frames = clear_responses(&bytes, tone, format, None);
        frames.truncate(7);
        let mut h = binary_server(frames, None).await;
        let (_, _, alias) = setup(&mut h.provider, "named");
        let req = StreamRequest {
            quality,
            ..request(alias)
        };
        assert_eq!(
            h.provider
                .audio_download_content(&track(), &req)
                .await
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError
        );
        assert_eq!(h.wire.await.unwrap().len(), 7);
    }
}

#[tokio::test]
async fn native_clear_download_checks_resource_identity_and_does_not_treat_mgm_as_cleartext() {
    for (field, value) in [
        ("fileKey", json!(" ")),
        ("fileKey", json!("short")),
        ("encryptionType", json!("1")),
        ("encryptionType", json!("")),
        ("contentId", json!("other")),
        ("copyrightId", json!("other")),
        ("formatId", json!("unknown")),
        ("suffix", json!("wav")),
        ("auditionsLength", json!(60)),
        ("size", json!(64 * 1024 * 1024 + 1)),
        (
            "url",
            json!("http://dlsdownfree.nf.migu.cn/wlansst?pars=fixture"),
        ),
        ("url", json!("https://evil.example/wlansst?pars=fixture")),
    ] {
        let mut frames = clear_responses(&plain(), "PQ", "020007", None);
        let response = String::from_utf8(frames[5].clone()).unwrap();
        let mut grant: serde_json::Value =
            serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
        grant["data"][field] = value;
        frames[5] = reply(grant, None).into_bytes();
        frames.truncate(6);
        let mut h = binary_server(frames, None).await;
        let (_, _, alias) = setup(&mut h.provider, "named");
        assert!(
            h.provider
                .audio_download_content(&track(), &request(alias))
                .await
                .is_err(),
            "{field}"
        );
        assert_eq!(h.wire.await.unwrap().len(), 6);
    }
}

#[tokio::test]
async fn native_clear_download_final_uid_and_secret_checks_withhold_complete_bytes() {
    for variant in 0..3 {
        let mut frames = clear_responses(&plain(), "PQ", "020007", None);
        if variant == 0 {
            frames[7] = profile("222", "").into_bytes();
        } else {
            let response = String::from_utf8(frames[5].clone()).unwrap();
            let mut body: serde_json::Value =
                serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
            body["data"]["url"] = json!(format!(
                "https://dlsdownfree.nf.migu.cn/wlansst/song?pars={}",
                if variant == 1 {
                    "native-token-fixture"
                } else {
                    "final-pacm"
                }
            ));
            frames[5] = reply(body, None).into_bytes();
            if variant == 1 {
                frames.truncate(6);
            }
        }
        let expected = frames.len();
        let mut h = binary_server(frames, None).await;
        let (store, original, alias) = setup(&mut h.provider, "caller");
        let failure = h
            .provider
            .audio_download_content(&track(), &request(alias))
            .await
            .unwrap_err();
        assert!(!format!("{failure:?}").contains("native-token-fixture"));
        assert_eq!(read(&store, alias), original);
        if variant == 0 {
            assert_eq!(failure.code, ErrorCode::AuthenticationRequired);
            assert!(h.provider.take_response_credential().unwrap().is_none());
        }
        assert_eq!(h.wire.await.unwrap().len(), expected);
    }
}

#[tokio::test]
async fn native_clear_download_preserves_original_login_generation_at_every_network_boundary() {
    for at in 0..8 {
        let mut frames = clear_responses(&plain(), "PQ", "020007", None);
        frames.truncate(at + 1);
        let mut h = binary_server(frames, Some(at)).await;
        let (store, _, alias) = setup(&mut h.provider, "named");
        let provider = h.provider.clone();
        let task = tokio::spawn(async move {
            provider
                .audio_download_content(&track(), &request(alias))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), h.seen)
            .await
            .unwrap()
            .unwrap();
        let replacement =
            MiguCredential::verified("111".into(), "replacement-login".into()).unwrap();
        store.put(&stored(alias, &replacement)).unwrap();
        h.release.send(()).unwrap();
        assert_eq!(
            task.await.unwrap().unwrap_err().code,
            ErrorCode::Conflict,
            "at {at}"
        );
        assert_eq!(read(&store, alias), replacement);
        assert_eq!(h.wire.await.unwrap().len(), at + 1);
    }
}

#[tokio::test]
async fn native_clear_download_cancel_clears_undelivered_caller_credentials_at_each_boundary() {
    for at in 0..8 {
        let mut frames = clear_responses(&plain(), "PQ", "020007", None);
        frames.truncate(at + 1);
        let mut h = binary_server(frames, Some(at)).await;
        let (store, original, alias) = setup(&mut h.provider, "caller");
        let provider = h.provider.clone();
        let task = tokio::spawn(async move {
            provider
                .audio_download_content(&track(), &request(alias))
                .await
        });
        tokio::time::timeout(Duration::from_secs(5), h.seen)
            .await
            .unwrap()
            .unwrap();
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        assert!(h.provider.take_response_credential().unwrap().is_none());
        assert_eq!(read(&store, alias), original);
        h.wire.abort();
        assert!(h.wire.await.unwrap_err().is_cancelled());
    }
}
