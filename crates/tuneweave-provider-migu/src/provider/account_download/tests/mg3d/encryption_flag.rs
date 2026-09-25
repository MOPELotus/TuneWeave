use super::*;

const FLAC: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/client/mg3d/flac/tests/synthetic-stereo.flac"
));
const FLAC_24: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/client/mg3d/flac/tests/synthetic-stereo-24-96k.flac"
));

fn flag_one_grant() -> serde_json::Value {
    let mut body = grant();
    body["data"]["encryptionType"] = json!("1");
    body
}

fn flag_one_frames(tone: &str, format: &str, clear: &[u8]) -> Vec<Vec<u8>> {
    let mut frames = responses();
    let response = String::from_utf8(frames[4].clone()).unwrap();
    let mut detail: serde_json::Value =
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    detail["resource"][0]["rateFormats"][0]["formatType"] = json!(tone);
    detail["resource"][0]["rateFormats"][0]["format"] = json!(format);
    frames[4] = reply(detail, None).into_bytes();
    let mut body = flag_one_grant();
    body["data"]["formatId"] = json!(format);
    body["data"]["suffix"] = json!(if matches!(tone, "PQ" | "HQ") {
        "mp3"
    } else {
        "flac"
    });
    body["data"]["size"] = json!(clear.len());
    frames[5] = reply(body, None).into_bytes();
    // Existing independently verified synthetic MG3D key, no platform media.
    let key = b"CB4E917FFEB2B4A056445F4B3544495E";
    let encrypted = clear
        .iter()
        .enumerate()
        .map(|(i, v)| (*v).wrapping_add(key[i % 32]))
        .collect::<Vec<_>>();
    frames[MEDIA] = media_frame(&encrypted, "");
    frames
}

