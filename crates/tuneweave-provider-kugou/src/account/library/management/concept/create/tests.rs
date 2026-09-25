use super::*;
use crate::account::tests::{frame, ok, server, session};

fn receipt() -> Value {
    json!({"userid":123456789,"total_ver":10,"pre_total_ver":9,"list_count":2,
        "info":{"code":1,"listid":37,"type":0,"source":1,"name":"Created"}})
}

#[tokio::test]
async fn concept_default_create_uses_v4_exact_plaintext_identity_and_signature_without_privacy() {
    let (client, requests) = server(vec![ok(receipt())]).await;
    let source = session(KugouLoginClient::Concept);
    let ack = client
        .native_create_concept_list(&source, "Created", 9)
        .await
        .unwrap();
    assert_eq!(ack.list_id, Some(37));
    assert_eq!(ack.previous_ver, Some(9));
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
    assert_eq!(query.len(), 7);
    assert_eq!(query["appid"], "3116");
    assert_eq!(query["uuid"], "-");
    assert_eq!(query["clientver"], source.client.clientver().to_string());
    assert_eq!(query["mid"], source.device.mid);
    assert_eq!(query["dfid"], source.device.dfid());
    let signature = query.remove("signature").unwrap();
    let query = query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    assert_eq!(signature, concept_signature(&query, body.as_bytes()));
    assert_eq!(
        serde_json::from_str::<Value>(body).unwrap(),
        json!({"userid":123456789,"token":source.token,
        "total_ver":9,"name":"Created","type":0,"source":1,"list_create_userid":0,"list_create_listid":0})
    );
    assert!(!head.contains(&source.token));
    assert!(!head.to_ascii_lowercase().contains("cookie:"));
    assert!(
        head.to_ascii_lowercase()
            .contains("content-type: application/json;charset=utf-8")
    );
}

#[test]
fn concept_default_create_receipt_binds_new_identity_name_kind_and_versions() {
    let bytes = |v: Value| serde_json::to_vec(&json!({"status":1,"data":v})).unwrap();
    acknowledge(&bytes(receipt()), "123456789", "Created", 9).unwrap();
    for (path, value) in [
        ("/userid", json!(222)),
        ("/pre_total_ver", json!(8)),
        ("/total_ver", json!(8)),
        ("/info/listid", json!(0)),
        ("/info/type", json!(1)),
        ("/info/name", json!("Other")),
        ("/info/source", json!(2)),
        ("/info/code", json!(0)),
        ("/info/code", json!(101)),
    ] {
        let mut r = receipt();
        *r.pointer_mut(path).unwrap() = value;
        assert!(
            acknowledge(&bytes(r), "123456789", "Created", 9).is_err(),
            "{path}"
        );
    }
    for path in [
        "/userid",
        "/total_ver",
        "/pre_total_ver",
        "/list_count",
        "/info",
        "/info/listid",
        "/info/code",
        "/info/type",
        "/info/name",
    ] {
        let mut r = receipt();
        *r.pointer_mut(path).unwrap() = Value::Null;
        assert!(
            acknowledge(&bytes(r), "123456789", "Created", 9).is_err(),
            "{path}"
        );
    }
    for value in [
        json!({"status":0,"error_code":20017}),
        json!({"status":1,"error_code":1,"data":receipt()}),
        json!({"status":1}),
    ] {
        assert!(
            acknowledge(
                &serde_json::to_vec(&value).unwrap(),
                "123456789",
                "Created",
                9
            )
            .is_err()
        );
    }
}

#[tokio::test]
async fn concept_default_create_rejects_invalid_client_names_and_legacy_explicit_visibility_before_io()
 {
    let (client, requests) = server(vec![]).await;
    assert!(
        client
            .native_create_concept_list(&session(KugouLoginClient::Standard), "Created", 9)
            .await
            .is_err()
    );
    for name in [String::new(), " ".into(), "界".repeat(21)] {
        assert!(
            client
                .native_create_concept_list(&session(KugouLoginClient::Concept), &name, 9)
                .await
                .is_err()
        );
    }
    for private in [false, true] {
        assert!(
            client
                .native_write_list(
                    &session(KugouLoginClient::Concept),
                    ListWrite::Add {
                        name: "Created",
                        private,
                        source: None
                    }
                )
                .await
                .is_err()
        );
    }
    assert!(requests.await.unwrap().is_empty());
}

#[tokio::test]
async fn concept_default_create_bounds_transport_and_never_retries_rejected_writes() {
    for response in [
        frame(302, "Location: https://example.invalid/\r\n", vec![]),
        frame(
            200,
            "Content-Type: text/html\r\n",
            b"<html>challenge</html>".to_vec(),
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
            .native_create_concept_list(&session(KugouLoginClient::Concept), "Created", 9)
            .await
        {
            Ok(_) => panic!("unexpected success"),
            Err(e) => assert!(!format!("{e:?}").contains("private-response-marker")),
        }
        assert_eq!(requests.await.unwrap().len(), 1);
    }
}
