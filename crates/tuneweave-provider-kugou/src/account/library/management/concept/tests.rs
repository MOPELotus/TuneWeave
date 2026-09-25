use super::*;
use crate::account::cloud::tests::{binary, encrypted, plaintext, server};
use crate::account::tests::session;

fn edit() -> ListEdit {
    ListEdit {
        name: "Name".into(),
        intro: String::new(),
        tags: String::new(),
        private: true,
        sort: 42,
        total_ver: 9,
    }
}
fn receipt() -> Value {
    json!({"userid":123456789,"total_ver":10,"pre_total_ver":9,"list_count":2,
        "info":{"code":1,"listid":37,"type":0,"name":"Name","sort":42}})
}

#[test]
fn concept_metadata_cipher_and_signature_match_java_reference_vectors() {
    let plain =
        br#"{"intro":"","listid":37,"name":"Name","sort":42,"tags":"","total_ver":9,"type":0}"#;
    let body = Cipher::random().unwrap().encode(plain).unwrap();
    assert_eq!(
        hex::encode(&body),
        "3b70ec73705a30521c0dff2e6243c0a2ddb5ee6da7c1b507aa49731d5f46c67fa1dfd529644874bb4c6a4838a3928453a30c987a79d4fe3234a612a19ce6fbbdf5f8db883fefcebbacf9445a80f0f1b336f9ea19a9c51d76b60305b2782be74d"
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
        concept_signature(&query, signed_body(&body).as_bytes()),
        "86d4714bd4c70b108a8bd02acff62012"
    );
    assert_ne!(
        concept_signature(&query, &body),
        "86d4714bd4c70b108a8bd02acff62012"
    );
}

#[test]
fn concept_metadata_signed_body_matches_java_invalid_utf8_consumption() {
    // Independent InputStreamReader(UTF-8) outputs, including Java/Rust differences.
    for (input, output) in [
        ("eda080", "efbfbd"),
        ("edbfbf", "efbfbd"),
        ("eda0", "efbfbd"),
        ("eda041", "efbfbd41"),
        ("00c2a2e282acf09f988000", "00c2a2e282acf09f988000"),
        (
            "c080e08080f0808080",
            "efbfbdefbfbdefbfbdefbfbdefbfbdefbfbdefbfbdefbfbdefbfbd",
        ),
        ("f48fbfbff4908080", "f48fbfbfefbfbdefbfbdefbfbdefbfbd"),
        ("e282", "efbfbd"),
        ("f09080", "efbfbd"),
        ("80fffe", "efbfbdefbfbdefbfbd"),
        ("ed80bfe080", "ed80bfefbfbdefbfbd"),
        ("e18041", "efbfbd41"),
        ("f0908041", "efbfbd41"),
    ] {
        assert_eq!(
            hex::encode(signed_body(&hex::decode(input).unwrap()).as_bytes()),
            output,
            "{input}"
        );
    }
}

#[tokio::test]
async fn concept_metadata_v1_wire_encrypts_body_and_binds_concept_identity_without_privacy_fields()
{
    let f = server(vec![encrypted(receipt())]).await;
    let source = session(KugouLoginClient::Concept);
    let ack = f
        .client
        .native_write_list(
            &source,
            ListWrite::Modify {
                list_id: 37,
                edit: &edit(),
            },
        )
        .await
        .unwrap();
    assert_eq!(ack.list_id, Some(37));
    assert_eq!(ack.total_ver, Some(10));
    let requests = f.requests.await.unwrap();
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
    let mut query: BTreeMap<String, String> = url
        .query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
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
    assert_eq!(query["p"], query["p"].to_ascii_uppercase());
    let sig = query.remove("signature").unwrap();
    let query = query.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
    assert_eq!(
        sig,
        concept_signature(&query, signed_body(&request.body).as_bytes())
    );
    assert_eq!(
        plaintext(request),
        json!({"total_ver":9,"listid":37,"type":0,"name":"Name","sort":42,"intro":"","tags":""})
    );
    assert!(!request.head.contains(&source.token));
    assert!(!String::from_utf8_lossy(&request.body).contains(&source.token));
    assert!(!request.head.to_ascii_lowercase().contains("cookie:"));
    assert!(
        request
            .head
            .to_ascii_lowercase()
            .contains("content-type: application/json;charset=utf-8")
    );
}

#[test]
fn concept_metadata_v1_receipt_requires_exact_target_name_order_and_versions() {
    let bytes =
        |v: Value| serde_json::to_vec(&json!({"status":1,"error_code":0,"data":v})).unwrap();
    let good = receipt();
    acknowledge(&bytes(good.clone()), "123456789", 37, &edit()).unwrap();
    acknowledge(
        &serde_json::to_vec(&json!({"status":1,"data":good})).unwrap(),
        "123456789",
        37,
        &edit(),
    )
    .unwrap();
    for value in [
        json!({"status":0,"error_code":20017}),
        json!({"status":0}),
        json!({"status":1,"error_code":1,"data":good}),
        json!({"status":1}),
    ] {
        assert!(
            acknowledge(
                &serde_json::to_vec(&value).unwrap(),
                "123456789",
                37,
                &edit()
            )
            .is_err()
        );
    }
    for path in [
        "/userid",
        "/total_ver",
        "/pre_total_ver",
        "/list_count",
        "/info/code",
        "/info/listid",
        "/info/type",
        "/info/name",
        "/info/sort",
    ] {
        let mut value = good.clone();
        *value.pointer_mut(path).unwrap() = Value::Null;
        assert!(
            acknowledge(&bytes(value), "123456789", 37, &edit()).is_err(),
            "{path}"
        );
    }
    for (path, value) in [
        ("/userid", json!(222)),
        ("/total_ver", json!(8)),
        ("/pre_total_ver", json!(8)),
        ("/info/code", json!(0)),
        ("/info/code", json!(205)),
        ("/info/listid", json!(99)),
        ("/info/type", json!(1)),
        ("/info/name", json!("Unapplied")),
        ("/info/sort", json!(43)),
    ] {
        let mut receipt = good.clone();
        *receipt.pointer_mut(path).unwrap() = value;
        assert!(
            acknowledge(&bytes(receipt), "123456789", 37, &edit()).is_err(),
            "{path}"
        );
    }
}

#[tokio::test]
async fn concept_metadata_v1_rejects_plain_success_bad_transport_and_other_clients() {
    for frame in [
        binary(
            200,
            "application/json",
            serde_json::to_vec(&json!({"status":1,"error_code":0,"data":receipt()})).unwrap(),
        ),
        binary(302, "application/json", vec![]),
        binary(200, "text/html", vec![0; 32]),
        binary(200, "application/octet-stream", vec![0; RESPONSE_LIMIT + 1]),
    ] {
        let f = server(vec![frame]).await;
        assert!(
            f.client
                .native_write_list(
                    &session(KugouLoginClient::Concept),
                    ListWrite::Modify {
                        list_id: 37,
                        edit: &edit()
                    }
                )
                .await
                .is_err()
        );
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
    let f = server(vec![]).await;
    assert!(
        f.client
            .native_modify_concept_list(&session(KugouLoginClient::Standard), 37, &edit())
            .await
            .is_err()
    );
    assert!(
        f.client
            .native_modify_concept_list(&session(KugouLoginClient::Concept), 0, &edit())
            .await
            .is_err()
    );
    assert!(f.requests.await.unwrap().is_empty());
}
