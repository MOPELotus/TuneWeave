use super::*;
use crate::account::tests::{frame, ok, request, server, session};

fn row(id: u64) -> Value {
    json!({"fileid":id,"sort":0,"mixsongid":"900","audio_id":50,
        "name":"Singer - Song","hash":"1234567890abcdef1234567890abcdef",
        "timelen":216000,"collecttime":1700000000,"privilege":10,
        "album_id":300,"albuminfo":{"id":"300","name":"Album","secret":"hidden"},
        "singerinfo":[{"id":0,"name":"Singer","token":"hidden"}],
        "cover":"https://c1.kgimg.com/custom/{size}/cover.jpg","unknown":"hidden"})
}
fn fixture(rows: Vec<Value>) -> Value {
    json!({"status":1,"error_code":0,"data":{"userid":"123456789","listid":7,
        "type":1,"list_ver":3,"count":rows.len(),"info":rows}})
}
fn decode(value: &Value) -> Result<TrackPage> {
    parse(&serde_json::to_vec(value).unwrap(), "123456789", 7, 1, 1)
}

#[test]
fn tracks_keep_occurrence_ids_repeated_catalogue_ids_and_bounded_metadata_without_entitlements() {
    let page = decode(&fixture(vec![row(81), row(u64::MAX)])).unwrap();
    assert_eq!(page.total, 2);
    assert_eq!(page.rows[1].file_id, u64::MAX);
    for r in page.rows {
        assert_eq!(r.sort, Some(0));
        let t = r.track.unwrap();
        assert_eq!(t.id, "900");
        assert_eq!(t.name, "Song");
        assert_eq!(t.duration_ms, Some(216000));
        assert_eq!(t.extensions["file_id"], r.file_id);
        assert_eq!(t.extensions["collected_at"], 1700000000);
        assert_eq!(t.extensions["audio_id"], "50");
        assert_eq!(t.artists[0].resource_ref, None);
        assert_eq!(
            t.album
                .as_ref()
                .unwrap()
                .resource_ref
                .as_ref()
                .unwrap()
                .id(),
            "300"
        );
        assert_eq!(t.playable, None);
        assert!(t.available_qualities.is_empty());
        assert!(!serde_json::to_string(&t).unwrap().contains("hidden"));
    }
}

#[test]
fn unresolved_occurrences_are_retained_without_fabricated_song_ids() {
    for (field, value) in [
        ("mixsongid", json!(0)),
        ("mixsongid", Value::Null),
        ("name", json!("")),
    ] {
        let mut r = row(1);
        r[field] = value;
        let p = decode(&fixture(vec![r])).unwrap();
        assert_eq!(p.rows.len(), 1);
        assert_eq!(p.rows[0].file_id, 1);
        assert!(p.rows[0].track.is_none());
    }
    let mut r = row(1);
    r["mixsongid"] = json!(0);
    r["add_mixsongid"] = json!(800);
    assert_eq!(
        decode(&fixture(vec![r])).unwrap().rows[0]
            .track
            .as_ref()
            .unwrap()
            .id,
        "800"
    );
}

#[test]
fn current_v3_envelope_requires_explicit_success_count_version_and_info() {
    let empty = fixture(vec![]);
    let mut without_status = empty.clone();
    without_status.as_object_mut().unwrap().remove("status");
    assert_eq!(decode(&without_status).unwrap().total, 0);
    for missing in ["count", "list_ver", "info"] {
        let mut v = empty.clone();
        v["data"].as_object_mut().unwrap().remove(missing);
        assert!(decode(&v).is_err(), "{missing}");
    }
    for (field, value) in [
        ("status", json!(0)),
        ("status", Value::Null),
        ("error_code", json!(1)),
    ] {
        let mut v = empty.clone();
        v[field] = value;
        assert!(decode(&v).is_err());
    }
    for (code, expected) in [
        (20010, ErrorCode::UpstreamError),
        (20017, ErrorCode::AuthenticationRequired),
    ] {
        let v = json!({"error_code":code,"data":"secret"});
        let e = decode(&v).err().unwrap();
        assert_eq!(e.code, expected);
        assert!(!format!("{e:?}").contains("secret"));
    }
    let legacy =
        json!({"status":1,"error_code":0,"data":{"count":0,"list_info":{"list_ver":3},"songs":[]}});
    assert!(decode(&legacy).is_err());
}

