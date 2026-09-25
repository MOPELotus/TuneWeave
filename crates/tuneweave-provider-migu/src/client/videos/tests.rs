use super::*;

pub(crate) fn detail(id: &str) -> Value {
    json!({"code":"000000","resource":[{"resourceType":"D","contentId":id,"songName":"MV title","singer":"Artist & Guest","singerId":"112,113","migumvDuration":"00:04:19","copyrightId":"MV123","songId":"999","imgs":[{"img":"https://d.musicapp.migu.cn/data/oss/resource/a.webp"}],"opNumItem":{"playNum":283133,"thumbNum":68,"keepNum":228,"commentNum":192,"shareNum":1850},"rateFormats":[{"resourceType":"D","formatType":"SQ","format":"050015","fileType":"mp4","size":"130718557","url":"https://private.invalid/not-authorized?secret=hidden"}]}]})
}
pub(crate) fn search(first: u32, count: u32, more: bool) -> Value {
    json!({"code":"000000","data":{"hasNext":more,"items":(first..first+count).map(|id|json!({"video":{"contentId":id.to_string(),"resourceType":"D","title":format!("MV {id}"),"duration":259000,"user":[{"type":2,"videoUserId":"112","nickName":"Artist","avatar":"https://d.musicapp.migu.cn/data/oss/resource/a.webp"}]}})).collect::<Vec<_>>()}})
}
pub(crate) fn artist(first: u32, count: u32, number: u32, more: bool) -> Value {
    let mut header = json!({});
    if more {
        header["nextPageUrl"] = json!(format!(
            "http://app.c.nf.migu.cn{ARTIST_PATH}?singerId=112&pageNo={}",
            number + 1
        ));
    }
    json!({"code":"000000","data":{"header":header,"contents":(first..first+count).map(|id|json!({"view":"ZJ-Mv","resType":"D","resId":id.to_string(),"action":format!("mgmusic://mv-info?id={id}"),"txt":format!("MV {id}"),"txt2":"Artist & Guest","txt3":"28.3万","txt4":"00:04:19"})).collect::<Vec<_>>()}})
}
#[test]
fn mv_detail_accepts_official_extensionless_resource_service_covers() {
    let url = "https://d.musicapp.migu.cn/data/resource-service/file-down/00/32/kf/81";
    let mut v = detail("7");
    v["resource"][0]["imgs"][0]["img"] = json!(url);
    assert_eq!(
        parse_detail(v, "7")
            .unwrap()
            .detail
            .video
            .cover_url
            .as_deref(),
        Some(url)
    );
    for bad in [
        url.replace("https:", "http:"),
        url.replace("d.musicapp.migu.cn", "outside.invalid"),
        format!("{url}?token=secret"),
        format!("{url}#fragment"),
        url.replace("/kf/", "/%6b%66/"),
        url.replace("/kf/", "/kff/"),
        url.replace("/kf/", "/"),
        url.replace("d.musicapp", "user@d.musicapp"),
    ] {
        assert!(mv_image_url(&bad).is_none(), "{bad}");
    }
}
#[test]
fn mv_detail_keeps_identity_units_exact_stats_and_unknown_authorization_separate() {
    let mut input = detail("7");
    input["resource"][0]["summary"] = json!("First line\nSecond line");
    let v = parse_detail(input, "7").unwrap();
    assert_eq!(v.detail.video.description, "First line\nSecond line");
    assert_eq!(v.detail.video.resource_ref.to_string(), "migu:7");
    assert_eq!(v.detail.video.duration_ms, Some(259000));
    assert_eq!(v.stats.view_count, Some(283133));
    assert_eq!(v.stats.favorite_count, Some(228));
    assert_eq!(v.stats.like_count, Some(68));
    assert_eq!(v.stats.comment_count, Some(192));
    assert_eq!(v.stats.share_count, Some(1850));
    assert!(v.stats.liked.is_none() && v.stats.favorited.is_none());
    assert!(v.detail.resolutions.is_empty());
    assert!(v.detail.video.creators[0].resource_ref.is_none());
    let encoded = serde_json::to_string(&v.detail).unwrap();
    assert!(!encoded.contains("hidden") && !encoded.contains("private.invalid"));
    assert_eq!(
        v.detail.video.extensions["catalogue_formats"][0]["size"],
        130718557u64
    );
    let mut data = detail("7");
    for key in [
        "opNumItem",
        "rateFormats",
        "migumvDuration",
        "imgs",
        "singer",
    ] {
        data["resource"][0].as_object_mut().unwrap().remove(key);
    }
    let v = parse_detail(data, "7").unwrap();
    assert!(v.stats.view_count.is_none());
    assert!(v.detail.video.duration_ms.is_none());
}
#[test]
fn mv_detail_rejects_ambiguous_resource_shapes_bad_units_counts_and_covers() {
    for case in 0..12 {
        let mut data = detail("7");
        let v = &mut data["resource"][0];
        match case {
            0 => v["contentId"] = json!("8"),
            1 => v["resourceType"] = json!("2"),
            2 => v["migumvDuration"] = json!("00:60:00"),
            3 => v["migumvDuration"] = json!("259"),
            4 => v["imgs"][0]["img"] = json!("https://outside.invalid/a.jpg"),
            5 => v["opNumItem"]["playNum"] = json!("28.3万"),
            6 => v["rateFormats"][0]["size"] = json!("18446744073709551616"),
            7 => v["songName"] = json!("bad\nname"),
            8 => v["opNumItem"] = json!([]),
            9 => v["rateFormats"][0]["resourceType"] = json!("2"),
            10 => v["summary"] = json!("bad\u{0000}description"),
            _ => data["resource"] = json!([v.clone(), v.clone()]),
        }
        assert!(parse_detail(data, "7").is_err(), "case {case}");
    }
    assert_eq!(
        parse_detail(json!({"resource":[]}), "7")
            .err()
            .unwrap()
            .code,
        ErrorCode::ResourceNotFound
    );
    assert!(parse_detail(json!({}), "7").is_err());
}
#[test]
fn mv_search_requires_explicit_end_and_never_treats_other_video_kinds_as_mv() {
    let parsed = parse_search(search(1, 20, true)["data"].clone()).unwrap();
    assert_eq!(parsed.items[0].duration_ms, Some(259000));
    assert!(parsed.more);
    assert!(parsed.items[0].creators[0].resource_ref.is_none());
    assert_eq!(
        parsed.items[0].extensions["creator_sources"][0]["video_user_id"],
        "112"
    );
    assert!(
        parse_search(json!({"hasNext":false}))
            .unwrap()
            .items
            .is_empty()
    );
    for value in [
        json!({}),
        json!({"hasNext":true}),
        json!({"hasNext":false,"items":null}),
        search(1, 1, true)["data"].clone(),
        search(1, 21, false)["data"].clone(),
    ] {
        assert!(parse_search(value).is_err());
    }
    for (key, value) in [
        ("resourceType", json!("2033")),
        ("duration", json!(-1)),
        ("duration", json!(u64::MAX)),
        ("contentId", json!("01")),
        ("showImg", json!("https://evil.invalid/a")),
    ] {
        let mut data = search(1, 1, false)["data"].clone();
        data["items"][0]["video"][key] = value;
        assert!(parse_search(data).is_err());
    }
}
#[test]
fn mv_artist_requires_mv_only_fixed_continuation_and_preserves_display_only_counts() {
    let data = artist(1, 10, 1, true)["data"].clone();
    let page = parse_artist(data.clone(), "112", 1).unwrap();
    assert!(page.more);
    assert!(page.items[0].play_count.is_none());
    assert_eq!(page.items[0].extensions["play_count_display"], "28.3万");
    for next in [
        format!("https://evil.invalid{ARTIST_PATH}?singerId=112&pageNo=2"),
        format!("https://app.c.nf.migu.cn{ARTIST_PATH}?singerId=113&pageNo=2"),
        format!("https://app.c.nf.migu.cn{ARTIST_PATH}?singerId=112&pageNo=1"),
        format!("https://app.c.nf.migu.cn{ARTIST_PATH}?singerId=112&pageNo=2&pageNo=2"),
        "http://app.c.nf.migu.cn/bmw/singer/video/v1.0?singerId=112&pageNo=2".into(),
    ] {
        let mut bad = data.clone();
        bad["header"]["nextPageUrl"] = json!(next);
        assert!(parse_artist(bad, "112", 1).is_err());
    }
    for case in 0..4 {
        let mut bad = data.clone();
        match case {
            0 => bad["contents"][0]["view"] = json!("ZJ-Singer-Vrbt"),
            1 => bad["contents"][0]["action"] = json!("mgmusic://mv-info?id=2"),
            2 => bad["contents"] = json!([]),
            _ => bad["header"] = Value::Null,
        }
        assert!(parse_artist(bad, "112", 1).is_err());
    }
    assert!(
        parse_artist(artist(1, 0, 1, false)["data"].clone(), "112", 1)
            .unwrap()
            .items
            .is_empty()
    );
}
