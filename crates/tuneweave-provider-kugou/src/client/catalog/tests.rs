use super::*;

fn body(items: Vec<Value>, total: u64, page: u32) -> Value {
    json!({"status":1,"error_code":0,"data":{
        "page":page,"pagesize":20,"from":u64::from(page-1)*20,"size":20,"total":total,"lists":items
    }})
}
fn album(id: u64) -> Value {
    json!({"albumid":id,"albumname":"Album"})
}
fn artist(id: u64) -> Value {
    json!({"AuthorId":id,"AuthorName":"Artist"})
}
fn decode(kind: CatalogKind, value: &Value, page: u32) -> Result<CatalogPage> {
    parse(kind, value.to_string().as_bytes(), page)
}

#[test]
fn catalogue_maps_real_multi_artist_and_global_playlist_identities_without_guessing_rights() {
    let a = json!({"albumid":960399,"albumname":"<em>Album</em>","singerid":"0","singer":"A、B",
        "singers":[{"name":"A","id":3520},{"name":"B","id":"3521"}],"songcount":11,
        "img":"http://imge.kugou.com/stdmusic/{size}/cover.jpg","language":"华语",
        "publish_time":"2008-10-15","company":"Publisher","privilege":10,
        "jump_url":"https://untrusted.invalid/","token":"do-not-export"});
    let p = decode(CatalogKind::Album, &body(vec![a], 1, 1), 1).unwrap();
    let SearchItem::Album(a) = &p.items[0] else {
        panic!()
    };
    assert_eq!(a.id, "960399");
    assert_eq!(a.name, "Album");
    assert_eq!(
        a.artists
            .iter()
            .map(|a| a.resource_ref.as_ref().unwrap().id())
            .collect::<Vec<_>>(),
        ["3520", "3521"]
    );
    assert_eq!(a.track_count, Some(11));
    assert_eq!(a.kind, None);
    assert_eq!(
        a.cover_url.as_deref(),
        Some("https://imge.kugou.com/stdmusic/400/cover.jpg")
    );
    assert!(!serde_json::to_string(a).unwrap().contains("do-not-export"));
    let a = json!({"AuthorId":3520,"AuthorName":"A","Identity":1135,"VideoCount":29,"AudioCount":40,"FansNum":"99","Avatar":"https://evil.invalid/a"});
    let p = decode(CatalogKind::Artist, &body(vec![a], 1, 1), 1).unwrap();
    let SearchItem::Artist(a) = &p.items[0] else {
        panic!()
    };
    assert_eq!(a.track_count, Some(40));
    assert_eq!(a.video_count, Some(29));
    assert_eq!(a.mv_count, None);
    assert!(a.identities.is_empty());
    assert_eq!(a.avatar_url, None);
    assert_eq!(a.extensions["identity_code"], 1135);
    let a = json!({"gid":"collection_3_222_4_0","specialid":999,"specialname":"P","suid":"222","nickname":"Owner","song_count":0,"tag_str":"rock,pop"});
    let p = decode(CatalogKind::Playlist, &body(vec![a], 1, 1), 1).unwrap();
    let SearchItem::Playlist(a) = &p.items[0] else {
        panic!()
    };
    assert_eq!(a.id, "collection_3_222_4_0");
    assert_eq!(a.extensions["special_id"], 999);
    assert_eq!(a.creator.as_ref().unwrap().resource_ref, None);
    assert_eq!(a.extensions["owner_id"], "222");
    assert_eq!(a.subscribed, None);
    assert_eq!(a.created_at, None);
    assert!(a.tags.is_empty());
}

#[test]
fn catalogue_pagination_requires_exact_echo_and_physical_count_including_tail_and_empty_pages() {
    for (total, page, count) in [(28, 1, 20), (28, 2, 8), (28, 3, 0), (0, 1, 0)] {
        let v = body((1..=count).map(artist).collect(), total, page);
        assert_eq!(
            decode(CatalogKind::Artist, &v, page).unwrap().items.len(),
            count as usize
        );
    }
    let base = body((1..=20).map(album).collect(), 40, 1);
    for (key, value) in [
        ("page", json!(2)),
        ("pagesize", json!(100)),
        ("from", json!(1)),
        ("size", json!(19)),
        ("total", json!(19)),
        ("lists", json!([])),
    ] {
        let mut v = base.clone();
        v["data"][key] = value;
        assert!(decode(CatalogKind::Album, &v, 1).is_err(), "{key}");
    }
    let mut v = base.clone();
    v["data"]["lists"].as_array_mut().unwrap().pop();
    assert!(decode(CatalogKind::Album, &v, 1).is_err());
    let mut v = base.clone();
    v["data"].as_object_mut().unwrap().remove("total");
    assert!(decode(CatalogKind::Album, &v, 1).is_err());
}

