use super::*;
use crate::account::tests::{frame, ok, request, server, session};

fn entry(id: u64, kind: u8) -> Value {
    json!({"listid":id,"type":kind,"status":2,"is_del":0,"name":"Synthetic playlist",
        "global_collection_id":format!("collection_3_123456789_{id}_0"),"list_ver":3,
        "list_create_userid":if kind==0 {123456789} else {222},
        "list_create_listid":if kind==0 {id} else {50},
        "list_create_gid":if kind==0 {format!("collection_3_123456789_{id}_0")} else {"collection_3_222_50_0".into()},
        "count":31,"m_count":33,"tags":"咖啡厅,下午茶,夜晚","sort":6,
        "pic":"https://c1.kgimg.com/custom/{size}/cover.jpg","list_create_username":"Creator",
        "is_pri":0,"is_publish":1,"is_drop":0,"create_time":1700000000})
}
fn fixture(items: Vec<Value>) -> Value {
    json!({"status":1,"error_code":0,"data":{"userid":123456789,"total_ver":4316,
        "list_count":166,"collect_count":47,"album_count":0,"info":items}})
}
fn decode(value: &Value) -> Result<LibraryPage> {
    parse(value.to_string().as_bytes(), "123456789", 1)
}

#[test]
fn visibility_library_retains_explicit_collaboration_without_inventing_missing_flags() {
    for flag in [json!(0), json!(1), json!("0"), json!("1")] {
        let mut row = entry(1, 0);
        row["is_mutual"] = flag.clone();
        let page = decode(&fixture(vec![row])).unwrap();
        assert_eq!(
            page.rows[0].playlist.as_ref().unwrap().extensions["is_mutual"],
            flag == json!(1) || flag == json!("1")
        );
    }
    let page = decode(&fixture(vec![entry(1, 0)])).unwrap();
    assert!(
        !page.rows[0]
            .playlist
            .as_ref()
            .unwrap()
            .extensions
            .contains_key("is_mutual")
    );
    for flag in [json!(2), json!(true), json!("01"), json!(-1)] {
        let mut row = entry(1, 0);
        row["is_mutual"] = flag;
        assert!(decode(&fixture(vec![row])).is_err());
    }
}

#[test]
fn library_preserves_local_source_system_versions_counts_and_actual_string_tags() {
    let mut default = entry(8, 0);
    default["is_def"] = json!(1);
    default["count"] = json!(0);
    let mut likes = entry(17, 0);
    likes["is_def"] = json!(2);
    let data = decode(&fixture(vec![default, likes, entry(160, 1)])).unwrap();
    let default = data.rows[0].playlist.as_ref().unwrap();
    assert_eq!(default.extensions["system_playlist"], "default_collection");
    assert_eq!(default.track_count, Some(0));
    let likes = data.rows[1].playlist.as_ref().unwrap();
    assert_eq!(likes.extensions["system_playlist"], "liked_tracks");
    assert_eq!(likes.id, "cloudlist:123456789:0:17");
    let saved = data.rows[2].playlist.as_ref().unwrap();
    assert_eq!(saved.id, "cloudlist:123456789:1:160");
    assert_eq!(saved.resource_ref.id(), saved.id);
    assert_eq!(saved.extensions["library_owner_id"], "123456789");
    assert_eq!(saved.extensions["source_user_id"], "222");
    assert_eq!(saved.extensions["source_list_id"], "50");
    assert_eq!(
        saved.extensions["global_collection_id"],
        "collection_3_123456789_160_0"
    );
    assert_eq!(
        saved.extensions["source_global_collection_id"],
        "collection_3_222_50_0"
    );
    assert_eq!(saved.extensions["status"], 2); // Observed metadata state; not a playback permission.
    assert_eq!(saved.extensions["m_count"], 33);
    assert_eq!(saved.track_count, Some(31));
    assert_eq!(saved.extensions["list_ver"], 3);
    assert_eq!(saved.extensions["total_ver"], 4316);
    assert_eq!(saved.tags, ["咖啡厅", "下午茶", "夜晚"]);
    assert_eq!(saved.subscribed, Some(true));
    assert!(saved.creator.as_ref().unwrap().resource_ref.is_none());
    assert_eq!(saved.created_at, None);
}

