use super::*;
use crate::account::cloud::tests::{binary, encrypted, plaintext, server};
use crate::account::tests::session;
use base64::engine::general_purpose::STANDARD as BASE64;

fn receipt() -> Value {
    json!({"userid":123456789,"pre_total_ver":9,"total_ver":10,"list_count":1,
        "info":[{"code":1,"listid":38},{"code":1,"listid":37,"type":0}]})
}

#[test]
fn concept_batch_delete_base64_cipher_and_signature_match_independent_java_vector() {
    let plain = br#"{"data":[{"listid":37,"type":0},{"listid":38,"type":0}],"total_ver":9}"#;
    let body = BASE64.encode(Cipher::random().unwrap().encode(plain).unwrap());
    assert_eq!(
        body,
        "NLtj5hKYzp94T7E/3A1o9mEAdZEPhoxqBeE4yZ621h4GSaQDZTxFJMIOThI22/Y6VxWgewmwiEotuc1drxKirBjr+B46FW0O+OOEBIXyw4g="
    );
    let query = BTreeMap::from([
        ("appid", "3116".into()),
        ("clienttime", "1700000000".into()),
        ("clientver", "11440".into()),
        ("dfid", "-".into()),
        ("key", "example".into()),
        ("mid", "synthetic-mid".into()),
        ("p", "ABCDEF".into()),
    ]);
    assert_eq!(
        concept_signature(&query, body.as_bytes()),
        "f6e6e48ec266fa15bdf6319b14e2dd16"
    );
}

#[tokio::test]
async fn concept_batch_delete_encrypts_exact_versioned_body_and_signs_identity_query() {
    let f = server(vec![encrypted(receipt())]).await;
    let source = session(KugouLoginClient::Concept);
    let ack = f
        .client
        .native_delete_concept_lists(&source, &[37, 38], 9)
        .await
        .unwrap();
    assert_eq!(ack.total_ver, Some(10));
    assert_eq!(ack.list_id, None);
    let requests = f.requests.await.unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    let target = request
        .head
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap();
    let url = url::Url::parse(&format!("http://localhost{target}")).unwrap();
    assert_eq!(url.path(), PATH);
    let mut query = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(query.len(), 8);
    assert_eq!(query["appid"], "3116");
    assert_eq!(query["clientver"], source.client.clientver().to_string());
    assert_eq!(query["mid"], source.device.mid);
    assert_eq!(query["dfid"], source.device.dfid());
    assert_eq!(
        query["key"],
        format!(
            "{:x}",
            Md5::digest(format!(
                "3116{APP_KEY}{}{}",
                source.client.clientver(),
                query["clienttime"]
            ))
        )
    );
    assert_eq!(query["p"].len(), 256);
    let sig = query.remove("signature").unwrap();
    let pairs = query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    assert_eq!(
        sig,
        concept_signature(&pairs, signed_body(&request.body).as_bytes())
    );
    let mut decoded_request = request.clone();
    decoded_request.body = BASE64.decode(&request.body).unwrap();
    assert_eq!(
        BASE64.encode(&decoded_request.body).as_bytes(),
        request.body
    );
    assert_eq!(
        plaintext(&decoded_request),
        json!({"total_ver":9,"data":[{"listid":37,"type":0},{"listid":38,"type":0}]})
    );
    assert!(!request.head.contains(&source.token));
    assert!(!request.head.to_ascii_lowercase().contains("cookie:"));
    assert!(!String::from_utf8_lossy(&request.body).contains(&source.token));
}

#[test]
fn concept_batch_delete_requires_exact_successful_target_set_and_root_identity() {
    let bytes = |v: Value| serde_json::to_vec(&json!({"status":1,"data":v})).unwrap();
    acknowledge(&bytes(receipt()), "123456789", &[37, 38], 9).unwrap();
    for path in [
        "/userid",
        "/total_ver",
        "/pre_total_ver",
        "/list_count",
        "/info",
        "/info/0/code",
        "/info/0/listid",
    ] {
        let mut v = receipt();
        *v.pointer_mut(path).unwrap() = Value::Null;
        assert!(
            acknowledge(&bytes(v), "123456789", &[37, 38], 9).is_err(),
            "{path}"
        );
    }
    for (path, value) in [
        ("/userid", json!(99)),
        ("/pre_total_ver", json!(8)),
        ("/total_ver", json!(8)),
        ("/info/0/code", json!(0)),
        ("/info/0/listid", json!(99)),
        ("/info/0/listid", json!(37)),
        ("/info/1/type", json!(1)),
        ("/info", json!([])),
        ("/info", json!([{"listid":37,"code":1}])),
    ] {
        let mut v = receipt();
        *v.pointer_mut(path).unwrap() = value;
        assert!(
            acknowledge(&bytes(v), "123456789", &[37, 38], 9).is_err(),
            "{path}"
        );
    }
    let mut v = receipt();
    v["info"]
        .as_array_mut()
        .unwrap()
        .push(json!({"listid":39,"code":1}));
    assert!(acknowledge(&bytes(v), "123456789", &[37, 38], 9).is_err());
    for v in [
        json!({"status":0,"error_code":20017}),
        json!({"status":1,"error_code":9,"data":receipt()}),
        json!({"status":1}),
    ] {
        assert!(acknowledge(&serde_json::to_vec(&v).unwrap(), "123456789", &[37, 38], 9).is_err());
    }
}

#[tokio::test]
async fn concept_batch_delete_rejects_invalid_scope_before_io_and_never_retries_failures() {
    let f = server(vec![]).await;
    let source = session(KugouLoginClient::Concept);
    for ids in [
        vec![],
        vec![37],
        vec![37, 37],
        vec![0, 37],
        (1..=101).collect(),
    ] {
        assert!(
            f.client
                .native_delete_concept_lists(&source, &ids, 9)
                .await
                .is_err()
        );
    }
    assert!(
        f.client
            .native_delete_concept_lists(&session(KugouLoginClient::Standard), &[37, 38], 9)
            .await
            .is_err()
    );
    assert!(f.requests.await.unwrap().is_empty());
    for frame in [
        binary(302, "application/json", vec![]),
        binary(200, "text/html", vec![]),
        binary(200, "application/octet-stream", vec![0; RESPONSE_LIMIT + 1]),
        binary(
            200,
            "application/json",
            serde_json::to_vec(&json!({"status":1,"data":receipt()})).unwrap(),
        ),
        encrypted(
            json!({"userid":123456789,"pre_total_ver":9,"total_ver":10,"list_count":2,"info":[{"code":1,"listid":37},{"code":0,"listid":38}]}),
        ),
    ] {
        let f = server(vec![frame]).await;
        let e = match f
            .client
            .native_delete_concept_lists(&source, &[37, 38], 9)
            .await
        {
            Ok(_) => panic!("invalid batch receipt was accepted"),
            Err(e) => e,
        };
        assert!(!format!("{e:?}").contains(&source.token));
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
}
