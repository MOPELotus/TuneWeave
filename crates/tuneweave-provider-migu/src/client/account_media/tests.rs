use super::*;

#[test]
fn account_media_rights_require_complete_exact_identity_and_booleans() {
    let valid = json!({"canListenRespItemList":[{"contentId":"123", "canListen":true,"limitLength":false}]});
    assert!(parse_rights(valid.clone(), "123").unwrap().can_listen);
    for value in [
        json!({}),
        json!({"canListenRespItemList":[]}),
        json!({"canListenRespItemList":[{"contentId":"123","canListen":true}]}),
        json!({"canListenRespItemList":[{"contentId":"123","canListen":true,"limitLength":true}]}),
        valid,
    ] {
        assert!(parse_rights(value, "999").is_err());
    }
}

fn track() -> Track {
    let mut track = Track::new(ResourceRef::new(Platform::Migu, "123").unwrap(), "Song");
    track.duration_ms = Some(212_000);
    track
        .extensions
        .insert("copyright_id".into(), json!("6005971HBUU"));
    track.extensions.insert("song_id".into(), json!("456"));
    track
}
fn playback() -> serde_json::Value {
    json!({"url":"https://freetyst.nf.migu.cn/public/product9th/product44/file.mp3?Tim=1&Key=synthetic&playSessionId=fixture",
        "audioFormatType":"PQ","freeListenType":"0",
        "song":{"contentId":"123","copyrightId":"6005971HBUU","songId":"456","resourceType":"2","duration":212}})
}
fn rights(trial: bool) -> Rights {
    Rights {
        content_id: "123".into(),
        can_listen: !trial,
        limit_length: trial,
    }
}
fn parse(value: serde_json::Value, trial: bool) -> Result<MediaStream> {
    parse_playback(
        value,
        &track(),
        &rights(trial),
        &StreamRequest::default(),
        "PQ",
        "secret-pacm-token",
    )
}

#[test]
fn account_media_exact_identity_actual_format_and_trial_bounds_are_required() {
    let full = parse(playback(), false).unwrap();
    assert_eq!(full.actual_quality, Quality::Standard);
    assert_eq!(full.bitrate, Some(128_000));
    assert!(full.headers.is_empty());
    assert!(full.expires_at.is_none());
    assert!(full.trial.is_none());
    let mut trial = playback();
    trial["auditionsStartTime"] = json!(65);
    trial["auditionsLength"] = json!(60);
    let stream = parse(trial.clone(), true).unwrap();
    assert_eq!(
        stream.trial,
        Some(TrialWindow {
            start_ms: 65000,
            end_ms: 125000
        })
    );
    assert_eq!(stream.duration_ms, Some(212000));
    assert!(parse(trial.clone(), false).is_err());
    for (key, value) in [
        ("auditionsLength", json!(0)),
        ("auditionsLength", json!(200)),
        ("auditionsStartTime", json!(u64::MAX)),
        ("auditionsLength", json!("bad")),
        ("auditionsStartTime", json!(null)),
    ] {
        let mut value2 = trial.clone();
        value2[key] = value;
        assert!(parse(value2, true).is_err(), "{key}");
    }
    for (key, value) in [
        ("contentId", json!("999")),
        ("copyrightId", json!("wrong")),
        ("resourceType", json!("D")),
        ("songId", json!("999")),
        ("duration", json!(213)),
        ("duration", json!(0)),
        ("duration", json!(null)),
    ] {
        let mut value2 = playback();
        value2["song"][key] = value;
        assert!(parse(value2, false).is_err(), "{key}");
    }
    for key in ["song", "audioFormatType"] {
        let mut value = playback();
        value.as_object_mut().unwrap().remove(key);
        assert!(parse(value, false).is_err());
    }
    trial["cannotCode"] = json!("440013");
    assert_eq!(
        parse(trial, true).unwrap_err().code,
        ErrorCode::PermissionDenied
    );
    let mut value = playback();
    value["audioFormatType"] = json!("HQ");
    assert_eq!(
        parse(value, false).unwrap_err().code,
        ErrorCode::PermissionDenied
    );
}

