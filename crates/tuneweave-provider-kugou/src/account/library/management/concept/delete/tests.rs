use super::*;
use crate::account::tests::{frame, ok, server, session};

fn receipt() -> Value {
    json!({"userid":123456789,"total_ver":10,"pre_total_ver":9,"list_count":1})
}

#[tokio::test]
async fn concept_single_uncollect_uses_selected_local_identity_type_one_and_v3_signature() {
    let (client, requests) = server(vec![ok(receipt())]).await;
    let source = session(KugouLoginClient::Concept);
    let ack = client
        .native_write_list(
            &source,
            ListWrite::Delete {
                list_id: 37,
                kind: 1,
                total_ver: 9,
            },
        )
        .await
        .unwrap();
    assert_eq!(ack.list_id, None);
    assert_eq!(ack.previous_ver, Some(9));
    let all = requests.await.unwrap();
    assert_eq!(all.len(), 1);
    let (head, body) = all[0].split_once("\r\n\r\n").unwrap();
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
    assert_eq!(query.len(), 7);
    assert_eq!(query["appid"], "3116");
    assert_eq!(query["mid"], source.device.mid);
    assert_eq!(query["dfid"], source.device.dfid());
    assert_eq!(query["uuid"], "-");
    let signature = query.remove("signature").unwrap();
    let query = query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    assert_eq!(signature, concept_signature(&query, body.as_bytes()));
    assert_eq!(
        serde_json::from_str::<Value>(body).unwrap(),
        json!({
            "userid":123456789,"token":source.token,"listid":37,"total_ver":9,"type":1,
        })
    );
    assert!(!head.contains(&source.token));
    assert!(!head.to_ascii_lowercase().contains("cookie:"));
}

#[tokio::test]
async fn concept_single_uncollect_rejects_unknown_kind_without_legacy_fallback() {
    let (client, requests) = server(vec![]).await;
    for kind in [2, u8::MAX] {
        assert!(
            client
                .native_write_list(
                    &session(KugouLoginClient::Concept),
                    ListWrite::Delete {
                        list_id: 37,
                        kind,
                        total_ver: 9
                    },
                )
                .await
                .is_err()
        );
    }
    assert!(requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn concept_single_delete_uses_exact_v3_plaintext_fields_and_current_version() {
    let (client, requests) = server(vec![ok(receipt())]).await;
    let source = session(KugouLoginClient::Concept);
    let ack = client
        .native_write_list(
            &source,
            ListWrite::Delete {
                list_id: 37,
                kind: 0,
                total_ver: 9,
            },
        )
        .await
        .unwrap();
    assert_eq!(ack.list_id, None);
    assert_eq!(ack.previous_ver, Some(9));
    assert_eq!(ack.total_ver, Some(10));
    assert_eq!(ack.list_count, Some(1));
    let all = requests.await.unwrap();
    assert_eq!(all.len(), 1);
    let (head, body) = all[0].split_once("\r\n\r\n").unwrap();
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
    assert_eq!(
        query.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "appid",
            "clienttime",
            "clientver",
            "dfid",
            "mid",
            "signature",
            "uuid"
        ]
    );
    assert_eq!(query["appid"], "3116");
    assert_eq!(query["clientver"], source.client.clientver().to_string());
    assert_eq!(query["mid"], source.device.mid);
    assert_eq!(query["dfid"], source.device.dfid());
    assert_eq!(query["uuid"], "-");
    let signature = query.remove("signature").unwrap();
    let query = query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    assert_eq!(signature, concept_signature(&query, body.as_bytes()));
    assert_eq!(
        serde_json::from_str::<Value>(body).unwrap(),
        json!({
            "userid":123456789,"token":source.token,"listid":37,"total_ver":9,"type":0,
        })
    );
    assert!(!head.contains(&source.token));
    assert!(!head.to_ascii_lowercase().contains("cookie:"));
    assert!(
        head.to_ascii_lowercase()
            .contains("content-type: application/json;charset=utf-8")
    );
}

#[test]
fn concept_single_delete_receipt_requires_root_identity_versions_and_count() {
    let bytes = |data: Value| serde_json::to_vec(&json!({"status":1,"data":data})).unwrap();
    acknowledge(&bytes(receipt()), "123456789", 9).unwrap();
    for key in ["userid", "total_ver", "pre_total_ver", "list_count"] {
        let mut missing = receipt();
        missing.as_object_mut().unwrap().remove(key);
        // A batch-like item ACK is not a replacement for the single receipt.
        missing["info"] = json!([{"listid":37,"code":1}]);
        assert!(
            acknowledge(&bytes(missing), "123456789", 9).is_err(),
            "{key}"
        );
        for value in [Value::Null, json!(-1), json!(1.5), json!("01")] {
            let mut invalid = receipt();
            invalid[key] = value;
            assert!(
                acknowledge(&bytes(invalid), "123456789", 9).is_err(),
                "{key}"
            );
        }
    }
    for (key, value) in [("userid", 222), ("pre_total_ver", 8), ("total_ver", 8)] {
        let mut data = receipt();
        data[key] = json!(value);
        match acknowledge(&bytes(data), "123456789", 9) {
            Ok(_) => panic!("invalid receipt was accepted"),
            Err(e) => assert_eq!(e.code, ErrorCode::Conflict),
        }
    }
    for (response, expected) in [
        (
            json!({"status":0,"error_code":20017}),
            ErrorCode::AuthenticationRequired,
        ),
        (
            json!({"status":0,"error_code":20010}),
            ErrorCode::UpstreamError,
        ),
        (
            json!({"status":1,"error_code":1,"data":receipt()}),
            ErrorCode::UpstreamError,
        ),
        (json!({"status":1,"data":[]}), ErrorCode::UpstreamError),
    ] {
        match acknowledge(&serde_json::to_vec(&response).unwrap(), "123456789", 9) {
            Ok(_) => panic!("invalid status was accepted"),
            Err(e) => assert_eq!(e.code, expected),
        }
    }
}

#[tokio::test]
async fn concept_single_delete_rejects_wrong_client_or_zero_list_before_io() {
    let (client, requests) = server(vec![]).await;
    assert!(
        client
            .native_delete_concept_list(&session(KugouLoginClient::Standard), 37, 0, 9)
            .await
            .is_err()
    );
    assert!(
        client
            .native_delete_concept_list(&session(KugouLoginClient::Concept), 0, 0, 9)
            .await
            .is_err()
    );
    assert!(requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn concept_single_delete_transport_failures_never_retry_or_echo_response_secrets() {
    for response in [
        frame(302, "Location: https://example.invalid/\r\n", vec![]),
        frame(
            503,
            "Content-Type: application/json\r\n",
            b"private-response-marker".to_vec(),
        ),
        frame(
            200,
            "Content-Type: text/html\r\n",
            b"private-response-marker".to_vec(),
        ),
        frame(
            200,
            "Content-Type: application/json\r\n",
            vec![b'x'; RESPONSE_LIMIT + 1],
        ),
        frame(
            200,
            "Content-Type: application/json\r\n",
            br#"{"status":0,"error_code":20017,"data":"private-response-marker"}"#.to_vec(),
        ),
    ] {
        let (client, requests) = server(vec![response]).await;
        match client
            .native_delete_concept_list(&session(KugouLoginClient::Concept), 37, 0, 9)
            .await
        {
            Ok(_) => panic!("invalid response was accepted"),
            Err(e) => assert!(!format!("{e:?}").contains("private-response-marker")),
        }
        assert_eq!(requests.await.unwrap().len(), 1);
    }
}
