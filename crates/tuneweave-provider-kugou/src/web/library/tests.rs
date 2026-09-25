use super::*;
use serde_json::json;

#[test]
fn legacy_web_library_parser_preserves_opaque_identity_order_and_storage_units() {
    let body = json!({"totalSize":"5368709120","list":[
        {"listID":"007","listName":"Same name"},
        {"listID":7,"listName":"Same name"},
        {"listID":"folder:中文 /?","listName":"Third","token":"not exposed"}
    ]});
    let (items, bytes) = parse(body.to_string().as_bytes(), "111").unwrap();
    assert_eq!(bytes, 5 * 1024 * 1024 * 1024);
    assert_eq!(items.len(), 3);
    for (item, raw) in items.iter().zip(["007", "7", "folder:中文 /?"]) {
        let (uid, decoded) = parse_reference(&item.id).unwrap();
        assert_eq!(uid, "111");
        assert_eq!(decoded, raw);
        assert_eq!(item.resource_ref.id(), item.id);
        assert!(item.creator.is_none());
        assert!(item.track_count.is_none());
        assert!(item.subscribed.is_none());
        assert!(item.cover_url.is_none());
        assert_eq!(item.extensions.len(), 2);
    }
    assert!(
        !serde_json::to_string(&items)
            .unwrap()
            .contains("not exposed")
    );
    assert_eq!(items[0].name, items[1].name);
    assert_ne!(items[0].id, items[1].id);
}

#[test]
fn legacy_web_library_parser_rejects_partial_ambiguous_or_business_error_directories() {
    for body in [
        json!(null),
        json!("fail"),
        json!([]),
        json!({"errno":105}),
        json!({"totalSize":0,"list":[],"status":0}),
        json!({"totalSize":-1,"list":[]}),
        json!({"totalSize":"01","list":[]}),
        json!({"totalSize":0,"list":[{"listID":1,"listName":"A"},{"listID":"1","listName":"B"}]}),
        json!({"totalSize":0,"list":[{"listID":true,"listName":"A"}]}),
        json!({"totalSize":0,"list":[{"listID":" ","listName":"A"}]}),
        json!({"totalSize":0,"list":[{"listID":"1","listName":"A\nB"}]}),
        json!({"totalSize":0,"list":[{"listID":"1","listName":"A"},{"listID":2}]}),
    ] {
        assert_eq!(
            parse(body.to_string().as_bytes(), "111").unwrap_err().code,
            ErrorCode::UpstreamError
        );
    }
    assert!(parse(br#"{"totalSize":0,"list":[],"list":[]}"#, "111").is_err());
    assert!(parse(&vec![b' '; LIMIT + 1], "111").is_err());
    let body = json!({"totalSize":0,"list":(0..=MAX_LISTS).map(|i|
        json!({"listID":i,"listName":"A"})).collect::<Vec<_>>()});
    assert!(parse(body.to_string().as_bytes(), "111").is_err());
    let (items, bytes) = parse(br#"{"totalSize":42,"list":[]}"#, "111").unwrap();
    assert!(items.is_empty());
    assert_eq!(bytes, 42);
}

#[test]
fn legacy_web_library_references_never_alias_native_or_public_playlists() {
    for id in [
        "1",
        "cloudlist:111:0:1",
        "legacy_web_collection:111:",
        "legacy_web_collection:011:MQ",
        "legacy_web_collection:111:MQ==",
        "legacy_web_collection:111:MQ:extra",
        "legacy_web_collection:111:AA",
    ] {
        assert_eq!(
            parse_reference(id).unwrap_err().code,
            ErrorCode::InvalidRequest
        );
    }
    assert_eq!(
        parse_reference("legacy_web_collection:111:MQ").unwrap(),
        ("111", "1".into())
    );
}

#[test]
fn legacy_web_track_parser_preserves_occurrences_and_source_units() {
    let body = br#"[
        {"fileHash":"same-hash","fileName":"First","fileTimeLen":"65000"},
        {"fileHash":"same-hash","fileName":"Second","fileTimeLen":78000},
        {"fileHash":"other","fileName":"Third","fileTimeLen":0,"ignored":"private"}
    ]"#;
    let rows = parse_tracks(body).unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].file_hash, "same-hash");
    assert_eq!(rows[0].file_name, "First");
    assert_eq!(rows[0].duration_ms, 65_000);
    assert_eq!(rows[1].file_hash, "same-hash");
    assert_eq!(rows[1].file_name, "Second");
    assert_eq!(rows[1].duration_ms, 78_000);
    assert_eq!(rows[2].duration_ms, 0);
}

#[test]
fn legacy_web_track_parser_rejects_incomplete_or_ambiguous_arrays() {
    for body in [
        json!(null),
        json!("fail"),
        json!({"data": []}),
        json!([{"fileHash":"h","fileName":"name"}]),
        json!([{"fileHash":"h","fileName":"name","fileTimeLen":-1}]),
        json!([{"fileHash":"h","fileName":"name","fileTimeLen":"01"}]),
        json!([{"fileHash":" ","fileName":"name","fileTimeLen":1}]),
        json!([{"fileHash":"h","fileName":"name\nleak","fileTimeLen":1}]),
        json!([{"fileHash":"h","fileName":" ","fileTimeLen":1}]),
    ] {
        assert!(parse_tracks(body.to_string().as_bytes()).is_err());
    }
    let oversized = json!(
        (0..=MAX_TRACKS)
            .map(|i| json!({"fileHash":format!("h{i}"),"fileName":"name","fileTimeLen":1}))
            .collect::<Vec<_>>()
    );
    assert!(parse_tracks(oversized.to_string().as_bytes()).is_err());
    assert!(parse_tracks(&vec![b' '; LIMIT + 1]).is_err());
}
