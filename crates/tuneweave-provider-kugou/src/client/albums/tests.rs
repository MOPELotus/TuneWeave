use super::*;

fn detail() -> Value {
    json!({"status":1,"error_code":0,"errcode":0,"data":[{
        "album_id":"42","album_name":"Album","authors":[{"author_id":"7","author_name":"A"},{"author_id":0,"author_name":"B"}],
        "sizable_cover":"http://imge.kugou.com/a/{size}.jpg","type":"LiveCD","publish_date":"2005-01-20",
        "publish_company":"Publisher","intro":"Description\n第二段","language":"国语","is_publish":"1"
    }]})
}
fn album() -> Album {
    parse_album(detail().to_string().as_bytes(), 42).unwrap()
}
fn song(index: u64) -> Value {
    json!({
        "base":{"album_id":42,"album_audio_id":100+index,"audio_id":200+index,"audio_name":format!("Song {index}"),"author_name":"A"},
        "album_info":{"album_name":"Album"},"authors":[{"author_id":7,"author_name":"A"}],
        "extend":{"disc":1,"sort":index+1},
        "audio_info":{"hash":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","hash_128":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "duration":271151,"duration_128":271151,"filesize":1234,"bitrate":128,"extname":"mp3",
            "hash_320":"BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB","duration_320":271107,"filesize_320":2345,
            "hash_flac":"CCCCCCCCCCCCCCCCCCCCCCCCCCCCCCCC","duration_flac":271106,"filesize_flac":3456,
            "hash_high":"","hash_super":"","extname_super":""},
        "copyright":{"privilege":10,"viponly_tag":1},"trans_param":{"arbitrary":"do-not-export"}
    })
}
fn page(songs: Vec<Value>, total: u64) -> Value {
    json!({"status":1,"error_code":0,"total":total,"extra":{"disc_cnt":1},"data":{"total":total,"songs":songs}})
}
fn parse(value: &Value, p: u32) -> Result<PhysicalPage> {
    parse_tracks(value.to_string().as_bytes(), &album(), p)
}

#[test]
fn album_metadata_preserves_authors_company_type_and_unknown_count_without_inventing_rights() {
    let a = album();
    assert_eq!(a.id, "42");
    assert_eq!(a.resource_ref.id(), "42");
    assert_eq!(a.artists.len(), 2);
    assert_eq!(a.artists[0].resource_ref.as_ref().unwrap().id(), "7");
    assert_eq!(a.artists[1].resource_ref, None);
    assert_eq!(a.company.as_deref(), Some("Publisher"));
    assert_eq!(a.kind.as_deref(), Some("LiveCD"));
    assert_eq!(a.track_count, None);
    assert_eq!(
        a.cover_url.as_deref(),
        Some("https://imge.kugou.com/a/400.jpg")
    );
    assert_eq!(a.description, "Description\n第二段");
    assert!(!a.extensions.contains_key("is_vip"));
    let mut d = detail();
    d["data"][0]["authors"] = json!([]);
    d["data"][0]["author_name"] = json!("Unknown author ID");
    let a = parse_album(d.to_string().as_bytes(), 42).unwrap();
    assert_eq!(a.artists[0].resource_ref, None);
}

#[test]
fn album_tracks_keep_album_audio_identity_asset_units_and_disc_positions_separate() {
    let p = parse(&page(vec![song(0)], 1), 1).unwrap();
    let t = &p.items[0];
    assert_eq!(t.id, "100");
    assert_eq!(t.extensions["album_audio_id"], "100");
    assert_eq!(t.extensions["audio_id"], "200");
    assert_eq!(
        t.album
            .as_ref()
            .unwrap()
            .resource_ref
            .as_ref()
            .unwrap()
            .id(),
        "42"
    );
    assert_eq!(t.duration_ms, Some(271151));
    assert_eq!(t.playable, None);
    assert_eq!(
        t.available_qualities,
        [Quality::Standard, Quality::High, Quality::Lossless]
    );
    assert_eq!(t.extensions["hash"], "A".repeat(32));
    assert_eq!(t.extensions["qualities"]["high"]["duration_ms"], 271107);
    assert_eq!(t.extensions["qualities"]["lossless"]["size"], 3456);
    assert_eq!(t.extensions["disc_number"], 1);
    assert_eq!(t.extensions["track_number"], 1);
    assert_eq!(t.extensions["album_position"], 0);
    assert!(!serde_json::to_string(t).unwrap().contains("do-not-export"));
    assert!(!t.extensions.contains_key("privilege"));
}

