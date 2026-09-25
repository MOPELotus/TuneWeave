use super::*;

pub(crate) fn detail(tracks: u64, albums: u64) -> Value {
    json!({"status":1,"error_code":0,"data":{
        "author_id":"42","author_name":"Artist","intro":"Brief biography",
        "long_intro":[{"title":"简介","content":"Full biography\nSecond paragraph"},{"title":"作品","content":"Catalogue notes"}],
        "sizable_avatar":"http://singerimg.kugou.com/a/{size}.jpg",
        "song_count":tracks,"album_count":albums,"mv_count":4,"fansnums":99,"birthday":"1980-01-01",
        "area_id":"3","is_publish":1,"user_status":0
    }})
}
pub(crate) fn song(n: u64) -> Value {
    json!({"album_audio_id":1000+n,"audio_id":2000+n,"audio_group_id":77,
        "author_id":7,"audio_name":format!("Song {n}"),"album_id":900,
        "album_info":{"album_name":"Album","cover":"http://imge.kugou.com/a.jpg"},
        "authors":[{"base":{"author_id":7,"author_name":"Collaborator"}},{"base":{"author_id":42,"author_name":"Artist"}}],
        "audio_info":{"hash":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA","filesize":1234,"timelength":245123,"bitrate":128,"extname":"mp3",
            "hash_super":"BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB","timelength_super":245100,"filesize_super":50000,"extname_super":"dff","bitrate_super":5644},
        "publish_date":"2020-01-02","copyright":{"privilege":10},"mvhash":"unrelated-video-hash","goods_info":{"could_buy_type":"audio"}
    })
}
pub(crate) fn tracks(page: u32, total: u64, sort: u8) -> Value {
    let offset = u64::from(page - 1) * 100;
    json!({"status":1,"error_code":0,"errcode":0,"extra":{"page_total":total},"data":{
        "total":total,"input_param":{"sort":sort,"author_identity":0},
        "songs":(offset..total.min(offset+100)).map(song).collect::<Vec<_>>()
    }})
}
pub(crate) fn albums(page: u32, total: u64) -> Value {
    let offset = u64::from(page - 1) * 30;
    json!({"status":1,"error_code":0,"total":total,"extra":{"page_total":total},"data":
        (offset..total.min(offset+30)).map(|i|json!({"album_id":900+i,"album_name":format!("Album {i}"),
            "authors":[{"author_id":42,"author_name":"Artist"}],"publish_date":"2020-01-02",
            "type":"EP专辑","publish_company":"Publisher","goods_info":{"album_price":1000}
        })).collect::<Vec<_>>()
    })
}

#[test]
fn artist_detail_preserves_biography_sections_counts_and_unknown_fields() {
    let mut v = detail(105, 31);
    let a = parse_artist(v.to_string().as_bytes(), 42).unwrap();
    assert_eq!(a.resource_ref.id(), "42");
    assert_eq!(a.biography_sections.len(), 2);
    assert_eq!(a.description, "Brief biography");
    assert_eq!(
        a.biography_sections[0].text,
        "Full biography\nSecond paragraph"
    );
    assert_eq!(a.track_count, Some(105));
    assert_eq!(a.mv_count, Some(4));
    assert_eq!(a.video_count, None);
    assert_eq!(
        a.avatar_url.as_deref(),
        Some("https://singerimg.kugou.com/a/400.jpg")
    );
    assert!(!a.extensions.contains_key("user_status"));
    assert!(!a.extensions.contains_key("is_followed"));
    for key in [
        "song_count",
        "album_count",
        "mv_count",
        "fansnums",
        "long_intro",
        "intro",
    ] {
        v["data"].as_object_mut().unwrap().remove(key);
    }
    let a = parse_artist(v.to_string().as_bytes(), 42).unwrap();
    assert_eq!(a.track_count, None);
    assert_eq!(a.album_count, None);
    assert!(a.biography_sections.is_empty());
}

#[test]
fn artist_detail_rejects_foreign_identity_duplicate_fields_and_excessive_biography() {
    for id in [json!(0), json!(43), json!("042"), json!(true)] {
        let mut v = detail(1, 1);
        v["data"]["author_id"] = id;
        assert!(parse_artist(v.to_string().as_bytes(), 42).is_err());
    }
    let wire = detail(1, 1).to_string().replace(
        "\"author_id\":\"42\"",
        "\"author_id\":\"42\",\"author_id\":\"42\"",
    );
    assert!(parse_artist(wire.as_bytes(), 42).is_err());
    let mut v = detail(1, 1);
    v["data"]["long_intro"] = json!([{"title":"Large","content":"x".repeat(262145)}]);
    assert!(parse_artist(v.to_string().as_bytes(), 42).is_err());
    v["data"] = json!([]);
    assert!(parse_artist(v.to_string().as_bytes(), 42).is_err());
}

