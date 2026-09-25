use super::*;

pub(crate) fn body(member: serde_json::Value) -> serde_json::Value {
    json!({"status_code":0,"status_info":{"now":1700000000,"now_ts_ms":1700000000000_u64},"membership":member})
}
pub(crate) fn fixture() -> serde_json::Value {
    body(
        json!({"is_membership":true,"membership_type":"vip","expire_time":1800000000,"is_paying_user":true,"is_about_to_expire":false,"in_grace_period":false,"last_membership_type":"free_vip"}),
    )
}

#[test]
fn commerce_membership_maps_explicit_fields_and_never_infers_rights_or_missing_status() {
    let value = parse(&serde_json::to_vec(&fixture()).unwrap(), "123456").unwrap();
    assert_eq!(value.active, Some(true));
    assert_eq!(value.expires_at.as_deref(), Some("2027-01-15T08:00:00Z"));
    assert_eq!(value.extensions["expires_at_epoch_seconds"], 1800000000);
    assert_eq!(value.extensions["membership_type"], "vip");
    assert_eq!(value.extensions["vip_stage"], "vip");
    assert_eq!(value.extensions["in_grace_period"], false);
    assert_eq!(value.user_ref.unwrap().to_string(), "soda:123456");
    assert!(value.level.is_none() && value.annual_count.is_none() && value.icon_url.is_none());
    for (member, active, expiry) in [
        (
            json!({"is_membership":false,"expire_time":0}),
            Some(false),
            None,
        ),
        (json!({"membership_type":"free_vip"}), None, None),
        (
            json!({"is_membership":true,"expire_time":1600000000,"in_grace_period":true}),
            Some(true),
            Some("2020-09-13T12:26:40Z"),
        ),
        (
            json!({"is_membership":false,"expire_time":1800000000}),
            Some(false),
            Some("2027-01-15T08:00:00Z"),
        ),
    ] {
        let mut data = body(member);
        data["play_entitlements"] = json!({"expire_at":4000000000_u64});
        data["offers"] = json!({"private":"do-not-export"});
        data["membership"]["membership_detail_map"] =
            json!({"svip":{"expire_time":4000000000_u64,"private":"do-not-export"}});
        let summary = parse(&serde_json::to_vec(&data).unwrap(), "123456").unwrap();
        assert_eq!(summary.active, active);
        assert_eq!(summary.expires_at.as_deref(), expiry);
        let out = serde_json::to_string(&summary).unwrap();
        assert!(
            !out.contains("do-not-export")
                && !out.contains("4000000000")
                && !out.contains("playable")
        );
    }
}

