use super::*;
use serde_json::Value;

pub(crate) fn source(number: u32) -> Value {
    let content = (600000 + number).to_string();
    let song = (1000 + number).to_string();
    let copyright = format!("CP{number}");
    json!({"resType":"2","resId":content,"songId":song,"copyrightId":copyright,
        "txt":format!("Song {number}"),"txt2":"Artist","txt5":"0",
        "img":"https://d.musicapp.migu.cn/data/oss/resource/cover.webp",
        "songData":json!({"resourceType":"2","contentId":content,"songId":song,
            "copyrightId":copyright,"songName":format!("Song {number}"),
            "duration":180,"albumId":"88","album":"Album",
            "singerList":[{"id":"99","name":"Artist"}],
            "img1":"https://d.musicapp.migu.cn/data/oss/resource/cover.webp",
            "audioFormats":[{"formatType":"SQ"}],"restrictType":1,"foreverListen":true
        }).to_string()})
}
pub(crate) fn catalogue() -> Value {
    json!({"code":"000000","data":{"header":{"dataVersion":"1789614335187","nextPageNo":1,"nextPageNo2":1,"update":false},
        "contents":[{"view":"ZJ-RANK-SPECIAL","style":"特色榜","contents":[
            {"view":"ZJ-Song-Scroll","rankId":"10","rankName":"全网热歌榜","imageUrl":"https://d.musicapp.migu.cn/data/oss/column/cover.webp"}]},
            {"view":"ZJ-RANK-NORMAL","style":"尖叫榜","contents":[
                {"rankId":"20","rankName":"国风热歌榜","contents":[source(1),source(2),source(3)]},
                {"rankId":"21","rankName":"新歌榜","contents":[]}]}]}})
}
pub(crate) fn detail(total: u32) -> Value {
    json!({"code":"000000","data":{"view":"ZJ-Song-Scroll","columnId":"20","periodColumnId":"20",
        "title":"国风热歌榜","desc":"Description\nSecond line","updateTime":"2026-09-17",
        "titlePic":"https://d.musicapp.migu.cn/data/oss/column/cover.webp",
        "hasNextPage":false,"totalCount":total,"contents":(1..=total).map(source).collect::<Vec<_>>()}})
}
pub(crate) fn long_timeout_client(origin: Url) -> MiguClient {
    let mut client = MiguClient::test_client().with_catalog_test_origin(origin);
    client.http = Client::builder()
        .timeout(Duration::from_secs(90))
        .redirect(Policy::none())
        .retry(reqwest::retry::never())
        .build()
        .unwrap();
    client
}
fn bytes(v: &Value) -> Vec<u8> {
    serde_json::to_vec(v).unwrap()
}

#[test]
fn charts_catalogue_preserves_all_groups_new_names_previews_and_unknown_rights() {
    for view in [
        ChartCatalogView::Overview,
        ChartCatalogView::Summary,
        ChartCatalogView::Modern,
    ] {
        let result = parse_catalogue(&bytes(&catalogue()), view).unwrap();
        assert_eq!(result.view, view);
        assert_eq!(result.groups.len(), 2);
        assert_eq!(result.groups[1].charts.len(), 2);
        let c = &result.groups[1].charts[0];
        assert_eq!(c.name, "国风热歌榜");
        assert_eq!(
            c.resource_ref.as_ref().unwrap().to_string(),
            "migu:chart:20"
        );
        assert!(c.track_count.is_none() && c.updated_at_ms.is_none() && c.playable.is_none());
        assert_eq!(c.previews.len(), 3);
        assert_eq!(
            c.previews[1].track_ref.as_ref().unwrap().to_string(),
            "migu:600002"
        );
        assert_eq!(c.previews[1].rank, Some(2));
        assert_eq!(c.previews[1].rank_change, Some(0));
        assert!(c.previews[1].previous_rank.is_none());
        assert_eq!(c.previews[1].extensions["song_id"], "1002");
        assert_eq!(c.previews[1].extensions["copyright_id"], "CP2");
    }
}

