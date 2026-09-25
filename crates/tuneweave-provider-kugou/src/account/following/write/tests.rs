use super::*;
use crate::account::cloud::tests::{binary, server};
use crate::account::tests::session;
use aes::cipher::BlockDecryptMut;

#[test]
fn artist_subscription_aes256_matches_independent_openssl_vector() {
    assert_eq!(
        encrypted_params(
            br#"{"singerid":42,"token":"synthetic-token"}"#,
            "0123456789abcdef"
        )
        .unwrap(),
        "2c5a58c74a7676d5bb3f03ea9fb437eb4f903e2c16024391e272e1b8543d0b47a4114f41b322e6229564a53f3f623623"
    );
}

#[test]
fn artist_subscription_ack_uses_status_not_optional_rank_or_message() {
    for value in [
        json!({"status":1}),
        json!({"status":1,"error_code":0,"data":null}),
        json!({"status":1,"data":{"rank":3,"msg":"secret-marker"}}),
    ] {
        acknowledge(value.to_string().as_bytes()).unwrap();
    }
    for value in [
        json!({"status":0,"error_code":20010,"data":{"rank":3,"msg":"secret-marker"}}),
        json!({"status":1,"error_code":20010}),
        json!({"status":"1"}),
        json!({"data":{"status":1}}),
    ] {
        let e = acknowledge(value.to_string().as_bytes()).unwrap_err();
        assert!(!format!("{e:?}").contains("secret-marker"));
    }
    assert!(acknowledge(br#"{"status":1,"status":0}"#).is_err());
    assert_eq!(
        acknowledge(br#"{"status":0,"error_code":20017}"#)
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
}

#[tokio::test]
async fn artist_subscription_wire_binds_exact_signed_body_and_encrypts_token() {
    for subscribed in [false, true] {
        let f = server(vec![binary(
            200,
            "application/json",
            br#"{"status":1}"#.to_vec(),
        )])
        .await;
        let session = session(KugouLoginClient::Standard);
        f.client
            .native_write_artist_subscription(&session, 42, subscribed)
            .await
            .unwrap();
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
        assert_eq!(
            url.path(),
            if subscribed {
                "/followservice/v3/follow_singer"
            } else {
                "/followservice/v3/unfollow_singer"
            }
        );
        let mut query: BTreeMap<String, String> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        assert_eq!(query.len(), 7);
        assert_eq!(query["uuid"], "-");
        assert_eq!(query["appid"], session.client.appid().to_string());
        assert_eq!(query["clientver"], session.client.clientver().to_string());
        assert_eq!(query["mid"], session.device.mid);
        assert_eq!(query["dfid"], session.device.dfid());
        let seconds = query["clienttime"].parse::<u64>().unwrap();
        assert!(seconds.abs_diff(now_ms().unwrap() / 1000) < 30);
        let signature = query.remove("signature").unwrap();
        // Reference concatenation uses sorted k=v strings without separators.
        let mut signed = ANDROID_SALT.as_bytes().to_vec();
        for (k, v) in &query {
            signed.extend_from_slice(format!("{k}={v}").as_bytes());
        }
        signed.extend_from_slice(&request.body);
        signed.extend_from_slice(ANDROID_SALT.as_bytes());
        assert_eq!(signature, format!("{:x}", Md5::digest(signed)));
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(body.as_object().unwrap().len(), 6);
        assert_eq!(body["plat"], "0");
        assert_eq!(body["userid"], 123456789);
        assert_eq!(body["singerid"], 42);
        assert_eq!(body["source"], 7);
        let p = body["p"].as_str().unwrap();
        assert_eq!(p.len(), 256);
        assert!(
            p.bytes()
                .all(|b| b.is_ascii_digit() || (b'A'..=b'F').contains(&b))
        );
        let key = b"4032af8d61035123906e58e067140cc5";
        let mut bytes = hex::decode(body["params"].as_str().unwrap()).unwrap();
        let decrypted = cbc::Decryptor::<Aes256>::new_from_slices(key, &key[16..])
            .unwrap()
            .decrypt_padded_mut::<Pkcs7>(&mut bytes)
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(decrypted).unwrap(),
            json!({"singerid":42,"token":session.token})
        );
        assert!(!request.head.contains(&session.token));
        assert!(!String::from_utf8_lossy(&request.body).contains(&session.token));
        assert!(
            request
                .head
                .to_ascii_lowercase()
                .contains("content-type: application/json;charset=utf-8")
        );
        assert!(!request.head.to_ascii_lowercase().contains("cookie:"));
    }
}

#[tokio::test]
async fn artist_subscription_transport_rejects_redirect_wrong_mime_and_over_budget() {
    for frame in [
        binary(302, "application/json", br#"{"status":1}"#.to_vec()),
        binary(200, "text/html", br#"{"status":1}"#.to_vec()),
        binary(200, "application/json", vec![b' '; RESPONSE_LIMIT + 1]),
        binary(200, "application/json", br#"{"status":1"#.to_vec()),
    ] {
        let f = server(vec![frame]).await;
        assert!(
            f.client
                .native_write_artist_subscription(&session(KugouLoginClient::Standard), 42, true)
                .await
                .is_err()
        );
        assert_eq!(f.requests.await.unwrap().len(), 1);
    }
    let f = server(vec![]).await;
    for id in [0, i64::MAX as u64 + 1] {
        assert_eq!(
            f.client
                .native_write_artist_subscription(&session(KugouLoginClient::Standard), id, true)
                .await
                .unwrap_err()
                .code,
            ErrorCode::InvalidRequest
        );
    }
    for client in [KugouLoginClient::Concept, KugouLoginClient::Web] {
        assert!(
            f.client
                .native_write_artist_subscription(&session(client), 42, true)
                .await
                .is_err()
        );
    }
    assert!(f.requests.await.unwrap().is_empty());
}