#[test]
fn library_keeps_deleted_physical_rows_and_unknown_fields_without_fake_empty_success() {
    let data = decode(&fixture(vec![
        json!({"listid":9,"type":0,"is_del":1}),
        entry(10, 0),
    ]))
    .unwrap();
    assert_eq!(data.rows.len(), 2);
    assert!(data.rows[0].playlist.is_none());
    assert_eq!(data.rows[0].list_id, 9);
    let data =
        decode(&json!({"status":1,"error_code":0,"data":{"total_ver":0,"info":[]}})).unwrap();
    assert!(data.rows.is_empty());
    let mut unknown = json!({"listid":1,"type":0,"name":"我喜欢","token":"do-not-export","unknown":{"cookie":"secret"}});
    let result = decode(&fixture(vec![unknown.clone()])).unwrap();
    let p = result.rows[0].playlist.as_ref().unwrap();
    assert_eq!(p.track_count, None);
    assert_eq!(p.subscribed, None);
    assert!(!p.extensions.contains_key("system_playlist"));
    assert!(!p.extensions.contains_key("global_collection_id"));
    assert!(!serde_json::to_string(p).unwrap().contains("do-not-export"));
    unknown["count"] = json!(0);
    unknown["m_count"] = json!(99);
    assert_eq!(
        decode(&fixture(vec![unknown])).unwrap().rows[0]
            .playlist
            .as_ref()
            .unwrap()
            .track_count,
        Some(0)
    );
    for data in [
        json!({}),
        json!({"info":[]}),
        json!({"total_ver":1}),
        json!(null),
        json!([]),
    ] {
        assert!(decode(&json!({"status":1,"error_code":0,"data":data})).is_err());
    }
}

#[test]
fn library_rejects_cross_account_echo_and_created_source_identity_conflicts() {
    let mut value = fixture(vec![entry(1, 0)]);
    value["data"]["userid"] = json!(222);
    assert_eq!(decode(&value).err().unwrap().code, ErrorCode::Conflict);
    for (field, value) in [
        ("list_create_userid", json!(222)),
        ("list_create_listid", json!(22)),
        ("list_create_gid", json!("different_gid")),
    ] {
        let mut row = entry(1, 0);
        row[field] = value;
        assert_eq!(
            decode(&fixture(vec![row])).err().unwrap().code,
            ErrorCode::Conflict
        );
    }
    // Opaque global IDs are never reverse-engineered into a UID or local list ID.
    let mut row = entry(160, 1);
    row["global_collection_id"] = json!("opaque_v9_collection");
    assert_eq!(decode(&fixture(vec![row])).unwrap().rows[0].list_id, 160);
}

#[test]
fn library_rejects_corrupt_numbers_duplicate_fields_and_inconsistent_physical_pages() {
    for number in [
        json!(0),
        json!(-1),
        json!(1.5),
        json!(true),
        json!("01"),
        json!("+1"),
        json!(" 1"),
        json!("18446744073709551616"),
    ] {
        let mut row = entry(1, 0);
        row["listid"] = number;
        assert!(decode(&fixture(vec![row])).is_err());
    }
    for (field, value) in [
        ("type", json!(2)),
        ("is_del", json!(2)),
        ("is_pri", json!(2)),
        ("tags", json!([])),
        ("name", json!(null)),
        ("pic", json!("https://evil.invalid/a")),
    ] {
        let mut row = entry(1, 0);
        row[field] = value;
        assert!(decode(&fixture(vec![row])).is_err(), "{field}");
    }
    assert!(decode(&fixture(vec![entry(1, 0), entry(1, 0)])).is_err());
    assert!(decode(&fixture((1..=31).map(|id| entry(id, 0)).collect())).is_err());
    assert!(decode(&fixture(vec![entry(1, 0), entry(1, 1)])).is_ok());
    for (key, value) in [("page", 2), ("pagesize", 100)] {
        let mut v = fixture(vec![]);
        v["data"][key] = json!(value);
        assert!(decode(&v).is_err());
    }
    let bytes = fixture(vec![entry(1, 0)]).to_string();
    for duplicate in [
        bytes.replace("\"status\":1", "\"status\":1,\"status\":1"),
        bytes.replace(
            "\"total_ver\":4316",
            "\"total_ver\":4316,\"total_ver\":4316",
        ),
        bytes.replace("\"listid\":1", "\"listid\":1,\"listid\":1"),
    ] {
        assert!(parse(duplicate.as_bytes(), "123456789", 1).is_err());
    }
}