#[test]
fn charts_complete_tracks_bind_distinct_ids_ranks_and_optional_changes_without_authorizing() {
    let mut v = detail(4);
    let legacy_cover = format!(
        "https://d.musicapp.migu.cn/prod/file-service/file-down/{}/{}/{}",
        "a".repeat(32),
        "b".repeat(32),
        "c".repeat(32)
    );
    v["data"]["contents"][0]["img"] = json!(legacy_cover);
    let mut song: Value =
        serde_json::from_str(v["data"]["contents"][0]["songData"].as_str().unwrap()).unwrap();
    song["img1"] = json!(legacy_cover);
    v["data"]["contents"][0]["songData"] = json!(song.to_string());
    for (row, change) in v["data"]["contents"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .zip([json!("12"), json!("-2"), Value::Null, json!("")])
    {
        row["txt5"] = change;
    }
    for tags in [false, true] {
        let result = parse_tracks(&bytes(&v), "20", tags).unwrap();
        assert_eq!(result.items.len(), 4);
        assert_eq!(
            result.items[0].album.as_ref().unwrap().cover_url.as_deref(),
            Some(legacy_cover.as_str())
        );
        for (index, t) in result.items.iter().enumerate() {
            assert_eq!(t.id, (600001 + index).to_string());
            assert_eq!(t.extensions["song_id"], (1001 + index).to_string());
            assert_eq!(t.extensions["chart_rank"], index + 1);
            assert_eq!(t.extensions["chart_position"], index);
            assert!(t.playable.is_none());
            assert_eq!(t.available_qualities, [Quality::Lossless]);
            assert_eq!(t.duration_ms, Some(180000));
            assert!(!t.extensions.contains_key("previous_rank"));
            assert_eq!(
                t.extensions.contains_key("chart_rank_change"),
                tags && index < 2
            );
        }
        if tags {
            assert_eq!(result.items[0].extensions["chart_rank_change"], 12);
            assert_eq!(result.items[1].extensions["chart_rank_change"], -2);
        }
        assert_eq!(result.extensions["complete_read"], true);
        assert_eq!(result.extensions["update_label"], "2026-09-17");
        assert_eq!(result.extensions["period_column_id"], "20");
    }
    assert!(
        parse_tracks(&bytes(&detail(0)), "20", true)
            .unwrap()
            .items
            .is_empty()
    );
}

#[test]
fn charts_reject_incomplete_counts_duplicate_ids_and_unbound_periods() {
    let mut incomplete = detail(99);
    incomplete["data"]["totalCount"] = json!(100);
    let e = parse_tracks(&bytes(&incomplete), "20", true).err().unwrap();
    assert_eq!(e.code, ErrorCode::UpstreamError);
    assert_eq!(e.details["reason"], "chart_count_mismatch");
    assert_eq!(e.details["declared_track_count"], 100);
    assert_eq!(e.details["received_track_count"], 99);
    for case in 0..12 {
        let mut v = detail(3);
        match case {
            0 => v["data"]["totalCount"] = json!(4),
            1 => v["data"]["totalCount"] = json!(2),
            2 => v["data"]["hasNextPage"] = json!(true),
            3 => v["data"]["columnId"] = json!("21"),
            4 => v["data"]["periodColumnId"] = json!("200"),
            5 => v["data"]["contents"][1]["resId"] = v["data"]["contents"][0]["resId"].clone(),
            6 => v["data"]
                .as_object_mut()
                .unwrap()
                .remove("totalCount")
                .map(|_| ())
                .unwrap(),
            7 => v["data"]["view"] = json!("other"),
            8 => v["data"]["totalCount"] = json!("3"),
            9 => v["data"]["hasNextPage"] = json!("false"),
            10 => v["data"]["contents"] = Value::Null,
            _ => v["data"]["titlePic"] = json!("https://elsewhere.invalid/cover"),
        }
        assert!(parse_tracks(&bytes(&v), "20", true).is_err(), "case {case}");
    }
    let mut v = detail(2);
    let first = v["data"]["contents"][0].clone();
    let second = &mut v["data"]["contents"][1];
    second["resId"] = first["resId"].clone();
    let mut song: Value = serde_json::from_str(second["songData"].as_str().unwrap()).unwrap();
    song["contentId"] = first["resId"].clone();
    second["songData"] = json!(song.to_string());
    assert!(parse_tracks(&bytes(&v), "20", true).is_err());
}

#[test]
fn charts_reject_corrupt_embedded_song_identity_and_rank_types_including_off_window_rows() {
    for case in 0..13 {
        let mut v = detail(3);
        let last = &mut v["data"]["contents"][2];
        match case {
            0 => last["resId"] = json!("9999"),
            1 => last["songId"] = json!("9999"),
            2 => last["copyrightId"] = json!("OTHER"),
            3 => last["resType"] = json!("D"),
            4 => last["songData"] = json!("{}"),
            5 => last["songData"] = json!({"contentId":"600003"}),
            6 => last["txt5"] = json!("NaN"),
            7 => last["txt5"] = json!(2),
            8 => last["txt5"] = json!("9223372036854775808"),
            9 => last["txt"] = json!("Bad\u{0}name"),
            10 => last["img"] = json!("https://d.musicapp.migu.cn/data/oss/cover.webp#secret"),
            11 => last["songData"] = json!(" ".repeat(65537)),
            _ => last["songId"] = json!("01003"),
        }
        assert!(parse_tracks(&bytes(&v), "20", true).is_err(), "case {case}");
        // Hidden tags do not suppress malformed upstream identity or ranking evidence.
        assert!(
            parse_tracks(&bytes(&v), "20", false).is_err(),
            "case {case}"
        );
    }
}

#[test]
fn charts_reject_partial_catalogues_wrong_shapes_and_duplicate_fields() {
    for case in 0..9 {
        let mut v = catalogue();
        match case {
            0 => v["data"]["header"]["nextPageNo"] = json!(2),
            1 => v["data"]["contents"][0]["view"] = json!("future"),
            2 => v["data"]["contents"][1]["contents"][0]["rankId"] = json!("10"),
            3 => v["data"]["contents"][0]["contents"][0]["view"] = json!("unknown"),
            4 => v["data"]["contents"][0]["contents"][0]["rankId"] = json!("01"),
            5 => {
                v["data"]["contents"][0]["contents"][0]["imageUrl"] =
                    json!("https://evil.invalid/a")
            }
            6 => v["data"]["contents"][1]["contents"][0]["contents"][1]["resId"] = json!("600001"),
            7 => v["data"]["contents"][1]["view"] = json!("ZJ-RANK-SPECIAL"),
            _ => v["data"]["contents"][1]["contents"][0]["contents"][0]["songId"] = json!("999"),
        }
        assert!(
            parse_catalogue(&bytes(&v), ChartCatalogView::Summary).is_err(),
            "case {case}"
        );
    }
    let bytes = detail(1)
        .to_string()
        .replace("\"totalCount\":1", "\"totalCount\":1,\"totalCount\":0");
    assert!(parse_tracks(bytes.as_bytes(), "20", true).is_err());
    for (body, expected) in [
        (json!({"code":"200000"}), ErrorCode::ResourceNotFound),
        (
            json!({"code":"000000","data":null}),
            ErrorCode::UpstreamError,
        ),
        (
            json!({"code":"999999","info":"untrusted"}),
            ErrorCode::UpstreamError,
        ),
    ] {
        let e = parse_tracks(body.to_string().as_bytes(), "20", true)
            .err()
            .unwrap();
        assert_eq!(e.code, expected);
        assert!(!e.message.contains("untrusted"));
    }
}
