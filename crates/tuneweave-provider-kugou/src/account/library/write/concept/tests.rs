use super::*;
use crate::account::tests::{frame, ok, server, session};

fn receipt() -> Value {
    json!({"userid":123456789,"listid":37,"list_ver":8,"pre_list_ver":7,"count":2})
}

#[tokio::test]
async fn concept_occurrence_remove_wire_binds_exact_file_id_version_and_plaintext_signature() {
    let (client, requests) = server(vec![ok(receipt())]).await;
    let source = session(KugouLoginClient::Concept);
    let ack = client
        .native_remove_concept_occurrence(&source, 37, 81, 7)
        .await
        .unwrap();
    assert_eq!(ack.version, Some(8));
    assert_eq!(ack.previous_version, Some(7));
    assert_eq!(ack.count, Some(2));
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
    assert_eq!(query["uuid"], "-");
    assert_eq!(query["mid"], source.device.mid);
    assert_eq!(query["dfid"], source.device.dfid());
    let signature = query.remove("signature").unwrap();
    let query = query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    assert_eq!(signature, concept_signature(&query, body.as_bytes()));
    assert_eq!(
        serde_json::from_str::<Value>(body).unwrap(),
        json!({"userid":123456789,
        "token":source.token,"listid":37,"list_ver":7,"type":0,"data":[{"fileid":81}]})
    );
    assert!(!head.contains(&source.token));
    for field in ["cookie:", "x-router:"] {
        assert!(!head.to_ascii_lowercase().contains(field));
    }
}

#[test]
fn concept_occurrence_remove_ack_requires_complete_target_identity_and_version_receipt() {
    let bytes = |data: Value| serde_json::to_vec(&json!({"status":1,"data":data})).unwrap();
    acknowledge(&bytes(receipt()), "123456789", 37, 7).unwrap();
    for key in ["userid", "listid", "list_ver", "pre_list_ver", "count"] {
        let mut data = receipt();
        data.as_object_mut().unwrap().remove(key);
        assert!(
            acknowledge(&bytes(data), "123456789", 37, 7).is_err(),
            "{key}"
        );
        for value in [Value::Null, json!(-1), json!(1.5), json!("01")] {
            let mut data = receipt();
            data[key] = value;
            assert!(
                acknowledge(&bytes(data), "123456789", 37, 7).is_err(),
                "{key}"
            );
        }
    }
    for (key, value) in [
        ("userid", 222),
        ("listid", 99),
        ("list_ver", 6),
        ("pre_list_ver", 6),
        ("type", 1),
    ] {
        let mut data = receipt();
        data[key] = json!(value);
        match acknowledge(&bytes(data), "123456789", 37, 7) {
            Ok(_) => panic!("wrong identity accepted"),
            Err(e) => assert_eq!(e.code, ErrorCode::Conflict),
        }
    }
    for response in [
        json!({"status":0,"error_code":20017}),
        json!({"status":1,"error_code":1,"data":receipt()}),
        json!({"status":1,"data":[]}),
    ] {
        assert!(acknowledge(&serde_json::to_vec(&response).unwrap(), "123456789", 37, 7).is_err());
    }
}

#[tokio::test]
async fn concept_occurrence_remove_rejects_legacy_zero_version_path_and_invalid_inputs_without_io()
{
    let (client, requests) = server(vec![]).await;
    let concept = session(KugouLoginClient::Concept);
    for (list, file) in [(0, 81), (37, 0)] {
        assert!(
            client
                .native_remove_concept_occurrence(&concept, list, file, 7)
                .await
                .is_err()
        );
    }
    assert!(
        client
            .native_remove_concept_occurrence(&session(KugouLoginClient::Standard), 37, 81, 7)
            .await
            .is_err()
    );
    assert!(
        client
            .native_write_tracks(&concept, 37, Write::Remove(&[81]))
            .await
            .is_err()
    );
    assert!(requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn concept_occurrence_remove_transport_rejections_never_retry_or_reflect_payloads() {
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
    ] {
        let (client, requests) = server(vec![response]).await;
        match client
            .native_remove_concept_occurrence(&session(KugouLoginClient::Concept), 37, 81, 7)
            .await
        {
            Ok(_) => panic!("rejected response accepted"),
            Err(e) => assert!(!format!("{e:?}").contains("private-response-marker")),
        }
        assert_eq!(requests.await.unwrap().len(), 1);
    }
}