#[tokio::test]
async fn native_library_transport_signs_exact_full_sync_body_for_both_native_clients() {
    for kind in [KugouLoginClient::Standard, KugouLoginClient::Concept] {
        let source = session(kind);
        let (client, task) = server(vec![ok(fixture(vec![entry(1, 0)])["data"].clone())]).await;
        client.native_library_page(&source, 2).await.unwrap();
        let requests = task.await.unwrap();
        assert_eq!(requests.len(), 1);
        let (query, body) = request(&requests[0], Endpoint::Library, kind);
        assert_eq!(query["plat"], "1");
        assert_eq!(query["userid"], source.user_id);
        assert_eq!(query["token"], source.token);
        assert_eq!(query["mid"], source.device.mid);
        assert_eq!(query["dfid"], source.device.dfid());
        assert_eq!(
            body,
            json!({"userid":123456789,"token":source.token,"total_ver":0,"type":2,"page":2,"pagesize":30})
        );
    }
}

#[tokio::test]
async fn native_library_transport_bounds_json_and_preserves_auth_verification_and_rate_errors() {
    let source = session(KugouLoginClient::Standard);
    let empty = fixture(vec![]).to_string();
    for (response, code) in [
        (frame(401, "", vec![]), ErrorCode::AuthenticationRequired),
        (
            frame(429, "Retry-After: 12\r\n", vec![]),
            ErrorCode::RateLimited,
        ),
        (
            frame(302, "Location: https://untrusted.invalid/\r\n", vec![]),
            ErrorCode::UpstreamError,
        ),
        (
            frame(
                200,
                "Content-Type: application/json\r\nSSA-Code: 2\r\n",
                empty.clone().into_bytes(),
            ),
            ErrorCode::PermissionDenied,
        ),
        (
            frame(
                200,
                "Content-Type: text/html\r\n",
                empty.clone().into_bytes(),
            ),
            ErrorCode::UpstreamError,
        ),
        (
            frame(
                200,
                "Content-Type: application/json\r\n",
                b"not JSON or an account secret".to_vec(),
            ),
            ErrorCode::UpstreamError,
        ),
        (
            frame(
                200,
                "Content-Type: application/json\r\n",
                br#"{"status":0,"error_code":20010,"data":"do-not-export"}"#.to_vec(),
            ),
            ErrorCode::UpstreamError,
        ),
        (
            frame(
                200,
                "Content-Type: application/json\r\n",
                br#"{"status":0,"error_code":20017,"data":"do-not-export"}"#.to_vec(),
            ),
            ErrorCode::AuthenticationRequired,
        ),
        (
            frame(
                200,
                "Content-Type: application/json\r\n",
                vec![b' '; 1_048_577],
            ),
            ErrorCode::UpstreamError,
        ),
    ] {
        let (client, task) = server(vec![response]).await;
        let e = client.native_library_page(&source, 1).await.err().unwrap();
        assert_eq!(e.code, code);
        assert!(!format!("{e:?}").contains("do-not-export"));
        if code == ErrorCode::RateLimited {
            assert_eq!(e.details["retry_after_secs"], 12);
        }
        assert_eq!(task.await.unwrap().len(), 1);
    }
    let padded = format!("{}{}", empty, " ".repeat(150_000));
    let (client, task) = server(vec![frame(
        200,
        "Content-Type: application/json\r\n",
        padded.into_bytes(),
    )])
    .await;
    assert!(
        client
            .native_library_page(&source, 1)
            .await
            .unwrap()
            .rows
            .is_empty()
    );
    task.await.unwrap();
}
