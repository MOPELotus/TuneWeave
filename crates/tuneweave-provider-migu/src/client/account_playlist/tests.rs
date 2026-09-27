use super::*;

pub(crate) fn metadata(id: &str, owner: &str, total: usize) -> serde_json::Value {
    json!({"resourceType":"2021","musicListId":id,"title":"Actual playlist name",
        "ownerId":owner,"ownerName":"Owner","musicNum":total,"publishTime":"20260915000000",
        "summary":"Description","type":"1","tags":[],"officialConfig":{"actionUrl":"https://music.migu.cn/?token=do-not-export"},
        "pacmtoken":"do-not-export","originalImgUrl":"https://d.musicapp.migu.cn/data/oss/cover.png"})
}
pub(crate) fn page(ids: &[u32], total: usize) -> serde_json::Value {
    json!({"playlistId":"77","ownerId":"111","totalCount":total,"publishTime":"20260915000000",
        "songList":ids.iter().map(|id|json!({"resourceType":"2","contentId":id.to_string(),
            "songId":format!("s{id}"),"copyrightId":format!("c{id}"),"songName":format!("Song {id}"),
            "singerList":[{"id":"123","name":"Artist"}],"duration":180,"pacmtoken":"do-not-export"
        })).collect::<Vec<_>>()})
}
pub(crate) fn home(id: &str) -> serde_json::Value {
    json!({"userId":"111","userPrivateItems":[
        {"title":"Other navigation","actionUrl":"https://music.migu.cn/"},
        {"title":" 喜欢的音乐 ","actionUrl":format!("mgmusic://navigation?musicListId={id}&ignored=do-not-export")}
    ],"myCreatedMusicLists":{"createdMusicLists":[{"musicListId":"999","title":"喜欢的音乐"}]}})
}

#[test]
fn native_playlist_metadata_readback_preserves_multiline_description() {
    let mut value = metadata("77", "111", 0);
    value["summary"] = json!("First\r\n第二行\t & + = # %\0");
    assert_eq!(
        detail(value, "77").unwrap().description,
        "First\r\n第二行\t & + = # %"
    );
}
#[test]
fn favorite_identity_is_taken_only_from_unique_official_private_navigation() {
    assert_eq!(favorite_id(home("77")).unwrap(), "77");
    // The URI is parsed solely as data. A target outside Migu must never be
    // followed or exported; only this validated ID reaches fixed API endpoints.
    let mut body = home("77");
    body["userPrivateItems"][1]["actionUrl"] =
        json!("https://never-fetch.invalid/ignored?musicListId=77&token=secret");
    assert_eq!(favorite_id(body).unwrap(), "77");
    for data in [
        json!({}),
        json!({"userPrivateItems":null}),
        json!({"userPrivateItems":[]}),
        json!({"userPrivateItems":[],"myCreatedMusicLists":{"createdMusicLists":[{"title":"喜欢的音乐","musicListId":"77"}]}}),
        json!({"userPrivateItems":[{"title":"喜欢的音乐","actionUrl":"https://music.migu.cn/?musicListId=77"},{"title":"喜欢的音乐","actionUrl":"https://music.migu.cn/?musicListId=77"}]}),
    ] {
        assert!(favorite_id(data).is_err());
    }
    for url in [
        "relative?musicListId=77",
        "https://music.migu.cn/?musicListId=77&musicListId=88",
        "https://music.migu.cn/?id=77",
        "https://music.migu.cn/?musicListId=077",
        "https://music.migu.cn/?musicListId=bad/id",
        "https://user:pass@music.migu.cn/?musicListId=77",
        "javascript:noop?musicListId=77",
        "https://music.migu.cn/?musicListId=77#musicListId=88",
    ] {
        let mut data = home("77");
        data["userPrivateItems"][1]["actionUrl"] = json!(url);
        assert!(favorite_id(data).is_err(), "{url}");
    }
}