#[test]
fn album_parsers_reject_duplicate_fields_foreign_identities_and_error_envelopes() {
    for key in ["album_id", "album_name"] {
        let d = detail();
        let needle = if key == "album_id" {
            "\"album_id\":\"42\""
        } else {
            "\"album_name\":\"Album\""
        };
        let wire = d.to_string().replace(needle, &format!("{needle},{needle}"));
        assert!(parse_album(wire.as_bytes(), 42).is_err());
    }
    for v in [json!(0), json!(43), json!("042"), json!(true)] {
        let mut d = detail();
        d["data"][0]["album_id"] = v;
        assert!(parse_album(d.to_string().as_bytes(), 42).is_err());
    }
    let mut d = detail();
    d["data"].as_array_mut().unwrap().clear();
    assert!(parse_album(d.to_string().as_bytes(), 42).is_err());
    for (key, v) in [
        ("album_id", json!(43)),
        ("album_audio_id", json!(0)),
        ("album_audio_id", json!("0100")),
        ("audio_name", json!("")),
    ] {
        let mut s = song(0);
        s["base"][key] = v;
        assert!(parse(&page(vec![s], 1), 1).is_err());
    }
    let mut s = song(0);
    s["base"].as_object_mut().unwrap().remove("album_audio_id");
    assert!(parse(&page(vec![s], 1), 1).is_err());
    for v in [json!({"album_id":43}), json!({"album_name":"Other album"})] {
        let mut s = song(0);
        s["album_info"] = v;
        assert!(parse(&page(vec![s], 1), 1).is_err());
    }
    for (status, code, errcode) in [(0, 20006, 0), (1, 0, 20018), (0, 0, 0)] {
        let v = json!({"status":status,"error_code":code,"errcode":errcode,"errmsg":"private-detail","data":"private-detail"});
        let e = check_status(v.to_string().as_bytes()).unwrap_err();
        assert!(!format!("{e:?}").contains("private-detail"));
    }
}

#[test]
fn album_pages_require_matching_totals_exact_counts_and_bounded_complete_traversal() {
    for (total, p, count) in [(25, 1, 20), (25, 2, 5), (25, 3, 0), (0, 1, 0)] {
        let v = page((0..count).map(song).collect(), total);
        assert_eq!(parse(&v, p).unwrap().items.len(), count as usize);
    }
    let mut v = page(vec![song(0)], 1);
    v["total"] = json!(2);
    assert!(parse(&v, 1).is_err());
    let v = page(vec![song(0)], 21);
    assert!(parse(&v, 1).is_err());
    let v = page((0..20).map(song).collect(), 1281);
    assert!(parse(&v, 1).is_err());
    let mut v = page(vec![song(0)], 1);
    v["extra"]["disc_cnt"] = json!(0);
    assert!(parse(&v, 1).is_err());
    let mut v = page(vec![song(0)], 1);
    v["data"]["songs"][0]["extend"]["disc"] = json!(2);
    assert!(parse(&v, 1).is_err());
    let wire = page(vec![song(0)], 1)
        .to_string()
        .replace("\"total\":1", "\"total\":1,\"total\":1");
    assert!(parse_tracks(wire.as_bytes(), &album(), 1).is_err());
}

#[test]
fn album_tracks_preserve_missing_assets_and_unknown_authors_without_fabricated_references() {
    let mut s = song(0);
    s["audio_info"] = json!({});
    s["authors"] = json!([{"author_id":0,"author_name":"A"},{"author_id":0,"author_name":"B"}]);
    let p = parse(&page(vec![s], 1), 1).unwrap();
    let t = &p.items[0];
    assert_eq!(t.duration_ms, None);
    assert!(t.available_qualities.is_empty());
    assert!(!t.extensions.contains_key("qualities"));
    assert_eq!(t.artists.len(), 2);
    assert!(t.artists.iter().all(|a| a.resource_ref.is_none()));
    for h in [
        "bad",
        "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA ",
        "https://untrusted.invalid/",
    ] {
        let mut s = song(0);
        s["audio_info"]["hash_320"] = json!(h);
        assert!(parse(&page(vec![s], 1), 1).is_err());
    }
}
