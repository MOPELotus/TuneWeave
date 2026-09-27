use super::*;
use crate::account::media::tests::tracker_response;

fn track() -> Track {
    let mut track = Track::new(ResourceRef::new(Platform::Kugou, "901").unwrap(), "Song");
    track.duration_ms = Some(123456);
    track.extensions.insert("qualities".into(),json!({"standard":{"hash":"abcdef0123456789abcdef0123456789","format":"mp3","bitrate":128,"duration_ms":123456}}));
    track
}
fn tracker() -> Value {
    json!({"status":1,"hash":"abcdef0123456789abcdef0123456789","url":["https://fs.kugou.com/audio.mp3"],
        "backupUrl":["https://fs.kugou.com/audio.mp3","http://fs2.kugou.com/audio.mp3"],"fileSize":"1975000","bitRate":128000,"timeLength":123,"extName":"mp3"})
}
fn mapped(body: Value, behavior: Behavior) -> Result<MediaStream> {
    Selection::new(track(), &StreamRequest::default())?.map(tracker_response(body), behavior)
}

#[cfg(debug_assertions)]
#[test]
fn tracker_shape_failure_fixture_exposes_types_without_values() {
    let mut body = tracker();
    body["fileSize"] = json!({"private_field": "fixture-secret-marker"});
    body["url"] = json!(["https://fixture.invalid/audio?token=fixture-url-marker"]);

    let bytes = body.to_string().into_bytes();
    let summary = safe_tracker_shape(&bytes).to_string();
    assert!(summary.contains("\"fileSize\":\"object\""));
    assert!(summary.contains("\"url\":\"array\""));
    assert!(!summary.contains("fixture-secret-marker"));
    assert!(!summary.contains("fixture-url-marker"));
    assert!(!summary.contains("fixture.invalid"));
    assert!(mapped(body, Behavior::Play).is_err());
}

#[test]
fn authorized_media_reports_actual_file_metadata_and_deduplicates_only_trusted_urls() {
    let stream = mapped(tracker(), Behavior::Play).unwrap();
    assert_eq!(stream.duration_ms, Some(123000));
    assert_eq!(stream.size, Some(1975000));
    assert_eq!(stream.actual_quality, Quality::Standard);
    assert_eq!(stream.backup_urls, ["https://fs2.kugou.com/audio.mp3"]);
    assert!(stream.trial.is_none() && stream.headers.is_empty() && stream.expires_at.is_none());
    let mut body = tracker();
    body["url"] = json!("https://fs.kugou.com/audio.mp3");
    body["extName"] = json!("ogg");
    body["bitRate"] = json!(64000);
    let stream = mapped(body, Behavior::Play).unwrap();
    assert_eq!(stream.actual_quality, Quality::Low);
    assert_eq!(stream.format.as_deref(), Some("ogg"));
    assert!(stream.codec.is_none());
}