#[test]
fn account_media_signed_urls_cannot_export_tokens_or_untrusted_destinations() {
    for url in [
        "https://attacker.example/file.mp3?Tim=1&Key=a&playSessionId=b",
        "http://freetyst.nf.migu.cn/public/product9th/product1/a.mp3?Tim=1&Key=a&playSessionId=b",
        "https://freetyst.nf.migu.cn/public/product9th/product1/a.mp3?Tim=1&Tim=2&Key=a&playSessionId=b",
        "https://freetyst.nf.migu.cn/public/product9th/product1/a.mp3?Tim=1&Key=secret-pacm-token&playSessionId=b",
        "https://freetyst.nf.migu.cn/public/product9th/product1/a.mp3?Tim=1&Key=%73ecret-pacm-token&playSessionId=b",
        "https://freetyst.nf.migu.cn/public/product9th/product1/%73ecret-pacm-token.mp3?Tim=1&Key=a&playSessionId=b",
        "https://freetyst.nf.migu.cn/public/product9th/product1/a.flac?Tim=1&Key=a&playSessionId=b",
        "",
    ] {
        let mut value = playback();
        value["url"] = json!(url);
        let failure = parse(value, false).unwrap_err();
        assert!(!format!("{failure:?}").contains("secret-pacm-token"));
    }
}

#[test]
fn account_media_pc_envelope_codec_requires_consistent_signature_magic_and_json() {
    let json = br#"{"code":"000000","data":{"value":"fixture"}}"#;
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    assert_eq!(
        decode_envelope(json, &headers, true).unwrap()["value"],
        "fixture"
    );
    let mut bytes = vec![0xab, 0xcd, 0x01, 13];
    // Independent encoder for the public SDK's JSON envelope codec.
    let key = b"Jk8qzuePiJ1qE3mDYhLQ3T73DtDoAhLP";
    bytes.extend(
        json.iter()
            .enumerate()
            .map(|(i, b)| b.wrapping_add(key[i % key.len()]).wrapping_sub(13)),
    );
    headers.insert("signature", HeaderValue::from_static("1"));
    assert_eq!(
        decode_envelope(&bytes, &headers, true).unwrap()["value"],
        "fixture"
    );
    assert!(decode_envelope(&bytes, &headers, false).is_err());
    assert!(decode_envelope(json, &headers, true).is_err());
    headers.append("signature", HeaderValue::from_static("1"));
    assert!(decode_envelope(&bytes, &headers, true).is_err());
    headers.remove("signature");
    for body in [
        b"<html>login</html>".as_slice(),
        b"{}",
        br#"{"code":"000000"}"#,
        br#"{"code":"290001","data":{}}"#,
        &bytes,
    ] {
        assert_eq!(
            decode_envelope(body, &headers, true).unwrap_err().code,
            ErrorCode::UpstreamError
        );
    }
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("text/html"));
    assert!(decode_envelope(json, &headers, true).is_err());
}

#[test]
fn account_media_incomplete_grants_are_errors_and_not_quality_fallback_denials() {
    for kind in [
        "missing-url",
        "empty-url",
        "invalid-timing",
        "duplicate-empty-signature",
    ] {
        let mut value = playback();
        match kind {
            "missing-url" => {
                value.as_object_mut().unwrap().remove("url");
            }
            "empty-url" => value["url"] = json!(""),
            "invalid-timing" => {
                value["auditionsStartTime"] = json!("bad");
                value["auditionsLength"] = json!("bad");
            }
            _ => value["url"] = json!(format!("{}&Key=", value["url"].as_str().unwrap())),
        }
        assert_eq!(
            parse(value, false).unwrap_err().code,
            ErrorCode::UpstreamError,
            "{kind}"
        );
    }
    for value in [
        json!({"canListenRespItemList":[{"contentId":"123","canListen":true}]}),
        json!({"canListenRespItemList":[{"contentId":"123","canListen":true,"limitLength":true}]}),
        json!({"canListenRespItemList":[{"contentId":"123","canListen":"true","limitLength":false}]}),
    ] {
        assert!(parse_rights(value, "123").is_err());
    }
}