#[test]
fn write_favorite_identity_is_optional_only_when_navigation_omits_its_action() {
    assert_eq!(
        favorite_id_for_write(home("77")).unwrap(),
        Some("77".into())
    );

    for action in [json!(null), json!(""), json!("  ")] {
        let mut data = home("77");
        data["userPrivateItems"][1]["actionUrl"] = action;
        assert_eq!(favorite_id_for_write(data.clone()).unwrap(), None);
        assert_eq!(
            favorite_id(data).unwrap_err().code,
            ErrorCode::CapabilityNotSupported
        );
    }

    assert_eq!(
        favorite_id_for_write(json!({"userPrivateItems":[]})).unwrap(),
        None
    );
    for action in [
        "relative?musicListId=77",
        "https://music.migu.cn/?musicListId=77&musicListId=88",
        "https://music.migu.cn/?id=77",
    ] {
        let mut data = home("77");
        data["userPrivateItems"][1]["actionUrl"] = json!(action);
        assert!(favorite_id_for_write(data).is_err(), "{action}");
    }
}

#[test]
fn favorite_identity_diagnostic_reports_shape_without_account_values() {
    let data = json!({
        "userId":"private-user-id",
        "userPrivateItems":[{
            "title":"喜欢的音乐",
            "actionUrl":"https://music.migu.cn/?musicListId=77&pacmtoken=private-token"
        }],
        "myCreatedMusicLists":{"createdMusicLists":[]}
    });
    let diagnostic = format!("{:?}", safe_home_diagnostic(&data));
    assert!(diagnostic.contains("userPrivateItems"));
    assert!(diagnostic.contains("favorite_navigation_count: Some(1)"));
    assert!(diagnostic.contains("actionUrl:string"));
    assert!(diagnostic.contains("favorite_action_url_string_count: Some(1)"));
    assert!(!diagnostic.contains("private-user-id"));
    assert!(!diagnostic.contains("private-token"));
    assert!(!diagnostic.contains("musicListId=77"));
    assert!(!diagnostic.contains("喜欢的音乐"));
}
#[test]
fn account_playlist_metadata_is_typed_and_does_not_export_account_navigation() {
    let p = detail(metadata("77", "111", 3), "77").unwrap();
    assert_eq!(p.track_count, Some(3));
    assert_eq!(p.creator.unwrap().resource_ref, None);
    assert_eq!(p.extensions["owner_id"], "111");
    assert!(
        !serde_json::to_string(&p.extensions)
            .unwrap()
            .contains("do-not-export")
    );
    for (field, value) in [
        ("musicNum", json!(null)),
        ("musicNum", json!(10001)),
        ("ownerId", json!("bad id")),
        ("musicListId", json!("88")),
        ("resourceType", json!("2003")),
        ("title", json!("")),
    ] {
        let mut v = metadata("77", "111", 3);
        v[field] = value;
        assert!(detail(v, "77").is_err(), "{field}");
    }
}
#[test]
fn account_playlist_tracks_preserve_duplicate_positions_and_require_explicit_arrays() {
    let parsed = tracks(page(&[1, 2, 1], 3), "77", 1).unwrap();
    assert_eq!(
        parsed
            .page
            .tracks
            .iter()
            .map(|t| t.id.as_str())
            .collect::<Vec<_>>(),
        vec!["1", "2", "1"]
    );
    for v in [
        json!({"totalCount":0}),
        json!({"totalCount":0,"songList":null}),
        json!({"totalCount":0,"songList":[],"playlistId":"88"}),
        json!({"totalCount":1,"songList":[]}),
        json!({"totalCount":0,"songList":[],"ownerId":"bad id"}),
        page(&[1; 51], 51),
    ] {
        assert!(tracks(v, "77", 1).is_err());
    }
    assert!(
        tracks(page(&[], 0), "77", 1)
            .unwrap()
            .page
            .tracks
            .is_empty()
    );
}
