use super::*;

fn dto(id: u64) -> Value {
    json!({"video_id":id.to_string(),"video_name":"Video","timelength":"233600",
        "audio_id":"52","album_audio_id":"82","audio_timelength":"229000",
        "audio_hash":"B".repeat(32),"authors":[{"author_id":"35","author_name":"Singer"}],
        "author_name":"Uploader","user_id":"91","user_name":"",
        "hdpic":"http://imge.kugou.com/mvhdpic/{size}/date/cover.jpg",
        "sd_hash":"a".repeat(32),"sd_height":"432","sd_width":"768","sd_filesize":"18378193",
        "sd_bitrate":629174,"sd_hash_265":"C".repeat(32),"sd_filesize_265":16286936,
        "is_publish":"1","deleted":"0","is_short":"4","play_times":"0"})
}
fn decode(items: Vec<Value>, ids: &[&str]) -> Result<Vec<VideoDetail>> {
    parse_details(
        json!({"status":1,"error_code":0,"errcode":0,"data":items})
            .to_string()
            .as_bytes(),
        &ids.iter().map(|v| (*v).to_owned()).collect::<Vec<_>>(),
        VideoResourceKind::Mv,
    )
}

#[test]
fn video_detail_keeps_video_audio_uploader_and_resolution_evidence_separate() {
    let mut d = dto(17);
    d["token"] = json!("never-export");
    d["thumb_mp4"] = json!("https://untrusted.invalid/private");
    let v = decode(vec![d], &["17"]).unwrap().remove(0);
    assert_eq!(v.video.resource_ref.to_string(), "kugou:mv:17");
    assert_eq!(v.video.duration_ms, Some(233600));
    assert_eq!(v.video.extensions["audio_duration_ms"], 229000);
    assert_eq!(v.video.extensions["audio_id"], "52");
    assert_eq!(v.video.extensions["album_audio_id"], "82");
    assert_eq!(
        v.video.creators[0].resource_ref.as_ref().unwrap().id(),
        "35"
    );
    assert_eq!(v.video.extensions["uploader"]["id"], "91");
    assert_eq!(v.video.extensions["uploader"]["name"], "Uploader");
    assert_eq!(
        v.video.cover_url.as_deref(),
        Some("https://imge.kugou.com/mvhdpic/400/date/cover.jpg")
    );
    assert_eq!(v.video.play_count, Some(0));
    assert_eq!(v.video.subscribed, None);
    assert_eq!(v.resolutions.len(), 1);
    assert_eq!(v.resolutions[0].resolution, 432);
    assert_eq!(v.resolutions[0].format, None);
    assert_eq!(
        v.video.extensions["catalogue_assets"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        v.video.extensions["catalogue_assets"][1]["height"],
        Value::Null
    );
    assert!(!serde_json::to_string(&v).unwrap().contains("never-export"));
    assert!(!serde_json::to_string(&v).unwrap().contains("untrusted"));
}

#[test]
fn video_detail_reorders_by_identity_and_never_delivers_partial_or_foreign_batches() {
    let v = decode(vec![dto(2), dto(1)], &["1", "2"]).unwrap();
    assert_eq!(
        v.iter().map(|v| v.video.id.as_str()).collect::<Vec<_>>(),
        ["1", "2"]
    );
    for (items, ids) in [
        (vec![dto(1), dto(1)], vec!["1", "2"]),
        (vec![dto(1), dto(3)], vec!["1", "2"]),
        (vec![dto(1)], vec!["1", "2"]),
        (vec![], vec!["1"]),
        (vec![dto(1), dto(2)], vec!["1", "1"]),
        (
            vec![json!({"unexpected":"not-a-missing-record"})],
            vec!["1"],
        ),
    ] {
        assert_eq!(
            decode(items, &ids).unwrap_err().code,
            ErrorCode::UpstreamError
        );
    }
    assert_eq!(
        decode(vec![dto(1), json!({})], &["1", "2"])
            .unwrap_err()
            .code,
        ErrorCode::ResourceNotFound
    );
    for (key, value) in [("deleted", json!("1")), ("is_publish", json!("0"))] {
        let mut d = dto(1);
        d[key] = value;
        assert_eq!(
            decode(vec![d], &["1"]).unwrap_err().code,
            ErrorCode::ResourceNotFound
        );
    }
}

#[test]
fn video_detail_rejects_ambiguous_fields_bad_identities_and_bad_numeric_resources() {
    for (key, value) in [
        ("video_id", json!("01")),
        ("video_id", json!(0)),
        ("timelength", json!(-1)),
        ("sd_width", json!("4294967296")),
        ("sd_hash", json!("https://bad.invalid/")),
        ("is_publish", json!(2)),
        (
            "authors",
            json!([{"author_id":35,"author_name":"A"},{"author_id":35,"author_name":"B"}]),
        ),
    ] {
        let mut d = dto(1);
        d[key] = value;
        assert!(decode(vec![d], &["1"]).is_err(), "{key}");
    }
    let raw = json!({"status":1,"error_code":0,"data":[dto(1)]}).to_string();
    for (old, new) in [
        (
            "\"video_id\":\"1\"",
            "\"video_id\":\"1\",\"video_id\":\"1\"",
        ),
        (
            "\"timelength\":\"233600\"",
            "\"timelength\":\"233600\",\"timelength\":1",
        ),
        (
            "\"sd_height\":\"432\"",
            "\"sd_height\":\"432\",\"sd_height\":1080",
        ),
    ] {
        let bytes = raw.replace(old, new);
        assert!(
            parse_details(bytes.as_bytes(), &["1".into()], VideoResourceKind::Mv).is_err(),
            "{old}"
        );
    }
}

#[test]
fn video_detail_unknowns_and_zero_artist_ids_are_not_invented_or_merged() {
    let d = json!({"video_id":1,"video_name":"Video","authors":[
        {"author_id":0,"author_name":"A"},{"author_id":0,"author_name":"B"}],
        "cover":"https://evil.invalid/image","user_id":52,"author_name":"Uploader"});
    let v = decode(vec![d], &["1"]).unwrap().remove(0);
    assert_eq!(v.video.creators.len(), 2);
    assert!(v.video.creators.iter().all(|v| v.resource_ref.is_none()));
    assert_eq!(v.video.duration_ms, None);
    assert_eq!(v.video.play_count, None);
    assert_eq!(v.video.cover_url, None);
    assert!(v.resolutions.is_empty());
    assert_eq!(v.video.extensions["uploader"]["id"], "52");
}

#[test]
fn video_empty_dimension_strings_are_unknown_without_relaxing_numeric_identity_or_duration() {
    let mut d = dto(1);
    for field in [
        "sd_width",
        "sd_height",
        "hd_width",
        "hd_height",
        "fhd_width",
        "fhd_height",
        "qhd_width",
        "qhd_height",
        "mkv_qhd_width",
        "mkv_qhd_height",
    ] {
        d[field] = json!("");
    }
    let v = decode(vec![d.clone()], &["1"]).unwrap().remove(0);
    assert!(v.resolutions.is_empty());
    assert_eq!(
        v.video.extensions["catalogue_assets"][0]["height"],
        Value::Null
    );
    assert_eq!(
        v.video.extensions["catalogue_assets"][0]["hash"],
        "A".repeat(32)
    );
    for (field, value) in [
        ("video_id", json!("")),
        ("timelength", json!("")),
        ("sd_width", json!(" ")),
        ("sd_height", json!(false)),
    ] {
        let mut invalid = d.clone();
        invalid[field] = value;
        assert!(decode(vec![invalid], &["1"]).is_err(), "{field}");
    }
}

#[test]
fn mv_search_maps_native_singers_and_source_hashes_without_inventing_media_or_rights() {
    let raw = json!({"MvID":17,"MvName":"MV","Singers":[{"id":0,"name":"A"},{"id":0,"name":"B"},{"id":35,"name":"Singer"}],
        "Duration":233,"AudioID":"52","MixSongID":"82","AlbumID":"90",
        "Pic":"bare-filename.jpg","MvHash":"A".repeat(32),"FileHash":"B".repeat(32),
        "MvHashMark":"1080P","MvHot":123,"Privilege":10,"IsUgc":1,"IsOfficial":0,
        "Userid":99,"Username":"Uploader","ExtName":"mp3","FileSize":1234});
    let SearchItem::Video(v) = map_mv(serde_json::from_value(raw).unwrap()).unwrap() else {
        panic!()
    };
    assert_eq!(v.resource_ref.to_string(), "kugou:mv:17");
    assert_eq!(v.duration_ms, Some(233000));
    assert_eq!(v.cover_url, None);
    assert_eq!(v.play_count, None);
    assert_eq!(v.subscribed, None);
    assert_eq!(v.creators.len(), 3);
    assert!(v.creators[0].resource_ref.is_none());
    assert_eq!(v.extensions["mv_hash"], "A".repeat(32));
    assert_eq!(v.extensions["search_file_hash"], "B".repeat(32));
    assert_eq!(v.extensions["is_ugc_code"], 1);
    assert!(!v.extensions.contains_key("resolution"));
    assert!(!v.extensions.contains_key("format"));
    let hit = json!({"MvID":1,"MvName":"MV","Duration":u64::MAX});
    assert!(map_mv(serde_json::from_value(hit).unwrap()).is_err());
}

#[test]
fn mv_search_distinguishes_page_limit_and_business_errors_from_empty_success() {
    use super::super::catalog::{CatalogKind, parse};
    let out =
        json!({"status":1,"error_code":149,"error_msg":"private","data":{"total":0,"lists":[]}});
    let e = parse(CatalogKind::Mv, out.to_string().as_bytes(), 26)
        .err()
        .unwrap();
    assert_eq!(e.code, ErrorCode::InvalidRequest);
    assert_eq!(e.details["platform_code"], 149);
    assert!(!format!("{e:?}").contains("private"));
    let bad = json!({"status":0,"error_code":44,"errmsg":"private","data":[{}]});
    let e = parse_details(
        bad.to_string().as_bytes(),
        &["1".into()],
        VideoResourceKind::Mv,
    )
    .unwrap_err();
    assert_eq!(e.code, ErrorCode::UpstreamError);
    assert_eq!(e.details["platform_code"], 44);
    assert!(!format!("{e:?}").contains("private"));
}