#[test]
fn catalogue_rejects_wrong_types_duplicate_fields_and_bad_ids_instead_of_empty_success() {
    for v in [
        json!(0),
        json!(-1),
        json!(true),
        json!(1.5),
        json!("01"),
        json!("+1"),
        json!(" 1"),
        json!("18446744073709551616"),
    ] {
        let mut a = album(1);
        a["albumid"] = v;
        assert!(decode(CatalogKind::Album, &body(vec![a], 1, 1), 1).is_err());
    }
    assert!(decode(CatalogKind::Album, &body(vec![album(1), album(1)], 2, 1), 1).is_err());
    assert!(decode(CatalogKind::Artist, &body(vec![album(1)], 1, 1), 1).is_err());
    assert!(
        decode(
            CatalogKind::Playlist,
            &body(vec![json!({"specialid":1,"specialname":"P"})], 1, 1),
            1
        )
        .is_err()
    );
    let mut a = album(1);
    a["singers"] = json!([{"id":2,"name":"A"},{"id":2,"name":"B"}]);
    assert!(decode(CatalogKind::Album, &body(vec![a], 1, 1), 1).is_err());
    let v = body(vec![album(1)], 1, 1).to_string();
    for bytes in [
        v.replace("\"albumid\":1", "\"albumid\":1,\"albumid\":2"),
        v.replace("\"total\":1", "\"total\":1,\"total\":1"),
        v.replace("\"status\":1", "\"status\":1,\"status\":1"),
    ] {
        assert!(parse(CatalogKind::Album, bytes.as_bytes(), 1).is_err());
    }
    for data in [json!([]), json!("private-account-error"), json!(null)] {
        let v =
            json!({"status":0,"error_code":20006,"error_msg":"private-account-error","data":data});
        let e = decode(CatalogKind::Album, &v, 1).err().unwrap();
        assert_eq!(e.details["platform_code"], 20006);
        assert!(!format!("{e:?}").contains("private-account-error"));
    }
}

#[test]
fn catalogue_optional_fields_stay_unknown_and_correction_metadata_is_bounded() {
    let mut v = body(vec![album(1)], 1, 1);
    v["data"]["correctiontype"] = json!(1);
    v["data"]["correctiontip"] = json!("Corrected");
    let p = decode(CatalogKind::Album, &v, 1).unwrap();
    assert_eq!(p.extensions["correction_tip"], "Corrected");
    let SearchItem::Album(a) = &p.items[0] else {
        panic!()
    };
    assert!(a.artists.is_empty());
    assert_eq!(a.track_count, None);
    assert_eq!(a.published_at, None);
    v["data"]["correctiontip"] = json!("x".repeat(513));
    assert!(decode(CatalogKind::Album, &v, 1).is_err());
    let mut a = album(1);
    a["singer"] = json!("Unknown ID");
    a["singerid"] = json!("0");
    let p = decode(CatalogKind::Album, &body(vec![a], 1, 1), 1).unwrap();
    let SearchItem::Album(a) = &p.items[0] else {
        panic!()
    };
    assert_eq!(a.artists[0].resource_ref, None);
}

#[test]
fn catalogue_preserves_distinct_singer_names_when_the_official_search_omits_their_ids() {
    let mut a = album(1);
    a["singers"] = json!([{"name":"A","id":0},{"name":"B","id":0},{"name":"C","id":3}]);
    let page = decode(CatalogKind::Album, &body(vec![a], 1, 1), 1).unwrap();
    let SearchItem::Album(a) = &page.items[0] else {
        panic!()
    };
    assert_eq!(
        a.artists
            .iter()
            .map(|v| v.name.as_str())
            .collect::<Vec<_>>(),
        ["A", "B", "C"]
    );
    assert_eq!(a.artists[0].resource_ref, None);
    assert_eq!(a.artists[1].resource_ref, None);
    assert_eq!(a.artists[2].resource_ref.as_ref().unwrap().id(), "3");
}
