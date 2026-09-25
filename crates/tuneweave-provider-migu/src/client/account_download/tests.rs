use super::*;

fn track() -> Track {
    let mut track = Track::new(ResourceRef::new(Platform::Migu, "123").unwrap(), "Song");
    track.extensions.insert("song_id".into(), json!("456"));
    track
        .extensions
        .insert("copyright_id".into(), json!("6005971HBUU"));
    track.extensions.insert(
        "rate_formats".into(),
        json!([{"formatType":"PQ","format":"020007"}]),
    );
    track
}
fn data() -> serde_json::Value {
    json!({"contentId":"123","copyrightId":"6005971HBUU","formatId":"020007","encryptionType":"0","size":"3450883","suffix":"mp3","url":"https://dlsdownfree.nf.migu.cn/wlansst/song?pars=fixture"})
}

#[test]
fn native_download_requires_identity_cleartext_and_exact_requested_format() {
    let parsed = parse_download(data(), &track(), &StreamRequest::default(), "PQ").unwrap();
    assert!(parsed.available);
    assert_eq!(parsed.actual_quality, Quality::Standard);
    assert_eq!(parsed.size, Some(3450883));
    for (key, value) in [
        ("contentId", json!("other")),
        ("copyrightId", json!("other")),
        ("songId", json!("other")),
        ("formatId", json!("020010")),
        ("encryptionType", json!("1")),
        ("encryptionType", json!("")),
        ("fileKey", json!("encrypted-key")),
        ("size", json!("invalid")),
        ("size", json!(0)),
        ("suffix", json!("flac")),
        ("auditionsLength", json!(60)),
        ("auditionsStartTime", json!(0)),
        (
            "url",
            json!("https://attacker.example/wlansst/song?pars=fixture"),
        ),
        ("url", json!("https://dlsdownfree.nf.migu.cn/wlansst/song")),
    ] {
        let mut candidate = data();
        candidate[key] = value;
        assert!(
            parse_download(candidate, &track(), &StreamRequest::default(), "PQ").is_err(),
            "{key}"
        );
    }
    assert!(parse_download(data(), &track(), &StreamRequest::default(), "HQ").is_err());
    let mut ambiguous = track();
    ambiguous.extensions.insert(
        "new_rate_formats".into(),
        json!([{"formatType":"HQ","format":"020007"}]),
    );
    assert!(parse_download(data(), &ambiguous, &StreamRequest::default(), "PQ").is_err());
}

#[test]
fn native_clear_content_accepts_only_empty_keys_and_explicit_plain_encryption_policy() {
    for key in [None, Some(json!(null)), Some(json!(""))] {
        let mut value = data();
        if let Some(key) = key {
            value["fileKey"] = key;
        }
        let grant =
            parse_download_content(value.clone(), &track(), &StreamRequest::default(), "PQ")
                .unwrap();
        assert!(grant.key.is_none());
        assert_eq!(grant.media.format.as_deref(), Some("mp3"));
        assert_eq!(grant.media.actual_quality, Quality::Standard);
        assert!(parse_download(value, &track(), &StreamRequest::default(), "PQ").is_ok());
    }
    for (field, value) in [
        ("fileKey", json!(" ")),
        ("fileKey", json!(0)),
        ("fileKey", json!({})),
        ("fileKey", json!("short")),
        ("fileKey", json!("z".repeat(32))),
        ("encryptionType", json!(null)),
        ("encryptionType", json!(0)),
        ("encryptionType", json!("1")),
        ("encryptionType", json!("2")),
    ] {
        let mut bad = data();
        bad[field] = value;
        assert!(
            parse_download_content(bad, &track(), &StreamRequest::default(), "PQ").is_err(),
            "{field}"
        );
    }
    let mut missing = data();
    missing.as_object_mut().unwrap().remove("encryptionType");
    assert!(parse_download_content(missing, &track(), &StreamRequest::default(), "PQ").is_err());
}

