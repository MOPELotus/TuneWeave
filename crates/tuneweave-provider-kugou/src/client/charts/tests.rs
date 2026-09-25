use super::*;

pub(crate) fn node(id: u64) -> Value {
    json!({"rankid":id,"rankname":format!("Chart {id}"),"rank_cid":0,"children":[],"haschildren":0,
        "classify":1,"ranktype":2,"intro":"Chart description","imgurl":"http://imge.kugou.com/{size}/chart.jpg",
        "update_frequency":"每天","rank_id_publish_date":"2026-09-15 08:30:00","issue":8,"play_times":1000,
        "show_play_button":0,"jump_url":"https://h5.kugou.com/chart/index.html?entry=1",
        "extra":{"resp":{"all_total":105,"rank_tag":[{"type":3,"desc":"榜单说明"}]}},
        "songinfo":[{"album_audio_id":1000,"name":"Song 0","author":"Artist","trans_param":{"union_cover":"http://imge.kugou.com/cover.jpg","rights":"not-exported"}}]
    })
}
pub(crate) fn catalogue() -> Value {
    json!({"status":1,"errcode":0,"error":"","data":{"timestamp":1234567890,"total":2,"info":[node(42),node(43)]}})
}
pub(crate) fn info(total: u64) -> Value {
    json!({"status":1,"errcode":0,"error":"","data":{"rankid":42,"rankname":"Chart 42","rank_cid":7,
        "extra":{"resp":{"all_total":total,"rank_tag":[{"type":3,"desc":"榜单说明"}]}}}})
}
pub(crate) fn song(n: u64) -> Value {
    json!({"album_audio_id":1000+n,"audio_id":2000+n,"songname":format!("Song {n}"),"album_id":900,
        "album_info":{"album_name":"Album","sizable_cover":"http://imge.kugou.com/cover.jpg"},
        "authors":[{"author_id":42,"author_name":"Artist"},{"author_id":0,"author_name":"Unknown collaborator"}],
        "audio_info":{"hash_128":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","duration_128":321234,"filesize_128":1200,"bitrate":128,"extname":"mp3",
            "hash_flac":"BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB","duration_flac":321300,"filesize_flac":4000,"bitrate_flac":881},
        "rank_cid":7,"business":{"rank_id":"7","parent_id":"42","sort":n+1,"original_index":n+1,
            "last_sort":0,"last_original_index":7,"rank_count":3,"issue":"8","rank_id_publish_date":"2026-09-15 08:30:00","recommend_reason":"推荐理由"},
        "trans_obj":{"rank_show_sort":1},"remarks":[{"type":6,"remark":"舞台原唱"}],"copyright":{"private":"not-exported"},
        "user_download":{"status_128":1},"video_info":{"video_hash":"unrelated-video"}
    })
}
pub(crate) fn tracks(page: u32, total: u64) -> Value {
    let offset = u64::from(page - 1) * 100;
    json!({"status":1,"error_code":0,"total":total,"extra":{"resp":{"all_total":total}},
        "data":{"total":total,"songlist":(offset..total.min(offset+100)).map(song).collect::<Vec<_>>()}})
}
fn snapshot() -> Snapshot {
    parse_info(info(105).to_string().as_bytes(), 42).unwrap()
}

#[test]
fn chart_catalogue_keeps_references_previews_unknown_rights_and_actual_update_labels() {
    let c = parse_catalogue(catalogue().to_string().as_bytes(), ChartCatalogView::Modern).unwrap();
    assert_eq!(c.view, ChartCatalogView::Modern);
    assert_eq!(c.groups[0].charts.len(), 2);
    let t = &c.groups[0].charts[0];
    assert_eq!(t.id.as_deref(), Some("42"));
    assert_eq!(t.resource_ref.as_ref().unwrap().id(), "chart:42");
    assert_eq!(t.track_count, Some(105));
    assert_eq!(t.play_count, Some(1000));
    assert_eq!(t.playable, None);
    assert_eq!(t.subscribed, None);
    assert_eq!(t.updated_at_ms, None);
    assert_eq!(t.update_frequency.as_deref(), Some("每天"));
    assert_eq!(t.previews[0].track_ref.as_ref().unwrap().id(), "1000");
    assert_eq!(t.previews[0].rank, None);
    assert_eq!(
        t.previews[0].cover_url.as_deref(),
        Some("https://imge.kugou.com/cover.jpg")
    );
    assert!(!serde_json::to_string(&c).unwrap().contains("not-exported"));
    let mut v = catalogue();
    v["data"]["info"][0]
        .as_object_mut()
        .unwrap()
        .remove("extra");
    assert_eq!(
        parse_catalogue(v.to_string().as_bytes(), ChartCatalogView::Summary)
            .unwrap()
            .groups[0]
            .charts[0]
            .track_count,
        None
    );
}

#[test]
fn chart_catalogue_preserves_group_children_and_rejects_contradictions_duplicates_and_depth() {
    let mut v = catalogue();
    v["data"]["total"] = json!(1);
    v["data"]["info"] =
        json!([{"rankname":"Group","haschildren":1,"children":[node(42),node(43)]}]);
    let c = parse_catalogue(v.to_string().as_bytes(), ChartCatalogView::Summary).unwrap();
    assert_eq!(c.groups[1].name, "Group");
    assert_eq!(c.groups[1].charts.len(), 2);
    assert_eq!(c.extensions["chart_count"], 2);
    assert_eq!(c.groups[1].extensions["source_path"], json!([0]));
    assert_eq!(
        c.groups[1].charts[1].extensions["source_path"],
        json!([0, 1])
    );
    v["data"]["info"][0]["haschildren"] = json!(0);
    assert!(parse_catalogue(v.to_string().as_bytes(), ChartCatalogView::Summary).is_err());
    let mut v = catalogue();
    v["data"]["info"][1] = node(42);
    assert!(parse_catalogue(v.to_string().as_bytes(), ChartCatalogView::Summary).is_err());
    let mut v = catalogue();
    v["data"]["total"] = json!(3);
    assert!(parse_catalogue(v.to_string().as_bytes(), ChartCatalogView::Summary).is_err());
    let mut nested = node(42);
    for _ in 0..10 {
        nested = json!({"rankname":"Group","haschildren":1,"children":[nested]});
    }
    let v = json!({"status":1,"errcode":0,"data":{"total":1,"info":[nested]}});
    assert!(parse_catalogue(v.to_string().as_bytes(), ChartCatalogView::Summary).is_err());
}