#[test]
fn commerce_membership_rejects_anonymous_empty_invalid_status_and_malformed_fields() {
    for mutation in 0..19 {
        let mut value = fixture();
        match mutation {
            0 => {
                value.as_object_mut().unwrap().remove("membership");
            }
            1 => value["membership"] = json!(null),
            2 => value["membership"] = json!({}),
            3 => value["membership"] = json!({"membership_type":"","is_membership":null}),
            4 => {
                value.as_object_mut().unwrap().remove("status_code");
            }
            5 => value["status_code"] = json!(2),
            6 => value["status_info"]["status_code"] = json!(2),
            7 => value["status_info"]["now"] = json!(0),
            8 => value["status_info"]["now_ts_ms"] = json!(1000),
            9 => value["membership"]["expire_time"] = json!("1800000000"),
            10 => value["membership"]["expire_time"] = json!(-1),
            11 => value["membership"]["expire_time"] = json!(MAX_TIME + 1),
            12 => value["membership"]["is_membership"] = json!(1),
            13 => value["membership"]["is_paying_user"] = json!("true"),
            14 => value["membership"]["in_grace_period"] = json!([]),
            15 => value["membership"]["membership_type"] = json!("a".repeat(65)),
            16 => value["membership"]["last_membership_type"] = json!("bad\nfield"),
            17 => value["status_info"]["now"] = json!(MAX_TIME + 1),
            _ => value["membership"] = json!([]),
        }
        assert_eq!(
            parse(&serde_json::to_vec(&value).unwrap(), "123456")
                .unwrap_err()
                .code,
            ErrorCode::UpstreamError,
            "mutation {mutation}"
        );
    }
    for value in [
        json!({"status_code":1000016}),
        json!({"status_info":{"status_code":1000016}}),
    ] {
        assert_eq!(
            parse(&serde_json::to_vec(&value).unwrap(), "123456")
                .unwrap_err()
                .code,
            ErrorCode::AuthenticationRequired
        );
    }
    assert!(parse(br#"{"status_code":0,"status_code":1}"#, "123456").is_err());
}

#[tokio::test]
async fn commerce_membership_sdk_uses_fixed_readonly_body_device_and_selected_cookie() {
    let (origin, server) = crate::test_http::serve(vec![crate::test_http::json(
        &fixture().to_string(),
        Some("sessionid_ss=candidate"),
    )])
    .await;
    let client = SodaClient::test_client().with_auth_test_origin(origin.clone());
    let source = SodaCredential::test_credential("selected-secret")
        .bind_user("123456")
        .unwrap();
    let value = client.commerce_membership(&source).await.unwrap();
    assert_eq!(
        value.credential.cookie_header().unwrap(),
        "sessionid_ss=candidate"
    );
    assert!(value.credential.same_login(&source));
    assert_eq!(
        source.cookie_header().unwrap(),
        "sessionid_ss=selected-secret"
    );
    assert_eq!(value.summary.extensions["backend"], BACKEND);
    let requests = server.await.unwrap();
    assert_eq!(requests.len(), 1);
    let (headers, raw) = requests[0].split_once("\r\n\r\n").unwrap();
    assert!(headers.starts_with(&format!("POST {PATH}?")));
    assert!(headers.contains("cookie: sessionid_ss=selected-secret\r\n"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(raw).unwrap(),
        json!({"includes":["membership"]})
    );
    let url = origin
        .join(
            headers
                .lines()
                .next()
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap(),
        )
        .unwrap();
    let query = url.query_pairs().collect::<BTreeMap<_, _>>();
    assert_eq!(query.len(), 9);
    assert_eq!(query["aid"], SODA_APP_ID);
    assert_eq!(query["version_code"], "20010000");
    assert_ne!(query["device_id"], query["iid"]);
    assert_eq!(query["fp"], query["device_id"]);
    assert!(!url.as_str().contains("secret"));
}

#[tokio::test]
async fn commerce_membership_sdk_rejects_failed_untrusted_oversized_and_secret_responses() {
    let good = crate::test_http::json(&fixture().to_string(), Some("sessionid_ss=candidate"));
    let mut reflected = fixture();
    reflected["membership"]["membership_type"] = json!("selected-secret");
    for (response, code) in [
        (
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".into(),
            ErrorCode::AuthenticationRequired,
        ),
        (
            crate::test_http::json(r#"{"status_code":1000016}"#, None),
            ErrorCode::AuthenticationRequired,
        ),
        (
            "HTTP/1.1 302 Found\r\nLocation: https://example.invalid/\r\nContent-Length: 0\r\n\r\n"
                .into(),
            ErrorCode::UpstreamError,
        ),
        (
            good.replace("application/json", "text/html"),
            ErrorCode::UpstreamError,
        ),
        (
            good.replace(
                "Content-Type:",
                "bdturing-verify: private-challenge\r\nContent-Type:",
            ),
            ErrorCode::CapabilityNotSupported,
        ),
        (
            crate::test_http::json(&body(json!({})).to_string(), None),
            ErrorCode::UpstreamError,
        ),
        (
            crate::test_http::json(&reflected.to_string(), None),
            ErrorCode::UpstreamError,
        ),
        (
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                MAX_BYTES + 1
            ),
            ErrorCode::UpstreamError,
        ),
        (
            format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{}",
                " ".repeat(MAX_BYTES + 1)
            ),
            ErrorCode::UpstreamError,
        ),
        (
            crate::test_http::json(&fixture().to_string(), Some("sessionid_ss=; Max-Age=0")),
            ErrorCode::AuthenticationRequired,
        ),
    ] {
        let (origin, server) = crate::test_http::serve(vec![response]).await;
        let client = SodaClient::test_client().with_auth_test_origin(origin);
        let source = SodaCredential::test_credential("selected-secret")
            .bind_user("123456")
            .unwrap();
        let err = client.commerce_membership(&source).await.unwrap_err();
        assert_eq!(err.code, code);
        assert!(
            !format!("{err:?}").contains("selected-secret")
                && !format!("{err:?}").contains("private-challenge")
        );
        assert_eq!(server.await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn commerce_membership_unbound_session_does_not_initialize_device() {
    let path = std::env::temp_dir().join(format!("soda-member-unbound-{}", rand::random::<u64>()));
    let client = SodaClient::new(&SodaConfig {
        device_path: Some(path.clone()),
        ..SodaConfig::default()
    })
    .unwrap();
    assert_eq!(
        client
            .commerce_membership(&SodaCredential::test_credential("secret"))
            .await
            .unwrap_err()
            .code,
        ErrorCode::AuthenticationRequired
    );
    assert!(!path.exists());
}
