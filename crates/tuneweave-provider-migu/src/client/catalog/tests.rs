use super::*;

fn parse(kind: CatalogKind, value: serde_json::Value) -> Result<CatalogPage> {
    parse_catalog_response(kind, &serde_json::to_vec(&value).unwrap())
}

#[test]
fn playlist_search_preserves_identity_unknown_counts_and_user_ownership() {
    let page = parse(CatalogKind::Playlist, json!({"code":"000000","data":{"hasNext":false,"items":[
        {"musicList":{"resourceType":"2021","musicListId":"225291289","title":" Playlist ","ownerId":"4d66fede-4ee3-49a9-b1f0-1f1160a5b180","ownerName":"Owner","publishTime":"20260904103307","originalImgUrl":"https://untrusted.example/a","imgItem":{"img":"/data/oss/resource/cover.webp"},"track":"private tracking","userIdentityInfoItems":[{"secret":"hidden"}]}},
        {"musicList":{"resourceType":"2021","musicListId":"12","title":"Empty","musicNum":0}}
    ]}})).unwrap();
    let SearchItem::Playlist(first) = &page.items[0] else {
        panic!("playlist expected")
    };
    assert_eq!(first.resource_ref.to_string(), "migu:225291289");
    assert_eq!(first.name, "Playlist");
    assert_eq!(first.track_count, None);
    let owner = first.creator.as_ref().unwrap();
    assert_eq!(owner.name, "Owner");
    assert!(owner.resource_ref.is_none());
    assert_eq!(
        first.extensions["owner_id"],
        "4d66fede-4ee3-49a9-b1f0-1f1160a5b180"
    );
    assert_eq!(first.extensions["publish_time"], "20260904103307");
    assert_eq!(
        first.cover_url.as_deref(),
        Some("https://d.musicapp.migu.cn/data/oss/resource/cover.webp")
    );
    assert!(first.subscribed.is_none());
    assert!(first.updated_at.is_none());
    let SearchItem::Playlist(empty) = &page.items[1] else {
        panic!("playlist expected")
    };
    assert_eq!(empty.track_count, Some(0));
    assert!(empty.creator.is_none());
    let encoded = serde_json::to_string(&page.items).unwrap();
    for raw in ["private tracking", "userIdentityInfoItems", "hidden"] {
        assert!(!encoded.contains(raw));
    }
}

#[test]
fn artist_search_maps_only_explicit_counts_and_preserves_multiline_biography() {
    let page = parse(CatalogKind::Artist,json!({"code":"000000","data":{"hasNext":false,"items":[
        {"singer":{"resourceType":"2002","singerId":"112","singer":"Artist","summary":" First\nSecond ","songNum":411,"albumNum":0,"mvNum":452,"artistNamePinyin":"not an alias","imgs":[{"img":"http://127.0.0.1/avatar"},{"img":"https://d.musicapp.migu.cn/data/oss/resource/artist.webp"}]}},
        {"singer":{"resourceType":"2002","singerId":"22","singer":"Unknown counts"}}
    ]}})).unwrap();
    let SearchItem::Artist(first) = &page.items[0] else {
        panic!("artist expected")
    };
    assert_eq!(first.resource_ref.to_string(), "migu:112");
    assert_eq!(first.description, "First\nSecond");
    assert_eq!(first.track_count, Some(411));
    assert_eq!(first.album_count, Some(0));
    assert_eq!(first.mv_count, Some(452));
    assert!(first.video_count.is_none());
    assert!(first.aliases.is_empty());
    assert!(first.identities.is_empty());
    assert!(first.cover_url.is_none());
    assert_eq!(
        first.avatar_url.as_deref(),
        Some("https://d.musicapp.migu.cn/data/oss/resource/artist.webp")
    );
    let SearchItem::Artist(unknown) = &page.items[1] else {
        panic!("artist expected")
    };
    assert!(unknown.track_count.is_none());
    assert!(unknown.album_count.is_none());
    assert!(unknown.mv_count.is_none());
    assert!(unknown.avatar_url.is_none());
}

#[test]
fn catalogue_empty_pages_require_explicit_success_and_termination() {
    for kind in [CatalogKind::Playlist, CatalogKind::Artist] {
        for data in [
            json!({"hasNext":false}),
            json!({"hasNext":false,"items":[]}),
        ] {
            let page = parse(kind, json!({"code":"000000","data":data})).unwrap();
            assert!(page.items.is_empty());
            assert!(!page.has_next);
        }
        for value in [
            json!({}),
            json!({"code":"000000"}),
            json!({"code":"000000","data":{}}),
            json!({"code":"000000","data":{"hasNext":true}}),
            json!({"code":"000000","data":{"hasNext":"false"}}),
            json!({"code":"000000","data":{"hasNext":false,"items":null}}),
            json!({"code":"123456","data":{"hasNext":false}}),
        ] {
            assert_eq!(
                parse(kind, value).err().unwrap().code,
                ErrorCode::UpstreamError
            );
        }
    }
}

#[test]
fn catalogue_rejects_wrong_types_identities_and_ambiguous_entries() {
    for (kind, field, base) in [
        (
            CatalogKind::Playlist,
            "musicList",
            json!({"resourceType":"2021","musicListId":"11","title":"Playlist"}),
        ),
        (
            CatalogKind::Artist,
            "singer",
            json!({"resourceType":"2002","singerId":"11","singer":"Artist"}),
        ),
    ] {
        let id_field = if field == "singer" {
            "singerId"
        } else {
            "musicListId"
        };
        let name_field = if field == "singer" { "singer" } else { "title" };
        for mutation in [
            "type",
            "id",
            "name",
            "missing",
            "other",
            "ambiguous",
            "count",
        ] {
            let mut item = json!({});
            item[field] = base.clone();
            match mutation {
                "type" => item[field]["resourceType"] = json!("2"),
                "id" => item[field][id_field] = json!("01"),
                "name" => item[field][name_field] = json!("\u{0000}"),
                "missing" => item = json!({}),
                "other" => item = json!({"song":{"resourceType":"2","contentId":"11"}}),
                "ambiguous" => {
                    item["musicList"] =
                        json!({"resourceType":"2021","musicListId":"11","title":"Playlist"});
                    item["singer"] =
                        json!({"resourceType":"2002","singerId":"11","singer":"Artist"});
                }
                "count" => {
                    item[field][if field == "singer" {
                        "songNum"
                    } else {
                        "musicNum"
                    }] = json!(-1)
                }
                _ => unreachable!(),
            }
            assert!(
                parse(
                    kind,
                    json!({"code":"000000","data":{"hasNext":false,"items":[item]}})
                )
                .is_err(),
                "{kind:?} {mutation}"
            );
        }
        for (count, more) in [(0, true), (19, true), (21, false)] {
            let mut item = json!({});
            item[field] = base.clone();
            assert!(
                parse(
                    kind,
                    json!({"code":"000000","data":{"hasNext":more,"items":vec![item;count]}})
                )
                .is_err()
            );
        }
    }
}