#[test]
fn chart_envelopes_require_their_own_business_protocol_and_positive_resolved_period() {
    for (key, value) in [
        ("rankid", json!(43)),
        ("rank_cid", json!(0)),
        ("rank_cid", json!("07")),
    ] {
        let mut v = info(0);
        v["data"][key] = value;
        assert!(parse_info(v.to_string().as_bytes(), 42).is_err());
    }
    for v in [
        json!({"status":0,"errcode":1001,"error":"private-message"}),
        json!({"status":1,"error_code":0,"data":{}}),
    ] {
        assert!(check_ocean_status(v.to_string().as_bytes()).is_err());
    }
    assert!(check_status(info(0).to_string().as_bytes()).is_err());
    let wire = info(0)
        .to_string()
        .replace("\"rank_cid\":7", "\"rank_cid\":7,\"rank_cid\":7");
    assert!(parse_info(wire.as_bytes(), 42).is_err());
    let wire = info(0)
        .to_string()
        .replace("\"errcode\":0", "\"errcode\":0,\"errcode\":0");
    assert!(parse_info(wire.as_bytes(), 42).is_err());
    assert!(parse_info(info(12801).to_string().as_bytes(), 42).is_err());
    assert_eq!(
        parse_info(info(0).to_string().as_bytes(), 42)
            .unwrap()
            .total,
        Some(0)
    );
}

#[test]
fn chart_tracks_preserve_actual_rank_and_audio_units_without_inventing_movement_or_rights() {
    let (items, total) =
        parse_tracks(tracks(2, 105).to_string().as_bytes(), &snapshot(), 2, true).unwrap();
    assert_eq!(total, 105);
    let t = &items[0];
    assert_eq!(t.id, "1100");
    assert_eq!(t.extensions["chart_rank"], 101);
    assert_eq!(t.extensions["last_sort"], 0);
    assert_eq!(t.extensions["last_original_index"], 7);
    assert!(!t.extensions.contains_key("rank_change"));
    assert_eq!(t.extensions["rank_cid"], "7");
    assert_eq!(t.duration_ms, Some(321234));
    assert_eq!(t.extensions["qualities"]["lossless"]["bitrate"], 881);
    assert_eq!(t.artists[1].resource_ref, None);
    assert_eq!(t.playable, None);
    assert_eq!(t.extensions["chart_remarks"][0]["text"], "舞台原唱");
    let wire = serde_json::to_string(t).unwrap();
    assert!(!wire.contains("not-exported"));
    assert!(!wire.contains("unrelated-video"));
    let (items, _) =
        parse_tracks(tracks(1, 1).to_string().as_bytes(), &snapshot(), 1, false).unwrap();
    assert!(!items[0].extensions.contains_key("chart_remarks"));
    assert!(!items[0].extensions.contains_key("recommend_reason"));
    assert_eq!(items[0].extensions["chart_rank"], 1);
}

#[test]
fn chart_tracks_reject_foreign_chart_period_bad_ranking_and_inconsistent_totals() {
    for case in 0..9 {
        let mut v = tracks(1, 1);
        match case {
            0 => v["data"]["songlist"][0]["rank_cid"] = json!(8),
            1 => v["data"]["songlist"][0]["business"]["rank_id"] = json!("8"),
            2 => v["data"]["songlist"][0]["business"]["parent_id"] = json!("43"),
            3 => v["data"]["songlist"][0]["business"]["original_index"] = json!(2),
            4 => v["total"] = json!(2),
            5 => v["extra"]["resp"]["all_total"] = json!(2),
            6 => v["data"]["songlist"][0]["album_audio_id"] = json!(0),
            7 => v["data"]["songlist"][0]["authors"] = json!([]),
            8 => v["data"]["songlist"] = json!([]),
            _ => unreachable!(),
        }
        assert!(
            parse_tracks(v.to_string().as_bytes(), &snapshot(), 1, true).is_err(),
            "case {case}"
        );
    }
    for (page, total, count) in [(1, 0, 0), (2, 105, 5), (3, 105, 0)] {
        assert_eq!(
            parse_tracks(
                tracks(page, total).to_string().as_bytes(),
                &snapshot(),
                page,
                true
            )
            .unwrap()
            .0
            .len(),
            count
        );
    }
    assert!(
        parse_tracks(
            tracks(1, 12801).to_string().as_bytes(),
            &snapshot(),
            1,
            true
        )
        .is_err()
    );
}

#[test]
fn chart_display_links_are_limited_to_official_https_targets() {
    assert_eq!(
        display_url("http://h5.kugou.com/chart?a=1").as_deref(),
        Some("https://h5.kugou.com/chart?a=1")
    );
    for url in [
        "https://evil.invalid/a",
        "https://h5.kugou.com.evil.invalid/",
        "https://user@h5.kugou.com/",
        "https://h5.kugou.com:444/a",
        "javascript:alert(1)",
        "kugou://rank/1",
        "https://h5.kugou.com/a\r\nx",
    ] {
        assert_eq!(display_url(url), None);
    }
}
