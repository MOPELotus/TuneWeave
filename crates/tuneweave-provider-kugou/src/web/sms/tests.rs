use super::*;

#[test]
fn sms_send_encryption_and_exact_json_signature_match_independent_vectors() {
    let v: Value = serde_json::from_str(include_str!("test_vectors.json")).unwrap();
    let device = serde_json::from_value(v["device"].clone()).unwrap();
    let (query, body) = send_parameters(
        v["phone"].as_str().unwrap(),
        &device,
        v["milliseconds"].as_u64().unwrap(),
        &crypto::WebCipher::test_cipher(),
    )
    .unwrap();
    assert_eq!(serde_json::to_value(&query).unwrap(), v["query"]);
    assert_eq!(
        std::str::from_utf8(&body).unwrap(),
        v["body"].as_str().unwrap()
    );
    assert_eq!(web_signature(&query, &body), v["signature"]);
    assert!(
        !std::str::from_utf8(&body)
            .unwrap()
            .contains(v["phone"].as_str().unwrap())
    );
    let (q, b) = verify_parameters(
        &device,
        "13800000000",
        "123456",
        None,
        None,
        false,
        1700000000123,
    )
    .unwrap();
    assert_eq!(q["clienttime"], "1700000000");
    assert_eq!(q["clientver"], "10");
    assert_eq!(q["uuid"], "1700000000123");
    assert_eq!(
        std::str::from_utf8(&b).unwrap(),
        r#"{"plat":4,"mobile":"13800000000","code":"123456","expire_day":1,"support_multi":1,"userid":"","force_login":0}"#
    );
    let (_, b) = verify_parameters(
        &device,
        "13800000000",
        "123456",
        Some("222"),
        None,
        false,
        1700000000123,
    )
    .unwrap();
    assert_eq!(
        std::str::from_utf8(&b).unwrap(),
        r#"{"plat":4,"mobile":"13800000000","code":"123456","expire_day":1,"support_multi":1,"userid":222,"force_login":0}"#
    );
    for uid in ["0", "0222", "+222", "-222", "", "18446744073709551616"] {
        assert!(
            verify_parameters(
                &device,
                "13800000000",
                "123456",
                Some(uid),
                None,
                false,
                1700000000123
            )
            .is_err()
        );
    }
    let (q, b) = choices_parameters(&device, "13800000000", "123456", 1700000000123).unwrap();
    assert_eq!(q["uuid"], q["mid"]);
    assert_eq!(q["clienttime"], "1700000000");
    assert_eq!(
        std::str::from_utf8(&b).unwrap(),
        r#"{"plat":4,"mobile":"13800000000","code":"123456","businessid":5,"query":{"duration":1,"p_grade":1}}"#
    );
}

#[test]
fn sms_cookie_fields_never_expand_scope_inject_attributes_or_accept_duplicate_identity_fields() {
    let device = KugouDevice::default().identity().into_web();
    let valid = json!({"name":"KuGoo","domain":".kugou.com","path":"/","value":"KugooID=111&t=synthetic-sms-token&a_id=1014"});
    let parse = |v: &Value| {
        let raw = serde_json::value::to_raw_value(v).unwrap();
        parse_session(&raw, &device)
    };
    assert_eq!(parse(&valid).unwrap().user_id, "111");
    for (k, v) in [
        ("name", json!("Other")),
        ("domain", json!("evil.test")),
        ("domain", json!("loginservice.kugou.com")),
        ("path", json!("/v1")),
        ("value", json!("KugooID=111&t=token; Domain=evil.test")),
        ("value", json!("KugooID=111&KugooID=222&t=token")),
        ("value", json!("KugooID=0111&t=token")),
    ] {
        let mut b = valid.clone();
        b[k] = v;
        assert!(parse(&b).is_err(), "{k}");
    }
    let raw=serde_json::value::RawValue::from_string(r#"{"name":"KuGoo","domain":".kugou.com","path":"/","value":"KugooID=111&t=one","value":"KugooID=222&t=two"}"#.into()).unwrap();
    assert!(parse_session(&raw, &device).is_err());
}

#[test]
fn sms_account_choices_are_typed_unique_bounded_and_contain_no_opaque_response_data() {
    let envelope = |data: Value| {
        serde_json::from_value::<Envelope>(json!({"status":1,"error_code":0,"data":data})).unwrap()
    };
    let accounts=parse_choices(envelope(json!({"info_list":[{"userid":111,"nickname":"One","pic":"https://foreign.invalid/a.jpg","token":"never-export"},{"userid":"222","duration":999}]}))).unwrap();
    assert_eq!(accounts[0].user_id, "111");
    assert!(accounts[0].avatar_url.is_none());
    assert!(accounts[1].nickname.is_none());
    assert!(
        !serde_json::to_string(&accounts)
            .unwrap()
            .contains("never-export")
    );
    for rows in [
        json!([]),
        json!([{"userid":"01"}]),
        json!([{"userid":true}]),
        json!([{"userid":1.5}]),
        json!([{"userid":111},{"userid":"111"}]),
        json!([{"userid":111,"nickname":"bad\u{0000}"}]),
        json!(vec![json!({"userid":111}); 129]),
    ] {
        assert!(parse_choices(envelope(json!({"info_list":rows}))).is_err());
    }
    let duplicate: Envelope = serde_json::from_str(
        r#"{"status":1,"error_code":0,"data":{"info_list":[{"userid":111,"userid":222}]}}"#,
    )
    .unwrap();
    assert!(parse_choices(duplicate).is_err());
    let state = AuthChallengeStatus::AccountSelectionRequired { accounts };
    let wire = serde_json::to_value(state).unwrap();
    assert_eq!(wire["state"], "account_selection_required");
    assert!(wire.get("credential").is_none());
}

#[test]
fn sms_inputs_and_debug_do_not_expose_contact_data_or_accept_ambiguous_codes() {
    for phone in [
        "",
        "+8613800000000",
        " 13800000000",
        "10000000000",
        "1380000000a",
    ] {
        assert!(validate_phone(phone).is_err());
    }
    for code in ["", "123", "123456789", "12 456", "１２３４５６", "123456\n"] {
        assert!(validate_code(code).is_err());
    }
    let request = KugouWebSmsRequest {
        phone: "13800000000".into(),
        allow_account_creation: false,
    };
    assert!(!format!("{request:?}").contains("13800000000"));
    let r = KugouWebSmsChallenge {
        request,
        device: KugouDevice::default().identity().into_web(),
        deadline: Deadline::now() + Duration::from_secs(300),
        attempts: 0,
        password_principal: None,
        state: Stage::Waiting,
    };
    assert!(!format!("{r:?}").contains("13800000000"));
}
