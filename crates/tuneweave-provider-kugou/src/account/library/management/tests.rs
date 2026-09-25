use super::*;
use crate::account::tests::{frame, ok, request, server, session};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use md5::{Digest, Md5};

fn edit() -> ListEdit {
    ListEdit {
        name: "New name".into(),
        intro: "Keep\nintro".into(),
        tags: "甲,乙".into(),
        private: true,
        sort: 42,
        total_ver: 9,
    }
}

#[tokio::test]
async fn visibility_wire_uses_standard_f_producer_encryption_signature_and_code_one() {
    for private in [false, true] {
        let mut payload = ack();
        payload["info"]["code"] = json!(1);
        let (client, requests) = server(vec![ok(payload)]).await;
        let source = session(KugouLoginClient::Standard);
        client
            .native_write_list(
                &source,
                ListWrite::Visibility {
                    list_id: 37,
                    name: "Name preserved",
                    private,
                    sort: 42,
                    total_ver: 9,
                },
            )
            .await
            .unwrap();
        let all = requests.await.unwrap();
        let (query, body) = request(&all[0], Endpoint::ListModify, KugouLoginClient::Standard);
        assert_eq!(query["userid"], "123456789");
        assert!(!body.to_string().contains(&source.token));
        assert_eq!(body["is_pri"], u8::from(private));
        assert_eq!(body["is_mutual"], 0);
        assert_eq!(body["support_pub"], 1);
        assert_eq!(body["name"], "Name preserved");
        assert_eq!(body["total_ver"], 9);
        assert_eq!(body["type"], 0);
        assert_eq!(body["sort"], 42);
        assert_eq!(body["enckey"].as_str().unwrap().len(), 256);
        let cipher = BASE64.decode(body["encstr"].as_str().unwrap()).unwrap();
        let plaintext = decrypt_device_registration_response(&cipher, TEST_RANDOM_SEED).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&plaintext).unwrap(),
            json!({"token":source.token})
        );
        assert_eq!(
            body.as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            [
                "enckey",
                "encstr",
                "is_mutual",
                "is_pri",
                "listid",
                "name",
                "sort",
                "support_pub",
                "total_ver",
                "type",
                "userid"
            ]
        );
    }
    for code in [None, Some(0), Some(205)] {
        let mut payload = ack();
        if let Some(code) = code {
            payload["info"]["code"] = json!(code);
        }
        let (client, requests) = server(vec![ok(payload)]).await;
        let error = client
            .native_write_list(
                &session(KugouLoginClient::Standard),
                ListWrite::Visibility {
                    list_id: 37,
                    name: "Name",
                    private: true,
                    sort: 42,
                    total_ver: 9,
                },
            )
            .await
            .err()
            .unwrap();
        assert_eq!(error.code, ErrorCode::UpstreamError);
        assert_eq!(requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn visibility_wire_rejects_concept_and_invalid_identity_without_network() {
    let (client, requests) = server(vec![]).await;
    for (kind, list_id, name) in [
        (KugouLoginClient::Concept, 37, "Name"),
        (KugouLoginClient::Standard, 0, "Name"),
        (KugouLoginClient::Standard, 37, ""),
    ] {
        assert!(
            client
                .native_write_list(
                    &session(kind),
                    ListWrite::Visibility {
                        list_id,
                        name,
                        private: true,
                        sort: 42,
                        total_ver: 9,
                    }
                )
                .await
                .is_err()
        );
    }
    assert!(requests.await.unwrap().is_empty());
}

fn encrypted_delete_response(value: Value) -> Vec<u8> {
    let plaintext = serde_json::to_vec(&json!({"status":1,"error_code":0,"data":value})).unwrap();
    let encoded = encrypt_device_profile(&plaintext, TEST_RANDOM_SEED).unwrap();
    BASE64.decode(encoded.as_bytes()).unwrap()
}

#[test]
fn official_standard_change_codes_are_separate_from_legacy_endpoint_codes() {
    for code in [None, Some(0), Some(1), Some(205)] {
        let mut payload = ack();
        if let Some(code) = code {
            payload["info"]["code"] = json!(code);
        }
        let bytes = serde_json::to_vec(&json!({"status":1,"error_code":0,"data":payload})).unwrap();
        assert_eq!(
            acknowledge(&bytes, "123456789", 0, Some(37), AckPolicy::StandardChange).is_ok(),
            code == Some(1)
        );
        assert_eq!(
            acknowledge(&bytes, "123456789", 0, Some(37), AckPolicy::LegacyZero).is_ok(),
            code.is_none_or(|v| v == 0)
        );
    }
}

#[test]
fn standard_single_delete_requires_its_full_versioned_identity_receipt() {
    let good = json!({"userid":123456789,"total_ver":10,"pre_total_ver":9,"list_count":2});
    let good_bytes = encrypted_delete_response(good.clone());
    let ack = acknowledge_standard_delete(&good_bytes, TEST_RANDOM_SEED, "123456789", 37).unwrap();
    assert_eq!(ack.list_id, Some(37));
    assert_eq!(ack.total_ver, Some(10));
    assert_eq!(ack.previous_ver, Some(9));
    assert_eq!(ack.list_count, Some(2));
    assert!(
        acknowledge_standard_delete(
            &serde_json::to_vec(&json!({"status":1,"error_code":0,"data":good.clone()})).unwrap(),
            TEST_RANDOM_SEED,
            "123456789",
            37,
        )
        .is_err()
    );

    let wrong_uid = encrypted_delete_response(json!({
        "userid":222,"total_ver":10,"pre_total_ver":9,"list_count":2
    }));
    let wrong_uid_error =
        acknowledge_standard_delete(&wrong_uid, TEST_RANDOM_SEED, "123456789", 37)
            .err()
            .unwrap();
    assert_eq!(wrong_uid_error.code, ErrorCode::Conflict);

    for omitted in ["userid", "total_ver", "pre_total_ver", "list_count"] {
        let mut data = good.clone();
        data.as_object_mut().unwrap().remove(omitted);
        let bytes = encrypted_delete_response(data);
        assert!(acknowledge_standard_delete(&bytes, TEST_RANDOM_SEED, "123456789", 37).is_err());
    }
    assert!(acknowledge_standard_delete(&good_bytes, TEST_RANDOM_SEED, "123456789", 0).is_err());
}
fn ack() -> Value {
    json!({"userid":123456789,"total_ver":10,"pre_total_ver":9,"list_count":2,
    "info":{"listid":37,"type":0,"global_collection_id":"collection_1_111_37_0"}})
}

#[test]
fn metadata_cipher_matches_independent_openssl_vector_and_distinct_client_keys() {
    // Independently computed using OpenSSL AES-128-CBC, MD5(seed) hex halves as UTF-8.
    for client in [KugouLoginClient::Standard, KugouLoginClient::Concept] {
        let (key, cipher) = token_fields(client, "deadbeef-token", "a1B2c3").unwrap();
        assert_eq!(cipher, "VaEdhw4/84jD8Xt1LOFB21/sBr/kUU2u3lVldhSikl4=");
        assert_eq!(key.len(), 256);
        assert_eq!(key, key.to_ascii_uppercase());
        assert_ne!(
            key,
            token_fields(client, "deadbeef-token", "a1B2c3").unwrap().0
        );
        let modulus =
            num_bigint::BigUint::parse_bytes(crypto::rsa_modulus(client).unwrap().as_bytes(), 16)
                .unwrap();
        assert!(num_bigint::BigUint::parse_bytes(key.as_bytes(), 16).unwrap() < modulus);
    }
    assert_ne!(
        crypto::rsa_modulus(KugouLoginClient::Standard).unwrap(),
        crypto::rsa_modulus(KugouLoginClient::Concept).unwrap()
    );
    assert!(token_fields(KugouLoginClient::Web, "token", "a1B2c3").is_err());
    assert!(token_fields(KugouLoginClient::Standard, "token", "short").is_err());
}

#[tokio::test]
async fn management_requests_sign_exact_client_bodies_and_keep_local_and_source_ids_separate() {
    let client_kind = KugouLoginClient::Standard;
    for mode in ["create", "collect", "modify", "delete"] {
        let mut payload = ack();
        if mode != "delete" {
            payload["info"]["code"] = json!(1);
        }
        if mode == "collect" {
            payload["info"]["type"] = json!(1);
        }
        let (client, requests) = server(vec![ok(payload)]).await;
        let source = session(client_kind);
        let edit = edit();
        let operation = match mode {
            "create" => ListWrite::Add {
                name: "Name",
                private: true,
                source: None,
            },
            "collect" => ListWrite::Add {
                name: "Source",
                private: false,
                source: Some(Collection {
                    user_id: 222,
                    list_id: 0,
                    gid: "collection_1_222_88_0",
                }),
            },
            "modify" => ListWrite::Modify {
                list_id: 37,
                edit: &edit,
            },
            _ => ListWrite::Delete {
                list_id: 37,
                kind: 0,
                total_ver: 9,
            },
        };
        let result = client.native_write_list(&source, operation).await.unwrap();
        assert_eq!(result.list_id, Some(37));
        assert_eq!(result.total_ver, Some(10));
        let all = requests.await.unwrap();
        assert_eq!(all.len(), 1);
        let endpoint = match mode {
            "create" => Endpoint::ListCreate,
            "collect" => Endpoint::ListCollect,
            "modify" => Endpoint::ListModify,
            _ => Endpoint::ListDeleteStandard,
        };
        let (query, body) = request(&all[0], endpoint, client_kind);
        if endpoint == Endpoint::ListDeleteStandard {
            assert_eq!(
                query.keys().map(String::as_str).collect::<Vec<_>>(),
                vec![
                    "appid",
                    "clienttime",
                    "clientver",
                    "dfid",
                    "key",
                    "mid",
                    "p"
                ]
            );
            assert_eq!(
                query["key"],
                hex::encode(Md5::digest(
                    format!(
                        "{}{}{}{}",
                        client_kind.appid(),
                        crate::signing::ANDROID_SALT,
                        client_kind.clientver(),
                        query["clienttime"]
                    )
                    .as_bytes()
                ))
            );
            assert_eq!(query["p"].len(), 256);
            assert_eq!(query["p"], query["p"].to_ascii_uppercase());
            for omitted in ["userid", "token", "uuid", "plat"] {
                assert!(
                    !query.contains_key(omitted),
                    "unexpected query field {omitted}"
                );
            }
        } else {
            assert_eq!(query["userid"], "123456789");
        }
        assert_eq!(query.contains_key("last_time"), mode == "create");
        if mode == "modify" {
            assert!(body.get("token").is_none());
            assert!(!body.to_string().contains(&source.token));
            assert_eq!(body["is_pri"], 1);
            assert_eq!(body["sort"], 42);
            assert_eq!(body["intro"], "Keep\nintro");
            assert_eq!(body["total_ver"], 9);
            assert_eq!(body["enckey"].as_str().unwrap().len(), 256);
        } else if mode == "delete" {
            let encoded = body.as_str().unwrap();
            assert_eq!(
                encoded,
                "wlUfC9HGiin0NOvH9XdQ9glyaaIXz5C1SPxsQDYlrmXQb/5sGLuoWWvi24iGh/7f"
            );
            let ciphertext = BASE64.decode(encoded).unwrap();
            let plaintext =
                decrypt_device_registration_response(&ciphertext, TEST_RANDOM_SEED).unwrap();
            assert_eq!(
                serde_json::from_slice::<Value>(&plaintext).unwrap(),
                json!({"listid":37,"total_ver":9,"type":0})
            );
            assert!(!encoded.contains(&source.token));
        } else {
            assert_eq!(body["userid"], 123456789);
            assert_eq!(body["token"], source.token);
            assert_eq!(body["total_ver"], 0);
        }
        if mode == "collect" {
            assert_eq!(body["type"], 1);
            assert_eq!(body["list_create_userid"], 222);
            assert_eq!(body["list_create_listid"], 0);
            assert_eq!(body["list_create_gid"], "collection_1_222_88_0");
        }
    }
}

#[tokio::test]
async fn concept_new_collection_sdk_rejects_before_network() {
    let (client, requests) = server(vec![]).await;
    let result = client
        .native_write_list(
            &session(KugouLoginClient::Concept),
            ListWrite::Add {
                name: "Source",
                private: false,
                source: Some(Collection {
                    user_id: 222,
                    list_id: 88,
                    gid: "collection_1_222_88_0",
                }),
            },
        )
        .await;
    let Err(error) = result else {
        panic!("Concept new subscription must not dispatch the generic producer");
    };
    assert_eq!(error.code, ErrorCode::CapabilityNotSupported);
    assert!(!error.retryable);
    assert!(requests.await.unwrap().is_empty());
}

#[test]
fn management_acknowledgements_require_creation_identity_and_reject_conflicting_nested_fields() {
    for (mut value, code) in [
        (ack(), ErrorCode::Conflict),
        (ack(), ErrorCode::UpstreamError),
    ] {
        if code == ErrorCode::Conflict {
            value["listid"] = json!(99);
        } else {
            value["info"]["code"] = json!(205);
        }
        let bytes = serde_json::to_vec(&json!({"status":1,"error_code":0,"data":value})).unwrap();
        assert_eq!(
            acknowledge(&bytes, "123456789", 0, None, AckPolicy::LegacyZero)
                .err()
                .unwrap()
                .code,
            code
        );
    }
    for payload in [
        json!({}),
        json!({"listid":0}),
        json!({"info":[{"listid":37}]}),
        json!({"listid":"037"}),
    ] {
        let bytes = serde_json::to_vec(&json!({"status":1,"error_code":0,"data":payload})).unwrap();
        assert!(acknowledge(&bytes, "123456789", 0, None, AckPolicy::LegacyZero).is_err());
    }
    let bytes = serde_json::to_vec(
        &json!({"status":1,"error_code":0,"data":{"listid":37,"userid":123456789,"type":1}}),
    )
    .unwrap();
    assert!(acknowledge(&bytes, "123456789", 0, Some(37), AckPolicy::LegacyZero).is_err());
    assert!(acknowledge(&bytes, "111", 1, Some(37), AckPolicy::LegacyZero).is_err());
}

#[tokio::test]
async fn management_transport_bounds_responses_and_never_retries_business_or_network_failures() {
    for response in [
        frame(302, "Location: https://example.invalid/\r\n", vec![]),
        frame(429, "Content-Type: application/json\r\n", b"{}".to_vec()),
        frame(
            200,
            "Content-Type: text/html\r\n",
            b"<html>challenge</html>".to_vec(),
        ),
        frame(
            200,
            "Content-Type: application/json\r\n",
            vec![b'x'; 1_048_577],
        ),
        frame(
            200,
            "Content-Type: application/json\r\n",
            serde_json::to_vec(
                &json!({"status":0,"error_code":20017,"data":"management-private-marker"}),
            )
            .unwrap(),
        ),
    ] {
        let (client, requests) = server(vec![response]).await;
        let error = client
            .native_write_list(
                &session(KugouLoginClient::Standard),
                ListWrite::Delete {
                    list_id: 37,
                    kind: 0,
                    total_ver: 9,
                },
            )
            .await
            .err()
            .unwrap();
        assert!(!format!("{error:?}").contains("management-private-marker"));
        assert_eq!(requests.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn invalid_management_payloads_fail_before_network() {
    let (client, requests) = server(vec![]).await;
    let source = session(KugouLoginClient::Standard);
    for operation in [
        ListWrite::Add {
            name: "",
            private: false,
            source: None,
        },
        ListWrite::Delete {
            list_id: 0,
            kind: 0,
            total_ver: 9,
        },
        ListWrite::Delete {
            list_id: 37,
            kind: 2,
            total_ver: 9,
        },
        ListWrite::Add {
            name: "Name",
            private: true,
            source: Some(Collection {
                user_id: 222,
                list_id: 0,
                gid: "collection_1_222_88_0",
            }),
        },
    ] {
        assert!(client.native_write_list(&source, operation).await.is_err());
    }
    assert!(requests.await.unwrap().is_empty());
}
