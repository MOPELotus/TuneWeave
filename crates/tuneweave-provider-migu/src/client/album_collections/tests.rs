use super::*;

pub(crate) fn entry(kind: AlbumKind, id: &str) -> serde_json::Value {
    let mut value = json!({"resourceType":kind.resource_type(),"title":format!("Album {id}"),"singer":"Artist A|Artist B","singerId":"11|22","imgItems":[{"img":"https://d.musicapp.migu.cn/data/oss/cover.webp"}],"pacmtoken":"never-export","purchased":true,"price":"100"});
    match kind {
        AlbumKind::Ordinary => value["albumId"] = json!(id),
        AlbumKind::Digital => {
            value["contentId"] = json!(id);
            value["albumId"] = json!("999999");
        }
    }
    value
}
fn parse(value: serde_json::Value) -> Result<CollectionPage> {
    parse_page(value.as_object().unwrap().clone(), "111")
}
#[test]
fn mixed_album_collection_keeps_same_id_resource_types_and_pairs_artists_without_inventing_purchases()
 {
    let page=parse(json!({"collections":[entry(AlbumKind::Ordinary,"77"),entry(AlbumKind::Digital,"77")],"totalCount":"2","hasNext":false})).unwrap();
    assert_eq!(page.total, Some(2));
    assert_eq!(page.has_next, Some(false));
    assert_eq!(page.items[0].identity(), ("2003", "77"));
    assert_eq!(page.items[1].identity(), ("5", "77"));
    for item in page.items {
        let (artists, extensions) = match item {
            CollectedAlbum::Ordinary(album) => {
                assert_eq!(album.track_count, None);
                assert_eq!(album.company, None);
                (album.artists, album.extensions)
            }
            CollectedAlbum::Digital(album) => {
                assert_eq!(album.purchased, None);
                assert_eq!(album.price, None);
                assert_eq!(album.purchasable, None);
                (album.artists, album.extensions)
            }
        };
        assert_eq!(
            artists
                .iter()
                .map(|v| v.resource_ref.as_ref().unwrap().to_string())
                .collect::<Vec<_>>(),
            ["migu:11", "migu:22"]
        );
        assert_eq!(artists[1].name, "Artist B");
        assert_eq!(extensions["source_user_id"], "111");
        assert!(
            !serde_json::to_string(&extensions)
                .unwrap()
                .contains("never-export")
        );
    }
}
#[test]
fn collection_parser_rejects_unknown_types_missing_id_pairs_invalid_images_and_unbounded_pages() {
    for (field, value) in [
        ("resourceType", json!("2021")),
        ("albumId", json!(null)),
        ("albumId", json!("077")),
        ("title", json!(" ")),
        ("singerId", json!("11")),
        ("singerId", json!("11|bad")),
        ("singer", json!(null)),
        ("singer", json!("A||B")),
        ("imgItems", json!([{"img":"https://outside.invalid/cover"}])),
        ("imgItems", json!([{"img":17}])),
    ] {
        let mut item = entry(AlbumKind::Ordinary, "77");
        item[field] = value;
        assert!(parse(json!({"collections":[item]})).is_err(), "{field}");
    }
    let mut wrong = entry(AlbumKind::Digital, "77");
    wrong.as_object_mut().unwrap().remove("contentId");
    assert!(parse(json!({"collections":[wrong]})).is_err());
    for v in [
        json!({}),
        json!({"collections":null}),
        json!({"collections":[],"totalCount":-1}),
        json!({"collections":[],"totalCount":641}),
        json!({"collections":[],"totalCount":"0x1"}),
        json!({"collections":[],"hasNext":0}),
        json!({"collections":vec![entry(AlbumKind::Ordinary,"77");11]}),
    ] {
        assert!(parse(v).is_err());
    }
    let page = parse(json!({"collections":[],"totalCount":null})).unwrap();
    assert_eq!(page.total, None);
    assert!(page.items.is_empty());
}
#[test]
fn collection_metadata_and_state_require_the_exact_requested_album_kind_and_id() {
    use crate::client::playlist_collection::{Kind, state};
    for kind in [AlbumKind::Ordinary, AlbumKind::Digital] {
        assert_eq!(title(kind, entry(kind, "77"), "77").unwrap(), "Album 77");
        assert!(title(kind, entry(kind, "78"), "77").is_err());
        let other = if kind == AlbumKind::Ordinary {
            AlbumKind::Digital
        } else {
            AlbumKind::Ordinary
        };
        assert!(title(kind, entry(other, "77"), "77").is_err());
        assert!(
            state(
                vec![json!({"isOP":"00","resourceType":other.resource_type()})],
                "77",
                Kind::Album(kind)
            )
            .is_err()
        );
        for (flag, expected) in [("00", true), ("01", false)] {
            assert_eq!(
                state(
                    vec![
                        json!({"isOP":flag,"resourceId":"77","resourceType":kind.resource_type()})
                    ],
                    "77",
                    Kind::Album(kind)
                )
                .unwrap(),
                expected
            );
        }
        assert!(state(vec![json!({"isOP":"02"})], "77", Kind::Album(kind)).is_err());
    }
}
