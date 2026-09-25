use super::*;

fn parse(section: Section, body: serde_json::Value) -> Result<LibraryPage> {
    parse_page(section, body.as_object().unwrap().clone(), "111")
}

#[test]
fn library_root_fields_preserve_selected_user_and_foreign_collection_owner() {
    for section in [Section::Created, Section::Saved] {
        let field = if section == Section::Created {
            "list"
        } else {
            "collections"
        };
        let owner = if section == Section::Created {
            "111"
        } else {
            "222"
        };
        let body = json!({field:[{"musicListId":"123","title":"Playlist","musicNum":"2","ownerId":owner,"ownerName":"Owner","imgItem":{"img":"https://d.musicapp.migu.cn/data/oss/cover.png"},"actionUrl":"do-not-export","pacmtoken":"do-not-export"}],"totalCount":"1"});
        let page = parse(section, body).unwrap();
        assert_eq!(page.total, Some(1));
        assert_eq!(page.items[0].track_count, Some(2));
        assert_eq!(page.items[0].extensions["owner_id"], owner);
        assert_eq!(page.items[0].extensions["source_user_id"], "111");
        assert_eq!(page.items[0].creator.as_ref().unwrap().resource_ref, None);
        assert!(
            !serde_json::to_string(&page.items)
                .unwrap()
                .contains("do-not-export")
        );
    }
    let page = parse(
        Section::Created,
        json!({"list":[{"musicListId":"123","title":"Unknown metadata"}]}),
    )
    .unwrap();
    assert_eq!(page.items[0].track_count, None);
    assert!(page.items[0].creator.is_none());
    assert!(!page.items[0].extensions.contains_key("owner_id"));
    assert_eq!(page.items[0].subscribed, None);
}

#[test]
fn library_rejects_missing_nested_malformed_and_overlarge_lists() {
    for body in [
        json!({}),
        json!({"data":{"list":[]}}),
        json!({"originData":{"list":[]}}),
        json!({"list":null}),
        json!({"list":{}}),
        json!({"list":[null]}),
        json!({"list":[],"totalCount":-1}),
        json!({"list":[],"totalCount":"1e3"}),
        json!({"list":[],"totalCount":1281}),
        json!({"list":[],"hasNext":"false"}),
        json!({"list":vec![json!({"musicListId":"1","title":"Name"});21]}),
        json!({"list":[{"musicListId":"1","title":"Name","ownerId":"222"}]}),
        json!({"list":[{"musicListId":"1","title":"Name","resourceType":"2003"}]}),
        json!({"list":[{"musicListId":"1","title":"Name","musicNum":false}]}),
        json!({"list":[{"musicListId":"1","title":"Name","imgItem":{"img":"https://evil.invalid/cover"}}]}),
        json!({"list":[{"musicListId":"bad/id","title":"Name"}]}),
        json!({"list":[{"musicListId":"1","title":"bad\nname"}]}),
    ] {
        assert!(parse(Section::Created, body.clone()).is_err(), "{body}");
    }
    assert!(
        parse(Section::Created, json!({"list":[]}))
            .unwrap()
            .items
            .is_empty()
    );
}