#[test]
fn artist_tracks_preserve_nested_collaborators_versions_milliseconds_and_dff_assets() {
    let (items, total) = parse_tracks(tracks(1, 2, 1).to_string().as_bytes(), 42, 1, 1).unwrap();
    assert_eq!(total, 2);
    let t = &items[0];
    assert_eq!(t.id, "1000");
    assert_eq!(t.artists.len(), 2);
    assert_eq!(t.artists[0].resource_ref.as_ref().unwrap().id(), "7");
    assert_eq!(t.artists[1].resource_ref.as_ref().unwrap().id(), "42");
    assert_eq!(t.duration_ms, Some(245123));
    assert_eq!(t.extensions["qualities"]["master"]["format"], "dff");
    assert_eq!(t.extensions["qualities"]["master"]["duration_ms"], 245100);
    assert_eq!(t.extensions["primary_author_id"], "7");
    assert_eq!(t.playable, None);
    assert_eq!(
        items[0].extensions["audio_group_id"],
        items[1].extensions["audio_group_id"]
    );
    assert_ne!(items[0].id, items[1].id);
    let wire = serde_json::to_string(t).unwrap();
    assert!(!wire.contains("unrelated-video-hash"));
    assert!(!wire.contains("could_buy_type"));
}

#[test]
fn artist_tracks_keep_missing_album_and_asset_metadata_without_fabricating_an_identity() {
    let mut v = tracks(1, 1, 1);
    let s = &mut v["data"]["songs"][0];
    s["album_id"] = json!(0);
    s.as_object_mut().unwrap().remove("album_info");
    s["audio_info"] = json!({});
    s["authors"]
        .as_array_mut()
        .unwrap()
        .push(json!({"base":{"author_id":0,"author_name":"Unknown collaborator"}}));
    let (items, _) = parse_tracks(v.to_string().as_bytes(), 42, 1, 1).unwrap();
    let t = &items[0];
    assert_eq!(t.album, None);
    assert_eq!(t.duration_ms, None);
    assert!(t.available_qualities.is_empty());
    assert!(!t.extensions.contains_key("qualities"));
    assert_eq!(t.artists[2].resource_ref, None);
}

#[test]
fn artist_tracks_reject_wrong_sort_counts_authors_and_conflicting_duration_aliases() {
    for case in 0..8 {
        let mut v = tracks(1, 1, 1);
        match case {
            0 => v["data"]["input_param"]["sort"] = json!(2),
            1 => v["extra"]["page_total"] = json!(2),
            2 => v["data"]["total"] = json!(101),
            3 => v["data"]["songs"][0]["authors"][1]["base"]["author_id"] = json!(43),
            4 => v["data"]["songs"][0]["authors"][1]["base"]["author_id"] = json!(7),
            5 => v["data"]["songs"][0]["album_audio_id"] = json!(0),
            6 => v["data"]["songs"][0]["audio_info"]["duration"] = json!(999),
            7 => v["data"]["songs"][0]["audio_info"]["hash_super"] = json!("bad"),
            _ => unreachable!(),
        }
        assert!(
            parse_tracks(v.to_string().as_bytes(), 42, 1, 1).is_err(),
            "case {case}"
        );
    }
    for (p, total, count) in [(1, 0, 0), (2, 105, 5), (3, 105, 0)] {
        assert_eq!(
            parse_tracks(tracks(p, total, 2).to_string().as_bytes(), 42, p, 2)
                .unwrap()
                .0
                .len(),
            count
        );
    }
    assert!(parse_tracks(tracks(1, 12801, 1).to_string().as_bytes(), 42, 1, 1).is_err());
}

#[test]
fn artist_albums_reuse_album_identity_metadata_and_reject_foreign_authors_or_totals() {
    let (a, total) = parse_albums(albums(2, 31).to_string().as_bytes(), 42, 2).unwrap();
    assert_eq!(total, 31);
    assert_eq!(a[0].id, "930");
    assert_eq!(a[0].company.as_deref(), Some("Publisher"));
    assert_eq!(a[0].kind.as_deref(), Some("EP专辑"));
    assert_eq!(a[0].track_count, None);
    assert!(!serde_json::to_string(&a).unwrap().contains("album_price"));
    for case in 0..4 {
        let mut v = albums(1, 1);
        match case {
            0 => v["data"][0]["authors"][0]["author_id"] = json!(43),
            1 => v["extra"]["page_total"] = json!(2),
            2 => v["data"][0]["album_id"] = json!(0),
            3 => v["data"] = json!([]),
            _ => unreachable!(),
        }
        assert!(parse_albums(v.to_string().as_bytes(), 42, 1).is_err());
    }
}
