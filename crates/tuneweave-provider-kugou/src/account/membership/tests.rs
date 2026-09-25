use super::*;

#[test]
fn membership_native_requests_match_independent_current_client_vectors() {
    let vectors: Vec<Value> = serde_json::from_str(include_str!("test_vectors.json")).unwrap();
    for v in vectors {
        let client = if v["client"] == "standard" {
            KugouLoginClient::Standard
        } else {
            KugouLoginClient::Concept
        };
        let query: BTreeMap<String, String> = serde_json::from_value(v["query"].clone()).unwrap();
        let session = NativeSession {
            client,
            device: crate::device::KugouDeviceIdentity {
                guid: v["guid"].as_str().unwrap().into(),
                mid: query["mid"].clone(),
                dfid: None,
            },
            user_id: "123456789".into(),
            token: format!("fixture-{}-token", v["client"].as_str().unwrap()),
            vip_token: None,
            t1: None,
        };
        let mut result = parameters(&session, 1789600000).unwrap();
        assert_eq!(result.remove("signature").unwrap(), v["signature"]);
        assert_eq!(serde_json::to_value(result).unwrap(), v["query"]);
    }
}

fn decode(data: Value, client: KugouLoginClient) -> Result<MembershipSummary> {
    let envelope = if client == KugouLoginClient::Standard {
        json!({"status":1,"errcode":0,"data":data})
    } else {
        json!({"status":1,"error_code":0,"data":data})
    };
    parse(envelope.to_string().as_bytes(), client, "123456789")
}

#[test]
fn membership_native_preserves_parallel_products_and_separate_dates() {
    let data = json!({"userid":"123456789","vip_type":5,"is_vip":0,"vip_end_time":"2024-01-01 00:00:00",
        "busi_vip":[
            {"userid":123456789,"busi_type":"concept","product_type":"svip","is_vip":1,"is_paid_vip":0,"y_type":0,"vip_end_time":"2027-01-02 03:04:05"},
            {"userid":"123456789","busi_type":"concept","product_type":"wvip","is_vip":1,"is_paid_vip":1,"y_type":1,"vip_end_time":"2028-02-03 00:00:00","paid_vip_expire_time":"2028-01-01 00:00:00"},
            {"userid":"123456789","busi_type":"concept","product_type":"future_vip","is_vip":2}
        ],"s_vip_end_time":"2029-03-04 05:06:07","token":"never-export","phone":"never-export"});
    let member = decode(data, KugouLoginClient::Concept).unwrap();
    assert_eq!(member.active, Some(false)); // Main VIP, not the active Concept product.
    assert_eq!(member.expires_at.as_deref(), Some("2024-01-01 00:00:00"));
    assert!(member.level.is_none());
    let details = &member.extensions["membership_details"];
    assert_eq!(details["busi_vip"].as_array().unwrap().len(), 3);
    assert_eq!(details["s_vip_end_time"], "2029-03-04 05:06:07");
    assert!(member.extensions["level_scope"].is_null());
    assert_eq!(
        details["busi_vip"][1]["paid_vip_expire_time"],
        "2028-01-01 00:00:00"
    );
    assert_eq!(details["busi_vip"][2]["is_vip"], 2);
    assert!(
        !serde_json::to_string(&member)
            .unwrap()
            .contains("never-export")
    );
    assert_eq!(member.extensions["date_timezone"], Value::Null);
}

#[test]
fn membership_standard_keeps_main_super_music_and_union_memberships_distinct() {
    let member = decode(json!({"userid":123456789,"vip_type":6,"vip_end_time":"2026-12-31 23:59:59",
        "svip_level":3,"user_type":8,"user_y_type":1,"su_vip_end_time":"2027-02-01 00:00:00",
        "m_type":2,"m_end_time":"2027-01-01 00:00:00",
        "union_vipinfo":{"busi_type":"child","product_type":"bundle","vip_valid":1,"vip_end_time":"2029-01-01 00:00:00"}}), KugouLoginClient::Standard).unwrap();
    assert_eq!(member.active, Some(true));
    assert_eq!(member.level, Some(3));
    assert_eq!(member.extensions["level_scope"], "super_membership");
    assert_eq!(member.expires_at.as_deref(), Some("2026-12-31 23:59:59"));
    assert_eq!(member.extensions["membership_details"]["user_type"], 8);
    assert_eq!(
        member.extensions["membership_details"]["union_vipinfo"]["vip_valid"],
        1
    );
    for code in [0, 5, 65530, 999] {
        let member = decode(
            json!({"userid":123456789,"vip_type":code}),
            KugouLoginClient::Standard,
        )
        .unwrap();
        assert_eq!(
            member.active,
            if code == 0 || code == 5 {
                Some(false)
            } else {
                None
            }
        );
        assert!(member.expires_at.is_none());
        assert!(member.level.is_none());
    }
}

#[test]
fn membership_native_rejects_wrong_identity_cross_client_schema_and_corrupt_fields() {
    for client in [KugouLoginClient::Standard, KugouLoginClient::Concept] {
        for uid in [json!("999"), json!(0), json!("0123456789"), json!(true)] {
            assert!(decode(json!({"userid":uid,"vip_type":0}), client).is_err());
        }
        for value in [json!(-1), json!(true), json!("01"), json!(1.1)] {
            assert!(decode(json!({"userid":123456789,"vip_type":value}), client).is_err());
        }
        for date in [
            json!("bad\nvalue"),
            json!(" 2027-01-01"),
            json!("a".repeat(129)),
            json!(123),
        ] {
            assert!(
                decode(
                    json!({"userid":123456789,"vip_type":0,"vip_end_time":date}),
                    client
                )
                .is_err()
            );
        }
    }
    assert!(
        decode(
            json!({"userid":123456789,"vip_type":0,"busi_vip":[]}),
            KugouLoginClient::Standard
        )
        .is_err()
    );
    let product = json!({"userid":123456789,"busi_type":"concept","product_type":"svip"});
    assert!(
        decode(
            json!({"userid":123456789,"vip_type":0,"busi_vip":[product,product]}),
            KugouLoginClient::Concept
        )
        .is_err()
    );
    assert!(decode(json!({"userid":123456789,"vip_type":0,"busi_vip":[{"userid":999,"busi_type":"concept","product_type":"svip"}]}),KugouLoginClient::Concept).is_err());
    assert!(
        parse(
            br#"{"status":1,"errcode":0,"data":{"userid":123456789,"userid":999,"vip_type":0}}"#,
            KugouLoginClient::Standard,
            "123456789"
        )
        .is_err()
    );
}

#[test]
fn membership_native_business_failures_are_not_non_members() {
    for (client, field) in [
        (KugouLoginClient::Standard, "errcode"),
        (KugouLoginClient::Concept, "error_code"),
    ] {
        for code in [20017, 20010] {
            let mut body = json!({"status":0,"data":{}});
            body[field] = json!(code);
            let e = parse(body.to_string().as_bytes(), client, "123456789").unwrap_err();
            assert_eq!(
                e.code,
                if code == 20017 {
                    ErrorCode::AuthenticationRequired
                } else {
                    ErrorCode::UpstreamError
                }
            );
        }
        assert!(parse(b"{}", client, "123456789").is_err());
        assert!(parse(&vec![b' '; LIMIT + 1], client, "123456789").is_err());
    }
}
