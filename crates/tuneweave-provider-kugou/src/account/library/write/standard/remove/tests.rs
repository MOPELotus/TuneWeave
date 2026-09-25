use super::*;
use crate::account::tests::{ok, server, session};

fn receipt() -> Value {
    json!({"userid":123456789,"listid":37,"list_ver":8,"pre_list_ver":7,"count":2})
}

#[tokio::test]
async fn standard_remove_wire_is_manual_versioned_fileid_only_and_signed() {
    for files in [vec![81], vec![81, 95, 99]] {
        let (client, requests) = server(vec![ok(receipt())]).await;
        let source = session(KugouLoginClient::Standard);
        let ack = client
            .native_remove_standard_tracks(&source, 37, 7, &files)
            .await
            .unwrap();
        assert_eq!(ack.version, Some(8));
        assert_eq!(ack.previous_version, Some(7));
        assert_eq!(ack.count, Some(2));
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
        assert_eq!(query.len(), 6);
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
            json!({"userid":123456789,"token":source.token,"listid":37,"list_ver":7,
                "type":0,"scene":"false,2",
                "data":files.iter().map(|id|json!({"fileid":id})).collect::<Vec<_>>()})
        );
        assert!(!head.to_ascii_lowercase().contains("cookie:"));
        assert!(!head.to_ascii_lowercase().contains("x-router:"));
    }
}

#[test]
fn standard_remove_ack_requires_full_identity_version_and_count() {
    let bytes = |value| serde_json::to_vec(&json!({"status":1,"data":value})).unwrap();
    assert!(acknowledge(&bytes(receipt()), "123456789", 37, 7).is_ok());
    for field in ["userid", "listid", "list_ver", "pre_list_ver", "count"] {
        let mut value = receipt();
        value.as_object_mut().unwrap().remove(field);
        assert!(
            acknowledge(&bytes(value), "123456789", 37, 7).is_err(),
            "{field}"
        );
    }
    for (field, value) in [
        ("userid", json!(2)),
        ("listid", json!(38)),
        ("pre_list_ver", json!(6)),
        ("list_ver", json!(7)),
        ("list_ver", json!(2147483648_u64)),
        ("count", json!(-1)),
        ("count", json!(2147483648_u64)),
        ("type", json!(1)),
    ] {
        let mut changed = receipt();
        changed[field] = value;
        assert!(
            acknowledge(&bytes(changed), "123456789", 37, 7).is_err(),
            "{field}"
        );
    }
    for value in [
        json!({"status":0,"error_code":20017}),
        json!({"status":1,"error_code":9,"data":receipt()}),
        json!({"status":1,"data":null}),
    ] {
        assert!(acknowledge(&serde_json::to_vec(&value).unwrap(), "123456789", 37, 7).is_err());
    }
}

#[tokio::test]
async fn standard_remove_rejects_wrong_profile_and_invalid_inputs_before_io() {
    let (client, requests) = server(vec![]).await;
    for profile in [KugouLoginClient::Concept, KugouLoginClient::Web] {
        assert!(
            client
                .native_remove_standard_tracks(&session(profile), 37, 7, &[81])
                .await
                .is_err()
        );
    }
    let source = session(KugouLoginClient::Standard);
    for (list, version, files) in [
        (0, 7, vec![81]),
        (37, 2147483648_u64, vec![81]),
        (37, 7, vec![]),
        (37, 7, vec![0]),
        (37, 7, vec![2147483648_u64]),
        (37, 7, vec![81, 81]),
        (37, 7, (1..=301).collect()),
    ] {
        assert!(
            client
                .native_remove_standard_tracks(&source, list, version, &files)
                .await
                .is_err()
        );
    }
    assert!(requests.await.unwrap().is_empty());
}
