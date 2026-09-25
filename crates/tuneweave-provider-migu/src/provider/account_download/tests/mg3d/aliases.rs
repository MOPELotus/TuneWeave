use super::*;

// All bytes are synthetic complete Layer III frames. The two request aliases
// still select the existing PQ/HQ rendition; they do not define new formats.
fn alias_frames(tone: &str, keyed: bool) -> (Vec<Vec<u8>>, Vec<u8>) {
    let (format, clear) = match tone {
        "PQ" => ("020007", plain()),
        "HQ" => {
            let mut frame = vec![0; 1044];
            frame[..4].copy_from_slice(&[0xff, 0xfb, 0xe0, 0]);
            ("020009", frame.repeat(40))
        }
        _ => panic!("unsupported fixture tone"),
    };
    let mut frames = responses();
    let response = String::from_utf8(frames[4].clone()).unwrap();
    let mut detail: serde_json::Value =
        serde_json::from_str(response.split_once("\r\n\r\n").unwrap().1).unwrap();
    detail["resource"][0]["rateFormats"][0]["formatType"] = json!(tone);
    detail["resource"][0]["rateFormats"][0]["format"] = json!(format);
    frames[4] = reply(detail, None).into_bytes();
    let mut body = grant();
    body["data"]["formatId"] = json!(format);
    body["data"]["size"] = json!(clear.len());
    if !keyed {
        body["data"].as_object_mut().unwrap().remove("fileKey");
    }
    frames[5] = reply(body, None).into_bytes();
    let media = if keyed {
        let key = b"CB4E917FFEB2B4A056445F4B3544495E";
        clear
            .iter()
            .enumerate()
            .map(|(i, value)| (*value).wrapping_add(key[i % 32]))
            .collect::<Vec<_>>()
    } else {
        clear.clone()
    };
    frames[MEDIA] = media_frame(&media, "");
    (frames, clear)
}

#[tokio::test]
async fn native_download_quality_aliases_keep_selected_accounts_and_exact_authorized_formats() {
    for (quality, tone) in [(Quality::Low, "PQ"), (Quality::Higher, "HQ")] {
        for keyed in [false, true] {
            for mode in ["default", "named", "caller"] {
                let (frames, clear) = alias_frames(tone, keyed);
                let mut h = binary_server(frames, None).await;
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
                assert_eq!(result.content_type, "audio/mpeg");
                assert_eq!(result.filename, "123.mp3");
                assert!(result.trial.is_none());
                assert_eq!(read(&store, "other").token(), "unrelated-pacm");
                if mode == "caller" {
                    assert_eq!(read(&store, alias), original);
                    let rotated = h.provider.take_response_credential().unwrap().unwrap();
                    assert_eq!(
                        MiguCredential::parse_caller(&rotated).unwrap().token(),
                        "final-pacm"
                    );
                    for secret in [FILE_KEY, "native-token-fixture", "do-not-retain-session"] {
                        assert!(!rotated.secret().contains(secret));
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
                    "pacmtoken",
                    "cookie:",
                    "token:",
                    "ce:",
                    "sign:",
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
                        .all(|r| !r.contains("/listen") && !r.contains("can-listen"))
                );
            }
        }
    }
}

#[tokio::test]
async fn native_download_quality_aliases_preserve_explicit_bitrate_selection() {
    for (quality, bitrate, tone) in [
        (Quality::Low, 320_000, "HQ"),
        (Quality::Higher, 128_000, "PQ"),
    ] {
        for keyed in [false, true] {
            let (frames, clear) = alias_frames(tone, keyed);
            let mut h = binary_server(frames, None).await;
            let (_, _, alias) = setup(&mut h.provider, "named");
            let result = h
                .provider
                .audio_download_content(
                    &track(),
                    &StreamRequest {
                        quality,
                        bitrate: Some(bitrate),
                        ..request(alias)
                    },
                )
                .await
                .unwrap();
            assert_eq!(result.bytes, clear);
            let wire = h.wire.await.unwrap();
            assert_eq!(wire.len(), 8);
            assert!(wire[5].contains(&format!("formatType={tone}&")));
        }
    }
}

#[tokio::test]
async fn native_download_quality_aliases_never_fall_back_after_denial_or_bad_grant() {
    for (quality, tone) in [(Quality::Low, "PQ"), (Quality::Higher, "HQ")] {
        for denied in [true, false] {
            let (mut frames, _) = alias_frames(tone, true);
            frames[5] = if denied {
                reply(json!({"code":"200010"}), None).into_bytes()
            } else {
                let mut body = grant();
                body["data"]["formatId"] = json!("unknown-format");
                reply(body, None).into_bytes()
            };
            frames.truncate(6);
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
            assert_eq!(
                failure.code,
                if denied {
                    ErrorCode::PermissionDenied
                } else {
                    ErrorCode::UpstreamError
                }
            );
            assert_eq!(read(&store, alias), original);
            assert!(!format!("{failure:?}").contains(FILE_KEY));
            let wire = h.wire.await.unwrap();
            assert_eq!(wire.len(), 6);
            assert!(wire[5].contains(&format!("formatType={tone}&")));
        }
    }
}

#[tokio::test]
async fn native_download_quality_aliases_preserve_original_generation_at_transfer_boundaries() {
    for keyed in [false, true] {
        for at in [5, MEDIA, 7] {
            let (mut frames, _) = alias_frames("HQ", keyed);
            frames.truncate(at + 1);
            let mut h = binary_server(frames, Some(at)).await;
            let (store, _, alias) = setup(&mut h.provider, "named");
            let provider = h.provider.clone();
            let task = tokio::spawn(async move {
                provider
                    .audio_download_content(
                        &track(),
                        &StreamRequest {
                            quality: Quality::Higher,
                            ..request(alias)
                        },
                    )
                    .await
            });
            tokio::time::timeout(Duration::from_secs(5), h.seen)
                .await
                .unwrap()
                .unwrap();
            let replacement =
                MiguCredential::verified("111".into(), "new-login-pacm".into()).unwrap();
            store.put(&stored(alias, &replacement)).unwrap();
            h.release.send(()).unwrap();
            assert_eq!(task.await.unwrap().unwrap_err().code, ErrorCode::Conflict);
            assert_eq!(read(&store, alias), replacement);
            assert_eq!(h.wire.await.unwrap().len(), at + 1);
        }
    }
}