#[tokio::test]
async fn native_mg3d_flag_one_uses_the_existing_decoder_for_each_authorized_rendition() {
    let pq = plain();
    let mut frame = vec![0_u8; 1044];
    frame[..4].copy_from_slice(&[0xff, 0xfb, 0xe0, 0]);
    let hq = frame.repeat(40);
    for (tone, format, quality, clear) in [
        ("PQ", "020007", Quality::Standard, pq.as_slice()),
        ("HQ", "020009", Quality::High, hq.as_slice()),
        ("SQ", "020010", Quality::Lossless, FLAC),
        ("ZQ24", "011005", Quality::Hires, FLAC_24),
    ] {
        for mode in ["default", "named", "caller"] {
            let mut h = binary_server(flag_one_frames(tone, format, clear), None).await;
            let (store, original, alias) = setup(&mut h.provider, mode);
            let result = h
                .provider
                .audio_download_content(
                    &track(),
                    &StreamRequest {
                        quality,
                        ..request(alias)
                    },
                )
                .await
                .unwrap();
            assert_eq!(result.bytes, clear);
            assert_eq!(result.track_ref, track().resource_ref);
            assert_eq!(
                result.content_type,
                if matches!(tone, "PQ" | "HQ") {
                    "audio/mpeg"
                } else {
                    "audio/flac"
                }
            );
            assert!(result.trial.is_none());
            assert_eq!(read(&store, "other").token(), "unrelated-pacm");
            if mode == "caller" {
                assert_eq!(read(&store, alias), original);
                let update = h.provider.take_response_credential().unwrap().unwrap();
                assert_eq!(
                    MiguCredential::parse_caller(&update).unwrap().token(),
                    "final-pacm"
                );
                for secret in [FILE_KEY, "native-token-fixture", "do-not-retain-session"] {
                    assert!(!update.secret().contains(secret));
                }
                assert!(h.provider.take_response_credential().unwrap().is_none());
            } else {
                assert_eq!(read(&store, alias).token(), "final-pacm");
            }
            let wire = h.wire.await.unwrap();
            assert_eq!(wire.len(), 8);
            assert!(wire[5].starts_with(&format!("GET /MIGUM2.0/strategy/download-url/by-songid/v1.0?contentId=123&formatType={tone}&songId=456 ")));
            for forbidden in [
                FILE_KEY,
                "cookie:",
                "pacmtoken",
                "token:",
                "sign:",
                "ce:",
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
async fn native_mg3d_flag_one_keeps_mgm_unknown_policy_and_incomplete_media_rejected() {
    for (field, value) in [
        ("fileKey", json!("")),
        ("fileKey", json!(null)),
        ("fileKey", json!("short")),
        ("encryptionType", json!("2")),
        ("auditionsLength", json!(60)),
        ("contentId", json!("999")),
        ("suffix", json!("mgm")),
        (
            "url",
            json!("http://dlsdownfree.nf.migu.cn/wlansst/song?pars=fixture"),
        ),
        (
            "url",
            json!("https://other.example/wlansst/song?pars=fixture"),
        ),
    ] {
        let mut frames = flag_one_frames("PQ", "020007", &plain());
        let mut body = flag_one_grant();
        body["data"][field] = value;
        frames[5] = reply(body, None).into_bytes();
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
    let mut corrupt = encrypted();
    corrupt[0] ^= 1;
    for bytes in [corrupt, encrypted()[..encrypted().len() - 1].to_vec()] {
        let mut frames = flag_one_frames("PQ", "020007", &plain());
        frames[MEDIA] = media_frame(&bytes, "");
        frames.truncate(MEDIA + 1);
        let mut h = binary_server(frames, None).await;
        let (store, original, alias) = setup(&mut h.provider, "caller");
        let failure = h
            .provider
            .audio_download_content(&track(), &request(alias))
            .await
            .unwrap_err();
        assert_eq!(failure.code, ErrorCode::UpstreamError);
        assert!(!format!("{failure:?}").contains(FILE_KEY));
        assert_eq!(read(&store, alias), original);
        assert_eq!(h.wire.await.unwrap().len(), 7);
    }
}

#[tokio::test]
async fn native_mg3d_flag_one_requires_native_final_uid_and_original_login_generation() {
    for at in [3, 7] {
        let mut frames = flag_one_frames("PQ", "020007", &plain());
        frames[at] = if at == 3 {
            validation("222")
        } else {
            profile("222", "")
        }
        .into_bytes();
        frames.truncate(at + 1);
        let mut h = binary_server(frames, None).await;
        let (_, _, alias) = setup(&mut h.provider, "caller");
        assert!(
            h.provider
                .audio_download_content(&track(), &request(alias))
                .await
                .is_err()
        );
        if at == 7 {
            assert!(h.provider.take_response_credential().unwrap().is_none());
        }
        assert_eq!(h.wire.await.unwrap().len(), at + 1);
    }
    for at in [5, MEDIA, 7] {
        let mut frames = flag_one_frames("PQ", "020007", &plain());
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
        let next = MiguCredential::verified("111".into(), "new-login-pacm".into()).unwrap();
        store.put(&stored(alias, &next)).unwrap();
        h.release.send(()).unwrap();
        assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
        assert_eq!(read(&store, alias), next);
        assert_eq!(h.wire.await.unwrap().len(), at + 1);
    }
}

#[tokio::test]
async fn native_mg3d_flag_one_never_exports_a_url_for_keyed_or_keyless_policy() {
    for key in [Some(json!(FILE_KEY)), None] {
        let mut frames = flag_one_frames("PQ", "020007", &plain());
        let mut body = flag_one_grant();
        body["data"].as_object_mut().unwrap().remove("fileKey");
        if let Some(key) = key {
            body["data"]["fileKey"] = key;
        }
        frames[5] = reply(body, None).into_bytes();
        frames.truncate(6);
        let mut h = binary_server(frames, None).await;
        let (_, _, alias) = setup(&mut h.provider, "named");
        assert_eq!(
            h.provider
                .download(&track(), &request(alias))
                .await
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        assert_eq!(h.wire.await.unwrap().len(), 6);
    }
}
