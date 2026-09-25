use super::*;
use crate::account::tests::{ok, server, session};

mod batch;

fn input() -> TrackInput {
    TrackInput {
        wire: TrackWire::Standard(StandardTrackInput {
            number: 1,
            name: "Artist - Song.mp3".into(),
            hash: "a".repeat(32),
            size: 120000,
            sort: 0,
            timelen: 216789,
            bitrate: 128,
            album_id: "42".into(),
            mixsongid: 900,
        }),
    }
}

fn receipt() -> Value {
    json!({"userid":123456789,"listid":37,"list_ver":8,"pre_list_ver":7,"count":2,
        "info":[{"fileid":99,"name":"Artist - Song.mp3","sort":1,"hash":"A".repeat(32),
        "album_id":"42","mixsongid":900,"code":1}]})
}

#[tokio::test]
async fn standard_add_single_wire_uses_current_version_official_query_and_signature() {
    let (client, requests) = server(vec![ok(receipt())]).await;
    let source = session(KugouLoginClient::Standard);
    let (ack, item) = client
        .native_add_standard_track(&source, 37, 7, &input())
        .await
        .unwrap();
    assert_eq!(ack.version, Some(8));
    assert_eq!((item.file_id, item.sort), (99, 1));
    let requests = requests.await.unwrap();
    assert_eq!(requests.len(), 1);
    let (head, body) = requests[0].split_once("\r\n\r\n").unwrap();
    let target = head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
    assert_eq!(url.path(), PATH);
    let mut query: BTreeMap<String, String> = url.query_pairs().into_owned().collect();
    let signature = query.remove("signature").unwrap();
    assert_eq!(query.len(), 8);
    assert_eq!(query["userid"], source.user_id);
    assert_eq!(query["token"], source.token);
    assert_eq!(query["appid"], "1005");
    assert_eq!(query["clientver"], "20809");
    assert_eq!(query["uuid"], "-");
    assert_eq!(query["mid"], source.device.mid);
    assert_eq!(query["dfid"], source.device.dfid());
    assert!(query["clienttime"].parse::<u64>().unwrap() > 0);
    let query = query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    assert_eq!(signature, android_signature(&query, body.as_bytes()));
    assert_eq!(
        serde_json::from_str::<Value>(body).unwrap(),
        json!({"userid":"123456789","token":source.token,"listid":37,"list_ver":7,
            "type":0,"slow_upload":1,"scene":"false;null","mode":1,"allow_part_fail":1,
            "data":[{"number":1,"name":"Artist - Song.mp3","hash":"a".repeat(32),
            "size":120000,"sort":0,"timelen":216789,"bitrate":128,"album_id":"42",
            "mixsongid":900}]})
    );
    assert!(!head.to_ascii_lowercase().contains("cookie:"));
    assert!(!head.to_ascii_lowercase().contains("x-router:"));
}

#[test]
fn standard_add_single_ack_requires_identity_version_and_standard_success_code() {
    let input = input();
    let TrackWire::Standard(input) = &input.wire else {
        unreachable!()
    };
    let bytes = |value| serde_json::to_vec(&json!({"status":1,"data":value})).unwrap();
    acknowledge(&bytes(receipt()), "123456789", 37, 7, &[input]).unwrap();
    let mut default_code = receipt();
    default_code["info"][0]
        .as_object_mut()
        .unwrap()
        .remove("code");
    acknowledge(&bytes(default_code), "123456789", 37, 7, &[input]).unwrap();
    for field in [
        "userid",
        "listid",
        "list_ver",
        "pre_list_ver",
        "count",
        "info",
    ] {
        let mut value = receipt();
        value.as_object_mut().unwrap().remove(field);
        assert!(
            acknowledge(&bytes(value), "123456789", 37, 7, &[input]).is_err(),
            "{field}"
        );
    }
    for (field, value) in [
        ("userid", json!(222)),
        ("listid", json!(38)),
        ("pre_list_ver", json!(6)),
        ("list_ver", json!(7)),
        ("type", json!(1)),
        ("is_edit", json!(1)),
        ("del_fileids", json!([70])),
        ("info", json!([])),
        ("info", json!([receipt()["info"][0], receipt()["info"][0]])),
    ] {
        let mut data = receipt();
        data[field] = value;
        assert!(
            acknowledge(&bytes(data), "123456789", 37, 7, &[input]).is_err(),
            "{field}"
        );
    }
    for (field, value) in [
        ("fileid", json!(0)),
        ("sort", json!(-1)),
        ("name", json!("")),
        ("hash", json!("b".repeat(32))),
        ("album_id", json!(43)),
        ("mixsongid", json!(901)),
        ("code", json!(0)),
        ("code", json!(205)),
        ("csong", json!(1)),
    ] {
        let mut data = receipt();
        data["info"][0][field] = value;
        assert!(
            acknowledge(&bytes(data), "123456789", 37, 7, &[input]).is_err(),
            "{field}"
        );
    }
    for (status, error_code, expected) in [
        (0, 20017, ErrorCode::AuthenticationRequired),
        (0, 20010, ErrorCode::UpstreamError),
        (1, 20010, ErrorCode::UpstreamError),
    ] {
        let bytes = serde_json::to_vec(&json!({"status":status,"error_code":error_code})).unwrap();
        let Err(error) = acknowledge(&bytes, "123456789", 37, 7, &[input]) else {
            panic!("rejected status was accepted");
        };
        assert_eq!(error.code, expected);
    }
}

#[tokio::test]
async fn standard_add_single_rejects_other_profiles_and_invalid_version_before_io() {
    let (client, requests) = server(vec![]).await;
    for profile in [KugouLoginClient::Concept, KugouLoginClient::Web] {
        assert!(
            client
                .native_add_standard_track(&session(profile), 37, 7, &input())
                .await
                .is_err()
        );
    }
    let source = session(KugouLoginClient::Standard);
    for (list, version) in [(0, 7), (i32::MAX as u64 + 1, 7), (37, u64::MAX)] {
        assert!(
            client
                .native_add_standard_track(&source, list, version, &input())
                .await
                .is_err()
        );
    }
    let wrong_input = TrackInput {
        wire: TrackWire::Catalogue(CatalogueTrackInput {
            number: 1,
            name: "Song".into(),
            hash: "a".repeat(32),
            size: 10,
            sort: 0,
            timelen: 0,
            bitrate: 0,
            album_id: 42,
            mixsongid: 900,
        }),
    };
    assert!(
        client
            .native_add_standard_track(&source, 37, 7, &wrong_input)
            .await
            .is_err()
    );
    assert!(requests.await.unwrap().is_empty());
}
