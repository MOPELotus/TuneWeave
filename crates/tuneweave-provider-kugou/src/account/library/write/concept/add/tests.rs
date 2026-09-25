use super::*;
use crate::account::tests::{ok, server, session};

fn input() -> ConceptAlbumTrack {
    ConceptAlbumTrack {
        name: "Artist - Song.mp3".into(),
        hash: "a".repeat(32),
        size: 120000,
        timelen: 216789,
        bitrate: 128,
        album_id: "42".into(),
        mixsongid: 900,
    }
}
fn receipt() -> Value {
    json!({"userid":123456789,"listid":37,"list_ver":8,"pre_list_ver":7,"count":2,
        "info":[{"fileid":99,"name":"Artist - Song.mp3","sort":0,"hash":"A".repeat(32),
        "album_id":"42","mixsongid":900,"code":0,"csong":0}]})
}

#[tokio::test]
async fn concept_add_wire_uses_string_uid_current_version_and_plaintext_signature() {
    let (client, requests) = server(vec![ok(receipt())]).await;
    let source = session(KugouLoginClient::Concept);
    let ack = client
        .native_add_concept_track(&source, 37, 7, &input())
        .await
        .unwrap();
    assert_eq!((ack.file_id, ack.sort), (99, 0));
    assert_eq!(ack.version.version, Some(8));
    let requests = requests.await.unwrap();
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
    let query = query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    assert_eq!(signature, concept_signature(&query, body.as_bytes()));
    assert_eq!(
        serde_json::from_str::<Value>(body).unwrap(),
        json!({
        "userid":"123456789","token":source.token,"listid":37,"list_ver":7,"type":0,
        "data":[{"number":1,"name":"Artist - Song.mp3","hash":"a".repeat(32),"size":120000,
        "sort":0,"timelen":216789,"bitrate":128,"album_id":"42","mixsongid":900}]})
    );
    assert!(!head.contains(&source.token));
    assert!(!head.to_ascii_lowercase().contains("cookie:"));
}

#[test]
fn concept_add_ack_requires_exact_single_item_identity_and_current_version() {
    let bytes = |value| serde_json::to_vec(&json!({"status":1,"data":value})).unwrap();
    acknowledge(&bytes(receipt()), "123456789", 37, 7, &input()).unwrap();
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
        assert!(acknowledge(&bytes(value), "123456789", 37, 7, &input()).is_err());
    }
    for (field, value) in [
        ("userid", json!(222)),
        ("listid", json!(38)),
        ("list_ver", json!(7)),
        ("pre_list_ver", json!(6)),
        ("type", json!(1)),
    ] {
        let mut data = receipt();
        data[field] = value;
        assert!(acknowledge(&bytes(data), "123456789", 37, 7, &input()).is_err());
    }
    for (field, value) in [
        ("fileid", json!(0)),
        ("hash", json!("b".repeat(32))),
        ("album_id", json!(43)),
        ("mixsongid", json!(901)),
        ("code", json!(205)),
        ("csong", json!(1)),
    ] {
        let mut data = receipt();
        data["info"][0][field] = value;
        assert!(acknowledge(&bytes(data), "123456789", 37, 7, &input()).is_err());
    }
    let mut data = receipt();
    data["info"] = json!([]);
    assert!(acknowledge(&bytes(data), "123456789", 37, 7, &input()).is_err());
}

#[tokio::test]
async fn concept_add_wire_rejects_other_profile_and_invalid_version_without_io() {
    let (client, requests) = server(vec![]).await;
    assert!(
        client
            .native_add_concept_track(&session(KugouLoginClient::Standard), 37, 7, &input())
            .await
            .is_err()
    );
    assert!(
        client
            .native_add_concept_track(&session(KugouLoginClient::Concept), 37, u64::MAX, &input())
            .await
            .is_err()
    );
    assert!(requests.await.unwrap().is_empty());
}

mod batch;