#[test]
fn native_clear_content_never_chooses_a_codec_from_an_unverified_suffix() {
    for (tone, format, suffix, quality) in [
        ("PQ", "020007", "mp3", Quality::Standard),
        ("HQ", "020008", "mp3", Quality::High),
        ("SQ", "020010", "flac", Quality::Lossless),
        ("ZQ24", "011005", "flac", Quality::Hires),
    ] {
        let mut fresh = track();
        fresh.extensions.insert(
            "rate_formats".into(),
            json!([{"formatType":tone,"format":format}]),
        );
        let mut value = data();
        value["formatId"] = json!(format);
        value["suffix"] = json!(suffix);
        let request = StreamRequest {
            quality,
            ..StreamRequest::default()
        };
        let grant = parse_download_content(value.clone(), &fresh, &request, tone).unwrap();
        assert!(grant.key.is_none());
        assert_eq!(grant.media.actual_quality, quality);
        for wrong in [
            "wav",
            "aac",
            "mgm",
            "MG3D",
            if suffix == "mp3" { "flac" } else { "mp3" },
        ] {
            let mut bad = value.clone();
            bad["suffix"] = json!(wrong);
            assert!(parse_download_content(bad, &fresh, &request, tone).is_err());
        }
        value["formatId"] = json!("unlisted");
        assert!(parse_download_content(value, &fresh, &request, tone).is_err());
    }
    assert!(parse_download_content(data(), &track(), &StreamRequest::default(), "ZQ").is_err());
}

#[test]
fn mg3d_download_accepts_zq24_only_from_fresh_format_metadata_as_hires_flac() {
    let mut fresh = track();
    fresh.extensions.insert(
        "rate_formats".into(),
        json!([{"formatType":"ZQ24","format":"011005"}]),
    );
    let mut value = data();
    value["formatId"] = json!("011005");
    value["suffix"] = json!("flac");
    value["fileKey"] = json!("0123456789abcdef0123456789abcdef");
    let request = StreamRequest {
        quality: Quality::Hires,
        ..StreamRequest::default()
    };
    let grant = parse_download_content(value.clone(), &fresh, &request, "ZQ24").unwrap();
    assert_eq!(grant.media.actual_quality, Quality::Hires);
    assert_eq!(grant.media.format.as_deref(), Some("flac"));
    assert_eq!(
        grant.key.as_ref(),
        Some(b"0123456789abcdef0123456789abcdef")
    );

    for (field, replacement) in [
        ("formatId", json!("020010")),
        ("formatId", json!("unlisted")),
        ("suffix", json!("mp3")),
        ("encryptionType", json!("2")),
        ("auditionsStartTime", json!(0)),
    ] {
        let mut bad = value.clone();
        bad[field] = replacement;
        assert!(
            parse_download_content(bad, &fresh, &request, "ZQ24").is_err(),
            "{field}"
        );
    }
    assert!(parse_download_content(value, &fresh, &request, "SQ").is_err());
}

#[test]
fn native_mg3d_flag_one_requires_a_real_key_and_keeps_url_download_denied() {
    for (tone, format, suffix, quality) in [
        ("PQ", "020007", "mp3", Quality::Standard),
        ("HQ", "020009", "mp3", Quality::High),
        ("SQ", "020010", "flac", Quality::Lossless),
        ("ZQ24", "011005", "flac", Quality::Hires),
    ] {
        let mut fresh = track();
        fresh.extensions.insert(
            "rate_formats".into(),
            json!([{"formatType":tone,"format":format}]),
        );
        let mut value = data();
        value["encryptionType"] = json!("1");
        value["fileKey"] = json!("0123456789aBCDEF0123456789aBCDEF");
        value["formatId"] = json!(format);
        value["suffix"] = json!(suffix);
        let request = StreamRequest {
            quality,
            ..Default::default()
        };
        let parsed = parse_download_content(value.clone(), &fresh, &request, tone).unwrap();
        assert_eq!(
            parsed.key.as_ref(),
            Some(b"0123456789aBCDEF0123456789aBCDEF")
        );
        assert_eq!(parsed.media.actual_quality, quality);
        assert_eq!(parsed.media.format.as_deref(), Some(suffix));
        assert_eq!(
            parse_download(value.clone(), &fresh, &request, tone)
                .unwrap_err()
                .code,
            ErrorCode::PermissionDenied
        );
        for key in [None, Some(json!(null)), Some(json!(""))] {
            let mut missing_key = value.clone();
            missing_key.as_object_mut().unwrap().remove("fileKey");
            if let Some(key) = key {
                missing_key["fileKey"] = key;
            }
            assert!(parse_download_content(missing_key, &fresh, &request, tone).is_err());
        }
    }
}

