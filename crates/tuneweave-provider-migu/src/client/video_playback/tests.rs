use super::*;
pub(crate) fn resource() -> Value {
    let mut v = super::super::videos::tests::detail("7");
    let r = &mut v["resource"][0];
    r["copyright"] = json!("1");
    r["migumvDuration"] = json!("00:00:08");
    r["rateFormats"] = json!([{"resourceType":"D","formatType":"PQ","format":"050019","fileType":"mp4","size":"1000","url":"/opaque%2Bvalue/file.mp4?F=050019"},{"resourceType":"D","formatType":"HQ","format":"050012","fileType":"mp4","size":"2000","url":"/opaque%2Bvalue/file.mp4?F=050012"},{"resourceType":"D","formatType":"SQ","format":"050015","fileType":"mp4","size":"3000","url":"/opaque%2Bvalue/file.mp4?F=050015"}]);
    v
}
pub(crate) fn grant() -> Value {
    json!({"code":"000000","bizcode":"unrelated-upstream-code","playUrl":"https://freevod.nf.migu.cn/opaque/index.m3u8?playSessionId=fixture&resourceId=7&resourceType=D","order":"0","allowtimes":"0"})
}
pub(crate) const MANIFEST: &str = "#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\nseg-1-v1-a1.ts\n#EXTINF:4,\nseg-2-v1-a1.ts\n#EXT-X-ENDLIST\n";
fn request(resolution: u32) -> VideoStreamRequest {
    VideoStreamRequest::new(VideoResourceKind::Mv, resolution)
}
#[test]
fn mv_play_source_uses_official_default_format_order_without_inventing_pixel_dimensions() {
    let a = parse_source(resource(), "7", &request(0)).unwrap();
    assert_eq!(a.level, "PQ");
    assert_eq!(a.duration_ms, 8000);
    assert_eq!(
        parse_source(resource(), "7", &request(1080)).unwrap().level,
        "SQ"
    );
    let mut r = resource();
    r["resource"][0]["rateFormats"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    assert_eq!(parse_source(r, "7", &request(0)).unwrap().level, "HQ");
    for case in 0..6 {
        let mut v = resource();
        let r = &mut v["resource"][0];
        match case {
            0 => r["copyright"] = json!("0"),
            1 => r["contentId"] = json!("8"),
            2 => r["rateFormats"][0]["url"] = json!("bad\nurl"),
            3 => r["rateFormats"][0]["size"] = json!("0"),
            4 => r["rateFormats"][1]["formatType"] = json!("PQ"),
            _ => r["migumvDuration"] = Value::Null,
        }
        assert!(parse_source(v, "7", &request(0)).is_err());
    }
    for resolution in [1, 360, 480, 720, 2160, u32::MAX] {
        assert!(validate("7", &request(resolution)).is_err());
    }
}
#[test]
fn mv_play_grants_require_real_https_and_bound_resources_and_never_upgrade_or_ignore_denial() {
    let source = parse_source(resource(), "7", &request(0)).unwrap();
    assert!(parse_grant(grant(), &source).is_ok());
    for case in 0..10 {
        let mut g = grant();
        match case {
            0 => g["code"] = json!("000001"),
            1 => g["code"] = json!("unknown"),
            2 => g["cannotType"] = json!("needLogin"),
            3 => g["offset"] = json!(3000),
            4 => {
                g["playUrl"] = json!(
                    "http://freevod.nf.migu.cn:8080/opaque/index.m3u8?playSessionId=x&resourceId=7&resourceType=D"
                )
            }
            5 => {
                g["playUrl"] = json!(
                    g["playUrl"]
                        .as_str()
                        .unwrap()
                        .replace("resourceId=7", "resourceId=8")
                )
            }
            6 => g["playUrl"] = json!(format!("{}&resourceId=7", g["playUrl"].as_str().unwrap())),
            7 => {
                g["playUrl"] = json!(
                    g["playUrl"]
                        .as_str()
                        .unwrap()
                        .replace("freevod.nf.migu.cn", "evil.invalid")
                )
            }
            8 => g["playUrl"] = json!(format!("{}#secret", g["playUrl"].as_str().unwrap())),
            _ => g["playUrl"] = Value::Null,
        }
        assert!(parse_grant(g, &source).is_err(), "{case}");
    }
}
#[test]
fn mv_play_manifest_durations_are_exact_bounded_and_do_not_authorize_short_or_external_content() {
    let s = parse_source(resource(), "7", &request(0)).unwrap();
    assert_eq!(
        manifest_duration(MANIFEST.as_bytes(), s.duration_ms).unwrap(),
        8000
    );
    let zero = MANIFEST.replace(
        "#EXT-X-ENDLIST",
        "#EXTINF:0,\nseg-3-v1-a1.ts\n#EXT-X-ENDLIST",
    );
    assert_eq!(
        manifest_duration(zero.as_bytes(), s.duration_ms).unwrap(),
        8000
    );
    let decimal = MANIFEST.replace("#EXTINF:4,", "#EXTINF:4.000500,");
    assert_eq!(
        manifest_duration(decimal.as_bytes(), s.duration_ms).unwrap(),
        8001
    );
    for bad in [
        MANIFEST.replace("#EXT-X-ENDLIST", ""),
        MANIFEST.replace("#EXTINF:4,", "#EXTINF:1,"),
        MANIFEST.replace("seg-1-v1-a1.ts", "https://evil.invalid/x.ts"),
        MANIFEST.replace("seg-1-v1-a1.ts", "../x.ts"),
        MANIFEST.replace("#EXTINF:4,", "#EXTINF:NaN,"),
        MANIFEST.replace(
            "#EXTM3U",
            "#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"https://evil.invalid/key\"",
        ),
        MANIFEST.replace("#EXT-X-ENDLIST", "#EXT-X-ENDLIST\n#EXTINF:1,\nx.ts"),
        MANIFEST.replace("#EXTINF:4,", "#EXTINF:0,"),
    ] {
        assert!(manifest_duration(bad.as_bytes(), s.duration_ms).is_err());
    }
}

fn native_grant(format: &str, offset: Value, background_duration: Value) -> Value {
    json!({
        "code":"000000",
        "info":"操作成功",
        "data":{
            "playUrl":"http://freevod.nf.migu.cn:8080/hls/v2/opaque-grant/index.m3u8?playSessionId=session&resourceId=7&resourceType=D&userId=opaque",
            "formatType":format,
            "offset":offset,
            "backgroundDuration":background_duration,
            "vipResolution":false,
            "uhdVip":false
        }
    })
}

#[test]
fn native_mv_grants_keep_format_offset_and_background_policy_explicit() {
    let source = parse_source(resource(), "7", &request(0)).unwrap();
    let grant = parse_native_grant(
        native_grant("HQ", json!(3000), json!(0)),
        "7",
        source.duration_ms,
        MiguNativeMvFormat::Auto,
    )
    .unwrap();
    assert_eq!(grant.format, "HQ");
    assert_eq!(grant.offset_ms, 3000);
    assert_eq!(url::Url::parse(&grant.url).unwrap().scheme(), "http");
    assert!(
        parse_native_grant(
            native_grant("HQ", json!(3000), json!(0)),
            "7",
            source.duration_ms,
            MiguNativeMvFormat::Hq
        )
        .is_ok()
    );
    assert!(
        parse_native_grant(
            native_grant("HQ", json!(3000), json!(0)),
            "7",
            source.duration_ms,
            MiguNativeMvFormat::Pq
        )
        .is_err()
    );
}

#[test]
fn native_mv_grants_reject_refusals_missing_policies_and_unbound_urls() {
    let source = parse_source(resource(), "7", &request(0)).unwrap();
    let mut cannot_play = native_grant("PQ", json!(3000), json!(0));
    cannot_play["data"]["cannotType"] = json!("needLogin");
    let mut missing_offset = native_grant("PQ", json!(3000), json!(0));
    missing_offset["data"]
        .as_object_mut()
        .unwrap()
        .remove("offset");
    let mut missing_background = native_grant("PQ", json!(3000), json!(0));
    missing_background["data"]
        .as_object_mut()
        .unwrap()
        .remove("backgroundDuration");
    let cases = [
        cannot_play,
        missing_offset,
        missing_background,
        native_grant("PQ", json!(9000), json!(0)),
        native_grant("PQ", json!(3000), json!(60)),
        {
            let mut value = native_grant("PQ", json!(3000), json!(0));
            value["data"]["playUrl"] = json!(
                "https://freevod.nf.migu.cn/opaque/index.m3u8?playSessionId=session&resourceId=7&resourceType=D"
            );
            value
        },
        {
            let mut value = native_grant("PQ", json!(3000), json!(0));
            value["data"]["playUrl"] = json!(
                "http://evil.invalid:8080/hls/v2/opaque/index.m3u8?playSessionId=session&resourceId=7&resourceType=D"
            );
            value
        },
        {
            let mut value = native_grant("PQ", json!(3000), json!(0));
            value["data"]["playUrl"] = json!(
                "http://freevod.nf.migu.cn:8080/hls/v2/opaque/index.m3u8?playSessionId=session&resourceId=8&resourceType=D"
            );
            value
        },
    ];
    for (index, value) in cases.into_iter().enumerate() {
        assert!(
            parse_native_grant(value, "7", source.duration_ms, MiguNativeMvFormat::Auto).is_err(),
            "case {index}"
        );
    }
}
