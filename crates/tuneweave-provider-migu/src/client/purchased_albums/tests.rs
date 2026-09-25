use super::*;

fn row(kind: &str) -> serde_json::Value {
    json!({"resourceType":kind,"contentId":"77","title":"Album","singer":"Artist A / Artist B","totalCount":"12","copyrightId":"ABC123","imgItems":[]})
}
fn page(rows: serde_json::Value, more: bool) -> Result<AlbumPage> {
    parse_page(json!({"resources":rows,"hasNextPage":more}), "111", 100)
}

#[test]
fn purchased_albums_distinguish_resource_types_and_do_not_invent_current_entitlements() {
    let result = page(json!([row("2003"), row("5"), row("2003")]), false).unwrap();
    assert_eq!(result.items.len(), 3);
    assert_eq!(result.items[0], result.items[2]);
    let ordinary = result.items[0].album.as_ref().unwrap();
    assert!(result.items[0].digital_album.is_none());
    assert_eq!(ordinary.id, "77");
    assert_eq!(ordinary.track_count, Some(12));
    assert_eq!(ordinary.artists[0].name, "Artist A / Artist B");
    assert!(ordinary.artists[0].resource_ref.is_none());
    assert!(result.items[1].album.is_none());
    let digital = result.items[1].digital_album.as_ref().unwrap();
    assert_eq!(digital.id, "77");
    assert_eq!(digital.track_count, Some(12));
    assert!(digital.purchased.is_none());
    assert!(digital.price.is_none());
    assert!(digital.purchasable.is_none());
    assert!(digital.is_free.is_none());
    assert!(digital.sale_count.is_none());
    assert_ne!(
        result.items[0].extensions["resource_type"],
        result.items[1].extensions["resource_type"]
    );
}

#[test]
fn purchased_albums_missing_titles_keep_records_and_missing_counts_remain_unknown() {
    for kind in ["2003", "5"] {
        for title in [json!(null), json!("")] {
            let mut r = row(kind);
            r["title"] = title;
            r.as_object_mut().unwrap().remove("totalCount");
            let result = page(json!([r]), false).unwrap();
            let item = &result.items[0];
            assert!(item.album.is_none() && item.digital_album.is_none());
            assert!(item.name.is_none());
            assert_eq!(item.extensions["resource_ref"], "migu:77");
            assert_eq!(item.extensions["catalogue_resolved"], false);
        }
        let mut r = row(kind);
        r.as_object_mut().unwrap().remove("totalCount");
        let result = page(json!([r]), false).unwrap();
        if kind == "5" {
            assert!(
                result.items[0]
                    .digital_album
                    .as_ref()
                    .unwrap()
                    .track_count
                    .is_none()
            );
        } else {
            assert!(
                result.items[0]
                    .album
                    .as_ref()
                    .unwrap()
                    .track_count
                    .is_none()
            );
        }
    }
}

#[test]
fn purchased_albums_reject_malformed_paging_types_identities_and_unsafe_metadata() {
    for value in [
        json!({}),
        json!({"resources":[]}),
        json!({"hasNextPage":false}),
        json!({"resources":null,"hasNextPage":false}),
        json!({"resources":[],"hasNextPage":"false"}),
        json!({"resources":[],"hasNextPage":true}),
    ] {
        assert!(parse_page(value, "111", 100).is_err());
    }
    assert!(page(json!([]), false).unwrap().items.is_empty());
    assert!(page(json!(vec![row("5"); PAGE_SIZE]), true).is_ok());
    assert!(page(json!(vec![row("5"); PAGE_SIZE + 1]), false).is_err());
    for (key, value) in [
        ("resourceType", json!("2037")),
        ("resourceType", json!(5)),
        ("contentId", json!("077")),
        ("contentId", json!(77)),
        ("contentId", json!("migu:77")),
        ("title", json!("a\nsecret")),
        ("singer", json!("x".repeat(8193))),
        ("copyrightId", json!("id/path")),
        ("totalCount", json!(-1)),
        ("totalCount", json!("unknown")),
        ("totalCount", json!("18446744073709551616")),
        (
            "imgItems",
            json!([{"img":"https://attacker.example/a.jpg"}]),
        ),
        ("imgItems", json!(vec![json!({}); 65])),
    ] {
        let mut r = row("5");
        r[key] = value;
        assert!(page(json!([r]), false).is_err(), "{key}");
    }
}