#[test]
fn native_mg3d_flag_one_does_not_relax_encryption_identity_or_preview_validation() {
    let mut value = data();
    value["encryptionType"] = json!("1");
    value["fileKey"] = json!("0123456789abcdef0123456789abcdef");
    for (field, replacement) in [
        ("encryptionType", json!("2")),
        ("encryptionType", json!("01")),
        ("encryptionType", json!("true")),
        ("encryptionType", json!("")),
        ("encryptionType", json!(1)),
        ("encryptionType", json!(null)),
        ("fileKey", json!("short")),
        ("fileKey", json!("z".repeat(32))),
        ("fileKey", json!(0)),
        ("contentId", json!("999")),
        ("copyrightId", json!("other")),
        ("songId", json!("other")),
        ("formatId", json!("unlisted")),
        ("suffix", json!("mgm")),
        ("auditionsStartTime", json!(0)),
        ("auditionsLength", json!(60)),
    ] {
        let mut bad = value.clone();
        bad[field] = replacement;
        assert!(
            parse_download_content(bad, &track(), &StreamRequest::default(), "PQ").is_err(),
            "{field}"
        );
    }
    value.as_object_mut().unwrap().remove("encryptionType");
    assert!(parse_download_content(value, &track(), &StreamRequest::default(), "PQ").is_err());
}

#[test]
fn native_errors_and_ephemeral_auth_never_reflect_tokens_or_messages() {
    let auth = NativeAuthorization {
        token: "fixture-secret".into(),
        uid: "111".into(),
    };
    assert!(!format!("{auth:?}").contains("fixture-secret"));
    for code in ["200010", "290001", "unknown-fixture-secret"] {
        let failure = success_data(
            json!({"code":code,"info":"fixture-secret","data":{"url":"fixture-secret"}}),
        )
        .unwrap_err();
        assert!(!failure.message.contains("fixture-secret"));
        assert!(
            !serde_json::to_string(&failure.details)
                .unwrap()
                .contains("fixture-secret")
        );
    }
}

#[test]
fn native_download_url_requires_https_exact_cdn_path_and_one_authorization() {
    assert!(validate_download_url("https://dlsdownfree.nf.migu.cn/wlansst?pars=fixture").is_ok());
    assert!(
        validate_download_url("https://dlsdownfree.nf.migu.cn/wlansst/song?pars=fixture").is_ok()
    );
    for url in [
        "http://dlsdownfree.nf.migu.cn/wlansst?pars=fixture",
        "https://attacker.example/wlansst?pars=fixture",
        "https://dlsdownfree.nf.migu.cn.attacker.example/wlansst?pars=fixture",
        "https://dlsdownfree.nf.migu.cn/other?pars=fixture",
        "https://dlsdownfree.nf.migu.cn/wlansst-other?pars=fixture",
        "https://dlsdownfree.nf.migu.cn/wlansst",
        "https://dlsdownfree.nf.migu.cn/wlansst?pars=",
        "https://dlsdownfree.nf.migu.cn/wlansst?pars=fixture&pars=other",
        "https://dlsdownfree.nf.migu.cn/wlansst?pars=fixture&pars=",
        "https://dlsdownfree.nf.migu.cn/wlansst?pars=fixture&%70ars=",
        "https://user@dlsdownfree.nf.migu.cn/wlansst?pars=fixture",
        "https://dlsdownfree.nf.migu.cn:8080/wlansst?pars=fixture",
        "https://dlsdownfree.nf.migu.cn/wlansst?pars=fixture#fragment",
    ] {
        assert!(validate_download_url(url).is_err(), "{url}");
    }
}