#[test]
fn authorized_trials_cannot_be_exposed_as_full_downloads_or_silent_short_files() {
    let mut body = tracker();
    body["timeLength"] = json!(60);
    assert!(mapped(body.clone(), Behavior::Play).is_err());
    body["hash_offset"] =
        json!({"start_ms":30000,"end_ms":90000,"start_byte":10,"end_byte":960009});
    let stream = mapped(body.clone(), Behavior::Play).unwrap();
    assert_eq!(
        stream.trial,
        Some(TrialWindow {
            start_ms: 30000,
            end_ms: 90000
        })
    );
    assert_eq!(
        mapped(body.clone(), Behavior::Download).unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    for offset in [
        json!({"end_ms":60000}),
        json!({"start_ms":90000,"end_ms":30000}),
        json!({"start_ms":0,"end_ms":1234570}),
        json!({"start_ms":0,"end_ms":60000,"start_byte":1}),
    ] {
        body["hash_offset"] = offset;
        assert!(mapped(body.clone(), Behavior::Play).is_err());
    }
}

#[test]
fn authorized_media_rejects_wrong_identity_unsupported_encryption_and_secret_echoes() {
    for (key, value) in [
        ("hash", json!("ffffffffffffffffffffffffffffffff")),
        ("album_audio_id", json!(902)),
        ("album_id", json!(70)),
        ("extName", json!("kgm")),
        ("fileSize", json!(0)),
        ("timeLength", json!(0)),
        ("bitRate", json!(320000)),
        ("fail_process", json!(["need_pay"])),
        ("url", json!(["https://example.invalid/audio.mp3"])),
        (
            "url",
            json!(["https://fs.kugou.com/a?auth=synthetic-song-auth"]),
        ),
        (
            "url",
            json!(["https://fs.kugou.com/a?auth=synthetic%2Duser%2Dauth"]),
        ),
        (
            "backupUrl",
            json!(["https://fs.kugou.com/a?token=synthetic-login-secret"]),
        ),
    ] {
        let mut body = tracker();
        body[key] = value;
        assert!(mapped(body, Behavior::Play).is_err(), "{key}");
    }
    let mut body = tracker();
    body["status"] = json!(2);
    let error = mapped(body, Behavior::Play).unwrap_err();
    assert_eq!(error.code, ErrorCode::PermissionDenied);
    assert!(!format!("{error:?}").contains("fs.kugou.com"));
}

#[test]
fn standard_media_rejects_catalogue_std_hash_when_response_hash_differs() {
    let mut body = tracker();
    body["hash"] = json!("ffffffffffffffffffffffffffffffff");
    body["std_hash"] = json!("abcdef0123456789abcdef0123456789");
    // This field is not the media duration; timeLength remains the independent
    // duration check for the returned file.
    body["std_hash_time"] = json!(42);

    assert!(mapped(body, Behavior::Play).is_err());
}

#[test]
fn std_hash_alias_is_rejected_for_nonstandard_assets() {
    let mut track = track();
    track.extensions.get_mut("qualities").unwrap()["high"] = json!({
        "hash":"fedcba9876543210fedcba9876543210",
        "format":"mp3",
        "bitrate":320,
        "duration_ms":123456
    });
    let request = StreamRequest {
        quality: Quality::High,
        ..Default::default()
    };
    let mut body = tracker();
    body["hash"] = json!("ffffffffffffffffffffffffffffffff");
    body["std_hash"] = json!("fedcba9876543210fedcba9876543210");

    let error = Selection::new(track, &request)
        .unwrap()
        .map(tracker_response(body), Behavior::Play)
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::UpstreamError);
}

#[test]
fn lossless_selection_never_labels_a_compressed_response_as_lossless() {
    let mut track = track();
    track.extensions.get_mut("qualities").unwrap()["lossless"] =
        json!({"hash":"abcdef0123456789abcdef0123456789","format":"flac","duration_ms":123456});
    let request = StreamRequest {
        quality: Quality::Lossless,
        ..Default::default()
    };
    let stream = Selection::new(track.clone(), &request)
        .unwrap()
        .map(tracker_response(tracker()), Behavior::Play)
        .unwrap();
    assert_eq!(stream.requested_quality, Quality::Lossless);
    assert_eq!(stream.actual_quality, Quality::Standard);
    let mut body = tracker();
    body["extName"] = json!("flac");
    body["bitRate"] = json!(900000);
    let stream = Selection::new(track, &request)
        .unwrap()
        .map(tracker_response(body), Behavior::Download)
        .unwrap();
    assert_eq!(stream.actual_quality, Quality::Lossless);
    assert_eq!(stream.codec.as_deref(), Some("flac"));
}

#[test]
fn vinyl_is_rejected_before_bitrate_override_for_public_and_account_media() {
    for bitrate in [None, Some(128000)] {
        let request = StreamRequest {
            quality: Quality::Vinyl,
            bitrate,
            ..Default::default()
        };
        assert_eq!(
            validate_request(&request).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            select_media_spec(&track(), &request).err().unwrap().code,
            ErrorCode::InvalidRequest
        );
    }
}

#[test]
fn dtsx_is_rejected_before_bitrate_override_for_public_and_account_media() {
    for bitrate in [None, Some(128000), Some(320000)] {
        let request = StreamRequest {
            quality: Quality::Dtsx,
            bitrate,
            ..Default::default()
        };
        assert_eq!(
            validate_request(&request).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert!(matches!(
            select_media_spec(&track(), &request),
            Err(error) if error.code == ErrorCode::InvalidRequest
        ));
    }
}

#[test]
fn sing_along_is_rejected_for_public_and_native_or_web_account_media() {
    for bitrate in [None, Some(128000)] {
        let request = StreamRequest {
            variant: StreamVariant::SingAlong,
            bitrate,
            ..Default::default()
        };
        assert_eq!(
            validate_request(&request).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            select_media_spec(&track(), &request).err().unwrap().code,
            ErrorCode::InvalidRequest
        );
    }
}