#[test]
fn tracks_reject_noncanonical_numbers_conflicting_identities_and_duplicate_fields() {
    for (field, value) in [
        ("userid", json!(222)),
        ("listid", json!(9)),
        ("type", json!(0)),
    ] {
        let mut v = fixture(vec![]);
        v["data"][field] = value;
        assert_eq!(decode(&v).err().unwrap().code, ErrorCode::Conflict);
    }
    for (field, value) in [
        ("fileid", json!(0)),
        ("fileid", json!("01")),
        ("mixsongid", json!(-1)),
        ("sort", json!(1.5)),
        ("hash", json!("not-a-hash")),
        ("cover", json!("https://evil.invalid/a")),
        ("albuminfo", json!({"id":22,"name":"Album"})),
    ] {
        let mut r = row(1);
        r[field] = value;
        assert!(decode(&fixture(vec![r])).is_err(), "{field}");
    }
    assert!(decode(&fixture(vec![row(1), row(1)])).is_err());
    let valid = fixture(vec![row(1)]).to_string();
    for bytes in [
        valid.replace("\"fileid\":1", "\"fileid\":1,\"fileid\":1"),
        valid.replace("\"count\":1", "\"count\":1,\"count\":1"),
        valid.replace("\"status\":1", "\"status\":1,\"status\":1"),
    ] {
        assert!(parse(bytes.as_bytes(), "123456789", 7, 1, 1).is_err());
    }
    for (field, value) in [("page", 2), ("pagesize", 30)] {
        let mut v = fixture(vec![]);
        v["data"][field] = json!(value);
        assert!(decode(&v).is_err());
    }
    assert!(decode(&fixture((1..=301).map(row).collect())).is_err());
}

#[tokio::test]
async fn tracks_transport_signs_the_current_v3_body_and_router_for_both_native_clients() {
    for client_kind in [KugouLoginClient::Standard, KugouLoginClient::Concept] {
        let source = session(client_kind);
        let (client, task) = server(vec![ok(fixture(vec![row(1)])["data"].clone())]).await;
        client
            .native_library_tracks_page(&source, 7, 1, 2)
            .await
            .unwrap();
        let requests = task.await.unwrap();
        let (q, b) = request(&requests[0], Endpoint::LibraryTracks, client_kind);
        assert_eq!(q["plat"], "1");
        assert_eq!(q["token"], source.token);
        assert_eq!(
            b,
            json!({"userid":123456789,"token":source.token,"listid":7,"type":1,
            "page":2,"pagesize":300,"area_code":1,"allplatform":1,"show_cover":1})
        );
    }
}

#[tokio::test]
async fn tracks_transport_enforces_response_limits_json_and_account_errors_without_retries() {
    let source = session(KugouLoginClient::Standard);
    let mut large = fixture(vec![]);
    large["ignored"] = json!("x".repeat(140000));
    let (client, task) = server(vec![frame(
        200,
        "Content-Type: application/json\r\n",
        serde_json::to_vec(&large).unwrap(),
    )])
    .await;
    assert_eq!(
        client
            .native_library_tracks_page(&source, 7, 1, 1)
            .await
            .unwrap()
            .total,
        0
    );
    task.await.unwrap();
    for (body, code) in [
        (frame(401, "", vec![]), ErrorCode::AuthenticationRequired),
        (
            frame(429, "Retry-After: 2\r\n", vec![]),
            ErrorCode::RateLimited,
        ),
        (
            frame(302, "Location: https://evil.invalid\r\n", vec![]),
            ErrorCode::UpstreamError,
        ),
        (
            frame(200, "Content-Type: text/html\r\n", b"{}".to_vec()),
            ErrorCode::UpstreamError,
        ),
        (
            frame(
                200,
                "Content-Type: application/json\r\nSSA-Code: 2\r\n",
                b"{}".to_vec(),
            ),
            ErrorCode::PermissionDenied,
        ),
        (
            frame(
                200,
                "Content-Type: application/json\r\n",
                vec![b' '; 4_194_305],
            ),
            ErrorCode::UpstreamError,
        ),
    ] {
        let (client, task) = server(vec![body]).await;
        assert_eq!(
            client
                .native_library_tracks_page(&source, 7, 1, 1)
                .await
                .err()
                .unwrap()
                .code,
            code
        );
        assert_eq!(task.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn invalid_native_track_page_inputs_never_reach_network() {
    let source = session(KugouLoginClient::Standard);
    let (client, task) = server(vec![]).await;
    for (id, kind, page) in [(0, 0, 1), (7, 2, 1), (7, 0, 0), (7, 0, 129)] {
        assert_eq!(
            client
                .native_library_tracks_page(&source, id, kind, page)
                .await
                .err()
                .unwrap()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    assert!(task.await.unwrap().is_empty());
}
